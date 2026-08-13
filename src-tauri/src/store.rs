#[cfg(test)]
mod tests {
    use super::*;

    fn mem_store() -> HistoryStore {
        HistoryStore::open(std::path::Path::new(":memory:")).unwrap()
    }

    #[test]
    fn capture_inserts_and_lists_newest_first() {
        let store = mem_store();
        store.capture(NewItem { kind: "text".into(), content: Some("a".into()), image_path: None, preview: "a".into(), dedup_source: "text:a".into() }).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        store.capture(NewItem { kind: "text".into(), content: Some("b".into()), image_path: None, preview: "b".into(), dedup_source: "text:b".into() }).unwrap();
        let history = store.get_history().unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].content.as_deref(), Some("b"));
    }

    #[test]
    fn repeat_copy_bumps_instead_of_duplicating() {
        let store = mem_store();
        let id1 = store.capture(NewItem { kind: "text".into(), content: Some("a".into()), image_path: None, preview: "a".into(), dedup_source: "text:a".into() }).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        store.capture(NewItem { kind: "text".into(), content: Some("b".into()), image_path: None, preview: "b".into(), dedup_source: "text:b".into() }).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let id2 = store.capture(NewItem { kind: "text".into(), content: Some("a".into()), image_path: None, preview: "a".into(), dedup_source: "text:a".into() }).unwrap();
        assert_eq!(id1, id2, "same dedup_source must reuse the row");
        let history = store.get_history().unwrap();
        assert_eq!(history.len(), 2, "no duplicate row created");
        assert_eq!(history[0].content.as_deref(), Some("a"), "repeated item bumped to top");
    }

    #[test]
    fn delete_item_removes_row_and_returns_image_path() {
        let store = mem_store();
        let id = store.capture(NewItem { kind: "image".into(), content: None, image_path: Some("img/1.png".into()), preview: "Image".into(), dedup_source: "image:hash1".into() }).unwrap();
        let returned_path = store.delete_item(id).unwrap();
        assert_eq!(returned_path.as_deref(), Some("img/1.png"));
        assert_eq!(store.get_history().unwrap().len(), 0);
    }

    #[test]
    fn prune_respects_max_items_setting() {
        let store = mem_store();
        store.set_setting("max_items", "2").unwrap();
        for i in 0..3 {
            store.capture(NewItem { kind: "text".into(), content: Some(i.to_string()), image_path: None, preview: i.to_string(), dedup_source: format!("text:{i}") }).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let history = store.get_history().unwrap();
        assert_eq!(history.len(), 2, "oldest item pruned once over max_items");
        assert_eq!(history[0].content.as_deref(), Some("2"));
        assert_eq!(history[1].content.as_deref(), Some("1"));
    }

    #[test]
    fn settings_round_trip() {
        let store = mem_store();
        // `open()` seeds defaults, so overwrite retention_days explicitly and
        // check that round-trips.
        store.set_setting("retention_days", "30").unwrap();
        assert_eq!(store.get_setting("retention_days").unwrap(), Some("30".into()));
    }

    #[test]
    fn open_seeds_default_settings_on_a_fresh_db() {
        let store = mem_store();
        assert_eq!(store.get_setting("max_items").unwrap(), Some("200".into()));
        assert_eq!(store.get_setting("retention_days").unwrap(), Some("30".into()));
    }

    #[test]
    fn open_does_not_overwrite_existing_settings() {
        // Re-opening an already-configured DB must not clobber the user's
        // chosen values back to the defaults.
        let dir = std::env::temp_dir().join(format!("cm-settings-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("history.db");
        {
            let store = HistoryStore::open(&db_path).unwrap();
            store.set_setting("max_items", "5").unwrap();
        }
        let store = HistoryStore::open(&db_path).unwrap();
        assert_eq!(store.get_setting("max_items").unwrap(), Some("5".into()), "reopening must not reset a configured value to the default");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn prune_deletes_image_files_for_evicted_rows() {
        let store = mem_store();
        store.set_setting("max_items", "1").unwrap();
        let dir = std::env::temp_dir().join(format!("cm-prune-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path1 = dir.join("a.png");
        std::fs::write(&path1, b"a").unwrap();
        let path2 = dir.join("b.png");
        std::fs::write(&path2, b"b").unwrap();

        store
            .capture(NewItem { kind: "image".into(), content: None, image_path: Some(path1.to_string_lossy().into()), preview: "a".into(), dedup_source: "image:a".into() })
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        store
            .capture(NewItem { kind: "image".into(), content: None, image_path: Some(path2.to_string_lossy().into()), preview: "b".into(), dedup_source: "image:b".into() })
            .unwrap();

        assert!(!path1.exists(), "evicted row's image file must be deleted by prune, not left as an orphan");
        assert!(path2.exists(), "surviving row's image file must not be deleted");
        std::fs::remove_dir_all(&dir).ok();
    }
}

use rusqlite::{params, Connection};

pub struct HistoryItem {
    pub id: i64,
    pub kind: String,
    pub content: Option<String>,
    pub image_path: Option<String>,
    pub preview: String,
    pub created_at: i64,
}

pub struct NewItem {
    pub kind: String,
    pub content: Option<String>,
    pub image_path: Option<String>,
    pub preview: String,
    pub dedup_source: String,
}

pub struct HistoryStore {
    conn: Connection,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn dedup_key(source: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(source.as_bytes());
    format!("{:x}", hasher.finalize())
}

impl HistoryStore {
    pub fn open(path: &std::path::Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS items (
                id INTEGER PRIMARY KEY,
                kind TEXT NOT NULL,
                content TEXT,
                image_path TEXT,
                preview TEXT NOT NULL,
                dedup_key TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_items_dedup ON items(dedup_key);
            CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )?;
        let store = Self { conn };
        store.seed_default_settings()?;
        Ok(store)
    }

    /// Seeds sensible defaults on a fresh install so pruning isn't a no-op
    /// until the user visits Settings. Only fills in values that are not
    /// already set, so an existing configured DB is never clobbered on
    /// every app start.
    fn seed_default_settings(&self) -> rusqlite::Result<()> {
        if self.get_setting("max_items")?.is_none() {
            self.set_setting("max_items", "200")?;
        }
        if self.get_setting("retention_days")?.is_none() {
            self.set_setting("retention_days", "30")?;
        }
        Ok(())
    }

    pub fn capture(&self, item: NewItem) -> rusqlite::Result<i64> {
        let key = dedup_key(&item.dedup_source);
        let existing: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM items WHERE dedup_key = ?1",
                params![key],
                |row| row.get(0),
            )
            .ok();

        let id = if let Some(id) = existing {
            self.conn.execute(
                "UPDATE items SET created_at = ?1 WHERE id = ?2",
                params![now_ms(), id],
            )?;
            id
        } else {
            self.conn.execute(
                "INSERT INTO items (kind, content, image_path, preview, dedup_key, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![item.kind, item.content, item.image_path, item.preview, key, now_ms()],
            )?;
            self.conn.last_insert_rowid()
        };

        self.prune()?;
        Ok(id)
    }

    pub fn get_history(&self) -> rusqlite::Result<Vec<HistoryItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, content, image_path, preview, created_at
             FROM items ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(HistoryItem {
                id: row.get(0)?,
                kind: row.get(1)?,
                content: row.get(2)?,
                image_path: row.get(3)?,
                preview: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;
        rows.collect()
    }

    pub fn delete_item(&self, id: i64) -> rusqlite::Result<Option<String>> {
        let image_path: Option<String> = self
            .conn
            .query_row("SELECT image_path FROM items WHERE id = ?1", params![id], |row| row.get(0))
            .ok()
            .flatten();
        self.conn.execute("DELETE FROM items WHERE id = ?1", params![id])?;
        Ok(image_path)
    }

    pub fn get_setting(&self, key: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row("SELECT value FROM settings WHERE key = ?1", params![key], |row| row.get(0))
            .ok()
            .map_or(Ok(None), |v| Ok(Some(v)))
    }

    pub fn set_setting(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Deletes rows past `max_items`/`retention_days`, and also removes the
    /// on-disk image file for any evicted row that had one -- otherwise
    /// pruned/duplicate images would orphan files under the app data dir
    /// forever, since SQLite deletion alone never touches the filesystem.
    fn prune(&self) -> rusqlite::Result<()> {
        let mut evicted_image_paths: Vec<String> = Vec::new();

        if let Some(max) = self.get_setting("max_items")?.and_then(|v| v.parse::<i64>().ok()) {
            evicted_image_paths.extend(self.image_paths_outside_limit(max)?);
            self.conn.execute(
                "DELETE FROM items WHERE id NOT IN (
                    SELECT id FROM items ORDER BY created_at DESC LIMIT ?1
                )",
                params![max],
            )?;
        }
        if let Some(days) = self.get_setting("retention_days")?.and_then(|v| v.parse::<i64>().ok()) {
            let cutoff = now_ms() - days * 24 * 60 * 60 * 1000;
            evicted_image_paths.extend(self.image_paths_before(cutoff)?);
            self.conn.execute("DELETE FROM items WHERE created_at < ?1", params![cutoff])?;
        }

        for path in evicted_image_paths {
            let _ = std::fs::remove_file(path);
        }
        Ok(())
    }

    fn image_paths_outside_limit(&self, max: i64) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT image_path FROM items WHERE image_path IS NOT NULL AND id NOT IN (
                SELECT id FROM items ORDER BY created_at DESC LIMIT ?1
            )",
        )?;
        let rows = stmt.query_map(params![max], |row| row.get::<_, String>(0))?;
        rows.collect()
    }

    fn image_paths_before(&self, cutoff: i64) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT image_path FROM items WHERE image_path IS NOT NULL AND created_at < ?1")?;
        let rows = stmt.query_map(params![cutoff], |row| row.get::<_, String>(0))?;
        rows.collect()
    }
}
