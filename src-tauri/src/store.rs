#[cfg(test)]
mod tests {
    use super::*;

    fn mem_store() -> HistoryStore {
        HistoryStore::open(std::path::Path::new(":memory:")).unwrap()
    }

    fn text_item(content: &str) -> NewItem {
        NewItem {
            kind: "text".into(),
            content: Some(content.into()),
            content_alt: None,
            image_path: None,
            thumb_path: None,
            preview: content.into(),
            dedup_source: format!("text:{content}"),
        }
    }

    #[test]
    fn capture_inserts_and_lists_newest_first() {
        let store = mem_store();
        store.capture(text_item("a")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        store.capture(text_item("b")).unwrap();
        let history = store.get_history().unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].content.as_deref(), Some("b"));
    }

    #[test]
    fn repeat_copy_bumps_instead_of_duplicating() {
        let store = mem_store();
        let id1 = store.capture(text_item("a")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        store.capture(text_item("b")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let id2 = store.capture(text_item("a")).unwrap();
        assert_eq!(id1, id2, "same dedup_source must reuse the row");
        let history = store.get_history().unwrap();
        assert_eq!(history.len(), 2, "no duplicate row created");
        assert_eq!(history[0].content.as_deref(), Some("a"), "repeated item bumped to top");
    }

    #[test]
    fn richtext_dedup_bump_refreshes_stale_content() {
        // richtext's dedup key is derived only from the plain-text alt, not
        // the HTML content, so two captures with different HTML but the same
        // alt collide on the same dedup key. A bump must still refresh the
        // row's content -- otherwise selecting the row later pastes stale
        // formatting instead of what was just copied.
        let store = mem_store();
        let id1 = store
            .capture(NewItem {
                kind: "richtext".into(),
                content: Some("<a>v1</a>".into()),
                content_alt: Some("v1".into()),
                image_path: None,
                thumb_path: None,
                preview: "v1".into(),
                dedup_source: "richtext:v1".into(),
            })
            .unwrap();

        std::thread::sleep(std::time::Duration::from_millis(2));
        let id2 = store
            .capture(NewItem {
                kind: "richtext".into(),
                content: Some("<b>v1</b>".into()),
                content_alt: Some("v1".into()),
                image_path: None,
                thumb_path: None,
                preview: "v1".into(),
                dedup_source: "richtext:v1".into(),
            })
            .unwrap();

        assert_eq!(id1, id2, "same dedup_source must reuse the row");
        let history = store.get_history().unwrap();
        assert_eq!(history.len(), 1, "no duplicate row created");
        assert_eq!(
            history[0].content.as_deref(),
            Some("<b>v1</b>"),
            "bump must refresh content, not leave the stale first-copy HTML"
        );
    }

    #[test]
    fn capture_round_trips_content_alt_and_thumb_path() {
        let store = mem_store();
        store
            .capture(NewItem {
                kind: "richtext".into(),
                content: Some("<b>hi</b>".into()),
                content_alt: Some("hi".into()),
                image_path: None,
                thumb_path: None,
                preview: "hi".into(),
                dedup_source: "richtext:hi".into(),
            })
            .unwrap();
        let history = store.get_history().unwrap();
        assert_eq!(history[0].content.as_deref(), Some("<b>hi</b>"));
        assert_eq!(history[0].content_alt.as_deref(), Some("hi"));

        std::thread::sleep(std::time::Duration::from_millis(2));
        store
            .capture(NewItem {
                kind: "image".into(),
                content: None,
                content_alt: None,
                image_path: Some("img/1.png".into()),
                thumb_path: Some("img/1_thumb.png".into()),
                preview: "Image".into(),
                dedup_source: "image:hash1".into(),
            })
            .unwrap();
        let history = store.get_history().unwrap();
        assert_eq!(history[0].thumb_path.as_deref(), Some("img/1_thumb.png"));
    }

    #[test]
    fn delete_item_removes_row_and_returns_leftover_paths() {
        let store = mem_store();
        let id = store
            .capture(NewItem {
                kind: "image".into(),
                content: None,
                content_alt: None,
                image_path: Some("img/1.png".into()),
                thumb_path: Some("img/1_thumb.png".into()),
                preview: "Image".into(),
                dedup_source: "image:hash1".into(),
            })
            .unwrap();
        let mut returned_paths = store.delete_item(id).unwrap();
        returned_paths.sort();
        assert_eq!(returned_paths, vec!["img/1.png".to_string(), "img/1_thumb.png".to_string()]);
        assert_eq!(store.get_history().unwrap().len(), 0);
    }

    #[test]
    fn prune_respects_max_items_setting() {
        let store = mem_store();
        store.set_setting("max_items", "2").unwrap();
        for i in 0..3 {
            store.capture(text_item(&i.to_string())).unwrap();
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
    fn opening_a_pre_existing_db_without_the_new_columns_migrates_it_in_place() {
        // Reproduces a real installed DB from before content_alt/thumb_path
        // existed: the old 7-column schema, with a real row already in it.
        let dir = std::env::temp_dir().join(format!("cm-migration-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("history.db");
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE items (
                    id INTEGER PRIMARY KEY,
                    kind TEXT NOT NULL,
                    content TEXT,
                    image_path TEXT,
                    preview TEXT NOT NULL,
                    dedup_key TEXT NOT NULL,
                    created_at INTEGER NOT NULL
                );
                CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
            )
            .unwrap();
            // A realistic recent timestamp -- not an ancient one, which the
            // default 30-day retention setting would legitimately prune,
            // masking what this test is actually checking (the migration).
            conn.execute(
                "INSERT INTO items (kind, content, image_path, preview, dedup_key, created_at)
                 VALUES ('text', 'pre-existing item', NULL, 'pre-existing item', 'text:pre-existing item', ?1)",
                params![now_ms()],
            )
            .unwrap();
        }

        // Before the fix, HistoryStore::open's CREATE TABLE IF NOT EXISTS is
        // a no-op against this pre-existing table, and get_history's SELECT
        // (which names content_alt/thumb_path) fails with "no such column".
        let store = HistoryStore::open(&db_path).unwrap();
        let history = store.get_history().expect("get_history must succeed against a migrated old DB");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].content.as_deref(), Some("pre-existing item"));
        assert_eq!(history[0].content_alt, None, "old rows have no content_alt, must read back as NULL not error");
        assert_eq!(history[0].thumb_path, None);

        // A fresh capture (the real-world "copy something" path) must also
        // succeed post-migration, not just reads.
        store
            .capture(NewItem {
                kind: "text".into(),
                content: Some("new item".into()),
                content_alt: None,
                image_path: None,
                thumb_path: None,
                preview: "new item".into(),
                dedup_source: "text:new item".into(),
            })
            .unwrap();
        assert_eq!(store.get_history().unwrap().len(), 2);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn prune_deletes_image_and_thumbnail_files_for_evicted_rows() {
        let store = mem_store();
        store.set_setting("max_items", "1").unwrap();
        let dir = std::env::temp_dir().join(format!("cm-prune-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let full1 = dir.join("a.png");
        let thumb1 = dir.join("a_thumb.png");
        std::fs::write(&full1, b"a").unwrap();
        std::fs::write(&thumb1, b"a-thumb").unwrap();
        let full2 = dir.join("b.png");
        std::fs::write(&full2, b"b").unwrap();

        store
            .capture(NewItem {
                kind: "image".into(),
                content: None,
                content_alt: None,
                image_path: Some(full1.to_string_lossy().into()),
                thumb_path: Some(thumb1.to_string_lossy().into()),
                preview: "a".into(),
                dedup_source: "image:a".into(),
            })
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        store
            .capture(NewItem {
                kind: "image".into(),
                content: None,
                content_alt: None,
                image_path: Some(full2.to_string_lossy().into()),
                thumb_path: None,
                preview: "b".into(),
                dedup_source: "image:b".into(),
            })
            .unwrap();

        assert!(!full1.exists(), "evicted row's full image file must be deleted by prune");
        assert!(!thumb1.exists(), "evicted row's thumbnail file must be deleted by prune too");
        assert!(full2.exists(), "surviving row's image file must not be deleted");
        std::fs::remove_dir_all(&dir).ok();
    }
}

use rusqlite::{params, Connection};

pub struct HistoryItem {
    pub id: i64,
    pub kind: String,
    pub content: Option<String>,
    pub content_alt: Option<String>,
    pub image_path: Option<String>,
    pub thumb_path: Option<String>,
    pub preview: String,
    pub created_at: i64,
}

pub struct NewItem {
    pub kind: String,
    pub content: Option<String>,
    pub content_alt: Option<String>,
    pub image_path: Option<String>,
    pub thumb_path: Option<String>,
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
                content_alt TEXT,
                image_path TEXT,
                thumb_path TEXT,
                preview TEXT NOT NULL,
                dedup_key TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_items_dedup ON items(dedup_key);
            CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )?;
        Self::migrate_add_column(&conn, "content_alt", "TEXT");
        Self::migrate_add_column(&conn, "thumb_path", "TEXT");
        let store = Self { conn };
        store.seed_default_settings()?;
        Ok(store)
    }

    /// Adds `column` to `items` if it isn't already there. `CREATE TABLE IF
    /// NOT EXISTS` above is a no-op against a pre-existing DB from before
    /// this column existed (e.g. an installed build's real history.db) --
    /// this migration is what actually brings an older DB's schema forward,
    /// so upgrading doesn't break existing history. Safe to call on every
    /// startup: `ALTER TABLE ADD COLUMN` errors (only) when the column is
    /// already present, which is expected and ignored.
    fn migrate_add_column(conn: &Connection, column: &str, sql_type: &str) {
        let _ = conn.execute(&format!("ALTER TABLE items ADD COLUMN {column} {sql_type}"), []);
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
                "UPDATE items SET created_at = ?1, content = ?2, content_alt = ?3, preview = ?4 WHERE id = ?5",
                params![now_ms(), item.content, item.content_alt, item.preview, id],
            )?;
            id
        } else {
            self.conn.execute(
                "INSERT INTO items (kind, content, content_alt, image_path, thumb_path, preview, dedup_key, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    item.kind,
                    item.content,
                    item.content_alt,
                    item.image_path,
                    item.thumb_path,
                    item.preview,
                    key,
                    now_ms()
                ],
            )?;
            self.conn.last_insert_rowid()
        };

        self.prune()?;
        Ok(id)
    }

    pub fn get_history(&self) -> rusqlite::Result<Vec<HistoryItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, content, content_alt, image_path, thumb_path, preview, created_at
             FROM items ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(HistoryItem {
                id: row.get(0)?,
                kind: row.get(1)?,
                content: row.get(2)?,
                content_alt: row.get(3)?,
                image_path: row.get(4)?,
                thumb_path: row.get(5)?,
                preview: row.get(6)?,
                created_at: row.get(7)?,
            })
        })?;
        rows.collect()
    }

    /// Deletes the row and returns every on-disk path (full image and/or
    /// thumbnail) that belonged to it, so the caller can remove them --
    /// SQLite deletion alone never touches the filesystem.
    pub fn delete_item(&self, id: i64) -> rusqlite::Result<Vec<String>> {
        let paths: Vec<String> = self
            .conn
            .query_row(
                "SELECT image_path, thumb_path FROM items WHERE id = ?1",
                params![id],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .ok()
            .map(|(image_path, thumb_path): (Option<String>, Option<String>)| {
                image_path.into_iter().chain(thumb_path).collect()
            })
            .unwrap_or_default();
        self.conn.execute("DELETE FROM items WHERE id = ?1", params![id])?;
        Ok(paths)
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
    /// on-disk image/thumbnail files for any evicted row that had them --
    /// otherwise pruned/duplicate images would orphan files under the app
    /// data dir forever, since SQLite deletion alone never touches the
    /// filesystem.
    fn prune(&self) -> rusqlite::Result<()> {
        let mut evicted_paths: Vec<String> = Vec::new();

        if let Some(max) = self.get_setting("max_items")?.and_then(|v| v.parse::<i64>().ok()) {
            evicted_paths.extend(self.paths_outside_limit(max)?);
            self.conn.execute(
                "DELETE FROM items WHERE id NOT IN (
                    SELECT id FROM items ORDER BY created_at DESC LIMIT ?1
                )",
                params![max],
            )?;
        }
        if let Some(days) = self.get_setting("retention_days")?.and_then(|v| v.parse::<i64>().ok()) {
            let cutoff = now_ms() - days * 24 * 60 * 60 * 1000;
            evicted_paths.extend(self.paths_before(cutoff)?);
            self.conn.execute("DELETE FROM items WHERE created_at < ?1", params![cutoff])?;
        }

        for path in evicted_paths {
            let _ = std::fs::remove_file(path);
        }
        Ok(())
    }

    fn paths_outside_limit(&self, max: i64) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT image_path, thumb_path FROM items WHERE id NOT IN (
                SELECT id FROM items ORDER BY created_at DESC LIMIT ?1
            )",
        )?;
        let rows = stmt.query_map(params![max], |row| {
            Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        let mut paths = Vec::new();
        for row in rows {
            let (image_path, thumb_path) = row?;
            paths.extend(image_path);
            paths.extend(thumb_path);
        }
        Ok(paths)
    }

    fn paths_before(&self, cutoff: i64) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT image_path, thumb_path FROM items WHERE created_at < ?1")?;
        let rows = stmt.query_map(params![cutoff], |row| {
            Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        let mut paths = Vec::new();
        for row in rows {
            let (image_path, thumb_path) = row?;
            paths.extend(image_path);
            paths.extend(thumb_path);
        }
        Ok(paths)
    }
}
