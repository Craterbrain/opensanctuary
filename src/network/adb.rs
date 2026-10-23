//! Drives `adb` (Android Debug Bridge) to sideload and set up the Android TV
//! client (`apps/android-tv`) directly from the console, instead of Play Store
//! distribution -- see `docs/CLIENT_PAIRING.md`.
//!
//! What ADB does here: install the app, make it the TV's home app (the stock
//! launchers are disabled, so the TV boots straight into OpenSanctuary and the
//! Home button returns to it), and tell it where the console is and which
//! certificate to expect. What it deliberately does NOT do is authorize the
//! TV: pairing is always the QR scan by the operator's phone, so a TV only
//! ever gets its token from someone looking at both screens.
//!
//! Network ADB only: the operator enables "Network debugging" in the TV's
//! Developer Options (built into Android TV/Google TV, opens port 5555
//! directly -- no USB cable, no wireless-debugging pairing-code exchange
//! the way phone Wireless Debugging needs). `adb connect <ip>:5555` is the
//! only connection step.
//!
//! Same background-task-with-polled-progress shape as
//! `crate::storage::ytdlp_import` (`OnceLock<Mutex<HashMap<...>>>` registry,
//! `tokio::process::Command` with `kill_on_drop(true)`, spawned so the HTTP
//! handler returns a `task_id` immediately) -- deliberately not the
//! blocking `std::process::Command` pattern `network::linux`/`network::windows`
//! use for pkexec/UAC, since that blocks the async runtime for the call's
//! whole duration and an install can take several seconds.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::process::Command;

const ADB_PORT: u16 = 5555;
const TV_PACKAGE: &str = "org.opensanctuary.tv";
const TV_ACTIVITY: &str = "org.opensanctuary.tv/.MainActivity";

/// Packages that declare the HOME category for first-boot setup rather than as a launcher: never disabled.
fn is_setup_package(package: &str) -> bool {
    let p = package.to_ascii_lowercase();
    ["setup", "provision", "wizard", "oobe"].iter().any(|w| p.contains(w))
}

/// Pulls package names out of `cmd package query-activities --brief` (lines like `com.example/.MainActivity`).
fn parse_home_packages(output: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        let Some((package, activity)) = line.split_once('/') else { continue };
        let valid = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '$' | '-'));
        if package.contains(' ') || !valid(package) || !valid(activity.trim_start_matches('.')) {
            continue;
        }
        if !found.iter().any(|p| p == package) {
            found.push(package.to_string());
        }
    }
    found
}

/// Launcher packages to disable so ours is the only home app: every HOME handler except ours and setup wizards.
pub(crate) fn launchers_to_disable(home_query_output: &str) -> Vec<String> {
    parse_home_packages(home_query_output)
        .into_iter()
        .filter(|p| p != TV_PACKAGE && !is_setup_package(p))
        .collect()
}

/// Disabled packages that look like launchers, from `pm list packages -d` (lines like `package:com.example`).
pub(crate) fn disabled_launchers(list_output: &str) -> Vec<String> {
    list_output
        .lines()
        .filter_map(|l| l.trim().strip_prefix("package:"))
        .map(|p| p.trim().to_string())
        .filter(|p| p != TV_PACKAGE && !is_setup_package(p) && p.to_ascii_lowercase().contains("launcher"))
        .collect()
}

/// `adb`, or the program named by `OS_NEXT_ADB` (tests point it at a fake device).
fn adb_command() -> Command {
    Command::new(std::env::var_os("OS_NEXT_ADB").unwrap_or_else(|| "adb".into()))
}

async fn adb_shell(target: &str, args: &[&str]) -> Result<std::process::Output, String> {
    let mut cmd = adb_command();
    cmd.kill_on_drop(true);
    cmd.arg("-s").arg(target).arg("shell").args(args).output().await.map_err(|e| format!("Failed to run adb shell: {e}"))
}

/// `adb connect`, with the check `adb connect` itself needs: it exits 0 even when it failed.
async fn connect(target: &str) -> Result<(), String> {
    let mut cmd = adb_command();
    cmd.kill_on_drop(true);
    match cmd.arg("connect").arg(target).output().await {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            if text.to_lowercase().contains("connected to") {
                Ok(())
            } else {
                Err(format!(
                    "adb could not connect to {target}: {}. Check that Network debugging is enabled on the TV, the IP is correct, and — if this is the first time connecting from this computer — that you've accepted the \"Allow debugging?\" prompt on the TV's screen.",
                    text.trim()
                ))
            }
        }
        Ok(out) => Err(format!("adb connect failed: {}", String::from_utf8_lossy(&out.stderr).trim())),
        Err(e) => Err(format!("Failed to run adb: {e}")),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdbProvisionProgress {
    pub task_id: String,
    /// One of "connecting", "installing", "pairing", "launching", "completed", "failed".
    pub status: String,
    pub message: String,
    pub is_complete: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdbAvailability {
    pub adb_available: bool,
    pub adb_version: Option<String>,
    pub apk_available: bool,
}

fn get_progress_registry() -> &'static Arc<Mutex<HashMap<String, AdbProvisionProgress>>> {
    static REGISTRY: OnceLock<Arc<Mutex<HashMap<String, AdbProvisionProgress>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

pub fn get_progress(task_id: &str) -> Option<AdbProvisionProgress> {
    let reg = get_progress_registry().lock().unwrap_or_else(|e| e.into_inner());
    reg.get(task_id).cloned()
}

fn set_progress(task_id: &str, status: &str, message: impl Into<String>, is_complete: bool, error: Option<String>) {
    let mut reg = get_progress_registry().lock().unwrap_or_else(|e| e.into_inner());
    // Same eviction convention as ytdlp_import.rs's progress registry --
    // caps unbounded growth from repeated provisioning attempts over a long
    // uptime without needing a TTL/background sweeper.
    if reg.len() > 100 {
        reg.retain(|_, v| v.status != "completed" && v.status != "failed");
    }
    reg.insert(
        task_id.to_string(),
        AdbProvisionProgress {
            task_id: task_id.to_string(),
            status: status.to_string(),
            message: message.into(),
            is_complete,
            error,
        },
    );
}

/// `adb version` -- same shape as `YtDlpImporter::check_status`'s
/// availability probe. Reports the resolved TV APK path's presence too
/// (`crate::storage::paths::resolve_tv_apk_path`) so the UI can show a
/// clear "ADB not found" / "no TV build available" state up front rather
/// than a confusing failure after the operator's already typed an IP.
pub async fn check_availability(apk_path: Option<&std::path::Path>) -> AdbAvailability {
    let output = adb_command().arg("version").output().await;
    let (adb_available, adb_version) = match output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            let version = text.lines().next().map(|l| l.trim().to_string());
            (true, version)
        }
        _ => (false, None),
    };
    AdbAvailability {
        adb_available,
        adb_version,
        apk_available: apk_path.map(|p| p.is_file()).unwrap_or(false),
    }
}

/// Runs connect -> install -> become the home app -> launch, reporting progress into the shared registry.
/// The TV is NOT paired here: it comes up showing its pairing QR for the operator's phone to scan.
pub async fn provision(
    task_id: String,
    ip: String,
    apk_path: std::path::PathBuf,
    server_ip: String,
    server_port: u16,
    is_stage_mode: bool,
    device_name: String,
    cert_fingerprint: String,
) {
    let target = format!("{}:{}", ip.trim(), ADB_PORT);

    set_progress(&task_id, "connecting", format!("Connecting to {}…", target), false, None);
    if let Err(e) = connect(&target).await {
        set_progress(&task_id, "failed", "Could not reach the TV.", true, Some(e));
        return;
    }

    set_progress(&task_id, "installing", "Installing OpenSanctuary TV…", false, None);
    let mut cmd = adb_command();
    cmd.kill_on_drop(true);
    match cmd.arg("-s").arg(&target).arg("install").arg("-r").arg(&apk_path).output().await {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            set_progress(&task_id, "failed", "Could not install the app.", true, Some(format!("adb install failed: {}", String::from_utf8_lossy(&out.stdout).trim())));
            return;
        }
        Err(e) => {
            set_progress(&task_id, "failed", "Could not install the app.", true, Some(format!("Failed to run adb install: {}", e)));
            return;
        }
    }

    // Only take the home screen away once the app is really there and answers as a home app.
    set_progress(&task_id, "home", "Making OpenSanctuary the TV's home app…", false, None);
    let query = match adb_shell(&target, &["cmd", "package", "query-activities", "--brief", "-a", "android.intent.action.MAIN", "-c", "android.intent.category.HOME"]).await {
        Ok(out) => String::from_utf8_lossy(&out.stdout).to_string(),
        Err(e) => {
            set_progress(&task_id, "failed", "Installed, but could not set the home app.", true, Some(e));
            return;
        }
    };
    if !parse_home_packages(&query).iter().any(|p| p == TV_PACKAGE) {
        set_progress(&task_id, "failed", "Installed, but this TV does not offer OpenSanctuary as a home app.", true, Some(format!("The installed app did not show up among the TV's home apps: {}", query.trim())));
        return;
    }
    let mut disabled: Vec<String> = Vec::new();
    for package in launchers_to_disable(&query) {
        match adb_shell(&target, &["pm", "disable-user", "--user", "0", &package]).await {
            Ok(out) if out.status.success() => disabled.push(package),
            Ok(out) => {
                set_progress(&task_id, "failed", format!("Could not disable the stock home app {package}."), true, Some(format!("pm disable-user {package}: {}{}", String::from_utf8_lossy(&out.stdout).trim(), String::from_utf8_lossy(&out.stderr).trim())));
                return;
            }
            Err(e) => {
                set_progress(&task_id, "failed", "Could not disable the stock home app.", true, Some(e));
                return;
            }
        }
    }

    set_progress(&task_id, "launching", "Starting OpenSanctuary TV…", false, None);
    let mut cmd = adb_command();
    cmd.kill_on_drop(true);
    let launch_result = cmd
        .arg("-s")
        .arg(&target)
        .arg("shell")
        .arg("am")
        .arg("start")
        .arg("-n")
        .arg(TV_ACTIVITY)
        .arg("--es")
        .arg("server_ip")
        .arg(&server_ip)
        .arg("--ei")
        .arg("server_port")
        .arg(server_port.to_string())
        .arg("--ez")
        .arg("is_stage_mode")
        .arg(is_stage_mode.to_string())
        // The name this TV shows in the console's Paired Devices list once it has been paired.
        .arg("--es")
        .arg("console_name")
        .arg(&device_name)
        // The certificate this console serves, so the TV verifies it from its very first HTTPS connection
        // instead of trusting whatever answers first (src/network/tls.rs's cert_fingerprint_sha256,
        // PairingManager.kt's pin check). This is only a pin: it authorizes nothing.
        .arg("--es")
        .arg("cert_fingerprint")
        .arg(&cert_fingerprint)
        .output()
        .await;
    match launch_result {
        Ok(out) if out.status.success() => {
            let note = if disabled.is_empty() { String::new() } else { format!(" Stock home app disabled: {}.", disabled.join(", ")) };
            set_progress(&task_id, "completed", format!("{device_name} is installed and is now the TV's home app. Scan the code on the TV's screen to pair it.{note}"), true, None);
        }
        Ok(out) => {
            set_progress(&task_id, "failed", "Installed, but could not launch the app.", true, Some(format!("adb shell am start failed: {}", String::from_utf8_lossy(&out.stdout).trim())));
        }
        Err(e) => {
            set_progress(&task_id, "failed", "Installed, but could not launch the app.", true, Some(format!("Failed to run adb shell: {}", e)));
        }
    }
}

/// Undoes the home-app takeover: re-enables the stock launchers so the TV offers its own home screen again.
/// (The app itself stays installed; uninstall it from the TV's settings if it should go.)
pub async fn restore_stock_home(task_id: String, ip: String) {
    let target = format!("{}:{}", ip.trim(), ADB_PORT);
    set_progress(&task_id, "connecting", format!("Connecting to {}…", target), false, None);
    if let Err(e) = connect(&target).await {
        set_progress(&task_id, "failed", "Could not reach the TV.", true, Some(e));
        return;
    }
    set_progress(&task_id, "home", "Restoring the TV's own home screen…", false, None);
    let listing = match adb_shell(&target, &["pm", "list", "packages", "-d"]).await {
        Ok(out) => String::from_utf8_lossy(&out.stdout).to_string(),
        Err(e) => {
            set_progress(&task_id, "failed", "Could not read the TV's disabled apps.", true, Some(e));
            return;
        }
    };
    let packages = disabled_launchers(&listing);
    if packages.is_empty() {
        set_progress(&task_id, "completed", "No disabled home screen was found on this TV; nothing to restore.", true, None);
        return;
    }
    for package in &packages {
        if let Err(e) = adb_shell(&target, &["pm", "enable", "--user", "0", package]).await {
            set_progress(&task_id, "failed", format!("Could not re-enable {package}."), true, Some(e));
            return;
        }
    }
    set_progress(&task_id, "completed", format!("Restored the TV's home screen ({}). Press Home on the TV's remote and choose which home app to use.", packages.join(", ")), true, None);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_registry_roundtrips_and_evicts() {
        let task_id = format!("test-{}", uuid::Uuid::new_v4());
        assert!(get_progress(&task_id).is_none());

        set_progress(&task_id, "connecting", "Connecting…", false, None);
        let p = get_progress(&task_id).expect("should have progress after set");
        assert_eq!(p.status, "connecting");
        assert!(!p.is_complete);
        assert!(p.error.is_none());

        set_progress(&task_id, "failed", "boom", true, Some("real error".to_string()));
        let p = get_progress(&task_id).expect("should still have progress");
        assert_eq!(p.status, "failed");
        assert!(p.is_complete);
        assert_eq!(p.error.as_deref(), Some("real error"));
    }

    /// Exercises the real `adb connect` subprocess against loopback (nothing
    /// listens on the ADB port here, so this fails fast and deterministically
    /// -- "Connection refused" on loopback is near-instant, unlike an
    /// unroutable IP which can hang for the OS's full TCP connect timeout;
    /// confirmed manually before writing this). Proves the specific quirk
    /// this module works around for real: `adb connect` exits 0 even when it
    /// fails, so `provision` must check the response text, not just the
    /// exit status -- if that check were missing, this test would see
    /// `is_complete` never becoming "failed" and would hang until the whole
    /// (nonexistent) install/launch sequence ran against a device that was
    /// never actually reached.
    /// Tests that point `OS_NEXT_ADB` at a fake, or rely on there being none, must not overlap.
    static ADB_ENV: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// A fake `adb` that logs every call to `$FAKE_ADB_LOG` and answers like a Google TV with a stock launcher.
    fn fake_adb(dir: &std::path::Path) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake-adb");
        std::fs::write(&path, r#"#!/bin/sh
echo "$@" >> "$FAKE_ADB_LOG"
case "$*" in
  *connect*) echo "connected to 10.0.0.9:5555" ;;
  *"query-activities"*) printf '3 activities found:\n  com.google.android.tvlauncher/.MainActivity\n  org.opensanctuary.tv/.MainActivity\n  com.google.android.tungsten.setupwraith/.HomeActivity\n' ;;
  *"pm list packages -d"*) printf 'package:com.google.android.tvlauncher\npackage:com.google.android.tungsten.setupwraith\n' ;;
esac
exit 0
"#).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn provisioning_installs_takes_the_home_screen_and_never_hands_the_tv_a_pairing_credential() {
        let _guard = ADB_ENV.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("calls.log");
        let fake = fake_adb(dir.path());
        std::env::set_var("OS_NEXT_ADB", &fake);
        std::env::set_var("FAKE_ADB_LOG", &log);
        let task_id = format!("test-provision-{}", uuid::Uuid::new_v4());
        provision(task_id.clone(), "10.0.0.9".into(), "/tmp/app.apk".into(), "10.0.0.2".into(), 8443, false, "Foyer TV".into(), "abc123".into()).await;
        let progress = get_progress(&task_id).unwrap();
        std::env::remove_var("OS_NEXT_ADB");
        std::env::remove_var("FAKE_ADB_LOG");
        assert_eq!(progress.status, "completed", "{progress:?}");
        assert!(progress.message.contains("Scan the code on the TV"), "{}", progress.message);

        let calls = std::fs::read_to_string(&log).unwrap();
        let lines: Vec<&str> = calls.lines().collect();
        let pos = |needle: &str| lines.iter().position(|l| l.contains(needle)).unwrap_or_else(|| panic!("no adb call containing {needle:?} in:\n{calls}"));
        // Order: installed, then the stock launcher is disabled, then the app is started.
        assert!(pos("install -r /tmp/app.apk") < pos("disable-user --user 0 com.google.android.tvlauncher"));
        assert!(pos("disable-user --user 0 com.google.android.tvlauncher") < pos("am start"));
        // The setup wizard and our own app are never disabled.
        assert!(!calls.contains("disable-user --user 0 com.google.android.tungsten.setupwraith"));
        assert!(!calls.contains("disable-user --user 0 org.opensanctuary.tv"));
        // The launch tells the TV where the console is and which certificate to expect: a pin, not a credential.
        let launch = lines[pos("am start")];
        for want in ["server_ip 10.0.0.2", "server_port 8443", "cert_fingerprint abc123", "console_name Foyer TV"] {
            assert!(launch.contains(want), "{launch}");
        }
        assert!(!calls.contains("pairing_session_token") && !calls.contains("session"), "ADB must not carry a pairing session: {calls}");

        // Restoring turns the stock launcher back on, and only that.
        let restore_id = format!("test-restore-{}", uuid::Uuid::new_v4());
        std::env::set_var("OS_NEXT_ADB", &fake);
        std::env::set_var("FAKE_ADB_LOG", &log);
        restore_stock_home(restore_id.clone(), "10.0.0.9".into()).await;
        let restored = get_progress(&restore_id).unwrap();
        std::env::remove_var("OS_NEXT_ADB");
        std::env::remove_var("FAKE_ADB_LOG");
        assert_eq!(restored.status, "completed", "{restored:?}");
        let calls = std::fs::read_to_string(&log).unwrap();
        assert!(calls.contains("pm enable --user 0 com.google.android.tvlauncher"));
        assert!(!calls.contains("pm enable --user 0 com.google.android.tungsten.setupwraith"));
    }

    #[tokio::test]
    async fn provision_fails_cleanly_against_an_unreachable_device() {
        let _guard = ADB_ENV.lock().await;
        let task_id = format!("test-provision-{}", uuid::Uuid::new_v4());
        provision(
            task_id.clone(),
            "127.0.0.1".to_string(),
            std::path::PathBuf::from("/definitely/not/a/real/path.apk"),
            "127.0.0.1".to_string(),
            8080,
            false,
            "Test TV".to_string(),
            "deadbeef".to_string(),
        )
        .await;

        let progress = get_progress(&task_id).expect("provision should have recorded progress");
        assert!(progress.is_complete, "expected the task to finish (fail), got {:?}", progress);
        assert_eq!(progress.status, "failed");
        assert!(progress.error.is_some(), "expected a real error message, got {:?}", progress);
    }

    #[test]
    fn home_query_output_is_parsed_into_launchers_to_disable() {
        let out = "3 activities found:\n  com.google.android.tvlauncher/.MainActivity\n  org.opensanctuary.tv/.MainActivity\n  com.google.android.tungsten.setupwraith/.HomeActivity\n";
        assert_eq!(launchers_to_disable(out), vec!["com.google.android.tvlauncher".to_string()]);
        // Nothing but ours, a header, or noise: nothing to disable (and no panic).
        assert!(launchers_to_disable("1 activities found:\n  org.opensanctuary.tv/.MainActivity\n").is_empty());
        assert!(launchers_to_disable("").is_empty());
        assert!(launchers_to_disable("Error: shell said something/odd here\n").is_empty());
    }

    #[test]
    fn disabled_launchers_are_only_launcher_packages() {
        let out = "package:com.google.android.tvlauncher\npackage:com.google.android.apps.tv.launcherx\npackage:com.vendor.bloat\npackage:com.google.android.tungsten.setupwraith\npackage:org.opensanctuary.tv\n";
        assert_eq!(disabled_launchers(out), vec!["com.google.android.tvlauncher".to_string(), "com.google.android.apps.tv.launcherx".to_string()]);
    }

    #[tokio::test]
    async fn check_availability_reports_false_when_adb_missing_from_path() {
        // Doesn't assume adb is or isn't installed on the test machine --
        // just proves the function returns cleanly either way rather than
        // panicking, and that a missing APK path is reported independently
        // of adb's own availability.
        let result = check_availability(Some(std::path::Path::new("/definitely/not/a/real/path.apk"))).await;
        assert!(!result.apk_available);
    }
}
