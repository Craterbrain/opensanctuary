use serde::{Deserialize, Serialize};
use crate::core::models::{Schedule, ScheduleItem, TransitionType};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event_type", content = "data")]
pub enum ShowEvent {
    ItemWentLive {
        item: Option<ScheduleItem>,
        slide_index: usize,
    },
    LiveSlideChanged {
        slide_index: usize,
    },
    ItemStaged {
        item: Option<ScheduleItem>,
        slide_index: usize,
    },
    StagedSlideChanged {
        slide_index: usize,
    },
    BlackoutToggled {
        is_blackout: bool,
    },
    ClearTextToggled {
        is_clear_text: bool,
    },
    LogoToggled {
        is_logo: bool,
    },
    AlertChanged {
        alert: Option<String>,
    },
    WebStreamChanged {
        web_stream: Option<crate::core::models::WebStreamState>,
    },
    TransitionChanged {
        transition: TransitionType,
    },
    ScheduleItemAdded {
        item: ScheduleItem,
        index: usize,
    },
    ScheduleItemRemoved {
        index: usize,
    },
    /// The text of a licensed passage on the schedule was filled in. Stored without the text.
    LicensedPassageSet {
        item_id: String,
        verses: Vec<crate::core::models::LicensedVerse>,
    },
    ScheduleItemsMerged {
        first_index: usize,
        second_index: usize,
        merged_item: ScheduleItem,
    },
    ScheduleReordered {
        from: usize,
        to: usize,
    },
    ScheduleSlidesReordered {
        item_index: Option<usize>,
        from: usize,
        to: usize,
    },
    ScheduleSlideDuplicated {
        item_index: usize,
        slide_index: usize,
    },
    ScheduleSlideRemoved {
        item_index: usize,
        slide_index: usize,
    },
    ArrangementSet {
        item_index: Option<usize>,
        arrangement: Vec<crate::core::models::ArrangementEntry>,
    },
    ScheduleItemExpandToggled {
        index: usize,
        is_expanded: bool,
    },
    ScheduleItemThemeSet {
        item_index: usize,
        theme_name: String,
        background: Option<String>,
    },
    SlideBackgroundSet {
        item_index: Option<usize>,
        slide_index: usize,
        background: String,
    },
    SlideDurationSet {
        item_index: usize,
        slide_index: usize,
        duration_seconds: Option<f64>,
    },
    PresentationDefaultDurationSet {
        item_index: usize,
        duration_seconds: Option<f64>,
    },
    PresentationLoopSet {
        item_index: usize,
        loop_enabled: bool,
    },
    GlobalThemeSet {
        theme_name: String,
        background: String,
    },
    ScheduleLoaded {
        schedule: Schedule,
    },
    ScheduleCleared,
    ScheduleTitleSet {
        title: String,
    },

    // Media Playback Controls
    MediaScheduledStart { target_pts: f64, start_at_epoch_ms: u64 },
    MediaPaused { current_time: f64 },
    MediaLoopSet { is_looping: bool },
    MediaMuteSet { is_muted: bool },
    MediaVolumeSet { volume: f64 },
    MediaPrerolled { target_pts: f64 },

    // Slide Editor Element Events (Phase 1)
    SlideElementAdded {
        item_index: usize,
        slide_index: usize,
        element: crate::core::models::SlideElement,
        at_index: Option<usize>,
    },
    SlideElementRemoved {
        item_index: usize,
        slide_index: usize,
        element_id: String,
    },
    SlideElementTransformUpdated {
        item_index: usize,
        slide_index: usize,
        element_id: String,
        transform: crate::core::models::ElementTransform,
    },
    TextBlockContentUpdated {
        item_index: usize,
        slide_index: usize,
        element_id: String,
        runs: Vec<crate::core::models::TextRun>,
        paragraph_style: Option<crate::core::models::TextParagraphStyle>,
    },
    ElementEffectsUpdated {
        item_index: usize,
        slide_index: usize,
        element_id: String,
        effects: crate::core::models::ElementEffects,
    },
    SlideElementsReordered {
        item_index: usize,
        slide_index: usize,
        element_id: String,
        to_z: i32,
    },
    SlideElementsGrouped {
        item_index: usize,
        slide_index: usize,
        element_ids: Vec<String>,
    },
    SlideElementsUngrouped {
        item_index: usize,
        slide_index: usize,
        group_id: String,
    },
    SlideBackgroundV2Set {
        item_index: Option<usize>,
        slide_index: usize,
        background: crate::core::models::SlideBackground,
    },
    SlideTransitionSet {
        item_index: Option<usize>,
        slide_index: usize,
        transition: crate::core::models::SlideTransition,
    },
    SlideSpeakerNotesSet {
        item_index: usize,
        slide_index: usize,
        notes: String,
    },
    SlideCcliMetadataSet {
        item_index: usize,
        slide_index: usize,
        metadata: crate::core::models::CcliMetadata,
    },
    SlideEditedBatch {
        item_index: usize,
        slide_index: usize,
        ops: Vec<crate::core::commands::SlideEditOp>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EventEnvelope {
    pub sequence_number: u64,
    pub event_id: String,
    pub timestamp_ms: u64,
    pub event: ShowEvent,
}

impl ShowEvent {
    /// This event as it may be stored: any licensed passage inside it loses its text (it keeps the
    /// reference, which is what a later look-up needs). See `ScheduleItem::without_licensed_text`.
    pub fn without_licensed_text(&self) -> std::borrow::Cow<'_, ShowEvent> {
        use std::borrow::Cow;
        let licensed = |i: &ScheduleItem| i.licensed.is_some();
        match self {
            ShowEvent::ItemWentLive { item: Some(i), slide_index } if licensed(i) => {
                Cow::Owned(ShowEvent::ItemWentLive { item: Some(i.without_licensed_text()), slide_index: *slide_index })
            }
            ShowEvent::ItemStaged { item: Some(i), slide_index } if licensed(i) => {
                Cow::Owned(ShowEvent::ItemStaged { item: Some(i.without_licensed_text()), slide_index: *slide_index })
            }
            ShowEvent::ScheduleItemAdded { item, index } if licensed(item) => {
                Cow::Owned(ShowEvent::ScheduleItemAdded { item: item.without_licensed_text(), index: *index })
            }
            ShowEvent::ScheduleItemsMerged { first_index, second_index, merged_item } if licensed(merged_item) => {
                Cow::Owned(ShowEvent::ScheduleItemsMerged {
                    first_index: *first_index,
                    second_index: *second_index,
                    merged_item: merged_item.without_licensed_text(),
                })
            }
            ShowEvent::ScheduleLoaded { schedule } if schedule.has_licensed() => {
                Cow::Owned(ShowEvent::ScheduleLoaded { schedule: schedule.without_licensed_text() })
            }
            ShowEvent::LicensedPassageSet { item_id, verses } if !verses.is_empty() => {
                Cow::Owned(ShowEvent::LicensedPassageSet { item_id: item_id.clone(), verses: Vec::new() })
            }
            other => Cow::Borrowed(other),
        }
    }
}
