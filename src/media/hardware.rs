use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareCapabilities {
    pub os: String,
    pub has_hardware_accel: bool,
    pub supported_decoders: Vec<String>,
    pub displays_detected: Vec<DisplayOutputInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayOutputInfo {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub is_primary: bool,
    pub refresh_rate: f32,
}

pub struct HardwareProber;

impl HardwareProber {
    pub fn probe() -> HardwareCapabilities {
        let os = std::env::consts::OS.to_string();
        let mut decoders = Vec::new();

        #[cfg(target_os = "linux")]
        {
            if std::path::Path::new("/dev/dri/renderD128").exists() {
                decoders.push("vaapi".to_string());
                decoders.push("vulkan".to_string());
            }
            if std::path::Path::new("/dev/nvidia0").exists() {
                decoders.push("nvdec".to_string());
                decoders.push("cuda".to_string());
            }
        }

        #[cfg(target_os = "windows")]
        {
            decoders.push("d3d11va".to_string());
            decoders.push("dxva2".to_string());
            decoders.push("nvdec".to_string());
        }

        #[cfg(target_os = "macos")]
        {
            decoders.push("videotoolbox".to_string());
            decoders.push("metal".to_string());
        }

        if decoders.is_empty() {
            decoders.push("software".to_string());
        }

        let displays = vec![
            DisplayOutputInfo {
                name: "Primary Display".to_string(),
                width: 1920,
                height: 1080,
                is_primary: true,
                refresh_rate: 60.0,
            }
        ];

        HardwareCapabilities {
            os,
            has_hardware_accel: !decoders.contains(&"software".to_string()) || decoders.len() > 1,
            supported_decoders: decoders,
            displays_detected: displays,
        }
    }
}

pub struct PrecisionClock {
    base_time: Instant,
    running: AtomicBool,
    paused_offset_ms: AtomicU64,
}

impl PrecisionClock {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            base_time: Instant::now(),
            running: AtomicBool::new(false),
            paused_offset_ms: AtomicU64::new(0),
        })
    }

    pub fn start(&self) {
        self.running.store(true, Ordering::SeqCst);
    }

    pub fn pause(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    pub fn current_time_ms(&self) -> u64 {
        if self.running.load(Ordering::SeqCst) {
            self.base_time.elapsed().as_millis() as u64 + self.paused_offset_ms.load(Ordering::SeqCst)
        } else {
            self.paused_offset_ms.load(Ordering::SeqCst)
        }
    }

    pub fn seek_ms(&self, ms: u64) {
        self.paused_offset_ms.store(ms, Ordering::SeqCst);
    }
}
