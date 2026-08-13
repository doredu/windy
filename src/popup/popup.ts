// Popup UI logic: renders clipboard history, handles selection (click or
// 1-9 digit keys), per-row delete, and closing on Esc/blur/click-outside.
// No command/event definitions here — those live in ../shared/bindings.ts.

import {
  getHistory,
  selectItem,
  deleteItem,
  onShowPopup,
  onHistoryUpdated,
  type HistoryItemDto,
} from "../shared/bindings.ts";
import { getCurrentWindow, PhysicalPosition } from "@tauri-apps/api/window";

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

onShowPopup(async (pos) => {
  await refresh();
  const win = getCurrentWindow();
  // `pos` is the already-clamped (screen-edge-aware) position computed in
  // Rust (position.rs, Task 4) — apply it directly, no re-clamping here.
  await win.setPosition(new PhysicalPosition(pos.x, pos.y));
  await win.show();
  await win.setFocus();
});

onHistoryUpdated(refresh);

refresh();
