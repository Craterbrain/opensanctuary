mod core;
mod bibles;
mod songs;
mod media;
mod themes;
mod presentations;
mod slide_templates;
mod settings;
mod keyring;
mod events;
mod paired_devices;
mod ccli;

pub use core::Database;
pub use bibles::InstalledBibleInfo;
pub use paired_devices::PairedDevice;
pub use ccli::CcliUsageRow;

use crate::core::models::Slide;

/// Load slides from JSON and synthesize v2 elements for any legacy slides
pub fn load_and_synthesize_slides(slides_json: &str) -> Vec<Slide> {
    let mut slides: Vec<Slide> = serde_json::from_str(slides_json).unwrap_or_default();
    for slide in &mut slides {
        slide.synthesize_v2_if_needed();
    }
    slides
}

/// Serialize slides to JSON for storage, ensuring v2 elements and legacy text projection are up to date
pub fn serialize_slides_for_storage(slides: &[Slide]) -> String {
    let mut slides_to_save = slides.to_vec();
    for slide in &mut slides_to_save {
        slide.synthesize_v2_if_needed();
        slide.project_text_from_elements();
    }
    serde_json::to_string(&slides_to_save).unwrap_or_else(|_| "[]".to_string())
}
