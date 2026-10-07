use rusqlite::{params, Result};

use crate::core::models::Theme;

use super::Database;

impl Database {
    // --- Themes CRUD (Resilient to legacy & modern schema) ---
    pub fn get_theme_by_name(&self, name: &str) -> Result<Option<Theme>> {
        let themes = self.get_themes()?;
        Ok(themes.into_iter().find(|t| t.name.eq_ignore_ascii_case(name)))
    }

    pub fn get_themes(&self) -> Result<Vec<Theme>> {
        let conn = self.conn();

        let mut stmt = conn.prepare_cached(
            "SELECT name, background, font_family, font_size, font_color, alignment,
                    text_shadow, line_height, letter_spacing, opacity,
                    margin_top, margin_bottom, margin_left, margin_right, vertical_align,
                    category, is_default, reference_position, chroma_key_enabled,
                    chroma_key_color, safe_area_percent
             FROM themes ORDER BY name ASC"
        )?;
        let theme_iter = stmt.query_map([], |row| {
            Ok(Theme {
                id: None,
                name: row.get(0)?,
                background: row.get(1)?,
                font_family: row.get(2)?,
                font_size: row.get(3)?,
                font_color: row.get(4)?,
                alignment: row.get(5)?,
                text_shadow: row.get(6)?,
                line_height: row.get(7)?,
                letter_spacing: row.get(8)?,
                opacity: row.get(9)?,
                margin_top: row.get(10)?,
                margin_bottom: row.get(11)?,
                margin_left: row.get(12)?,
                margin_right: row.get(13)?,
                vertical_align: row.get(14)?,
                category: row.get(15)?,
                is_default: row.get::<_, i64>(16)? != 0,
                reference_position: row.get(17)?,
                chroma_key_enabled: row.get::<_, i64>(18)? != 0,
                chroma_key_color: row.get(19)?,
                safe_area_percent: row.get::<_, i64>(20)? as u8,
            })
        })?;

        let mut themes = Vec::new();
        for t in theme_iter {
            themes.push(t?);
        }
        Ok(themes)
    }

    pub fn insert_theme(&self, theme: &Theme) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT OR REPLACE INTO themes (
                name, background, font_family, font_size, font_color, alignment,
                text_shadow, line_height, letter_spacing, opacity,
                margin_top, margin_bottom, margin_left, margin_right, vertical_align,
                category, is_default, reference_position, chroma_key_enabled,
                chroma_key_color, safe_area_percent
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
            params![
                theme.name, theme.background, theme.font_family, theme.font_size, theme.font_color, theme.alignment,
                theme.text_shadow, theme.line_height, theme.letter_spacing, theme.opacity,
                theme.margin_top, theme.margin_bottom, theme.margin_left, theme.margin_right, theme.vertical_align,
                theme.category, theme.is_default as i64, theme.reference_position, theme.chroma_key_enabled as i64,
                theme.chroma_key_color, theme.safe_area_percent as i64
            ],
        )?;

        Ok(())
    }

    pub fn delete_theme(&self, name: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM themes WHERE name = ?1", params![name])?;

        Ok(())
    }

    /// Clears `is_default` for every theme in `category`, then sets it on `name` —
    /// enforces exactly one default theme per category. Newly added songs/scriptures/
    /// presentations auto-apply their category's default (see `AddScheduleItem`-style
    /// commands in engine.rs).
    pub fn set_default_theme(&self, category: &str, name: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("UPDATE themes SET is_default = 0 WHERE category = ?1", params![category])?;
        conn.execute(
            "UPDATE themes SET is_default = 1 WHERE category = ?1 AND name = ?2",
            params![category, name],
        )?;
        Ok(())
    }

    /// Looks up the one `is_default` theme for a category (song/scripture/presentation),
    /// used to auto-apply a look when a schedule item is added without an explicit
    /// theme — see `apply_default_theme_if_unset` in src/api/routes.rs.
    pub fn get_default_theme_for_category(&self, category: &str) -> Result<Option<Theme>> {
        Ok(self.get_themes()?.into_iter().find(|t| t.category == category && t.is_default))
    }

    /// Seeds a starter theme catalog on first run only (no-op once any theme exists),
    /// so a fresh install has usable, editable Song/Scripture/Presentation themes
    /// instead of relying on the frontend's display-only DEFAULT_THEMES fallback.
    pub fn seed_default_themes_if_empty(&self) -> Result<()> {
        if !self.get_themes()?.is_empty() {
            return Ok(());
        }

        let song_gradients: &[(&str, &str, &str)] = &[
            ("Midnight Ocean", "linear-gradient(135deg, #0f2027, #203a43, #2c5364)", "Segoe UI"),
            ("Golden Sanctuary", "linear-gradient(135deg, #1f1c18, #473e34, #1f1c18)", "Georgia"),
            ("Deep Worship", "linear-gradient(135deg, #200122, #6f0000)", "Segoe UI"),
            ("Celestial Blue", "linear-gradient(135deg, #000428, #004e92)", "Arial"),
            ("Forest Grace", "linear-gradient(135deg, #134e5e, #71b280)", "Verdana"),
            ("Solid Dark", "#1a1a1a", "Segoe UI"),
            ("Ember Fire", "linear-gradient(135deg, #1c100e, #36150b, #1c100e)", "Segoe UI"),
        ];
        for (i, (name, bg, font)) in song_gradients.iter().enumerate() {
            self.insert_theme(&Theme {
                id: None,
                name: name.to_string(),
                background: bg.to_string(),
                font_family: font.to_string(),
                category: "song".to_string(),
                is_default: i == 0,
                ..Default::default()
            })?;
        }

        self.insert_theme(&Theme {
            id: None,
            name: "Scripture Classic".to_string(),
            background: "linear-gradient(135deg, #1a1a2e, #16213e)".to_string(),
            font_family: "Georgia".to_string(),
            category: "scripture".to_string(),
            is_default: true,
            reference_position: "top".to_string(),
            ..Default::default()
        })?;
        self.insert_theme(&Theme {
            id: None,
            name: "Scripture Inline".to_string(),
            background: "linear-gradient(135deg, #232526, #414345)".to_string(),
            font_family: "Georgia".to_string(),
            category: "scripture".to_string(),
            is_default: false,
            reference_position: "inline".to_string(),
            ..Default::default()
        })?;

        self.insert_theme(&Theme {
            id: None,
            name: "Presentation Light".to_string(),
            background: "#ffffff".to_string(),
            font_family: "Inter, sans-serif".to_string(),
            font_color: "#1a1a1a".to_string(),
            category: "presentation".to_string(),
            is_default: true,
            ..Default::default()
        })?;
        self.insert_theme(&Theme {
            id: None,
            name: "Presentation Dark".to_string(),
            background: "#0f172a".to_string(),
            font_family: "Inter, sans-serif".to_string(),
            category: "presentation".to_string(),
            is_default: false,
            ..Default::default()
        })?;

        Ok(())
    }
}
