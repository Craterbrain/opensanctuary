use std::collections::HashMap;
use std::path::Path;
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::core::models::{
    ArrangementEntry, ElementEffects, ElementTransform, Schedule,
    ScheduleItem, ScheduleItemType, Slide, SlideBackground, SlideElement,
    TextBlock, TextParagraphStyle, TextRun,
};
use crate::storage::openlp_import::slide_background_to_legacy;

/// Importer for FreeShow show files (`.show` and JSON project formats).
///
/// FreeShow stores presentations as JSON in one of two formats:
/// 1. Array wrapper: `[ id_string, show_object ]` (as exported by `exportShow`)
/// 2. Direct object: `{ ...show_object }`
///
/// The struct shapes below (`FreeShowShow`/`FreeShowSlide`/`FreeShowItem`/`FreeShowLine`/
/// `FreeShowMedia`, etc.) are checked field-for-field against FreeShow's own real
/// TypeScript type definitions — `src/types/Show.ts` in ChurchApps/FreeShow at commit
/// `95d57096245e` (github.com/ChurchApps/FreeShow) — not guessed from general
/// knowledge of what a slideshow JSON format might look like. In particular the
/// nested `Line.text[] -> { value, style, customType }` run-level shape and
/// `Slide.settings.{background, color, backgroundImage}` match the real source
/// exactly. Only the fields this importer actually needs were modeled (serde
/// ignores unrecognized JSON fields by default, so this is safe); many real fields
/// unrelated to text/background import (timers, MIDI, cameras, charts, etc.) are
/// intentionally not represented here.
///
/// Supports:
/// - Slide sequencing via activeLayout (with fallback to slide collection)
/// - Slide groups and arrangement entries (Verse, Chorus, Bridge, etc.)
/// - Rich per-span text formatting (bold, italic, underline, strike, color, font-family, font-size, letter-spacing)
/// - Paragraph alignment (left, center, right, justified)
/// - Slide backgrounds: solid colors, embedded base64 images/videos, and external
///   URLs/local file paths (copied into the app's own media directory when local)
pub struct FreeShowShowImporter;

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct FreeShowShow {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub meta: Option<FreeShowMeta>,
    #[serde(default)]
    pub settings: Option<FreeShowSettings>,
    #[serde(default)]
    pub slides: HashMap<String, FreeShowSlide>,
    #[serde(default)]
    pub layouts: HashMap<String, FreeShowLayout>,
    #[serde(default)]
    pub media: HashMap<String, FreeShowMedia>,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct FreeShowMeta {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub author: Option<String>,
    pub composer: Option<String>,
    pub publisher: Option<String>,
    pub copyright: Option<String>,
    #[serde(rename = "CCLI")]
    pub ccli: Option<String>,
    pub year: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct FreeShowSettings {
    #[serde(rename = "activeLayout")]
    pub active_layout: Option<String>,
    pub template: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct FreeShowLayout {
    pub id: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub slides: Vec<FreeShowLayoutSlide>,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct FreeShowLayoutSlide {
    pub id: String,
    pub parent: Option<String>,
    pub background: Option<String>,
    pub color: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct FreeShowSlide {
    pub id: Option<String>,
    pub group: Option<String>,
    pub color: Option<String>,
    pub children: Option<Vec<String>>,
    pub notes: Option<String>,
    #[serde(default)]
    pub items: Vec<FreeShowItem>,
    #[serde(default)]
    pub settings: Option<FreeShowSlideSettings>,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct FreeShowSlideSettings {
    pub color: Option<String>,
    #[serde(rename = "backgroundImage")]
    pub background_image: Option<String>,
    pub background: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct FreeShowItem {
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub item_type: Option<String>,
    pub style: Option<String>,
    pub align: Option<String>,
    pub lines: Option<Vec<FreeShowLine>>,
    pub src: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct FreeShowLine {
    pub align: Option<String>,
    #[serde(default)]
    pub text: Vec<FreeShowTextSpan>,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct FreeShowTextSpan {
    #[serde(default)]
    pub value: String,
    pub style: Option<String>,
    #[serde(rename = "customType")]
    pub custom_type: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct FreeShowMedia {
    pub id: Option<String>,
    pub name: Option<String>,
    pub path: Option<String>,
    pub base64: Option<String>,
    #[serde(rename = "type")]
    pub media_type: Option<String>,
    #[serde(rename = "loop")]
    pub loop_playback: Option<bool>,
    pub muted: Option<bool>,
}

impl FreeShowShowImporter {
    /// Import a FreeShow `.show` file from raw bytes.
    pub fn import_show_bytes(
        bytes: &[u8],
        media_dir: &Path,
    ) -> Result<Schedule, Box<dyn std::error::Error>> {
        let text = std::str::from_utf8(bytes)?;
        Self::import_show_json_str(text, media_dir)
    }

    /// Import a FreeShow `.show` file from a JSON string.
    pub fn import_show_json_str(
        json_str: &str,
        media_dir: &Path,
    ) -> Result<Schedule, Box<dyn std::error::Error>> {
        let show = Self::parse_show_json(json_str)?;
        Self::build_schedule_from_show(&show, media_dir)
    }

    /// Parse FreeShow JSON, supporting both `[id, show]` and `{ ...show }` structures.
    pub fn parse_show_json(json_str: &str) -> Result<FreeShowShow, Box<dyn std::error::Error>> {
        let trimmed = json_str.trim().trim_start_matches('\u{feff}'); // Strip BOM if present
        if trimmed.starts_with('[') {
            let pair: (serde_json::Value, FreeShowShow) = serde_json::from_str(trimmed)?;
            Ok(pair.1)
        } else {
            let show: FreeShowShow = serde_json::from_str(trimmed)?;
            Ok(show)
        }
    }

    /// Construct a Schedule and ScheduleItem from a parsed FreeShowShow.
    pub fn build_schedule_from_show(
        show: &FreeShowShow,
        media_dir: &Path,
    ) -> Result<Schedule, Box<dyn std::error::Error>> {
        let title = show
            .meta
            .as_ref()
            .and_then(|m| m.title.as_deref())
            .or_else(|| show.name.as_deref())
            .filter(|t| !t.trim().is_empty())
            .unwrap_or("FreeShow Presentation")
            .to_string();

        let author = show
            .meta
            .as_ref()
            .and_then(|m| m.author.as_deref().or_else(|| m.artist.as_deref()))
            .filter(|a| !a.trim().is_empty())
            .unwrap_or("Unknown")
            .to_string();

        // 1. Determine slide ordering via active layout or slide collection
        let ordered_slide_entries = Self::determine_slide_order(show);

        let mut slides = Vec::new();
        let mut arrangements = Vec::new();
        let mut has_verse_or_chorus = false;

        for (slide_idx, (slide_ref, layout_bg_opt)) in ordered_slide_entries.iter().enumerate() {
            let mut slide = Slide::default();

            // Set slide label & group tag
            if let Some(ref group) = slide_ref.group {
                if !group.trim().is_empty() {
                    let tag = shorten_group_tag(group);
                    slide.label = Some(group.clone());
                    slide.header = Some(tag.clone());
                    slide.tag = Some(tag.clone());
                    let lower = group.to_lowercase();
                    if lower.contains("verse") || lower.contains("chorus") || lower.contains("bridge") || lower.contains("pre") {
                        has_verse_or_chorus = true;
                    }
                    arrangements.push(ArrangementEntry {
                        section_id: tag,
                        source_slide_index: slide_idx,
                        background_override: None,
                    });
                }
            }
            if slide.label.is_none() {
                slide.label = Some(format!("Slide {}", slide_idx + 1));
            }

            // Extract text, runs, and alignment from items
            let (plain_text, runs, alignment, has_formatting) = Self::extract_slide_text_and_runs(&slide_ref.items);
            slide.text = plain_text;

            if has_formatting && !runs.is_empty() {
                slide.elements = vec![SlideElement::TextBlock {
                    id: format!("tb_{}", uuid::Uuid::new_v4()),
                    transform: ElementTransform::default(),
                    block: TextBlock {
                        runs,
                        paragraph_style: TextParagraphStyle {
                            align: alignment,
                            line_height: 1.25,
                            ..Default::default()
                        },
                        effects: ElementEffects::default(),
                        autofit: true,
                    },
                }];
            }

            // Resolve background
            if let Some(bg) = Self::resolve_slide_background(slide_ref, layout_bg_opt.as_deref(), show, media_dir) {
                slide.background = Some(slide_background_to_legacy(&bg));
                slide.background_v2 = Some(bg);
            }

            slides.push(slide);
        }

        let item_type = if has_verse_or_chorus {
            ScheduleItemType::Song
        } else {
            ScheduleItemType::Presentation
        };

        let mut item = ScheduleItem::new(item_type, &title);
        item.subtitle = Some(author.clone());
        item.slides = slides;
        item.arrangement = arrangements;

        if let Some(ref m) = show.meta {
            if let Some(ref c) = m.ccli {
                item.notes = Some(format!("CCLI: {}", c));
            }
        }

        let schedule = Schedule {
            id: format!("sched_{}", uuid::Uuid::new_v4()),
            title,
            items: vec![item],
            selected_item_index: Some(0),
            is_modified: false,
            schedule_version: 0,
        };

        Ok(schedule)
    }

    /// Walk layouts to establish slide order, including child slides.
    fn determine_slide_order(show: &FreeShowShow) -> Vec<(FreeShowSlide, Option<String>)> {
        let mut result = Vec::new();

        if let Some(ref settings) = show.settings {
            if let Some(ref active_layout_id) = settings.active_layout {
                if let Some(layout) = show.layouts.get(active_layout_id) {
                    for layout_slide in &layout.slides {
                        if let Some(slide) = show.slides.get(&layout_slide.id) {
                            result.push((slide.clone(), layout_slide.background.clone()));
                            if let Some(ref children) = slide.children {
                                for child_id in children {
                                    if let Some(child) = show.slides.get(child_id) {
                                        result.push((child.clone(), layout_slide.background.clone()));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Fallback: If layout resolution returned no slides, iterate over the slide collection
        if result.is_empty() {
            let mut sorted_slides: Vec<_> = show.slides.iter().collect();
            sorted_slides.sort_by_key(|(k, _)| (*k).clone());
            for (_, slide) in sorted_slides {
                result.push((slide.clone(), None));
            }
        }

        result
    }

    /// Extract plain text, TextRuns, alignment, and formatting flag from FreeShow items.
    fn extract_slide_text_and_runs(
        items: &[FreeShowItem],
    ) -> (String, Vec<TextRun>, String, bool) {
        let mut plain_text = String::new();
        let mut runs = Vec::new();
        let mut detected_alignment = "left".to_string();
        let mut has_formatting = false;
        let mut first_item = true;

        for item in items {
            let lines = match &item.lines {
                Some(l) if !l.is_empty() => l,
                _ => continue,
            };

            if !first_item {
                plain_text.push_str("\n\n");
                runs.push(TextRun {
                    text: "\n\n".to_string(),
                    ..Default::default()
                });
            }
            first_item = false;

            let item_style = parse_css_style(item.style.as_deref().unwrap_or(""));

            for (line_idx, line) in lines.iter().enumerate() {
                if line_idx == 0 {
                    if let Some(align_str) = line.align.as_deref().or(item.align.as_deref()) {
                        detected_alignment = parse_text_alignment(align_str);
                    }
                }

                if line_idx > 0 {
                    plain_text.push('\n');
                    runs.push(TextRun {
                        text: "\n".to_string(),
                        ..Default::default()
                    });
                }

                for span in &line.text {
                    if span.value.is_empty() {
                        continue;
                    }
                    plain_text.push_str(&span.value);

                    let span_style = parse_css_style(span.style.as_deref().unwrap_or(""));
                    let merged = item_style.merge(&span_style);

                    let bold = merged.bold.unwrap_or(false);
                    let italic = merged.italic.unwrap_or(false);
                    let underline = merged.underline.unwrap_or(false);
                    let strike = merged.strike.unwrap_or(false);
                    let color = merged.color.unwrap_or_else(|| "#ffffff".to_string());
                    let font_family = merged.font_family.unwrap_or_else(|| "Inter, sans-serif".to_string());
                    let font_size_pt = merged.font_size_pt.unwrap_or(36.0);
                    let letter_spacing_px = merged.letter_spacing_px.unwrap_or(0.0);

                    if bold
                        || italic
                        || underline
                        || strike
                        || color != "#ffffff"
                        || font_family != "Inter, sans-serif"
                        || (font_size_pt - 36.0).abs() > 0.1
                        || letter_spacing_px.abs() > 0.01
                    {
                        has_formatting = true;
                    }

                    runs.push(TextRun {
                        text: span.value.clone(),
                        bold,
                        italic,
                        underline,
                        strike,
                        color,
                        font_family,
                        font_size_pt,
                        letter_spacing_px,
                        baseline_shift: "normal".to_string(),
                    });
                }
            }
        }

        if detected_alignment != "left" {
            has_formatting = true;
        }

        (plain_text, runs, detected_alignment, has_formatting)
    }

    /// Resolve a slide's background from slide settings, layout background, or embedded media.
    fn resolve_slide_background(
        slide: &FreeShowSlide,
        layout_bg: Option<&str>,
        show: &FreeShowShow,
        media_dir: &Path,
    ) -> Option<SlideBackground> {
        // 1. Slide-level direct color
        if let Some(ref color) = slide.color {
            let trimmed = color.trim();
            if !trimmed.is_empty() && trimmed != "transparent" {
                return Some(SlideBackground::Solid(trimmed.to_string()));
            }
        }

        // 2. Slide-level solid color setting
        if let Some(ref settings) = slide.settings {
            if let Some(ref color) = settings.color {
                let trimmed = color.trim();
                if !trimmed.is_empty() && trimmed != "transparent" {
                    return Some(SlideBackground::Solid(trimmed.to_string()));
                }
            }
        }

        // 2. Slide-level background image / media reference
        if let Some(ref settings) = slide.settings {
            if let Some(ref bg_img) = settings.background_image {
                if let Some(bg) = Self::resolve_media_or_string(bg_img, show, media_dir) {
                    return Some(bg);
                }
            }
        }

        // 3. Layout slide background setting
        if let Some(bg_str) = layout_bg {
            if let Some(bg) = Self::resolve_media_or_string(bg_str, show, media_dir) {
                return Some(bg);
            }
        }

        None
    }

    /// Resolve a background string (either media dictionary ID, data URI, hex color, or path).
    fn resolve_media_or_string(
        target: &str,
        show: &FreeShowShow,
        media_dir: &Path,
    ) -> Option<SlideBackground> {
        let trimmed = target.trim();
        if trimmed.is_empty() {
            return None;
        }

        // Check if target is a solid hex color
        if trimmed.starts_with('#') {
            return Some(SlideBackground::Solid(trimmed.to_string()));
        }

        // Check if target matches a key in show.media. `media_type` (FreeShow's real
        // `Media.type: MediaType`, which includes "video") decides Image vs Video —
        // background loop videos are a common real FreeShow use case, and treating
        // one as a static Image would be silently wrong, not just imprecise.
        if let Some(media) = show.media.get(trimmed) {
            let is_video = media.media_type.as_deref() == Some("video");
            let to_background = |file_path: String| {
                if is_video {
                    SlideBackground::Video {
                        file_path,
                        loop_playback: media.loop_playback.unwrap_or(true),
                        is_muted: media.muted.unwrap_or(false),
                    }
                } else {
                    SlideBackground::Image { file_path, opacity: 1.0 }
                }
            };

            // A. Embedded base64 data
            if let Some(ref b64) = media.base64 {
                if let Some(file_path) = save_embedded_base64(b64, media_dir) {
                    return Some(to_background(file_path));
                }
            }
            // B. External/local file path referenced by FreeShow's own media dict —
            // copy it into our servable media dir if it exists locally, or use a
            // remote URL directly. Only return `None` here (not a further fallback
            // to the raw string below) once we've actually tried to resolve it —
            // this dict entry existing means we know definitively whether it's a
            // video, so don't let a resolution failure silently fall through to the
            // generic string fallback below, which can't know that.
            if let Some(ref p) = media.path {
                return resolve_external_media_path(p, media_dir).map(to_background);
            }
            return None;
        }

        // Check if target is a raw data URI
        if trimmed.starts_with("data:image/") {
            return save_embedded_base64(trimmed, media_dir)
                .map(|file_path| SlideBackground::Image { file_path, opacity: 1.0 });
        }

        // Not a hex color, not a known media-dict key, not a data URI — only trust
        // it if it's a remote URL (servable as-is) or an existing local file we can
        // copy in. Never fabricate a reference to something we can't actually
        // resolve: an unservable local path or filename that exists nowhere the web
        // server can reach is worse than no background at all.
        resolve_external_media_path(trimmed, media_dir).map(|resolved| {
            let lower = resolved.to_lowercase();
            if FREESHOW_VIDEO_EXTS.iter().any(|ext| lower.ends_with(ext)) {
                SlideBackground::Video { file_path: resolved, loop_playback: true, is_muted: false }
            } else {
                SlideBackground::Image { file_path: resolved, opacity: 1.0 }
            }
        })
    }
}

const FREESHOW_VIDEO_EXTS: &[&str] = &[".mp4", ".mov", ".webm", ".avi", ".mkv", ".m4v"];

/// Resolves a media reference that isn't already an embedded base64 blob: a remote
/// URL is returned as-is (browsers load those directly, no copy needed), an existing
/// local file is copied into `media_dir` and returned as a servable `/media/images/`
/// URL, and anything else (a path that exists nowhere, a bare filename) resolves to
/// `None` rather than a reference nothing can ever serve.
fn resolve_external_media_path(path_or_url: &str, media_dir: &Path) -> Option<String> {
    let trimmed = path_or_url.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Some(trimmed.to_string());
    }
    let new_filename = crate::storage::media_sniff::copy_sniffed_media(Path::new(trimmed), media_dir, "freeshow")?;
    Some(format!("/media/images/{}", new_filename))
}

/// Decode base64 or data-uri string and write to `media_dir/freeshow_<uuid>.<ext>`
fn save_embedded_base64(data_or_b64: &str, media_dir: &Path) -> Option<String> {
    let b64_clean = match data_or_b64.find(";base64,") {
        Some(idx) => &data_or_b64[idx + 8..],
        None => data_or_b64.trim(),
    };

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64_clean.trim().as_bytes())
        .ok()?;

    let filename = crate::storage::media_sniff::write_sniffed_media(media_dir, "freeshow", &bytes)?;
    Some(format!("/media/images/{}", filename))
}

/// Simplified short label for slide groups: "Verse 1" -> "V1", "Chorus" -> "C1"
fn shorten_group_tag(group: &str) -> String {
    let lower = group.to_lowercase();
    let num: String = group.chars().filter(|c| c.is_ascii_digit()).collect();
    let num_str = if num.is_empty() { "1".to_string() } else { num };

    if lower.contains("chorus") {
        format!("C{}", num_str)
    } else if lower.contains("bridge") {
        format!("B{}", num_str)
    } else if lower.contains("pre") {
        format!("P{}", num_str)
    } else if lower.contains("intro") {
        format!("I{}", num_str)
    } else if lower.contains("ending") {
        format!("E{}", num_str)
    } else {
        format!("V{}", num_str)
    }
}

#[derive(Default, Clone)]
struct ParsedStyle {
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strike: Option<bool>,
    color: Option<String>,
    font_family: Option<String>,
    font_size_pt: Option<f64>,
    letter_spacing_px: Option<f64>,
}

impl ParsedStyle {
    fn merge(&self, other: &ParsedStyle) -> ParsedStyle {
        ParsedStyle {
            bold: other.bold.or(self.bold),
            italic: other.italic.or(self.italic),
            underline: other.underline.or(self.underline),
            strike: other.strike.or(self.strike),
            color: other.color.clone().or_else(|| self.color.clone()),
            font_family: other.font_family.clone().or_else(|| self.font_family.clone()),
            font_size_pt: other.font_size_pt.or(self.font_size_pt),
            letter_spacing_px: other.letter_spacing_px.or(self.letter_spacing_px),
        }
    }
}

/// Parse CSS string properties into ParsedStyle.
fn parse_css_style(style_str: &str) -> ParsedStyle {
    let mut style = ParsedStyle::default();

    for declaration in style_str.split(';') {
        let decl = declaration.trim();
        if decl.is_empty() {
            continue;
        }
        let parts: Vec<&str> = decl.splitn(2, ':').collect();
        if parts.len() != 2 {
            continue;
        }
        let property = parts[0].trim().to_lowercase();
        let value = parts[1].trim();

        match property.as_str() {
            "font-weight" => {
                let v = value.to_lowercase();
                if v == "bold" || v == "bolder" {
                    style.bold = Some(true);
                } else if let Ok(weight) = v.parse::<u32>() {
                    style.bold = Some(weight >= 600);
                } else if v == "normal" {
                    style.bold = Some(false);
                }
            }
            "font-style" => {
                let v = value.to_lowercase();
                if v == "italic" || v == "oblique" {
                    style.italic = Some(true);
                } else if v == "normal" {
                    style.italic = Some(false);
                }
            }
            "text-decoration" | "text-decoration-line" => {
                let v = value.to_lowercase();
                if v.contains("underline") {
                    style.underline = Some(true);
                }
                if v.contains("line-through") {
                    style.strike = Some(true);
                }
                if v == "none" {
                    style.underline = Some(false);
                    style.strike = Some(false);
                }
            }
            "color" => {
                let clean = value.trim_matches('\'').trim_matches('"');
                if !clean.is_empty() {
                    style.color = Some(clean.to_string());
                }
            }
            "font-family" => {
                let clean = value.trim_matches('\'').trim_matches('"');
                if !clean.is_empty() {
                    style.font_family = Some(clean.to_string());
                }
            }
            "font-size" => {
                let v = value.to_lowercase();
                if let Some(pt_str) = v.strip_suffix("pt") {
                    if let Ok(pt) = pt_str.trim().parse::<f64>() {
                        style.font_size_pt = Some(pt);
                    }
                } else if let Some(px_str) = v.strip_suffix("px") {
                    if let Ok(px) = px_str.trim().parse::<f64>() {
                        style.font_size_pt = Some(px * 0.75); // 1px = 0.75pt
                    }
                } else if let Ok(num) = v.parse::<f64>() {
                    style.font_size_pt = Some(num);
                }
            }
            "letter-spacing" => {
                let v = value.to_lowercase();
                if let Some(px_str) = v.strip_suffix("px") {
                    if let Ok(px) = px_str.trim().parse::<f64>() {
                        style.letter_spacing_px = Some(px);
                    }
                }
            }
            _ => {}
        }
    }

    style
}

/// Convert string alignment to alignment string.
fn parse_text_alignment(align: &str) -> String {
    match align.trim().to_lowercase().as_str() {
        "center" => "center".to_string(),
        "right" => "right".to_string(),
        "justify" | "justified" => "justify".to_string(),
        _ => "left".to_string(),
    }
}
