//! Converts imported video away from H.264 so it plays back as a native
//! hardware-decoded stream on the target TV hardware instead of relying on
//! H.264's (comparatively inefficient) hardware decoder path. AV1, VP9,
//! HEVC, and AVS2 are all hardware-decoded natively on that hardware and are
//! left untouched; H.264 is the one codec actively converted away from,
//! since the same visual quality costs a much larger file/bitrate in H.264
//! than in the others.
//!
//! VP9 (not AV1) is the transcode target: software AV1 encoding is
//! dramatically slower than realtime on typical hardware, which would turn
//! routine video imports into multi-hour background jobs. VP9 is still a
//! natively hardware-decoded codec on the target TV and encodes at a usable
//! speed on a CPU.
//!
//! Mirrors `ytdlp_import.rs`'s process-handling style (async
//! `tokio::process::Command`, `.kill_on_drop(true)`) rather than the
//! blocking `std::process::Command` pattern used for pkexec/UAC elevation.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

fn is_native_codec(codec: &str) -> bool {
    !matches!(codec.to_ascii_lowercase().as_str(), "h264" | "avc" | "avc1")
}

pub async fn ffmpeg_available() -> bool {
    Command::new(crate::storage::paths::resolve_ffmpeg_command())
        .arg("-version")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub async fn ffprobe_available() -> bool {
    Command::new(crate::storage::paths::resolve_ffprobe_command())
        .arg("-version")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

async fn probe_video_codec(path: &Path) -> Option<String> {
    let output = Command::new(crate::storage::paths::resolve_ffprobe_command())
        .args([
            "-v", "error",
            "-select_streams", "v:0",
            "-show_entries", "stream=codec_name",
            "-of", "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }
    let codec = String::from_utf8_lossy(&output.stdout).trim().to_lowercase();
    if codec.is_empty() {
        None
    } else {
        Some(codec)
    }
}

async fn probe_duration_secs(path: &Path) -> Option<f64> {
    let output = Command::new(crate::storage::paths::resolve_ffprobe_command())
        .args([
            "-v", "error",
            "-show_entries", "format=duration",
            "-of", "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout).trim().parse::<f64>().ok()
}

/// True if `path`'s video stream should be converted for native playback.
/// False (never blocks import) if ffprobe is missing or the codec can't be
/// determined -- transcoding is a quality-of-life optimization, not a
/// requirement for a file to be usable.
pub async fn needs_transcode(path: &Path) -> bool {
    match probe_video_codec(path).await {
        Some(codec) => !is_native_codec(&codec),
        None => false,
    }
}

/// Transcodes `input` (expected H.264) to VP9/Opus in a sibling `.webm`
/// file, deletes `input` on success, and returns the new path. Leaves
/// `input` untouched on any failure (missing ffmpeg, encode error, etc.).
pub async fn transcode_h264_to_vp9(
    input: &Path,
    mut on_progress: impl FnMut(f32) + Send,
) -> Result<PathBuf, String> {
    let output = input.with_extension("webm");
    if output == input {
        return Err("input is already a .webm file".into());
    }

    let duration = probe_duration_secs(input).await;

    let mut cmd = Command::new(crate::storage::paths::resolve_ffmpeg_command());
    cmd.kill_on_drop(true);
    cmd.arg("-y")
        .arg("-i").arg(input)
        .arg("-c:v").arg("libvpx-vp9")
        .arg("-b:v").arg("0")
        .arg("-crf").arg("32")
        .arg("-deadline").arg("good")
        .arg("-cpu-used").arg("2")
        .arg("-row-mt").arg("1")
        .arg("-c:a").arg("libopus")
        .arg("-b:a").arg("128k")
        .arg("-progress").arg("pipe:1")
        .arg("-nostats")
        .arg(&output);
    cmd.stdout(Stdio::piped()).stderr(Stdio::null());

    let mut child = cmd.spawn().map_err(|e| format!("failed to start ffmpeg: {e}"))?;
    let stdout = child.stdout.take().ok_or("failed to capture ffmpeg stdout")?;
    let mut lines = BufReader::new(stdout).lines();

    // ffmpeg's `-progress` key is misleadingly named: `out_time_ms` is
    // actually microseconds, not milliseconds (long-standing ffmpeg quirk).
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(us_str) = line.strip_prefix("out_time_ms=") {
            if let (Some(total_secs), Ok(out_us)) = (duration, us_str.trim().parse::<f64>()) {
                if total_secs > 0.0 {
                    let pct = ((out_us / 1_000_000.0) / total_secs * 100.0).clamp(0.0, 100.0) as f32;
                    on_progress(pct);
                }
            }
        }
    }

    let status = child.wait().await.map_err(|e| format!("ffmpeg wait failed: {e}"))?;
    if !status.success() {
        let _ = tokio::fs::remove_file(&output).await;
        return Err("ffmpeg exited with an error".into());
    }

    if let Err(e) = tokio::fs::remove_file(input).await {
        tracing::warn!(
            "transcode succeeded but failed to remove original H.264 file {:?}: {}",
            input, e
        );
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_codecs_are_not_flagged_for_transcode() {
        for codec in ["av1", "av01", "vp9", "hevc", "h265", "avs2", "cavs"] {
            assert!(is_native_codec(codec), "{codec} should be treated as native");
        }
    }

    #[test]
    fn h264_is_flagged_for_transcode() {
        for codec in ["h264", "avc", "avc1", "H264", "AVC1"] {
            assert!(!is_native_codec(codec), "{codec} should need transcoding");
        }
    }

    #[tokio::test]
    async fn needs_transcode_is_false_for_a_nonexistent_file() {
        let path = PathBuf::from("/nonexistent/path/does-not-exist.mp4");
        assert!(!needs_transcode(&path).await);
    }

    #[tokio::test]
    async fn transcode_fails_cleanly_on_a_non_video_file() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("not-a-video.mp4");
        std::fs::write(&input, b"not actually a video file").unwrap();

        let result = transcode_h264_to_vp9(&input, |_| {}).await;
        assert!(result.is_err());
        // Original file must survive a failed transcode.
        assert!(input.exists());
    }

    /// Real ffmpeg/ffprobe, not a fixture: encodes a genuine H.264 test clip
    /// via `ffmpeg -f lavfi`, confirms `needs_transcode` flags it, runs the
    /// real conversion, and confirms the output ffprobes back as VP9 with
    /// the input file gone. `#[ignore]`d since it needs ffmpeg/ffprobe on
    /// PATH with libvpx-vp9/libopus support -- not guaranteed in every CI
    /// environment. Run explicitly with `cargo test --lib -- --ignored`
    /// when verifying this path after a change.
    #[tokio::test]
    #[ignore]
    async fn real_h264_sample_transcodes_to_real_vp9() {
        if !ffmpeg_available().await || !ffprobe_available().await {
            panic!("ffmpeg/ffprobe not available on PATH -- this test needs both");
        }

        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("sample.mp4");

        let encode = Command::new("ffmpeg")
            .args([
                "-y",
                "-f", "lavfi", "-i", "testsrc=duration=1:size=160x120:rate=10",
                "-f", "lavfi", "-i", "sine=frequency=440:duration=1",
                "-c:v", "libx264", "-c:a", "aac", "-pix_fmt", "yuv420p",
            ])
            .arg(&input)
            .output()
            .await
            .expect("failed to run ffmpeg to build the test sample");
        assert!(encode.status.success(), "sample encode failed: {}", String::from_utf8_lossy(&encode.stderr));

        assert_eq!(probe_video_codec(&input).await.as_deref(), Some("h264"));
        assert!(needs_transcode(&input).await);

        let mut last_progress = 0.0f32;
        let output = transcode_h264_to_vp9(&input, |pct| last_progress = pct)
            .await
            .expect("real transcode should succeed");

        assert!(!input.exists(), "original H.264 file should be removed on success");
        assert!(output.exists());
        assert_eq!(probe_video_codec(&output).await.as_deref(), Some("vp9"));
        assert!(last_progress > 0.0, "progress callback should have fired at least once");
    }
}
