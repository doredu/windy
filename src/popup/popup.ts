// Popup UI logic: renders clipboard history, handles selection (click or
// 1-9 digit keys), per-row delete, and closing on Esc/blur/click-outside.
// No command/event definitions here — those live in ../shared/bindings.ts.

import {
  getHistory,
  getSettings,
  selectItem,
  deleteItem,
  onTogglePopup,
  onHistoryUpdated,
  onSettingsUpdated,
  type HistoryItemDto,
} from "../shared/bindings.ts";
import { getCurrentWindow, PhysicalPosition } from "@tauri-apps/api/window";
import { hexToRgba, isLightColor } from "../shared/color.ts";

const FOLDER_ICON_SVG =
  `<svg viewBox="0 0 16 16" width="16" height="16" fill="currentColor"><path d="M1.5 3A1.5 1.5 0 0 1 3 1.5h3.172a1.5 1.5 0 0 1 1.06.44l1.329 1.328A.5.5 0 0 0 8.914 3H13A1.5 1.5 0 0 1 14.5 4.5v8A1.5 1.5 0 0 1 13 14H3a1.5 1.5 0 0 1-1.5-1.5v-9Z"/></svg>`;
const DOC_ICON_SVG =
  // The second path's fill is a "punch-out" of the document lines against
  // the outer shape, which is drawn in currentColor (light in the default
  // dark theme, dark when the user picks a light popup background --
  // see popup.ts applyTheme's light-bg toggle). A hardcoded dark fill here
  // would vanish against a currentColor shape that's also dark, so it uses
  // a themeable CSS variable instead (see popup.css --thumb-icon-line).
  `<svg viewBox="0 0 16 16" width="16" height="16" fill="currentColor"><path d="M4 1.5A1.5 1.5 0 0 0 2.5 3v10A1.5 1.5 0 0 0 4 14.5h8a1.5 1.5 0 0 0 1.5-1.5V5.621a1.5 1.5 0 0 0-.44-1.06L10.44 1.94A1.5 1.5 0 0 0 9.378 1.5H4Z"/><path fill="var(--thumb-icon-line, #1e1e22)" d="M5 6.5h6v1H5zM5 9h6v1H5z"/></svg>`;

// item.created_at is a unix ms timestamp (see src-tauri store.rs) that was
// fetched but never surfaced in the UI -- render it as a short relative age
// so users can tell how old a history entry is at a glance.
function formatRelativeTime(createdAtMs: number): string {
  const diffSec = Math.max(0, Math.round((Date.now() - createdAtMs) / 1000));
  if (diffSec < 5) return "now";
  if (diffSec < 60) return `${diffSec}s`;
  const diffMin = Math.round(diffSec / 60);
  if (diffMin < 60) return `${diffMin}m`;
  const diffHour = Math.round(diffMin / 60);
  if (diffHour < 24) return `${diffHour}h`;
  const diffDay = Math.round(diffHour / 24);
  return `${diffDay}d`;
}

function createThumbnail(item: HistoryItemDto): HTMLElement | null {
  if (item.kind === "image") {
    if (!item.thumbnail) return null;
    const img = document.createElement("img");
    img.className = "thumb";
    img.src = item.thumbnail;
    img.alt = "";
    return img;
  }
  if (item.kind === "files" || item.kind === "richtext") {
    const icon = document.createElement("span");
    icon.className = "thumb-icon";
    icon.innerHTML = item.kind === "files" ? FOLDER_ICON_SVG : DOC_ICON_SVG;
    // The folder/document icons convey the item's kind visually only --
    // unlike images (self-evident thumbnail) or text (plain preview text),
    // a first-time user has no textual confirmation of what "files" or
    // "richtext" actually means until they hover.
    icon.title = item.kind === "files" ? "Files" : "Rich text";
    return icon;
  }
  return null;
}

const listEl = document.getElementById("list")!;
const searchEl = document.getElementById("search") as HTMLInputElement;
const clearSearchEl = document.getElementById("clear-search")!;
const countEl = document.getElementById("count")!;
const errorEl = document.getElementById("error")!;
let items: HistoryItemDto[] = [];
let filtered: HistoryItemDto[] = [];
let selectedIndex = 0;

document.getElementById("close")!.addEventListener("click", async () => {
  await getCurrentWindow().hide();
});

clearSearchEl.addEventListener("click", () => {
  searchEl.value = "";
  applyFilter();
  searchEl.focus();
});

// `select_item`/`delete_item` can fail (e.g. the item was deleted
// concurrently, or the OS clipboard write failed) -- without surfacing
// that, the popup would just silently stay open (select) or appear to do
// nothing (delete), the same class of silent-failure bug fixed in Settings
// (save errors, capture-type validation).
let errorTimeout: number | undefined;

function showError(err: unknown) {
  errorEl.textContent = String(err);
  errorEl.classList.add("visible");
  // Errors previously only cleared on the next search/filter action -- if
  // the user doesn't touch the search box afterward, a stale failure
  // message could linger on screen indefinitely.
  clearTimeout(errorTimeout);
  errorTimeout = setTimeout(() => errorEl.classList.remove("visible"), 4000);
}

async function selectAndClose(id: number) {
  try {
    await selectItem(id);
  } catch (err) {
    showError(err);
    return;
  }
  await getCurrentWindow().hide();
}

function applyFilter(resetSelection = true) {
  clearTimeout(errorTimeout);
  errorEl.classList.remove("visible");
  const query = searchEl.value.trim().toLowerCase();
  filtered = query ? items.filter((item) => item.preview.toLowerCase().includes(query)) : items;
  if (resetSelection) selectedIndex = 0;
  clearSearchEl.classList.toggle("visible", searchEl.value.length > 0);
  countEl.textContent = query
    ? `${filtered.length} of ${items.length}`
    : items.length
    ? `${items.length}`
    : "";
  render();
}

searchEl.addEventListener("input", () => applyFilter());

function render() {
  listEl.innerHTML = "";
  if (filtered.length === 0) {
    searchEl.removeAttribute("aria-activedescendant");
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = items.length === 0
      ? "Clipboard history is empty"
      : "No matches";
    listEl.appendChild(empty);
    return;
  }
  if (selectedIndex >= filtered.length) selectedIndex = filtered.length - 1;
  if (selectedIndex < 0) selectedIndex = 0;
  filtered.forEach((item, i) => {
    const row = document.createElement("div");
    row.className = i === selectedIndex ? "row active" : "row";
    row.id = `row-${i}`;
    row.setAttribute("role", "option");
    row.setAttribute("aria-selected", i === selectedIndex ? "true" : "false");

    const badge = document.createElement("span");
    badge.className = "badge";
    if (i < 9) {
      badge.textContent = String(i + 1);
      // Surface the digit-key shortcut on hover -- the badge shows the key
      // but nothing previously explained what pressing it does.
      badge.title = `Press ${i + 1} to select`;
    }
    row.appendChild(badge);

    const thumb = createThumbnail(item);
    if (thumb) row.appendChild(thumb);

    const preview = document.createElement("span");
    preview.className = "preview";
    preview.textContent = item.preview;
    // Preview text is clipped with an ellipsis in CSS -- surface the full
    // text on hover so long entries aren't otherwise unreadable.
    preview.title = item.preview;
    row.appendChild(preview);

    if (item.size) {
      const size = document.createElement("span");
      size.className = "size";
      size.textContent = item.size;
      row.appendChild(size);
    }

    const time = document.createElement("span");
    time.className = "time";
    time.textContent = formatRelativeTime(item.created_at);
    time.title = new Date(item.created_at).toLocaleString();
    row.appendChild(time);

    // copy_count is only meaningful once an item has been copied more than
    // once -- surfacing it always would clutter every row for no benefit,
    // and it's the only visible signal for what "Sort by: Number of copies"
    // (Settings > Storage) is even sorting on, which was previously
    // invisible in the popup despite the backend already tracking it.
    if (item.copy_count > 1) {
      const copyCount = document.createElement("span");
      copyCount.className = "copy-count";
      copyCount.textContent = `×${item.copy_count}`;
      copyCount.title = `Copied ${item.copy_count} times`;
      row.appendChild(copyCount);
    }

    const del = document.createElement("button");
    del.className = "delete";
    del.textContent = "×";
    // Mirror the badge's "Press N to select" hover hint (above) so the Delete
    // key shortcut is discoverable from the row itself, not just the × click.
    del.title = "Delete (or press Del)";
    del.setAttribute("aria-label", "Delete");
    del.onclick = async (e) => {
      e.stopPropagation();
      try {
        await deleteItem(item.id);
      } catch (err) {
        showError(err);
        return;
      }
      // `delete_item` doesn't emit `history-updated` (that event only fires
      // on new clipboard captures), so refresh locally to drop the row now
      // instead of waiting for the next backend event.
      await refresh(false);
    };
    row.appendChild(del);

    row.onclick = () => selectAndClose(item.id);

    // Keep keyboard selection (used by Enter/scrollIntoView) in sync with
    // the mouse -- otherwise hovering a different row than the last
    // arrow-key selection and pressing Enter would select the wrong item.
    row.onmouseenter = () => {
      if (selectedIndex === i) return;
      selectedIndex = i;
      render();
    };

    listEl.appendChild(row);
  });
  searchEl.setAttribute("aria-activedescendant", `row-${selectedIndex}`);
  listEl.children[selectedIndex]?.scrollIntoView({ block: "nearest" });
}

// `resetSelection` is false for post-delete refreshes so removing an item
// in the middle of a long list keeps the selection near where it was,
// instead of jumping back to the top every time (render() still clamps
// selectedIndex to the new, shorter list's bounds).
async function refresh(resetSelection = true) {
  items = await getHistory();
  applyFilter(resetSelection);
}

async function applyTheme() {
  const settings = await getSettings();
  const root = document.documentElement.style;
  root.setProperty("--popup-bg", hexToRgba(settings.popup_bg_color, settings.popup_opacity));
  // --popup-accent-bg/--popup-accent-color are set inline here unconditionally,
  // which means the var(--popup-accent-color, <fallback>) fallbacks used by
  // light-bg CSS rules (e.g. .badge in popup.css) never actually apply -- the
  // property is always defined. When the popup background is light AND the
  // accent color is still its light default (#ffffff), a low-alpha white tint
  // on a light background is invisible, wiping out the row hover/active
  // highlight, the search-focus border, and the .badge accent color all at
  // once. Fall back to a dark tint in that case instead of relying on dead
  // CSS fallbacks.
  const bgIsLight = isLightColor(settings.popup_bg_color);
  if (bgIsLight && isLightColor(settings.popup_accent_color)) {
    root.setProperty("--popup-accent-bg", "rgba(0,0,0,0.08)");
    root.setProperty("--popup-accent-color", "rgba(0,0,0,0.55)");
  } else {
    // Fixed alpha, independent of the opacity slider (which only controls the
    // popup background) -- matches the original hardcoded hover look
    // (`rgba(255,255,255,0.08)`) when the accent color is left at its default.
    root.setProperty("--popup-accent-bg", hexToRgba(settings.popup_accent_color, 0.08));
    root.setProperty("--popup-accent-color", settings.popup_accent_color);
  }
  // Text/icon colors default to a light palette suited to the default dark
  // background -- flip to a dark palette when the user picks a light
  // background so text stays readable instead of light-on-light.
  document.documentElement.classList.toggle("light-bg", bgIsLight);
}

// Registered once at module load — not per-render — so we never accumulate
// duplicate listeners across refreshes.
document.addEventListener("keydown", async (e) => {
  if (e.key === "Escape") {
    // First Escape clears an active search (and keeps the popup open so the
    // user can keep browsing); only a second Escape (or Escape with no
    // search text) closes the popup. This applies regardless of which
    // control currently has focus (search box, close button, a row's
    // delete button, etc.) so Escape behaves consistently.
    if (searchEl.value) {
      searchEl.value = "";
      applyFilter();
      return;
    }
    await getCurrentWindow().hide();
    return;
  }
  if (e.key === "ArrowDown") {
    e.preventDefault();
    if (filtered.length === 0) return;
    selectedIndex = (selectedIndex + 1) % filtered.length;
    render();
    return;
  }
  if (e.key === "ArrowUp") {
    e.preventDefault();
    if (filtered.length === 0) return;
    selectedIndex = (selectedIndex - 1 + filtered.length) % filtered.length;
    render();
    return;
  }
  // Home/End jump to the first/last row, mirroring the ArrowUp/ArrowDown
  // wraparound above -- useful once history grows past a handful of items.
  if (e.key === "Home") {
    if (document.activeElement === searchEl && searchEl.selectionStart !== 0) return;
    if (filtered.length === 0) return;
    e.preventDefault();
    selectedIndex = 0;
    render();
    return;
  }
  if (e.key === "End") {
    if (
      document.activeElement === searchEl &&
      searchEl.selectionStart !== searchEl.value.length
    ) return;
    if (filtered.length === 0) return;
    e.preventDefault();
    selectedIndex = filtered.length - 1;
    render();
    return;
  }
  if (e.key === "Enter") {
    if (!filtered[selectedIndex]) return;
    await selectAndClose(filtered[selectedIndex].id);
    return;
  }
  // Delete key removes the selected row, mirroring the per-row × button, so
  // keyboard-only users aren't forced to reach for the mouse to clean up
  // history. Guarded on search focus so it doesn't hijack forward-delete
  // while editing the query.
  if (e.key === "Delete") {
    if (document.activeElement === searchEl) return;
    if (!filtered[selectedIndex]) return;
    e.preventDefault();
    try {
      await deleteItem(filtered[selectedIndex].id);
    } catch (err) {
      showError(err);
      return;
    }
    await refresh(false);
    return;
  }
  // Digit shortcuts only select a row when the search box isn't focused --
  // otherwise typing "1" while searching would select an item instead of
  // filtering.
  if (document.activeElement === searchEl) return;
  const n = Number(e.key);
  if (Number.isInteger(n) && n >= 1 && n <= 9 && filtered[n - 1]) {
    await selectAndClose(filtered[n - 1].id);
  }
});

// Losing focus (click-outside on Windows moves focus away from the
// undecorated popup webview) closes the popup, same as Esc.
window.addEventListener("blur", async () => {
  await getCurrentWindow().hide();
});

// Ctrl+Shift+V toggles: if the popup is already open, close it (same as
// Esc/click-outside); otherwise position it at the cursor and open it.
onTogglePopup(async (pos) => {
  const win = getCurrentWindow();
  if (await win.isVisible()) {
    await win.hide();
    return;
  }
  selectedIndex = 0;
  searchEl.value = "";
  await refresh();
  // `pos` is the already-clamped (screen-edge-aware) position computed in
  // Rust (position.rs, Task 4) — apply it directly, no re-clamping here.
  await win.setPosition(new PhysicalPosition(pos.x, pos.y));
  await win.show();
  await win.setFocus();
  searchEl.focus();
});

// Relative times ("now", "5s", "2m") are stamped into the DOM once at
// render() time and never revisited -- if the popup is left open (e.g. while
// searching or browsing), those labels silently go stale. Refresh just the
// .time text nodes in place rather than calling render(), which would also
// re-run scrollIntoView() and yank the user's manual scroll position back to
// the selected row.
setInterval(() => {
  const rows = listEl.querySelectorAll<HTMLElement>(".row");
  rows.forEach((row, i) => {
    const item = filtered[i];
    const timeEl = row.querySelector(".time");
    if (item && timeEl) timeEl.textContent = formatRelativeTime(item.created_at);
  });
}, 30000);

onHistoryUpdated(refresh);
// Settings changes (e.g. sort order, capture types) can change what
// get_history returns, not just theming -- re-fetch the list too, not just
// re-apply colors, so an already-open popup doesn't show stale ordering
// until the next capture event.
onSettingsUpdated(async () => {
  await applyTheme();
  await refresh(false);
});

applyTheme();
refresh();
