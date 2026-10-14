//! Command-line tools the host keeps installed and current for a plugin (docs/PLUGIN_PERMISSIONS.md).
//!
//! A plugin declares a tool in its manifest (a GitHub release, the asset per OS, the checksums
//! asset, the signature asset, and a public key that ships *inside the plugin folder*, so the
//! approval's digest pins it). The host downloads the release, checks the detached OpenPGP
//! signature of the checksums file against that key, checks the asset's SHA-256 against the signed
//! checksums, and only then installs it, under `<data>/tools/plugins/<plugin>/`. Nothing here runs a
//! tool; that is `plugin_process`, which can only run commands the manifest declares.
//!
//! This is the verified-release flow the built-in yt-dlp updater used, generalised: yt-dlp signs
//! `SHA2-256SUMS` with GPG (RSA-4096), so verification uses `gpgrv` (pure Rust, verify-only, no
//! `gpg` binary needed on Windows).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::core::plugin_permissions::ToolSpec;
use crate::network::updater::sha256_hex;

/// Largest release asset the host will download for a tool.
const MAX_ASSET_BYTES: usize = 300 * 1024 * 1024;

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("opensanctuary-tools/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(300))
        .read_timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))
}

/// This build's OS key in a `ToolSpec::assets` map.
pub fn os_key() -> &'static str {
    match std::env::consts::OS {
        "windows" => "windows",
        "macos" => "macos",
        _ => "linux",
    }
}

fn plugin_tools_dir(tools_dir: &Path, plugin_id: &str) -> PathBuf {
    tools_dir.join("plugins").join(plugin_id)
}

/// Where the host's own copy of a tool lives (it may not exist yet).
pub fn managed_binary_path(tools_dir: &Path, plugin_id: &str, tool_id: &str) -> PathBuf {
    plugin_tools_dir(tools_dir, plugin_id).join(format!("{tool_id}{}", std::env::consts::EXE_SUFFIX))
}

fn version_marker_path(tools_dir: &Path, plugin_id: &str, tool_id: &str) -> PathBuf {
    plugin_tools_dir(tools_dir, plugin_id).join(format!("{tool_id}.version"))
}

/// What to run for a tool: the host's managed copy if it has one, else the declared system
/// command (resolved through `PATH` by the OS), else nothing.
pub fn resolve_command(tools_dir: &Path, plugin_id: &str, spec: &ToolSpec) -> Option<PathBuf> {
    let managed = managed_binary_path(tools_dir, plugin_id, &spec.id);
    if managed.is_file() {
        return Some(managed);
    }
    spec.system_command.as_ref().map(PathBuf::from)
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolStatus {
    pub installed: bool,
    /// `managed` (the host's verified copy), `system` (found on PATH) or `none`.
    pub source: &'static str,
    pub version: Option<String>,
}

/// Whether a command can actually be run, and its version output.
async fn run_version(path: &Path, args: &[String]) -> Option<String> {
    let out = tokio::time::timeout(std::time::Duration::from_secs(15), Command::new(path).args(args).output())
        .await
        .ok()?
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().lines().next().unwrap_or("").to_string())
}

pub async fn status(tools_dir: &Path, plugin_id: &str, spec: &ToolSpec) -> ToolStatus {
    let managed = managed_binary_path(tools_dir, plugin_id, &spec.id);
    if managed.is_file() {
        let version = run_version(&managed, &spec.version_args).await;
        return ToolStatus { installed: version.is_some(), source: "managed", version };
    }
    if let Some(sys) = &spec.system_command {
        if let Some(version) = run_version(Path::new(sys), &spec.version_args).await {
            return ToolStatus { installed: true, source: "system", version: Some(version) };
        }
    }
    ToolStatus { installed: false, source: "none", version: None }
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    assets: Vec<GhAsset>,
}

fn verify_signature(key_armored: &str, sums: &[u8], sig: &[u8]) -> Result<(), String> {
    let mut keyring = gpgrv::Keyring::new();
    keyring
        .append_keys_from_armoured(std::io::Cursor::new(key_armored.as_bytes()))
        .map_err(|e| format!("the plugin's public key couldn't be loaded: {e:?}"))?;
    gpgrv::verify_detached(std::io::Cursor::new(sig), std::io::Cursor::new(sums), &keyring)
        .map_err(|e| format!("the checksums signature did not verify against the plugin's public key: {e:?}"))
}

/// The expected hash of `asset_name` in a "<sha256>  <name>" checksums file.
pub fn expected_hash(sums_text: &str, asset_name: &str) -> Option<String> {
    sums_text.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?.trim_start_matches('*');
        (name == asset_name && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())).then(|| hash.to_lowercase())
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateOutcome {
    pub updated: bool,
    pub error: bool,
    pub version: Option<String>,
    pub message: String,
}

/// Makes sure the tool is installed and current. With nothing managed and a usable system copy,
/// the system copy is left alone. Every failure is folded into the outcome's `message`.
pub async fn check_and_update(tools_dir: &Path, plugin_id: &str, spec: &ToolSpec, key_armored: &str) -> UpdateOutcome {
    match check_and_update_inner(tools_dir, plugin_id, spec, key_armored).await {
        Ok(o) => o,
        Err(message) => UpdateOutcome { updated: false, error: true, version: None, message },
    }
}

async fn check_and_update_inner(tools_dir: &Path, plugin_id: &str, spec: &ToolSpec, key_armored: &str) -> Result<UpdateOutcome, String> {
    let managed = managed_binary_path(tools_dir, plugin_id, &spec.id);
    let has_managed = managed.is_file();
    if !has_managed {
        if let Some(sys) = &spec.system_command {
            if run_version(Path::new(sys), &spec.version_args).await.is_some() {
                return Ok(UpdateOutcome { updated: false, error: false, version: None, message: format!("Using the {sys} already on this computer; not managing a copy") });
            }
        }
    }

    let http = client()?;
    let url = format!("https://api.github.com/repos/{}/releases/latest", spec.github);
    let release: GhRelease = http
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to reach GitHub: {e}"))?
        .error_for_status()
        .map_err(|e| format!("GitHub returned an error: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Failed to parse the release: {e}"))?;

    if has_managed {
        if let Ok(current) = std::fs::read_to_string(version_marker_path(tools_dir, plugin_id, &spec.id)) {
            if current.trim() == release.tag_name {
                return Ok(UpdateOutcome { updated: false, error: false, version: Some(release.tag_name), message: "Already up to date".into() });
            }
        }
    }

    let asset_name = spec.assets.get(os_key()).ok_or_else(|| format!("{} isn't available for {}", spec.id, os_key()))?;
    let download_prefix = format!("https://github.com/{}/releases/download/", spec.github);
    let find = |name: &str| {
        release
            .assets
            .iter()
            .find(|a| a.name == name && a.browser_download_url.starts_with(&download_prefix))
            .map(|a| a.browser_download_url.clone())
    };
    let asset_url = find(asset_name).ok_or_else(|| format!("release {} has no {asset_name} asset", release.tag_name))?;
    let sums_url = find(&spec.checksums).ok_or_else(|| format!("release has no {} asset", spec.checksums))?;
    let sig_url = find(&spec.signature.asset).ok_or_else(|| format!("release has no {} asset", spec.signature.asset))?;

    let fetch = |url: String| {
        let http = http.clone();
        async move {
            let mut resp = http
                .get(&url)
                .send()
                .await
                .map_err(|e| format!("Failed to download {url}: {e}"))?
                .error_for_status()
                .map_err(|e| format!("Download failed for {url}: {e}"))?;
            let mut body = Vec::new();
            while let Some(chunk) = resp.chunk().await.map_err(|e| format!("Failed to read {url}: {e}"))? {
                if body.len() + chunk.len() > MAX_ASSET_BYTES {
                    return Err(format!("{url} is too large"));
                }
                body.extend_from_slice(&chunk);
            }
            Ok::<Vec<u8>, String>(body)
        }
    };

    let sums = fetch(sums_url).await?;
    let sig = fetch(sig_url).await?;
    verify_signature(key_armored, &sums, &sig)?;
    let asset = fetch(asset_url).await?;
    let want = expected_hash(&String::from_utf8_lossy(&sums), asset_name)
        .ok_or_else(|| format!("{} has no entry for {asset_name}", spec.checksums))?;
    let got = sha256_hex(&asset);
    if got != want {
        return Err(format!("Downloaded {asset_name} hash mismatch: expected {want}, got {got}"));
    }

    let dir = plugin_tools_dir(tools_dir, plugin_id);
    tokio::fs::create_dir_all(&dir).await.map_err(|e| format!("Failed to create {dir:?}: {e}"))?;
    let tmp = dir.join(format!("{}.download-{}", spec.id, uuid::Uuid::new_v4()));
    tokio::fs::write(&tmp, &asset).await.map_err(|e| format!("Failed to write {tmp:?}: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
            .await
            .map_err(|e| format!("Failed to chmod {tmp:?}: {e}"))?;
    }
    tokio::fs::rename(&tmp, &managed).await.map_err(|e| format!("Failed to install {managed:?}: {e}"))?;
    tokio::fs::write(version_marker_path(tools_dir, plugin_id, &spec.id), &release.tag_name)
        .await
        .map_err(|e| format!("Failed to write the version marker: {e}"))?;
    Ok(UpdateOutcome { updated: true, error: false, version: Some(release.tag_name.clone()), message: format!("Installed {} {}", spec.id, release.tag_name) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn spec(system: Option<&str>) -> ToolSpec {
        ToolSpec {
            id: "demo".into(),
            github: "o/r".into(),
            assets: BTreeMap::from([("linux".into(), "demo_linux".into())]),
            checksums: "SUMS".into(),
            signature: crate::core::plugin_permissions::ToolSignature { asset: "SUMS.sig".into(), key_file: "key.asc".into() },
            system_command: system.map(|s| s.to_string()),
            version_args: vec!["--version".into()],
        }
    }

    #[test]
    fn managed_copy_wins_over_the_system_command_and_lives_per_plugin() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(resolve_command(dir.path(), "p1", &spec(Some("demo"))), Some(PathBuf::from("demo")));
        assert_eq!(resolve_command(dir.path(), "p1", &spec(None)), None);
        let managed = managed_binary_path(dir.path(), "p1", "demo");
        assert!(managed.starts_with(dir.path().join("plugins").join("p1")));
        std::fs::create_dir_all(managed.parent().unwrap()).unwrap();
        std::fs::write(&managed, b"x").unwrap();
        assert_eq!(resolve_command(dir.path(), "p1", &spec(Some("demo"))), Some(managed.clone()));
        // Another plugin's copy of a same-named tool is separate.
        assert_eq!(resolve_command(dir.path(), "p2", &spec(None)), None);
    }

    #[test]
    fn finds_the_signed_hash_for_exactly_the_named_asset() {
        let hash = "a".repeat(64);
        let sums = format!("{hash}  demo_linux\n{}  *demo.exe\nnothex  demo_mac\n", "b".repeat(64));
        assert_eq!(expected_hash(&sums, "demo_linux"), Some(hash.clone()));
        assert_eq!(expected_hash(&sums, "demo.exe"), Some("b".repeat(64)));
        assert_eq!(expected_hash(&sums, "demo_mac"), None, "a malformed hash is not a match");
        assert_eq!(expected_hash(&sums, "demo"), None, "no prefix matching");
    }

    #[test]
    fn a_tampered_checksums_file_or_bad_key_is_rejected() {
        assert!(verify_signature("not a key", b"sums", b"sig").is_err());
        let key = include_str!("../../web/plugins/ytdlp/yt-dlp-key.asc");
        assert!(verify_signature(key, b"tampered content", b"not a real signature").is_err());
    }

    #[test]
    fn the_pinned_ytdlp_key_loads() {
        let key = include_str!("../../web/plugins/ytdlp/yt-dlp-key.asc");
        assert!(key.starts_with("-----BEGIN PGP PUBLIC KEY BLOCK-----"));
        let mut keyring = gpgrv::Keyring::new();
        keyring.append_keys_from_armoured(std::io::Cursor::new(key.as_bytes())).expect("the pinned key should parse");
    }

    /// Real network: the real yt-dlp release verifies against the key shipped in the plugin folder, and
    /// a bootstrap install then an immediate re-check behave as expected. Run explicitly:
    /// `cargo test --release --lib live_ytdlp -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn live_ytdlp_release_verifies_installs_and_is_idempotent() {
        let key = include_str!("../../web/plugins/ytdlp/yt-dlp-key.asc");
        let manifest: crate::core::plugin_permissions::PluginManifest = serde_json::from_slice(
            &std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web/plugins/ytdlp/plugin.json")).unwrap(),
        )
        .unwrap();
        let mut tool = manifest.tools[0].clone();
        tool.system_command = None; // force a real bootstrap even if this machine has yt-dlp
        let dir = tempfile::tempdir().unwrap();
        let first = check_and_update(dir.path(), "ytdlp", &tool, key).await;
        assert!(first.updated, "{}", first.message);
        let st = status(dir.path(), "ytdlp", &tool).await;
        assert!(st.installed && st.source == "managed", "{st:?}");
        println!("installed yt-dlp {:?}", st.version);
        let second = check_and_update(dir.path(), "ytdlp", &tool, key).await;
        assert!(!second.updated && !second.error, "{}", second.message);
    }
}
