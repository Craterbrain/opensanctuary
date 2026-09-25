//! Online stock-image search providers for the Media resource tab.
//!
//! Defines the `MediaSearchProvider` trait and two built-in implementations:
//! - `PexelsProvider` – https://www.pexels.com/api/documentation/
//! - `PixabayProvider` – https://pixabay.com/api/docs/
//!
//! New providers can be added by implementing `MediaSearchProvider` and
//! registering them in `MediaSearchRegistry::new_default()`. Mirrors the
//! `BibleProvider`/`BibleProviderRegistry` pattern in `bible_providers.rs`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type ProviderError = Box<dyn std::error::Error + Send + Sync>;

/// A single image result, shaped for direct thumbnail-grid rendering on the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaSearchResult {
    /// Namespaced by provider (e.g. "pexels_12345") so results from different
    /// providers never collide.
    pub id: String,
    pub provider: String,
    pub thumbnail_url: String,
    pub full_url: String,
    pub width: u32,
    pub height: u32,
    /// Required attribution per each provider's API terms of use.
    #[serde(default)]
    pub photographer: Option<String>,
    #[serde(default)]
    pub photographer_url: Option<String>,
    #[serde(default)]
    pub source_page_url: Option<String>,
    /// Descriptive keywords for the image — Pixabay's own `tags` field, or
    /// Pexels' `alt` text used the same way. Neither provider's search
    /// endpoint is title-only (both match broadly against their own
    /// tag/description index already), but this data was previously dropped
    /// entirely on our side — surfaced here so it's visible in the result
    /// preview and carried into the imported MediaItem's name, so a *local*
    /// re-search of the Media library afterward can find it by tag too.
    #[serde(default)]
    pub tags: Option<String>,
}

#[async_trait]
pub trait MediaSearchProvider: Send + Sync {
    fn provider_name(&self) -> &'static str;
    fn display_name(&self) -> &'static str;

    /// Searches this provider's image catalog. `api_key` is passed in fresh on
    /// every call (loaded from settings by the caller) rather than cached, so
    /// a key change in the Options modal takes effect immediately.
    async fn search(&self, query: &str, api_key: &str) -> Result<Vec<MediaSearchResult>, ProviderError>;
}

fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("OpenSanctuary/1.0 (Church Presentation Engine; https://github.com/opensanctuary)")
        .timeout(std::time::Duration::from_secs(20))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

// ─── Pexels ─────────────────────────────────────────────────────────────────

pub struct PexelsProvider;

#[derive(Debug, Deserialize)]
struct PexelsResponse {
    photos: Vec<PexelsPhoto>,
}

#[derive(Debug, Deserialize)]
struct PexelsPhoto {
    id: i64,
    width: u32,
    height: u32,
    url: String,
    photographer: String,
    photographer_url: String,
    #[serde(default)]
    alt: Option<String>,
    src: PexelsPhotoSrc,
}

#[derive(Debug, Deserialize)]
struct PexelsPhotoSrc {
    medium: String,
    large2x: String,
}

#[async_trait]
impl MediaSearchProvider for PexelsProvider {
    fn provider_name(&self) -> &'static str {
        "pexels"
    }
    fn display_name(&self) -> &'static str {
        "Pexels"
    }

    async fn search(&self, query: &str, api_key: &str) -> Result<Vec<MediaSearchResult>, ProviderError> {
        if api_key.trim().is_empty() {
            return Err("Pexels API key is not configured".into());
        }
        let client = build_client();
        let resp: PexelsResponse = client
            .get("https://api.pexels.com/v1/search")
            .header("Authorization", api_key)
            .query(&[("query", query), ("per_page", "24")])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        Ok(resp
            .photos
            .into_iter()
            .map(|p| MediaSearchResult {
                id: format!("pexels_{}", p.id),
                provider: "pexels".to_string(),
                thumbnail_url: p.src.medium,
                full_url: p.src.large2x,
                width: p.width,
                height: p.height,
                photographer: Some(p.photographer),
                photographer_url: Some(p.photographer_url),
                source_page_url: Some(p.url),
                tags: p.alt.filter(|a| !a.trim().is_empty()),
            })
            .collect())
    }
}

// ─── Pixabay ────────────────────────────────────────────────────────────────

pub struct PixabayProvider;

#[derive(Debug, Deserialize)]
struct PixabayResponse {
    hits: Vec<PixabayHit>,
}

#[derive(Debug, Deserialize)]
struct PixabayHit {
    id: i64,
    #[serde(rename = "previewURL")]
    preview_url: String,
    #[serde(rename = "largeImageURL")]
    large_image_url: String,
    #[serde(rename = "imageWidth")]
    image_width: u32,
    #[serde(rename = "imageHeight")]
    image_height: u32,
    user: String,
    #[serde(rename = "pageURL")]
    page_url: String,
    #[serde(default)]
    tags: Option<String>,
}

#[async_trait]
impl MediaSearchProvider for PixabayProvider {
    fn provider_name(&self) -> &'static str {
        "pixabay"
    }
    fn display_name(&self) -> &'static str {
        "Pixabay"
    }

    async fn search(&self, query: &str, api_key: &str) -> Result<Vec<MediaSearchResult>, ProviderError> {
        if api_key.trim().is_empty() {
            return Err("Pixabay API key is not configured".into());
        }
        let client = build_client();
        let resp: PixabayResponse = client
            .get("https://pixabay.com/api/")
            .query(&[
                ("key", api_key),
                ("q", query),
                ("image_type", "photo"),
                ("per_page", "24"),
                ("safesearch", "true"),
            ])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        Ok(resp
            .hits
            .into_iter()
            .map(|h| MediaSearchResult {
                id: format!("pixabay_{}", h.id),
                provider: "pixabay".to_string(),
                thumbnail_url: h.preview_url,
                full_url: h.large_image_url,
                width: h.image_width,
                height: h.image_height,
                photographer: Some(h.user),
                photographer_url: None,
                source_page_url: Some(h.page_url),
                tags: h.tags.filter(|t| !t.trim().is_empty()),
            })
            .collect())
    }
}

// ─── Registry ───────────────────────────────────────────────────────────────

pub struct MediaSearchRegistry {
    pub providers: Vec<Box<dyn MediaSearchProvider>>,
}

impl MediaSearchRegistry {
    pub fn new_default() -> Self {
        Self {
            providers: vec![Box::new(PexelsProvider), Box::new(PixabayProvider)],
        }
    }

    /// Registers an additional media search provider dynamically (e.g. from a plugin or extension).
    pub fn register<P: MediaSearchProvider + 'static>(&mut self, provider: P) {
        self.providers.push(Box::new(provider));
    }

    /// Registers a boxed media search provider dynamically.
    pub fn register_boxed(&mut self, provider: Box<dyn MediaSearchProvider>) {
        self.providers.push(provider);
    }

    /// Searches every registered provider that has a non-empty API key in
    /// `api_keys` (keyed by `provider_name()`). Individual provider failures
    /// (bad key, network error, rate limit) are logged and skipped rather than
    /// failing the whole search.
    pub async fn search_all(&self, query: &str, api_keys: &HashMap<String, String>) -> Vec<MediaSearchResult> {
        let mut all = Vec::new();
        for provider in &self.providers {
            let key = api_keys.get(provider.provider_name()).cloned().unwrap_or_default();
            if key.trim().is_empty() {
                continue;
            }
            match provider.search(query, &key).await {
                Ok(mut results) => all.append(&mut results),
                Err(e) => tracing::warn!("MediaSearchProvider '{}' search error: {}", provider.provider_name(), e),
            }
        }
        all
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_has_defaults() {
        let reg = MediaSearchRegistry::new_default();
        let names: Vec<&str> = reg.providers.iter().map(|p| p.provider_name()).collect();
        assert!(names.contains(&"pexels"));
        assert!(names.contains(&"pixabay"));
    }

    struct FakeOkProvider;
    #[async_trait]
    impl MediaSearchProvider for FakeOkProvider {
        fn provider_name(&self) -> &'static str { "fake_ok" }
        fn display_name(&self) -> &'static str { "Fake OK" }
        async fn search(&self, query: &str, api_key: &str) -> Result<Vec<MediaSearchResult>, ProviderError> {
            if api_key.is_empty() {
                return Err("missing key".into());
            }
            Ok(vec![MediaSearchResult {
                id: format!("fake_ok_{}", query),
                provider: "fake_ok".to_string(),
                thumbnail_url: "https://example.com/thumb.jpg".to_string(),
                full_url: "https://example.com/full.jpg".to_string(),
                width: 100,
                height: 100,
                photographer: None,
                photographer_url: None,
                source_page_url: None,
                tags: None,
            }])
        }
    }

    struct FakeFailingProvider;
    #[async_trait]
    impl MediaSearchProvider for FakeFailingProvider {
        fn provider_name(&self) -> &'static str { "fake_fail" }
        fn display_name(&self) -> &'static str { "Fake Failing" }
        async fn search(&self, _query: &str, _api_key: &str) -> Result<Vec<MediaSearchResult>, ProviderError> {
            Err("simulated upstream failure".into())
        }
    }

    #[tokio::test]
    async fn test_search_all_aggregates_and_tolerates_provider_failure() {
        let reg = MediaSearchRegistry {
            providers: vec![Box::new(FakeOkProvider), Box::new(FakeFailingProvider)],
        };
        let mut keys = HashMap::new();
        keys.insert("fake_ok".to_string(), "a-key".to_string());
        keys.insert("fake_fail".to_string(), "a-key".to_string());

        let results = reg.search_all("sunrise", &keys).await;
        assert_eq!(results.len(), 1, "the failing provider must not suppress the working one's results");
        assert_eq!(results[0].id, "fake_ok_sunrise");
    }

    #[tokio::test]
    async fn test_search_all_skips_providers_with_no_configured_key() {
        let reg = MediaSearchRegistry {
            providers: vec![Box::new(FakeOkProvider)],
        };
        let results = reg.search_all("sunrise", &HashMap::new()).await;
        assert!(results.is_empty(), "a provider with no key configured must be skipped, not errored");
    }

    #[test]
    fn test_pexels_response_captures_alt_as_tags() {
        let json = r#"{
            "photos": [
                {
                    "id": 12345,
                    "width": 4000,
                    "height": 6000,
                    "url": "https://www.pexels.com/photo/example-12345/",
                    "photographer": "Jane Doe",
                    "photographer_url": "https://www.pexels.com/@jane-doe",
                    "alt": "Orange sunset over the ocean",
                    "src": { "medium": "https://images.pexels.com/photos/12345/medium.jpeg", "large2x": "https://images.pexels.com/photos/12345/large2x.jpeg" }
                },
                {
                    "id": 67890,
                    "width": 100,
                    "height": 100,
                    "url": "https://www.pexels.com/photo/example-67890/",
                    "photographer": "No Alt Person",
                    "photographer_url": "https://www.pexels.com/@no-alt",
                    "src": { "medium": "https://images.pexels.com/photos/67890/medium.jpeg", "large2x": "https://images.pexels.com/photos/67890/large2x.jpeg" }
                }
            ]
        }"#;
        let resp: PexelsResponse = serde_json::from_str(json).expect("valid Pexels response shape must deserialize");
        assert_eq!(resp.photos.len(), 2);
        assert_eq!(resp.photos[0].alt.as_deref(), Some("Orange sunset over the ocean"));
        assert_eq!(resp.photos[1].alt, None, "alt is optional — Pexels doesn't always return it");
    }

    #[test]
    fn test_pixabay_response_captures_tags() {
        let json = r#"{
            "hits": [
                {
                    "id": 591576,
                    "previewURL": "https://cdn.pixabay.com/photo/preview.jpg",
                    "largeImageURL": "https://cdn.pixabay.com/photo/large.jpg",
                    "imageWidth": 4000,
                    "imageHeight": 3000,
                    "user": "JillWellington",
                    "pageURL": "https://pixabay.com/photos/woman-591576/",
                    "tags": "woman, portrait, model"
                }
            ]
        }"#;
        let resp: PixabayResponse = serde_json::from_str(json).expect("valid Pixabay response shape must deserialize");
        assert_eq!(resp.hits.len(), 1);
        assert_eq!(resp.hits[0].tags.as_deref(), Some("woman, portrait, model"));
    }
}
