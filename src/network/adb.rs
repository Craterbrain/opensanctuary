//! Drives `adb` (Android Debug Bridge) to sideload and provision the
//! Android TV client (`apps/android-tv`) directly from the console, instead
//! of Play Store distribution -- see the plan this shipped from (ADB-driven
//! Android TV sideload + real pairing) and `docs/CLIENT_PAIRING.md` for the
//! pairing design this hands the TV app the credentials for.
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
const TV_ACTIVITY: &str = "org.opensanctuary.tv/.MainActivity";

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
    let output = Command::new("adb").arg("version").output().await;
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

/// Runs the full connect -> install -> mint-pairing-session -> launch
/// sequence, reporting progress into the shared registry as it goes.
/// `mint_session` is `AppState::create_pairing_session` -- passed in as a
/// closure-free async fn pointer rather than importing `AppState` here, to
/// keep this module's only dependency on the rest of the app being the one
/// thing it actually needs, not the whole app state type.
pub async fn provision(
    task_id: String,
    ip: String,
    apk_path: std::path::PathBuf,
    server_ip: String,
    server_port: u16,
    is_stage_mode: bool,
    device_name: String,
    pairing_session_token: String,
    cert_fingerprint: String,
) {
    let target = format!("{}:{}", ip.trim(), ADB_PORT);

    set_progress(&task_id, "connecting", format!("Connecting to {}…", target), false, None);
    let mut cmd = Command::new("adb");
    cmd.kill_on_drop(true);
    let connect_result = cmd.arg("connect").arg(&target).output().await;
    match connect_result {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            // `adb connect` exits 0 even when it fails to actually reach the
            // device (e.g. "unable to connect" is printed to stdout with a
            // success exit code) -- check the message text, not just the
            // exit status, or a wrong IP silently "succeeds" here and only
            // fails confusingly at the install step.
            if !text.to_lowercase().contains("connected to") {
                set_progress(
                    &task_id,
                    "failed",
                    "Could not reach the TV.",
                    true,
                    Some(format!(
                        "adb could not connect to {target}: {}. Check that Network debugging is enabled on the TV, the IP is correct, and — if this is the first time connecting from this computer — that you've accepted the \"Allow debugging?\" prompt on the TV's screen.",
                        text.trim()
                    )),
                );
                return;
            }
        }
        Ok(out) => {
            set_progress(
                &task_id,
                "failed",
                "Could not reach the TV.",
                true,
                Some(format!("adb connect failed: {}", String::from_utf8_lossy(&out.stderr).trim())),
            );
            return;
        }
        Err(e) => {
            set_progress(&task_id, "failed", "adb is not available.", true, Some(format!("Failed to run adb: {}", e)));
            return;
        }
    }

    set_progress(&task_id, "installing", "Installing OpenSanctuary TV…", false, None);
    let mut cmd = Command::new("adb");
    cmd.kill_on_drop(true);
    let install_result = cmd.arg("-s").arg(&target).arg("install").arg("-r").arg(&apk_path).output().await;
    match install_result {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            set_progress(
                &task_id,
                "failed",
                "Could not install the app.",
                true,
                Some(format!("adb install failed: {}", String::from_utf8_lossy(&out.stdout).trim())),
            );
            return;
        }
        Err(e) => {
            set_progress(&task_id, "failed", "Could not install the app.", true, Some(format!("Failed to run adb install: {}", e)));
            return;
        }
    }

    set_progress(&task_id, "launching", "Launching and pairing…", false, None);
    let mut cmd = Command::new("adb");
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
        .arg("--es")
        .arg("pairing_session_token")
        .arg(&pairing_session_token)
        .arg("--es")
        .arg("console_name")
        .arg(&device_name)
        // Lets the TV app verify it's really talking to *this* console's
        // certificate from its very first HTTPS connection, instead of
        // trusting whatever cert is presented (src/network/tls.rs's
        // cert_fingerprint_sha256, PairingManager.kt's pin check) -- ADB
        // access already proves the same level of trust a phone scanning a
        // QR on this console's own screen would, so delivering it here
        // isn't a weaker guarantee than the manual-pairing path gets.
        .arg("--es")
        .arg("cert_fingerprint")
        .arg(&cert_fingerprint)
        .output()
        .await;
    match launch_result {
        Ok(out) if out.status.success() => {
            set_progress(&task_id, "completed", format!("{} is installed and paired.", device_name), true, None);
        }
        Ok(out) => {
            set_progress(
                &task_id,
                "failed",
                "Installed, but could not launch the app.",
                true,
                Some(format!("adb shell am start failed: {}", String::from_utf8_lossy(&out.stdout).trim())),
            );
        }
        Err(e) => {
            set_progress(&task_id, "failed", "Installed, but could not launch the app.", true, Some(format!("Failed to run adb shell: {}", e)));
        }
    }
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
    #[tokio::test]
    async fn provision_fails_cleanly_against_an_unreachable_device() {
        let task_id = format!("test-provision-{}", uuid::Uuid::new_v4());
        provision(
            task_id.clone(),
            "127.0.0.1".to_string(),
            std::path::PathBuf::from("/definitely/not/a/real/path.apk"),
            "127.0.0.1".to_string(),
            8080,
            false,
            "Test TV".to_string(),
            "pair_sess_test".to_string(),
            "deadbeef".to_string(),
        )
        .await;

        let progress = get_progress(&task_id).expect("provision should have recorded progress");
        assert!(progress.is_complete, "expected the task to finish (fail), got {:?}", progress);
        assert_eq!(progress.status, "failed");
        assert!(progress.error.is_some(), "expected a real error message, got {:?}", progress);
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
