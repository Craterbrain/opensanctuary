use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// NOTE: ElementTransform / TextBlock / SlideElement below are hand-mirrored by
// web/src/editor/types.ts (same names) — there's no shared schema or codegen
// between them. If you add/rename/remove a field here, make the matching edit
// there too, or the two sides will silently drift (wire JSON that (de)serializes
// fine on one side but is missing/misread on the other).

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ElementTransform {
    pub x: f64,              // 0.0..1.0 relative to canvas width
    pub y: f64,              // 0.0..1.0 relative to canvas height
    pub w: f64,              // 0.0..1.0 relative to canvas width
    pub h: f64,              // 0.0..1.0 relative to canvas height
    #[serde(default)]
    pub rotation_deg: f64,   // 0.0..360.0
    #[serde(default)]
    pub z_index: i32,        // Dense integer z-order (0..n)
    #[serde(default)]
    pub locked: bool,
    #[serde(default = "default_element_opacity")]
    pub opacity: f64,        // 0.0..1.0
}

fn default_element_opacity() -> f64 { 1.0 }

impl Default for ElementTransform {
    fn default() -> Self {
        Self {
            x: 0.05,
            y: 0.07,
            w: 0.90,
            h: 0.86,
            rotation_deg: 0.0,
            z_index: 0,
            locked: false,
            opacity: 1.0,
        }
    }
}

impl ElementTransform {
    pub fn normalized(mut self) -> Self {
        self.x = self.x.clamp(0.0, 1.0);
        self.y = self.y.clamp(0.0, 1.0);
        self.w = self.w.clamp(0.0, 1.0);
        self.h = self.h.clamp(0.0, 1.0);
        self.opacity = self.opacity.clamp(0.0, 1.0);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextRun {
    pub text: String,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub underline: bool,
    #[serde(default)]
    pub strike: bool,
    #[serde(default = "default_text_color")]
    pub color: String,
    #[serde(default = "default_font_family")]
    pub font_family: String,
    #[serde(default = "default_font_size_pt")]
    pub font_size_pt: f64,
    #[serde(default)]
    pub letter_spacing_px: f64,
    #[serde(default = "default_baseline_shift")]
    pub baseline_shift: String, // "normal", "super", "sub"
}

fn default_text_color() -> String { "#ffffff".to_string() }
fn default_font_size_pt() -> f64 { 36.0 }
fn default_baseline_shift() -> String { "normal".to_string() }

impl Default for TextRun {
    fn default() -> Self {
        Self {
            text: String::new(),
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            color: default_text_color(),
            font_family: default_font_family(),
            font_size_pt: 36.0,
            letter_spacing_px: 0.0,
            baseline_shift: "normal".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextParagraphStyle {
    #[serde(default = "default_alignment")]
    pub align: String, // "left", "center", "right", "justify"
    #[serde(default = "default_line_height_num")]
    pub line_height: f64,
    #[serde(default)]
    pub space_before_pt: f64,
    #[serde(default)]
    pub space_after_pt: f64,
    #[serde(default = "default_bullet_kind")]
    pub bullet_kind: String, // "none", "disc", "decimal", "liturgical"
    #[serde(default)]
    pub indent_level: u32,
}

fn default_line_height_num() -> f64 { 1.25 }
fn default_bullet_kind() -> String { "none".to_string() }

impl Default for TextParagraphStyle {
    fn default() -> Self {
        Self {
            align: "center".to_string(),
            line_height: 1.25,
            space_before_pt: 0.0,
            space_after_pt: 0.0,
            bullet_kind: "none".to_string(),
            indent_level: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextOutline {
    pub width: f64,
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ElementShadow {
    pub dx: f64,
    pub dy: f64,
    pub blur: f64,
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ElementReflection {
    pub enabled: bool,
    pub opacity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ElementGlow {
    pub radius: f64,
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ElementEffects {
    #[serde(default)]
    pub outline: Option<TextOutline>,
    #[serde(default)]
    pub shadow: Option<ElementShadow>,
    #[serde(default)]
    pub reflection: Option<ElementReflection>,
    #[serde(default)]
    pub glow: Option<ElementGlow>,
    #[serde(default = "default_blend_mode")]
    pub blend_mode: String,
}

fn default_blend_mode() -> String { "normal".to_string() }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TextBlock {
    #[serde(default)]
    pub runs: Vec<TextRun>,
    #[serde(default)]
    pub paragraph_style: TextParagraphStyle,
    #[serde(default)]
    pub effects: ElementEffects,
    #[serde(default = "default_true")]
    pub autofit: bool,
}

fn default_true() -> bool { true }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CropRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum SlideElement {
    TextBlock {
        id: String,
        transform: ElementTransform,
        block: TextBlock,
    },
    Image {
        id: String,
        transform: ElementTransform,
        file_path: String,
        #[serde(default)]
        crop: Option<CropRect>,
        #[serde(default)]
        mask_shape: Option<String>,
        #[serde(default)]
        alt_text: Option<String>,
    },
    Video {
        id: String,
        transform: ElementTransform,
        file_path: String,
        #[serde(default)]
        in_point_s: Option<f64>,
        #[serde(default)]
        out_point_s: Option<f64>,
        #[serde(default = "default_true")]
        loop_playback: bool,
        #[serde(default)]
        is_muted: bool,
        #[serde(default = "default_volume")]
        volume: f64,
    },
    Shape {
        id: String,
        transform: ElementTransform,
        shape_kind: String, // "rect", "ellipse", "triangle", "star", "callout", etc.
        fill_color: String,
        #[serde(default)]
        stroke_color: Option<String>,
        #[serde(default)]
        stroke_width: f64,
    },
    Line {
        id: String,
        transform: ElementTransform,
        line_kind: String, // "straight", "arrow", "elbow", "curved"
        color: String,
        stroke_width: f64,
        #[serde(default)]
        start_arrow: bool,
        #[serde(default)]
        end_arrow: bool,
    },
    Table {
        id: String,
        transform: ElementTransform,
        rows: usize,
        cols: usize,
        cells: Vec<Vec<String>>,
    },
    Group {
        id: String,
        transform: ElementTransform,
        children: Vec<SlideElement>,
    },
}

fn default_volume() -> f64 { 1.0 }

impl SlideElement {
    pub fn id(&self) -> &str {
        match self {
            SlideElement::TextBlock { id, .. } => id,
            SlideElement::Image { id, .. } => id,
            SlideElement::Video { id, .. } => id,
            SlideElement::Shape { id, .. } => id,
            SlideElement::Line { id, .. } => id,
            SlideElement::Table { id, .. } => id,
            SlideElement::Group { id, .. } => id,
        }
    }

    pub fn transform(&self) -> &ElementTransform {
        match self {
            SlideElement::TextBlock { transform, .. } => transform,
            SlideElement::Image { transform, .. } => transform,
            SlideElement::Video { transform, .. } => transform,
            SlideElement::Shape { transform, .. } => transform,
            SlideElement::Line { transform, .. } => transform,
            SlideElement::Table { transform, .. } => transform,
            SlideElement::Group { transform, .. } => transform,
        }
    }

    pub fn transform_mut(&mut self) -> &mut ElementTransform {
        match self {
            SlideElement::TextBlock { transform, .. } => transform,
            SlideElement::Image { transform, .. } => transform,
            SlideElement::Video { transform, .. } => transform,
            SlideElement::Shape { transform, .. } => transform,
            SlideElement::Line { transform, .. } => transform,
            SlideElement::Table { transform, .. } => transform,
            SlideElement::Group { transform, .. } => transform,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GradientStop {
    pub color: String,
    pub offset: f64, // 0.0..1.0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "data")]
pub enum SlideBackground {
    Solid(String),
    Gradient {
        kind: String, // "linear" or "radial"
        stops: Vec<GradientStop>,
        angle_deg: Option<f64>,
    },
    Image {
        file_path: String,
        opacity: f64,
    },
    Video {
        file_path: String,
        loop_playback: bool,
        is_muted: bool,
    },
    Pattern {
        pattern_type: String,
        fg_color: String,
        bg_color: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SlideTransition {
    pub kind: String, // "cut", "crossfade", "slide_left", "slide_right", "wipe_down", "dissolve"
    pub duration_ms: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CcliMetadata {
    pub song_title: Option<String>,
    pub author: Option<String>,
    pub copyright: Option<String>,
    pub ccli_number: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Slide {
    pub text: String,
    pub header: Option<String>,
    pub label: Option<String>,
    pub background: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub elements: Vec<SlideElement>,
    #[serde(default)]
    pub speaker_notes: Option<String>,
    #[serde(default)]
    pub ccli_metadata: Option<CcliMetadata>,
    #[serde(default)]
    pub background_v2: Option<SlideBackground>,
    #[serde(default)]
    pub transition: Option<SlideTransition>,
    #[serde(default)]
    pub slide_document_version: u8,
    /// Per-slide slideshow auto-advance duration override, in seconds. `None`
    /// falls back to the owning `ScheduleItem`'s `default_slide_duration_seconds`.
    #[serde(default)]
    pub duration_seconds: Option<f64>,
    /// Scripture verse reference (e.g. "John 3:16", "2:1"), kept separate from `text`
    /// so a Scripture theme's `reference_position` can render it inline, pinned top,
    /// or pinned bottom instead of it always being baked into the verse body.
    #[serde(default)]
    pub reference_label: Option<String>,
}

impl Default for Slide {
    fn default() -> Self {
        Self {
            text: String::new(),
            header: None,
            label: None,
            background: None,
            notes: None,
            tag: None,
            elements: Vec::new(),
            speaker_notes: None,
            ccli_metadata: None,
            background_v2: None,
            transition: None,
            slide_document_version: 0,
            duration_seconds: None,
            reference_label: None,
        }
    }
}

impl Slide {
    pub fn new_legacy(
        text: String,
        header: Option<String>,
        label: Option<String>,
        background: Option<String>,
        notes: Option<String>,
        tag: Option<String>,
    ) -> Self {
        Self {
            text,
            header,
            label,
            background,
            notes,
            tag,
            elements: Vec::new(),
            speaker_notes: None,
            ccli_metadata: None,
            background_v2: None,
            transition: None,
            slide_document_version: 0,
            duration_seconds: None,
            reference_label: None,
        }
    }

    pub fn synthesize_v2_if_needed(&mut self) {
        if self.elements.is_empty() && !self.text.trim().is_empty() {
            let element = SlideElement::TextBlock {
                id: format!("el_{}", uuid::Uuid::new_v4()),
                transform: ElementTransform::default(),
                block: TextBlock {
                    runs: vec![TextRun {
                        text: self.text.clone(),
                        ..Default::default()
                    }],
                    paragraph_style: TextParagraphStyle::default(),
                    effects: ElementEffects::default(),
                    autofit: true,
                },
            };
            self.elements.push(element);
            self.slide_document_version = 1;
        }
        if self.speaker_notes.is_none() && self.notes.is_some() {
            self.speaker_notes = self.notes.clone();
        }
        if self.background_v2.is_none() {
            if let Some(ref bg) = self.background {
                if !bg.trim().is_empty() {
                    let bg_trim = bg.trim();
                    if bg_trim.starts_with('#') || bg_trim.starts_with("rgb") {
                        self.background_v2 = Some(SlideBackground::Solid(bg_trim.to_string()));
                    } else {
                        self.background_v2 = Some(SlideBackground::Image {
                            file_path: bg_trim.to_string(),
                            opacity: 1.0,
                        });
                    }
                }
            }
        }
    }

    pub fn project_text_from_elements(&mut self) -> String {
        if self.elements.is_empty() {
            return self.text.clone();
        }
        let mut parts = Vec::new();
        for el in &self.elements {
            if let SlideElement::TextBlock { block, .. } = el {
                let block_text = block
                    .runs
                    .iter()
                    .map(|r| r.text.as_str())
                    .collect::<Vec<_>>()
                    .join("");
                if !block_text.trim().is_empty() {
                    parts.push(block_text);
                }
            }
        }
        let result = if parts.is_empty() {
            self.text.clone()
        } else {
            parts.join("\n\n")
        };
        self.text = result.clone();
        result
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SlideDocument {
    pub elements: Vec<SlideElement>,
    #[serde(default)]
    pub background: Option<SlideBackground>,
    #[serde(default)]
    pub transition: Option<SlideTransition>,
    #[serde(default)]
    pub speaker_notes: Option<String>,
    #[serde(default)]
    pub ccli_metadata: Option<CcliMetadata>,
    #[serde(default)]
    pub version: u8,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Song {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub alternate_title: Option<String>,
    pub author: String,
    pub copyright: Option<String>,
    pub ccli_number: Option<String>,
    pub slides: Vec<Slide>,
    #[serde(default)]
    pub theme_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScriptureVerse {
    pub verse_number: u32,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScriptureItem {
    pub id: String,
    pub book: String,
    pub chapter: u32,
    pub verse_start: u32,
    pub verse_end: u32,
    pub version: String,
    pub reference: String,
    pub verses: Vec<ScriptureVerse>,
    #[serde(default)]
    pub theme_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MediaItem {
    pub id: String,
    pub name: String,
    pub media_type: String, // "video", "image", "audio"
    pub file_path: String,
    #[serde(default)]
    pub duration_seconds: Option<f64>,
    #[serde(default)]
    pub thumbnail_path: Option<String>,
    #[serde(default)]
    pub loop_playback: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Presentation {
    pub id: String,
    pub title: String,
    pub author: String,
    pub slides: Vec<Slide>,
    #[serde(default)]
    pub theme_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArrangementEntry {
    /// Stable identifier of the master section, e.g. "V1", "C1", "B".
    /// Matches the tag of the source slide.
    pub section_id: String,
    /// Index into `ScheduleItem.slides` indicating which master slide to render.
    pub source_slide_index: usize,
    /// Per-play background override. None = inherit from master slide.
    #[serde(default)]
    pub background_override: Option<String>,
}

/// Generates an identity arrangement mapping each slide to its authored position (1:1).
/// `section_id` derives from source slide's `tag` when present, else `S{n}` where `n = source_slide_index + 1`.
pub fn identity_arrangement(slides: &[Slide]) -> Vec<ArrangementEntry> {
    slides
        .iter()
        .enumerate()
        .map(|(idx, slide)| ArrangementEntry {
            section_id: slide
                .tag
                .as_ref()
                .or(slide.label.as_ref())
                .map(|t| t.trim())
                .filter(|t| !t.is_empty())
                .map(|t| t.to_string())
                .unwrap_or_else(|| format!("S{}", idx + 1)),
            source_slide_index: idx,
            background_override: None,
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScheduleItem {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    pub item_type: String, // "song", "scripture", "presentation", "media", "header"
    #[serde(default)]
    pub author_or_ref: String,
    pub slides: Vec<Slide>,
    #[serde(default)]
    pub background: Option<String>,
    /// Name of the `Theme` applied via drag-and-drop or the Theme Editor. Resolved
    /// against the live themes list at render time (see `resolveThemeAt` in
    /// web/src/core/presentation_helpers.ts) for typography — `background` above is a
    /// literal snapshot taken at apply-time so it still works if the theme is renamed.
    #[serde(default)]
    pub theme_name: Option<String>,
    #[serde(default)]
    pub is_expanded: bool,
    #[serde(default)]
    pub is_section_header: bool,
    #[serde(default)]
    pub arrangement: Vec<ArrangementEntry>,
    /// Fallback slideshow auto-advance duration (seconds) for any slide that has
    /// no per-slide `Slide::duration_seconds` override. `None` disables auto-advance
    /// for slides without their own override.
    #[serde(default)]
    pub default_slide_duration_seconds: Option<f64>,
    /// Whether slideshow playback should loop back to the first slide on reaching
    /// the end, instead of stopping.
    #[serde(default)]
    pub slideshow_loop: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Schedule {
    pub id: String,
    pub title: String,
    pub items: Vec<ScheduleItem>,
    /// Deprecated: moved to `ShowState::selected_item_index`, which the
    /// WebSocket broadcast diffing (see `schedule_version` below) always
    /// sends, unlike this field — kept only so old serialized schedules
    /// deserialize without error; no code reads or writes it any more.
    #[serde(default)]
    pub selected_item_index: Option<usize>,
    #[serde(default)]
    pub is_modified: bool,
    /// Incremented by `ShowEngine::apply_event` whenever an event actually mutates the
    /// schedule (item/slide add/remove/reorder/edit), and left untouched by events that
    /// only touch `ShowState` (live/staged navigation, blackout, alerts, media transport,
    /// etc). Lets the WebSocket broadcast skip re-sending the full schedule — every item,
    /// slide, and canvas element — on state-only changes; see `src/api/ws.rs`.
    #[serde(default)]
    pub schedule_version: u64,
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            title: "Sunday Worship Service".to_string(),
            items: Vec::new(),
            selected_item_index: None,
            is_modified: false,
            schedule_version: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Theme {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default = "default_bg")]
    pub background: String,
    #[serde(default = "default_font_family")]
    pub font_family: String,
    #[serde(default = "default_font_size")]
    pub font_size: String,
    #[serde(default = "default_font_color")]
    pub font_color: String,
    #[serde(default = "default_alignment")]
    pub alignment: String,
    #[serde(default = "default_text_shadow")]
    pub text_shadow: String,
    #[serde(default = "default_line_height")]
    pub line_height: String,
    #[serde(default = "default_letter_spacing")]
    pub letter_spacing: String,
    #[serde(default = "default_opacity")]
    pub opacity: String,
    #[serde(default = "default_margin")]
    pub margin_top: String,
    #[serde(default = "default_margin")]
    pub margin_bottom: String,
    #[serde(default = "default_margin")]
    pub margin_left: String,
    #[serde(default = "default_margin")]
    pub margin_right: String,
    #[serde(default = "default_valign")]
    pub vertical_align: String,
    /// "song" | "scripture" | "presentation" — governs which schedule item types this
    /// theme can be applied to and which fields its editor exposes (e.g. Scripture-only
    /// `reference_position`).
    #[serde(default = "default_category")]
    pub category: String,
    /// Exactly one theme per category may have this set — enforced by
    /// `Database::set_default_theme`, not by this struct. Newly added songs/scriptures/
    /// presentations auto-apply their category's default theme.
    #[serde(default)]
    pub is_default: bool,
    /// Scripture themes only: "inline" (baked into the verse text, today's only
    /// behavior), "top", or "bottom" (rendered as a separately positioned overlay).
    #[serde(default = "default_reference_position")]
    pub reference_position: String,
    /// Lower-third / streaming support: when enabled, the resolved background is
    /// forced to `chroma_key_color` (a flat color for OBS/video-switcher keying)
    /// regardless of any image/gradient/video background otherwise configured.
    #[serde(default)]
    pub chroma_key_enabled: bool,
    #[serde(default = "default_chroma_key_color")]
    pub chroma_key_color: String,
    /// 0 = full-bleed (default). >0 constrains text to the bottom N% of the screen,
    /// for lower-third-style overlays on a camera feed.
    #[serde(default)]
    pub safe_area_percent: u8,
}

fn default_category() -> String { "song".to_string() }
fn default_reference_position() -> String { "inline".to_string() }
fn default_chroma_key_color() -> String { "#00ff00".to_string() }

impl Default for Theme {
    fn default() -> Self {
        Self {
            id: None,
            name: String::new(),
            background: default_bg(),
            font_family: default_font_family(),
            font_size: default_font_size(),
            font_color: default_font_color(),
            alignment: default_alignment(),
            text_shadow: default_text_shadow(),
            line_height: default_line_height(),
            letter_spacing: default_letter_spacing(),
            opacity: default_opacity(),
            margin_top: default_margin(),
            margin_bottom: default_margin(),
            margin_left: default_margin(),
            margin_right: default_margin(),
            vertical_align: default_valign(),
            category: default_category(),
            is_default: false,
            reference_position: default_reference_position(),
            chroma_key_enabled: false,
            chroma_key_color: default_chroma_key_color(),
            safe_area_percent: 0,
        }
    }
}

fn default_bg() -> String { "#102027".to_string() }
fn default_font_family() -> String { "Inter, sans-serif".to_string() }
fn default_font_size() -> String { "48px".to_string() }
fn default_font_color() -> String { "#ffffff".to_string() }
fn default_alignment() -> String { "center".to_string() }
fn default_text_shadow() -> String { "2px 2px 8px rgba(0,0,0,0.8)".to_string() }
fn default_line_height() -> String { "1.3".to_string() }
fn default_letter_spacing() -> String { "0px".to_string() }
fn default_opacity() -> String { "1.0".to_string() }
fn default_margin() -> String { "5%".to_string() }
fn default_valign() -> String { "center".to_string() }

/// A reusable, author-designed slide layout: a positioned set of elements
/// plus an optional background, saved from the canvas editor and re-applied
/// to seed new slides or replace an existing one. Deliberately its own
/// struct rather than extending `Theme` (a flat CSS-string preset with no
/// element concept) or reusing `Slide` (which carries schedule-positional
/// text/notes/CCLI concerns a reusable pattern doesn't need).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SlideTemplate {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub elements: Vec<SlideElement>,
    #[serde(default)]
    pub background: Option<SlideBackground>,
    #[serde(default)]
    pub thumbnail_data_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "duration_ms")]
pub enum TransitionType {
    Cut,
    Crossfade(u32),
    SlideLeft(u32),
    SlideRight(u32),
    WipeDown(u32),
    Dissolve(u32),
}

impl Default for TransitionType {
    fn default() -> Self {
        TransitionType::Crossfade(300)
    }
}

/// Live-projected external web/video feed (YouTube embeds, arbitrary iframe-able pages).
/// Set via `ShowCommand::SetWebStream`, cleared by sending `None`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WebStreamState {
    /// Final, directly-embeddable URL (already normalized client-side, e.g. a YouTube watch
    /// link is converted to a youtube-nocookie.com/embed URL before it ever reaches here).
    pub url: String,
    /// One of "background" (full-bleed, behind lyrics), "pip" (lower-third corner box), or
    /// "fullscreen" (covers everything except the blackout override).
    pub mode: String,
}

/// Server-authoritative state for a dedicated media (video) item playing as the primary
/// live content, e.g. an announcements video — as opposed to a slide's background loop,
/// which has no playback state of its own. Populated by the `Media*` commands/events
/// (`MediaScheduledStart`, `MediaPaused`, `MediaLoopSet`, `MediaMuteSet`, `MediaVolumeSet`,
/// `MediaPrerolled`) and consumed by every connected display client to stay in sync with
/// each other, since they only share this value via the server, never directly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MediaPlaybackState {
    pub is_playing: bool,
    /// Playback position in seconds as of `timestamp_ms`. A client currently playing should
    /// project this forward using `timestamp_ms`/`start_at_epoch_ms`, not treat it as live.
    pub current_time: f64,
    /// Server wall-clock time (ms since epoch) when `current_time` was recorded.
    pub timestamp_ms: u64,
    /// When set, playback should begin/resume at this future epoch ms so every client starts
    /// the same frame at the same instant, rather than each client starting as soon as it
    /// individually receives the command.
    pub start_at_epoch_ms: Option<u64>,
    pub is_looping: bool,
    pub is_muted: bool,
    pub volume: f64,
    /// Increments on every change; lets a client detect it missed an update (e.g. a dropped
    /// poll) rather than silently acting on stale data.
    pub sync_version: u32,
}

impl Default for MediaPlaybackState {
    fn default() -> Self {
        Self {
            is_playing: false,
            current_time: 0.0,
            timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
            start_at_epoch_ms: None,
            is_looping: false,
            is_muted: false,
            volume: 1.0,
            sync_version: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShowState {
    pub live_item: Option<ScheduleItem>,
    pub live_slide_index: usize,
    pub staged_item: Option<ScheduleItem>,
    pub staged_slide_index: usize,
    pub is_blackout: bool,
    pub is_clear_text: bool,
    #[serde(rename = "is_logo_override", alias = "is_logo")]
    pub is_logo: bool,
    #[serde(rename = "alert_message", alias = "alert_text")]
    pub alert_text: Option<String>,
    pub global_theme: Option<String>,
    pub global_background: Option<String>,
    pub web_stream: Option<WebStreamState>,
    pub transition: TransitionType,
    #[serde(default)]
    pub media_playback: Option<MediaPlaybackState>,
    pub sequence_number: u64,
    pub last_event_id: String,
    pub server_time_ms: u64,
    /// Per-letter "last visited slide index" for the JumpSection ('V'/'C'/
    /// 'B'/etc.) keyboard shortcuts — lets repeated presses of the same
    /// letter cycle forward through that section type (verse 1, then 2,
    /// then 3...) instead of always landing back on the first match, and
    /// keeps each letter's position independent (jumping to Chorus and
    /// back to Verse resumes at the next verse after the one you left, not
    /// verse 1). Updated on every live slide change to whichever letter the
    /// new slide's section belongs to, so manually stepping through slides
    /// (Next/Prev/numbered jump) also keeps this in sync — not just
    /// JumpSection itself. Reset whenever a different item goes live, since
    /// verse positions from one song aren't meaningful in another. Keyed by
    /// uppercased single letter (e.g. "V"); absent = never visited yet.
    #[serde(default)]
    pub section_jump_cursor: HashMap<String, usize>,
    /// Which schedule item the operator is currently focused on — the target
    /// for NextItem/PrevItem and the "selected" (orange) row highlight.
    /// Lives here, not on `Schedule`, because it's set by `ItemWentLive`/
    /// `ItemStaged` (state-only events that don't bump `Schedule::schedule_version`
    /// and so aren't guaranteed to reach clients via the schedule field of a
    /// diffed WS broadcast — see `src/api/ws.rs`); `ShowState` itself is always
    /// broadcast in full, so this always reaches clients on every command.
    #[serde(default)]
    pub selected_item_index: Option<usize>,
}

impl Default for ShowState {
    fn default() -> Self {
        Self {
            live_item: None,
            live_slide_index: 0,
            staged_item: None,
            staged_slide_index: 0,
            is_blackout: false,
            is_clear_text: false,
            is_logo: false,
            alert_text: None,
            global_theme: None,
            global_background: None,
            web_stream: None,
            transition: TransitionType::default(),
            media_playback: None,
            sequence_number: 0,
            last_event_id: String::new(),
            server_time_ms: chrono::Utc::now().timestamp_millis() as u64,
            section_jump_cursor: HashMap::new(),
            selected_item_index: None,
        }
    }
}

pub trait ToScheduleItem {
    fn to_schedule_item(&self) -> ScheduleItem;
}

/// A slide beyond this many lines gets split into two — the operator never
/// has to notice or act on it. Chosen well above a typical 4-6 line verse so
/// normal slides are untouched, but low enough to catch two stanzas that
/// were run together as one overlong "verse" (see split_one_slide below).
const MAX_SLIDE_LINES: usize = 8;

/// Runs every slide through `split_one_slide`, expanding any that come back
/// split into two. Applied wherever a song enters a schedule (see
/// `ToScheduleItem for Song` below) so the Slide Editor, Preview and Live
/// output all see the same already-split slides with no operator action —
/// there is no separate "split" UI to find or remember to use.
fn split_long_slides(slides: Vec<Slide>) -> Vec<Slide> {
    let mut out = Vec::with_capacity(slides.len());
    for slide in slides {
        match split_one_slide(&slide) {
            Some((a, b)) => {
                out.push(a);
                out.push(b);
            }
            None => out.push(slide),
        }
    }
    out
}

/// Splits one slide's text into two at an automatically chosen line
/// boundary — the blank line nearest the midpoint if one exists (the
/// natural seam between two stanzas run together with no separator),
/// otherwise the midpoint by line count. Labels, headers and tags get an
/// "A"/"B" suffix (label "V2" / header "Verse 2" -> "V2A"/"Verse 2A" and
/// "V2B"/"Verse 2B") so both halves still match the same jump-to-section
/// prefix. Returns None (no split) when the slide is short enough already,
/// or has no text to split on.
fn split_one_slide(slide: &Slide) -> Option<(Slide, Slide)> {
    let text_el_idx = slide
        .elements
        .iter()
        .position(|e| matches!(e, SlideElement::TextBlock { .. }));

    // Flatten every run's text into individual lines, remembering which run
    // (by index into block.runs) each line came from, so per-run styling
    // (bold/italic/color/etc.) survives the split.
    let lines: Vec<(String, usize)> = match text_el_idx {
        Some(idx) => {
            if let SlideElement::TextBlock { block, .. } = &slide.elements[idx] {
                block
                    .runs
                    .iter()
                    .enumerate()
                    .flat_map(|(run_idx, run)| {
                        run.text
                            .split('\n')
                            .map(|line| (line.to_string(), run_idx))
                            .collect::<Vec<_>>()
                    })
                    .collect()
            } else {
                unreachable!()
            }
        }
        None => slide
            .text
            .split('\n')
            .map(|line| (line.to_string(), 0))
            .collect(),
    };

    if lines.len() <= MAX_SLIDE_LINES {
        return None;
    }

    let midpoint = lines.len() as f64 / 2.0;
    let mut split_at = midpoint.ceil() as usize;
    let mut best_dist = f64::INFINITY;
    for (i, (text, _)) in lines.iter().enumerate() {
        if text.trim().is_empty() && i > 0 {
            let dist = (i as f64 - midpoint).abs();
            if dist < best_dist {
                best_dist = dist;
                split_at = i + 1; // the blank line ends part A
            }
        }
    }
    split_at = split_at.clamp(1, lines.len() - 1);

    let (lines_a, lines_b) = lines.split_at(split_at);
    if lines_a.is_empty() || lines_b.is_empty() {
        return None;
    }

    let build_runs = |slice: &[(String, usize)], source_runs: &[TextRun]| -> Vec<TextRun> {
        let mut runs_out: Vec<(TextRun, usize)> = Vec::new();
        for (text, run_idx) in slice {
            if let Some((last_run, last_idx)) = runs_out.last_mut() {
                if last_idx == run_idx {
                    last_run.text.push('\n');
                    last_run.text.push_str(text);
                    continue;
                }
            }
            let mut cloned = source_runs
                .get(*run_idx)
                .cloned()
                .unwrap_or_default();
            cloned.text = text.clone();
            runs_out.push((cloned, *run_idx));
        }
        runs_out.into_iter().map(|(r, _)| r).collect()
    };

    let suffix = |base: &Option<String>, letter: &str| base.as_ref().map(|s| format!("{}{}", s, letter));

    let mut slide_a = slide.clone();
    let mut slide_b = slide.clone();

    if let Some(idx) = text_el_idx {
        let source_runs = if let SlideElement::TextBlock { block, .. } = &slide.elements[idx] {
            block.runs.clone()
        } else {
            Vec::new()
        };
        if let SlideElement::TextBlock { block, .. } = &mut slide_a.elements[idx] {
            block.runs = build_runs(lines_a, &source_runs);
        }
        if let SlideElement::TextBlock { block, id, .. } = &mut slide_b.elements[idx] {
            block.runs = build_runs(lines_b, &source_runs);
            *id = format!("{}-b", id);
        }
        slide_a.project_text_from_elements();
        slide_b.project_text_from_elements();
    } else {
        slide_a.text = lines_a.iter().map(|(t, _)| t.clone()).collect::<Vec<_>>().join("\n");
        slide_b.text = lines_b.iter().map(|(t, _)| t.clone()).collect::<Vec<_>>().join("\n");
    }

    slide_a.label = suffix(&slide.label, "A");
    slide_a.header = suffix(&slide.header, "A");
    slide_a.tag = suffix(&slide.tag, "A");
    slide_b.label = suffix(&slide.label, "B");
    slide_b.header = suffix(&slide.header, "B");
    slide_b.tag = suffix(&slide.tag, "B");

    Some((slide_a, slide_b))
}

impl ToScheduleItem for Song {
    fn to_schedule_item(&self) -> ScheduleItem {
        let slides = split_long_slides(self.slides.clone());
        let arrangement = identity_arrangement(&slides);
        ScheduleItem {
            id: format!("sched_song_{}", self.id),
            title: self.title.clone(),
            item_type: "song".to_string(),
            author_or_ref: self.author.clone(),
            slides,
            background: None,
            theme_name: self.theme_name.clone(),
            is_expanded: false,
            is_section_header: false,
            notes: None,
            subtitle: None,
            arrangement,
            default_slide_duration_seconds: None,
            slideshow_loop: false,
        }
    }
}

impl ToScheduleItem for ScriptureItem {
    fn to_schedule_item(&self) -> ScheduleItem {
        let slides: Vec<Slide> = self.verses.iter().map(|v| Slide {
            text: v.text.clone(),
            header: Some(format!("{} {}:{}", self.book, self.chapter, v.verse_number)),
            label: Some(format!("V{}", v.verse_number)),
            background: None,
            notes: None,
            tag: Some(format!("V{}", v.verse_number)),
            reference_label: Some(format!("{} {}:{}", self.book, self.chapter, v.verse_number)),
            ..Default::default()
        }).collect();
        let arrangement = identity_arrangement(&slides);

        ScheduleItem {
            id: format!("sched_scrip_{}", self.id),
            title: self.reference.clone(),
            item_type: "scripture".to_string(),
            author_or_ref: self.version.clone(),
            slides,
            background: None,
            theme_name: self.theme_name.clone(),
            is_expanded: false,
            is_section_header: false,
            notes: None,
            subtitle: None,
            arrangement,
            default_slide_duration_seconds: None,
            slideshow_loop: false,
        }
    }
}

impl ToScheduleItem for Presentation {
    fn to_schedule_item(&self) -> ScheduleItem {
        let arrangement = identity_arrangement(&self.slides);
        ScheduleItem {
            id: format!("sched_pres_{}", self.id),
            title: self.title.clone(),
            item_type: "presentation".to_string(),
            author_or_ref: self.author.clone(),
            slides: self.slides.clone(),
            background: None,
            theme_name: self.theme_name.clone(),
            is_expanded: false,
            is_section_header: false,
            notes: None,
            subtitle: None,
            arrangement,
            default_slide_duration_seconds: None,
            slideshow_loop: false,
        }
    }
}

impl ToScheduleItem for MediaItem {
    fn to_schedule_item(&self) -> ScheduleItem {
        let slides = vec![Slide {
            text: String::new(),
            header: Some(self.name.clone()),
            label: Some("1".to_string()),
            background: Some(self.file_path.clone()),
            notes: None,
            tag: Some("1".to_string()),
            ..Default::default()
        }];
        let arrangement = identity_arrangement(&slides);
        ScheduleItem {
            id: format!("sched_media_{}", self.id),
            title: self.name.clone(),
            item_type: "media".to_string(),
            author_or_ref: self.media_type.clone(),
            slides,
            background: Some(self.file_path.clone()),
            theme_name: None,
            is_expanded: false,
            is_section_header: false,
            notes: None,
            subtitle: None,
            arrangement,
            default_slide_duration_seconds: None,
            slideshow_loop: false,
        }
    }
}


impl Song {
    pub fn new(title: &str, author: &str) -> Self {
        Self {
            id: format!("song_{}", uuid::Uuid::new_v4()),
            title: title.to_string(),
            author: author.to_string(),
            copyright: None,
            ccli_number: None,
            slides: Vec::new(),
            theme_name: None, alternate_title: None,
        }
    }
    
    pub fn add_slide(&mut self, label: impl Into<String>, header: impl Into<String>, text: impl Into<String>) {
        let label_str = label.into();
        self.slides.push(Slide {
            text: text.into(),
            header: Some(header.into()),
            label: Some(label_str.clone()),
            background: None,
            notes: None,
            tag: Some(label_str),
            ..Default::default()
        });
    }
}

pub enum ScheduleItemType {
    Song,
    Scripture,
    Media,
    Presentation,
    Header,
}

impl ScheduleItem {
    pub fn new(item_type: ScheduleItemType, title: &str) -> Self {
        let type_str = match item_type {
            ScheduleItemType::Song => "song",
            ScheduleItemType::Scripture => "scripture",
            ScheduleItemType::Media => "media",
            ScheduleItemType::Presentation => "presentation",
            ScheduleItemType::Header => "header",
        };
        Self {
            id: format!("sched_{}_{}", type_str, uuid::Uuid::new_v4()),
            title: title.to_string(),
            item_type: type_str.to_string(),
            author_or_ref: String::new(),
            slides: Vec::new(),
            background: None,
            theme_name: None,
            is_expanded: false,
            is_section_header: matches!(item_type, ScheduleItemType::Header),
            notes: None,
            subtitle: None,
            arrangement: Vec::new(),
            default_slide_duration_seconds: None,
            slideshow_loop: false,
        }
    }
}

impl ScriptureItem {
    pub fn new(book: &str, chapter: u32, verse_start: u32, verse_end: u32, version: &str) -> Self {
        Self {
            id: format!("scrip_{}", uuid::Uuid::new_v4()),
            book: book.to_string(),
            chapter,
            verse_start,
            verse_end,
            version: version.to_string(),
            reference: format!("{} {}:{}-{}", book, chapter, verse_start, verse_end),
            verses: Vec::new(),
            theme_name: None,
        }
    }
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MediaType {
    Video,
    Audio,
    Image,
}

impl MediaItem {
    pub fn new(name: &str, file_path: &str, media_type: MediaType) -> Self {
        let type_str = match media_type {
            MediaType::Video => "video",
            MediaType::Audio => "audio",
            MediaType::Image => "image",
        };
        Self {
            id: format!("media_{}", uuid::Uuid::new_v4()),
            name: name.to_string(),
            media_type: type_str.to_string(),
            file_path: file_path.to_string(),
            duration_seconds: None,
            thumbnail_path: None,
            loop_playback: matches!(media_type, MediaType::Video),
        }
    }
}
