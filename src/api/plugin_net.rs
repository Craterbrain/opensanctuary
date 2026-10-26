//! The network access the host gives plugins that were granted `network.outbound`
//! (docs/PLUGIN_PERMISSIONS.md): the host makes the request, and only to hosts the operator
//! approved. A sandboxed plugin can't connect anywhere itself, and a browser couldn't read a
//! cross-origin page anyway, so this is how a plugin like the Genius lyrics importer reaches
//! its source.
//!
//! The request is checked at every hop: https only, no credentials in the URL, the host (and
//! port) must be on the granted list, and the address it resolves to must be public. The client
//! is pinned to the address that was checked, so a DNS answer can't change between the check and
//! the connect.

use std::net::{IpAddr, SocketAddr};

use reqwest::redirect::Policy;

/// The most of a response body the host will read for a plugin. Large on purpose: a whole Bible
/// translation is one 10-15 MB JSON file.
pub const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;
const MAX_REDIRECTS: usize = 4;

pub struct Fetched {
    pub status: u16,
    pub content_type: String,
    pub body: String,
    pub final_url: String,
    /// How much longer the publisher lets this response be kept, from its `Cache-Control`/`Age`.
    pub cache_secs: Option<u64>,
}

/// A granted host: `example.com`, `*.example.com` (subdomains only), optionally `:port`.
/// A bare host means port 443.
fn host_matches(entry: &str, host: &str, port: u16) -> bool {
    let (entry_host, entry_port) = match entry.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() => (h, p.parse().unwrap_or(0)),
        _ => (entry, 443u16),
    };
    if entry_port != port {
        return false;
    }
    let host = host.to_ascii_lowercase();
    let entry_host = entry_host.to_ascii_lowercase();
    match entry_host.strip_prefix("*.") {
        Some(suffix) => host.len() > suffix.len() + 1 && host.ends_with(&format!(".{suffix}")),
        None => host == entry_host,
    }
}

/// Whether `url` may be fetched under `allowed`. Pure (no network), so it is unit-tested.
pub fn check_url(allowed: &[String], url: &reqwest::Url) -> Result<(String, u16), String> {
    if url.scheme() != "https" {
        return Err("only https:// URLs can be fetched".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("URLs with credentials are refused".to_string());
    }
    let host = url.host_str().ok_or("URL has no host")?.to_string();
    let port = url.port_or_known_default().unwrap_or(443);
    if host.parse::<IpAddr>().is_ok() {
        return Err("fetching an IP address directly is refused".to_string());
    }
    if !allowed.iter().any(|a| host_matches(a, &host, port)) {
        return Err(format!("'{host}' isn't one of the hosts this plugin was granted"));
    }
    Ok((host, port))
}

/// Public, globally routable addresses only: not loopback, private, link-local, CGNAT,
/// unique-local, multicast or unspecified (including an IPv4 address mapped into IPv6).
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || o[0] == 0
                || (o[0] == 100 && (64..=127).contains(&o[1])) // CGNAT 100.64.0.0/10
                || o[0] >= 240) // reserved
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_ip(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (first & 0xfe00) == 0xfc00 // unique local fc00::/7
                || (first & 0xffc0) == 0xfe80) // link-local fe80::/10
        }
    }
}

async fn resolve_public(host: &str, port: u16) -> Result<SocketAddr, String> {
    // A resolver that never answers must not hold the request open.
    let lookup = tokio::time::timeout(std::time::Duration::from_secs(8), tokio::net::lookup_host((host, port)))
        .await
        .map_err(|_| format!("looking up {host} timed out"))?;
    let addrs = lookup.map_err(|e| format!("can't resolve {host}: {e}"))?;
    addrs
        .into_iter()
        .find(|a| is_public_ip(a.ip()))
        .ok_or_else(|| format!("{host} doesn't resolve to a public address"))
}

/// Request headers a plugin may add: enough for an API key and a content preference, nothing that
/// could change where or how the request is routed (no Host, Cookie, Proxy-*, ...). `x-yvp-app-key`
/// is the YouVersion Platform's API-key header; it only ever reaches the hosts the plugin was granted.
const ALLOWED_HEADERS: [&str; 4] = ["authorization", "accept", "accept-language", "x-yvp-app-key"];

/// Validates a plugin's header map into `(name, value)` pairs.
pub fn clean_headers(headers: &serde_json::Map<String, serde_json::Value>) -> Result<Vec<(String, String)>, String> {
    if headers.len() > 4 {
        return Err("too many headers".to_string());
    }
    let mut out = Vec::new();
    for (name, value) in headers {
        let lower = name.to_ascii_lowercase();
        if !ALLOWED_HEADERS.contains(&lower.as_str()) {
            return Err(format!("the '{name}' header can't be set (allowed: {})", ALLOWED_HEADERS.join(", ")));
        }
        let v = value.as_str().ok_or("header values must be strings")?;
        if v.len() > 2048 || v.chars().any(|c| c.is_control()) {
            return Err(format!("bad value for the '{name}' header"));
        }
        out.push((lower, v.to_string()));
    }
    Ok(out)
}

pub struct FetchedBytes {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
    pub final_url: String,
    pub cache_secs: Option<u64>,
}

/// Seconds a response may still be kept: `max-age` minus `Age` (what a CDN already used up), or 0 for
/// `no-store` / `no-cache` / `private`. `None` when the response says nothing about caching.
pub fn cache_lifetime(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    let cc = headers.get(reqwest::header::CACHE_CONTROL).and_then(|v| v.to_str().ok())?.to_ascii_lowercase();
    let mut max_age: Option<u64> = None;
    for part in cc.split(',').map(str::trim) {
        if matches!(part, "no-store" | "no-cache" | "private") {
            return Some(0);
        }
        if let Some(n) = part.strip_prefix("s-maxage=").or_else(|| part.strip_prefix("max-age=")) {
            if let Ok(n) = n.trim_matches('"').parse::<u64>() {
                max_age = Some(max_age.map_or(n, |m| m.min(n)));
            }
        }
    }
    let age = headers.get(reqwest::header::AGE).and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<u64>().ok()).unwrap_or(0);
    max_age.map(|m| m.saturating_sub(age))
}

/// GET `url` on behalf of a plugin allowed to reach `allowed` hosts, reading at most `max_bytes`.
pub async fn fetch_bytes(allowed: &[String], url: &str, headers: &[(String, String)], max_bytes: usize) -> Result<FetchedBytes, String> {
    let mut current = reqwest::Url::parse(url.trim()).map_err(|e| format!("bad URL: {e}"))?;
    for _ in 0..=MAX_REDIRECTS {
        let (host, port) = check_url(allowed, &current)?;
        let addr = resolve_public(&host, port).await?;
        let client = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            // Total ceiling for a big download on a slow link; a stalled transfer still dies after 30s
            // of silence (read_timeout).
            .timeout(std::time::Duration::from_secs(180))
            .read_timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .redirect(Policy::none())
            .resolve(&host, addr) // connect to exactly the address that was just checked
            .build()
            .map_err(|e| e.to_string())?;
        let mut req = client.get(current.clone());
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let mut resp = req.send().await.map_err(|e| format!("request failed: {}", e.without_url()))?;
        if resp.status().is_redirection() {
            let next = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or("redirect without a Location")?;
            // Re-checked against the allowlist at the top of the next hop.
            current = current.join(next).map_err(|e| format!("bad redirect: {e}"))?;
            continue;
        }
        let status = resp.status().as_u16();
        let cache_secs = cache_lifetime(resp.headers());
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        if resp.content_length().is_some_and(|n| n > max_bytes as u64) {
            return Err("response is too large".to_string());
        }
        let mut body: Vec<u8> = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|e| format!("read failed: {}", e.without_url()))? {
            if body.len() + chunk.len() > max_bytes {
                return Err("response is too large".to_string());
            }
            body.extend_from_slice(&chunk);
        }
        return Ok(FetchedBytes { status, content_type, body, final_url: current.to_string(), cache_secs });
    }
    Err("too many redirects".to_string())
}

/// GET `url` as text (lossy UTF-8) on behalf of a plugin allowed to reach `allowed` hosts.
pub async fn fetch(allowed: &[String], url: &str, headers: &[(String, String)]) -> Result<Fetched, String> {
    let f = fetch_bytes(allowed, url, headers, MAX_BODY_BYTES).await?;
    Ok(Fetched { status: f.status, content_type: f.content_type, body: String::from_utf8_lossy(&f.body).into_owned(), final_url: f.final_url, cache_secs: f.cache_secs })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }
    fn url(s: &str) -> reqwest::Url {
        reqwest::Url::parse(s).unwrap()
    }

    #[test]
    fn only_granted_https_hosts_pass() {
        let a = allowed(&["genius.com", "*.genius.com", "api.example.org:8443"]);
        assert!(check_url(&a, &url("https://genius.com/x")).is_ok());
        assert!(check_url(&a, &url("https://www.genius.com/x")).is_ok());
        assert!(check_url(&a, &url("https://API.EXAMPLE.ORG:8443/x")).is_ok());
        for bad in [
            "http://genius.com/x",                 // not https
            "https://genius.com@evil.test/x",      // credentials trick
            "https://user:pw@genius.com/x",
            "https://genius.com.evil.test/x",      // suffix trick
            "https://notgenius.com/x",
            "https://evil.test/genius.com",
            "https://genius.com:8443/x",           // wrong port
            "https://api.example.org/x",           // port must match the grant
            "https://127.0.0.1/x",                 // IPs never
            "https://[::1]/x",
            "file:///etc/passwd",
        ] {
            assert!(check_url(&a, &url(bad)).is_err(), "should refuse {bad}");
        }
        // A wildcard covers subdomains only, not the bare domain.
        let w = allowed(&["*.genius.com"]);
        assert!(check_url(&w, &url("https://www.genius.com/")).is_ok());
        assert!(check_url(&w, &url("https://genius.com/")).is_err());
        assert!(check_url(&[], &url("https://genius.com/")).is_err());
    }

    #[test]
    fn cache_lifetime_is_max_age_minus_age() {
        use reqwest::header::{HeaderMap, HeaderValue, AGE, CACHE_CONTROL};
        let h = |cc: Option<&str>, age: Option<&str>| {
            let mut m = HeaderMap::new();
            if let Some(c) = cc { m.insert(CACHE_CONTROL, HeaderValue::from_str(c).unwrap()); }
            if let Some(a) = age { m.insert(AGE, HeaderValue::from_str(a).unwrap()); }
            m
        };
        assert_eq!(cache_lifetime(&h(Some("public, max-age=3600"), None)), Some(3600));
        assert_eq!(cache_lifetime(&h(Some("public, max-age=3600"), Some("600"))), Some(3000));
        assert_eq!(cache_lifetime(&h(Some("max-age=60"), Some("500"))), Some(0), "already used up");
        assert_eq!(cache_lifetime(&h(Some("public, max-age=3600, s-maxage=120"), None)), Some(120), "the shorter wins");
        assert_eq!(cache_lifetime(&h(Some("private, max-age=3600"), None)), Some(0));
        assert_eq!(cache_lifetime(&h(Some("no-store"), None)), Some(0));
        assert_eq!(cache_lifetime(&h(Some("public"), None)), None);
        assert_eq!(cache_lifetime(&h(None, Some("5"))), None);
    }

    #[test]
    fn plugins_may_only_set_a_few_headers() {
        let ok = serde_json::json!({ "Authorization": "key123", "Accept": "application/json", "X-YVP-App-Key": "k" });
        assert_eq!(clean_headers(ok.as_object().unwrap()).unwrap().len(), 3);
        for bad in [
            serde_json::json!({ "Host": "evil.test" }),
            serde_json::json!({ "Cookie": "a=b" }),
            serde_json::json!({ "X-Forwarded-For": "1.2.3.4" }),
            serde_json::json!({ "Authorization": "a\r\nHost: x" }),
            serde_json::json!({ "Authorization": 5 }),
        ] {
            assert!(clean_headers(bad.as_object().unwrap()).is_err(), "{bad}");
        }
    }

    #[test]
    fn only_public_addresses_are_reachable() {
        for ip in ["127.0.0.1", "10.1.2.3", "172.16.0.1", "192.168.1.1", "169.254.169.254", "100.64.0.1", "0.0.0.0",
                   "224.0.0.1", "255.255.255.255", "::1", "::", "fe80::1", "fc00::1", "fd12::1", "::ffff:127.0.0.1", "::ffff:10.0.0.1"] {
            assert!(!is_public_ip(ip.parse().unwrap()), "{ip} must not be reachable");
        }
        for ip in ["8.8.8.8", "1.1.1.1", "93.184.216.34", "2606:4700:4700::1111"] {
            assert!(is_public_ip(ip.parse().unwrap()), "{ip} should be reachable");
        }
    }
}
