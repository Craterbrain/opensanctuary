//! Keeps `yt-dlp` current -- a verified-release flow in the same spirit as
//! this app's own auto-updater (`src/network/updater.rs`, `docs/update.md`),
//! but for a third-party tool rather than this binary itself.
//!
//! Why this exists: Windows has no system package manager to install/update
//! `yt-dlp` via (unlike Linux's `.deb`, which lists it as a `Recommends`
//! alongside `ffmpeg` -- `Cargo.toml`'s `[package.metadata.deb]`), and
//! `yt-dlp` ships new releases very frequently specifically because sites
//! keep changing and breaking extraction -- a copy bundled once at install
//! time would likely stop working against real sites within weeks to
//! months. So instead of bundling a frozen copy, this module manages its
//! own copy under the data directory (`ResolvedPaths::tools_dir()`),
//! fetching the latest release on startup and periodically after.
//!
//! Verification: yt-dlp signs `SHA2-256SUMS` with GPG (RSA-4096), not
//! minisign, so this uses `gpgrv` (pure Rust, verify-only -- no `gpg`
//! binary needed, which matters on Windows) instead of `minisign-verify`.
//! The public key (`ytdlp_public_key.asc`) is pinned as a file in this repo
//! rather than fetched live from yt-dlp's own repo at check time the way
//! their documented verification instructions do (`curl .../public.key |
//! gpg --import`) -- fetching the key from the same place as the payload
//! means a compromised yt-dlp GitHub account could swap the key and a
//! malicious release's signature together. Pinning avoids that, the same
//! reasoning `RELEASE_PUBLIC_KEY_B64` is embedded rather than fetched in
//! this project's own updater.
//!
//! `ytdlp_public_key.asc` was fetched from
//! <https://raw.githubusercontent.com/yt-dlp/yt-dlp/master/public.key> and
//! confirmed at the time this was written: RSA 4096, fingerprint
//! `AC0C BBE6 848D 6A87 3464 AF4E 57CF 6593 3B5A 7581`, uid "Simon Sawicki
//! (yt-dlp signing key) <contact@grub4k.xyz>" -- verified against a real
//! `SHA2-256SUMS`/`SHA2-256SUMS.sig` pair from their actual latest release
//! at the time, not just imported on faith.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::network::updater::sha256_hex;

pub const YTDLP_REPO: &str = "yt-dlp/yt-dlp";

const YTDLP_PUBLIC_KEY_ARMORED: &str = include_str!("ytdlp_public_key.asc");

fn ytdlp_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("opensanctuary-ytdlp-updater/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(120))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))
}

fn platform_asset_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "yt-dlp.exe"
    } else if cfg!(target_os = "macos") {
        "yt-dlp_macos"
    } else {
        "yt-dlp_linux"
    }
}

/// Where this module's self-managed copy lives, if it has one. `tools_dir`
/// is `ResolvedPaths::tools_dir()` -- under the data directory, not the
/// install directory, since this is mutable state the app writes and
/// updates itself.
pub fn managed_binary_path(tools_dir: &Path) -> PathBuf {
    let filename = format!("yt-dlp{}", std::env::consts::EXE_SUFFIX);
    tools_dir.join(filename)
}

fn managed_version_marker_path(tools_dir: &Path) -> PathBuf {
    tools_dir.join("yt-dlp.version")
}

/// The command `ytdlp_import.rs` should actually run: the self-managed copy
/// if one exists (this module keeps it current -- see `check_and_update`
/// below), otherwise the bare command name for `Command::new` to resolve
/// via `PATH` (a system-installed copy, e.g. the `.deb`'s
/// `Recommends: yt-dlp` on Linux). Never overrides an existing system
/// install this module doesn't own.
pub fn resolve_ytdlp_command(tools_dir: &Path) -> PathBuf {
    let managed = managed_binary_path(tools_dir);
    if managed.is_file() {
        managed
    } else {
        PathBuf::from("yt-dlp")
    }
}

async fn system_ytdlp_available() -> bool {
    Command::new("yt-dlp").arg("--version").output().await.map(|o| o.status.success()).unwrap_or(false)
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

async fn fetch_latest_release(client: &reqwest::Client) -> Result<GhRelease, String> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", YTDLP_REPO);
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to reach GitHub: {}", e))?
        .error_for_status()
        .map_err(|e| format!("GitHub returned an error: {}", e))?;
    resp.json().await.map_err(|e| format!("Failed to parse release JSON: {}", e))
}

fn verify_sha2sums_signature(sums_bytes: &[u8], sig_bytes: &[u8]) -> Result<(), String> {
    let mut keyring = gpgrv::Keyring::new();
    keyring
        .append_keys_from_armoured(std::io::Cursor::new(YTDLP_PUBLIC_KEY_ARMORED.as_bytes()))
        .map_err(|e| format!("Failed to load embedded yt-dlp public key: {:?}", e))?;
    gpgrv::verify_detached(std::io::Cursor::new(sig_bytes), std::io::Cursor::new(sums_bytes), &keyring)
        .map_err(|e| format!("SHA2-256SUMS.sig did not verify against the embedded public key: {:?}", e))
}

/// Outcome of a single `check_and_update` run, for logging/surfacing --
/// never panics or fails the caller; any error is folded into `message`
/// with `error: true`, not an `Err` the caller has to handle specially.
/// `error` is what lets a UI distinguish "worth a toast" (`updated` or
/// `error`) from the routine, silent-majority case (already up to date, or
/// deferring to an existing system install).
#[derive(Debug, Clone, Serialize)]
pub struct YtdlpUpdateOutcome {
    pub updated: bool,
    pub error: bool,
    pub version: Option<String>,
    pub message: String,
}

/// Ensures `yt-dlp` is available and current: if nothing is self-managed or
/// on `PATH`, fetches the latest release now (first-run bootstrap); if a
/// self-managed copy already exists, checks for a newer release and
/// replaces it; if nothing is self-managed but something is already on
/// `PATH`, leaves it alone (respects an existing system install, e.g. the
/// `.deb`'s `Recommends: yt-dlp`, rather than shadowing it). Never blocks
/// the caller on failure -- every error becomes a `message`, not a panic or
/// an `Err` the caller has to handle specially.
pub async fn check_and_update(tools_dir: &Path) -> YtdlpUpdateOutcome {
    match check_and_update_inner(tools_dir).await {
        Ok(outcome) => outcome,
        Err(message) => YtdlpUpdateOutcome { updated: false, error: true, version: None, message },
    }
}

async fn check_and_update_inner(tools_dir: &Path) -> Result<YtdlpUpdateOutcome, String> {
    let managed_path = managed_binary_path(tools_dir);
    let has_managed = managed_path.is_file();

    if !has_managed && system_ytdlp_available().await {
        return Ok(YtdlpUpdateOutcome {
            updated: false,
            error: false,
            version: None,
            message: "Using a system-installed yt-dlp on PATH; not self-managing".into(),
        });
    }

    let client = ytdlp_client()?;
    let release = fetch_latest_release(&client).await?;

    if has_managed {
        if let Ok(current) = std::fs::read_to_string(managed_version_marker_path(tools_dir)) {
            if current.trim() == release.tag_name {
                return Ok(YtdlpUpdateOutcome {
                    updated: false,
                    error: false,
                    version: Some(release.tag_name),
                    message: "Already up to date".into(),
                });
            }
        }
    }

    download_and_install(tools_dir, &client, &release).await?;
    Ok(YtdlpUpdateOutcome {
        updated: true,
        error: false,
        version: Some(release.tag_name.clone()),
        message: format!("Installed yt-dlp {}", release.tag_name),
    })
}

async fn download_and_install(tools_dir: &Path, client: &reqwest::Client, release: &GhRelease) -> Result<(), String> {
    let asset_name = platform_asset_name();
    let find_url = |name: &str| release.assets.iter().find(|a| a.name == name).map(|a| a.browser_download_url.clone());

    let asset_url = find_url(asset_name).ok_or_else(|| format!("release {} has no {} asset", release.tag_name, asset_name))?;
    let sums_url = find_url("SHA2-256SUMS").ok_or("release has no SHA2-256SUMS asset")?;
    let sig_url = find_url("SHA2-256SUMS.sig").ok_or("release has no SHA2-256SUMS.sig asset")?;

    let fetch_bytes = |url: String| {
        let client = client.clone();
        async move {
            client
                .get(&url)
                .send()
                .await
                .map_err(|e| format!("Failed to download {}: {}", url, e))?
                .error_for_status()
                .map_err(|e| format!("Download failed for {}: {}", url, e))?
                .bytes()
                .await
                .map_err(|e| format!("Failed to read response body for {}: {}", url, e))
        }
    };

    let sums_bytes = fetch_bytes(sums_url).await?;
    let sig_bytes = fetch_bytes(sig_url).await?;
    verify_sha2sums_signature(&sums_bytes, &sig_bytes)?;

    let asset_bytes = fetch_bytes(asset_url).await?;
    let sums_text = String::from_utf8_lossy(&sums_bytes);
    let expected_hash = sums_text
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            let name = parts.next()?.trim_start_matches('*');
            (name == asset_name).then(|| hash.to_lowercase())
        })
        .ok_or_else(|| format!("SHA2-256SUMS has no entry for {}", asset_name))?;
    let actual_hash = sha256_hex(&asset_bytes);
    if actual_hash != expected_hash {
        return Err(format!("Downloaded {} hash mismatch: expected {}, got {}", asset_name, expected_hash, actual_hash));
    }

    tokio::fs::create_dir_all(tools_dir).await.map_err(|e| format!("Failed to create {:?}: {}", tools_dir, e))?;

    let final_path = managed_binary_path(tools_dir);
    let tmp_path = tools_dir.join(format!("yt-dlp.download-{}", uuid::Uuid::new_v4()));
    tokio::fs::write(&tmp_path, &asset_bytes).await.map_err(|e| format!("Failed to write {:?}: {}", tmp_path, e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = tokio::fs::metadata(&tmp_path)
            .await
            .map_err(|e| format!("Failed to stat {:?}: {}", tmp_path, e))?
            .permissions();
        perms.set_mode(0o755);
        tokio::fs::set_permissions(&tmp_path, perms)
            .await
            .map_err(|e| format!("Failed to chmod {:?}: {}", tmp_path, e))?;
    }

    tokio::fs::rename(&tmp_path, &final_path)
        .await
        .map_err(|e| format!("Failed to install {:?} -> {:?}: {}", tmp_path, final_path, e))?;
    tokio::fs::write(managed_version_marker_path(tools_dir), &release.tag_name)
        .await
        .map_err(|e| format!("Failed to write version marker: {}", e))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_public_key_is_well_formed_armor() {
        assert!(YTDLP_PUBLIC_KEY_ARMORED.starts_with("-----BEGIN PGP PUBLIC KEY BLOCK-----"));
        assert!(YTDLP_PUBLIC_KEY_ARMORED.trim_end().ends_with("-----END PGP PUBLIC KEY BLOCK-----"));
    }

    #[test]
    fn embedded_public_key_loads_into_a_keyring() {
        let mut keyring = gpgrv::Keyring::new();
        keyring
            .append_keys_from_armoured(std::io::Cursor::new(YTDLP_PUBLIC_KEY_ARMORED.as_bytes()))
            .expect("the pinned key should parse as a valid OpenPGP public key");
    }

    #[test]
    fn managed_path_resolution_prefers_managed_copy_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("library.db");
        let paths = crate::storage::paths::resolve(Some(db_path.to_str().unwrap()), None, None, None);
        let tools_dir = paths.tools_dir();

        // Nothing managed yet -- falls back to bare "yt-dlp".
        assert_eq!(resolve_ytdlp_command(&tools_dir), PathBuf::from("yt-dlp"));

        let managed = managed_binary_path(&tools_dir);
        std::fs::create_dir_all(managed.parent().unwrap()).unwrap();
        std::fs::write(&managed, b"fake yt-dlp binary").unwrap();
        assert_eq!(resolve_ytdlp_command(&tools_dir), managed);
    }

    #[tokio::test]
    async fn verify_rejects_a_tampered_sums_file() {
        // Can't easily get a *valid* signature without yt-dlp's private
        // key, but we can confirm a mismatched signature is rejected --
        // the real acceptance path is covered by the #[ignore]'d live test
        // below against yt-dlp's actual current release.
        let result = verify_sha2sums_signature(b"tampered content", b"not a real signature");
        assert!(result.is_err());
    }

    /// Real network, real yt-dlp release, real signature: fetches the
    /// actual latest SHA2-256SUMS/.sig from yt-dlp/yt-dlp and confirms the
    /// embedded public key verifies it. `#[ignore]`d for the same reason
    /// this project's other live-GitHub tests are (rate-limit pressure on
    /// routine `cargo test` runs) -- run explicitly with
    /// `cargo test --lib -- --ignored` when actually verifying this path.
    #[tokio::test]
    #[ignore]
    async fn real_sha2sums_from_latest_release_verifies() {
        let client = ytdlp_client().expect("build client");
        let release = fetch_latest_release(&client).await.expect("fetch latest yt-dlp release");

        let find_url = |name: &str| release.assets.iter().find(|a| a.name == name).map(|a| a.browser_download_url.clone());
        let sums_url = find_url("SHA2-256SUMS").expect("release should have SHA2-256SUMS");
        let sig_url = find_url("SHA2-256SUMS.sig").expect("release should have SHA2-256SUMS.sig");

        let sums_bytes = client.get(&sums_url).send().await.unwrap().bytes().await.unwrap();
        let sig_bytes = client.get(&sig_url).send().await.unwrap().bytes().await.unwrap();

        verify_sha2sums_signature(&sums_bytes, &sig_bytes).expect("real release signature should verify");
    }

    /// Real network, real download, real install: runs `check_and_update`
    /// twice against a scratch data directory -- the first run should
    /// bootstrap-fetch yt-dlp from nothing (no managed copy, and almost
    /// certainly no system yt-dlp on this machine's PATH during a `cargo
    /// test` run), verify it, install it, and report `updated: true`; the
    /// second run against the same directory should see the version marker
    /// already matches and report `updated: false` without re-downloading.
    /// `#[ignore]`d for the same live-network reason as the test above.
    #[tokio::test]
    #[ignore]
    async fn check_and_update_bootstraps_then_is_idempotent() {
        // This dev machine may well have a real system yt-dlp on PATH
        // already (that's a legitimate, respected case -- see
        // `check_and_update_inner`), so sanitize PATH down to just enough
        // to run the test (curl via reqwest doesn't need PATH, but the
        // process itself does for dynamic linking lookups on some
        // platforms) to force the bootstrap branch. Run this test alone
        // (`cargo test -- --ignored --test-threads=1 check_and_update`),
        // not concurrently with others -- PATH is process-global.
        let original_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", "/nonexistent-for-this-test");

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("library.db");
        let paths = crate::storage::paths::resolve(Some(db_path.to_str().unwrap()), None, None, None);
        let tools_dir = paths.tools_dir();

        let first = check_and_update(&tools_dir).await;
        std::env::set_var("PATH", &original_path);
        assert!(first.updated, "first run should bootstrap-fetch: {}", first.message);
        assert!(first.version.is_some());

        let managed = managed_binary_path(&tools_dir);
        assert!(managed.is_file(), "managed binary should exist after install");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&managed).unwrap().permissions().mode();
            assert!(mode & 0o111 != 0, "installed binary should be executable");
        }

        let second = check_and_update(&tools_dir).await;
        assert!(!second.updated, "second run against the same version should be a no-op: {}", second.message);
        assert_eq!(second.version, first.version);
    }
}
