use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::{Json, Router};
use serde::Deserialize;

use crate::api::ws::AppState;
use crate::core::models::{Presentation, Song};

use super::common::{require_host_token, validate_safe_path};

use crate::storage::freeshow_import;

pub(super) async fn get_bibles(State(app): State<AppState>) -> impl IntoResponse {
    let bibles = app.db.get_installed_bibles().unwrap_or_default();
    let mut out: Vec<serde_json::Value> = bibles.into_iter().filter_map(|b| serde_json::to_value(b).ok()).collect();
    // Translations a plugin offers without storing them (YouVersion's NIV...): listed like any other, with a
    // `remote` block the console reads for the tag and for how to look them up. Metadata only.
    for r in app.db.all_remote_bibles().unwrap_or_default() {
        out.push(serde_json::json!({
            "id": format!("remote:{}:{}", r.source, r.bible_id),
            "name": r.title, "abbreviation": r.abbreviation, "language": r.language,
            "is_default": false, "is_licensed": true,
            "remote": r,
        }));
    }
    Json(out)
}

#[derive(Deserialize)]
pub struct ParseRefQuery {
    q: String,
}

pub(super) async fn parse_bible_ref(Query(query): Query<ParseRefQuery>) -> impl IntoResponse {
    if let Some(parsed) = freeshow_import::parse_scripture_reference(&query.q) {
        Json(serde_json::json!({ "success": true, "parsed": parsed }))
    } else {
        Json(serde_json::json!({ "success": false }))
    }
}

#[derive(Deserialize)]
pub struct BiblePassageQuery {
    pub book: String,
    pub chapter: u32,
    pub version: String,
    pub verse_start: Option<u32>,
    pub verse_end: Option<u32>,
}

pub(super) async fn get_bible_passage(
    State(app): State<AppState>,
    Query(query): Query<BiblePassageQuery>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let version = query.version.trim();
    if version.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Missing or empty Bible version".to_string()));
    }
    let book = query.book.trim();
    if book.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Missing or empty book name".to_string()));
    }

    match app.db.get_passage(version, book, query.chapter) {
        Ok(Some(mut item)) => {
            if let Some(v_start) = query.verse_start {
                let v_end = query.verse_end.unwrap_or(v_start);
                item.verses.retain(|v| v.verse_number >= v_start && v.verse_number <= v_end);
                item.verse_start = v_start;
                item.verse_end = v_end;
                item.reference = format!("{} {}:{}-{} ({})", item.book, item.chapter, v_start, v_end, item.version);
            }
            Ok(Json(serde_json::json!({
                "success": true,
                "passage": item,
                "verses": item.verses
            })))
        }
        Ok(None) => Ok(Json(serde_json::json!({
            "success": false,
            "error": format!("Passage not found: {} {} in {}", book, query.chapter, version)
        }))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e))),
    }
}

pub(super) async fn delete_bible(
    State(app): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    require_host_token(&headers, &app).map_err(|(status, Json(body))| {
        (status, body.get("error").and_then(|v| v.as_str()).unwrap_or("Unauthorized").to_string())
    })?;
    // A translation a plugin offers (remote:<plugin>:<id>) is taken off the list; there is no text to delete.
    if let Some(rest) = id.strip_prefix("remote:") {
        let removed = rest.rsplit_once(':').and_then(|(plugin, bible)| {
            let bible: i64 = bible.parse().ok()?;
            let mut list = app.db.get_remote_bibles(plugin).ok()?;
            let before = list.len();
            list.retain(|b| b.bible_id != bible);
            (list.len() != before).then(|| app.db.set_remote_bibles(plugin, &list).is_ok())
        });
        return Ok(Json(match removed {
            Some(true) => serde_json::json!({ "success": true, "deleted": id }),
            _ => serde_json::json!({ "success": false, "error": format!("Translation '{}' not found", id) }),
        }));
    }
    match app.db.delete_installed_bible(&id) {
        Ok(true) => Ok(Json(serde_json::json!({ "success": true, "deleted": id }))),
        Ok(false) => Ok(Json(serde_json::json!({ "success": false, "error": format!("Bible translation '{}' not found", id) }))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("Database error deleting Bible: {}", e))),
    }
}

#[derive(Deserialize, Default)]
pub struct OpenLpImportReq {
    pub path: Option<String>,
    pub file_name: Option<String>,
    pub file_data_base64: Option<String>,
}

/// Import OpenLP SQLite databases (songs.sqlite or *.sqlite bibles) or auto-detect local OpenLP installation
pub(super) async fn import_openlp(
    State(app): State<AppState>,
    Json(req): Json<OpenLpImportReq>,
) -> impl IntoResponse {
    let media_dir = app.media_dir.join("images");
    // 1. If payload contains base64 data or path, process the uploaded file
    if req.file_data_base64.is_some() || req.path.is_some() {
        let (bytes, stem) = if let Some(ref b64) = req.file_data_base64 {
            use base64::Engine;
            let decoded = match base64::engine::general_purpose::STANDARD.decode(b64.trim()) {
                Ok(b) => b,
                Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "message": format!("Invalid base64: {}", e) }))),
            };
            let name_stem = req.file_name.as_deref()
                .and_then(|f| std::path::Path::new(f).file_stem().and_then(|s| s.to_str()))
                .unwrap_or("openlp_import");
            (decoded, name_stem.to_string())
        } else if let Some(ref p) = req.path {
            let val_path = match validate_safe_path(p) {
                Ok(path) => path,
                Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "message": e.1 }))),
            };
            let file_bytes = match std::fs::read(&val_path) {
                Ok(b) => b,
                Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "message": format!("Could not read path: {}", e) }))),
            };
            let name_stem = val_path.file_stem().and_then(|s| s.to_str()).unwrap_or("openlp_import").to_string();
            (file_bytes, name_stem)
        } else {
            unreachable!();
        };

        // Write bytes to temp file to query via rusqlite
        let mut tmp_file = match tempfile::NamedTempFile::new() {
            Ok(f) => f,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "success": false, "message": format!("Tempfile creation failed: {}", e) }))),
        };
        if let Err(e) = std::io::Write::write_all(&mut tmp_file, &bytes) {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "success": false, "message": format!("Failed writing bytes: {}", e) })));
        }

        let conn = match rusqlite::Connection::open(tmp_file.path()) {
            Ok(c) => c,
            Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "message": format!("Failed to open SQLite database: {}", e) }))),
        };

        // Inspect sqlite_master for songs or bible tables
        let has_songs: bool = conn.query_row(
            "SELECT count(*) > 0 FROM sqlite_master WHERE type='table' AND name='songs';",
            [],
            |r| r.get(0),
        ).unwrap_or(false);

        let has_bible: bool = conn.query_row(
            "SELECT count(*) > 0 FROM sqlite_master WHERE type='table' AND name='verse';",
            [],
            |r| r.get(0),
        ).unwrap_or(false);

        if has_songs {
            match crate::storage::OpenLPImporter::import_songs_db_with_media(tmp_file.path(), Some(&media_dir)) {
                Ok(songs) => {
                    let mut imported = 0;
                    for song in &songs {
                        if let Ok(()) = app.db.insert_song(song) {
                            imported += 1;
                        }
                    }
                    (StatusCode::OK, Json(serde_json::json!({
                        "success": true,
                        "type": "songs",
                        "count": imported,
                        "total": songs.len(),
                        "message": format!("Successfully imported {} songs from OpenLP database ({})", imported, stem)
                    })))
                }
                Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "message": format!("Error parsing OpenLP songs: {}", e) }))),
            }
        } else if has_bible {
            match crate::storage::OpenLPImporter::import_bible_db(tmp_file.path()) {
                Ok(scriptures) => {
                    if scriptures.is_empty() {
                        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "message": "No verses found in OpenLP Bible database" })));
                    }
                    let version = scriptures[0].version.clone();
                    match app.db.insert_scriptures_batch(&scriptures) {
                        Ok(()) => (StatusCode::OK, Json(serde_json::json!({
                            "success": true,
                            "type": "bibles",
                            "count": scriptures.len(),
                            "version": version,
                            "message": format!("Successfully imported Bible '{}' ({} chapters) from OpenLP database", version, scriptures.len())
                        }))),
                        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "success": false, "message": format!("Database error saving Bible: {}", e) }))),
                    }
                }
                Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "message": format!("Error parsing OpenLP Bible: {}", e) }))),
            }
        } else {
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({
                "success": false,
                "message": "Unrecognized OpenLP database format. Expected a songs database (with 'songs' table) or a Bible database (with 'verse' table)."
            })))
        }
    } else {
        // 2. No file provided — trigger auto-detection of local OpenLP installation
        match crate::storage::OpenLPImporter::detect_and_import_local(&app.db, Some(&media_dir)) {
            Ok(summary) => {
                let status = if summary.success { StatusCode::OK } else { StatusCode::NOT_FOUND };
                (status, Json(serde_json::json!(summary)))
            }
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "success": false, "message": format!("Auto-detect scan failed: {}", e) }))),
        }
    }
}

#[derive(Deserialize, Default)]
pub struct PptxImportReq {
    pub file_name: Option<String>,
    pub file_data_base64: String,
}

/// Import a PowerPoint `.pptx` file's slide text into the Presentations library.
/// Text-only fidelity (no layout/background/speaker-notes reconstruction) — see
/// `PptxImporter::import_pptx_bytes` doc comment for why.
pub(super) async fn import_pptx(
    State(app): State<AppState>,
    Json(req): Json<PptxImportReq>,
) -> impl IntoResponse {
    use base64::Engine;
    let bytes = match base64::engine::general_purpose::STANDARD.decode(req.file_data_base64.trim()) {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "message": format!("Invalid base64: {}", e) }))),
    };
    let stem = req.file_name.as_deref()
        .and_then(|f| std::path::Path::new(f).file_stem().and_then(|s| s.to_str()))
        .unwrap_or("Imported Presentation")
        .to_string();

    let media_dir = app.media_dir.join("images");
    match crate::storage::PptxImporter::import_pptx_bytes(&bytes, &stem, &media_dir) {
        Ok(pres) => match app.db.insert_presentation(&pres) {
            Ok(()) => (StatusCode::OK, Json(serde_json::json!({
                "success": true,
                "id": pres.id,
                "title": pres.title,
                "slide_count": pres.slides.len(),
                "message": format!("Successfully imported '{}' ({} slides) from PowerPoint", pres.title, pres.slides.len())
            }))),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "success": false, "message": format!("Database error saving presentation: {}", e) }))),
        },
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "message": format!("Error parsing .pptx: {}", e) }))),
    }
}

#[derive(Deserialize)]
pub struct OdpImportReq {
    pub file_name: Option<String>,
    pub file_data_base64: String,
}

/// Import an OpenDocument Presentation `.odp` file into the Presentations library, as one
/// positioned element per shape (text boxes, inline images) -- see
/// `OdpImporter::import_odp_bytes` doc comment for exactly what is and isn't reconstructed.
pub(super) async fn import_odp(
    State(app): State<AppState>,
    Json(req): Json<OdpImportReq>,
) -> impl IntoResponse {
    use base64::Engine;
    let bytes = match base64::engine::general_purpose::STANDARD.decode(req.file_data_base64.trim()) {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "message": format!("Invalid base64: {}", e) }))),
    };
    let stem = req.file_name.as_deref()
        .and_then(|f| std::path::Path::new(f).file_stem().and_then(|s| s.to_str()))
        .unwrap_or("Imported Presentation")
        .to_string();

    let media_dir = app.media_dir.join("images");
    match crate::storage::OdpImporter::import_odp_bytes(&bytes, &stem, &media_dir) {
        Ok(pres) => match app.db.insert_presentation(&pres) {
            Ok(()) => (StatusCode::OK, Json(serde_json::json!({
                "success": true,
                "id": pres.id,
                "title": pres.title,
                "slide_count": pres.slides.len(),
                "message": format!("Successfully imported '{}' ({} slides) from OpenDocument Presentation", pres.title, pres.slides.len())
            }))),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "success": false, "message": format!("Database error saving presentation: {}", e) }))),
        },
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "message": format!("Error parsing .odp: {}", e) }))),
    }
}

#[derive(Deserialize)]
pub struct EwpxImportReq {
    pub file_name: Option<String>,
    pub file_data_base64: String,
    /// A presentation theme for the imported deck; its background shows on every slide that has no picture of its own.
    #[serde(default)]
    pub theme_name: Option<String>,
}

/// The first `#rrggbb` in a CSS background (a solid colour, or the first stop of a gradient), as 0.0 (black) to 1.0 (white).
fn background_lightness(css: &str) -> Option<f64> {
    let i = css.find('#')?;
    let hex: String = css[i + 1..].chars().take_while(char::is_ascii_hexdigit).collect();
    let hex = match hex.len() {
        3 => hex.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 | 8 => hex[..6].to_string(),
        _ => return None,
    };
    let c = |k: usize| u8::from_str_radix(&hex[k..k + 2], 16).map(f64::from);
    Some((0.2126 * c(0).ok()? + 0.7152 * c(2).ok()? + 0.0722 * c(4).ok()?) / 255.0)
}

/// The share (0.0 to 1.0) of a deck's text, by characters, that is light (a run with no readable colour counts as dark), or `None`
/// if the deck has no text.
fn light_text_share(slides: &[crate::core::models::Slide]) -> Option<f64> {
    use crate::core::models::SlideElement;
    let (mut light, mut total) = (0usize, 0usize);
    for run in slides
        .iter()
        .flat_map(|s| s.elements.iter())
        .filter_map(|e| if let SlideElement::TextBlock { block, .. } = e { Some(block) } else { None })
        .flat_map(|b| b.runs.iter())
    {
        let n = run.text.chars().filter(|c| !c.is_whitespace()).count();
        total += n;
        if background_lightness(&run.color).is_some_and(|l| l > 0.5) {
            light += n;
        }
    }
    (total > 0).then(|| light as f64 / total as f64)
}

/// The theme to give a deck whose own pictures are missing. The default presentation theme is a light page, which would hide a deck
/// written with light text, so for such a deck the first dark presentation theme is used instead.
fn theme_for_deck(app: &AppState, slides: &[crate::core::models::Slide]) -> Option<String> {
    let default = app.db.get_default_theme_for_category("presentation").ok().flatten();
    let light_deck = light_text_share(slides).is_some_and(|share| share > 0.5);
    let default_is_light = default.as_ref().and_then(|t| background_lightness(&t.background)).is_none_or(|l| l > 0.5);
    if light_deck && default_is_light {
        let themes = app.db.get_themes().unwrap_or_default();
        if let Some(t) = themes.iter().find(|t| t.category == "presentation" && background_lightness(&t.background).is_some_and(|l| l < 0.35)) {
            return Some(t.name.clone());
        }
    }
    None
}

/// Clears the picture or video background of every slide whose file isn't in the library (an EasyWorship package does not carry
/// its media), so the deck's theme background shows there instead of nothing. Returns how many slides it changed.
fn clear_missing_backgrounds(slides: &mut [crate::core::models::Slide], media_dir: &std::path::Path) -> usize {
    use crate::core::models::SlideBackground;
    let videos = media_dir.parent().map(|p| p.join("videos"));
    let present = |file: &str| -> bool {
        let name = file.rsplit(['/', '\\']).next().unwrap_or(file);
        !name.is_empty() && (media_dir.join(name).is_file() || videos.as_ref().is_some_and(|v| v.join(name).is_file()) || std::path::Path::new(file).is_absolute() && std::path::Path::new(file).is_file())
    };
    let mut cleared = 0;
    for slide in slides.iter_mut() {
        let file = match &slide.background_v2 {
            Some(SlideBackground::Image { file_path, .. }) | Some(SlideBackground::Video { file_path, .. }) => Some(file_path.clone()),
            Some(_) => None,
            None => slide.background.clone().filter(|b| !b.trim().is_empty() && !b.starts_with('#') && !b.starts_with("rgb") && !b.contains("gradient")),
        };
        if file.is_some_and(|f| !present(&f)) {
            slide.background = None;
            slide.background_v2 = None;
            cleared += 1;
        }
    }
    cleared
}

/// Import an EasyWorship presentation package (`.ewpx`) into the Presentations library. A package holds no pictures, so a slide whose
/// background picture isn't in the library gets the deck's theme background (the chosen theme, or the default presentation theme).
pub(super) async fn import_ewpx(
    State(app): State<AppState>,
    Json(req): Json<EwpxImportReq>,
) -> impl IntoResponse {
    use base64::Engine;
    let fail = |code: StatusCode, msg: String| (code, Json(serde_json::json!({ "success": false, "message": msg })));
    let bytes = match base64::engine::general_purpose::STANDARD.decode(req.file_data_base64.trim()) {
        Ok(b) => b,
        Err(e) => return fail(StatusCode::BAD_REQUEST, format!("Invalid base64: {}", e)),
    };
    let stem = req.file_name.as_deref()
        .and_then(|f| std::path::Path::new(f).file_stem().and_then(|s| s.to_str()))
        .unwrap_or("Imported Presentation")
        .to_string();
    let theme = req.theme_name.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
    if let Some(t) = &theme {
        if !app.db.get_themes().map(|all| all.iter().any(|x| &x.name == t)).unwrap_or(false) {
            return fail(StatusCode::BAD_REQUEST, format!("There is no theme called '{t}'."));
        }
    }
    let media_dir = app.media_dir.join("images");
    let schedule = match crate::storage::EwsxManager::load_schedule_from_bytes_with_media_dir(&bytes, &stem, &media_dir) {
        Ok(s) => s,
        Err(e) => return fail(StatusCode::BAD_REQUEST, format!("Error reading .ewpx: {e}")),
    };
    let existing: Vec<String> = app.db.get_presentations().map(|p| p.into_iter().map(|x| x.title).collect()).unwrap_or_default();
    let (mut imported, mut slides_total, mut cleared_total) = (Vec::new(), 0usize, 0usize);
    for item in schedule.items.into_iter().filter(|i| i.item_type == "presentation" && !i.slides.is_empty()) {
        let mut slides = item.slides;
        let cleared = clear_missing_backgrounds(&mut slides, &media_dir);
        cleared_total += cleared;
        let base = if item.title.trim().is_empty() { stem.clone() } else { item.title.trim().to_string() };
        let mut title = base.clone();
        let mut n = 2;
        while existing.contains(&title) || imported.iter().any(|(t, _): &(String, String)| *t == title) {
            title = format!("{base} {n}");
            n += 1;
        }
        let deck_theme = theme.clone().or(item.theme_name).or_else(|| if cleared > 0 { theme_for_deck(&app, &slides) } else { None });
        let pres = Presentation {
            id: format!("pres_{}", uuid::Uuid::new_v4().simple()),
            title: title.clone(),
            author: item.author_or_ref,
            slides,
            theme_name: deck_theme,
        };
        slides_total += pres.slides.len();
        if let Err(e) = app.db.insert_presentation(&pres) {
            return fail(StatusCode::INTERNAL_SERVER_ERROR, format!("Database error saving presentation: {e}"));
        }
        imported.push((title, pres.id));
    }
    if imported.is_empty() {
        return fail(StatusCode::BAD_REQUEST, "That package has no presentation slides in it.".to_string());
    }
    let names = imported.iter().map(|(t, _)| format!("'{t}'")).collect::<Vec<_>>().join(", ");
    let bg = if cleared_total > 0 { format!(" {cleared_total} slide(s) pointed at a picture that is not in this library, so they use the theme background instead.") } else { String::new() };
    (StatusCode::OK, Json(serde_json::json!({
        "success": true,
        "id": imported[0].1,
        "title": imported[0].0,
        "slide_count": slides_total,
        "backgrounds_replaced": cleared_total,
        "message": format!("Imported {names} ({slides_total} slides) from EasyWorship.{bg}")
    })))
}

#[derive(Deserialize, Default)]
pub struct FreeShowShowImportReq {
    pub file_data_base64: String,
}

/// Import a FreeShow `.show` or JSON file into the library (Songs or Presentations).
pub(super) async fn import_freeshow_show(
    State(app): State<AppState>,
    Json(req): Json<FreeShowShowImportReq>,
) -> impl IntoResponse {
    use base64::Engine;
    let bytes = match base64::engine::general_purpose::STANDARD.decode(req.file_data_base64.trim()) {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "message": format!("Invalid base64: {}", e) }))),
    };

    let media_dir = app.media_dir.join("images");
    match crate::storage::FreeShowShowImporter::import_show_bytes(&bytes, &media_dir) {
        Ok(sched) => {
            if let Some(item) = sched.items.first() {
                let is_song = item.item_type == "song";
                let slide_count = item.slides.len();
                let title = item.title.clone();
                let author = item.subtitle.clone().unwrap_or_else(|| "Unknown".to_string());

                if is_song {
                    let mut song = Song::new(&title, &author);
                    song.id = format!("song_{}", uuid::Uuid::new_v4());
                    song.slides = item.slides.clone();
                    if let Some(ref notes) = item.notes {
                        if let Some(ccli_str) = notes.strip_prefix("CCLI: ") {
                            song.ccli_number = Some(ccli_str.trim().to_string());
                        }
                    }

                    match app.db.insert_song(&song) {
                        Ok(()) => (StatusCode::OK, Json(serde_json::json!({
                            "success": true,
                            "id": song.id,
                            "title": song.title,
                            "type": "song",
                            "slide_count": slide_count,
                            "message": format!("Successfully imported song '{}' ({} slides) from FreeShow", song.title, slide_count)
                        }))),
                        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "success": false, "message": format!("Database error saving song: {}", e) }))),
                    }
                } else {
                    let pres = Presentation {
                        id: format!("pres_{}", uuid::Uuid::new_v4()),
                        title: title.clone(),
                        author: author.clone(),
                        slides: item.slides.clone(),
                        theme_name: None,
                    };

                    match app.db.insert_presentation(&pres) {
                        Ok(()) => (StatusCode::OK, Json(serde_json::json!({
                            "success": true,
                            "id": pres.id,
                            "title": pres.title,
                            "type": "presentation",
                            "slide_count": slide_count,
                            "message": format!("Successfully imported presentation '{}' ({} slides) from FreeShow", pres.title, slide_count)
                        }))),
                        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "success": false, "message": format!("Database error saving presentation: {}", e) }))),
                    }
                }
            } else {
                (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "message": "No slides found in FreeShow file" })))
            }
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "message": format!("Error parsing FreeShow show: {}", e) }))),
    }
}

/// Parses and installs a Bible, off the async runtime. 409 (with `code: "already_installed"`) when one with that
/// abbreviation is there and `replace` wasn't asked for, so the console can ask before replacing it.
async fn install_bible_bytes(
    app: &AppState,
    bytes: Vec<u8>,
    file_name: String,
    abbreviation: Option<String>,
    replace: bool,
    source: String,
) -> impl IntoResponse {
    use crate::storage::bible_import;
    let db = app.db.clone();
    let result = tokio::task::spawn_blocking(move || {
        let parsed = bible_import::parse_bible(&bytes, &file_name)?;
        let abbr = abbreviation.as_deref().map(bible_import::clean_abbreviation).filter(|a| !a.is_empty()).unwrap_or_else(|| parsed.abbreviation.clone());
        if !replace && bible_import::is_installed(&db, &abbr).unwrap_or(false) {
            return Ok::<_, bible_import::BibleImportError>(Err((abbr, parsed)));
        }
        bible_import::install(&db, &parsed, &abbr, replace, &source).map(Ok)
    })
    .await;
    match result {
        Ok(Ok(Ok(installed))) => (StatusCode::OK, Json(serde_json::json!({ "success": true, "bible": installed }))),
        Ok(Ok(Err((abbr, parsed)))) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "success": false, "code": "already_installed", "abbreviation": abbr,
                "error": format!("{abbr} is already installed"),
                "bible": crate::storage::bible_import::describe(&parsed, &abbr),
            })),
        ),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "error": e.0 }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "success": false, "error": format!("the import failed: {e}") }))),
    }
}

#[derive(Deserialize)]
pub struct ImportBibleLinkReq {
    pub url: String,
    #[serde(default)]
    pub abbreviation: Option<String>,
    #[serde(default)]
    pub replace: bool,
}

/// POST /api/bibles/import/link -- the operator pastes a link to a Bible file (USFM, USX, OSIS, Zefania or OpenSong, loose or in a
/// zip); the server downloads it (public https only, size-capped, see `plugin_net::check_public_url`), reads it and
/// installs it. Console only: it makes the server fetch an address a person typed.
pub(super) async fn import_bible_link(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ImportBibleLinkReq>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    require_host_token(&headers, &app)?;
    let url = req.url.trim().to_string();
    let fetched = crate::api::plugin_net::fetch_public_bytes(&url, crate::storage::bible_import::MAX_INPUT_BYTES)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "error": e }))))?;
    if fetched.status != 200 {
        return Err((StatusCode::BAD_REQUEST, Json(serde_json::json!({ "success": false, "error": format!("the site answered {}", fetched.status) }))));
    }
    let file_name = reqwest::Url::parse(&url).ok().and_then(|u| u.path_segments().and_then(|s| s.last().map(str::to_string))).unwrap_or_default();
    Ok(install_bible_bytes(&app, fetched.body, file_name, req.abbreviation, req.replace, url).await)
}

#[derive(Deserialize)]
pub struct ImportBibleFileQuery {
    pub name: Option<String>,
    pub abbreviation: Option<String>,
    #[serde(default)]
    pub replace: bool,
}

/// POST /api/bibles/import/file?name=&abbreviation=&replace= -- the same importer for a file the operator has
/// (the raw bytes are the body).
pub(super) async fn import_bible_file(
    State(app): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ImportBibleFileQuery>,
    body: axum::body::Bytes,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    require_host_token(&headers, &app)?;
    let name = q.name.unwrap_or_default();
    Ok(install_bible_bytes(&app, body.to_vec(), name.clone(), q.abbreviation, q.replace, format!("file {name}")).await)
}

pub(super) fn router() -> Router<AppState> {
    use axum::routing::{get, post};
    Router::new()
        .route("/api/bibles", get(get_bibles))
        .route("/api/bibles/{id}", axum::routing::delete(delete_bible))
        .route("/api/bibles/passage", get(get_bible_passage))
        .route("/api/bibles/parse-ref", get(parse_bible_ref))
        .route("/api/bibles/import/link", post(import_bible_link))
        .route(
            "/api/bibles/import/file",
            post(import_bible_file).layer(axum::extract::DefaultBodyLimit::max(crate::storage::bible_import::MAX_INPUT_BYTES + 1024 * 1024)),
        )
        .route("/api/import/openlp", post(import_openlp))
        .route("/api/import/pptx", post(import_pptx))
        .route("/api/import/odp", post(import_odp))
        .route("/api/import/ewpx", post(import_ewpx))
        .route("/api/import/freeshow-show", post(import_freeshow_show))
}

#[cfg(test)]
mod ewpx_tests {
    use super::*;

    #[test]
    fn lightness_reads_hex_colours_and_the_first_gradient_stop() {
        assert!(background_lightness("#ffffff").unwrap() > 0.999);
        assert_eq!(background_lightness("#000"), Some(0.0));
        assert!(background_lightness("#ffff99").unwrap() > 0.5 && background_lightness("#ff0000").unwrap() < 0.5);
        assert!(background_lightness("linear-gradient(135deg, #0f2027, #203a43)").unwrap() < 0.2);
        assert_eq!(background_lightness("red"), None);
        assert_eq!(background_lightness("#12"), None);
    }

    #[test]
    fn a_missing_picture_is_cleared_and_a_present_one_or_a_colour_is_kept() {
        use crate::core::models::{Slide, SlideBackground};
        let dir = tempfile::tempdir().unwrap();
        let images = dir.path().join("images");
        std::fs::create_dir_all(&images).unwrap();
        std::fs::write(images.join("here.jpg"), b"x").unwrap();
        let img = |name: &str| Slide { background: Some(name.into()), background_v2: Some(SlideBackground::Image { file_path: name.into(), opacity: 1.0 }), ..Default::default() };
        let mut slides = vec![
            img("gone.jpg"),
            img("/media/images/here.jpg"),
            img("C:\\Users\\x\\Pictures\\gone.jpg"),
            Slide { background: Some("#112233".into()), background_v2: Some(SlideBackground::Solid("#112233".into())), ..Default::default() },
            Slide::default(),
        ];
        assert_eq!(clear_missing_backgrounds(&mut slides, &images), 2);
        assert!(slides[0].background.is_none() && slides[0].background_v2.is_none());
        assert!(slides[1].background.is_some());
        assert!(slides[2].background.is_none());
        assert_eq!(slides[3].background.as_deref(), Some("#112233"));
    }
}
