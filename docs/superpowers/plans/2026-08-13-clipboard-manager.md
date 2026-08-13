# Windows Clipboard Manager Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Windows background app that records clipboard history (text/images/files) and shows a popup near the cursor when Ctrl+C is held 3 seconds, letting the user browse/select/delete items with mouse or `1`-`9`.

**Architecture:** Tauri (Rust core + WebView2 frontend). A native watcher thread owns a low-level keyboard hook (hold detection) and a clipboard-format-listener window (capture). A SQLite-backed store handles dedup/prune/CRUD, exposed to the frontend only via Tauri commands. Two webview windows (popup, settings) built from Deno-bundled TS/HTML/CSS. A tray icon plus `tauri-plugin-autostart` handle background operation.

**Tech Stack:** Rust, Tauri 2.x, `rusqlite`, `arboard` (clipboard text/image read-write), `windows-rs` (hook, clipboard listener, `CF_HDROP`, cursor position), `sha2`, `tauri-plugin-autostart`, Deno (frontend build via `npm:esbuild`), TypeScript/HTML/CSS.

**Spec:** `docs/superpowers/specs/2026-08-13-clipboard-manager-design.md`

## Global Constraints

- Windows only (no cross-platform code paths).
- Selecting an item writes it back to the OS clipboard only — never simulates a paste keystroke.
- Text content capped at 200KB before storage; images downscaled to a max dimension before being saved as the stored PNG.
- No pinning in v1.
- `CF_EXCLUDECLIPBOARDHISTORY`-marked clipboard changes are never recorded.
- Format capture priority: `CF_HDROP` (files) > image (`CF_DIB`/PNG) > `CF_UNICODETEXT` (text).
- Settings (`max_items`, `retention_days`, `start_with_windows`) are stored in the `settings` table and are configurable, not hardcoded.

---

## File Structure

```
src-tauri/
  Cargo.toml
  tauri.conf.json
  src/
    main.rs              # app entry: builds windows, tray, wires commands, starts watcher thread
    store.rs              # SQLite schema, dedup/prune/CRUD, settings get/set
    hold_detector.rs       # pure Ctrl+C-hold state machine
    position.rs             # pure popup-position clamping
    watcher.rs               # keyboard hook + clipboard-format-listener thread (drives hold_detector, calls store)
    clipboard_io.rs            # read current clipboard into a NewItem; write a HistoryItem back to clipboard
    commands.rs                 # #[tauri::command] handlers calling store.rs / clipboard_io.rs

src/
  popup/
    index.html
    popup.ts
    popup.css
  settings/
    index.html
    settings.ts
    settings.css
  shared/
    bindings.ts          # TS types + typed wrappers around Tauri invoke/listen

deno.json               # Deno tasks: build (esbuild bundle popup.ts + settings.ts to dist/)
```

---

### Task 1: Project scaffold

**Files:**
- Create: `src-tauri/Cargo.toml`
- Create: `src-tauri/tauri.conf.json`
- Create: `src-tauri/src/main.rs`
- Create: `deno.json`
- Create: `src/popup/index.html`, `src/settings/index.html`

**Interfaces:**
- Produces: a running `cargo tauri dev` that opens one blank window, and a `deno task build` that produces `dist/popup/popup.js` and `dist/settings/settings.js` from placeholder `.ts` entry points (empty files with `console.log("ready")` — real content added in Tasks 8/9).

- [ ] **Step 1: Create `deno.json` with a build task**

```json
{
  "tasks": {
    "build": "deno run -A npm:esbuild src/popup/popup.ts src/settings/settings.ts --bundle --outdir=dist --entry-names=[dir]/[name]"
  }
}
```

- [ ] **Step 2: Create placeholder frontend entry points**

`src/popup/popup.ts`:
```ts
console.log("popup ready");
```

`src/settings/settings.ts`:
```ts
console.log("settings ready");
```

`src/popup/index.html`:
```html
<!doctype html>
<html><head><meta charset="utf-8"><script type="module" src="../../dist/popup/popup.js"></script></head>
<body></body></html>
```

`src/settings/index.html`: same pattern pointing at `dist/settings/settings.js`.

- [ ] **Step 3: Run the build task and verify output**

Run: `deno task build`
Expected: `dist/popup/popup.js` and `dist/settings/settings.js` exist, no errors.

- [ ] **Step 4: Scaffold the Tauri project**

Run: `cargo install tauri-cli --version "^2"` (if not already installed), then `cargo tauri init` inside `src-tauri/`, pointing `devPath`/`distDir` at the Deno-built `dist/` and the two HTML entry points above. Add to `src-tauri/Cargo.toml` dependencies (versions as of latest stable at implementation time): `tauri`, `rusqlite` (`bundled` feature), `arboard`, `windows` (crate, with `Win32_UI_WindowsAndMessaging`, `Win32_UI_Input_KeyboardAndMouse`, `Win32_System_DataExchange`, `Win32_Graphics_Gdi` features), `sha2`, `tauri-plugin-autostart`.

- [ ] **Step 5: Verify `main.rs` builds and runs a blank window**

`src-tauri/src/main.rs`:
```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

Run: `cargo tauri dev`
Expected: a window opens showing the popup `index.html`, no build errors.

- [ ] **Step 6: Commit**

```bash
git init
git add deno.json src/popup src/settings src-tauri
git commit -m "chore: scaffold Tauri + Deno-built frontend"
```

---

### Task 2: History store (SQLite)

**Files:**
- Create: `src-tauri/src/store.rs`
- Modify: `src-tauri/src/main.rs` (add `mod store;`)

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces (used by Tasks 6, 7):
  - `pub struct HistoryItem { pub id: i64, pub kind: String, pub content: Option<String>, pub image_path: Option<String>, pub preview: String, pub created_at: i64 }`
  - `pub struct NewItem { pub kind: String, pub content: Option<String>, pub image_path: Option<String>, pub preview: String, pub dedup_source: String }`
  - `pub struct HistoryStore { ... }`
  - `impl HistoryStore { pub fn open(path: &std::path::Path) -> rusqlite::Result<Self>; pub fn capture(&self, item: NewItem) -> rusqlite::Result<i64>; pub fn get_history(&self) -> rusqlite::Result<Vec<HistoryItem>>; pub fn delete_item(&self, id: i64) -> rusqlite::Result<Option<String>>; pub fn get_setting(&self, key: &str) -> rusqlite::Result<Option<String>>; pub fn set_setting(&self, key: &str, value: &str) -> rusqlite::Result<()>; }`

- [ ] **Step 1: Write failing tests for capture/dedup/get_history**

`src-tauri/src/store.rs` (top, test module):
```rust
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
        assert_eq!(store.get_setting("retention_days").unwrap(), None);
        store.set_setting("retention_days", "30").unwrap();
        assert_eq!(store.get_setting("retention_days").unwrap(), Some("30".into()));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml store::tests`
Expected: FAIL — `HistoryStore`, `NewItem` not defined yet.

- [ ] **Step 3: Implement `HistoryStore`**

```rust
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
        Ok(Self { conn })
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

    fn prune(&self) -> rusqlite::Result<()> {
        if let Some(max) = self.get_setting("max_items")?.and_then(|v| v.parse::<i64>().ok()) {
            self.conn.execute(
                "DELETE FROM items WHERE id NOT IN (
                    SELECT id FROM items ORDER BY created_at DESC LIMIT ?1
                )",
                params![max],
            )?;
        }
        if let Some(days) = self.get_setting("retention_days")?.and_then(|v| v.parse::<i64>().ok()) {
            let cutoff = now_ms() - days * 24 * 60 * 60 * 1000;
            self.conn.execute("DELETE FROM items WHERE created_at < ?1", params![cutoff])?;
        }
        Ok(())
    }
}
```

Add `mod store;` to `src-tauri/src/main.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml store::tests`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/store.rs src-tauri/src/main.rs
git commit -m "feat: add SQLite history store with dedup and pruning"
```

---

### Task 3: Ctrl+C hold-detector (pure state machine)

**Files:**
- Create: `src-tauri/src/hold_detector.rs`
- Modify: `src-tauri/src/main.rs` (add `mod hold_detector;`)

**Interfaces:**
- Produces (used by Task 5):
  - `pub struct HoldDetector { ... }` with `pub fn new() -> Self`, `pub fn set_ctrl(&mut self, down: bool)`, `pub fn set_c(&mut self, down: bool)`, `pub fn check(&mut self, now: std::time::Instant, threshold: std::time::Duration) -> bool`.

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn does_not_fire_before_threshold() {
        let mut d = HoldDetector::new();
        let t0 = Instant::now();
        d.set_ctrl(true);
        d.set_c(true);
        assert!(!d.check(t0, Duration::from_secs(3)));
        assert!(!d.check(t0 + Duration::from_millis(2999), Duration::from_secs(3)));
    }

    #[test]
    fn fires_once_at_threshold_and_not_again_while_held() {
        let mut d = HoldDetector::new();
        let t0 = Instant::now();
        d.set_ctrl(true);
        d.set_c(true);
        d.check(t0, Duration::from_secs(3));
        assert!(d.check(t0 + Duration::from_secs(3), Duration::from_secs(3)));
        assert!(!d.check(t0 + Duration::from_secs(4), Duration::from_secs(3)));
    }

    #[test]
    fn releasing_c_resets_and_requires_full_hold_again() {
        let mut d = HoldDetector::new();
        let t0 = Instant::now();
        d.set_ctrl(true);
        d.set_c(true);
        d.check(t0, Duration::from_secs(3));
        d.set_c(false);
        assert!(!d.check(t0 + Duration::from_secs(3), Duration::from_secs(3)));
        d.set_c(true);
        assert!(!d.check(t0 + Duration::from_secs(3), Duration::from_secs(3)), "must restart the 3s window");
        assert!(d.check(t0 + Duration::from_secs(6), Duration::from_secs(3)));
    }

    #[test]
    fn only_ctrl_or_only_c_never_fires() {
        let mut d = HoldDetector::new();
        let t0 = Instant::now();
        d.set_ctrl(true);
        assert!(!d.check(t0 + Duration::from_secs(10), Duration::from_secs(3)));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml hold_detector::tests`
Expected: FAIL — `HoldDetector` not defined.

- [ ] **Step 3: Implement `HoldDetector`**

```rust
use std::time::{Duration, Instant};

pub struct HoldDetector {
    ctrl_down: bool,
    c_down: bool,
    hold_started_at: Option<Instant>,
    fired: bool,
}

impl HoldDetector {
    pub fn new() -> Self {
        Self { ctrl_down: false, c_down: false, hold_started_at: None, fired: false }
    }

    pub fn set_ctrl(&mut self, down: bool) {
        self.ctrl_down = down;
        self.on_state_change();
    }

    pub fn set_c(&mut self, down: bool) {
        self.c_down = down;
        self.on_state_change();
    }

    fn on_state_change(&mut self) {
        if !(self.ctrl_down && self.c_down) {
            self.hold_started_at = None;
            self.fired = false;
        }
    }

    /// Call periodically (e.g. every 100ms) from the watcher thread's timer.
    pub fn check(&mut self, now: Instant, threshold: Duration) -> bool {
        if self.ctrl_down && self.c_down {
            let start = *self.hold_started_at.get_or_insert(now);
            if !self.fired && now.duration_since(start) >= threshold {
                self.fired = true;
                return true;
            }
        }
        false
    }
}
```

Add `mod hold_detector;` to `src-tauri/src/main.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml hold_detector::tests`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/hold_detector.rs src-tauri/src/main.rs
git commit -m "feat: add pure Ctrl+C hold-detection state machine"
```

---

### Task 4: Popup position clamping (pure)

**Files:**
- Create: `src-tauri/src/position.rs`
- Modify: `src-tauri/src/main.rs` (add `mod position;`)

**Interfaces:**
- Produces (used by Task 5): `pub fn clamp_popup_position(cursor: (i32, i32), popup_size: (i32, i32), screen: (i32, i32, i32, i32)) -> (i32, i32)` where `screen` is `(x, y, width, height)`.

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_as_is_when_room_available() {
        let pos = clamp_popup_position((100, 100), (200, 300), (0, 0, 1920, 1080));
        assert_eq!(pos, (100, 100));
    }

    #[test]
    fn flips_left_when_overflowing_right_edge() {
        let pos = clamp_popup_position((1850, 100), (200, 300), (0, 0, 1920, 1080));
        assert_eq!(pos, (1650, 100));
    }

    #[test]
    fn flips_up_when_overflowing_bottom_edge() {
        let pos = clamp_popup_position((100, 1000), (200, 300), (0, 0, 1920, 1080));
        assert_eq!(pos, (100, 700));
    }

    #[test]
    fn clamps_to_screen_origin_in_top_left_corner() {
        let pos = clamp_popup_position((-50, -50), (200, 300), (0, 0, 1920, 1080));
        assert_eq!(pos, (0, 0));
    }

    #[test]
    fn works_on_a_non_primary_monitor_with_negative_origin() {
        // secondary monitor to the left of the primary, virtual-screen coords are negative
        let pos = clamp_popup_position((-1800, 100), (200, 300), (-1920, 0, 1920, 1080));
        assert_eq!(pos, (-1800, 100));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml position::tests`
Expected: FAIL — `clamp_popup_position` not defined.

- [ ] **Step 3: Implement `clamp_popup_position`**

```rust
pub fn clamp_popup_position(
    cursor: (i32, i32),
    popup_size: (i32, i32),
    screen: (i32, i32, i32, i32),
) -> (i32, i32) {
    let (cx, cy) = cursor;
    let (pw, ph) = popup_size;
    let (sx, sy, sw, sh) = screen;

    let mut x = cx;
    let mut y = cy;

    if x + pw > sx + sw {
        x = cx - pw;
    }
    if y + ph > sy + sh {
        y = cy - ph;
    }

    x = x.max(sx);
    y = y.max(sy);

    (x, y)
}
```

Add `mod position;` to `src-tauri/src/main.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml position::tests`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/position.rs src-tauri/src/main.rs
git commit -m "feat: add popup position clamping logic"
```

---

### Task 5: Clipboard read/write-back

**Files:**
- Create: `src-tauri/src/clipboard_io.rs`
- Modify: `src-tauri/src/main.rs` (add `mod clipboard_io;`)

**Interfaces:**
- Consumes: `store::NewItem`, `store::HistoryItem` (Task 2).
- Produces (used by Tasks 6, 7): `pub fn is_excluded_from_history() -> bool`, `pub fn capture_current_clipboard() -> Option<store::NewItem>`, `pub fn write_item_to_clipboard(item: &store::HistoryItem) -> Result<(), String>`.

- [ ] **Step 1: Write a failing round-trip test for text**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::HistoryItem;

    #[test]
    fn write_then_capture_round_trips_text() {
        let item = HistoryItem {
            id: 1,
            kind: "text".into(),
            content: Some("clipboard round trip".into()),
            image_path: None,
            preview: "clipboard round trip".into(),
            created_at: 0,
        };
        write_item_to_clipboard(&item).unwrap();
        let captured = capture_current_clipboard().expect("expected a captured item");
        assert_eq!(captured.kind, "text");
        assert_eq!(captured.content.as_deref(), Some("clipboard round trip"));
    }

    #[test]
    fn text_over_cap_is_truncated_on_capture() {
        write_item_to_clipboard(&HistoryItem {
            id: 1,
            kind: "text".into(),
            content: Some("x".repeat(300_000)),
            image_path: None,
            preview: String::new(),
            created_at: 0,
        }).unwrap();
        let captured = capture_current_clipboard().expect("expected a captured item");
        assert!(captured.content.unwrap().len() <= 200_000);
    }
}
```

Note: these tests touch the real OS clipboard and must run single-threaded (`cargo test -- --test-threads=1`) to avoid clashing with other clipboard tests.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml clipboard_io::tests -- --test-threads=1`
Expected: FAIL — functions not defined.

- [ ] **Step 3: Implement `clipboard_io.rs`**

```rust
use crate::store::{HistoryItem, NewItem};

const TEXT_CAP_BYTES: usize = 200_000;
const IMAGE_MAX_DIMENSION: u32 = 1600;

pub fn is_excluded_from_history() -> bool {
    // Windows convention: apps (e.g. password managers) register a custom
    // clipboard format named "ExcludeClipboardContentFromMonitorProcessing"
    // to opt out of history tools. If present on the clipboard, skip capture.
    unsafe { crate::watcher::win32::clipboard_has_exclude_format() }
}

pub fn capture_current_clipboard() -> Option<NewItem> {
    if is_excluded_from_history() {
        return None;
    }

    if let Some(paths) = unsafe { crate::watcher::win32::read_hdrop() } {
        let joined = paths.join("\n");
        let preview = if paths.len() == 1 {
            paths[0].clone()
        } else {
            format!("{} files", paths.len())
        };
        let content = serde_json::to_string(&paths).ok()?;
        return Some(NewItem {
            kind: "files".into(),
            content: Some(content),
            image_path: None,
            preview,
            dedup_source: format!("files:{joined}"),
        });
    }

    let mut clipboard = arboard::Clipboard::new().ok()?;

    if let Ok(image) = clipboard.get_image() {
        let (w, h) = (image.width as u32, image.height as u32);
        let scale = (IMAGE_MAX_DIMENSION as f32 / w.max(h) as f32).min(1.0);
        let (out_w, out_h) = ((w as f32 * scale) as u32, (h as f32 * scale) as u32);
        let img_buf = image::RgbaImage::from_raw(w, h, image.bytes.into_owned())?;
        let resized = image::imageops::resize(&img_buf, out_w.max(1), out_h.max(1), image::imageops::FilterType::Triangle);
        let id = uuid::Uuid::new_v4();
        let dir = crate::watcher::images_dir();
        std::fs::create_dir_all(&dir).ok()?;
        let path = dir.join(format!("{id}.png"));
        resized.save(&path).ok()?;
        return Some(NewItem {
            kind: "image".into(),
            content: None,
            image_path: Some(path.to_string_lossy().to_string()),
            preview: format!("Image ({out_w}x{out_h})"),
            dedup_source: format!("image:{}", path.to_string_lossy()),
        });
    }

    if let Ok(text) = clipboard.get_text() {
        let truncated: String = text.chars().take(TEXT_CAP_BYTES).collect();
        let preview: String = truncated.chars().take(120).collect();
        return Some(NewItem {
            kind: "text".into(),
            content: Some(truncated.clone()),
            image_path: None,
            preview,
            dedup_source: format!("text:{truncated}"),
        });
    }

    None
}

pub fn write_item_to_clipboard(item: &HistoryItem) -> Result<(), String> {
    match item.kind.as_str() {
        "text" => {
            let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
            clipboard.set_text(item.content.clone().unwrap_or_default()).map_err(|e| e.to_string())
        }
        "image" => {
            let path = item.image_path.clone().ok_or("missing image_path")?;
            let img = image::open(&path).map_err(|e| e.to_string())?.to_rgba8();
            let (w, h) = img.dimensions();
            let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
            clipboard
                .set_image(arboard::ImageData { width: w as usize, height: h as usize, bytes: img.into_raw().into() })
                .map_err(|e| e.to_string())
        }
        "files" => {
            let paths: Vec<String> = serde_json::from_str(item.content.as_deref().unwrap_or("[]")).map_err(|e| e.to_string())?;
            unsafe { crate::watcher::win32::write_hdrop(&paths) }
        }
        other => Err(format!("unknown item kind: {other}")),
    }
}
```

Add `mod clipboard_io;` to `src-tauri/src/main.rs`. (`serde_json`, `image`, `uuid` added to `Cargo.toml` dependencies; `crate::watcher::win32` and `crate::watcher::images_dir` are implemented in Task 6.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml clipboard_io::tests -- --test-threads=1`
Expected: PASS (2 tests). Requires running on an actual Windows session (not headless CI without a desktop) since it touches the real clipboard.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/clipboard_io.rs src-tauri/src/main.rs src-tauri/Cargo.toml
git commit -m "feat: capture and write back clipboard content (text/image/files)"
```

---

### Task 6: Watcher thread (keyboard hook + clipboard listener)

**Files:**
- Create: `src-tauri/src/watcher.rs`
- Modify: `src-tauri/src/main.rs` (add `mod watcher;`, start the watcher thread on app setup)

**Interfaces:**
- Consumes: `hold_detector::HoldDetector` (Task 3), `clipboard_io::{is_excluded_from_history, capture_current_clipboard}` (Task 5), `store::HistoryStore` (Task 2).
- Produces (used by Task 7): `pub fn spawn(app_handle: tauri::AppHandle, store: std::sync::Arc<std::sync::Mutex<store::HistoryStore>>)` — starts the watcher; emits Tauri event `"show-popup"` with payload `{ x: i32, y: i32 }` on a 3s hold, and `"history-updated"` with no payload after every successful capture. Also exposes `pub fn images_dir() -> std::path::PathBuf` and a `pub mod win32` with `pub unsafe fn clipboard_has_exclude_format() -> bool`, `pub unsafe fn read_hdrop() -> Option<Vec<String>>`, `pub unsafe fn write_hdrop(paths: &[String]) -> Result<(), String>`, `pub unsafe fn cursor_position() -> (i32, i32)`.

This task is OS-hook integration and cannot be meaningfully unit-tested (there is no way to synthesize a real global low-level keyboard hook or a real `WM_CLIPBOARDUPDATE` in a test process — this is explicitly called out as manual-only in the spec's Testing section). It is verified by the manual checklist in Task 9. It composes the already-tested `HoldDetector` and `clamp_popup_position` rather than re-implementing their logic, so the risk surface here is limited to correct Win32 wiring.

- [ ] **Step 1: Implement the Win32 helpers**

```rust
pub mod win32 {
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::Win32::UI::Input::KeyboardAndMouse::{VK_C, VK_CONTROL};
    use windows::Win32::System::DataExchange::*;

    pub unsafe fn cursor_position() -> (i32, i32) {
        let mut pt = windows::Win32::Foundation::POINT::default();
        let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt);
        (pt.x, pt.y)
    }

    pub unsafe fn clipboard_has_exclude_format() -> bool {
        let format = RegisterClipboardFormatW(windows::core::w!("ExcludeClipboardContentFromMonitorProcessing"));
        IsClipboardFormatAvailable(format).is_ok()
    }

    pub unsafe fn read_hdrop() -> Option<Vec<String>> {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard};
        use windows::Win32::System::Ole::CF_HDROP;
        use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

        if IsClipboardFormatAvailable(CF_HDROP.0 as u32).is_err() {
            return None;
        }
        if OpenClipboard(HWND(0)).is_err() {
            return None;
        }
        let result = (|| {
            let handle = GetClipboardData(CF_HDROP.0 as u32).ok()?;
            let hdrop = HDROP(handle.0 as isize);
            let count = DragQueryFileW(hdrop, 0xFFFFFFFF, None);
            let mut paths = Vec::with_capacity(count as usize);
            for i in 0..count {
                let len = DragQueryFileW(hdrop, i, None) as usize;
                let mut buf = vec![0u16; len + 1];
                DragQueryFileW(hdrop, i, Some(&mut buf));
                paths.push(String::from_utf16_lossy(&buf[..len]));
            }
            Some(paths)
        })();
        let _ = CloseClipboard();
        result
    }

    pub unsafe fn write_hdrop(paths: &[String]) -> Result<(), String> {
        use windows::Win32::Foundation::{HANDLE, HWND};
        use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
        use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
        use windows::Win32::System::Ole::CF_HDROP;
        use windows::Win32::UI::Shell::DROPFILES;

        let mut wide: Vec<u16> = Vec::new();
        for p in paths {
            wide.extend(p.encode_utf16());
            wide.push(0);
        }
        wide.push(0); // extra terminating null for the double-null-terminated list

        let header_size = std::mem::size_of::<DROPFILES>();
        let total_size = header_size + wide.len() * 2;

        let hglobal = GlobalAlloc(GMEM_MOVEABLE, total_size).map_err(|e| e.to_string())?;
        let ptr = GlobalLock(hglobal) as *mut u8;
        if ptr.is_null() {
            return Err("GlobalLock failed".into());
        }
        let dropfiles = DROPFILES {
            pFiles: header_size as u32,
            pt: Default::default(),
            fNC: false.into(),
            fWide: true.into(),
        };
        std::ptr::copy_nonoverlapping(&dropfiles as *const DROPFILES as *const u8, ptr, header_size);
        std::ptr::copy_nonoverlapping(wide.as_ptr() as *const u8, ptr.add(header_size), wide.len() * 2);
        let _ = GlobalUnlock(hglobal);

        OpenClipboard(HWND(0)).map_err(|e| e.to_string())?;
        let _ = EmptyClipboard();
        let set_result = SetClipboardData(CF_HDROP.0 as u32, HANDLE(hglobal.0)).map_err(|e| e.to_string());
        let _ = CloseClipboard();
        set_result.map(|_| ())
    }
}
```

Verify manually per Task 9 (copy files in Explorer → confirm they show in history; select a `files` item → paste into a folder).

- [ ] **Step 2: Implement the hook + listener thread and `spawn`**

```rust
use crate::{clipboard_io, hold_detector::HoldDetector, position::clamp_popup_position, store::HistoryStore};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

pub fn images_dir() -> std::path::PathBuf {
    dirs::data_dir().unwrap().join("clipboard-manager").join("images")
}

pub fn spawn(app_handle: AppHandle, store: Arc<Mutex<HistoryStore>>) {
    let detector = Arc::new(Mutex::new(HoldDetector::new()));

    // Timer thread: polls the shared detector every 100ms and asks it to fire.
    {
        let detector = detector.clone();
        let app_handle = app_handle.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(100));
            let fired = detector.lock().unwrap().check(Instant::now(), Duration::from_secs(3));
            if fired {
                let (x, y) = unsafe { win32::cursor_position() };
                let (px, py) = clamp_popup_position((x, y), (260, 320), primary_screen_bounds());
                let _ = app_handle.emit("show-popup", serde_json::json!({ "x": px, "y": py }));
            }
        });
    }

    // Keyboard hook thread: installs WH_KEYBOARD_LL, feeds key state into `detector`.
    // Runs its own message loop (required for a low-level hook to receive callbacks).
    {
        let detector = detector.clone();
        std::thread::spawn(move || unsafe {
            install_keyboard_hook(detector);
        });
    }

    // Clipboard listener thread: hidden message-only window + AddClipboardFormatListener.
    {
        let app_handle = app_handle.clone();
        std::thread::spawn(move || unsafe {
            run_clipboard_listener(app_handle, store);
        });
    }
}

fn primary_screen_bounds() -> (i32, i32, i32, i32) {
    unsafe {
        let w = windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(windows::Win32::UI::WindowsAndMessaging::SM_CXSCREEN);
        let h = windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(windows::Win32::UI::WindowsAndMessaging::SM_CYSCREEN);
        (0, 0, w, h)
    }
}

unsafe fn install_keyboard_hook(detector: Arc<Mutex<HoldDetector>>) {
    // Stores `detector` in thread-local storage so the static HOOKPROC can reach it,
    // calls SetWindowsHookExW(WH_KEYBOARD_LL, hook_proc, ...), then runs a GetMessageW
    // loop to keep the thread (and hook) alive. hook_proc reads VK code + WM_KEYDOWN/UP,
    // updates detector.set_ctrl()/set_c() for VK_LCONTROL/VK_RCONTROL/VK_CONTROL and VK_C,
    // and always calls CallNextHookEx so key delivery is never blocked.
    HOOK_DETECTOR.with(|cell| *cell.borrow_mut() = Some(detector));
    let hook = windows::Win32::UI::WindowsAndMessaging::SetWindowsHookExW(
        windows::Win32::UI::WindowsAndMessaging::WH_KEYBOARD_LL,
        Some(hook_proc),
        None,
        0,
    ).expect("failed to install keyboard hook");
    let mut msg = windows::Win32::UI::WindowsAndMessaging::MSG::default();
    while windows::Win32::UI::WindowsAndMessaging::GetMessageW(&mut msg, None, 0, 0).into() {}
    let _ = windows::Win32::UI::WindowsAndMessaging::UnhookWindowsHookEx(hook);
}

thread_local! {
    static HOOK_DETECTOR: std::cell::RefCell<Option<Arc<Mutex<HoldDetector>>>> = std::cell::RefCell::new(None);
}

unsafe extern "system" fn hook_proc(
    code: i32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{KBDLLHOOKSTRUCT, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP};
    if code >= 0 {
        let data = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
        let up = matches!(wparam.0 as u32, WM_KEYUP | WM_SYSKEYUP);
        if down || up {
            HOOK_DETECTOR.with(|cell| {
                if let Some(detector) = cell.borrow().as_ref() {
                    let mut d = detector.lock().unwrap();
                    match data.vkCode {
                        0xA2 | 0xA3 | 0x11 => d.set_ctrl(down), // VK_LCONTROL, VK_RCONTROL, VK_CONTROL
                        0x43 => d.set_c(down),                   // VK_C
                        _ => {}
                    }
                }
            });
        }
    }
    windows::Win32::UI::WindowsAndMessaging::CallNextHookEx(None, code, wparam, lparam)
}

unsafe fn run_clipboard_listener(app_handle: AppHandle, store: Arc<Mutex<HistoryStore>>) {
    use windows::core::w;
    use windows::Win32::System::DataExchange::AddClipboardFormatListener;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DispatchMessageW, GetMessageW, RegisterClassW, TranslateMessage,
        HWND_MESSAGE, MSG, WNDCLASSW,
    };

    LISTENER_CTX.with(|cell| *cell.borrow_mut() = Some((app_handle.clone(), store.clone())));

    let class_name = w!("ClipboardManagerListenerWindow");
    let hinstance = GetModuleHandleW(None).unwrap();

    let wc = WNDCLASSW {
        lpfnWndProc: Some(listener_wndproc),
        hInstance: hinstance.into(),
        lpszClassName: class_name,
        ..Default::default()
    };
    RegisterClassW(&wc);

    let hwnd = CreateWindowExW(
        Default::default(),
        class_name,
        w!("ClipboardManagerListener"),
        Default::default(),
        0, 0, 0, 0,
        HWND_MESSAGE,
        None,
        hinstance,
        None,
    );

    let _ = AddClipboardFormatListener(hwnd);

    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).into() {
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

thread_local! {
    static LISTENER_CTX: std::cell::RefCell<Option<(AppHandle, Arc<Mutex<HistoryStore>>)>> = std::cell::RefCell::new(None);
}

unsafe extern "system" fn listener_wndproc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{DefWindowProcW, WM_CLIPBOARDUPDATE};

    if msg == WM_CLIPBOARDUPDATE {
        LISTENER_CTX.with(|cell| {
            if let Some((app_handle, store)) = cell.borrow().as_ref() {
                if let Some(item) = crate::clipboard_io::capture_current_clipboard() {
                    if store.lock().unwrap().capture(item).is_ok() {
                        let _ = app_handle.emit("history-updated", ());
                    }
                }
            }
        });
        return windows::Win32::Foundation::LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

- [ ] **Step 3: Wire `spawn` into `main.rs` app setup**

```rust
// in main.rs, inside tauri::Builder::default().setup(|app| { ... })
let store = std::sync::Arc::new(std::sync::Mutex::new(
    store::HistoryStore::open(&app.path().app_data_dir().unwrap().join("history.db")).unwrap(),
));
watcher::spawn(app.handle().clone(), store.clone());
app.manage(store);
```

- [ ] **Step 4: Manual verification**

Run: `cargo tauri dev`. Focus any text field, hold Ctrl+C for 3 seconds. Confirm: (a) a `show-popup` event fires (check via `console.log` temporarily in `popup.ts`, replaced by real handling in Task 8), (b) copying text elsewhere still works normally (hook isn't blocking keys), (c) copying a file in Explorer and a plain-text selection both produce a `history-updated` event.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/watcher.rs src-tauri/src/main.rs
git commit -m "feat: wire global hold-detection hook and clipboard-change listener"
```

---

### Task 7: Tauri commands, tray, and autostart

**Files:**
- Create: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/main.rs` (register commands, tray, autostart plugin)

**Interfaces:**
- Consumes: `store::HistoryStore` (Task 2), `clipboard_io::write_item_to_clipboard` (Task 5).
- Produces (used by Tasks 8, 9): Tauri commands `get_history() -> Vec<HistoryItemDto>`, `select_item(id: i64) -> Result<(), String>`, `delete_item(id: i64) -> Result<(), String>`, `get_settings() -> SettingsDto`, `set_settings(settings: SettingsDto) -> Result<(), String>`, where `SettingsDto { max_items: Option<i64>, retention_days: Option<i64>, start_with_windows: bool }`.

- [ ] **Step 1: Implement `commands.rs`**

```rust
use crate::store::HistoryStore;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};

#[derive(Serialize)]
pub struct HistoryItemDto {
    pub id: i64,
    pub kind: String,
    pub preview: String,
    pub created_at: i64,
}

#[derive(Serialize, Deserialize)]
pub struct SettingsDto {
    pub max_items: Option<i64>,
    pub retention_days: Option<i64>,
    pub start_with_windows: bool,
}

type Store = Arc<Mutex<HistoryStore>>;

#[tauri::command]
pub fn get_history(store: State<Store>) -> Result<Vec<HistoryItemDto>, String> {
    store
        .lock()
        .unwrap()
        .get_history()
        .map(|items| {
            items
                .into_iter()
                .map(|i| HistoryItemDto { id: i.id, kind: i.kind, preview: i.preview, created_at: i.created_at })
                .collect()
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn select_item(id: i64, store: State<Store>) -> Result<(), String> {
    let history = store.lock().unwrap().get_history().map_err(|e| e.to_string())?;
    let item = history.into_iter().find(|i| i.id == id).ok_or("item not found")?;
    crate::clipboard_io::write_item_to_clipboard(&item)
}

#[tauri::command]
pub fn delete_item(id: i64, store: State<Store>) -> Result<(), String> {
    let image_path = store.lock().unwrap().delete_item(id).map_err(|e| e.to_string())?;
    if let Some(path) = image_path {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

#[tauri::command]
pub fn get_settings(store: State<Store>) -> Result<SettingsDto, String> {
    let s = store.lock().unwrap();
    Ok(SettingsDto {
        max_items: s.get_setting("max_items").map_err(|e| e.to_string())?.and_then(|v| v.parse().ok()),
        retention_days: s.get_setting("retention_days").map_err(|e| e.to_string())?.and_then(|v| v.parse().ok()),
        start_with_windows: s.get_setting("start_with_windows").map_err(|e| e.to_string())?.map(|v| v == "true").unwrap_or(false),
    })
}

#[tauri::command]
pub fn set_settings(settings: SettingsDto, store: State<Store>, app: AppHandle) -> Result<(), String> {
    let s = store.lock().unwrap();
    if let Some(v) = settings.max_items {
        s.set_setting("max_items", &v.to_string()).map_err(|e| e.to_string())?;
    }
    if let Some(v) = settings.retention_days {
        s.set_setting("retention_days", &v.to_string()).map_err(|e| e.to_string())?;
    }
    s.set_setting("start_with_windows", if settings.start_with_windows { "true" } else { "false" }).map_err(|e| e.to_string())?;

    use tauri_plugin_autostart::ManagerExt;
    let autostart = app.autolaunch();
    if settings.start_with_windows {
        let _ = autostart.enable();
    } else {
        let _ = autostart.disable();
    }
    Ok(())
}
```

- [ ] **Step 2: Register commands, tray, and autostart plugin in `main.rs`**

```rust
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .invoke_handler(tauri::generate_handler![
            commands::get_history,
            commands::select_item,
            commands::delete_item,
            commands::get_settings,
            commands::set_settings,
        ])
        .setup(|app| {
            // store + watcher::spawn from Task 6 stay here

            let open_history = tauri::menu::MenuItemBuilder::with_id("open_history", "Open History").build(app)?;
            let settings = tauri::menu::MenuItemBuilder::with_id("settings", "Settings").build(app)?;
            let quit = tauri::menu::MenuItemBuilder::with_id("quit", "Quit").build(app)?;
            let menu = tauri::menu::MenuBuilder::new(app).items(&[&open_history, &settings, &quit]).build()?;

            let _tray = tauri::tray::TrayIconBuilder::new()
                .menu(&menu)
                .on_menu_event(move |app, event| match event.id().as_ref() {
                    "open_history" => {
                        if let Some(w) = app.get_webview_window("popup") { let _ = w.show(); }
                    }
                    "settings" => {
                        if let Some(w) = app.get_webview_window("settings") { let _ = w.show(); }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

Add `mod commands;` and add `tauri-plugin-autostart` to `src-tauri/Cargo.toml`. Add `"popup"` and `"settings"` window entries to `tauri.conf.json` (undecorated, transparent, `alwaysOnTop: true`, initially hidden, for the popup; normal decorated window for settings).

- [ ] **Step 3: Manual verification**

Run: `cargo tauri dev`. Right-click the tray icon; confirm Open History, Settings, and Quit all work. Toggle `start_with_windows` via a temporary manual `set_settings` call (real UI comes in Task 9) and confirm a Run-key entry appears under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/main.rs src-tauri/Cargo.toml src-tauri/tauri.conf.json
git commit -m "feat: add Tauri commands, tray menu, and autostart wiring"
```

---

### Task 8: Popup frontend

**Files:**
- Modify: `src/popup/popup.ts`, `src/popup/index.html`
- Create: `src/popup/popup.css`
- Create: `src/shared/bindings.ts`

**Interfaces:**
- Consumes: Tauri commands `get_history`, `select_item`, `delete_item` and events `show-popup`, `history-updated` (Task 7).

- [ ] **Step 1: Create typed bindings**

`src/shared/bindings.ts`:
```ts
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface HistoryItemDto {
  id: number;
  kind: "text" | "image" | "files";
  preview: string;
  created_at: number;
}

export const getHistory = () => invoke<HistoryItemDto[]>("get_history");
export const selectItem = (id: number) => invoke<void>("select_item", { id });
export const deleteItem = (id: number) => invoke<void>("delete_item", { id });
export const onShowPopup = (cb: (pos: { x: number; y: number }) => void) =>
  listen<{ x: number; y: number }>("show-popup", (e) => cb(e.payload));
export const onHistoryUpdated = (cb: () => void) => listen("history-updated", () => cb());
```

(Add `@tauri-apps/api` to a `package.json`/`import_map` resolvable by the Deno/esbuild build — via `deno.json` `imports` mapping to the npm package.)

- [ ] **Step 2: Implement `popup.ts`**

```ts
import { getHistory, selectItem, deleteItem, onShowPopup, onHistoryUpdated, type HistoryItemDto } from "../shared/bindings.ts";
import { getCurrentWindow } from "@tauri-apps/api/window";

const listEl = document.getElementById("list")!;
let items: HistoryItemDto[] = [];

function render() {
  listEl.innerHTML = "";
  items.forEach((item, i) => {
    const row = document.createElement("div");
    row.className = "row";

    const badge = document.createElement("span");
    badge.className = "badge";
    badge.textContent = i < 9 ? String(i + 1) : "";
    row.appendChild(badge);

    const preview = document.createElement("span");
    preview.className = "preview";
    preview.textContent = item.preview;
    row.appendChild(preview);

    const del = document.createElement("button");
    del.className = "delete";
    del.textContent = "×";
    del.onclick = async (e) => {
      e.stopPropagation();
      await deleteItem(item.id);
    };
    row.appendChild(del);

    row.onclick = async () => {
      await selectItem(item.id);
      await getCurrentWindow().hide();
    };

    listEl.appendChild(row);
  });
}

async function refresh() {
  items = await getHistory();
  render();
}

document.addEventListener("keydown", async (e) => {
  if (e.key === "Escape") {
    await getCurrentWindow().hide();
    return;
  }
  const n = Number(e.key);
  if (n >= 1 && n <= 9 && items[n - 1]) {
    await selectItem(items[n - 1].id);
    await getCurrentWindow().hide();
  }
});

window.addEventListener("blur", async () => {
  await getCurrentWindow().hide();
});

onShowPopup(async () => {
  await refresh();
  await getCurrentWindow().show();
  await getCurrentWindow().setFocus();
});

onHistoryUpdated(refresh);

refresh();
```

`src/popup/popup.css`:
```css
body { margin: 0; font-family: "Segoe UI", sans-serif; background: rgba(30,30,34,0.9); color: #eee; border-radius: 8px; overflow: hidden; }
#list { max-height: 320px; overflow-y: auto; }
.row { display: flex; align-items: center; gap: 8px; padding: 6px 10px; cursor: pointer; }
.row:hover { background: rgba(255,255,255,0.08); }
.row:hover .delete { visibility: visible; }
.badge { font-family: monospace; font-size: 11px; opacity: 0.6; width: 14px; }
.preview { flex: 1; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-size: 13px; }
.delete { visibility: hidden; background: none; border: none; color: #eee; cursor: pointer; font-size: 14px; }
```

`src/popup/index.html`:
```html
<!doctype html>
<html><head><meta charset="utf-8"><link rel="stylesheet" href="popup.css">
<script type="module" src="../../dist/popup/popup.js"></script></head>
<body><div id="list"></div></body></html>
```

- [ ] **Step 3: Build and manually verify**

Run: `deno task build && cargo tauri dev`. Hold Ctrl+C for 3s over any focused text field; confirm the popup appears at the cursor with history, `1`-`9` badges on the first nine rows, hover reveals `×`, clicking an item or pressing its digit sets the clipboard and closes the popup, Esc and clicking outside also close it.

- [ ] **Step 4: Commit**

```bash
git add src/popup src/shared/bindings.ts
git commit -m "feat: implement popup UI (history list, keybinds, delete)"
```

---

### Task 9: Settings frontend + end-to-end manual verification

**Files:**
- Modify: `src/settings/settings.ts`, `src/settings/index.html`
- Create: `src/settings/settings.css`

**Interfaces:**
- Consumes: `get_settings`, `set_settings` commands (Task 7).

- [ ] **Step 1: Implement `settings.ts`**

```ts
import { invoke } from "@tauri-apps/api/core";

interface SettingsDto {
  max_items: number | null;
  retention_days: number | null;
  start_with_windows: boolean;
}

const maxItemsEl = document.getElementById("maxItems") as HTMLInputElement;
const retentionEl = document.getElementById("retentionDays") as HTMLInputElement;
const autostartEl = document.getElementById("startWithWindows") as HTMLInputElement;
const form = document.getElementById("form") as HTMLFormElement;
const status = document.getElementById("status")!;

async function load() {
  const settings = await invoke<SettingsDto>("get_settings");
  maxItemsEl.value = settings.max_items?.toString() ?? "";
  retentionEl.value = settings.retention_days?.toString() ?? "";
  autostartEl.checked = settings.start_with_windows;
}

form.addEventListener("submit", async (e) => {
  e.preventDefault();
  await invoke("set_settings", {
    settings: {
      max_items: maxItemsEl.value ? Number(maxItemsEl.value) : null,
      retention_days: retentionEl.value ? Number(retentionEl.value) : null,
      start_with_windows: autostartEl.checked,
    },
  });
  status.textContent = "Saved.";
  setTimeout(() => (status.textContent = ""), 1500);
});

load();
```

`src/settings/index.html`:
```html
<!doctype html>
<html><head><meta charset="utf-8"><link rel="stylesheet" href="settings.css">
<script type="module" src="../../dist/settings/settings.js"></script></head>
<body>
  <form id="form">
    <label>Max items <input id="maxItems" type="number" min="1"></label>
    <label>Retention (days) <input id="retentionDays" type="number" min="1"></label>
    <label><input id="startWithWindows" type="checkbox"> Start with Windows</label>
    <button type="submit">Save</button>
    <span id="status"></span>
  </form>
</body></html>
```

`src/settings/settings.css`:
```css
body { font-family: "Segoe UI", sans-serif; padding: 16px; }
form { display: flex; flex-direction: column; gap: 12px; max-width: 280px; }
label { display: flex; justify-content: space-between; align-items: center; gap: 8px; font-size: 13px; }
input[type="number"] { width: 80px; }
button { align-self: flex-start; }
#status { font-size: 12px; color: #2a7; }
```

- [ ] **Step 2: Build and verify settings round-trip**

Run: `deno task build && cargo tauri dev`. Open Settings from the tray, set max items to 5, retention to 7, enable "Start with Windows", save, close and reopen the window — confirm the saved values reload correctly.

- [ ] **Step 3: Release build check**

Run: `cargo build --manifest-path src-tauri/Cargo.toml --release`
Expected: builds successfully with no warnings about unused `unsafe` blocks or unreachable code.

- [ ] **Step 4: Full manual verification checklist**

Run `cargo tauri build` to produce a release binary and run it standalone (not under `cargo tauri dev`) for these checks, per the spec's Testing section:

- Hold Ctrl+C for 3s in Notepad → popup appears at the cursor with recent items.
- Copy text in another app while the hook is running → confirm the copy still works normally (hook doesn't block keys).
- Copy a file in Explorer → appears in history as a `files` item; selecting it and pasting into another folder copies the file.
- Copy an image (e.g. Snipping Tool) → appears as an `image` item with a reasonable preview size.
- Drag the popup-triggering cursor near a screen edge/corner (including on a secondary monitor if available) → popup stays fully on-screen.
- Copy the same text twice → history shows one row, bumped to the top, not two rows.
- Set `max_items` to 3 in Settings, copy 5 different things → only 3 remain.
- Copy something from a password manager that marks `CF_EXCLUDECLIPBOARDHISTORY` (or `ExcludeClipboardContentFromMonitorProcessing`) → confirm it does not appear in history.
- Enable "Start with Windows" → confirm a `Run` registry entry is created; disable → confirm it's removed.
- Restart the app → confirm prior history is still present (persisted to `history.db`).

- [ ] **Step 5: Commit**

```bash
git add src/settings
git commit -m "feat: implement settings UI and complete manual verification"
```
