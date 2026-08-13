# Thumbnails + Richtext Capture Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `richtext` (CF_HTML) clipboard capture kind and row thumbnails (real image previews, static type icons for files/richtext) to the popup.

**Architecture:** Additive changes to the existing single-crate capture pipeline: two new nullable DB columns (`content_alt`, `thumb_path`), a new capture branch and write-back arm in `clipboard_io.rs`, a DTO field serving base64 thumbnails from `commands.rs`, and frontend rendering in `popup.ts`/`popup.css`. No new subsystems.

**Tech Stack:** Rust (`arboard` for HTML clipboard read/write, `image` for thumbnail resize, new `base64` dependency), TypeScript/Deno, SQLite (`rusqlite`).

**Spec:** `docs/superpowers/specs/2026-08-13-thumbnails-and-richtext-design.md`

## Global Constraints

- Richtext HTML content is capped at `TEXT_CAP_BYTES` (200,000 bytes), same cap already used for plain text, via the existing `truncate_to_byte_cap` helper.
- Image thumbnails are capped at a new `THUMBNAIL_MAX_DIMENSION` of 40px (max dimension), same resize approach (`image::imageops::resize`, `FilterType::Triangle`) as the existing full-size 1600px cap.
- Capture priority order is `image > files > richtext > text` (richtext inserted between the existing files and text checks).
- No DB migration: the two new columns go straight into the `CREATE TABLE` statement (confirmed with the user — no existing `history.db` needs to survive this change).
- `files` and `richtext` items get a single static SVG icon per kind (no per-extension icons, no rendered HTML preview) — the icons live inline in `popup.ts`.
- `text` items get no thumbnail slot at all (row layout unchanged for that kind).
- `tauri.conf.json`'s CSP needs `img-src 'self' data:` added, since it currently has no `img-src` directive and falls back to `default-src 'self'`, which blocks `data:` URIs.
- Existing Rust tests that touch the real OS clipboard must keep passing with `cargo test -- --test-threads=1` (pre-existing constraint — these tests race on shared OS clipboard state when run in parallel; this is not something this plan fixes).

---

### Task 1: Add `base64` dependency and CSP `img-src` directive

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/tauri.conf.json`

**Interfaces:**
- Produces: the `base64` crate (`base64::engine::general_purpose::STANDARD`) available to `commands.rs` in Task 5; a CSP that permits `data:` image URLs for the thumbnails Task 5/6 wire up.

- [ ] **Step 1: Add the `base64` dependency**

In `src-tauri/Cargo.toml`, add this line to the `[dependencies]` section (after `image = "0.25"`):

```toml
base64 = "0.22"
```

- [ ] **Step 2: Add `img-src` to the CSP**

In `src-tauri/tauri.conf.json`, change:

```json
      "csp": "default-src 'self'; script-src 'self'; style-src 'self'"
```

to:

```json
      "csp": "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:"
```

- [ ] **Step 3: Verify the crate still builds**

Run: `cd src-tauri && cargo build`
Expected: builds successfully, `base64` appears in `Cargo.lock`.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/tauri.conf.json
git commit -m "chore: add base64 dependency and allow data: image URLs in CSP"
```

---

### Task 2: Extend the store schema and structs with `content_alt`/`thumb_path`

This task threads two new nullable fields through the whole store layer so the
crate keeps compiling and all existing behavior is unchanged — no new capture
logic yet (that's Tasks 3/4). `image`/`text`/`files` capture sites just pass
`None` for both new fields until Tasks 3/4 give `image` a real `thumb_path`
and add the `richtext` branch.

**Files:**
- Modify: `src-tauri/src/store.rs`
- Modify: `src-tauri/src/clipboard_io.rs` (only the `NewItem`/`HistoryItem` literal sites — no new capture logic)
- Modify: `src-tauri/src/commands.rs` (only `delete_item` command, to match the new `store::delete_item` return type)

**Interfaces:**
- Produces: `store::NewItem { kind, content, content_alt, image_path, thumb_path, preview, dedup_source }`, `store::HistoryItem { id, kind, content, content_alt, image_path, thumb_path, preview, created_at }`, `HistoryStore::delete_item(id) -> rusqlite::Result<Vec<String>>` (all leftover on-disk paths for the deleted row, 0-2 entries).
- Consumes: nothing new (builds on the existing `HistoryStore`/`rusqlite` setup).

- [ ] **Step 1: Rewrite `src-tauri/src/store.rs`**

Replace the entire file with:

```rust
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
```

- [ ] **Step 2: Update `NewItem`/`HistoryItem` literals in `src-tauri/src/clipboard_io.rs`**

In the `files` branch of `capture_current_clipboard`, change:

```rust
        return Some(NewItem {
            kind: "files".into(),
            content: Some(content),
            image_path: None,
            preview,
            dedup_source: format!("files:{joined}"),
        });
```

to:

```rust
        return Some(NewItem {
            kind: "files".into(),
            content: Some(content),
            content_alt: None,
            image_path: None,
            thumb_path: None,
            preview,
            dedup_source: format!("files:{joined}"),
        });
```

In the `image` branch, change:

```rust
        return Some(NewItem {
            kind: "image".into(),
            content: None,
            image_path: Some(path.to_string_lossy().to_string()),
            preview: format!("Image ({out_w}x{out_h})"),
            dedup_source: format!("image:{hash}"),
        });
```

to:

```rust
        return Some(NewItem {
            kind: "image".into(),
            content: None,
            content_alt: None,
            image_path: Some(path.to_string_lossy().to_string()),
            thumb_path: None,
            preview: format!("Image ({out_w}x{out_h})"),
            dedup_source: format!("image:{hash}"),
        });
```

(Task 3 will change `thumb_path: None` to a real path here.)

In the `text` branch, change:

```rust
        return Some(NewItem {
            kind: "text".into(),
            content: Some(truncated.clone()),
            image_path: None,
            preview,
            dedup_source: format!("text:{truncated}"),
        });
```

to:

```rust
        return Some(NewItem {
            kind: "text".into(),
            content: Some(truncated.clone()),
            content_alt: None,
            image_path: None,
            thumb_path: None,
            preview,
            dedup_source: format!("text:{truncated}"),
        });
```

- [ ] **Step 3: Update the `HistoryItem` literals in `clipboard_io.rs`'s own test module**

Each of the three `HistoryItem { id: 1, kind: "text".into(), content: Some(...), image_path: None, preview: ..., created_at: 0 }` literals (in `write_then_capture_round_trips_text`, `text_over_cap_is_truncated_on_capture`, and `non_ascii_text_over_cap_is_truncated_by_bytes_not_chars`) need `content_alt: None,` and `thumb_path: None,` added. For example, change:

```rust
        let item = HistoryItem {
            id: 1,
            kind: "text".into(),
            content: Some("clipboard round trip".into()),
            image_path: None,
            preview: "clipboard round trip".into(),
            created_at: 0,
        };
```

to:

```rust
        let item = HistoryItem {
            id: 1,
            kind: "text".into(),
            content: Some("clipboard round trip".into()),
            content_alt: None,
            image_path: None,
            thumb_path: None,
            preview: "clipboard round trip".into(),
            created_at: 0,
        };
```

Apply the same two-field addition (`content_alt: None,` after `content`, `thumb_path: None,` after `image_path`) to the other two `HistoryItem` literals in that test module.

- [ ] **Step 4: Update `delete_item` in `src-tauri/src/commands.rs`**

Change:

```rust
#[tauri::command]
pub fn delete_item(id: i64, store: State<Store>) -> Result<(), String> {
    let image_path = store.lock().unwrap_or_else(PoisonError::into_inner).delete_item(id).map_err(|e| e.to_string())?;
    if let Some(path) = image_path {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}
```

to:

```rust
#[tauri::command]
pub fn delete_item(id: i64, store: State<Store>) -> Result<(), String> {
    let paths = store.lock().unwrap_or_else(PoisonError::into_inner).delete_item(id).map_err(|e| e.to_string())?;
    for path in paths {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}
```

- [ ] **Step 5: Run the full test suite**

Run: `cd src-tauri && cargo test -- --test-threads=1`
Expected: all tests pass (existing tests still pass unchanged; the three new `store.rs` tests — `capture_round_trips_content_alt_and_thumb_path`, `delete_item_removes_row_and_returns_leftover_paths`, `prune_deletes_image_and_thumbnail_files_for_evicted_rows` — pass).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/store.rs src-tauri/src/clipboard_io.rs src-tauri/src/commands.rs
git commit -m "feat: add content_alt/thumb_path columns and thread them through the store"
```

---

### Task 3: Generate image thumbnails on capture

**Files:**
- Modify: `src-tauri/src/clipboard_io.rs`

**Interfaces:**
- Consumes: `store::NewItem.thumb_path` field (from Task 2), `crate::win32::images_dir()` (existing).
- Produces: every captured `image` item now has `thumb_path: Some(...)` pointing at a real 40px-max-dimension PNG alongside the existing full-size image.

- [ ] **Step 1: Write the failing test**

Add this test to the `#[cfg(test)] mod tests` block in `src-tauri/src/clipboard_io.rs` (near the other image-capture tests):

```rust
    #[test]
    fn image_capture_produces_a_smaller_thumbnail_file() {
        let pixels = image::RgbaImage::from_pixel(200, 100, image::Rgba([50, 60, 70, 255]));
        let mut clipboard = arboard::Clipboard::new().unwrap();
        clipboard
            .set_image(arboard::ImageData { width: 200, height: 100, bytes: pixels.into_raw().into() })
            .unwrap();
        let captured = capture_current_clipboard().expect("expected an image capture");
        assert_eq!(captured.kind, "image");
        let thumb_path = captured.thumb_path.expect("image capture must produce a thumb_path");
        let thumb_meta = std::fs::metadata(&thumb_path).expect("thumbnail file must exist on disk");
        let full_meta = std::fs::metadata(captured.image_path.unwrap()).unwrap();
        assert!(thumb_meta.len() < full_meta.len(), "thumbnail file should be smaller than the full-size image");

        let thumb_img = image::open(&thumb_path).unwrap();
        assert!(
            thumb_img.width() <= 40 && thumb_img.height() <= 40,
            "thumbnail dimensions ({}, {}) must be capped at 40px",
            thumb_img.width(),
            thumb_img.height()
        );
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd src-tauri && cargo test image_capture_produces_a_smaller_thumbnail_file -- --test-threads=1`
Expected: FAIL — `captured.thumb_path` is `None` (`.expect(...)` panics).

- [ ] **Step 3: Add the `THUMBNAIL_MAX_DIMENSION` constant**

In `src-tauri/src/clipboard_io.rs`, next to the existing `IMAGE_MAX_DIMENSION` constant, add:

```rust
const THUMBNAIL_MAX_DIMENSION: u32 = 40;
```

- [ ] **Step 4: Generate and save the thumbnail in the image branch**

In the image branch of `capture_current_clipboard`, after the existing full-size resize/hash/save block (right before `return Some(NewItem { ... })`), insert:

```rust
        let thumb_scale = (THUMBNAIL_MAX_DIMENSION as f32 / w.max(h) as f32).min(1.0);
        let (thumb_w, thumb_h) = ((w as f32 * thumb_scale) as u32, (h as f32 * thumb_scale) as u32);
        let thumbnail = image::imageops::resize(&img_buf, thumb_w.max(1), thumb_h.max(1), image::imageops::FilterType::Triangle);
        let thumb_path = dir.join(format!("{hash}_thumb.png"));
        if !thumb_path.exists() {
            thumbnail.save(&thumb_path).ok()?;
        }
```

Then change the `thumb_path: None,` field in that same branch's `NewItem { ... }` literal to:

```rust
            thumb_path: Some(thumb_path.to_string_lossy().to_string()),
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cd src-tauri && cargo test image_capture_produces_a_smaller_thumbnail_file -- --test-threads=1`
Expected: PASS

- [ ] **Step 6: Run the full test suite**

Run: `cd src-tauri && cargo test -- --test-threads=1`
Expected: all tests pass, including the pre-existing image dedup tests (they still hash/dedup on the full image content — the thumbnail path is derived from the same hash, so dedup behavior is unaffected).

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/clipboard_io.rs
git commit -m "feat: generate a 40px thumbnail alongside every captured image"
```

---

### Task 4: Capture and write back richtext (CF_HTML)

**Files:**
- Modify: `src-tauri/src/clipboard_io.rs`

**Interfaces:**
- Consumes: `arboard::Clipboard::get().html() -> Result<String, arboard::Error>`, `arboard::Clipboard::set().html(html, alt_text) -> Result<(), arboard::Error>` (both confirmed present in `arboard` 3.6.1's Windows backend).
- Produces: `capture_current_clipboard()` returns `NewItem { kind: "richtext", content: Some(html), content_alt: Some(plain_text), .. }` whenever HTML is on the clipboard (checked after `files`, before `text`); `write_item_to_clipboard` handles `item.kind == "richtext"`.

- [ ] **Step 1: Write the failing capture test**

Add to `src-tauri/src/clipboard_io.rs`'s test module:

```rust
    #[test]
    fn html_on_clipboard_is_captured_as_richtext_with_plain_text_alt() {
        let mut clipboard = arboard::Clipboard::new().unwrap();
        clipboard.set().html("<b>hello</b>", Some("hello")).unwrap();
        let captured = capture_current_clipboard().expect("expected a richtext capture");
        assert_eq!(captured.kind, "richtext");
        assert_eq!(captured.content.as_deref(), Some("<b>hello</b>"));
        assert_eq!(captured.content_alt.as_deref(), Some("hello"));
        assert_eq!(captured.preview, "hello");
    }

    #[test]
    fn richtext_write_then_capture_round_trips() {
        let item = HistoryItem {
            id: 1,
            kind: "richtext".into(),
            content: Some("<i>styled</i>".into()),
            content_alt: Some("styled".into()),
            image_path: None,
            thumb_path: None,
            preview: "styled".into(),
            created_at: 0,
        };
        write_item_to_clipboard(&item).unwrap();
        let captured = capture_current_clipboard().expect("expected a richtext capture");
        assert_eq!(captured.kind, "richtext");
        assert_eq!(captured.content.as_deref(), Some("<i>styled</i>"));
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd src-tauri && cargo test richtext -- --test-threads=1`
Expected: FAIL — both tests currently capture as `"text"` (or hit the `unknown item kind` error in `write_item_to_clipboard`), since there's no richtext branch yet.

- [ ] **Step 3: Add the tag-strip fallback helper**

In `src-tauri/src/clipboard_io.rs`, near `truncate_to_byte_cap`, add:

```rust
/// Last-resort plain-text fallback for a richtext capture when the
/// clipboard offers CF_HTML but no CF_UNICODETEXT alongside it (rare in
/// practice -- most rich sources set both). Strips anything between `<`
/// and `>` rather than parsing HTML properly, which is good enough for a
/// preview/dedup/alt-text string.
fn strip_html_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}
```

- [ ] **Step 4: Add the richtext capture branch**

In `capture_current_clipboard`, insert this block after the `files` (`read_hdrop`) branch and before the `text` (`get_text`) branch:

```rust
    if let Ok(html) = retry(|| clipboard.get().html()) {
        if !html.trim().is_empty() {
            let truncated_html = truncate_to_byte_cap(&html, TEXT_CAP_BYTES);
            let alt = match retry(|| clipboard.get_text()) {
                Ok(text) => truncate_to_byte_cap(&text, TEXT_CAP_BYTES),
                Err(_) => truncate_to_byte_cap(&strip_html_tags(&truncated_html), TEXT_CAP_BYTES),
            };
            let preview: String = alt.chars().take(120).collect();
            return Some(NewItem {
                kind: "richtext".into(),
                content: Some(truncated_html),
                content_alt: Some(alt.clone()),
                image_path: None,
                thumb_path: None,
                preview,
                dedup_source: format!("richtext:{alt}"),
            });
        }
    }
```

- [ ] **Step 5: Add the richtext write-back arm**

In `write_item_to_clipboard`, add a new match arm before the `other => ...` fallback:

```rust
        "richtext" => {
            let html = item.content.clone().unwrap_or_default();
            let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
            clipboard.set().html(html, item.content_alt.clone()).map_err(|e| e.to_string())
        }
```

- [ ] **Step 6: Run the new tests to verify they pass**

Run: `cd src-tauri && cargo test richtext -- --test-threads=1`
Expected: PASS

- [ ] **Step 7: Run the full test suite**

Run: `cd src-tauri && cargo test -- --test-threads=1`
Expected: all tests pass. In particular, confirm the pre-existing `write_then_capture_round_trips_text` test still passes — writing plain text with `clipboard.set_text(...)` does not place any `CF_HTML` on the clipboard, so it's still captured as `"text"`, not `"richtext"`.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/clipboard_io.rs
git commit -m "feat: capture and write back richtext (CF_HTML) clipboard content"
```

---

### Task 5: Serve base64 thumbnails from `get_history`

**Files:**
- Modify: `src-tauri/src/commands.rs`

**Interfaces:**
- Consumes: `base64::engine::general_purpose::STANDARD` (from Task 1's dependency), `store::HistoryItem` (from Task 2, now with `thumb_path`).
- Produces: `HistoryItemDto { id, kind, preview, thumbnail: Option<String>, created_at }` — `thumbnail` is a `data:image/png;base64,...` string when `thumb_path` is set and readable, `None` otherwise. `history_item_to_dto(item: HistoryItem) -> HistoryItemDto`, a free function usable in later tasks/tests without a `tauri::State`.

- [ ] **Step 1: Write the failing tests**

Add this test module to the bottom of `src-tauri/src/commands.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::HistoryItem;

    #[test]
    fn dto_conversion_includes_base64_thumbnail_when_thumb_path_set() {
        let dir = std::env::temp_dir().join(format!("cm-dto-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let thumb_path = dir.join("thumb.png");
        std::fs::write(&thumb_path, b"fake-png-bytes").unwrap();

        let item = HistoryItem {
            id: 1,
            kind: "image".into(),
            content: None,
            content_alt: None,
            image_path: Some("full.png".into()),
            thumb_path: Some(thumb_path.to_string_lossy().to_string()),
            preview: "Image (10x10)".into(),
            created_at: 0,
        };
        let dto = history_item_to_dto(item);
        let expected = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(b"fake-png-bytes")
        );
        assert_eq!(dto.thumbnail, Some(expected));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dto_conversion_has_no_thumbnail_when_thumb_path_absent() {
        let item = HistoryItem {
            id: 1,
            kind: "text".into(),
            content: Some("hi".into()),
            content_alt: None,
            image_path: None,
            thumb_path: None,
            preview: "hi".into(),
            created_at: 0,
        };
        let dto = history_item_to_dto(item);
        assert!(dto.thumbnail.is_none());
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test dto_conversion -- --test-threads=1`
Expected: FAIL to compile — `history_item_to_dto` doesn't exist yet and `HistoryItemDto` has no `thumbnail` field.

- [ ] **Step 3: Add the `base64` import, `thumbnail` field, and conversion helper**

At the top of `src-tauri/src/commands.rs`, add:

```rust
use base64::Engine;
```

Change the `HistoryItemDto` struct from:

```rust
#[derive(Serialize)]
pub struct HistoryItemDto {
    pub id: i64,
    pub kind: String,
    pub preview: String,
    pub created_at: i64,
}
```

to:

```rust
#[derive(Serialize)]
pub struct HistoryItemDto {
    pub id: i64,
    pub kind: String,
    pub preview: String,
    pub thumbnail: Option<String>,
    pub created_at: i64,
}

/// Converts a stored `HistoryItem` into its wire DTO, reading and
/// base64-encoding the (already-tiny, precomputed) thumbnail file for image
/// items. Kept as a free function, separate from the `#[tauri::command]`
/// wrapper, so it's testable without a `tauri::State`/running app.
fn history_item_to_dto(item: crate::store::HistoryItem) -> HistoryItemDto {
    let thumbnail = item.thumb_path.as_deref().and_then(|path| {
        std::fs::read(path)
            .ok()
            .map(|bytes| format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
    });
    HistoryItemDto {
        id: item.id,
        kind: item.kind,
        preview: item.preview,
        thumbnail,
        created_at: item.created_at,
    }
}
```

- [ ] **Step 4: Use the helper in `get_history`**

Change:

```rust
#[tauri::command]
pub fn get_history(store: State<Store>) -> Result<Vec<HistoryItemDto>, String> {
    store
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_history()
        .map(|items| {
            items
                .into_iter()
                .map(|i| HistoryItemDto {
                    id: i.id,
                    kind: i.kind,
                    preview: i.preview,
                    created_at: i.created_at,
                })
                .collect()
        })
        .map_err(|e| e.to_string())
}
```

to:

```rust
#[tauri::command]
pub fn get_history(store: State<Store>) -> Result<Vec<HistoryItemDto>, String> {
    store
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_history()
        .map(|items| items.into_iter().map(history_item_to_dto).collect())
        .map_err(|e| e.to_string())
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test dto_conversion -- --test-threads=1`
Expected: PASS

- [ ] **Step 6: Run the full test suite**

Run: `cd src-tauri && cargo test -- --test-threads=1`
Expected: all tests pass, `cargo build` succeeds.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "feat: serve base64 image thumbnails in get_history's DTO"
```

---

### Task 6: Render thumbnails in the popup

**Files:**
- Modify: `src/shared/bindings.ts`
- Modify: `src/popup/popup.ts`
- Modify: `src/popup/popup.css`

**Interfaces:**
- Consumes: `HistoryItemDto.kind` now includes `"richtext"`; new `HistoryItemDto.thumbnail: string | null` field (from Task 5).
- Produces: each popup row shows an `<img class="thumb">` for image items with a thumbnail, a `<span class="thumb-icon">` (folder/document SVG) for files/richtext items, and no thumbnail element at all for text items.

- [ ] **Step 1: Update `HistoryItemDto` in `src/shared/bindings.ts`**

Change:

```ts
export interface HistoryItemDto {
  id: number;
  kind: "text" | "image" | "files";
  preview: string;
  created_at: number;
}
```

to:

```ts
export interface HistoryItemDto {
  id: number;
  kind: "text" | "image" | "files" | "richtext";
  preview: string;
  thumbnail: string | null;
  created_at: number;
}
```

- [ ] **Step 2: Add thumbnail icons and a `createThumbnail` helper in `src/popup/popup.ts`**

Add these two constants near the top of the file (after the imports):

```ts
const FOLDER_ICON_SVG =
  `<svg viewBox="0 0 16 16" width="16" height="16" fill="currentColor"><path d="M1.5 3A1.5 1.5 0 0 1 3 1.5h3.172a1.5 1.5 0 0 1 1.06.44l1.329 1.328A.5.5 0 0 0 8.914 3H13A1.5 1.5 0 0 1 14.5 4.5v8A1.5 1.5 0 0 1 13 14H3a1.5 1.5 0 0 1-1.5-1.5v-9Z"/></svg>`;
const DOC_ICON_SVG =
  `<svg viewBox="0 0 16 16" width="16" height="16" fill="currentColor"><path d="M4 1.5A1.5 1.5 0 0 0 2.5 3v10A1.5 1.5 0 0 0 4 14.5h8a1.5 1.5 0 0 0 1.5-1.5V5.621a1.5 1.5 0 0 0-.44-1.06L10.44 1.94A1.5 1.5 0 0 0 9.378 1.5H4Z"/><path fill="#1e1e22" d="M5 6.5h6v1H5zM5 9h6v1H5z"/></svg>`;

function createThumbnail(item: HistoryItemDto): HTMLElement | null {
  if (item.kind === "image") {
    if (!item.thumbnail) return null;
    const img = document.createElement("img");
    img.className = "thumb";
    img.src = item.thumbnail;
    return img;
  }
  if (item.kind === "files" || item.kind === "richtext") {
    const icon = document.createElement("span");
    icon.className = "thumb-icon";
    icon.innerHTML = item.kind === "files" ? FOLDER_ICON_SVG : DOC_ICON_SVG;
    return icon;
  }
  return null;
}
```

- [ ] **Step 3: Wire `createThumbnail` into `render()`**

In `render()`, change:

```ts
    const badge = document.createElement("span");
    badge.className = "badge";
    badge.textContent = i < 9 ? String(i + 1) : "";
    row.appendChild(badge);

    const preview = document.createElement("span");
```

to:

```ts
    const badge = document.createElement("span");
    badge.className = "badge";
    badge.textContent = i < 9 ? String(i + 1) : "";
    row.appendChild(badge);

    const thumb = createThumbnail(item);
    if (thumb) row.appendChild(thumb);

    const preview = document.createElement("span");
```

- [ ] **Step 4: Add thumbnail styles to `src/popup/popup.css`**

Add these two rules after the existing `.badge` rule:

```css
.thumb { width: 16px; height: 16px; border-radius: 3px; object-fit: cover; flex-shrink: 0; }
.thumb-icon { width: 16px; height: 16px; flex-shrink: 0; opacity: 0.7; display: inline-flex; }
```

- [ ] **Step 5: Type-check and build the frontend**

Run: `deno check src/popup/popup.ts`
Expected: no type errors.

Run: `deno task build`
Expected: builds successfully, `src/dist/popup/popup.js` is regenerated.

- [ ] **Step 6: Commit**

```bash
git add src/shared/bindings.ts src/popup/popup.ts src/popup/popup.css
git commit -m "feat: render image/files/richtext thumbnails in the popup"
```

---

### Task 7: Manual end-to-end verification

Automated tests cover the capture/store/serve logic; the popup's visual
rendering and the real Windows clipboard formats set by other applications
need a manual pass (the project's existing testing convention — no
automated UI/e2e layer, per the original design doc).

**Files:** none (verification only).

- [ ] **Step 1: Build and run the app**

Run: `cargo tauri dev` (from `src-tauri`, or the project's existing dev workflow if different)

- [ ] **Step 2: Verify image thumbnails**

Copy an image (e.g. a screenshot, or right-click "Copy image" in a browser). Press Ctrl+Alt+V. Confirm the popup shows a small real thumbnail of that image next to the row, not just text.

- [ ] **Step 3: Verify files show a folder icon**

Select a file in File Explorer and press Ctrl+C. Press Ctrl+Alt+V. Confirm the row shows the folder icon and the correct file-path preview text.

- [ ] **Step 4: Verify richtext capture**

Select some formatted text on a web page (bold/colored/etc.) and copy it. Press Ctrl+Alt+V. Confirm the row shows the document icon, and that the preview text is the plain-text version of what was copied (not raw HTML tags).

- [ ] **Step 5: Verify richtext write-back**

Click that richtext row to select it, then paste (Ctrl+V) into an editor that preserves formatting (e.g. a browser text field, Word, or an email compose window). Confirm the formatting (bold/color/etc.) is preserved, not just plain text.

- [ ] **Step 6: Verify plain text still works and has no thumbnail slot**

Copy plain text from a source that sets no `CF_HTML` (e.g. a terminal). Press Ctrl+Alt+V. Confirm it's captured as plain text (no icon column shifting the row layout) exactly as it looked before this change.

- [ ] **Step 7: Verify delete cleans up thumbnail files**

Note an image row's thumbnail file path (check `%APPDATA%\clipboard-manager\images\`), delete that row via the popup's `×` button, and confirm both the full image and `_thumb.png` file are gone from disk afterward.
