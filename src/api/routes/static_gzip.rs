//! Gzip for the static web UI (the `ServeDir` fallback) without paying for it on every
//! request: each file is compressed once per version -- keyed by path + `Last-Modified` +
//! length, so a rebuilt bundle or an edited `style.css` is picked up immediately -- and the
//! compressed bytes are reused from memory after that. No `.gz` files on disk to go stale
//! under `bun --watch`, and no per-request deflate the way a plain compression layer would.

use std::collections::HashMap;
use std::io::Write;
use std::sync::{Mutex, OnceLock};

use axum::body::{Body, Bytes};
use axum::extract::Request;
use axum::http::header::{ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, LAST_MODIFIED, RANGE, VARY};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// Smaller than this, the gzip framing eats most of the win.
const MIN_BYTES: u64 = 1024;
/// Larger than this isn't a UI asset (bundles are ~1MB); serve it as-is rather than buffer it.
const MAX_BYTES: u64 = 8 * 1024 * 1024;

struct Entry {
    last_modified: HeaderValue,
    len: u64,
    gz: Bytes,
}

fn cache() -> &'static Mutex<HashMap<String, Entry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn accepts_gzip(headers: &HeaderMap) -> bool {
    headers.get_all(ACCEPT_ENCODING).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(',')).any(|item| {
        let mut parts = item.split(';');
        let name = parts.next().unwrap_or("").trim();
        let refused = parts.any(|p| matches!(p.trim().strip_prefix("q="), Some(q) if q.trim().parse::<f32>().map_or(false, |q| q == 0.0)));
        (name.eq_ignore_ascii_case("gzip") || name == "*") && !refused
    })
}

fn compressible(headers: &HeaderMap) -> bool {
    let Some(ct) = headers.get(CONTENT_TYPE).and_then(|v| v.to_str().ok()) else { return false };
    let ct = ct.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    ct.starts_with("text/") || matches!(ct.as_str(), "application/javascript" | "application/json" | "application/manifest+json" | "image/svg+xml" | "application/wasm")
}

fn gzip(data: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut enc = flate2::write::GzEncoder::new(Vec::with_capacity(data.len() / 3), flate2::Compression::best());
    enc.write_all(data)?;
    enc.finish()
}

fn gzipped(mut res: Response, gz: Bytes) -> Response {
    let h = res.headers_mut();
    h.insert(CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    h.insert(CONTENT_LENGTH, HeaderValue::from(gz.len()));
    *res.body_mut() = Body::from(gz);
    res
}

pub(super) async fn gzip_static(req: Request, next: Next) -> Response {
    // GET only: ServeDir answers HEAD with the file's length but an empty body, and gzipping
    // (and caching) that would hand every later GET a compressed empty file.
    let wants_gzip = req.method() == Method::GET && accepts_gzip(req.headers()) && !req.headers().contains_key(RANGE);
    let path = req.uri().path().to_owned();
    let mut res = next.run(req).await;

    if !compressible(res.headers()) {
        return res;
    }
    // The same URL can come back in either encoding, so caches must key on it.
    res.headers_mut().append(VARY, HeaderValue::from_static("accept-encoding"));
    if !wants_gzip || res.status() != StatusCode::OK || res.headers().contains_key(CONTENT_ENCODING) {
        return res;
    }
    let Some(last_modified) = res.headers().get(LAST_MODIFIED).cloned() else { return res };
    let Some(len) = res.headers().get(CONTENT_LENGTH).and_then(|v| v.to_str().ok()?.parse::<u64>().ok()) else { return res };
    if !(MIN_BYTES..=MAX_BYTES).contains(&len) {
        return res;
    }

    let hit = {
        let c = cache().lock().unwrap_or_else(|e| e.into_inner());
        c.get(&path).filter(|e| e.len == len && e.last_modified == last_modified).map(|e| e.gz.clone())
    };
    if let Some(gz) = hit {
        // Dropping ServeDir's body unpolled means the file is never actually read.
        return gzipped(res, gz);
    }

    let (parts, body) = res.into_parts();
    let Ok(raw) = axum::body::to_bytes(body, MAX_BYTES as usize).await else {
        // The file failed mid-read; the headers already promised a length we can't deliver.
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let to_compress = raw.clone();
    let gz = match tokio::task::spawn_blocking(move || gzip(&to_compress)).await {
        Ok(Ok(gz)) if (gz.len() as u64) < len => Bytes::from(gz),
        _ => return Response::from_parts(parts, Body::from(raw)),
    };
    cache().lock().unwrap_or_else(|e| e.into_inner()).insert(path, Entry { last_modified, len, gz: gz.clone() });
    gzipped(Response::from_parts(parts, Body::empty()), gz)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header::IF_MODIFIED_SINCE;
    use std::io::Read;
    use tower::Service;

    fn app(dir: &std::path::Path) -> axum::Router {
        axum::Router::new().fallback_service(tower::layer::Layer::layer(
            &axum::middleware::from_fn(gzip_static),
            tower_http::services::ServeDir::new(dir),
        ))
    }

    async fn get(dir: &std::path::Path, path: &str, headers: &[(axum::http::HeaderName, &str)]) -> (Response, Vec<u8>) {
        let mut req = Request::builder().uri(path);
        for (k, v) in headers {
            req = req.header(k, *v);
        }
        // Router is always ready, so no poll_ready dance is needed.
        let res = app(dir).call(req.body(Body::empty()).unwrap()).await.unwrap();
        let (parts, body) = res.into_parts();
        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap().to_vec();
        (Response::from_parts(parts, Body::empty()), bytes)
    }

    fn gunzip(b: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(b).read_to_end(&mut out).unwrap();
        out
    }

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("os-static-gzip-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[tokio::test]
    async fn compresses_once_and_picks_up_a_rebuilt_file() {
        let d = tmp_dir("rebuild");
        let v1 = "console.log('v1');\n".repeat(500);
        std::fs::write(d.join("bundle.js"), &v1).unwrap();

        for _ in 0..2 {
            let (res, body) = get(&d, "/bundle.js", &[(ACCEPT_ENCODING, "gzip, deflate, br")]).await;
            assert_eq!(res.headers()[CONTENT_ENCODING], "gzip");
            assert_eq!(res.headers()[CONTENT_LENGTH], body.len().to_string().as_str());
            assert!(body.len() < v1.len() / 10);
            assert_eq!(gunzip(&body), v1.as_bytes());
        }

        // A different length is a different version even within the same mtime second.
        let v2 = "console.log('v2!');\n".repeat(500);
        std::fs::write(d.join("bundle.js"), &v2).unwrap();
        let (_, body) = get(&d, "/bundle.js", &[(ACCEPT_ENCODING, "gzip")]).await;
        assert_eq!(gunzip(&body), v2.as_bytes());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn plain_when_not_accepted_ranged_tiny_or_binary() {
        let d = tmp_dir("plain");
        let css = "body { color: red; }\n".repeat(200);
        std::fs::write(d.join("style.css"), &css).unwrap();
        std::fs::write(d.join("tiny.js"), "x()").unwrap();
        std::fs::write(d.join("pic.png"), vec![7u8; 4096]).unwrap();

        for (path, hdrs) in [
            ("/style.css", vec![]),
            ("/style.css", vec![(ACCEPT_ENCODING, "gzip;q=0, identity")]),
            ("/style.css", vec![(ACCEPT_ENCODING, "gzip"), (RANGE, "bytes=0-9")]),
            ("/tiny.js", vec![(ACCEPT_ENCODING, "gzip")]),
            ("/pic.png", vec![(ACCEPT_ENCODING, "gzip")]),
        ] {
            let (res, _) = get(&d, path, &hdrs).await;
            assert!(res.headers().get(CONTENT_ENCODING).is_none(), "{path} {hdrs:?}");
        }
        let (res, body) = get(&d, "/style.css", &[]).await;
        assert_eq!(res.headers()[VARY], "accept-encoding");
        assert_eq!(body, css.as_bytes());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn a_head_request_neither_gets_nor_caches_a_gzip_of_its_empty_body() {
        let d = tmp_dir("head");
        let js = "console.log('head');\n".repeat(400);
        std::fs::write(d.join("head.js"), &js).unwrap();
        let req = Request::builder().method("HEAD").uri("/head.js").header(ACCEPT_ENCODING, "gzip").body(Body::empty()).unwrap();
        let res = app(&d).call(req).await.unwrap();
        assert!(res.headers().get(CONTENT_ENCODING).is_none());
        // The GET after it must still get the real file.
        let (res, body) = get(&d, "/head.js", &[(ACCEPT_ENCODING, "gzip")]).await;
        assert_eq!(res.headers()[CONTENT_ENCODING], "gzip");
        assert_eq!(gunzip(&body), js.as_bytes());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn revalidation_still_gets_a_304() {
        let d = tmp_dir("304");
        std::fs::write(d.join("index.html"), "<p>hi</p>\n".repeat(300)).unwrap();
        let (res, _) = get(&d, "/index.html", &[(ACCEPT_ENCODING, "gzip")]).await;
        let lm = res.headers()[LAST_MODIFIED].to_str().unwrap().to_owned();
        let (res, body) = get(&d, "/index.html", &[(ACCEPT_ENCODING, "gzip"), (IF_MODIFIED_SINCE, &lm)]).await;
        assert_eq!(res.status(), StatusCode::NOT_MODIFIED);
        assert!(body.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }
}
