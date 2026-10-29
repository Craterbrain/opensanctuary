//! Runs the local-HTTPS setup script (`packaging/setup-local-https-{linux.sh,windows.ps1}`,
//! see `docs/LOCAL_HTTPS.md`) from the console and keeps its output so the UI can show it live.
//!
//! Same background-task-with-polled-progress shape as `crate::network::adb`: the HTTP handler
//! returns a `task_id` immediately and the console polls for new log lines.
//!
//! The scripts are embedded in the binary, so nothing extra has to ship with an install, and on
//! Linux the script text is handed to `bash -c` instead of a temp file, so there is no file
//! another process could swap between the polkit prompt and the run.
//!
//! The Cloudflare token never goes on a command line. Linux gets it on stdin, Windows in the
//! child's environment, and any line of output that contains it is masked before it is stored.

use serde::Serialize;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::Command;

/// Installed by the .deb and by `install.sh --system`, together with the polkit policy that names it.
#[cfg(unix)]
const POLKIT_HELPER: &str = "/usr/libexec/opensanctuary/setup-local-https";
#[cfg(unix)]
const POLKIT_POLICY: &str = "/usr/share/polkit-1/actions/io.github.craterbrain.opensanctuary.local-https.policy";
#[cfg(unix)]
const LINUX_SCRIPT: &str = include_str!("../../packaging/setup-local-https-linux.sh");
#[cfg(windows)]
const WINDOWS_SCRIPT: &str = include_str!("../../packaging/setup-local-https-windows.ps1");

/// Oldest lines are dropped past this, so a runaway script can't grow memory without bound.
const MAX_LINES: usize = 2000;
const MAX_RUNTIME: std::time::Duration = std::time::Duration::from_secs(15 * 60);

#[derive(Debug, Clone, Serialize)]
pub struct SetupProgress {
    pub task_id: String,
    /// "running", "completed" or "failed".
    pub status: String,
    /// New lines since the `since` the caller asked for.
    pub lines: Vec<String>,
    /// Pass this back as `since` on the next poll.
    pub next: usize,
    pub is_complete: bool,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
}

struct Task {
    lines: Vec<String>,
    /// Lines already dropped from the front, so `next` stays an absolute index.
    dropped: usize,
    status: &'static str,
    exit_code: Option<i32>,
    error: Option<String>,
}

fn registry() -> &'static Arc<Mutex<HashMap<String, Task>>> {
    static REGISTRY: OnceLock<Arc<Mutex<HashMap<String, Task>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

fn with_task<R>(task_id: &str, f: impl FnOnce(&mut Task) -> R) -> Option<R> {
    let mut reg = registry().lock().unwrap_or_else(|e| e.into_inner());
    reg.get_mut(task_id).map(f)
}

fn push_line(task_id: &str, line: String) {
    with_task(task_id, |t| {
        t.lines.push(line);
        if t.lines.len() > MAX_LINES {
            let excess = t.lines.len() - MAX_LINES;
            t.lines.drain(..excess);
            t.dropped += excess;
        }
    });
}

fn finish(task_id: &str, exit_code: Option<i32>, error: Option<String>) {
    with_task(task_id, |t| {
        t.status = if error.is_none() && exit_code == Some(0) { "completed" } else { "failed" };
        t.exit_code = exit_code;
        t.error = error;
    });
}

fn any_running() -> bool {
    let reg = registry().lock().unwrap_or_else(|e| e.into_inner());
    reg.values().any(|t| t.status == "running")
}

pub fn get_progress(task_id: &str, since: usize) -> Option<SetupProgress> {
    let reg = registry().lock().unwrap_or_else(|e| e.into_inner());
    let t = reg.get(task_id)?;
    let end = t.dropped + t.lines.len();
    let from = since.max(t.dropped);
    let lines = if from < end { t.lines[from - t.dropped..].to_vec() } else { Vec::new() };
    Some(SetupProgress {
        task_id: task_id.to_string(),
        status: t.status.to_string(),
        lines,
        next: end,
        is_complete: t.status != "running",
        exit_code: t.exit_code,
        error: t.error.clone(),
    })
}

/// Lowercases and checks a hostname such as `connect.yourchurch.org`. It ends up in a root
/// script's arguments and a Caddyfile, so only plain DNS labels get through.
pub fn validate_domain(domain: &str) -> Result<String, String> {
    let d = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    if d.is_empty() {
        return Err("A domain is required (e.g. connect.yourchurch.org).".to_string());
    }
    if d.len() > 253 {
        return Err("That domain is too long.".to_string());
    }
    let labels: Vec<&str> = d.split('.').collect();
    if labels.len() < 2 {
        return Err("Use a full domain such as connect.yourchurch.org.".to_string());
    }
    for label in labels {
        let ok = !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !ok {
            return Err("The domain may only contain letters, numbers, '-' and '.' (no https://, no path).".to_string());
        }
    }
    Ok(d)
}

fn validate_token(token: &str) -> Result<String, String> {
    let t = token.trim().to_string();
    if t.is_empty() {
        return Err("A Cloudflare API token is required.".to_string());
    }
    if t.len() > 512 || t.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("That doesn't look like a Cloudflare API token.".to_string());
    }
    Ok(t)
}

/// Masks the token in a line of script output (it should never appear, but the output is shown
/// on screen and held in memory, so this is a backstop).
fn redact(line: &str, secret: Option<&str>) -> String {
    match secret {
        Some(s) if !s.is_empty() => line.replace(s, "********"),
        _ => line.to_string(),
    }
}

#[cfg(unix)]
fn is_root() -> bool {
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
        .unwrap_or(false)
}

/// Starts the setup in the background and returns its `task_id`.
pub fn start(domain: &str, port: u16, dry_run: bool, token: &str) -> Result<String, String> {
    let domain = validate_domain(domain)?;
    if port == 0 {
        return Err("Invalid port.".to_string());
    }
    // A dry run only prints what it would do. It still asks for a token, so give it a placeholder.
    let token = if dry_run && token.trim().is_empty() { "dry-run-placeholder".to_string() } else { validate_token(token)? };

    let prepared = build_command(&domain, port, dry_run, &token)?;

    if any_running() {
        return Err("A setup is already running. Wait for it to finish.".to_string());
    }
    let task_id = format!("lhs_{}", uuid::Uuid::new_v4().simple());
    {
        let mut reg = registry().lock().unwrap_or_else(|e| e.into_inner());
        if reg.len() > 20 {
            reg.retain(|_, t| t.status == "running");
        }
        reg.insert(
            task_id.clone(),
            Task { lines: Vec::new(), dropped: 0, status: "running", exit_code: None, error: None },
        );
    }
    push_line(&task_id, format!("Setting up local HTTPS for {domain}{}", if dry_run { " (dry run, nothing will be changed)" } else { "" }));
    tokio::spawn(run(task_id.clone(), prepared, token));
    Ok(task_id)
}

/// A command ready to run, plus how its output and the token reach us.
struct Prepared {
    cmd: Command,
    /// Written to the child's stdin when set (Linux).
    stdin_token: Option<String>,
    /// Output file to follow instead of the child's pipes (Windows: the elevated process can't be piped).
    log_file: Option<std::path::PathBuf>,
}

#[cfg(unix)]
fn build_command(domain: &str, port: u16, dry_run: bool, token: &str) -> Result<Prepared, String> {
    if std::env::var_os("FLATPAK_ID").is_some() && !dry_run {
        return Err("This is the Flatpak build and can't ask for administrator rights. Run the script from a terminal instead: sudo packaging/setup-local-https-linux.sh (see docs/LOCAL_HTTPS.md).".to_string());
    }
    let helper_installed = std::path::Path::new(POLKIT_HELPER).is_file() && std::path::Path::new(POLKIT_POLICY).is_file();
    let elevate = !dry_run && !is_root();
    if elevate && !which_in_path("pkexec") {
        return Err("Administrator rights are needed and pkexec (polkit) isn't installed. Run the script from a terminal instead: sudo packaging/setup-local-https-linux.sh (see docs/LOCAL_HTTPS.md).".to_string());
    }
    let mut cmd = if elevate && helper_installed {
        // The packaged helper plus its polkit policy: a prompt that says what it is for, and a root-owned
        // script that can't be swapped between the prompt and the run.
        let mut c = Command::new("pkexec");
        c.arg(POLKIT_HELPER);
        c
    } else {
        // No packaged helper (e.g. a user-level install or a dev build): hand the script text to bash.
        // `bash -c SCRIPT NAME ARGS...`: NAME becomes $0, so the script's own "$@" parsing sees ARGS.
        let mut c = if elevate { Command::new("pkexec") } else { Command::new("bash") };
        if elevate {
            c.arg("bash");
        }
        c.arg("-c").arg(LINUX_SCRIPT).arg("setup-local-https");
        c
    };
    cmd.arg("--domain").arg(domain).arg("--port").arg(port.to_string());
    if dry_run {
        cmd.arg("--dry-run");
    }
    Ok(Prepared { cmd, stdin_token: Some(token.to_string()), log_file: None })
}

#[cfg(unix)]
fn which_in_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(name).is_file()))
        .unwrap_or(false)
}

/// Encodes a PowerShell script for `-EncodedCommand` (base64 of UTF-16LE), which avoids every quoting problem.
#[cfg(windows)]
fn encode_ps(script: &str) -> String {
    use base64::Engine;
    let bytes: Vec<u8> = script.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Escapes a value for a single-quoted PowerShell string.
#[cfg(windows)]
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

#[cfg(windows)]
fn build_command(domain: &str, port: u16, dry_run: bool, token: &str) -> Result<Prepared, String> {
    // Real runs need Administrator, so the script is started through UAC (`Start-Process -Verb RunAs`).
    // An elevated process can't be piped, so it appends its output to a log file we follow, and it reads
    // the token from a file in a folder only this user can open (it deletes the file right after).
    let dir = std::env::temp_dir().join(format!("opensanctuary-local-https-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create a temp folder: {e}"))?;
    if let Ok(user) = std::env::var("USERNAME") {
        // Drop inherited permissions and keep full control for this user only.
        let _ = std::process::Command::new("icacls")
            .arg(&dir)
            .args(["/inheritance:r", "/grant:r", &format!("{user}:(OI)(CI)F")])
            .output();
    }
    let script_path = dir.join("setup-local-https-windows.ps1");
    let log_path = dir.join("setup.log");
    let token_path = dir.join("token.txt");
    std::fs::write(&script_path, WINDOWS_SCRIPT).map_err(|e| format!("Could not write the setup script: {e}"))?;
    std::fs::write(&log_path, "").map_err(|e| format!("Could not create the log file: {e}"))?;
    std::fs::write(&token_path, token).map_err(|e| format!("Could not write the token file: {e}"))?;

    let dry = if dry_run { " -DryRun" } else { "" };
    // Runs elevated: the script's output (Write-Host included) is merged and appended line by line.
    let inner = format!(
        "$ErrorActionPreference='Continue'; & {script} -Domain {domain} -Port {port} -TokenFile {token}{dry} *>&1 | ForEach-Object {{ Add-Content -LiteralPath {log} -Value ([string]$_) -Encoding UTF8 }}; exit $LASTEXITCODE",
        script = ps_quote(&script_path.to_string_lossy()),
        domain = ps_quote(domain),
        port = ps_quote(&port.to_string()),
        token = ps_quote(&token_path.to_string_lossy()),
        log = ps_quote(&log_path.to_string_lossy()),
    );
    let elevate = if dry_run { "" } else { " -Verb RunAs" };
    let outer = format!(
        "try {{ $p = Start-Process powershell{elevate} -Wait -PassThru -WindowStyle Hidden -ArgumentList '-NoProfile -ExecutionPolicy Bypass -EncodedCommand {enc}'; exit $p.ExitCode }} catch {{ Write-Output 'Administrator permission was not granted (the Windows prompt was cancelled or refused).'; exit 126 }}",
        enc = encode_ps(&inner),
    );
    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand", &encode_ps(&outer)]);
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    Ok(Prepared { cmd, stdin_token: None, log_file: Some(log_path) })
}

async fn run(task_id: String, prepared: Prepared, secret: String) {
    let Prepared { mut cmd, stdin_token, log_file } = prepared;
    cmd.stdin(if stdin_token.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            finish(&task_id, None, Some(format!("Could not start the setup: {e}")));
            return;
        }
    };
    if let (Some(token), Some(mut stdin)) = (stdin_token, child.stdin.take()) {
        let _ = stdin.write_all(format!("{token}\n").as_bytes()).await;
        drop(stdin);
    }
    let out = child.stdout.take().map(|s| tokio::spawn(pump(task_id.clone(), s, secret.clone())));
    let err = child.stderr.take().map(|s| tokio::spawn(pump(task_id.clone(), s, secret.clone())));
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let tail = log_file.map(|path| tokio::spawn(tail_log(task_id.clone(), path, secret.clone(), stop_rx)));

    let waited = tokio::time::timeout(MAX_RUNTIME, child.wait()).await;
    let (code, error) = match waited {
        Ok(Ok(status)) => {
            let code = status.code();
            let error = (code != Some(0)).then(|| match code {
                // pkexec: 126 = dismissed/not authorized, 127 = authentication failed. Windows wrapper uses 126 too.
                Some(126) | Some(127) => "Administrator permission was not granted.".to_string(),
                Some(c) => format!("The setup script stopped with exit code {c}."),
                None => "The setup script was stopped.".to_string(),
            });
            (code, error)
        }
        Ok(Err(e)) => (None, Some(format!("Lost track of the setup process: {e}"))),
        Err(_) => {
            let _ = child.kill().await;
            (None, Some("The setup took longer than 15 minutes and was stopped.".to_string()))
        }
    };
    // Let the readers drain what the script printed before the status flips to complete.
    for h in [out, err].into_iter().flatten() {
        let _ = h.await;
    }
    let _ = stop_tx.send(true);
    if let Some(h) = tail {
        let _ = h.await;
    }
    finish(&task_id, code, error);
}

/// Follows a log file the elevated script appends to, pushing each new complete line.
async fn tail_log(task_id: String, path: std::path::PathBuf, secret: String, mut stop: tokio::sync::watch::Receiver<bool>) {
    let mut offset: usize = 0;
    let mut partial = String::new();
    loop {
        let stopping = *stop.borrow();
        if let Ok(bytes) = tokio::fs::read(&path).await {
            if bytes.len() > offset {
                partial.push_str(&String::from_utf8_lossy(&bytes[offset..]));
                offset = bytes.len();
            }
            while let Some(i) = partial.find('\n') {
                let line: String = partial.drain(..=i).collect();
                let line = line.trim_end_matches(['\r', '\n']).trim_start_matches('\u{feff}');
                push_line(&task_id, redact(line, Some(&secret)));
            }
        }
        if stopping {
            if !partial.is_empty() {
                push_line(&task_id, redact(partial.trim_end_matches('\r'), Some(&secret)));
            }
            return;
        }
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_millis(300)) => {}
            _ = stop.changed() => {}
        }
    }
}

async fn pump<R: AsyncRead + Unpin>(task_id: String, reader: R, secret: String) {
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        push_line(&task_id, redact(&line, Some(&secret)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_validation_accepts_plain_hostnames() {
        assert_eq!(validate_domain("Connect.YourChurch.org").unwrap(), "connect.yourchurch.org");
        assert_eq!(validate_domain(" a-b.example.com. ").unwrap(), "a-b.example.com");
    }

    #[test]
    fn domain_validation_rejects_anything_that_isnt_a_hostname() {
        for bad in [
            "", "localhost", "https://connect.example.org", "a b.example.org", "ex$(id).org", "a.example.org;rm -rf /",
            "-a.example.org", "a-.example.org", "a..example.org", "a.example.org/path", "под.example.org",
        ] {
            assert!(validate_domain(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn token_validation_rejects_blank_and_whitespace_tokens() {
        assert!(validate_token("   ").is_err());
        assert!(validate_token("abc def").is_err());
        assert!(validate_token("abc\ndef").is_err());
        assert_eq!(validate_token("  abcDEF123_-  ").unwrap(), "abcDEF123_-");
    }

    #[test]
    fn redact_masks_the_token_wherever_it_appears() {
        assert_eq!(redact("token=sekrit and sekrit", Some("sekrit")), "token=******** and ********");
        assert_eq!(redact("nothing here", Some("sekrit")), "nothing here");
        assert_eq!(redact("unchanged", None), "unchanged");
    }

    #[test]
    fn progress_is_incremental_and_old_lines_are_dropped() {
        let id = "lhs_test_incremental";
        registry().lock().unwrap().insert(
            id.to_string(),
            Task { lines: Vec::new(), dropped: 0, status: "running", exit_code: None, error: None },
        );
        for i in 0..(MAX_LINES + 5) {
            push_line(id, format!("line {i}"));
        }
        let first = get_progress(id, 0).unwrap();
        assert_eq!(first.lines.len(), MAX_LINES);
        assert_eq!(first.lines[0], "line 5");
        assert_eq!(first.next, MAX_LINES + 5);
        push_line(id, "last".to_string());
        let second = get_progress(id, first.next).unwrap();
        assert_eq!(second.lines, vec!["last".to_string()]);
        assert!(!second.is_complete);
        finish(id, Some(0), None);
        assert_eq!(get_progress(id, second.next).unwrap().status, "completed");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_streams_output_masks_the_secret_and_reports_the_exit_code() {
        let id = "lhs_test_run".to_string();
        registry().lock().unwrap().insert(
            id.clone(),
            Task { lines: Vec::new(), dropped: 0, status: "running", exit_code: None, error: None },
        );
        let mut cmd = Command::new("sh");
        // Reads the "token" from stdin like the real script does, prints it, writes to stderr, fails.
        cmd.arg("-c").arg("read t; echo got $t; echo oops 1>&2; exit 3");
        run(id.clone(), Prepared { cmd, stdin_token: Some("sekrit".to_string()), log_file: None }, "sekrit".to_string()).await;
        let p = get_progress(&id, 0).unwrap();
        assert!(p.is_complete);
        assert_eq!(p.status, "failed");
        assert_eq!(p.exit_code, Some(3));
        assert!(p.lines.contains(&"got ********".to_string()), "{:?}", p.lines);
        assert!(p.lines.contains(&"oops".to_string()));
        assert!(!p.lines.iter().any(|l| l.contains("sekrit")));
    }

    #[tokio::test]
    async fn tail_log_follows_a_file_strips_the_bom_and_masks_the_secret() {
        let id = "lhs_test_tail".to_string();
        registry().lock().unwrap().insert(
            id.clone(),
            Task { lines: Vec::new(), dropped: 0, status: "running", exit_code: None, error: None },
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("setup.log");
        std::fs::write(&path, "\u{feff}first\r\nsecond sekrit\r\npart").unwrap();
        let (tx, rx) = tokio::sync::watch::channel(false);
        let h = tokio::spawn(tail_log(id.clone(), path, "sekrit".to_string(), rx));
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        tx.send(true).unwrap();
        h.await.unwrap();
        let p = get_progress(&id, 0).unwrap();
        assert_eq!(p.lines, vec!["first".to_string(), "second ********".to_string(), "part".to_string()]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_embedded_linux_script_dry_runs_without_root_and_without_touching_anything() {
        let id = "lhs_test_dry".to_string();
        registry().lock().unwrap().insert(
            id.clone(),
            Task { lines: Vec::new(), dropped: 0, status: "running", exit_code: None, error: None },
        );
        let prepared = build_command("connect.example.org", 8080, true, "dry-run-placeholder").unwrap();
        run(id.clone(), prepared, "dry-run-placeholder".to_string()).await;
        let p = get_progress(&id, 0).unwrap();
        assert_eq!(p.status, "completed", "{:?}", p.lines);
        let log = p.lines.join("\n");
        assert!(log.contains("connect.example.org"), "{log}");
        assert!(log.contains("reverse_proxy localhost:8080"), "{log}");
        assert!(log.contains("resolvers 1.1.1.1"), "{log}");
        assert!(log.contains("+ ufw allow from") || log.contains("firewall"), "{log}");
    }
}
