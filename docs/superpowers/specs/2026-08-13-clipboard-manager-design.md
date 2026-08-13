# Windows Clipboard Manager — Design

Date: 2026-08-13

## Summary

A Windows background utility that tracks clipboard history (text, images,
files) and shows a small popup near the cursor when Ctrl+C is held for 3
seconds, letting the user browse and select a past clipboard item via mouse
or the `1`–`9` keys.

## Stack

- **Core**: Rust, built with [Tauri](https://tauri.app/).
- **Frontend**: HTML/CSS/TypeScript, built with Deno tooling, rendered in
  Tauri's webview (WebView2 on Windows).
- **Storage**: SQLite (`rusqlite`) + loose PNG files for images.
- **Windows APIs**: `windows-rs` for the low-level keyboard hook, clipboard
  format listener, cursor position, and clipboard read/write.

## Architecture

One Tauri binary with three logical parts:

1. **Native watcher thread** (Rust) — owns:
   - A `WH_KEYBOARD_LL` low-level keyboard hook. Tracks when Ctrl and C are
     both down; starts a timer on that state; if still held at 3000ms,
     emits a `show-popup` event carrying the current cursor position (via
     `GetCursorPos`, virtual-screen coordinates — multi-monitor safe by
     construction). The hook never blocks or consumes the keystrokes — it
     only observes — so normal copy behavior everywhere else is untouched.
   - A hidden message-only window registered with
     `AddClipboardFormatListener`. Receives `WM_CLIPBOARDUPDATE` the instant
     the clipboard changes — event-driven, no polling.

2. **Core/store** (Rust) — SQLite database at
   `%APPDATA%\clipboard-manager\history.db`; images written as PNG files to
   `%APPDATA%\clipboard-manager\images\<id>.png`. Owns capture, dedup,
   pruning, and deletion. Exposed to the frontend only through Tauri
   commands — the webview never touches the DB or filesystem directly:
   - `get_history()`
   - `select_item(id)`
   - `delete_item(id)`
   - `get_settings()` / `set_settings(...)`

3. **Webview UI** (Deno-built TS/HTML/CSS) — two windows:
   - **Popup**: undecorated, transparent background, always-on-top, sized
     to content, positioned at the cursor (clamped to stay on-screen).
     Listens for `show-popup` and `history-updated` events. Renders the
     history newest-first with `1`–`9` badges on the first nine rows.
     Closes on Esc, click-outside, or item selection.
   - **Settings**: separate plain window, opened from the tray menu. Fields:
     max items, retention days, start-with-Windows toggle.

4. **Tray icon** — menu: Open History, Settings, Quit. Autostart via
   `tauri-plugin-autostart` (standard Run registry key).

## Data Model

```sql
items(
  id INTEGER PRIMARY KEY,
  kind TEXT CHECK(kind IN ('text','image','files')),
  content TEXT,      -- text: the string itself; files: JSON array of paths; image: NULL
  image_path TEXT,   -- image: path to the stored PNG; else NULL
  preview TEXT,       -- short display string (truncated text, "N files", or "Image WxH")
  dedup_key TEXT,      -- hash of normalized content, indexed, used for bump-to-top
  created_at INTEGER   -- unix ms; updated (not re-inserted) on a repeat copy
)

settings(key TEXT PRIMARY KEY, value TEXT)
  -- keys: max_items, retention_days, start_with_windows
```

No pinning in v1 — cut during design as unrequested scope.

## Flows

**Capture**: clipboard changes → hidden window gets `WM_CLIPBOARDUPDATE` →
check `CF_EXCLUDECLIPBOARDHISTORY` (skip if a password manager marked it
sensitive) → detect format, priority `CF_HDROP` (files) > `CF_DIB`/PNG
(image) > `CF_UNICODETEXT` (text) → compute `dedup_key` → if a row with that
key exists, update its `created_at` (bump to top) instead of inserting a new
row → otherwise insert → prune rows beyond `max_items` or older than
`retention_days` → emit `history-updated` to the popup if it's open.

**Show popup**: hook fires at 3s hold → Rust reads cursor position → popup
window is created/moved/shown at that point → webview calls `get_history` →
renders the list.

**Select** (click or `1`–`9`): webview calls `select_item(id)` → Rust writes
that item back to the OS clipboard in its native format (text / PNG /
`CF_HDROP`) → popup closes. No auto-paste — the user pastes manually.

**Delete**: hover reveals a `×` on the row → click calls `delete_item(id)` →
row removed from the DB; associated image file (if any) deleted from disk →
`history-updated` re-emitted.

## Edge Cases & Error Handling

- **Hook install fails** (e.g. blocked by security software): app keeps
  running; tray icon shows a warning state; the popup just won't
  auto-appear. Settings window provides a manual "Open history" fallback so
  the app isn't otherwise unusable.
- **Large payloads**: text capped (e.g. 200KB) before storing so old huge
  entries don't bloat the DB; images downscaled to a max dimension before
  being saved as the stored PNG.
- **Rapid duplicate copies**: dedup comparison is an indexed key lookup, so
  bump-to-top stays cheap regardless of history size.
- **Popup near screen edge**: position is clamped/flipped (above/left of the
  cursor) so it always stays fully on-screen.
- **DB missing or corrupt on startup**: recreate an empty DB rather than
  crashing — history isn't critical data.
- **Multi-monitor**: `GetCursorPos` already returns virtual-screen
  coordinates, so popup placement needs no extra per-monitor handling.

## Testing

- **Rust unit tests**: dedup-key computation, prune-boundary logic
  (`max_items` / `retention_days`), format-priority selection.
- **Manual/integration checks** (hard to automate against real OS hooks):
  hold-to-show timing, hook not interfering with normal copy elsewhere,
  multi-monitor popup positioning, autostart registry entry, exclusion
  against a real password manager.
- No automated UI/e2e layer for v1 — single-user background utility, not
  worth the harness for this scope.

## Explicitly Out of Scope (v1)

- Pinning items.
- Auto-paste on selection (selection only sets the OS clipboard).
- Non-Windows platforms.
- Syncing history across machines.
