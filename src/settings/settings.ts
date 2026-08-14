import {
  getSettings,
  setSettings,
  getUpdateStatus,
  checkForUpdates,
  installUpdate,
  clearHistory,
  getHistory,
  onWindowFocusChanged,
  onCloseRequested,
  type CaptureType,
  type SettingsDto,
  type UpdateStatusDto,
} from "../shared/bindings.ts";
import { getCurrentWindow } from "@tauri-apps/api/window";

const maxItemsEl = document.getElementById("maxItems") as HTMLInputElement;
const retentionEl = document.getElementById("retentionDays") as HTMLInputElement;
const autostartEl = document.getElementById("startWithWindows") as HTMLInputElement;
const hotkeyEl = document.getElementById("hotkey") as HTMLInputElement;
const hotkeyRecordEl = document.getElementById("hotkeyRecord") as HTMLButtonElement;
const hotkeyErrorEl = document.getElementById("hotkeyError")!;
const captureErrorEl = document.getElementById("captureError")!;
const storageErrorEl = document.getElementById("storageError")!;
const autoCheckUpdatesEl = document.getElementById("autoCheckUpdates") as HTMLInputElement;
const sortModeEl = document.getElementById("sortMode") as HTMLSelectElement;
const captureTextEl = document.getElementById("captureText") as HTMLInputElement;
const captureImageEl = document.getElementById("captureImage") as HTMLInputElement;
const captureFilesEl = document.getElementById("captureFiles") as HTMLInputElement;
const captureRichtextEl = document.getElementById("captureRichtext") as HTMLInputElement;
const opacityEl = document.getElementById("popupOpacity") as HTMLInputElement;
const opacityValueEl = document.getElementById("popupOpacityValue")!;
const bgColorEl = document.getElementById("popupBgColor") as HTMLInputElement;
const bgColorValueEl = document.getElementById("popupBgColorValue")!;
const accentColorEl = document.getElementById("popupAccentColor") as HTMLInputElement;
const accentColorValueEl = document.getElementById("popupAccentColorValue")!;
const resetOpacityEl = document.getElementById("resetOpacity") as HTMLButtonElement;
const resetBgColorEl = document.getElementById("resetBgColor") as HTMLButtonElement;
const resetAccentColorEl = document.getElementById("resetAccentColor") as HTMLButtonElement;
const popupPositionEl = document.getElementById("popupPosition") as HTMLSelectElement;
const popupPinEl = document.getElementById("popupPin") as HTMLSelectElement;
const clearHistoryOnQuitEl = document.getElementById("clearHistoryOnQuit") as HTMLInputElement;
const clearClipboardOnQuitEl = document.getElementById("clearClipboardOnQuit") as HTMLInputElement;
const clearHistoryBtn = document.getElementById("clearHistoryBtn") as HTMLButtonElement;
const clearHistoryStatusEl = document.getElementById("clearHistoryStatus")!;
const form = document.getElementById("form") as HTMLFormElement;
const saveBtn = document.getElementById("saveBtn") as HTMLButtonElement;
const status = document.getElementById("status")!;
const unsavedIndicatorEl = document.getElementById("unsavedIndicator")!;
const updateBannerEl = document.getElementById("updateBanner")!;
const updateTextEl = document.getElementById("updateText")!;
const updateActionEl = document.getElementById("updateAction") as HTMLButtonElement;
const updateNotesEl = document.getElementById("updateNotes")!;

const captureCheckboxes: Record<CaptureType, HTMLInputElement> = {
  text: captureTextEl,
  image: captureImageEl,
  files: captureFilesEl,
  richtext: captureRichtextEl,
};

for (const el of Object.values(captureCheckboxes)) {
  el.addEventListener("change", () => {
    if (Object.values(captureCheckboxes).some((c) => c.checked)) {
      captureErrorEl.textContent = "";
    }
  });
}

const tabEls = Array.from(document.querySelectorAll<HTMLButtonElement>(".tab"));

function activateTab(tab: HTMLButtonElement, focus: boolean) {
  tabEls.forEach((t) => {
    t.classList.remove("active");
    t.setAttribute("aria-selected", "false");
    t.tabIndex = -1;
  });
  document.querySelectorAll(".tab-panel").forEach((p) => p.classList.remove("active"));
  tab.classList.add("active");
  tab.setAttribute("aria-selected", "true");
  tab.tabIndex = 0;
  document.querySelector(`.tab-panel[data-panel="${tab.dataset.tab}"]`)?.classList.add("active");
  if (focus) tab.focus();
}

tabEls.forEach((tab) => {
  tab.addEventListener("click", () => activateTab(tab, false));

  // Standard ARIA tabs keyboard pattern: Left/Right (and Home/End) move
  // focus and activate the tab, mirroring the roving-tabindex convention
  // screen reader users expect from role="tablist".
  tab.addEventListener("keydown", (e) => {
    let target: HTMLButtonElement | undefined;
    if (e.key === "ArrowRight") {
      target = tabEls[(tabEls.indexOf(tab) + 1) % tabEls.length];
    } else if (e.key === "ArrowLeft") {
      target = tabEls[(tabEls.indexOf(tab) - 1 + tabEls.length) % tabEls.length];
    } else if (e.key === "Home") {
      target = tabEls[0];
    } else if (e.key === "End") {
      target = tabEls[tabEls.length - 1];
    }
    if (!target) return;
    e.preventDefault();
    activateTab(target, true);
  });
});

clearHistoryOnQuitEl.addEventListener("change", () => {
  clearClipboardOnQuitEl.disabled = !clearHistoryOnQuitEl.checked;
  if (!clearHistoryOnQuitEl.checked) clearClipboardOnQuitEl.checked = false;
});

opacityEl.addEventListener("input", () => {
  opacityValueEl.textContent = `${opacityEl.value}%`;
});

// <input type="color"> only exposes its value via the swatch itself -- no
// hex text is shown anywhere, unlike the opacity slider's percentage
// readout above, so users can't read or share the exact color they picked.
bgColorEl.addEventListener("input", () => {
  bgColorValueEl.textContent = bgColorEl.value;
});
accentColorEl.addEventListener("input", () => {
  accentColorValueEl.textContent = accentColorEl.value;
});

// Matches the backend's built-in defaults (store.rs ensure_defaults) --
// once a user customizes these, there was previously no way back short of
// manually re-entering "90", "#1E1E22", "#FFFFFF" from memory.
const DEFAULT_POPUP_OPACITY = "90";
const DEFAULT_POPUP_BG_COLOR = "#1e1e22";
const DEFAULT_POPUP_ACCENT_COLOR = "#ffffff";

resetOpacityEl.addEventListener("click", () => {
  opacityEl.value = DEFAULT_POPUP_OPACITY;
  opacityValueEl.textContent = `${opacityEl.value}%`;
  markDirty();
});
resetBgColorEl.addEventListener("click", () => {
  bgColorEl.value = DEFAULT_POPUP_BG_COLOR;
  bgColorValueEl.textContent = bgColorEl.value;
  markDirty();
});
resetAccentColorEl.addEventListener("click", () => {
  accentColorEl.value = DEFAULT_POPUP_ACCENT_COLOR;
  accentColorValueEl.textContent = accentColorEl.value;
  markDirty();
});

for (const el of [maxItemsEl, retentionEl]) {
  el.addEventListener("input", () => {
    if (!el.value || (Number.isInteger(Number(el.value)) && Number(el.value) >= 1)) {
      storageErrorEl.textContent = "";
    }
  });
}

// Tracks whether any form field has changed since the last load()/successful
// save, so Escape (added in a prior iteration to close the window) can warn
// before silently discarding in-progress edits instead of just hiding.
let dirty = false;
function markDirty() {
  dirty = true;
  unsavedIndicatorEl.classList.add("visible");
}
function clearDirty() {
  dirty = false;
  unsavedIndicatorEl.classList.remove("visible");
}
form.addEventListener("input", markDirty);
form.addEventListener("change", markDirty);

async function load() {
  const settings = await getSettings();
  maxItemsEl.value = settings.max_items?.toString() ?? "";
  retentionEl.value = settings.retention_days?.toString() ?? "";
  autostartEl.checked = settings.start_with_windows;
  hotkeyEl.value = settings.hotkey;
  autoCheckUpdatesEl.checked = settings.auto_check_updates;
  sortModeEl.value = settings.sort_mode;
  for (const [kind, el] of Object.entries(captureCheckboxes)) {
    el.checked = settings.capture_types.includes(kind as CaptureType);
  }
  opacityEl.value = Math.round(settings.popup_opacity * 100).toString();
  opacityValueEl.textContent = `${opacityEl.value}%`;
  bgColorEl.value = settings.popup_bg_color;
  bgColorValueEl.textContent = settings.popup_bg_color;
  accentColorEl.value = settings.popup_accent_color;
  accentColorValueEl.textContent = settings.popup_accent_color;
  popupPositionEl.value = settings.popup_position;
  popupPinEl.value = settings.popup_pin;
  clearHistoryOnQuitEl.checked = settings.clear_history_on_quit;
  clearClipboardOnQuitEl.checked = settings.clear_clipboard_on_quit;
  clearClipboardOnQuitEl.disabled = !settings.clear_history_on_quit;
  clearDirty();
}

// Press-to-record hotkey capture: click Record, then press a modifier +
// single alnum key combo; it's captured on the first valid press. Escape
// cancels and restores the previous value.
let recording = false;
let previousHotkey = "";

function stopRecording() {
  recording = false;
  hotkeyEl.classList.remove("recording");
  hotkeyRecordEl.classList.remove("recording");
  hotkeyRecordEl.textContent = "Record";
}

hotkeyRecordEl.addEventListener("click", () => {
  if (recording) {
    stopRecording();
    hotkeyEl.value = previousHotkey;
    hotkeyErrorEl.textContent = "";
    return;
  }
  previousHotkey = hotkeyEl.value;
  recording = true;
  hotkeyErrorEl.textContent = "";
  hotkeyEl.classList.add("recording");
  hotkeyRecordEl.classList.add("recording");
  hotkeyRecordEl.textContent = "Cancel";
  hotkeyEl.value = "Press keys…";
});

// If recording is active and the user clicks into a different field instead
// of pressing Escape or Record/Cancel, the keydown handler below would keep
// swallowing every keystroke (it doesn't check focus) -- silently blocking
// typing anywhere else in the form. Treat focus leaving the hotkey control
// as an implicit cancel, same as Escape.
document.addEventListener("focusin", (e) => {
  if (!recording) return;
  if (e.target === hotkeyEl || e.target === hotkeyRecordEl) return;
  hotkeyEl.value = previousHotkey;
  hotkeyErrorEl.textContent = "";
  stopRecording();
});

document.addEventListener("keydown", (e) => {
  if (!recording) return;
  e.preventDefault();

  if (e.key === "Escape") {
    hotkeyEl.value = previousHotkey;
    hotkeyErrorEl.textContent = "";
    stopRecording();
    return;
  }
  if (["Control", "Alt", "Shift", "Meta"].includes(e.key)) return;
  if (!/^[a-zA-Z0-9]$/.test(e.key)) {
    hotkeyErrorEl.textContent = `"${e.key}" isn't supported — use a letter or digit`;
    return;
  }
  if (!e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey) {
    hotkeyErrorEl.textContent = "Add a modifier (Ctrl/Alt/Shift)";
    return;
  }

  const parts: string[] = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  if (e.metaKey) parts.push("Win");
  parts.push(e.key.toUpperCase());

  hotkeyEl.value = parts.join("+");
  hotkeyErrorEl.textContent = "";
  stopRecording();
  // hotkeyEl.value is assigned programmatically above, which doesn't fire a
  // native `input` event, so the delegated dirty-tracking listener on the
  // form wouldn't otherwise notice this change.
  markDirty();
});

// Shared by Escape and the native titlebar close button (main.rs emits
// `close-requested` instead of hiding directly so both paths get the same
// unsaved-changes protection -- previously only Escape checked `dirty`,
// so clicking the OS window's X silently discarded in-progress edits.
async function closeIfConfirmed() {
  if (dirty && !confirm("Discard unsaved changes?")) return;
  await getCurrentWindow().hide();
}

// The Settings window is a decorated OS window, but it had no Escape
// shortcut, unlike the popup window which closes on Escape. Skip while a
// hotkey recording is in progress -- that state already consumes Escape
// itself (cancel recording) via the keydown listener above.
document.addEventListener("keydown", async (e) => {
  if (e.key !== "Escape" || recording) return;
  await closeIfConfirmed();
});

onCloseRequested(closeIfConfirmed);

let statusTimeout: number | undefined;

form.addEventListener("submit", async (e) => {
  e.preventDefault();
  hotkeyErrorEl.textContent = "";
  // If the user clicks Save while a hotkey recording is in progress (without
  // pressing a key or Escape first), hotkeyEl.value still holds the literal
  // "Press keys…" placeholder -- submitting that as the hotkey would send
  // garbage to the backend. Treat an in-flight recording as canceled instead.
  if (recording) {
    hotkeyEl.value = previousHotkey;
    stopRecording();
  }
  const storageTab = document.getElementById("tab-storage") as HTMLButtonElement;
  // The submit handler calls preventDefault() before any native constraint
  // validation runs, so the inputs' `min="1"` attribute never actually
  // blocks a save -- a user can type 0 or a negative number here, which
  // would make the next prune wipe out the whole history silently.
  if (maxItemsEl.value && !Number.isInteger(Number(maxItemsEl.value))) {
    activateTab(storageTab, false);
    storageErrorEl.textContent = "Max items must be blank (unlimited) or a whole number";
    return;
  }
  if (maxItemsEl.value && Number(maxItemsEl.value) < 1) {
    activateTab(storageTab, false);
    storageErrorEl.textContent = "Max items must be blank (unlimited) or at least 1";
    return;
  }
  if (retentionEl.value && !Number.isInteger(Number(retentionEl.value))) {
    activateTab(storageTab, false);
    storageErrorEl.textContent = "Retention (days) must be blank (unlimited) or a whole number";
    return;
  }
  if (retentionEl.value && Number(retentionEl.value) < 1) {
    activateTab(storageTab, false);
    storageErrorEl.textContent = "Retention (days) must be blank (unlimited) or at least 1";
    return;
  }
  storageErrorEl.textContent = "";
  const capture_types = (Object.entries(captureCheckboxes) as [CaptureType, HTMLInputElement][])
    .filter(([, el]) => el.checked)
    .map(([kind]) => kind);
  if (capture_types.length === 0) {
    // Saving with every capture type unchecked would silently stop the
    // clipboard watcher from recording anything at all, with no other
    // feedback -- block the save instead of letting that happen by accident.
    activateTab(storageTab, false);
    captureErrorEl.textContent = "Select at least one type to save to history";
    return;
  }
  captureErrorEl.textContent = "";
  const settings: SettingsDto = {
    max_items: maxItemsEl.value ? Number(maxItemsEl.value) : null,
    retention_days: retentionEl.value ? Number(retentionEl.value) : null,
    start_with_windows: autostartEl.checked,
    hotkey: hotkeyEl.value,
    auto_check_updates: autoCheckUpdatesEl.checked,
    sort_mode: sortModeEl.value as SettingsDto["sort_mode"],
    capture_types,
    popup_opacity: Number(opacityEl.value) / 100,
    popup_bg_color: bgColorEl.value,
    popup_accent_color: accentColorEl.value,
    popup_position: popupPositionEl.value as SettingsDto["popup_position"],
    popup_pin: popupPinEl.value as SettingsDto["popup_pin"],
    clear_history_on_quit: clearHistoryOnQuitEl.checked,
    clear_clipboard_on_quit: clearClipboardOnQuitEl.checked,
  };
  saveBtn.disabled = true;
  saveBtn.textContent = "Saving…";
  try {
    await setSettings(settings);
  } catch (err) {
    // #hotkeyError only lives in the General panel, which is display:none
    // when another tab is active -- without switching tabs, a save failure
    // triggered from Storage/Appearance/Advanced would set the error text
    // invisibly and the Save button would silently appear to do nothing.
    const generalTab = document.getElementById("tab-general") as HTMLButtonElement;
    activateTab(generalTab, false);
    hotkeyErrorEl.textContent = String(err);
    return;
  } finally {
    saveBtn.disabled = false;
    saveBtn.textContent = "Save";
  }
  clearDirty();
  status.classList.add("visible");
  // Guard against a stale timeout from an earlier save hiding this run's
  // status message early if Save is clicked again within the display window.
  clearTimeout(statusTimeout);
  statusTimeout = setTimeout(() => status.classList.remove("visible"), 1500);
});

let clearHistoryStatusTimeout: number | undefined;

clearHistoryBtn.addEventListener("click", async () => {
  // The confirm prompt previously gave no sense of scale ("all clipboard
  // history" could mean 2 items or 2,000) -- showing the actual count makes
  // this destructive action's impact concrete before the user commits to it.
  let count: number | undefined;
  try {
    count = (await getHistory()).length;
  } catch {
    // If the count fetch fails, fall back to the generic wording below
    // rather than blocking the clear action entirely.
  }
  const message = count === undefined
    ? "Delete all clipboard history? This can't be undone."
    : count === 0
    ? "Clipboard history is already empty."
    : `Delete all ${count} clipboard item${count === 1 ? "" : "s"}? This can't be undone.`;
  if (count === 0) {
    clearHistoryStatusEl.classList.remove("error");
    clearHistoryStatusEl.textContent = message;
    clearTimeout(clearHistoryStatusTimeout);
    clearHistoryStatusTimeout = setTimeout(() => (clearHistoryStatusEl.textContent = ""), 2000);
    return;
  }
  if (!confirm(message)) return;
  clearHistoryBtn.disabled = true;
  clearHistoryBtn.textContent = "Clearing…";
  try {
    await clearHistory();
    clearHistoryStatusEl.classList.remove("error");
    clearHistoryStatusEl.textContent = "History cleared";
  } catch (err) {
    clearHistoryStatusEl.classList.add("error");
    clearHistoryStatusEl.textContent = String(err);
  } finally {
    clearHistoryBtn.disabled = false;
    clearHistoryBtn.textContent = "Clear all history";
    // Guard against a stale timeout from an earlier click clearing this
    // run's status message early if the button is clicked again within 2s.
    clearTimeout(clearHistoryStatusTimeout);
    clearHistoryStatusTimeout = setTimeout(() => (clearHistoryStatusEl.textContent = ""), 2000);
  }
});

function renderUpdateStatus(status: UpdateStatusDto) {
  updateBannerEl.classList.remove("error");
  if (status.available) {
    updateTextEl.textContent = `Update available: v${status.version}`;
    updateActionEl.textContent = "Update";
    updateActionEl.onclick = installNow;
    updateBannerEl.classList.remove("hidden");
    // The updater fetches the release's notes/changelog (backend
    // UpdateStatusDto.notes) but nothing in the UI ever rendered it --
    // users had to click Update blind, with no way to see what changed.
    if (status.notes) {
      updateNotesEl.textContent = status.notes;
      updateNotesEl.classList.remove("hidden");
    } else {
      updateNotesEl.classList.add("hidden");
    }
  } else {
    updateTextEl.textContent = "Up to date";
    updateActionEl.textContent = "Check for updates";
    updateActionEl.onclick = manualCheck;
    updateBannerEl.classList.remove("hidden");
    updateNotesEl.classList.add("hidden");
  }
}

async function manualCheck() {
  updateActionEl.disabled = true;
  const previousLabel = updateActionEl.textContent;
  updateActionEl.textContent = "Checking…";
  try {
    const status = await checkForUpdates();
    renderUpdateStatus(status);
  } catch {
    updateBannerEl.classList.add("error");
    updateTextEl.textContent = "Check failed — try again later";
    updateActionEl.textContent = previousLabel;
  } finally {
    updateActionEl.disabled = false;
  }
}

async function installNow() {
  updateActionEl.disabled = true;
  updateActionEl.textContent = "Updating…";
  try {
    await installUpdate();
  } catch {
    updateBannerEl.classList.add("error");
    updateTextEl.textContent = "Update failed — try again later";
    updateActionEl.textContent = "Retry";
    updateActionEl.disabled = false;
    updateActionEl.onclick = installNow;
  }
}

async function loadUpdateStatus() {
  const status = await getUpdateStatus();
  renderUpdateStatus(status);
}

load();
loadUpdateStatus();

onWindowFocusChanged((focused) => {
  if (focused) loadUpdateStatus();
});
