use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use crate::core::models::{MediaItem, MediaType};
use crate::storage::Database;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YtDlpDownloadOptions {
    pub task_id: Option<String>,
    pub url: String,
    pub browser: Option<String>,
    #[serde(default = "default_true")]
    pub sponsorblock_remove_all: bool,
    #[serde(default)]
    pub audio_only: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YtDlpProgress {
    pub task_id: String,
    pub percent: f32,
    pub speed: Option<String>,
    pub eta: Option<String>,
    pub status: String,
    pub is_complete: bool,
    pub error: Option<String>,
    pub media_item: Option<MediaItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YtDlpStatusResponse {
    pub available: bool,
    pub version: Option<String>,
    pub supported_browsers: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct YtDlpJsonPayload {
    id: Option<String>,
    title: Option<String>,
    duration: Option<f64>,
    thumbnail: Option<String>,
    _filename: Option<String>,
    filename: Option<String>,
    ext: Option<String>,
}

fn get_progress_registry() -> &'static Arc<Mutex<HashMap<String, YtDlpProgress>>> {
    static REGISTRY: OnceLock<Arc<Mutex<HashMap<String, YtDlpProgress>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

pub struct YtDlpImporter;

impl YtDlpImporter {
    pub async fn check_status() -> YtDlpStatusResponse {
        let output = Command::new("yt-dlp")
            .arg("--version")
            .output()
            .await;

        match output {
            Ok(out) if out.status.success() => {
                let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
                YtDlpStatusResponse {
                    available: true,
                    version: Some(ver),
                    supported_browsers: vec![
                        "chrome".into(),
                        "firefox".into(),
                        "brave".into(),
                        "edge".into(),
                        "chromium".into(),
                        "opera".into(),
                        "vivaldi".into(),
                        "safari".into(),
                    ],
                }
            }
            _ => YtDlpStatusResponse {
                available: false,
                version: None,
                supported_browsers: Vec::new(),
            },
        }
    }

    pub fn get_progress(task_id: &str) -> Option<YtDlpProgress> {
        let reg = get_progress_registry().lock().unwrap_or_else(|e| e.into_inner());
        reg.get(task_id).cloned()
    }

    pub fn set_progress(task_id: &str, progress: YtDlpProgress) {
        let mut reg = get_progress_registry().lock().unwrap_or_else(|e| e.into_inner());
        if reg.len() > 100 {
            reg.retain(|_, v| v.status != "completed" && v.status != "failed");
        }
        reg.insert(task_id.to_string(), progress);
    }

    pub fn update_progress<F>(task_id: &str, f: F)
    where
        F: FnOnce(&mut YtDlpProgress),
    {
        let mut reg = get_progress_registry().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = reg.get_mut(task_id) {
            f(p);
        }
    }

    /// Spawns asynchronous non-blocking background download task with real-time progress streaming
    pub fn start_background_download(
        options: YtDlpDownloadOptions,
        media_dir: PathBuf,
        tools_dir: PathBuf,
        db: Database,
    ) -> String {
        let task_id = options.task_id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        
        let initial_progress = YtDlpProgress {
            task_id: task_id.clone(),
            percent: 0.0,
            speed: None,
            eta: None,
            status: "Starting download...".into(),
            is_complete: false,
            error: None,
            media_item: None,
        };
        Self::set_progress(&task_id, initial_progress);

        let t_id = task_id.clone();
        tokio::spawn(async move {
            match Self::download_media_with_progress(&options, &media_dir, &tools_dir, &t_id).await {
                Ok(media_item) => {
                    let _ = db.insert_media(&media_item);
                    Self::update_progress(&t_id, |p| {
                        p.percent = 100.0;
                        p.status = format!("Completed: {}", media_item.name);
                        p.is_complete = true;
                        p.media_item = Some(media_item);
                    });
                }
                Err(e) => {
                    Self::update_progress(&t_id, |p| {
                        p.is_complete = true;
                        p.error = Some(e.to_string());
                        p.status = format!("Failed: {}", e);
                    });
                }
            }
        });

        task_id
    }

    /// Internal execution tracking stdout and stderr lines for live download percentage
    async fn download_media_with_progress(
        options: &YtDlpDownloadOptions,
        media_dir: &Path,
        tools_dir: &Path,
        task_id: &str,
    ) -> Result<MediaItem, Box<dyn std::error::Error + Send + Sync>> {
        let target_subfolder = if options.audio_only { "audio" } else { "videos" };
        let full_output_dir = media_dir.join(target_subfolder);
        tokio::fs::create_dir_all(&full_output_dir).await?;

        let output_template = full_output_dir
            .join("%(title).100B [%(id)s].%(ext)s")
            .to_string_lossy()
            .to_string();

        let clean_url = options.url.trim();
        if clean_url.is_empty() {
            return Err("URL cannot be empty".into());
        }

        Self::update_progress(task_id, |p| {
            p.status = "Connecting to video source...".into();
        });

        // 1. First attempt with requested browser cookies
        let res = Self::run_ytdlp_process(options, clean_url, &output_template, options.browser.as_deref(), tools_dir, task_id).await;

        // 2. If failed and browser cookies were specified, retry without cookies
        let (stdout_lines, last_json_opt) = match res {
            Ok(tuple) => tuple,
            Err(e) => {
                tracing::warn!("yt-dlp with browser cookies failed ({}), retrying without cookies: {}", e, clean_url);
                Self::update_progress(task_id, |p| {
                    p.status = "Retrying direct download without cookies...".into();
                });
                Self::run_ytdlp_process(options, clean_url, &output_template, None, tools_dir, task_id).await?
            }
        };

        // If json was captured in stdout
        let info_payload = if let Some(json_str) = last_json_opt {
            serde_json::from_str::<YtDlpJsonPayload>(&json_str).ok()
        } else {
            // Find any json line in captured stdout
            stdout_lines
                .iter()
                .find_map(|l| {
                    let trimmed = l.trim();
                    if trimmed.starts_with('{') && trimmed.ends_with('}') {
                        serde_json::from_str::<YtDlpJsonPayload>(trimmed).ok()
                    } else {
                        None
                    }
                })
        };

        let info = info_payload.unwrap_or(YtDlpJsonPayload {
            id: None,
            title: None,
            duration: None,
            thumbnail: None,
            _filename: None,
            filename: None,
            ext: None,
        });

        let video_id = info.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let title = info.title.unwrap_or_else(|| format!("Online Media [{}]", video_id));
        let duration = info.duration;
        let thumbnail = info.thumbnail;

        let filename_path = info._filename.or(info.filename).map(PathBuf::from);

        let mut disk_path = filename_path.unwrap_or_else(|| {
            let ext = info.ext.clone().unwrap_or_else(|| if options.audio_only { "mp3".into() } else { "mp4".into() });
            full_output_dir.join(format!("{}.{}", video_id, ext))
        });

        // Last-resort H.264 downloads (the format selector above only falls
        // here when neither AV1 nor VP9 was available) get converted to VP9
        // so they still play back as a native hardware-decoded stream. Never
        // fails the whole import if ffmpeg is missing or the convert errors
        // out -- the original H.264 file is still perfectly playable.
        if !options.audio_only && disk_path.is_file() {
            if crate::storage::transcode::needs_transcode(&disk_path).await {
                Self::update_progress(task_id, |p| {
                    p.status = "Converting H.264 to VP9 for native playback...".into();
                });
                let t_id = task_id.to_string();
                match crate::storage::transcode::transcode_h264_to_vp9(&disk_path, move |pct| {
                    Self::update_progress(&t_id, |p| {
                        p.status = format!("Converting H.264 to VP9: {:.0}%", pct);
                    });
                })
                .await
                {
                    Ok(new_path) => disk_path = new_path,
                    Err(e) => tracing::warn!("transcode skipped for {:?}: {}", disk_path, e),
                }
            }
        }

        let final_file_path = if let Ok(rel) = disk_path.strip_prefix(media_dir) {
            format!("/media/{}", rel.to_string_lossy().replace('\\', "/"))
        } else {
            disk_path.to_string_lossy().replace('\\', "/")
        };

        let media_type = if options.audio_only {
            MediaType::Audio
        } else {
            MediaType::Video
        };

        let mut media_item = MediaItem::new(&title, &final_file_path, media_type);
        media_item.id = format!("ytdlp_{}", video_id);
        media_item.duration_seconds = duration;
        media_item.thumbnail_path = thumbnail;
        
        Ok(media_item)
    }

    async fn run_ytdlp_process(
        options: &YtDlpDownloadOptions,
        url: &str,
        output_template: &str,
        browser_opt: Option<&str>,
        tools_dir: &Path,
        task_id: &str,
    ) -> Result<(Vec<String>, Option<String>), Box<dyn std::error::Error + Send + Sync>> {
        let clean_url = url.trim();
        if !clean_url.starts_with("http://") && !clean_url.starts_with("https://") {
            return Err("Invalid URL scheme: must begin with http:// or https://".into());
        }

        let mut cmd = Command::new(crate::network::ytdlp_updater::resolve_ytdlp_command(tools_dir));
        cmd.kill_on_drop(true);
        cmd.arg("--no-playlist")
            .arg("--newline")
            .arg("--progress")
            .arg("--print-json")
            .arg("-o")
            .arg(output_template);

        // yt-dlp shells out to its own ffmpeg for merging video+audio
        // streams -- it doesn't know about our bundled Windows copy
        // (src/storage/paths.rs) unless told explicitly. Only passed when
        // we actually resolved a bundled copy; when relying on PATH (the
        // Linux/macOS case, system-installed), omit the flag and let
        // yt-dlp do its own default PATH search, unchanged.
        let ffmpeg_cmd = crate::storage::paths::resolve_ffmpeg_command();
        if let Some(ffmpeg_dir) = ffmpeg_cmd.parent().filter(|p| !p.as_os_str().is_empty()) {
            cmd.arg("--ffmpeg-location").arg(ffmpeg_dir);
        }

        // SponsorBlock
        if options.sponsorblock_remove_all {
            cmd.arg("--sponsorblock-remove").arg("all");
        }

        // Browser Cookies with strict allowlist
        const ALLOWED_BROWSERS: &[&str] = &["chrome", "firefox", "brave", "edge", "chromium", "opera", "vivaldi", "safari"];
        if let Some(b) = browser_opt {
            let b_clean = b.trim().to_lowercase();
            if !b_clean.is_empty() && b_clean != "none" {
                if b_clean == "auto" {
                    cmd.arg("--cookies-from-browser").arg("chrome");
                } else if ALLOWED_BROWSERS.contains(&b_clean.as_str()) {
                    cmd.arg("--cookies-from-browser").arg(&b_clean);
                } else {
                    return Err(format!("Unsupported browser for cookies: {}", b_clean).into());
                }
            }
        }

        // Format selection
        if options.audio_only {
            cmd.arg("-x")
                .arg("--audio-format")
                .arg("mp3")
                .arg("--audio-quality")
                .arg("0");
        } else {
            // Prefer AV1/VP9 (webm) over H.264/mp4: both are hardware-decoded
            // natively on the target TV at a fraction of H.264's bitrate for
            // the same quality. No --merge-output-format: omitting it lets
            // yt-dlp pick the container that matches each branch's streams
            // (webm+webm, or mp4+m4a) instead of forcing a remux. Any H.264
            // files that still fall through to the last-resort branch get
            // converted by `transcode::transcode_h264_to_vp9` below.
            cmd.arg("-f").arg(
                "bestvideo[vcodec^=av01][ext=webm]+bestaudio[ext=webm]/\
                 bestvideo[vcodec^=vp9][ext=webm]+bestaudio[ext=webm]/\
                 bestvideo[ext=mp4]+bestaudio[ext=m4a]/best",
            );
        }

        cmd.arg("--").arg(clean_url);

        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd.spawn()?;
        let stdout = child.stdout.take().ok_or("Failed to capture child stdout")?;
        let stderr = child.stderr.take().ok_or("Failed to capture child stderr")?;

        let mut reader = BufReader::new(stdout).lines();
        let mut err_reader = BufReader::new(stderr).lines();

        let mut stdout_captured = Vec::new();
        let mut last_json = None;

        let t_id = task_id.to_string();
        let mut stdout_done = false;
        let mut stderr_done = false;

        while !stdout_done || !stderr_done {
            tokio::select! {
                line_res = reader.next_line(), if !stdout_done => {
                    match line_res {
                        Ok(Some(line)) => {
                            let trimmed = line.trim();
                            if trimmed.starts_with('{') && trimmed.ends_with('}') {
                                last_json = Some(trimmed.to_string());
                            } else {
                                Self::parse_and_update_progress_line(&t_id, trimmed);
                            }
                            stdout_captured.push(line);
                        }
                        Ok(None) | Err(_) => {
                            stdout_done = true;
                        }
                    }
                }
                err_res = err_reader.next_line(), if !stderr_done => {
                    match err_res {
                        Ok(Some(err_line)) => {
                            let trimmed = err_line.trim();
                            Self::parse_and_update_progress_line(&t_id, trimmed);
                        }
                        Ok(None) | Err(_) => {
                            stderr_done = true;
                        }
                    }
                }
            }
        }

        let status = child.wait().await?;
        if !status.success() {
            return Err("yt-dlp process exited with error status".into());
        }

        Ok((stdout_captured, last_json))
    }

    fn parse_and_update_progress_line(task_id: &str, line: &str) {
        if line.is_empty() {
            return;
        }

        // 1. Download progress percentage: "[download] 45.2% of 12.34MiB at 3.45MiB/s ETA 00:02"
        if line.contains("[download]") {
            let mut pct_opt = None;
            let mut speed_opt = None;
            let mut eta_opt = None;

            if let Some(pct_idx) = line.find('%') {
                let prefix = &line[..pct_idx];
                if let Some(space_idx) = prefix.rfind(' ') {
                    let num_str = prefix[space_idx + 1..].trim();
                    if let Ok(num) = num_str.parse::<f32>() {
                        pct_opt = Some(num);
                    }
                }
            }

            if let Some(at_idx) = line.find(" at ") {
                let after_at = &line[at_idx + 4..];
                let speed_str = after_at.split_whitespace().next().unwrap_or("");
                if !speed_str.is_empty() {
                    speed_opt = Some(speed_str.to_string());
                }
            }

            if let Some(eta_idx) = line.find(" ETA ") {
                let after_eta = &line[eta_idx + 5..];
                let eta_str = after_eta.split_whitespace().next().unwrap_or("");
                if !eta_str.is_empty() {
                    eta_opt = Some(eta_str.to_string());
                }
            }

            Self::update_progress(task_id, |p| {
                if let Some(pct) = pct_opt {
                    p.percent = pct;
                }
                if speed_opt.is_some() {
                    p.speed = speed_opt;
                }
                if eta_opt.is_some() {
                    p.eta = eta_opt;
                }
                if p.percent >= 99.5 {
                    p.status = "Merging video & audio format...".into();
                } else {
                    p.status = format!("Downloading: {:.1}%", p.percent);
                }
            });
        } else if line.contains("[SponsorBlock]") {
            Self::update_progress(task_id, |p| {
                p.status = "✂️ SponsorBlock: Stripping sponsor segments...".into();
            });
        } else if line.contains("[ExtractAudio]") || line.contains("[Merger]") || line.contains("[Fixup") {
            Self::update_progress(task_id, |p| {
                p.status = "Processing and finalizing media file...".into();
            });
        }
    }
}
