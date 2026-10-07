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

/// One edit within a `BatchSlideEdit` (the "Apply" button's full diff of
/// everything changed in one editor sitting -- see
/// `EditorHistoryManager.computeBatchOps` in web/src/editor/history.ts,
/// the only real producer of these on the wire). `#[serde(tag = "op_type",
/// content = "payload")]` (an "adjacently tagged" enum) was chosen
/// specifically to keep the wire shape byte-for-byte identical to the
/// previous stringly-typed `{op_type: String, payload: Value}` struct --
/// `{"op_type": "RemoveElement", "payload": {"element_id": "..."}}` -- so
/// this is a pure internal hardening with no required frontend change.
/// `AddElement`'s payload is the bare `SlideElement` itself (not wrapped in
/// a named field), matching `history.ts`'s `payload: el` -- a single-field
/// tuple variant serializes its content directly under "payload" rather
/// than as a one-element array, which is what makes that match exactly.
///
/// Before this, malformed/unknown ops were individually dropped with a
/// `tracing::warn!` deep inside `apply_slide_edit_op` while the rest of the
/// batch still applied (see that function's git history). Now a malformed
/// op fails to deserialize at all, which fails the *whole* `ShowCommand`
/// (see the `Err(e)` arm added alongside this in `src/api/ws.rs`) --
/// arguably more correct for an atomic "Apply" batch, but a real behavior
/// change from "partial apply" to "all or nothing," not a risk-free rename.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op_type", content = "payload")]
pub enum SlideEditOp {
    AddElement(crate::core::models::SlideElement),
    RemoveElement {
        element_id: String,
    },
    UpdateTransform {
        element_id: String,
        transform: crate::core::models::ElementTransform,
    },
    UpdateTextBlockContent {
        element_id: String,
        runs: Vec<crate::core::models::TextRun>,
        #[serde(default)]
        paragraph_style: Option<crate::core::models::TextParagraphStyle>,
    },
    UpdateElementEffects {
        element_id: String,
        effects: crate::core::models::ElementEffects,
    },
    ReorderElements {
        element_id: String,
        to_z: i32,
    },
    GroupElements {
        element_ids: Vec<String>,
    },
    UngroupElements {
        group_id: String,
    },
    SetSpeakerNotes {
        notes: String,
    },
    SetCcliMetadata {
        metadata: crate::core::models::CcliMetadata,
    },
    SetBackground {
        background: crate::core::models::SlideBackground,
    },
    SetTransition {
        transition: crate::core::models::SlideTransition,
    },
}

impl SlideEditOp {
    pub fn add_element(element: crate::core::models::SlideElement) -> Self {
        Self::AddElement(element)
    }
    pub fn remove_element(element_id: impl Into<String>) -> Self {
        Self::RemoveElement { element_id: element_id.into() }
    }
    pub fn update_transform(element_id: impl Into<String>, transform: crate::core::models::ElementTransform) -> Self {
        Self::UpdateTransform { element_id: element_id.into(), transform }
    }
    pub fn update_text_block_content(
        element_id: impl Into<String>,
        runs: Vec<crate::core::models::TextRun>,
        paragraph_style: Option<crate::core::models::TextParagraphStyle>,
    ) -> Self {
        Self::UpdateTextBlockContent { element_id: element_id.into(), runs, paragraph_style }
    }
    pub fn update_element_effects(element_id: impl Into<String>, effects: crate::core::models::ElementEffects) -> Self {
        Self::UpdateElementEffects { element_id: element_id.into(), effects }
    }
    pub fn reorder_elements(element_id: impl Into<String>, to_z: i32) -> Self {
        Self::ReorderElements { element_id: element_id.into(), to_z }
    }
    pub fn group_elements(element_ids: Vec<String>) -> Self {
        Self::GroupElements { element_ids }
    }
    pub fn ungroup_elements(group_id: impl Into<String>) -> Self {
        Self::UngroupElements { group_id: group_id.into() }
    }
    pub fn set_speaker_notes(notes: impl Into<String>) -> Self {
        Self::SetSpeakerNotes { notes: notes.into() }
    }
    pub fn set_background(background: crate::core::models::SlideBackground) -> Self {
        Self::SetBackground { background }
    }
    pub fn set_transition(transition: crate::core::models::SlideTransition) -> Self {
        Self::SetTransition { transition }
    }
}

#[cfg(test)]
mod slide_edit_op_wire_tests {
    use super::SlideEditOp;

    /// The whole point of `#[serde(tag = "op_type", content = "payload")]`
    /// is that `web/src/editor/history.ts`'s hand-built `{op_type, payload}`
    /// literals keep deserializing exactly as before -- these are literal
    /// copies of what that file actually sends (see its `computeBatchOps`),
    /// not idealized examples, so a wire-format regression here would be a
    /// real compatibility break, not just a test artifact.
    #[test]
    fn deserializes_add_element_payload_as_bare_slide_element() {
        // history.ts: `ops.push({ op_type: 'AddElement', payload: el })` --
        // payload is the SlideElement itself, not `{element: el}`.
        let json = r#"{
            "op_type": "AddElement",
            "payload": {
                "type": "TextBlock",
                "id": "el_1",
                "transform": {"x": 0.1, "y": 0.1, "w": 0.8, "h": 0.8},
                "block": {"runs": [{"text": "Hello"}], "paragraph_style": {}, "effects": {}}
            }
        }"#;
        let op: SlideEditOp = serde_json::from_str(json).expect("AddElement payload must deserialize");
        match op {
            SlideEditOp::AddElement(el) => assert_eq!(el.id(), "el_1"),
            other => panic!("expected AddElement, got {:?}", other),
        }
    }

    #[test]
    fn deserializes_remove_element_payload() {
        let json = r#"{"op_type": "RemoveElement", "payload": {"element_id": "el_2"}}"#;
        let op: SlideEditOp = serde_json::from_str(json).expect("RemoveElement payload must deserialize");
        assert_eq!(op, SlideEditOp::remove_element("el_2"));
    }

    #[test]
    fn deserializes_update_transform_payload() {
        let json = r#"{
            "op_type": "UpdateTransform",
            "payload": {
                "element_id": "el_3",
                "transform": {"x": 0.2, "y": 0.2, "w": 0.5, "h": 0.5, "rotation_deg": 0.0, "z_index": 1, "locked": false, "opacity": 1.0}
            }
        }"#;
        let op: SlideEditOp = serde_json::from_str(json).expect("UpdateTransform payload must deserialize");
        match op {
            SlideEditOp::UpdateTransform { element_id, .. } => assert_eq!(element_id, "el_3"),
            other => panic!("expected UpdateTransform, got {:?}", other),
        }
    }

    #[test]
    fn deserializes_update_text_block_content_with_omitted_paragraph_style() {
        // history.ts always sends paragraph_style for this op in practice,
        // but the pre-refactor code tolerated it being absent from the
        // payload entirely (`op.payload.get("paragraph_style")` returning
        // None) -- `#[serde(default)]` on that field preserves exactly that
        // tolerance for the enum.
        let json = r#"{
            "op_type": "UpdateTextBlockContent",
            "payload": {"element_id": "el_4", "runs": [{"text": "Hi"}]}
        }"#;
        let op: SlideEditOp = serde_json::from_str(json).expect("payload missing paragraph_style must still deserialize");
        match op {
            SlideEditOp::UpdateTextBlockContent { paragraph_style, .. } => assert!(paragraph_style.is_none()),
            other => panic!("expected UpdateTextBlockContent, got {:?}", other),
        }
    }

    #[test]
    fn deserializes_set_background_with_nested_tagged_enum() {
        let json = r##"{
            "op_type": "SetBackground",
            "payload": {"background": {"kind": "Solid", "data": "#000000"}}
        }"##;
        let op: SlideEditOp = serde_json::from_str(json).expect("SetBackground payload must deserialize");
        assert_eq!(op, SlideEditOp::set_background(crate::core::models::SlideBackground::Solid("#000000".to_string())));
    }

    #[test]
    fn deserializes_set_transition_payload() {
        let json = r#"{"op_type": "SetTransition", "payload": {"transition": {"kind": "crossfade", "duration_ms": 300}}}"#;
        let op: SlideEditOp = serde_json::from_str(json).expect("SetTransition payload must deserialize");
        assert_eq!(op, SlideEditOp::set_transition(crate::core::models::SlideTransition { kind: "crossfade".to_string(), duration_ms: 300 }));
    }

    #[test]
    fn unknown_op_type_fails_to_deserialize_instead_of_silently_matching_a_catch_all() {
        let json = r#"{"op_type": "DoesNotExist", "payload": {}}"#;
        assert!(serde_json::from_str::<SlideEditOp>(json).is_err());
    }

    #[test]
    fn round_trips_through_serialize_then_deserialize() {
        let original = SlideEditOp::set_speaker_notes("Preach here");
        let json = serde_json::to_string(&original).expect("serialize");
        assert!(json.contains(r#""op_type":"SetSpeakerNotes""#));
        assert!(json.contains(r#""payload":{"notes":"Preach here"}"#));
        let round_tripped: SlideEditOp = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(original, round_tripped);
    }
}

