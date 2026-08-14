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
// Tracks Settings > Storage's "Sort by" mode so the per-row time label can
// reflect what the list is actually ordered by -- see the comment in
// render() below.
let sortMode: "last_copied" | "first_copied" | "most_copied" = "last_copied";

// Tracks whether the popup is currently meant to be open, updated
// synchronously (before any `await`) by every path that opens or closes the
// window. onTogglePopup relies on this instead of an async `win.isVisible()`
// query to sidestep a real race: a tray-icon click both fires this window's
// own `blur` listener (which hides it) and delivers a `toggle-popup` IPC
// event for the same click, and `isVisible()` could go either way depending
// on which one the webview processes first. See the `blurHideAt` guard below
// for the other half of that race.
let popupOpen = false;
// Timestamp of the most recent blur-triggered hide, used by onTogglePopup to
// tell "this toggle-popup event is the tray click that just blurred us" from
// "a genuinely new toggle request" (see comment there).
let blurHideAt = 0;

document.getElementById("close")!.addEventListener("click", async () => {
  popupOpen = false;
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
  popupOpen = false;
  await getCurrentWindow().hide();
}

function applyFilter(resetSelection = true, scroll = resetSelection) {
  clearTimeout(errorTimeout);
  errorEl.classList.remove("visible");
  // Capture the currently-selected item's id before rebuilding `filtered` so
  // a resetSelection=false refresh (e.g. a new capture landing at index 0
  // while the popup is open) can re-locate the same item by id instead of
  // reusing a stale numeric index that now points at a different row.
  const selectedId = !resetSelection ? filtered[selectedIndex]?.id : undefined;
  const query = searchEl.value.trim().toLowerCase();
  filtered = query
    ? items.filter(
        (item) => item.preview.toLowerCase().includes(query) || (item.search_text?.includes(query) ?? false),
      )
    : items;
  if (resetSelection) {
    selectedIndex = 0;
  } else if (selectedId !== undefined) {
    const byId = filtered.findIndex((item) => item.id === selectedId);
    selectedIndex = byId >= 0 ? byId : Math.min(selectedIndex, filtered.length - 1);
  }
  clearSearchEl.classList.toggle("visible", searchEl.value.length > 0);
  // Empty-history previously left #count blank, so the aria-live region
  // (iteration 22) never announced anything to screen reader users -- unlike
  // the "0 of N" case for a search with no matches, which is announced fine.
  // Check items.length first: with an empty history, typing a search query
  // previously announced "0 of 0" (read as "no results for your search"),
  // which misleads screen-reader users into thinking a retry might help when
  // there's actually no history to search at all.
  countEl.textContent = items.length === 0
    ? "No items"
    : query
    ? `${filtered.length} of ${items.length}`
    : `${items.length}`;
  render(scroll);
}

searchEl.addEventListener("input", () => applyFilter());

function render(scroll = true) {
  // Rebuilding the list below removes the focused node from the document if
  // focus was on a row's delete button (e.g. mid keyboard-driven cleanup),
  // which per DOM spec silently drops focus to <body> with no refocus. Track
  // *which row index* had focus (not just whether one did) so a rebuild
  // triggered by mouseenter changing selectedIndex to a different row (e.g.
  // the mouse passing over row 7 while row 3's delete button is keyboard-
  // focused) restores focus to the row that actually had it, instead of
  // silently redirecting keyboard focus -- and the next Enter/Space press --
  // to whichever row the mouse happens to be hovering.
  let focusedDeleteRowIndex = -1;
  if (listEl.contains(document.activeElement)) {
    const active = document.activeElement as HTMLElement;
    if (active.classList.contains("delete")) {
      const row = active.closest(".row");
      focusedDeleteRowIndex = row ? Array.from(listEl.children).indexOf(row) : -1;
    }
  }
  listEl.innerHTML = "";
  if (filtered.length === 0) {
    searchEl.removeAttribute("aria-activedescendant");
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = items.length === 0
      ? "Clipboard history is empty"
      : "No matches";
    listEl.appendChild(empty);
    if (focusedDeleteRowIndex !== -1) searchEl.focus();
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
      // but nothing previously explained what pressing it does. The shortcut
      // only fires when the search box isn't focused (see the keydown
      // handler below), so don't promise it while search has focus -- the
      // popup auto-focuses search on open, making that the common case.
      if (document.activeElement !== searchEl) {
        badge.title = `Press ${i + 1} to select`;
      }
    }
    row.appendChild(badge);

    const thumb = createThumbnail(item);
    if (thumb) row.appendChild(thumb);

    const preview = document.createElement("span");
    preview.className = "preview";
    preview.textContent = item.preview;
    // Preview text is clipped with an ellipsis in CSS -- surface the full
    // text on hover so long entries aren't otherwise unreadable. full_text
    // can be up to 200KB, so cap it here to avoid an unbounded native
    // tooltip on large captures.
    const fullText = item.full_text ?? item.preview;
    preview.title =
      fullText.length > 2000 ? `${fullText.slice(0, 2000)}…` : fullText;
    row.appendChild(preview);

    if (item.size) {
      const size = document.createElement("span");
      size.className = "size";
      size.textContent = item.size;
      row.appendChild(size);
    }

    // created_at is bumped to "now" on every re-copy (see store.rs), so it
    // reflects last-copy time, not original-capture time. When sorted by
    // "Time of first copy" the list order is pinned to first_copied_at and
    // doesn't move on re-copy -- showing created_at there would make an
    // item's age tick down on re-copy while its position in the list stays
    // put, contradicting what it's sorted by. Show whichever timestamp the
    // current sort mode actually orders by.
    const timeValue = sortMode === "first_copied" ? item.first_copied_at : item.created_at;
    const time = document.createElement("span");
    time.className = "time";
    time.textContent = formatRelativeTime(timeValue);
    time.title = `${sortMode === "first_copied" ? "First copied" : "Last copied"}: ${new Date(timeValue).toLocaleString()}`;
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
    // `.delete` is `visibility: hidden` at rest, only shown via :hover/.active/
    // :focus-visible (popup.css). A visibility:hidden element can't receive
    // focus at all (per DOM spec), so when this row is the target of the
    // focus-restore below but isn't the (possibly different) selectedIndex
    // row, mark it explicitly visible so the .focus() call two blocks down
    // actually lands instead of silently no-oping and dropping focus to
    // <body> -- see the focusedDeleteRowIndex tracking above.
    del.className = i === focusedDeleteRowIndex ? "delete focus-target" : "delete";
    del.textContent = "×";
    // Mirror the badge's "Press N to select" hover hint (above) so the Delete
    // key shortcut is discoverable from the row itself, not just the × click.
    del.title = "Delete (or press Del)";
    del.setAttribute("aria-label", `Delete "${item.preview.slice(0, 40)}"`);
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
    // Uses mousemove rather than mouseenter: render() rebuilds every row from
    // scratch on each call (including on ArrowUp/ArrowDown keypresses), and
    // browsers resynthesize mouseenter/:hover on the new element sitting
    // under a *stationary* cursor after a DOM change -- so with mouseenter, a
    // keyboard-driven render() while the mouse rests anywhere over the list
    // would immediately snap selectedIndex back to the hovered row, silently
    // overriding the arrow-key press. mousemove only fires on genuine pointer
    // movement, so it doesn't get triggered by the rebuild itself.
    row.onmousemove = () => {
      if (selectedIndex === i) return;
      selectedIndex = i;
      render();
    };

    listEl.appendChild(row);
  });
  searchEl.setAttribute("aria-activedescendant", `row-${selectedIndex}`);
  // Only scroll for renders that reflect a real selection change (initial
  // open, search, keyboard nav, click). A background history-updated refresh
  // keeps the same selected item (see applyFilter's id re-location above),
  // so scrolling here would yank the user's manual scroll position back to
  // the selected row even though nothing they're looking at moved -- the
  // same class of disruption already avoided for the .time refresh below.
  if (scroll) listEl.children[selectedIndex]?.scrollIntoView({ block: "nearest" });
  if (focusedDeleteRowIndex !== -1) {
    const restoreIndex = Math.min(focusedDeleteRowIndex, filtered.length - 1);
    listEl.children[restoreIndex]?.querySelector<HTMLButtonElement>(".delete")?.focus();
  }
}

// render()'s focus-restore logic (see the `focusedDeleteRowIndex` comment
// inside render()) is meant to keep Tab-focus on a row's delete button
// across a *mouse-hover*-triggered re-render (row.onmousemove), so a
// keyboard-focused button doesn't silently lose focus just because the
// mouse happened to be resting over a different row. But render() can't
// tell what triggered it, so without this, an explicit ArrowUp/ArrowDown/
// Home/End keypress while a delete button has Tab focus would re-trigger
// that same restore logic and pin focus to the *old* row's delete button --
// leaving the focus ring stuck behind while the visual selection (and
// scrollIntoView) moves to a different row. Blur first so these genuinely
// keyboard-driven selection changes don't fight the restore mechanism.
function blurFocusedDeleteButton() {
  const active = document.activeElement as HTMLElement | null;
  if (active?.classList.contains("delete")) active.blur();
}

// `resetSelection` is false for post-delete refreshes so removing an item
// in the middle of a long list keeps the selection near where it was,
// instead of jumping back to the top every time (render() still clamps
// selectedIndex to the new, shorter list's bounds).
//
// refresh() is called from several independent, overlappable triggers
// (delete, history-updated events, settings-updated, popup toggle) with no
// coordination between them -- two in-flight getHistory() calls can resolve
// out of order (e.g. a slow delete-triggered refresh resolving after a
// newer history-updated refresh already rendered), silently reverting the
// list to stale data. A monotonic sequence number lets a resolution detect
// it's been superseded and bail out instead of overwriting newer state.
let refreshSeq = 0;
async function refresh(resetSelection = true, scroll = resetSelection) {
  const seq = ++refreshSeq;
  const result = await getHistory();
  if (seq !== refreshSeq) return;
  items = result;
  applyFilter(resetSelection, scroll);
}

async function applyTheme() {
  const settings = await getSettings();
  sortMode = settings.sort_mode;
  const root = document.documentElement.style;
  root.setProperty("--popup-bg", hexToRgba(settings.popup_bg_color, settings.popup_opacity));
  // --popup-accent-bg/--popup-accent-color are set inline here unconditionally,
  // which means the var(--popup-accent-color, <fallback>) fallbacks used by
  // light-bg CSS rules (e.g. .badge in popup.css) never actually apply -- the
  // property is always defined. When the popup background and the accent
  // color share the same lightness (both light, e.g. white-on-white, or both
  // dark, e.g. near-black-on-near-black), a low-alpha tint of the accent
  // color on a same-lightness background is invisible or near-invisible,
  // wiping out the row hover/active highlight, the search-focus border, the
  // .badge/.copy-count text, and the focus-visible outline all at once. Fall
  // back to a tint contrasting with the *background* in that case instead of
  // relying on dead CSS fallbacks.
  const bgIsLight = isLightColor(settings.popup_bg_color);
  if (bgIsLight === isLightColor(settings.popup_accent_color)) {
    root.setProperty("--popup-accent-bg", bgIsLight ? "rgba(0,0,0,0.08)" : "rgba(255,255,255,0.08)");
    root.setProperty("--popup-accent-color", bgIsLight ? "rgba(0,0,0,0.55)" : "rgba(255,255,255,0.75)");
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

// Right-click anywhere in this window previously fell through to WebView2's
// default context menu (Reload / Inspect Element / etc.), which is
// meaningless -- and exposes devtools -- in this small borderless
// single-purpose popup. Iteration 82 suppressed this on rows only; every
// other surface (header, search box, empty-list area) was still exposed, so
// suppress it document-wide instead.
document.addEventListener("contextmenu", (e) => e.preventDefault());

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
    popupOpen = false;
    await getCurrentWindow().hide();
    return;
  }
  if (e.key === "ArrowDown") {
    e.preventDefault();
    if (filtered.length === 0) return;
    blurFocusedDeleteButton();
    selectedIndex = (selectedIndex + 1) % filtered.length;
    render();
    return;
  }
  if (e.key === "ArrowUp") {
    e.preventDefault();
    if (filtered.length === 0) return;
    blurFocusedDeleteButton();
    selectedIndex = (selectedIndex - 1 + filtered.length) % filtered.length;
    render();
    return;
  }
  // Home/End jump to the first/last row, mirroring the ArrowUp/ArrowDown
  // wraparound above -- useful once history grows past a handful of items.
  if (e.key === "Home") {
    if (
      document.activeElement === searchEl &&
      (searchEl.selectionStart !== 0 || searchEl.selectionEnd !== 0)
    )
      return;
    if (filtered.length === 0) return;
    e.preventDefault();
    blurFocusedDeleteButton();
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
    blurFocusedDeleteButton();
    selectedIndex = filtered.length - 1;
    render();
    return;
  }
  if (e.key === "Enter") {
    // While an IME composition is active (e.g. typing Japanese/Chinese/Korean
    // search text), the browser fires a keydown with key === "Enter" and
    // isComposing === true when the user presses Enter to confirm the
    // composed text -- that keystroke is meant to commit the IME candidate,
    // not to select-and-close the popup. Ignoring it here lets composition
    // finish normally instead of pasting the wrong item and closing the
    // popup mid-input.
    if (e.isComposing) return;
    // Tabbing can focus a row's delete button, or the header's Clear-search/
    // Close buttons, independently of selectedIndex (same hazard as the
    // Delete-key handler below) -- if we acted on filtered[selectedIndex]
    // here, pressing Enter on any of those would both paste the wrong
    // (selectedIndex) item AND trigger the focused button's own native
    // Enter-activation. Let any focused <button>'s own click handler own
    // Enter instead of double-acting.
    const active = document.activeElement as HTMLElement | null;
    if (active?.tagName === "BUTTON") return;
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
    // Tabbing focuses a row's delete button independently of selectedIndex
    // (which only tracks arrow-key/mouse-hover selection) -- deleting
    // filtered[selectedIndex] in that case would silently remove a
    // different row than the one the focus ring is actually on. Resolve
    // the target from the focused delete button when present, falling back
    // to selectedIndex otherwise (arrow-key/mouse-driven selection).
    let targetIndex = selectedIndex;
    const active = document.activeElement as HTMLElement | null;
    if (active?.classList.contains("delete")) {
      const row = active.closest(".row");
      const rowIndex = row ? Array.from(listEl.children).indexOf(row) : -1;
      if (rowIndex !== -1) targetIndex = rowIndex;
    }
    if (!filtered[targetIndex]) return;
    e.preventDefault();
    try {
      await deleteItem(filtered[targetIndex].id);
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
  if (!popupOpen) return;
  popupOpen = false;
  blurHideAt = Date.now();
  await getCurrentWindow().hide();
});

// Ctrl+Shift+V toggles: if the popup is already open, close it (same as
// Esc/click-outside); otherwise position it at the cursor and open it.
onTogglePopup(async (pos) => {
  const win = getCurrentWindow();
  // On Windows, isVisible() reports true even while minimized (e.g. after
  // Win+D "show desktop"), so without this check a minimized popup would
  // take the hide() branch below and never reappear -- same platform quirk
  // already fixed for the settings window's tray menu handler (main.rs).
  if (popupOpen && !(await win.isMinimized())) {
    popupOpen = false;
    await win.hide();
    return;
  }
  // A tray-icon click while the popup is already open first steals focus
  // (firing the `blur` handler above, which hides the popup and stamps
  // blurHideAt) and then delivers this same toggle-popup event for that
  // very click. Without this guard, popupOpen would already be false by
  // now and we'd fall through to the "reopen" logic below, turning a
  // single tray click meant to *close* the popup into a flash-close-then-
  // reopen instead.
  if (Date.now() - blurHideAt < 400) return;
  popupOpen = true;
  selectedIndex = 0;
  searchEl.value = "";
  await refresh();
  // `pos` is the already-clamped (screen-edge-aware) position computed in
  // Rust (position.rs, Task 4) — apply it directly, no re-clamping here.
  await win.setPosition(new PhysicalPosition(pos.x, pos.y));
  await win.unminimize();
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
    if (item && timeEl) {
      timeEl.textContent = formatRelativeTime(
        sortMode === "first_copied" ? item.first_copied_at : item.created_at,
      );
    }
  });
}, 30000);

// A new capture landing while the user is mid-browse shouldn't reset their
// keyboard selection/scroll position back to the top -- same reasoning as
// the post-delete and settings-updated refreshes below.
onHistoryUpdated(() => refresh(false));
// Settings changes (e.g. sort order, capture types) can change what
// get_history returns, not just theming -- re-fetch the list too, not just
// re-apply colors, so an already-open popup doesn't show stale ordering
// until the next capture event.
onSettingsUpdated(async () => {
  await applyTheme();
  await refresh(false);
});

// Sequenced (not fire-and-forget in parallel) so sortMode is populated by
// applyTheme() before refresh()'s initial render() reads it for the .time
// label -- otherwise the first paint could briefly use the wrong timestamp
// field if getHistory() resolved before getSettings().
applyTheme().then(() => refresh());
