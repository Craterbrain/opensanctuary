use serde::{Deserialize, Serialize};
use crate::core::models::{ScheduleItem, TransitionType};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]

pub enum ShowCommand {
    // Live Presentation
    GoLive { item_index: Option<usize>, slide_index: Option<usize> },
    StageItem { item_index: Option<usize>, slide_index: Option<usize> },
    /// Stages an item that isn't (yet) in the schedule — e.g. a Library
    /// Preview click — without adding it to the schedule. The frontend never
    /// constructs this directly; it sends the `{item_type, item_id}`
    /// shorthand (mirroring AddToSchedule's), which the REST/WS layer
    /// resolves via `ToScheduleItem::to_schedule_item()` (applying the same
    /// auto-split as scheduling) before rewriting into this command.
    StageItemDirect(ScheduleItem),
    NextSlide,
    PrevSlide,
    NextItem,
    PrevItem,
    JumpSlide(usize),
    JumpSection(String), // "V", "C", "B", "E", "P", "I", "O"

    // Overlays & Screens
    ToggleBlackout,
    ToggleClearText,
    ToggleLogo,
    SetAlert(Option<String>),
    /// Projects (or, with `None`, clears) an external web feed on the FOH display.
    SetWebStream(Option<crate::core::models::WebStreamState>),
    SetTransition(TransitionType),


    // Media Playback Controls
    MediaScheduledStart { target_pts: f64, start_at_epoch_ms: u64 },
    MediaPause { current_time: f64 },
    MediaSetLoop(bool),
    MediaSetMute(bool),
    MediaSetVolume(f64),
    MediaPreroll { target_pts: f64 },

    // Themes & Styling
    SetItemTheme { item_index: usize, theme_name: String, background: Option<String> },
    SetSlideBackground { item_index: Option<usize>, slide_index: usize, background: String },
    /// Sets (or clears, with `None`) a per-slide slideshow auto-advance duration
    /// override, in seconds. `slide_index` addresses `ScheduleItem.slides` directly
    /// (master-slide index, not a play position).
    SetSlideDuration { item_index: usize, slide_index: usize, duration_seconds: Option<f64> },
    /// Sets (or clears, with `None`) the fallback slideshow duration for any slide
    /// in the item without its own `SetSlideDuration` override.
    SetPresentationDefaultDuration { item_index: usize, duration_seconds: Option<f64> },
    /// Sets whether slideshow playback loops back to the first slide on reaching
    /// the end, instead of stopping.
    SetPresentationLoop { item_index: usize, loop_enabled: bool },
    SetGlobalTheme { theme_name: String, background: String },

    // Schedule Manipulation
    AddToSchedule(ScheduleItem),
    AddScheduleHeader { title: String, index: Option<usize> },
    RemoveFromSchedule(usize),
    ReorderSchedule { from: usize, to: usize },
    ReorderItemSlides {
        #[serde(default)]
        item_index: Option<usize>,
        from: usize,
        to: usize,
    },
    DuplicateItemSlide { item_index: usize, slide_index: usize },
    RemoveItemSlide { item_index: usize, slide_index: usize },
    /// Combines two schedule items' resolved slide sequences (in schedule order)
    /// into one new "presentation" item, replacing both.
    MergeScheduleItems { first_item_index: usize, second_item_index: usize },
    SetArrangement {
        #[serde(default)]
        item_index: Option<usize>,
        arrangement: Vec<crate::core::models::ArrangementEntry>,
    },
    ToggleScheduleItemExpand(usize),
    SetScheduleTitle(String),
    NewSchedule,
    LoadSchedule(crate::core::models::Schedule),

    // Slide Editor Element Manipulation (Phase 1)
    AddSlideElement {
        item_index: usize,
        slide_index: usize,
        element: crate::core::models::SlideElement,
        at_index: Option<usize>,
    },
    RemoveSlideElement {
        item_index: usize,
        slide_index: usize,
        element_id: String,
    },
    UpdateSlideElementTransform {
        item_index: usize,
        slide_index: usize,
        element_id: String,
        transform: crate::core::models::ElementTransform,
    },
    UpdateTextBlockContent {
        item_index: usize,
        slide_index: usize,
        element_id: String,
        runs: Vec<crate::core::models::TextRun>,
        paragraph_style: Option<crate::core::models::TextParagraphStyle>,
    },
    UpdateElementEffects {
        item_index: usize,
        slide_index: usize,
        element_id: String,
        effects: crate::core::models::ElementEffects,
    },
    ReorderSlideElements {
        item_index: usize,
        slide_index: usize,
        element_id: String,
        to_z: i32,
    },
    GroupSlideElements {
        item_index: usize,
        slide_index: usize,
        element_ids: Vec<String>,
    },
    UngroupSlideElements {
        item_index: usize,
        slide_index: usize,
        group_id: String,
    },
    SetSlideBackgroundV2 {
        item_index: Option<usize>,
        slide_index: usize,
        background: crate::core::models::SlideBackground,
    },
    SetSlideTransition {
        item_index: Option<usize>,
        slide_index: usize,
        transition: crate::core::models::SlideTransition,
    },
    SetSlideSpeakerNotes {
        item_index: usize,
        slide_index: usize,
        notes: String,
    },
    SetSlideCcliMetadata {
        item_index: usize,
        slide_index: usize,
        metadata: crate::core::models::CcliMetadata,
    },
    BatchSlideEdit {
        item_index: usize,
        slide_index: usize,
        ops: Vec<SlideEditOp>,
    },

    // History
    Undo,
    Redo,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SlideEditOp {
    pub op_type: String,
    pub payload: serde_json::Value,
}

impl SlideEditOp {
    pub fn add_element(element: crate::core::models::SlideElement) -> Self {
        Self {
            op_type: "AddElement".to_string(),
            payload: serde_json::to_value(element).unwrap_or_default(),
        }
    }
    pub fn remove_element(element_id: impl Into<String>) -> Self {
        Self {
            op_type: "RemoveElement".to_string(),
            payload: serde_json::json!({ "element_id": element_id.into() }),
        }
    }
    pub fn update_transform(element_id: impl Into<String>, transform: crate::core::models::ElementTransform) -> Self {
        Self {
            op_type: "UpdateTransform".to_string(),
            payload: serde_json::json!({
                "element_id": element_id.into(),
                "transform": transform,
            }),
        }
    }
    pub fn update_text_block_content(
        element_id: impl Into<String>,
        runs: Vec<crate::core::models::TextRun>,
        paragraph_style: Option<crate::core::models::TextParagraphStyle>,
    ) -> Self {
        Self {
            op_type: "UpdateTextBlockContent".to_string(),
            payload: serde_json::json!({
                "element_id": element_id.into(),
                "runs": runs,
                "paragraph_style": paragraph_style,
            }),
        }
    }
    pub fn update_element_effects(element_id: impl Into<String>, effects: crate::core::models::ElementEffects) -> Self {
        Self {
            op_type: "UpdateElementEffects".to_string(),
            payload: serde_json::json!({
                "element_id": element_id.into(),
                "effects": effects,
            }),
        }
    }
    pub fn reorder_elements(element_id: impl Into<String>, to_z: i32) -> Self {
        Self {
            op_type: "ReorderElements".to_string(),
            payload: serde_json::json!({
                "element_id": element_id.into(),
                "to_z": to_z,
            }),
        }
    }
    pub fn group_elements(element_ids: Vec<String>) -> Self {
        Self {
            op_type: "GroupElements".to_string(),
            payload: serde_json::json!({
                "element_ids": element_ids,
            }),
        }
    }
    pub fn ungroup_elements(group_id: impl Into<String>) -> Self {
        Self {
            op_type: "UngroupElements".to_string(),
            payload: serde_json::json!({
                "group_id": group_id.into(),
            }),
        }
    }
    pub fn set_speaker_notes(notes: impl Into<String>) -> Self {
        Self {
            op_type: "SetSpeakerNotes".to_string(),
            payload: serde_json::json!({
                "notes": notes.into(),
            }),
        }
    }
    pub fn set_background(background: crate::core::models::SlideBackground) -> Self {
        Self {
            op_type: "SetBackground".to_string(),
            payload: serde_json::json!({
                "background": background,
            }),
        }
    }
    pub fn set_transition(transition: crate::core::models::SlideTransition) -> Self {
        Self {
            op_type: "SetTransition".to_string(),
            payload: serde_json::json!({
                "transition": transition,
            }),
        }
    }
}

