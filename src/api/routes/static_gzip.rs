//! Gzip for the static web UI (the `ServeDir` fallback) without paying for it on every
//! request: each distinct file content is compressed once -- keyed by a hash of the bytes, so a
//! rebuilt bundle or an edited `style.css` is picked up immediately (even a same-length rewrite
//! within the same second), and every spelling of one file's URL shares one entry -- and the
//! compressed bytes are reused from memory after that. Compressing is what's saved; reading and
//! hashing the file is skipped too once a path's (Last-Modified, length) has been seen and that
//! mtime is old enough that no same-second rewrite can still share it. Requests that miss together
//! wait for one compression. The cache is bounded. No `.gz` files on disk to go stale under `bun --watch`.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::sync::{Arc, Mutex, OnceLock};

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

/// The most compressed bytes kept; past it the cache starts again empty (the UI's own files are a
/// few MB, so this only bites when something keeps asking for new content).
const MAX_CACHED_BYTES: usize = 64 * 1024 * 1024;
/// The most entries kept (content slots plus path shortcuts), so content that doesn't compress --
/// which costs no bytes -- can't grow the maps without bound either.
const MAX_ENTRIES: usize = 4096;
/// How far a file's Last-Modified (whole seconds) must be in the past before (path, Last-Modified,
/// length) is trusted to mean the same content: a rewrite within that second could keep all three.
const SETTLED_SECS: i64 = 2;

/// One content's compressed form, filled by whichever request gets there first (`None` when
/// compressing didn't make it smaller).
type Slot = Arc<tokio::sync::OnceCell<Option<Bytes>>>;

type ContentKey = (u64, usize);

#[derive(Default)]
struct Cache {
    slots: HashMap<ContentKey, Slot>,
    /// (request path, Last-Modified, length) of a settled file -> its content's key.
    by_path: HashMap<(String, HeaderValue, u64), ContentKey>,
    bytes: usize,
    /// Bumped on every reset, so a compression that finishes after one doesn't count its bytes
    /// against the new, empty cache it isn't in.
    generation: u64,
}

impl Cache {
    fn make_room(&mut self) {
        if self.bytes > MAX_CACHED_BYTES || self.slots.len() + self.by_path.len() >= MAX_ENTRIES {
            let generation = self.generation + 1;
            *self = Cache { generation, ..Cache::default() };
        }
    }
}

/// Whether a Last-Modified value is far enough in the past that the file can't still be rewritten
/// within that same second.
fn settled(last_modified: &HeaderValue) -> bool {
    let Some(t) = last_modified.to_str().ok().and_then(|v| chrono::DateTime::parse_from_rfc2822(v).ok()) else { return false };
    chrono::Utc::now().signed_duration_since(t).num_seconds() >= SETTLED_SECS
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Cache::default()))
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
    let Some(len) = res.headers().get(CONTENT_LENGTH).and_then(|v| v.to_str().ok()?.parse::<u64>().ok()) else { return res };
    if !(MIN_BYTES..=MAX_BYTES).contains(&len) {
        return res;
    }

    let path_key = res.headers().get(LAST_MODIFIED).filter(|lm| settled(lm)).map(|lm| (path, lm.clone(), len));
    if let Some(pk) = &path_key {
        let known = {
            let c = cache().lock().unwrap_or_else(|e| e.into_inner());
            c.by_path.get(pk).and_then(|key| c.slots.get(key)).and_then(|slot| slot.get().cloned().flatten())
        };
        if let Some(gz) = known {
            // Unchanged since it was hashed: the file's body is dropped unread.
            return gzipped(res, gz);
        }
    }

    let (parts, body) = res.into_parts();
    let Ok(raw) = axum::body::to_bytes(body, MAX_BYTES as usize).await else {
        // The file failed mid-read; the headers already promised a length we can't deliver.
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let key = {
        let mut h = DefaultHasher::new();
        raw.hash(&mut h);
        (h.finish(), raw.len())
    };
    let (slot, generation): (Slot, u64) = {
        let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
        if !c.slots.contains_key(&key) {
            c.make_room();
        }
        if let Some(pk) = path_key {
            if !c.by_path.contains_key(&pk) {
                c.make_room();
            }
            c.by_path.insert(pk, key);
        }
        (c.slots.entry(key).or_default().clone(), c.generation)
    };
    let to_compress = raw.clone();
    let gz = slot.get_or_init(|| async move {
        let gz = match tokio::task::spawn_blocking(move || gzip(&to_compress)).await {
            Ok(Ok(gz)) if (gz.len() as u64) < len => Some(Bytes::from(gz)),
            _ => None,
        };
        let size = gz.as_ref().map_or(0, Bytes::len);
        let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
        if c.generation == generation {
            c.bytes += size;
        }
        gz
    }).await.clone();
    match gz {
        Some(gz) => gzipped(Response::from_parts(parts, Body::empty()), gz),
        None => Response::from_parts(parts, Body::from(raw)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header::{IF_MODIFIED_SINCE, LAST_MODIFIED};
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
        // So is a rewrite of the same length within that second: the content is what's keyed.
        let v3 = "console.log('v3!');\n".repeat(500);
        assert_eq!(v3.len(), v2.len());
        std::fs::write(d.join("bundle.js"), &v3).unwrap();
        let (_, body) = get(&d, "/bundle.js", &[(ACCEPT_ENCODING, "gzip")]).await;
        assert_eq!(gunzip(&body), v3.as_bytes());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn spellings_of_one_file_and_concurrent_misses_share_one_entry() {
        let d = tmp_dir("alias");
        let js = "console.log('alias');\n".repeat(700);
        std::fs::write(d.join("bundle.js"), &js).unwrap();
        // Several requests at once, under different spellings of the same path.
        let paths = ["/bundle.js", "/b%75ndle.js", "/%62undle.js", "/bu%6edle.js"];
        let results = futures_util::future::join_all(paths.iter().map(|p| get(&d, p, &[(ACCEPT_ENCODING, "gzip")]))).await;
        for (res, body) in &results {
            assert_eq!(res.headers()[CONTENT_ENCODING], "gzip");
            assert_eq!(gunzip(body), js.as_bytes());
        }
        // Keyed by content: the one entry all four spellings shared.
        let key = { let mut h = DefaultHasher::new(); Bytes::from(js.clone()).hash(&mut h); (h.finish(), js.len()) };
        assert!(cache().lock().unwrap().slots.contains_key(&key));
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
    async fn a_settled_file_is_served_without_rereading_it() {
        let d = tmp_dir("settled");
        let file = d.join("old.js");
        let set = |text: &str| {
            std::fs::write(&file, text).unwrap();
            let f = std::fs::File::options().write(true).open(&file).unwrap();
            f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600)).unwrap();
        };
        let v1 = "console.log('old1');\n".repeat(400);
        set(&v1);
        let (_, body) = get(&d, "/old.js", &[(ACCEPT_ENCODING, "gzip")]).await;
        assert_eq!(gunzip(&body), v1.as_bytes());
        // Same path, Last-Modified and length: taken on trust, so the stale copy proves the
        // shortcut skipped the read. (Real files can't do this: their mtime moves on a rewrite.)
        let v2 = "console.log('old2');\n".repeat(400);
        set(&v2);
        let (_, body) = get(&d, "/old.js", &[(ACCEPT_ENCODING, "gzip")]).await;
        assert_eq!(gunzip(&body), v1.as_bytes());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_reset_drops_bytes_counted_by_an_older_generation() {
        let mut c = Cache { bytes: MAX_CACHED_BYTES + 1, generation: 7, ..Cache::default() };
        c.make_room();
        assert_eq!((c.bytes, c.generation), (0, 8));
        for i in 0..MAX_ENTRIES {
            c.slots.insert((i as u64, 0), Slot::default());
        }
        c.make_room();
        assert!(c.slots.is_empty());
        assert_eq!(c.generation, 9);
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
