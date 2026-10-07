//! Release update verification (docs/update.md).
//!
//! This is the verification half only -- checking a signature against an
//! embedded public key. It deliberately does *not* pull in the `minisign`
//! crate (dev-only, see `examples/sign_release.rs`); the app only ever
//! verifies, never signs, so `minisign-verify` (a small, zero-dependency
//! crate by minisign's own author) is all it needs.
//!
//! There are two flows: the manual one (an operator downloads a release
//! themselves and picks the file in Settings -> About -> "Install Update
//! from File" -- `check_local_file_against_sibling_checksums` verifies it
//! against a `checksums.txt`/`checksums.txt.minisig` sitting next to it, if
//! the operator downloaded those too, and `open_with_system_handler` hands
//! off to the OS's own installer so this app never needs elevated privileges
//! itself), and the automated one (`check_latest_release` /
//! `download_and_verify_release` -- polls a GitHub Releases repo, compares
//! `tag_name` against this build's own version, and on request downloads +
//! verifies the matching asset the same way the manual flow does, reusing
//! `verify_signature`/`sha256_hex` rather than duplicating that logic).

use std::fs;
use std::path::{Path, PathBuf};

use minisign_verify::{PublicKey, Signature};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// The GitHub repo the automated check flow polls. **This is currently a
/// public test/release-mirror repo, not the real project repo**
/// (`Craterbrain/opensanctuary`, still private as of this writing -- an
/// unauthenticated `GET .../releases/latest`, the only safe way for a
/// distributed binary to check for updates, 404s against a private repo;
/// see "Blocker to resolve first" in `docs/update.md`). Swapping this one
/// constant is the entire migration once the real repo goes public, or a
/// dedicated public release-mirror repo exists -- nothing else in this
/// module or its callers needs to change.
pub const RELEASES_REPO: &str = "Craterbrain/opensanctuary-release-testing";

/// The release-signing public key (minisign key ID 80DBAA0356AE9867; the
/// matching encrypted secret key is kept offline, never in this repo). Releases
/// previously signed with the old throwaway test key will no longer verify.
/// `RELEASES_REPO` above is still the test mirror and must be swapped to the
/// real release repo before this verifies a release handed to real users.
pub const RELEASE_PUBLIC_KEY_B64: &str = "RWRnmK5WA6rbgDlBqOj4+R+5vQK/wGsexQQ2NsjgUPiqkC5iUUZMmk42";

/// Verifies `data` was signed by the holder of the private key matching
/// `public_key_b64`, using a detached minisign signature (the contents of a
/// `.minisig` file, e.g. `checksums.txt.minisig`).
///
/// `public_key_b64` is the base64 public key string minisign prints/writes
/// (the second line of a `.pub` file, or `PublicKey::to_base64()`'s output)
/// -- not the whole `.pub` file with its comment header.
pub fn verify_signature(data: &[u8], signature_text: &str, public_key_b64: &str) -> Result<(), String> {
    let public_key = PublicKey::from_base64(public_key_b64)
        .map_err(|e| format!("Invalid public key: {}", e))?;
    let signature = Signature::decode(signature_text)
        .map_err(|e| format!("Invalid signature: {}", e))?;
    public_key
        .verify(data, &signature, false)
        .map_err(|e| format!("Signature verification failed: {}", e))
}

pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ChecksumCheck {
    /// The file's hash matches its entry in a sibling `checksums.txt`,
    /// and that manifest's `.minisig` verified against `public_key_b64`.
    VerifiedAndSigned,
    /// The hash matched a sibling `checksums.txt`, but no `.minisig` sat
    /// next to it to check the manifest itself hasn't been tampered with.
    ChecksumMatchedUnsigned,
    /// A sibling `checksums.txt` exists and has a `.minisig`, but the
    /// signature didn't verify -- the manifest itself isn't trustworthy,
    /// so its hash entries aren't either. Distinct from a hash mismatch.
    ChecksumFileUnverified { reason: String },
    /// A sibling `checksums.txt` exists but the file's hash doesn't match
    /// its entry.
    Mismatch { expected: String, actual: String },
    /// A sibling `checksums.txt` exists but has no entry for this filename.
    NoEntryForFile,
    /// No `checksums.txt` sits next to the picked file at all -- common
    /// during alpha (see module docs); not itself an error.
    NoChecksumFile,
}

/// Looks for `checksums.txt` (and optionally `checksums.txt.minisig`) in the
/// same directory as `file_path`, and checks `file_path`'s own hash against
/// it. `public_key_b64` is only used if a `.minisig` is actually present.
///
/// This is deliberately permissive, not a hard gate: during alpha, releases
/// aren't guaranteed to ship a signed manifest yet (see module docs), and an
/// operator picking a file in this flow already chose to trust it by
/// downloading it themselves. Settings/UI decide what to do with the result
/// (e.g. warn loudly on `Mismatch` and require confirmation, proceed quietly
/// on `NoChecksumFile`) -- this function only reports what it found.
pub fn check_local_file_against_sibling_checksums(file_path: &Path, public_key_b64: &str) -> ChecksumCheck {
    let Some(dir) = file_path.parent() else {
        return ChecksumCheck::NoChecksumFile;
    };
    let checksums_path = dir.join("checksums.txt");
    let Ok(checksums_text) = fs::read_to_string(&checksums_path) else {
        return ChecksumCheck::NoChecksumFile;
    };

    let file_name = file_path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let expected_hash = checksums_text.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?.trim_start_matches('*');
        (name == file_name).then(|| hash.to_lowercase())
    });
    let Some(expected_hash) = expected_hash else {
        return ChecksumCheck::NoEntryForFile;
    };

    let sig_path = dir.join("checksums.txt.minisig");
    let signed = if let Ok(sig_text) = fs::read_to_string(&sig_path) {
        match verify_signature(checksums_text.as_bytes(), &sig_text, public_key_b64) {
            Ok(()) => true,
            Err(e) => return ChecksumCheck::ChecksumFileUnverified { reason: e },
        }
    } else {
        false
    };

    let Ok(file_bytes) = fs::read(file_path) else {
        return ChecksumCheck::Mismatch { expected: expected_hash, actual: "(could not read file)".to_string() };
    };
    let actual_hash = sha256_hex(&file_bytes);

    if actual_hash != expected_hash {
        return ChecksumCheck::Mismatch { expected: expected_hash, actual: actual_hash };
    }

    if signed {
        ChecksumCheck::VerifiedAndSigned
    } else {
        ChecksumCheck::ChecksumMatchedUnsigned
    }
}

/// Hands `file_path` off to the OS's own default handler for it (e.g. `apt`/
/// `dpkg`'s GUI installer for a `.deb` via `xdg-open`, Explorer's installer
/// flow for a Windows `.exe`). This app never runs a privileged install
/// itself -- the OS's own installer already knows how to prompt for that
/// trust boundary correctly on each platform, and doing it ourselves would
/// mean either running as root all the time or reimplementing a polkit/UAC
/// elevation flow badly.
#[cfg(feature = "open")]
pub fn open_with_system_handler(file_path: &Path) -> Result<(), String> {
    open::that(file_path).map_err(|e| format!("Could not open {}: {}", file_path.display(), e))
}

/// One release, as reported by `GET /repos/{RELEASES_REPO}/releases/latest`.
/// `download_url` points at the platform installer asset (`.deb` on Linux
/// today -- see `download_and_verify_release`'s doc comment for why other
/// platforms aren't wired to auto-apply yet); `checksums_url`/
/// `checksums_sig_url` point at the manifest and its detached signature,
/// the same two files the manual flow expects sitting next to a
/// hand-downloaded file.
#[derive(Debug, Clone, Serialize)]
pub struct ReleaseInfo {
    pub version: String,
    pub asset_name: String,
    pub download_url: String,
    pub checksums_url: String,
    pub checksums_sig_url: String,
}

/// Cached result of the last automated check (startup, or a manual "Check
/// for Updates" click) -- `AppState::update_status` holds one of these so
/// `GET /api/updates/status` can answer instantly without hitting GitHub on
/// every poll.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateCheckResult {
    pub checked_at_ms: i64,
    pub current_version: String,
    pub latest: Option<ReleaseInfo>,
    pub error: Option<String>,
}

fn releases_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        // The GitHub Releases API 403s an unauthenticated request with no
        // User-Agent at all -- unlike this crate's other reqwest callers
        // (genius_import.rs, freeshow_import.rs), which set one to look
        // like a browser, this just needs to identify the app honestly.
        .user_agent(concat!("opensanctuary-updater/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(15))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))
}

/// The asset name pattern each platform's automated download looks for.
/// Only Linux's `.deb` is actually wired to `download_and_verify_release`
/// today (see that fn's doc comment) -- this still resolves a Windows/macOS
/// asset name so `check_latest_release`'s result is meaningful cross-platform
/// for display purposes (e.g. "v0.3.0 is available" in Settings) even before
/// auto-apply exists for those platforms.
fn platform_asset_suffix() -> &'static str {
    if cfg!(target_os = "windows") {
        "setup-x64.exe"
    } else if cfg!(target_os = "macos") {
        ".dmg"
    } else {
        ".deb"
    }
}

/// Polls `RELEASES_REPO`'s latest GitHub release and compares its `tag_name`
/// against this build's own version (`CARGO_PKG_VERSION`) using real semver
/// ordering (release tags carry `-alpha`/`-beta`/`-test` suffixes that need
/// proper prerelease ordering, not lexical comparison -- see "Check flow" in
/// `docs/update.md`). Returns `Ok(None)` when already up to date, `Ok(Some(..))`
/// when a newer release exists.
pub async fn check_latest_release() -> Result<Option<ReleaseInfo>, String> {
    let release = fetch_latest_release_json().await?;
    let tag_name = release.get("tag_name").and_then(|v| v.as_str()).ok_or("Release response had no tag_name")?;
    if !is_version_newer(tag_name, env!("CARGO_PKG_VERSION"))? {
        return Ok(None);
    }

    Ok(Some(release_info_from_json(&release)?))
}

/// True if `latest_tag` (a GitHub release `tag_name`, optionally `v`-prefixed)
/// is a newer version than `current` under real semver ordering -- pulled
/// out of `check_latest_release` so it's directly unit-testable without a
/// network call. Real semver ordering matters here, not lexical comparison:
/// a prerelease sorts *below* its own release (`0.2.0-test` < `0.2.0`), so a
/// tag like `v0.2.0-test` correctly does NOT count as newer than a running
/// `0.2.0` build -- that's the same rule `docs/update.md`'s "Check flow"
/// section calls for.
fn is_version_newer(latest_tag: &str, current: &str) -> Result<bool, String> {
    let latest_str = latest_tag.trim_start_matches('v');
    let latest = semver::Version::parse(latest_str)
        .map_err(|e| format!("Could not parse release tag '{}' as semver: {}", latest_tag, e))?;
    let current = semver::Version::parse(current).map_err(|e| format!("Could not parse '{}' as semver: {}", current, e))?;
    Ok(latest > current)
}

/// Extracts a `ReleaseInfo` (asset name/URL for this platform, plus the
/// checksum manifest and its signature) out of a raw GitHub release JSON
/// object -- shared by `check_latest_release` (after it's decided the
/// release is actually newer) and directly by tests that need a real
/// `ReleaseInfo` for a release that isn't "newer" than this dev build's own
/// version (e.g. the test repo's own `v0.2.0-test` tag).
fn release_info_from_json(release: &serde_json::Value) -> Result<ReleaseInfo, String> {
    let tag_name = release.get("tag_name").and_then(|v| v.as_str()).ok_or("Release response had no tag_name")?;
    let version = tag_name.trim_start_matches('v').to_string();

    let assets = release.get("assets").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let find_asset = |predicate: &dyn Fn(&str) -> bool| -> Option<(String, String)> {
        assets.iter().find_map(|a| {
            let name = a.get("name")?.as_str()?;
            if predicate(name) {
                let url = a.get("browser_download_url")?.as_str()?;
                Some((name.to_string(), url.to_string()))
            } else {
                None
            }
        })
    };

    let suffix = platform_asset_suffix();
    let (asset_name, download_url) = find_asset(&|name| name.ends_with(suffix))
        .ok_or_else(|| format!("Release {} has no asset matching '{}'", tag_name, suffix))?;
    let (_, checksums_url) = find_asset(&|name| name == "checksums.txt").ok_or("Release has no checksums.txt asset")?;
    let (_, checksums_sig_url) =
        find_asset(&|name| name == "checksums.txt.minisig").ok_or("Release has no checksums.txt.minisig asset")?;

    Ok(ReleaseInfo { version, asset_name, download_url, checksums_url, checksums_sig_url })
}

/// Fetches `RELEASES_REPO`'s raw latest-release JSON, with no version
/// comparison -- the shared fetch step behind `check_latest_release`, split
/// out so tests can build a real `ReleaseInfo` from the actual latest
/// release regardless of whether it happens to be "newer" than the test
/// build's own version.
async fn fetch_latest_release_json() -> Result<serde_json::Value, String> {
    let client = releases_client()?;
    let url = format!("https://api.github.com/repos/{}/releases/latest", RELEASES_REPO);
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to reach GitHub: {}", e))?
        .error_for_status()
        .map_err(|e| format!("GitHub returned an error: {}", e))?;
    resp.json().await.map_err(|e| format!("Failed to parse release JSON: {}", e))
}

/// True if `url` is an https download from `RELEASES_REPO`'s GitHub releases.
fn is_release_download_url(url: &str) -> bool {
    let prefix = format!("https://github.com/{}/releases/download/", RELEASES_REPO);
    url.starts_with(&prefix) && !url[prefix.len()..].contains("..")
}

/// Whether `asset_name` embeds `version`'s `major.minor.patch`. Platforms
/// whose installer name is unversioned (Windows `...-setup-x64.exe`) can't be
/// bound this way and are not auto-applied (see `download_and_verify_release`),
/// so they're rejected rather than silently skipped.
fn asset_name_matches_version(asset_name: &str, version: &str, suffix: &str) -> Result<bool, String> {
    let v = semver::Version::parse(version.trim_start_matches('v'))
        .map_err(|e| format!("Could not parse release version '{}': {}", version, e))?;
    if suffix != ".deb" {
        return Ok(false);
    }
    let core = format!("{}.{}.{}", v.major, v.minor, v.patch);
    Ok(asset_name.contains(&core))
}

/// Downloads `release`'s installer asset plus `checksums.txt` and
/// `checksums.txt.minisig` into a fresh temp directory, verifies the
/// manifest's signature against `RELEASE_PUBLIC_KEY_B64` and the asset's
/// hash against the now-trusted manifest -- reusing `verify_signature`/
/// `sha256_hex` rather than duplicating that logic -- and returns the
/// verified installer's local path. Fails closed: any download error,
/// signature failure, or hash mismatch returns `Err` and leaves nothing
/// for a caller to install.
///
/// Only ever actually invoked for a Linux `.deb` today -- `routes.rs`'s
/// `/api/updates/download-and-install` handler hands the result straight to
/// `open_with_system_handler` (`apt`/the desktop's `.deb` installer).
/// Windows/macOS auto-apply through this automated path isn't built yet
/// (Windows already has its own manual-download path per `docs/update.md`);
/// `check_latest_release` still resolves their asset names so a future
/// auto-apply implementation doesn't need to touch the check step.
pub async fn download_and_verify_release(release: &ReleaseInfo) -> Result<PathBuf, String> {
    // The release JSON is only as trustworthy as whoever can publish to
    // RELEASES_REPO, and the signed manifest carries no version of its own.
    // Pin every URL to this repo's release downloads, and require the
    // (signed) asset filename to carry the release's version -- so a replayed
    // old manifest + old installer can't pass under a newer tag.
    for url in [&release.download_url, &release.checksums_url, &release.checksums_sig_url] {
        if !is_release_download_url(url) {
            return Err(format!("Refusing release asset outside {}'s release downloads: {}", RELEASES_REPO, url));
        }
    }
    if !asset_name_matches_version(&release.asset_name, &release.version, platform_asset_suffix())? {
        return Err(format!(
            "Release asset '{}' does not carry release version {}; refusing possible rollback",
            release.asset_name, release.version
        ));
    }
    if !is_version_newer(&release.version, env!("CARGO_PKG_VERSION"))? {
        return Err(format!("Release {} is not newer than this build ({})", release.version, env!("CARGO_PKG_VERSION")));
    }

    let client = releases_client()?;
    let dir = std::env::temp_dir().join(format!("os-next-update-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create temp dir: {}", e))?;

    let fetch_text = |url: String| {
        let client = client.clone();
        async move {
            let resp = client.get(&url).send().await.map_err(|e| format!("Failed to download {}: {}", url, e))?;
            resp.error_for_status()
                .map_err(|e| format!("Download failed for {}: {}", url, e))?
                .text()
                .await
                .map_err(|e| format!("Failed to read response body for {}: {}", url, e))
        }
    };
    let fetch_bytes = |url: String| {
        let client = client.clone();
        async move {
            let resp = client.get(&url).send().await.map_err(|e| format!("Failed to download {}: {}", url, e))?;
            resp.error_for_status()
                .map_err(|e| format!("Download failed for {}: {}", url, e))?
                .bytes()
                .await
                .map_err(|e| format!("Failed to read response body for {}: {}", url, e))
        }
    };

    let checksums_text = fetch_text(release.checksums_url.clone()).await?;
    let checksums_sig_text = fetch_text(release.checksums_sig_url.clone()).await?;
    verify_signature(checksums_text.as_bytes(), &checksums_sig_text, RELEASE_PUBLIC_KEY_B64)
        .map_err(|e| format!("checksums.txt.minisig did not verify: {}", e))?;

    let asset_bytes = fetch_bytes(release.download_url.clone()).await?;
    let expected_hash = checksums_text
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            let name = parts.next()?.trim_start_matches('*');
            (name == release.asset_name).then(|| hash.to_lowercase())
        })
        .ok_or_else(|| format!("checksums.txt has no entry for {}", release.asset_name))?;
    let actual_hash = sha256_hex(&asset_bytes);
    if actual_hash != expected_hash {
        return Err(format!(
            "Downloaded {} hash mismatch: expected {}, got {}",
            release.asset_name, expected_hash, actual_hash
        ));
    }

    let asset_path = dir.join(&release.asset_name);
    fs::write(&asset_path, &asset_bytes).map_err(|e| format!("Failed to write {}: {}", asset_path.display(), e))?;
    Ok(asset_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generates a real keypair and a real signature with the `minisign`
    /// crate (dev-only), then checks `verify_signature` -- the actual
    /// production code path -- accepts it. Proves the two crates'
    /// implementations of the same file format actually interoperate,
    /// not just that each compiles in isolation.
    fn sign_with_minisign(data: &[u8]) -> (String, String) {
        let kp = minisign::KeyPair::generate_unencrypted_keypair().expect("generate keypair");
        let sig_box = minisign::sign(None, &kp.sk, data, None, None).expect("sign");
        (kp.pk.to_base64(), sig_box.into_string())
    }

    #[test]
    fn accepts_a_real_minisign_signature_from_the_matching_key() {
        let data = b"checksums.txt contents go here\nos-next-linux-x64.tar.gz  deadbeef...\n";
        let (public_key_b64, signature_text) = sign_with_minisign(data);

        let result = verify_signature(data, &signature_text, &public_key_b64);
        assert!(result.is_ok(), "expected valid signature to verify, got {:?}", result);
    }

    #[test]
    fn rejects_tampered_data() {
        let data = b"checksums.txt original contents\n";
        let (public_key_b64, signature_text) = sign_with_minisign(data);

        let tampered = b"checksums.txt TAMPERED contents\n";
        let result = verify_signature(tampered, &signature_text, &public_key_b64);
        assert!(result.is_err(), "expected tampered data to fail verification");
    }

    #[test]
    fn rejects_a_signature_from_a_different_key() {
        let data = b"checksums.txt contents\n";
        let (_correct_pk, signature_text) = sign_with_minisign(data);

        // A totally different keypair's public key should not validate a
        // signature it didn't produce.
        let other_kp = minisign::KeyPair::generate_unencrypted_keypair().expect("generate keypair");
        let result = verify_signature(data, &signature_text, &other_kp.pk.to_base64());
        assert!(result.is_err(), "expected signature from a different key to fail verification");
    }

    fn scratch_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "os-next-updater-test-{}-{}-{:?}",
            label,
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sha256_hex_matches_a_known_vector() {
        // sha256("") -- the standard empty-input test vector.
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }

    #[test]
    fn checksum_check_reports_no_checksum_file_when_none_present() {
        let dir = scratch_dir("none");
        let file = dir.join("package.deb");
        fs::write(&file, b"fake package bytes").unwrap();

        let result = check_local_file_against_sibling_checksums(&file, "unused");
        assert_eq!(result, ChecksumCheck::NoChecksumFile);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn checksum_check_matches_unsigned_manifest() {
        let dir = scratch_dir("unsigned");
        let file = dir.join("package.deb");
        let bytes = b"fake package bytes";
        fs::write(&file, bytes).unwrap();
        fs::write(dir.join("checksums.txt"), format!("{}  package.deb\n", sha256_hex(bytes))).unwrap();

        let result = check_local_file_against_sibling_checksums(&file, "unused");
        assert_eq!(result, ChecksumCheck::ChecksumMatchedUnsigned);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn checksum_check_detects_mismatch() {
        let dir = scratch_dir("mismatch");
        let file = dir.join("package.deb");
        fs::write(&file, b"fake package bytes").unwrap();
        fs::write(dir.join("checksums.txt"), "0000000000000000000000000000000000000000000000000000000000000  package.deb\n").unwrap();

        let result = check_local_file_against_sibling_checksums(&file, "unused");
        assert!(matches!(result, ChecksumCheck::Mismatch { .. }), "expected Mismatch, got {:?}", result);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn checksum_check_detects_missing_entry() {
        let dir = scratch_dir("no-entry");
        let file = dir.join("package.deb");
        fs::write(&file, b"fake package bytes").unwrap();
        fs::write(dir.join("checksums.txt"), "abc123  some-other-file.tar.gz\n").unwrap();

        let result = check_local_file_against_sibling_checksums(&file, "unused");
        assert_eq!(result, ChecksumCheck::NoEntryForFile);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn checksum_check_verifies_a_real_signed_manifest() {
        let dir = scratch_dir("signed");
        let file = dir.join("package.deb");
        let bytes = b"fake package bytes";
        fs::write(&file, bytes).unwrap();

        let checksums = format!("{}  package.deb\n", sha256_hex(bytes));
        fs::write(dir.join("checksums.txt"), &checksums).unwrap();
        let (public_key_b64, signature_text) = sign_with_minisign(checksums.as_bytes());
        fs::write(dir.join("checksums.txt.minisig"), &signature_text).unwrap();

        let result = check_local_file_against_sibling_checksums(&file, &public_key_b64);
        assert_eq!(result, ChecksumCheck::VerifiedAndSigned);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn checksum_check_rejects_a_manifest_signed_by_the_wrong_key() {
        let dir = scratch_dir("wrong-key");
        let file = dir.join("package.deb");
        let bytes = b"fake package bytes";
        fs::write(&file, bytes).unwrap();

        let checksums = format!("{}  package.deb\n", sha256_hex(bytes));
        fs::write(dir.join("checksums.txt"), &checksums).unwrap();
        let (_signing_pk, signature_text) = sign_with_minisign(checksums.as_bytes());
        fs::write(dir.join("checksums.txt.minisig"), &signature_text).unwrap();

        let other_kp = minisign::KeyPair::generate_unencrypted_keypair().expect("generate keypair");
        let result = check_local_file_against_sibling_checksums(&file, &other_kp.pk.to_base64());
        assert!(matches!(result, ChecksumCheck::ChecksumFileUnverified { .. }), "expected ChecksumFileUnverified, got {:?}", result);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn release_urls_must_be_this_repos_github_downloads() {
        let ok = format!("https://github.com/{}/releases/download/v1.0.0/a.deb", RELEASES_REPO);
        assert!(is_release_download_url(&ok));
        assert!(!is_release_download_url("https://evil.example/a.deb"));
        assert!(!is_release_download_url("http://github.com/x/releases/download/a.deb"));
        assert!(!is_release_download_url(&format!("https://github.com/{}/releases/download/../../x", RELEASES_REPO)));
    }

    #[test]
    fn asset_name_must_carry_the_release_version() {
        assert!(asset_name_matches_version("opensanctuary_0.3.0-1_amd64.deb", "0.3.0", ".deb").unwrap());
        assert!(!asset_name_matches_version("opensanctuary_0.1.0-1_amd64.deb", "v99.0.0", ".deb").unwrap());
        assert!(!asset_name_matches_version("opensanctuary-setup-x64.exe", "0.3.0", "setup-x64.exe").unwrap());
    }

    #[test]
    fn is_version_newer_uses_real_semver_ordering_not_lexical() {
        assert!(is_version_newer("v0.3.0", "0.2.0").unwrap());
        assert!(!is_version_newer("v0.2.0", "0.2.0").unwrap(), "equal versions are not newer");
        assert!(!is_version_newer("v0.1.0", "0.2.0").unwrap());
        // A prerelease sorts *below* its own release -- this is the exact
        // case that makes the real test repo's "v0.2.0-test" tag correctly
        // NOT count as an update over a running "0.2.0" build.
        assert!(!is_version_newer("v0.2.0-test", "0.2.0").unwrap());
        assert!(is_version_newer("v0.2.0", "0.2.0-test").unwrap());
        // Real ordering, not lexical: "0.10.0" > "0.9.0" even though "1" < "9".
        assert!(is_version_newer("v0.10.0", "0.9.0").unwrap());
    }

    #[test]
    fn is_version_newer_rejects_unparseable_versions() {
        assert!(is_version_newer("not-a-version", "0.2.0").is_err());
        assert!(is_version_newer("v1.0.0", "also-not-a-version").is_err());
    }

    /// Exercises the real network path end to end against the actual public
    /// test repo (`RELEASES_REPO`): fetch /releases/latest for real, build a
    /// `ReleaseInfo` from it (bypassing the version-newer check, since this
    /// crate's own dev version may not be "older" than the test tag -- see
    /// `is_version_newer_uses_real_semver_ordering_not_lexical` above for
    /// why "v0.2.0-test" specifically does NOT count as newer than "0.2.0"),
    /// then really download and verify it. Proves `download_and_verify_release`
    /// works against a real GitHub release signed with the real (test) key
    /// embedded in `RELEASE_PUBLIC_KEY_B64`, not just against synthetic
    /// fixtures.
    ///
    /// `#[ignore]`d unlike most of this codebase's live-network tests (e.g.
    /// `test_fetch_churchapps_catalog_live`) -- confirmed directly that
    /// running this (and `test_update_check_and_status_routes_against_real_test_repo`
    /// in `tests/core_tests.rs`) on every single `cargo test` during this
    /// feature's own iterative development was the dominant source of real
    /// GitHub requests this session, enough to trip GitHub's secondary
    /// abuse-rate-limit mid-session even with the primary 60/hour budget
    /// nowhere near exhausted. Run explicitly with
    /// `cargo test --lib -- --ignored` when actually verifying the live
    /// path (e.g. after a real key rotation), not as part of routine
    /// `cargo test`.
    #[tokio::test]
    #[ignore]
    async fn downloads_and_verifies_the_real_test_repo_release() {
        let release_json = fetch_latest_release_json().await.expect("Should fetch the real test repo's latest release");
        let release = release_info_from_json(&release_json).expect("Should extract a ReleaseInfo from the real release");
        assert!(release.asset_name.ends_with(".deb"), "expected a .deb asset on Linux, got {}", release.asset_name);

        let path = download_and_verify_release(&release).await.expect("Should download and verify the real signed release");
        assert!(path.exists());
        assert_eq!(path.file_name().and_then(|n| n.to_str()), Some(release.asset_name.as_str()));
        fs::remove_dir_all(path.parent().unwrap()).ok();
    }
}
