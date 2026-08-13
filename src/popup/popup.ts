// Popup UI logic: renders clipboard history, handles selection (click or
// 1-9 digit keys), per-row delete, and closing on Esc/blur/click-outside.
// No command/event definitions here — those live in ../shared/bindings.ts.

import {
  getHistory,
  selectItem,
  deleteItem,
  onTogglePopup,
  onHistoryUpdated,
  type HistoryItemDto,
} from "../shared/bindings.ts";
import { getCurrentWindow, PhysicalPosition } from "@tauri-apps/api/window";

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

    const thumb = createThumbnail(item);
    if (thumb) row.appendChild(thumb);

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

    listEl.appendChild(row);
  });
}

async function refresh() {
  items = await getHistory();
  render();
}

// Registered once at module load — not per-render — so we never accumulate
// duplicate listeners across refreshes.
document.addEventListener("keydown", async (e) => {
  if (e.key === "Escape") {
    await getCurrentWindow().hide();
    return;
  }
  const n = Number(e.key);
  if (Number.isInteger(n) && n >= 1 && n <= 9 && items[n - 1]) {
    await selectItem(items[n - 1].id);
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
  await refresh();
  // `pos` is the already-clamped (screen-edge-aware) position computed in
  // Rust (position.rs, Task 4) — apply it directly, no re-clamping here.
  await win.setPosition(new PhysicalPosition(pos.x, pos.y));
  await win.show();
  await win.setFocus();
});

onHistoryUpdated(refresh);

refresh();
