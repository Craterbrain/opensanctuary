//! Bible Provider Plugin System
//!
//! Defines the `BibleProvider` trait and built-in implementations:
//! - `ChurchAppsBibleProvider` – fetches from ChurchApps / Bolls.life APIs
//! - `GitHubRepoBibleProvider` – parses GitHub repositories in the
//!   `arron-taylor/bible-versions` format (Book → Chapter → Verse JSON)
//!
//! New providers can be added by implementing `BibleProvider` and registering
//! them in `BibleProviderRegistry::default()`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::core::models::{ScriptureItem, ScriptureVerse};
use crate::storage::freeshow_import::{
    normalize_language_name, FreeShowImporter, OnlineBibleCatalogItem,
};

// ─── Provider Trait ───────────────────────────────────────────────────────────

/// Metadata about a single downloadable Bible translation returned by a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BibleCatalogEntry {
    /// Stable ID used to identify this Bible in a download request.
    pub id: String,
    /// Human-readable name (e.g. "King James Version (Authorized 1769)").
    pub name: String,
    /// ISO-639 language name, normalised (e.g. "English", "Spanish").
    pub language: Option<String>,
    /// Short abbreviation (e.g. "KJV").
    pub abbreviation: Option<String>,
    /// Display label for the data source (e.g. "ChurchApps", "GitHub").
    pub source: String,
    /// Provider-specific opaque key used by `download_bible()`.
    /// For GitHub this is the raw file download URL.
    pub source_key: Option<String>,
    /// Name of the provider that owns this entry (matches `BibleProvider::provider_name()`).
    pub provider: String,
}

impl From<OnlineBibleCatalogItem> for BibleCatalogEntry {
    fn from(item: OnlineBibleCatalogItem) -> Self {
        Self {
            id: item.id.clone(),
            name: item.name,
            language: item.language,
            abbreviation: item.abbreviation,
            source: item.source.unwrap_or_else(|| "Unknown".to_string()),
            source_key: item.source_key,
            provider: "churchapps".to_string(),
        }
    }
}

/// Common error type for providers.
pub type ProviderError = Box<dyn std::error::Error + Send + Sync>;

#[async_trait]
pub trait BibleProvider: Send + Sync {
    /// Unique machine-readable name (e.g. "churchapps", "github").
    fn provider_name(&self) -> &'static str;

    /// Human-readable display label shown in the UI.
    fn display_name(&self) -> &'static str;

    /// Return all available Bible translations from this provider.
    async fn catalog(&self) -> Result<Vec<BibleCatalogEntry>, ProviderError>;

    /// Download a full Bible given a `BibleCatalogEntry::id` and optional
    /// provider-specific `source_key`.  Returns a flat list of chapter items
    /// (one `ScriptureItem` per chapter).
    async fn download_bible(
        &self,
        translation_id: &str,
        source_key: Option<&str>,
    ) -> Result<Vec<ScriptureItem>, ProviderError>;
}

// ─── Registry ────────────────────────────────────────────────────────────────

/// Central registry of all enabled `BibleProvider` plugins.
#[derive(Clone)]
pub struct BibleProviderRegistry {
    pub providers: Arc<Vec<Box<dyn BibleProvider>>>,
}

impl BibleProviderRegistry {
    /// Build the default registry with all built-in providers.
    pub fn new_default() -> Self {
        let providers: Vec<Box<dyn BibleProvider>> = vec![
            Box::new(ChurchAppsBibleProvider),
            Box::new(GitHubRepoBibleProvider::arron_taylor()),
        ];
        Self {
            providers: Arc::new(providers),
        }
    }

    /// Find a provider by its `provider_name`.
    pub fn get(&self, name: &str) -> Option<&dyn BibleProvider> {
        self.providers
            .iter()
            .find(|p| p.provider_name() == name)
            .map(|p| p.as_ref())
    }
}

// ─── ChurchApps / Bolls.life Provider ────────────────────────────────────────

/// Built-in provider backed by ChurchApps, Bolls.life, and the curated OpenLP
/// community list.  This is what the application previously called directly
/// from the route handlers; it is now isolated behind the `BibleProvider` trait
/// so the core binary ships cleanly.
pub struct ChurchAppsBibleProvider;

#[async_trait]
impl BibleProvider for ChurchAppsBibleProvider {
    fn provider_name(&self) -> &'static str {
        "churchapps"
    }

    fn display_name(&self) -> &'static str {
        "Online Bibles (ChurchApps / Bolls.life)"
    }

    async fn catalog(&self) -> Result<Vec<BibleCatalogEntry>, ProviderError> {
        let items = FreeShowImporter::fetch_churchapps_catalog().await?;
        Ok(items.into_iter().map(BibleCatalogEntry::from).collect())
    }

    async fn download_bible(
        &self,
        translation_id: &str,
        _source_key: Option<&str>,
    ) -> Result<Vec<ScriptureItem>, ProviderError> {
        FreeShowImporter::download_full_bible(translation_id).await
    }
}

// ─── GitHub Repo Provider ─────────────────────────────────────────────────────

/// Parses GitHub repositories structured like `arron-taylor/bible-versions`:
///
/// ```text
/// versions/
///   en/
///     KING JAMES BIBLE.json
///     WORLD ENGLISH BIBLE.json
///   ...
/// ```
///
/// Each JSON file is a nested map:
/// ```json
/// { "Genesis": { "1": { "1": "In the beginning…" } } }
/// ```
pub struct GitHubRepoBibleProvider {
    pub owner: String,
    pub repo: String,
    /// Path inside the repo containing language subdirectories.
    pub versions_path: String,
}

impl GitHubRepoBibleProvider {
    /// Returns a provider pre-configured for `arron-taylor/bible-versions`.
    pub fn arron_taylor() -> Self {
        Self {
            owner: "arron-taylor".to_string(),
            repo: "bible-versions".to_string(),
            versions_path: "versions".to_string(),
        }
    }

    /// Create a provider for any compatible repo.
    pub fn new(owner: impl Into<String>, repo: impl Into<String>) -> Self {
        Self {
            owner: owner.into(),
            repo: repo.into(),
            versions_path: "versions".to_string(),
        }
    }

    fn api_url(&self, path: &str) -> String {
        format!(
            "https://api.github.com/repos/{}/{}/contents/{}",
            self.owner, self.repo, path
        )
    }

    fn build_client() -> reqwest::Client {
        reqwest::Client::builder()
            .user_agent(
                "OpenSanctuary/1.0 (Church Presentation Engine; https://github.com/opensanctuary)",
            )
            .timeout(std::time::Duration::from_secs(60))
            .connect_timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    }
}

/// A single item in the GitHub Contents API response.
#[derive(Debug, Deserialize)]
struct GhContentsEntry {
    name: String,
    #[serde(rename = "type")]
    entry_type: String,
    path: String,
    download_url: Option<String>,
}

#[async_trait]
impl BibleProvider for GitHubRepoBibleProvider {
    fn provider_name(&self) -> &'static str {
        "github"
    }

    fn display_name(&self) -> &'static str {
        "GitHub Bible Repos"
    }

    async fn catalog(&self) -> Result<Vec<BibleCatalogEntry>, ProviderError> {
        let client = Self::build_client();
        let mut entries: Vec<BibleCatalogEntry> = Vec::new();

        // List top-level versions/ directory (contains language subdirs like en/, es/, …)
        let lang_dirs: Vec<GhContentsEntry> = client
            .get(self.api_url(&self.versions_path))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        for lang_dir in &lang_dirs {
            if lang_dir.entry_type != "dir" {
                continue;
            }
            let lang_code = &lang_dir.name; // e.g. "en"
            let lang_display = normalize_language_name(lang_code);

            // List files within that language dir; skip silently on error
            let files: Vec<GhContentsEntry> = match client
                .get(self.api_url(&lang_dir.path))
                .send()
                .await
            {
                Ok(resp) => resp.json().await.unwrap_or_default(),
                Err(_) => continue,
            };

            for file in &files {
                if file.entry_type != "file" {
                    continue;
                }
                if !file.name.ends_with(".json") {
                    continue;
                }

                // "KING JAMES BIBLE.json"  →  abbreviation "KJB", name "KING JAMES BIBLE"
                let stem = file.name.trim_end_matches(".json").trim().to_string();
                let abbr = make_abbreviation(&stem);
                let id = format!("gh_{}_{}", lang_code, abbr.to_lowercase());

                let download_url = file
                    .download_url
                    .clone()
                    .unwrap_or_else(|| {
                        format!(
                            "https://raw.githubusercontent.com/{}/{}/main/{}",
                            self.owner, self.repo, file.path
                        )
                    });

                entries.push(BibleCatalogEntry {
                    id: id.clone(),
                    name: title_case(&stem),
                    language: Some(lang_display.clone()),
                    abbreviation: Some(abbr),
                    source: format!(
                        "GitHub: {}/{}",
                        self.owner, self.repo
                    ),
                    source_key: Some(download_url),
                    provider: self.provider_name().to_string(),
                });
            }
        }

        Ok(entries)
    }

    async fn download_bible(
        &self,
        _translation_id: &str,
        source_key: Option<&str>,
    ) -> Result<Vec<ScriptureItem>, ProviderError> {
        let url = source_key.ok_or("No download URL provided for GitHub bible")?;
        // `source_key` round-trips through the client, so don't trust it to
        // still be a GitHub raw URL (SSRF into loopback/LAN otherwise).
        if !url.starts_with("https://raw.githubusercontent.com/") {
            return Err("GitHub Bible download URL must be on raw.githubusercontent.com".into());
        }

        let client = Self::build_client();
        let raw_json: serde_json::Value = client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        // Parse the JSON:  { "BookName": { "ChapterNum": { "VerseNum": "text" } } }
        let book_map = raw_json
            .as_object()
            .ok_or("GitHub Bible JSON: expected top-level object")?;

        // Derive version name from the URL filename
        let version_name = url
            .rsplit('/')
            .next()
            .and_then(|f| {
                let s = f.trim_end_matches(".json");
                if s.is_empty() { None } else { Some(make_abbreviation(s)) }
            })
            .unwrap_or_else(|| "BIBLE".to_string());

        let mut items: Vec<ScriptureItem> = Vec::new();

        for (book_name, chapters_val) in book_map {
            let chapters = match chapters_val.as_object() {
                Some(c) => c,
                None => continue,
            };

            // Sort chapters numerically
            let mut chapter_nums: Vec<u64> = chapters
                .keys()
                .filter_map(|k| k.parse::<u64>().ok())
                .collect();
            chapter_nums.sort_unstable();

            for chapter_num in chapter_nums {
                let ch_key = chapter_num.to_string();
                let verse_map = match chapters.get(&ch_key).and_then(|v| v.as_object()) {
                    Some(m) => m,
                    None => continue,
                };

                // Sort verses numerically
                let mut verse_nums: Vec<u64> = verse_map
                    .keys()
                    .filter_map(|k| k.parse::<u64>().ok())
                    .collect();
                verse_nums.sort_unstable();

                let mut verse_list: Vec<ScriptureVerse> = Vec::with_capacity(verse_nums.len());
                for v_num in &verse_nums {
                    let text = verse_map
                        .get(&v_num.to_string())
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    verse_list.push(ScriptureVerse {
                        verse_number: *v_num as u32,
                        text,
                    });
                }

                if verse_list.is_empty() {
                    continue;
                }

                let start_v = verse_list.first().map(|v| v.verse_number).unwrap_or(1);
                let end_v = verse_list.last().map(|v| v.verse_number).unwrap_or(start_v);

                let mut item = ScriptureItem::new(
                    book_name,
                    chapter_num as u32,
                    start_v,
                    end_v,
                    &version_name,
                );
                item.reference = format!("{} {}", book_name, chapter_num);
                item.id = format!(
                    "scrip_gh_{}_{}_{:03}",
                    version_name.to_lowercase(),
                    sanitize_id(book_name),
                    chapter_num
                );
                item.verses = verse_list;
                items.push(item);
            }
        }

        if items.is_empty() {
            return Err(
                format!("GitHub Bible at '{}' parsed zero chapters", url).into(),
            );
        }

        Ok(items)
    }
}

// ─── GitHub catalog with custom repo ─────────────────────────────────────────

/// Request body for browsing a user-supplied GitHub repo.
#[derive(Debug, Deserialize, Serialize)]
pub struct GitHubRepoCatalogRequest {
    /// Full repo slug, e.g. "arron-taylor/bible-versions".
    pub repo: String,
    /// Subdirectory inside the repo (default: "versions").
    #[serde(default = "default_versions_path")]
    pub versions_path: String,
}

fn default_versions_path() -> String {
    "versions".to_string()
}

impl GitHubRepoCatalogRequest {
    pub fn into_provider(self) -> Result<GitHubRepoBibleProvider, String> {
        let parts: Vec<&str> = self.repo.trim_matches('/').splitn(2, '/').collect();
        if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
            return Err(format!(
                "Invalid repo format '{}': expected 'owner/repo'",
                self.repo
            ));
        }
        Ok(GitHubRepoBibleProvider {
            owner: parts[0].to_string(),
            repo: parts[1].to_string(),
            versions_path: self.versions_path,
        })
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Turn "KING JAMES BIBLE" → "KJB"
fn make_abbreviation(s: &str) -> String {
    s.split_whitespace()
        .filter_map(|w| w.chars().next())
        .collect::<String>()
        .to_uppercase()
}

/// Turn "KING JAMES BIBLE" → "King James Bible"
fn title_case(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => {
                    f.to_uppercase().collect::<String>()
                        + &c.as_str().to_lowercase()
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Sanitize a book name so it can be used in a DB row ID.
fn sanitize_id(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() { c.to_lowercase().next().unwrap_or('_') } else { '_' })
        .collect()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_make_abbreviation() {
        assert_eq!(make_abbreviation("KING JAMES BIBLE"), "KJB");
        assert_eq!(make_abbreviation("WORLD ENGLISH BIBLE"), "WEB");
        assert_eq!(make_abbreviation("KJV"), "K");
    }

    #[test]
    fn test_title_case() {
        assert_eq!(title_case("KING JAMES BIBLE"), "King James Bible");
        assert_eq!(title_case("WORLD ENGLISH BIBLE"), "World English Bible");
    }

    #[test]
    fn test_gh_repo_request_parsing() {
        let req = GitHubRepoCatalogRequest {
            repo: "arron-taylor/bible-versions".to_string(),
            versions_path: "versions".to_string(),
        };
        let p = req.into_provider().unwrap();
        assert_eq!(p.owner, "arron-taylor");
        assert_eq!(p.repo, "bible-versions");
    }

    #[test]
    fn test_gh_repo_request_invalid() {
        let req = GitHubRepoCatalogRequest {
            repo: "invalid-no-slash".to_string(),
            versions_path: "versions".to_string(),
        };
        assert!(req.into_provider().is_err());
    }

    #[test]
    fn test_provider_registry_has_defaults() {
        let reg = BibleProviderRegistry::new_default();
        assert!(reg.get("churchapps").is_some());
        assert!(reg.get("github").is_some());
        assert!(reg.get("nonexistent").is_none());
    }
}
