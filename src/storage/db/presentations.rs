use rusqlite::{params, Result};

use crate::core::models::Presentation;

use super::{load_and_synthesize_slides, serialize_slides_for_storage, Database};

impl Database {
    // --- Presentation CRUD ---
    pub fn get_presentation_by_id(&self, id: &str) -> Result<Option<Presentation>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached("SELECT id, title, author, slides_json, theme_name FROM presentations WHERE id = ?1")?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            let slides_json: String = row.get(3)?;
            let slides = load_and_synthesize_slides(&slides_json);
            Ok(Some(Presentation {
                id: row.get(0)?,
                title: row.get(1)?,
                author: row.get(2)?,
                slides,
                theme_name: row.get(4)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn get_presentations(&self) -> Result<Vec<Presentation>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached("SELECT id, title, author, slides_json, theme_name FROM presentations ORDER BY title ASC")?;
        let pres_iter = stmt.query_map([], |row| {
            let slides_json: String = row.get(3)?;
            let slides = load_and_synthesize_slides(&slides_json);
            Ok(Presentation {
                id: row.get(0)?,
                title: row.get(1)?,
                author: row.get(2)?,
                slides,
                theme_name: row.get(4)?,
            })
        })?;

        let mut items = Vec::new();
        for p in pres_iter {
            items.push(p?);
        }
        Ok(items)
    }

    pub fn insert_presentation(&self, pres: &Presentation) -> Result<()> {
        let conn = self.conn();
        let slides_json = serialize_slides_for_storage(&pres.slides);
        conn.execute(
            "INSERT OR REPLACE INTO presentations (id, title, author, slides_json, theme_name)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![pres.id, pres.title, pres.author, slides_json, pres.theme_name],
        )?;

        Ok(())
    }

    pub fn delete_presentation(&self, id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM presentations WHERE id = ?1", params![id])?;

        Ok(())
    }
}
