# Thumbnails + Richtext Capture — Design

Date: 2026-08-13

## Summary

Two additions to the clipboard manager:

1. A new `richtext` item kind that captures HTML clipboard content
   (`CF_HTML`) separately from plain `text`, so formatting-rich copies
   (browsers, Word, Slack, etc.) round-trip through history with their
   formatting intact.
2. Row thumbnails in the popup: a real small preview image for `image`
   items, and a static type icon for `files`/`richtext` items. `text` items
   keep their existing text-only preview (no thumbnail).

This builds on the existing capture pipeline in `clipboard_io.rs`,
`store.rs`, `commands.rs`, and `popup.ts` — no new subsystems, just new
branches/fields in the existing flow.

## Capture priority

Current: `image (CF_DIB) > files (CF_HDROP) > text (CF_UNICODETEXT)`.

New: `image > files > richtext (CF_HTML) > text`.

Rationale: images and file drops are unambiguous and take precedence as
today. Richtext is checked before plain text because most rich sources
(browsers, Office, Slack) place both `CF_HTML` and `CF_UNICODETEXT` on the
clipboard simultaneously — preferring richtext means formatting is kept
whenever it's actually available, and plain `text` capture becomes the
fallback for sources that truly only offer plain text (terminals, most
code editors, etc.).

`arboard`'s `Clipboard::get().html()` already reads `CF_HTML` on Windows
(confirmed in `arboard` 3.6.1's Windows backend), so no new Win32 FFI is
needed for the read side; `Clipboard::set().html(html, alt_text)` handles
write-back the same way.

## Data model changes

`items` table gains two nullable columns (added directly to the
`CREATE TABLE` statement — no existing history.db needs to survive this
change, confirmed with the user):

```sql
items(
  ...,               -- existing columns unchanged
  content_alt TEXT,  -- richtext only: plain-text fallback (preview basis,
                      -- dedup source component, and alt_text for write-back)
  thumb_path TEXT     -- image only: path to a small (40px max dimension)
                       -- content-addressed thumbnail PNG, alongside the
                       -- existing capped-1600px full-size image_path
)
```

`NewItem` / `HistoryItem` (`store.rs`) gain matching `content_alt: Option<String>`
and `thumb_path: Option<String>` fields, threaded through `capture()` and
`get_history()` the same way `content`/`image_path` already are.

## Capture flow (`clipboard_io.rs`)

**Richtext:**
- `clipboard.get().html()` succeeds and is non-empty →
  - `content` = HTML, truncated to the existing `TEXT_CAP_BYTES` (200,000
    bytes) cap via the existing `truncate_to_byte_cap` helper.
  - `content_alt` = `clipboard.get().text()` if that also succeeds;
    otherwise a naive tag-strip of the HTML (`Regex`-free — a simple
    manual scan removing `<...>` sequences) as a last-resort fallback.
  - `preview` = first 120 chars of `content_alt`, same as `text` today.
  - `dedup_source` = `format!("richtext:{content_alt_truncated}")` — keyed
    off the plain-text alternative (not the raw HTML) so trivial
    HTML-attribute differences between two copies of the same visible text
    don't produce spurious duplicate rows.
  - `kind` = `"richtext"`.

**Image thumbnail generation** (added inside the existing image branch,
after the full-size resize/hash/save):
- Resize the already-decoded image buffer down to a 40px max dimension
  (new constant `THUMBNAIL_MAX_DIMENSION`), same `Triangle` filter as the
  full-size resize.
- Content-addressed by the *same* hash as the full image (they're derived
  from the same source pixels): `<hash>_thumb.png`, written to the same
  `images_dir()`. Skipped if it already exists, mirroring the existing
  `path.exists()` check for the full image.
- `thumb_path` set to this new file's path on the returned `NewItem`.

## Serving thumbnails (`commands.rs`)

`get_history()`'s DTO mapping reads `thumb_path` (image items only), loads
the tiny PNG, and base64-encodes it into a `data:image/png;base64,...`
string on a new `thumbnail: Option<String>` DTO field. Files/richtext/text
items leave `thumbnail` as `None` — the frontend derives their icon purely
from `kind`, no backend data needed.

This read-and-encode happens on every `get_history()` call (popup
open/refresh), not just at capture time — but the source file is already a
40px PNG, so the decode+base64 cost per item is negligible even at a
few-hundred-item history.

## Write-back (`clipboard_io.rs::write_item_to_clipboard`)

New `"richtext"` arm: `clipboard.set().html(content, content_alt)`.

## Cleanup (`store.rs`)

`delete_item` and `prune`'s eviction paths currently fetch and remove
`image_path` for evicted rows. They're extended to also fetch and remove
`thumb_path` the same way, so thumbnails never orphan on disk after their
row (or the row's image) is gone.

## Frontend (`popup.ts` / `popup.css` / `bindings.ts`)

- `HistoryItemDto` gains `kind: "text" | "image" | "files" | "richtext"`
  and `thumbnail: string | null`.
- `render()` prepends a thumbnail slot to each row, before the preview
  text:
  - `kind === "image"` and `thumbnail` present → `<img>` with that data
    URL.
  - `kind === "files"` → static inline folder SVG icon.
  - `kind === "richtext"` → static inline document/formatting SVG icon.
  - `kind === "text"` → no thumbnail slot rendered (unchanged row layout).
- CSS: a `.thumb` class (image) and `.thumb-icon` class (SVG icons), fixed
  size (~18–20px) matching the row's existing compact height, `flex-shrink:
  0` so long preview text doesn't squeeze it.
- `preview.textContent` continues to show `item.preview` for all kinds
  unchanged (richtext's `preview` is already plain-text per the capture
  flow above, so no HTML ever reaches `textContent`).

## Config changes

- `tauri.conf.json`: CSP gains `img-src 'self' data:` (currently no
  `img-src` directive, so it falls back to `default-src 'self'`, which
  does not permit `data:` URIs — the base64 thumbnails would be silently
  blocked without this).
- `Cargo.toml`: new `base64` dependency for the encode step in
  `commands.rs`.

## Testing

- Rust unit tests (mirroring existing `clipboard_io.rs`/`store.rs` test
  patterns):
  - Richtext capture: HTML + plain-text both present → `kind ==
    "richtext"`, `content`/`content_alt` populated correctly.
  - Richtext capture: HTML present, plain-text absent → tag-strip fallback
    produces a non-empty `content_alt`.
  - Richtext write-back round-trip (write → capture → same HTML content).
  - Image thumbnail: `thumb_path` is set, file exists, is smaller than the
    full image.
  - `delete_item`/`prune` remove `thumb_path` files, not just
    `image_path`.
- No new frontend test harness (matches existing project convention — no
  automated UI/e2e layer per the original design doc); thumbnail rendering
  verified manually by running the app and copying an image, a file, rich
  HTML (e.g. from a browser), and plain text, then opening the popup.

## Explicitly out of scope

- Per-file-type icons for `files` items (single generic folder icon for
  all file drops, regardless of extension/count).
- Rendering actual HTML content for richtext thumbnails (static icon
  only, per the design decision above — avoids the sanitization surface
  of rendering clipboard-sourced markup).
- Migrating/preserving any existing `history.db` across this schema
  change.
