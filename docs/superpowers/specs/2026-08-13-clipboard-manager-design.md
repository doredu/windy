# Windows Clipboard Manager — Design

Date: 2026-08-13

## Summary

A Windows background utility that tracks clipboard history (text, images,
files) and toggles a small popup near the cursor on a single press of
Ctrl+Alt+V, letting the user browse and select a past clipboard item via
mouse or the `1`–`9` keys.

**Revision note:** the original design used a 3-second Ctrl+C hold as the
trigger. That was replaced after implementation testing surfaced two real
problems: (1) holding Ctrl+C is exactly the terminal SIGINT combo, so a
held-down repeat could interrupt/kill running terminal processes; (2) a
hand-rolled `WH_KEYBOARD_LL` hook state machine for detecting a 3-key combo
(first Ctrl+Shift+V, then Ctrl+Alt+V) proved unreliable in practice —
Ctrl+Shift collided with Windows' input-language-switch hotkey on this
machine's locale, and even Ctrl+Alt exhibited intermittent missed key-up
delivery (Alt has special system-menu-tracking behavior in the low-level
input pipeline). The fix was to stop hand-rolling combo detection entirely
and use Windows' native `RegisterHotKey` API (`WM_HOTKEY`), the purpose-built
mechanism for exactly this scenario — a single press of Ctrl+Alt+V now
toggles the popup open/closed reliably, verified across repeated real-input
cycles.

## Stack

- **Core**: Rust, built with [Tauri](https://tauri.app/).
- **Frontend**: HTML/CSS/TypeScript, built with Deno tooling, rendered in
  Tauri's webview (WebView2 on Windows).
- **Storage**: SQLite (`rusqlite`) + loose PNG files for images.
- **Windows APIs**: `windows-rs` for `RegisterHotKey` (global toggle hotkey),
  clipboard format listener, cursor position, and clipboard read/write.

## Architecture

One Tauri binary with three logical parts:

1. **Native watcher thread** (Rust) — owns:
   - A `RegisterHotKey` registration for Ctrl+Alt+V (with `MOD_NOREPEAT`).
     Windows delivers a `WM_HOTKEY` message on each press; the handler emits
     a `toggle-popup` event carrying the current cursor position (via
     `GetCursorPos`, virtual-screen coordinates — multi-monitor safe by
     construction). `RegisterHotKey` only fires for the exact registered
     combo — it never intercepts or blocks any other keystroke, including
     Ctrl+C — so normal copy behavior everywhere else is untouched.
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

**Toggle popup**: Ctrl+Alt+V pressed → `WM_HOTKEY` fires → Rust reads cursor
position → if the popup is already visible, it's hidden; otherwise the
window is moved to that point and shown → webview calls `get_history` →
renders the list.

**Select** (click or `1`–`9`): webview calls `select_item(id)` → Rust writes
that item back to the OS clipboard in its native format (text / PNG /
`CF_HDROP`) → popup closes. No auto-paste — the user pastes manually.

**Delete**: hover reveals a `×` on the row → click calls `delete_item(id)` →
row removed from the DB; associated image file (if any) deleted from disk →
`history-updated` re-emitted.

## Edge Cases & Error Handling

- **Hotkey registration fails** (e.g. combo already claimed by another app,
  or blocked by security software): app keeps running; tray icon shows a
  warning state; the popup just won't respond to Ctrl+Alt+V. Tray menu's
  "Open History" item is the fallback so the app isn't otherwise unusable.
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
  Ctrl+Alt+V toggle-open/toggle-close reliability across repeated presses,
  multi-monitor popup positioning, autostart registry entry, exclusion
  against a real password manager.
- No automated UI/e2e layer for v1 — single-user background utility, not
  worth the harness for this scope.

## Explicitly Out of Scope (v1)

- Pinning items.
- Auto-paste on selection (selection only sets the OS clipboard).
- Non-Windows platforms.
- Syncing history across machines.
