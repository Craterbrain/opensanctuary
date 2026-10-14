//! Running commands for plugins (docs/PLUGIN_PERMISSIONS.md): the host runs a *fixed command line the
//! plugin declared in its manifest* -- the operator reads it at review -- and nothing else.
//!
//! - The program is a tool the plugin declared and the host manages (`plugin_tools`); the plugin
//!   never names a path.
//! - Arguments come from the manifest's template. Parameters are typed (url / enum / bool / string),
//!   validated by the host, and substituted as whole `argv` elements: there is no shell, and a value
//!   can't become a flag (a value used as a whole argument may not start with `-`).
//! - Each job runs in its own folder under `<data>/plugin-jobs/<plugin>/<job>/`; output is kept as
//!   lines the plugin polls for. Finished files reach the library only through
//!   `library.addMedia`, which accepts a file *from that job's folder* and nowhere else.
//!
//! What the tool itself then does (for example, reaching the network) is not confined by the host:
//! that is exactly what the `process.run` grant tells the operator.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{Map, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;

use crate::core::plugin_permissions::{ArgItem, CommandSpec, ParamSpec, BUILTIN_PLACEHOLDERS};

const MAX_LINE_BYTES: usize = 1024 * 1024;
const MAX_LINES: usize = 5000;
const MAX_RUNNING_PER_PLUGIN: usize = 2;
const MAX_JOBS_KEPT: usize = 200;
const JOB_TIMEOUT: Duration = Duration::from_secs(3 * 60 * 60);
const FINISHED_JOB_TTL: Duration = Duration::from_secs(2 * 60 * 60);
const STRAY_DIR_TTL: Duration = Duration::from_secs(24 * 60 * 60);

// ---- parameters and templates -------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Value3 {
    Bool(bool),
    Text(String),
}

fn has_control(s: &str) -> bool {
    s.chars().any(|c| c.is_control())
}

fn typed(name: &str, spec: &ParamSpec, v: Option<&Value>) -> Result<Option<Value3>, String> {
    let Some(v) = v.filter(|v| !v.is_null()) else {
        return match spec.kind.as_str() {
            "bool" => Ok(Some(Value3::Bool(false))),
            _ if spec.optional => Ok(None),
            _ => Err(format!("missing parameter '{name}'")),
        };
    };
    match spec.kind.as_str() {
        "bool" => v.as_bool().map(|b| Some(Value3::Bool(b))).ok_or_else(|| format!("'{name}' must be true or false")),
        "enum" => {
            let s = v.as_str().ok_or_else(|| format!("'{name}' must be text"))?;
            if spec.values.iter().any(|a| a == s) {
                Ok(Some(Value3::Text(s.to_string())))
            } else {
                Err(format!("'{name}' must be one of: {}", spec.values.join(", ")))
            }
        }
        "url" => {
            let s = v.as_str().ok_or_else(|| format!("'{name}' must be text"))?.trim();
            if s.is_empty() || s.len() > 2048 || has_control(s) || s.chars().any(char::is_whitespace) || s.starts_with('-') {
                return Err(format!("'{name}' isn't a valid URL"));
            }
            let url = reqwest::Url::parse(s).map_err(|_| format!("'{name}' isn't a valid URL"))?;
            if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
                return Err(format!("'{name}' must be an http:// or https:// URL"));
            }
            Ok(Some(Value3::Text(url.to_string())))
        }
        "string" => {
            let s = v.as_str().ok_or_else(|| format!("'{name}' must be text"))?.trim();
            if s.is_empty() && !spec.optional {
                return Err(format!("'{name}' can't be empty"));
            }
            if s.len() > spec.max_len.unwrap_or(200).min(2000) || has_control(s) {
                return Err(format!("'{name}' is too long or has control characters"));
            }
            Ok(Some(Value3::Text(s.to_string())))
        }
        other => Err(format!("unsupported parameter type '{other}'")),
    }
}

/// Expands a manifest command template into the exact `argv` (after the program). `output_dir` is
/// the job's folder; `ffmpeg_dir` the folder of a bundled ffmpeg, when there is one.
pub fn expand(spec: &CommandSpec, params: &Map<String, Value>, output_dir: &Path, ffmpeg_dir: Option<&Path>) -> Result<Vec<String>, String> {
    for k in params.keys() {
        if !spec.params.contains_key(k) {
            return Err(format!("unknown parameter '{k}'"));
        }
    }
    let mut values: HashMap<&str, Value3> = HashMap::new();
    for (name, pspec) in &spec.params {
        if let Some(v) = typed(name, pspec, params.get(name))? {
            values.insert(name.as_str(), v);
        }
    }
    let truthy = |name: &str| -> bool {
        match name {
            "output_dir" => true,
            "ffmpeg_dir" => ffmpeg_dir.is_some(),
            _ => matches!(values.get(name), Some(Value3::Bool(true)) | Some(Value3::Text(_))),
        }
    };
    let expand_one = |arg: &str| -> Result<String, String> {
        let whole = arg.strip_prefix('{').and_then(|r| r.strip_suffix('}')).filter(|n| !n.contains(['{', '}']));
        let mut out = String::new();
        let mut rest = arg;
        while let Some(start) = rest.find('{') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            let Some(end) = after.find('}') else {
                out.push_str(&rest[start..]);
                rest = "";
                break;
            };
            let name = &after[..end];
            let text = match name {
                "output_dir" => output_dir.to_string_lossy().to_string(),
                "ffmpeg_dir" => ffmpeg_dir.map(|p| p.to_string_lossy().to_string()).ok_or("ffmpeg_dir isn't available")?,
                _ => match values.get(name) {
                    Some(Value3::Text(t)) => t.clone(),
                    Some(Value3::Bool(_)) => return Err(format!("'{name}' is a boolean and can't be substituted")),
                    None => return Err(format!("parameter '{name}' wasn't given")),
                },
            };
            // A value that stands alone as an argument must never be mistaken for a flag.
            if whole == Some(name) && !BUILTIN_PLACEHOLDERS.contains(&name) && text.starts_with('-') {
                return Err(format!("'{name}' can't start with '-'"));
            }
            out.push_str(&text);
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        Ok(out)
    };
    let mut argv = Vec::new();
    for item in &spec.args {
        match item {
            ArgItem::Literal(s) => argv.push(expand_one(s)?),
            ArgItem::Conditional { if_param, args, else_args } => {
                let chosen = if truthy(if_param) { args } else { else_args };
                for a in chosen {
                    argv.push(expand_one(a)?);
                }
            }
        }
    }
    Ok(argv)
}

// ---- jobs ---------------------------------------------------------------------------------------

pub struct JobState {
    lines: VecDeque<String>,
    /// Absolute index of `lines[0]` (older lines are dropped once there are `MAX_LINES`).
    first_index: usize,
    pub status: &'static str, // running | exited | failed | cancelled | timeout
    pub exit_code: Option<i32>,
    finished_at: Option<Instant>,
}

pub struct Job {
    pub id: String,
    pub plugin_id: String,
    pub command: String,
    pub output_dir: PathBuf,
    pub state: Mutex<JobState>,
    cancel: tokio::sync::Notify,
}

fn registry() -> &'static Mutex<HashMap<String, Arc<Job>>> {
    static REG: OnceLock<Mutex<HashMap<String, Arc<Job>>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn job_for(plugin_id: &str, job_id: &str) -> Option<Arc<Job>> {
    registry().lock().unwrap_or_else(|e| e.into_inner()).get(job_id).filter(|j| j.plugin_id == plugin_id).cloned()
}

fn push_line(job: &Job, mut line: String) {
    if line.len() > MAX_LINE_BYTES {
        let mut cut = MAX_LINE_BYTES;
        while !line.is_char_boundary(cut) {
            cut -= 1;
        }
        line.truncate(cut);
    }
    let mut st = job.state.lock().unwrap_or_else(|e| e.into_inner());
    st.lines.push_back(line);
    if st.lines.len() > MAX_LINES {
        st.lines.pop_front();
        st.first_index += 1;
    }
}

/// Reads lines without ever holding more than `MAX_LINE_BYTES` of a single one (the rest of an
/// over-long line is discarded), so a tool can't exhaust memory with a line that never ends.
async fn read_capped_lines<R: AsyncRead + Unpin>(reader: R, job: Arc<Job>) {
    let mut reader = BufReader::new(reader);
    let mut line: Vec<u8> = Vec::new();
    let mut discarding = false;
    loop {
        let (consumed, found_newline) = {
            let buf = match reader.fill_buf().await {
                Ok([]) | Err(_) => break,
                Ok(b) => b,
            };
            match buf.iter().position(|b| *b == b'\n') {
                Some(i) => {
                    if !discarding {
                        let room = MAX_LINE_BYTES.saturating_sub(line.len());
                        line.extend_from_slice(&buf[..i.min(room)]);
                    }
                    (i + 1, true)
                }
                None => {
                    if !discarding {
                        let room = MAX_LINE_BYTES.saturating_sub(line.len());
                        line.extend_from_slice(&buf[..buf.len().min(room)]);
                        if buf.len() > room {
                            discarding = true;
                        }
                    }
                    (buf.len(), false)
                }
            }
        };
        reader.consume(consumed);
        if found_newline {
            push_line(&job, String::from_utf8_lossy(&line).trim_end_matches('\r').to_string());
            line.clear();
            discarding = false;
        }
    }
    if !line.is_empty() {
        push_line(&job, String::from_utf8_lossy(&line).trim_end_matches('\r').to_string());
    }
}

fn prune(jobs_root: &Path) {
    let mut reg = registry().lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    let expired: Vec<String> = reg
        .iter()
        .filter(|(_, j)| {
            let st = j.state.lock().unwrap_or_else(|e| e.into_inner());
            st.finished_at.is_some_and(|t| now.duration_since(t) > FINISHED_JOB_TTL)
        })
        .map(|(id, _)| id.clone())
        .collect();
    for id in expired {
        if let Some(j) = reg.remove(&id) {
            let _ = std::fs::remove_dir_all(&j.output_dir);
        }
    }
    // Oldest finished jobs go first if there are still too many.
    while reg.len() > MAX_JOBS_KEPT {
        let oldest = reg
            .iter()
            .filter_map(|(id, j)| j.state.lock().unwrap_or_else(|e| e.into_inner()).finished_at.map(|t| (id.clone(), t)))
            .min_by_key(|(_, t)| *t)
            .map(|(id, _)| id);
        match oldest.and_then(|id| reg.remove(&id)) {
            Some(j) => {
                let _ = std::fs::remove_dir_all(&j.output_dir);
            }
            None => break,
        }
    }
    drop(reg);
    // Folders left behind by an earlier run of the server.
    if let Ok(plugins) = std::fs::read_dir(jobs_root) {
        for plugin in plugins.flatten() {
            if let Ok(jobs) = std::fs::read_dir(plugin.path()) {
                for dir in jobs.flatten() {
                    let known = registry().lock().unwrap_or_else(|e| e.into_inner()).contains_key(dir.file_name().to_string_lossy().as_ref());
                    let old = dir.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > STRAY_DIR_TTL);
                    if !known && old {
                        let _ = std::fs::remove_dir_all(dir.path());
                    }
                }
            }
        }
    }
}

/// Starts a job. `program` is the already-resolved tool; `argv` the expanded arguments.
pub fn start(
    data_dir: &Path,
    plugin_id: &str,
    command_id: &str,
    program: PathBuf,
    argv_builder: impl FnOnce(&Path) -> Result<Vec<String>, String>,
) -> Result<Arc<Job>, String> {
    let jobs_root = data_dir.join("plugin-jobs");
    prune(&jobs_root);
    let running = registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .filter(|j| j.plugin_id == plugin_id && j.state.lock().unwrap_or_else(|e| e.into_inner()).status == "running")
        .count();
    if running >= MAX_RUNNING_PER_PLUGIN {
        return Err(format!("this plugin already has {MAX_RUNNING_PER_PLUGIN} jobs running"));
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    let output_dir = jobs_root.join(plugin_id).join(&id);
    std::fs::create_dir_all(&output_dir).map_err(|e| format!("can't create the job folder: {e}"))?;
    let argv = match argv_builder(&output_dir) {
        Ok(a) => a,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&output_dir);
            return Err(e);
        }
    };
    let job = Arc::new(Job {
        id: id.clone(),
        plugin_id: plugin_id.to_string(),
        command: command_id.to_string(),
        output_dir: output_dir.clone(),
        state: Mutex::new(JobState { lines: VecDeque::new(), first_index: 0, status: "running", exit_code: None, finished_at: None }),
        cancel: tokio::sync::Notify::new(),
    });
    registry().lock().unwrap_or_else(|e| e.into_inner()).insert(id, job.clone());

    let runner = job.clone();
    tokio::spawn(async move {
        let finish = |status: &'static str, code: Option<i32>| {
            let mut st = runner.state.lock().unwrap_or_else(|e| e.into_inner());
            st.status = status;
            st.exit_code = code;
            st.finished_at = Some(Instant::now());
        };
        let mut cmd = Command::new(&program);
        cmd.args(&argv)
            .current_dir(&runner.output_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                push_line(&runner, format!("could not start {}: {e}", program.display()));
                return finish("failed", None);
            }
        };
        let out_task = child.stdout.take().map(|s| tokio::spawn(read_capped_lines(s, runner.clone())));
        let err_task = child.stderr.take().map(|s| tokio::spawn(read_capped_lines(s, runner.clone())));
        let outcome = tokio::select! {
            r = child.wait() => r.map(|s| ("exited", s.code())).map_err(|e| e.to_string()),
            _ = runner.cancel.notified() => { let _ = child.kill().await; Ok(("cancelled", None)) }
            _ = tokio::time::sleep(JOB_TIMEOUT) => { let _ = child.kill().await; Ok(("timeout", None)) }
        };
        // Let the readers drain what the process wrote before it died.
        for t in [out_task, err_task].into_iter().flatten() {
            let _ = tokio::time::timeout(Duration::from_secs(5), t).await;
        }
        match outcome {
            Ok((status, code)) => finish(status, code),
            Err(e) => {
                push_line(&runner, format!("lost the process: {e}"));
                finish("failed", None)
            }
        }
    });
    Ok(job)
}

pub fn cancel(job: &Job) {
    job.cancel.notify_one();
}

/// Forgets a job and deletes its folder.
pub fn release(plugin_id: &str, job_id: &str) -> bool {
    let removed = {
        let mut reg = registry().lock().unwrap_or_else(|e| e.into_inner());
        match reg.get(job_id) {
            Some(j) if j.plugin_id == plugin_id => reg.remove(job_id),
            _ => None,
        }
    };
    match removed {
        Some(j) => {
            // A job still running is stopped first.
            j.cancel.notify_one();
            let _ = std::fs::remove_dir_all(&j.output_dir);
            true
        }
        None => false,
    }
}

/// Output since `since` (an absolute line index), plus status; the finished job's files too.
pub fn poll(job: &Job, since: usize) -> Value {
    let st = job.state.lock().unwrap_or_else(|e| e.into_inner());
    let start = since.max(st.first_index);
    let skip = start - st.first_index;
    let lines: Vec<&String> = st.lines.iter().skip(skip).take(500).collect();
    let next = start + lines.len();
    let mut files = Vec::new();
    if st.status != "running" {
        if let Ok(rd) = std::fs::read_dir(&job.output_dir) {
            for e in rd.flatten().take(200) {
                if let Ok(m) = std::fs::symlink_metadata(e.path()) {
                    let name = e.file_name().to_string_lossy().to_string();
                    if m.is_file() && !name.starts_with('.') {
                        files.push(serde_json::json!({ "name": name, "size": m.len() }));
                    }
                }
            }
        }
    }
    serde_json::json!({
        "status": st.status, "exit_code": st.exit_code, "lines": lines, "next": next, "files": files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(json: serde_json::Value) -> CommandSpec {
        serde_json::from_value(json).unwrap()
    }

    fn ytdlp_like() -> CommandSpec {
        spec(serde_json::json!({
            "id": "download", "tool": "yt-dlp",
            "args": [
                "--no-playlist", "-o", "{output_dir}/%(title)s.%(ext)s",
                { "if": "ffmpeg_dir", "args": ["--ffmpeg-location", "{ffmpeg_dir}"] },
                { "if": "sponsorblock", "args": ["--sponsorblock-remove", "all"] },
                { "if": "browser", "args": ["--cookies-from-browser", "{browser}"] },
                { "if": "audio_only", "args": ["-x"], "else": ["-f", "best"] },
                "--", "{url}"
            ],
            "params": {
                "url": { "type": "url" },
                "browser": { "type": "enum", "values": ["chrome", "firefox"], "optional": true },
                "audio_only": { "type": "bool" }, "sponsorblock": { "type": "bool" },
                "note": { "type": "string", "optional": true, "max_len": 20 }
            }
        }))
    }

    fn params(v: serde_json::Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn expands_a_template_into_exact_argv() {
        let out = Path::new("/jobs/j1");
        let argv = expand(&ytdlp_like(), &params(serde_json::json!({ "url": "https://example.com/v?id=1" })), out, None).unwrap();
        assert_eq!(argv, ["--no-playlist", "-o", "/jobs/j1/%(title)s.%(ext)s", "-f", "best", "--", "https://example.com/v?id=1"]);
        let argv = expand(&ytdlp_like(), &params(serde_json::json!({
            "url": "https://example.com/v", "audio_only": true, "sponsorblock": true, "browser": "firefox" })), out, Some(Path::new("/opt/ffmpeg"))).unwrap();
        assert_eq!(argv, ["--no-playlist", "-o", "/jobs/j1/%(title)s.%(ext)s", "--ffmpeg-location", "/opt/ffmpeg",
                          "--sponsorblock-remove", "all", "--cookies-from-browser", "firefox", "-x", "--", "https://example.com/v"]);
    }

    #[test]
    fn refuses_anything_that_could_change_the_command() {
        let out = Path::new("/jobs/j1");
        let run = |p: serde_json::Value| expand(&ytdlp_like(), &params(p), out, None);
        // Not a URL, wrong scheme, whitespace, a flag in disguise, missing.
        for bad in ["--exec rm -rf /", "-o /etc/passwd", "file:///etc/passwd", "ftp://x/y", "javascript:alert(1)", "https://x y", "https://example.com/\nfoo", "", "  "] {
            assert!(run(serde_json::json!({ "url": bad })).is_err(), "should refuse url {bad:?}");
        }
        assert!(run(serde_json::json!({})).is_err());
        assert!(run(serde_json::json!({ "url": 5 })).is_err());
        // Unknown parameters can't smuggle arguments in.
        assert!(run(serde_json::json!({ "url": "https://x.test/", "extra": "--exec x" })).is_err());
        // Enum, bool and string rules.
        assert!(run(serde_json::json!({ "url": "https://x.test/", "browser": "chrome; rm -rf" })).is_err());
        assert!(run(serde_json::json!({ "url": "https://x.test/", "browser": "--evil" })).is_err());
        assert!(run(serde_json::json!({ "url": "https://x.test/", "audio_only": "yes" })).is_err());
        assert!(run(serde_json::json!({ "url": "https://x.test/", "note": "x".repeat(21) })).is_err());
        assert!(run(serde_json::json!({ "url": "https://x.test/", "note": "bad\nnote" })).is_err());
        // A URL with shell metacharacters is just one argument, untouched by any shell.
        let argv = run(serde_json::json!({ "url": "https://x.test/a;rm%20-rf?x=$(id)&y=`id`" })).unwrap();
        assert_eq!(argv.last().unwrap(), "https://x.test/a;rm%20-rf?x=$(id)&y=`id`");
        assert_eq!(argv.iter().filter(|a| a.contains("rm%20-rf")).count(), 1);
    }

    #[test]
    fn a_string_that_stands_alone_cannot_start_with_a_dash() {
        let s = spec(serde_json::json!({
            "id": "c", "tool": "t", "args": ["{name}", "--label={name}"],
            "params": { "name": { "type": "string" } }
        }));
        let out = Path::new("/j");
        assert!(expand(&s, &params(serde_json::json!({ "name": "-x" })), out, None).is_err());
        assert_eq!(expand(&s, &params(serde_json::json!({ "name": "ok-name" })), out, None).unwrap(), ["ok-name", "--label=ok-name"]);
        // Inside a larger argument a leading dash is harmless: the flag is the literal part.
        let t = spec(serde_json::json!({ "id": "c", "tool": "t", "args": ["--label={name}"], "params": { "name": { "type": "string" } } }));
        assert_eq!(expand(&t, &params(serde_json::json!({ "name": "-x" })), out, None).unwrap(), ["--label=-x"]);
    }

    async fn wait_done(job: &Job) {
        for _ in 0..200 {
            if job.state.lock().unwrap().status != "running" {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("job didn't finish");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn runs_a_job_collects_output_and_files_and_can_be_released() {
        let dir = tempfile::tempdir().unwrap();
        let job = start(dir.path(), "plug", "go", PathBuf::from("/bin/sh"), |out| {
            Ok(vec!["-c".into(), format!("echo one; echo two 1>&2; printf partial; echo done > '{}/result.txt'; exit 3", out.display())])
        })
        .unwrap();
        wait_done(&job).await;
        let v = poll(&job, 0);
        assert_eq!(v["status"], "exited");
        assert_eq!(v["exit_code"], 3);
        let lines: Vec<&str> = v["lines"].as_array().unwrap().iter().map(|l| l.as_str().unwrap()).collect();
        assert!(lines.contains(&"one") && lines.contains(&"two") && lines.contains(&"partial"), "{lines:?}");
        assert_eq!(v["files"][0]["name"], "result.txt");
        // Polling from the returned index only gives what's new.
        let next = v["next"].as_u64().unwrap() as usize;
        assert!(poll(&job, next)["lines"].as_array().unwrap().is_empty());
        // Another plugin can't see or release it.
        assert!(job_for("someone-else", &job.id).is_none());
        assert!(!release("someone-else", &job.id));
        let out_dir = job.output_dir.clone();
        assert!(release("plug", &job.id));
        assert!(!out_dir.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancel_stops_a_running_job_and_the_per_plugin_limit_holds() {
        let dir = tempfile::tempdir().unwrap();
        let sleeper = || start(dir.path(), "limited", "go", PathBuf::from("/bin/sh"), |_| Ok(vec!["-c".into(), "sleep 30".into()]));
        let a = sleeper().unwrap();
        let _b = sleeper().unwrap();
        assert!(sleeper().is_err(), "a third concurrent job is refused");
        cancel(&a);
        wait_done(&a).await;
        assert_eq!(a.state.lock().unwrap().status, "cancelled");
        assert!(sleeper().is_ok(), "a slot frees up once a job ends");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_endless_line_is_capped_not_buffered() {
        let dir = tempfile::tempdir().unwrap();
        let job = start(dir.path(), "caps", "go", PathBuf::from("/bin/sh"), |_| {
            Ok(vec!["-c".into(), "head -c 3000000 /dev/zero | tr '\\0' 'a'; echo; echo after".into()])
        })
        .unwrap();
        wait_done(&job).await;
        let v = poll(&job, 0);
        let lines = v["lines"].as_array().unwrap();
        assert_eq!(lines[0].as_str().unwrap().len(), MAX_LINE_BYTES);
        assert_eq!(lines.last().unwrap(), "after");
    }

    #[tokio::test]
    async fn a_missing_program_fails_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let job = start(dir.path(), "missing", "go", PathBuf::from("/definitely/not/here"), |_| Ok(vec![])).unwrap();
        wait_done(&job).await;
        assert_eq!(job.state.lock().unwrap().status, "failed");
        assert!(poll(&job, 0)["lines"][0].as_str().unwrap().contains("could not start"));
    }
}
