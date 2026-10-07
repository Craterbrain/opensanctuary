use std::collections::HashMap;
use std::sync::RwLock;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct AssetId(pub String);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AssetKind {
    Image,
    Video,
    Audio,
    Svg,
    DynamicGradient,
}

#[derive(Debug, Clone)]
pub struct PrewarmedAsset {
    pub id: AssetId,
    pub path: PathBuf,
    pub kind: AssetKind,
    pub byte_data: Option<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub last_accessed_ms: u64,
}

pub struct AssetGraph {
    cache: RwLock<HashMap<AssetId, PrewarmedAsset>>,
    base_media_dir: PathBuf,
}

impl AssetGraph {
    pub fn new(base_media_dir: impl AsRef<Path>) -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            base_media_dir: base_media_dir.as_ref().to_path_buf(),
        }
    }

    pub fn resolve_uri(&self, uri: &str) -> PathBuf {
        if uri.starts_with("asset://") {
            let stripped = uri.trim_start_matches("asset://");
            self.base_media_dir.join(stripped)
        } else if uri.starts_with('/') || (uri.len() > 2 && uri.chars().nth(1) == Some(':')) {
            PathBuf::from(uri)
        } else {
            self.base_media_dir.join(uri)
        }
    }

    pub fn detect_kind(path: &Path) -> AssetKind {
        let ext = path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        match ext.as_str() {
            "mp4" | "mkv" | "mov" | "webm" | "avi" | "m4v" => AssetKind::Video,
            "mp3" | "wav" | "aac" | "flac" | "ogg" | "m4a" => AssetKind::Audio,
            "svg" => AssetKind::Svg,
            "jpg" | "jpeg" | "png" | "webp" | "bmp" | "gif" => AssetKind::Image,
            _ => AssetKind::Image,
        }
    }

    pub fn prewarm_asset(&self, uri: &str) -> Result<AssetId, String> {
        let path = self.resolve_uri(uri);
        let id = AssetId(uri.to_string());

        {
            let mut cache = self.cache.write().unwrap_or_else(|e| e.into_inner());
            if let Some(existing) = cache.get_mut(&id) {
                existing.last_accessed_ms = chrono::Utc::now().timestamp_millis() as u64;
                return Ok(id);
            }
        }

        let kind = Self::detect_kind(&path);
        let mut byte_data = None;
        let mut width = 0;
        let mut height = 0;

        if path.exists() && kind == AssetKind::Image {
            if let Ok(bytes) = std::fs::read(&path) {
                if let Ok(img) = image::load_from_memory(&bytes) {
                    width = img.width();
                    height = img.height();
                }
                byte_data = Some(bytes);
            }
        }

        let prewarmed = PrewarmedAsset {
            id: id.clone(),
            path,
            kind,
            byte_data,
            width,
            height,
            last_accessed_ms: chrono::Utc::now().timestamp_millis() as u64,
        };

        {
            let mut cache = self.cache.write().unwrap_or_else(|e| e.into_inner());
            cache.insert(id.clone(), prewarmed);
        }

        Ok(id)
    }

    pub fn get_asset(&self, id: &AssetId) -> Option<PrewarmedAsset> {
        let cache = self.cache.read().unwrap_or_else(|e| e.into_inner());
        cache.get(id).cloned()
    }

    pub fn evict_older_than(&self, age_ms: u64) {
        let now = chrono::Utc::now().timestamp_millis() as u64;
        let mut cache = self.cache.write().unwrap_or_else(|e| e.into_inner());
        cache.retain(|_, v| now.saturating_sub(v.last_accessed_ms) < age_ms);
    }
}
