use rusqlite::{params, Result};

use super::Database;

impl Database {
    // --- Slide Template CRUD ---
    pub fn get_slide_templates(&self) -> Result<Vec<crate::core::models::SlideTemplate>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT id, name, category, elements_json, background_json, thumbnail_data_url FROM slide_templates ORDER BY name ASC"
        )?;
        let iter = stmt.query_map([], |row| {
            let elements_json: String = row.get(3)?;
            let background_json: Option<String> = row.get(4)?;
            let elements = serde_json::from_str(&elements_json).unwrap_or_default();
            let background = background_json.and_then(|j| serde_json::from_str(&j).ok());
            Ok(crate::core::models::SlideTemplate {
                id: row.get(0)?,
                name: row.get(1)?,
                category: row.get(2)?,
                elements,
                background,
                thumbnail_data_url: row.get(5)?,
            })
        })?;

        let mut items = Vec::new();
        for t in iter {
            items.push(t?);
        }
        Ok(items)
    }

    pub fn insert_slide_template(&self, template: &crate::core::models::SlideTemplate) -> Result<()> {
        let conn = self.conn();
        let elements_json = serde_json::to_string(&template.elements).unwrap_or_else(|_| "[]".to_string());
        let background_json = template.background.as_ref().map(|b| serde_json::to_string(b).unwrap_or_default());
        conn.execute(
            "INSERT OR REPLACE INTO slide_templates (id, name, category, elements_json, background_json, thumbnail_data_url)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![template.id, template.name, template.category, elements_json, background_json, template.thumbnail_data_url],
        )?;

        Ok(())
    }

    /// Seeds the 4 EasyWorship-style master slide layouts on first run only (no-op
    /// once any slide template exists) — these structure a Presentation item's slides;
    /// the active Presentation-category `Theme` supplies the look on top of whichever
    /// one the operator applies via the editor's existing "Apply Template" dropdown.
    pub fn seed_master_layouts_if_empty(&self) -> Result<()> {
        use crate::core::models::{
            ElementEffects, ElementTransform, SlideElement, SlideTemplate, TextBlock,
            TextParagraphStyle, TextRun,
        };

        if !self.get_slide_templates()?.is_empty() {
            return Ok(());
        }

        fn text_element(x: f64, y: f64, w: f64, h: f64, text: &str, align: &str, font_size_pt: f64) -> SlideElement {
            SlideElement::TextBlock {
                id: format!("el_{}", uuid::Uuid::new_v4()),
                transform: ElementTransform { x, y, w, h, rotation_deg: 0.0, z_index: 0, locked: false, opacity: 1.0 },
                block: TextBlock {
                    runs: vec![TextRun { text: text.to_string(), font_size_pt, ..Default::default() }],
                    paragraph_style: TextParagraphStyle { align: align.to_string(), ..Default::default() },
                    effects: ElementEffects::default(),
                    autofit: true,
                },
            }
        }

        let layouts: Vec<(&str, Vec<SlideElement>)> = vec![
            ("Title Slide", vec![text_element(0.1, 0.35, 0.8, 0.3, "Title", "center", 54.0)]),
            (
                "Title & Content",
                vec![
                    text_element(0.06, 0.06, 0.88, 0.18, "Title", "left", 40.0),
                    text_element(0.06, 0.28, 0.88, 0.64, "Content", "left", 28.0),
                ],
            ),
            ("Content Only", vec![text_element(0.06, 0.08, 0.88, 0.84, "Content", "left", 28.0)]),
            ("Blank", vec![]),
        ];

        for (name, elements) in layouts {
            self.insert_slide_template(&SlideTemplate {
                id: format!("master_layout_{}", name.to_lowercase().replace(' ', "_").replace('&', "and")),
                name: format!("Master: {}", name),
                category: Some("Master Layout".to_string()),
                elements,
                background: None,
                thumbnail_data_url: None,
            })?;
        }

        Ok(())
    }

    pub fn delete_slide_template(&self, id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM slide_templates WHERE id = ?1", params![id])?;

        Ok(())
    }
}
