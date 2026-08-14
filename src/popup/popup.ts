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
import { hexToRgba } from "../shared/color.ts";

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

const listEl = document.getElementById("list")!;
const searchEl = document.getElementById("search") as HTMLInputElement;
const clearSearchEl = document.getElementById("clear-search")!;
const countEl = document.getElementById("count")!;
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

function applyFilter() {
  const query = searchEl.value.trim().toLowerCase();
  filtered = query ? items.filter((item) => item.preview.toLowerCase().includes(query)) : items;
  selectedIndex = 0;
  clearSearchEl.classList.toggle("visible", searchEl.value.length > 0);
  countEl.textContent = query
    ? `${filtered.length} of ${items.length}`
    : items.length
    ? `${items.length}`
    : "";
  render();
}

searchEl.addEventListener("input", applyFilter);

function render() {
  listEl.innerHTML = "";
  if (filtered.length === 0) {
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

    const badge = document.createElement("span");
    badge.className = "badge";
    badge.textContent = i < 9 ? String(i + 1) : "";
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

    const del = document.createElement("button");
    del.className = "delete";
    del.textContent = "×";
    del.title = "Delete";
    del.onclick = async (e) => {
      e.stopPropagation();
      await deleteItem(item.id);
      // `delete_item` doesn't emit `history-updated` (that event only fires
      // on new clipboard captures), so refresh locally to drop the row now
      // instead of waiting for the next backend event.
      await refresh();
    };
    row.appendChild(del);

    row.onclick = async () => {
      await selectItem(item.id);
      await getCurrentWindow().hide();
    };

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
  listEl.children[selectedIndex]?.scrollIntoView({ block: "nearest" });
}

async function refresh() {
  items = await getHistory();
  applyFilter();
}

async function applyTheme() {
  const settings = await getSettings();
  const root = document.documentElement.style;
  root.setProperty("--popup-bg", hexToRgba(settings.popup_bg_color, settings.popup_opacity));
  // Fixed alpha, independent of the opacity slider (which only controls the
  // popup background) -- matches the original hardcoded hover look
  // (`rgba(255,255,255,0.08)`) when the accent color is left at its default.
  root.setProperty("--popup-accent-bg", hexToRgba(settings.popup_accent_color, 0.08));
  root.setProperty("--popup-accent-color", settings.popup_accent_color);
}

// Registered once at module load — not per-render — so we never accumulate
// duplicate listeners across refreshes.
document.addEventListener("keydown", async (e) => {
  if (e.key === "Escape") {
    // First Escape clears an active search (and keeps the popup open so the
    // user can keep browsing); only a second Escape (or Escape with no
    // search text) closes the popup.
    if (document.activeElement === searchEl && searchEl.value) {
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
  if (e.key === "Enter") {
    if (!filtered[selectedIndex]) return;
    await selectItem(filtered[selectedIndex].id);
    await getCurrentWindow().hide();
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
    await deleteItem(filtered[selectedIndex].id);
    await refresh();
    return;
  }
  // Digit shortcuts only select a row when the search box isn't focused --
  // otherwise typing "1" while searching would select an item instead of
  // filtering.
  if (document.activeElement === searchEl) return;
  const n = Number(e.key);
  if (Number.isInteger(n) && n >= 1 && n <= 9 && filtered[n - 1]) {
    await selectItem(filtered[n - 1].id);
    await getCurrentWindow().hide();
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

onHistoryUpdated(refresh);
onSettingsUpdated(applyTheme);

applyTheme();
refresh();
