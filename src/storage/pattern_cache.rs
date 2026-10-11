//! Cache of pre-rendered animated slide backgrounds.
//!
//! Animated "pattern:<key>" backgrounds are procedural (SVG/CSS), which a slow
//! TV browser has to redraw every frame. The operator console can instead draw
//! every frame of one loop off-screen, POST them here, and have `ffmpeg` encode
//! them to a VP9 `.webm` that the live output plays back as an ordinary looping
//! video background (hardware-decoded, same codec policy as `transcode.rs`).
//!
//! Clips live in `<media_dir>/pattern_cache/` (served at `/media/pattern_cache/`
//! by the existing static route, which already handles HTTP range requests).
//! The file name encodes everything that changes the picture -- pattern key, a
//! revision hash of the pattern's definition, size and frame rate -- so editing
//! a pattern or changing resolution simply produces a new clip. Clips that
//! haven't been used for [`CACHE_TTL`] (3 months) are deleted by
//! [`maintenance_loop`]; every status check that finds a clip "uses" it by
//! refreshing its modified time.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};

use tokio::io::AsyncWriteExt;
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::Mutex;

/// Unused clips are deleted after this long (three months).
pub const CACHE_TTL: Duration = Duration::from_secs(90 * 24 * 60 * 60);
/// A render that hasn't received a frame for this long is abandoned.
const JOB_IDLE_LIMIT: Duration = Duration::from_secs(5 * 60);
/// Only refresh a clip's "last used" time when it is at least this stale.
const TOUCH_GRANULARITY: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_CONCURRENT_JOBS: usize = 2;
const MAX_FRAMES: u32 = 4800;

pub const CACHE_SUBDIR: &str = "pattern_cache";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipSpec {
    pub key: String,
    pub rev: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

fn is_slug(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl ClipSpec {
    pub fn validate(&self) -> Result<(), String> {
        if !is_slug(&self.key, 48) {
            return Err("invalid pattern key".into());
        }
        if !is_slug(&self.rev, 24) {
            return Err("invalid revision".into());
        }
        if !(320..=3840).contains(&self.width) || !(180..=2160).contains(&self.height) {
            return Err("unsupported resolution".into());
        }
        if self.width % 2 != 0 || self.height % 2 != 0 {
            return Err("width and height must be even (yuv420p)".into());
        }
        if !(10..=60).contains(&self.fps) {
            return Err("unsupported frame rate".into());
        }
        Ok(())
    }

    pub fn file_name(&self) -> String {
        format!("{}_{}_{}x{}_{}fps.webm", self.key, self.rev, self.width, self.height, self.fps)
    }
}

pub fn cache_dir(media_dir: &Path) -> PathBuf {
    media_dir.join(CACHE_SUBDIR)
}

pub fn clip_url(spec: &ClipSpec) -> String {
    format!("/media/{}/{}", CACHE_SUBDIR, spec.file_name())
}

struct Job {
    spec: ClipSpec,
    stdin: Option<ChildStdin>,
    child: Child,
    part_path: PathBuf,
    final_path: PathBuf,
    frames_expected: u32,
    frames_received: u32,
    last_activity: std::time::Instant,
}

/// `name` is kept outside the job lock so status checks never block on a
/// render that is busy writing a frame.
struct JobEntry {
    name: String,
    job: Arc<Mutex<Job>>,
}

type Jobs = Mutex<HashMap<String, JobEntry>>;

fn jobs() -> &'static Jobs {
    static JOBS: OnceLock<Jobs> = OnceLock::new();
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Ready(String),
    Rendering,
    Missing,
}

/// Where a clip is: finished, being rendered by some client, or absent.
/// A finished clip counts as "used" and has its age refreshed.
pub async fn status(media_dir: &Path, spec: &ClipSpec) -> Status {
    let path = cache_dir(media_dir).join(spec.file_name());
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.is_file() && meta.len() > 0 {
            touch_if_stale(&path, &meta);
            return Status::Ready(clip_url(spec));
        }
    }
    let name = spec.file_name();
    if jobs().lock().await.values().any(|j| j.name == name) {
        return Status::Rendering;
    }
    Status::Missing
}

fn touch_if_stale(path: &Path, meta: &std::fs::Metadata) {
    let age = meta.modified().ok().and_then(|m| SystemTime::now().duration_since(m).ok()).unwrap_or_default();
    if age >= TOUCH_GRANULARITY {
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(path) {
            let _ = f.set_modified(SystemTime::now());
        }
    }
}

/// Best finished clip for a pattern revision at any resolution (the live output
/// doesn't know which size the console chose to render). Prefers the widest
/// and counts as a use.
pub fn find_best(media_dir: &Path, key: &str, rev: &str) -> Option<String> {
    if !is_slug(key, 48) || !is_slug(rev, 24) {
        return None;
    }
    let prefix = format!("{key}_{rev}_");
    let mut best: Option<(u32, String, PathBuf, std::fs::Metadata)> = None;
    for e in std::fs::read_dir(cache_dir(media_dir)).ok()?.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Some(rest) = name.strip_prefix(&prefix) else { continue };
        if !name.ends_with(".webm") {
            continue;
        }
        let width: u32 = rest.split('x').next().and_then(|w| w.parse().ok()).unwrap_or(0);
        let Ok(meta) = e.metadata() else { continue };
        if meta.len() == 0 || width == 0 {
            continue;
        }
        if best.as_ref().map(|b| width > b.0).unwrap_or(true) {
            best = Some((width, name, e.path(), meta));
        }
    }
    let (_, name, path, meta) = best?;
    touch_if_stale(&path, &meta);
    Some(format!("/media/{}/{}", CACHE_SUBDIR, name))
}

pub enum StartOutcome {
    Ready(String),
    /// Someone else is already rendering this exact clip.
    AlreadyRendering,
    Started { job_id: String },
}

pub async fn start(media_dir: &Path, spec: ClipSpec, frames: u32) -> Result<StartOutcome, (u16, String)> {
    spec.validate().map_err(|e| (400, e))?;
    if frames == 0 || frames > MAX_FRAMES {
        return Err((400, "unsupported frame count".into()));
    }
    match status(media_dir, &spec).await {
        Status::Ready(url) => return Ok(StartOutcome::Ready(url)),
        Status::Rendering => return Ok(StartOutcome::AlreadyRendering),
        Status::Missing => {}
    }
    if !crate::storage::transcode::ffmpeg_available().await {
        return Err((503, "ffmpeg is not available".into()));
    }

    let mut map = jobs().lock().await;
    if map.len() >= MAX_CONCURRENT_JOBS {
        return Err((429, "too many renders in progress".into()));
    }
    let dir = cache_dir(media_dir);
    std::fs::create_dir_all(&dir).map_err(|e| (500, format!("cache dir: {e}")))?;
    let final_path = dir.join(spec.file_name());
    let part_path = dir.join(format!("{}.part", spec.file_name()));

    let mut child = Command::new(crate::storage::paths::resolve_ffmpeg_command())
        .args(["-y", "-v", "error", "-f", "image2pipe", "-framerate"])
        .arg(spec.fps.to_string())
        .args(["-c:v", "png", "-i", "-"])
        // VP9 is the project's video codec of choice (see transcode.rs). CRF mode
        // with b:v 0 = constant quality; the soft, low-detail backgrounds are tiny.
        .args(["-c:v", "libvpx-vp9", "-pix_fmt", "yuv420p", "-b:v", "0", "-crf", "32"])
        .args(["-row-mt", "1", "-deadline", "good", "-cpu-used", "4", "-g"])
        .arg((spec.fps * 2).to_string())
        .args(["-an", "-f", "webm"])
        .arg(&part_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| (500, format!("failed to start ffmpeg: {e}")))?;
    let stdin = child.stdin.take();

    let job_id = uuid::Uuid::new_v4().to_string();
    let name = spec.file_name();
    map.insert(
        job_id.clone(),
        JobEntry {
            name,
            job: Arc::new(Mutex::new(Job {
                spec,
                stdin,
                child,
                part_path,
                final_path,
                frames_expected: frames,
                frames_received: 0,
                last_activity: std::time::Instant::now(),
            })),
        },
    );
    Ok(StartOutcome::Started { job_id })
}

async fn get_job(job_id: &str) -> Result<Arc<Mutex<Job>>, (u16, String)> {
    jobs().lock().await.get(job_id).map(|e| e.job.clone()).ok_or((404, "unknown render job".into()))
}

/// Feeds one PNG frame (frames must arrive in order, one at a time).
pub async fn push_frame(job_id: &str, png: &[u8]) -> Result<u32, (u16, String)> {
    if png.len() < 8 || &png[..4] != b"\x89PNG" {
        return Err((400, "frame is not a PNG".into()));
    }
    let job = get_job(job_id).await?;
    let mut job = job.lock().await;
    if job.frames_received >= job.frames_expected {
        return Err((400, "more frames than announced".into()));
    }
    let Some(stdin) = job.stdin.as_mut() else {
        return Err((409, "render already finished".into()));
    };
    if let Err(e) = stdin.write_all(png).await {
        drop(job);
        abort(job_id).await;
        return Err((500, format!("encoder stopped: {e}")));
    }
    job.frames_received += 1;
    job.last_activity = std::time::Instant::now();
    Ok(job.frames_received)
}

/// Closes the encoder input, waits for ffmpeg, and publishes the clip.
pub async fn finish(job_id: &str) -> Result<String, (u16, String)> {
    let job = get_job(job_id).await?;
    let (url, part, fin) = {
        let mut j = job.lock().await;
        if j.frames_received != j.frames_expected {
            let msg = format!("expected {} frames, got {}", j.frames_expected, j.frames_received);
            drop(j);
            abort(job_id).await;
            return Err((400, msg));
        }
        j.stdin.take(); // EOF
        (clip_url(&j.spec), j.part_path.clone(), j.final_path.clone())
    };
    // Wait outside the job lock so status checks stay responsive.
    let result = {
        let mut j = job.lock().await;
        j.child.wait().await
    };
    jobs().lock().await.remove(job_id);
    match result {
        Ok(st) if st.success() => {
            std::fs::rename(&part, &fin).map_err(|e| (500, format!("publish clip: {e}")))?;
            Ok(url)
        }
        _ => {
            let _ = std::fs::remove_file(&part);
            Err((500, "encoder failed".into()))
        }
    }
}

/// Cancels a render and removes its partial file.
pub async fn abort(job_id: &str) {
    if let Some(entry) = jobs().lock().await.remove(job_id) {
        let mut j = entry.job.lock().await;
        j.stdin.take();
        let _ = j.child.start_kill();
        let _ = std::fs::remove_file(&j.part_path);
    }
}

#[derive(Debug, Default, serde::Serialize)]
pub struct CacheInfo {
    pub files: usize,
    pub bytes: u64,
    pub ttl_days: u64,
}

pub fn info(media_dir: &Path) -> CacheInfo {
    let mut out = CacheInfo { ttl_days: CACHE_TTL.as_secs() / 86400, ..Default::default() };
    if let Ok(rd) = std::fs::read_dir(cache_dir(media_dir)) {
        for e in rd.flatten() {
            if let Ok(m) = e.metadata() {
                if m.is_file() && e.path().extension().map(|x| x == "webm").unwrap_or(false) {
                    out.files += 1;
                    out.bytes += m.len();
                }
            }
        }
    }
    out
}

/// Deletes every cached clip (and stray partial files). Returns how many.
pub fn clear(media_dir: &Path) -> usize {
    let mut n = 0;
    if let Ok(rd) = std::fs::read_dir(cache_dir(media_dir)) {
        for e in rd.flatten() {
            if e.path().is_file() && std::fs::remove_file(e.path()).is_ok() {
                n += 1;
            }
        }
    }
    n
}

/// Removes clips not used within `ttl` and partial files from dead renders.
/// Returns the number of files removed.
pub fn prune(media_dir: &Path, ttl: Duration, now: SystemTime) -> usize {
    let mut removed = 0;
    let Ok(rd) = std::fs::read_dir(cache_dir(media_dir)) else { return 0 };
    for e in rd.flatten() {
        let path = e.path();
        let Ok(meta) = e.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let age = meta.modified().ok().and_then(|m| now.duration_since(m).ok()).unwrap_or_default();
        let is_part = path.extension().map(|x| x == "part").unwrap_or(false);
        // A .part younger than the job idle limit may belong to a live render.
        let limit = if is_part { JOB_IDLE_LIMIT * 2 } else { ttl };
        if age > limit && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

async fn reap_idle_jobs() {
    let ids: Vec<String> = {
        let map = jobs().lock().await;
        let mut stale = Vec::new();
        for (id, j) in map.iter() {
            if let Ok(j) = j.job.try_lock() {
                if j.last_activity.elapsed() > JOB_IDLE_LIMIT {
                    stale.push(id.clone());
                }
            }
        }
        stale
    };
    for id in ids {
        abort(&id).await;
    }
}

/// Startup + hourly: delete expired clips and abandon stuck renders.
pub async fn maintenance_loop(media_dir: PathBuf) {
    let mut interval = tokio::time::interval(Duration::from_secs(60 * 60));
    loop {
        interval.tick().await;
        reap_idle_jobs().await;
        let dir = media_dir.clone();
        if let Ok(n) = tokio::task::spawn_blocking(move || prune(&dir, CACHE_TTL, SystemTime::now())).await {
            if n > 0 {
                tracing::info!("pattern cache: removed {n} unused file(s)");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> ClipSpec {
        ClipSpec { key: "sprouting-leaves".into(), rev: "ab12cd".into(), width: 640, height: 360, fps: 10 }
    }

    #[test]
    fn validates_and_names_clips() {
        assert!(spec().validate().is_ok());
        assert_eq!(spec().file_name(), "sprouting-leaves_ab12cd_640x360_10fps.webm");
        for bad in [
            ClipSpec { key: "../etc".into(), ..spec() },
            ClipSpec { key: "A".into(), ..spec() },
            ClipSpec { rev: "".into(), ..spec() },
            ClipSpec { width: 641, ..spec() },
            ClipSpec { width: 8000, ..spec() },
            ClipSpec { fps: 240, ..spec() },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn prune_removes_only_expired_clips() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache_dir(dir.path());
        std::fs::create_dir_all(&cache).unwrap();
        let now = SystemTime::now();
        let write = |name: &str, age: Duration| {
            let p = cache.join(name);
            std::fs::write(&p, b"x").unwrap();
            std::fs::OpenOptions::new().write(true).open(&p).unwrap().set_modified(now - age).unwrap();
        };
        write("fresh.webm", Duration::from_secs(60));
        write("two_months.webm", Duration::from_secs(60 * 86400));
        write("four_months.webm", Duration::from_secs(120 * 86400));
        write("dead.webm.part", Duration::from_secs(3600));
        write("live.webm.part", Duration::from_secs(10));
        assert_eq!(prune(dir.path(), CACHE_TTL, now), 2);
        assert!(cache.join("fresh.webm").exists());
        assert!(cache.join("two_months.webm").exists());
        assert!(!cache.join("four_months.webm").exists());
        assert!(!cache.join("dead.webm.part").exists());
        assert!(cache.join("live.webm.part").exists());
        assert_eq!(info(dir.path()).files, 2);
        assert_eq!(info(dir.path()).ttl_days, 90);
        assert_eq!(clear(dir.path()), 3);
    }

    #[test]
    fn find_best_prefers_widest_matching_revision() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache_dir(dir.path());
        std::fs::create_dir_all(&cache).unwrap();
        for n in [
            "rain_aaa_1920x1080_30fps.webm",
            "rain_aaa_2560x1440_30fps.webm",
            "rain_bbb_3840x2160_30fps.webm",
            "rain-heavy_aaa_3840x2160_30fps.webm",
        ] {
            std::fs::write(cache.join(n), b"x").unwrap();
        }
        assert_eq!(find_best(dir.path(), "rain", "aaa").unwrap(), "/media/pattern_cache/rain_aaa_2560x1440_30fps.webm");
        assert!(find_best(dir.path(), "rain", "ccc").is_none());
        assert!(find_best(dir.path(), "../x", "aaa").is_none());
    }

    #[tokio::test]
    async fn status_touches_stale_clip_and_reports_ready() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache_dir(dir.path());
        std::fs::create_dir_all(&cache).unwrap();
        let s = spec();
        let p = cache.join(s.file_name());
        std::fs::write(&p, b"webm").unwrap();
        let old = SystemTime::now() - Duration::from_secs(30 * 86400);
        std::fs::OpenOptions::new().write(true).open(&p).unwrap().set_modified(old).unwrap();
        assert_eq!(status(dir.path(), &s).await, Status::Ready(clip_url(&s)));
        let age = SystemTime::now().duration_since(std::fs::metadata(&p).unwrap().modified().unwrap()).unwrap();
        assert!(age < Duration::from_secs(60), "used clip should look freshly used");
        let other = ClipSpec { rev: "zzzz".into(), ..s };
        assert_eq!(status(dir.path(), &other).await, Status::Missing);
    }

    /// Full path through real ffmpeg: PNG frames in, VP9 webm out.
    #[tokio::test]
    async fn encodes_frames_to_vp9_webm() {
        if !crate::storage::transcode::ffmpeg_available().await {
            eprintln!("ffmpeg not installed; skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        // Make a few PNGs with ffmpeg itself.
        let mut frames = Vec::new();
        for i in 0..6 {
            let out = Command::new(crate::storage::paths::resolve_ffmpeg_command())
                .args(["-v", "error", "-f", "lavfi", "-i", &format!("color=c=0x{:02x}8040:s=640x360", i * 40)])
                .args(["-frames:v", "1", "-f", "image2pipe", "-c:v", "png", "-"])
                .output()
                .await
                .unwrap();
            assert!(out.status.success());
            frames.push(out.stdout);
        }
        let s = spec();
        let StartOutcome::Started { job_id } = start(dir.path(), s.clone(), 6).await.unwrap() else { panic!("not started") };
        assert_eq!(status(dir.path(), &s).await, Status::Rendering);
        for f in &frames {
            push_frame(&job_id, f).await.unwrap();
        }
        let url = finish(&job_id).await.unwrap();
        assert_eq!(url, clip_url(&s));
        let clip = cache_dir(dir.path()).join(s.file_name());
        assert!(std::fs::metadata(&clip).unwrap().len() > 100);
        let probe = Command::new(crate::storage::paths::resolve_ffprobe_command())
            .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=codec_name,width,height,nb_read_frames", "-count_frames", "-of", "csv=p=0"])
            .arg(&clip)
            .output()
            .await;
        if let Ok(p) = probe {
            let text = String::from_utf8_lossy(&p.stdout);
            assert!(text.contains("vp9") && text.contains("640") && text.trim().ends_with('6'), "probe: {text}");
        }
        assert!(matches!(start(dir.path(), s, 6).await.unwrap(), StartOutcome::Ready(_)));
    }

    #[tokio::test]
    async fn rejects_wrong_frame_counts_and_non_png() {
        if !crate::storage::transcode::ffmpeg_available().await {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let s = ClipSpec { rev: "cnt".into(), ..spec() };
        let StartOutcome::Started { job_id } = start(dir.path(), s.clone(), 3).await.unwrap() else { panic!() };
        assert_eq!(push_frame(&job_id, b"not a png at all").await.unwrap_err().0, 400);
        assert_eq!(finish(&job_id).await.unwrap_err().0, 400); // 0 of 3 frames
        assert!(!cache_dir(dir.path()).join(s.file_name()).exists());
        assert_eq!(status(dir.path(), &s).await, Status::Missing);
    }
}
