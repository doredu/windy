import {
  getSettings,
  setSettings,
  getUpdateStatus,
  checkForUpdates,
  installUpdate,
  clearHistory,
  countHistory,
  onWindowFocusChanged,
  onCloseRequested,
  onQuitRequested,
  quitApp,
  type CaptureType,
  type SettingsDto,
  type UpdateStatusDto,
} from "../shared/bindings.ts";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getVersion } from "@tauri-apps/api/app";

const maxItemsEl = document.getElementById("maxItems") as HTMLInputElement;
const retentionEl = document.getElementById("retentionDays") as HTMLInputElement;
const autostartEl = document.getElementById("startWithWindows") as HTMLInputElement;
const hotkeyEl = document.getElementById("hotkey") as HTMLInputElement;
const hotkeyRecordEl = document.getElementById("hotkeyRecord") as HTMLButtonElement;
const hotkeyResetEl = document.getElementById("hotkeyReset") as HTMLButtonElement;
const hotkeyErrorEl = document.getElementById("hotkeyError")!;
const hotkeyInactiveWarningEl = document.getElementById("hotkeyInactiveWarning")!;
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
const resetMaxItemsEl = document.getElementById("resetMaxItems") as HTMLButtonElement;
const resetRetentionDaysEl = document.getElementById("resetRetentionDays") as HTMLButtonElement;
const popupPositionEl = document.getElementById("popupPosition") as HTMLSelectElement;
const popupPinEl = document.getElementById("popupPin") as HTMLSelectElement;
const popupPinHintEl = document.getElementById("popupPinHint") as HTMLSpanElement;
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

function updateCaptureError() {
  if (!captureErrorEl.textContent) return;
  captureErrorEl.textContent = Object.values(captureCheckboxes).some((c) => c.checked)
    ? ""
    : "Select at least one type to save to history";
}

for (const el of Object.values(captureCheckboxes)) {
  el.addEventListener("change", updateCaptureError);
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

// "Bottom-right corner" already anchors the popup to the screen's bottom
// edge, so position.rs's on-screen clamp forces both Pin options to the
// exact same y-coordinate for that mode -- Pin has no visible effect there.
// Disable it instead of leaving a dead control that appears to do something.
function updatePinAvailability() {
  const disabled = popupPositionEl.value === "bottom_right";
  popupPinEl.disabled = disabled;
  popupPinHintEl.textContent = disabled
    ? "Not used for Bottom-right corner: the popup always sits above the bottom edge there"
    : "Whether the popup opens below or above the chosen Position point";
}
popupPositionEl.addEventListener("change", updatePinAvailability);

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
const DEFAULT_HOTKEY = "Ctrl+Alt+V";
const DEFAULT_MAX_ITEMS = "200";
const DEFAULT_RETENTION_DAYS = "30";

resetOpacityEl.addEventListener("click", () => {
  if (opacityEl.value !== DEFAULT_POPUP_OPACITY) markDirty();
  opacityEl.value = DEFAULT_POPUP_OPACITY;
  opacityValueEl.textContent = `${opacityEl.value}%`;
});
resetBgColorEl.addEventListener("click", () => {
  if (bgColorEl.value !== DEFAULT_POPUP_BG_COLOR) markDirty();
  bgColorEl.value = DEFAULT_POPUP_BG_COLOR;
  bgColorValueEl.textContent = bgColorEl.value;
});
resetAccentColorEl.addEventListener("click", () => {
  if (accentColorEl.value !== DEFAULT_POPUP_ACCENT_COLOR) markDirty();
  accentColorEl.value = DEFAULT_POPUP_ACCENT_COLOR;
  accentColorValueEl.textContent = accentColorEl.value;
});
// Unlike the Appearance color/opacity fields, the hotkey has no native input
// to type a value into -- it's an opaque string set only via Record or here,
// so this is the only way back to the default without recording Ctrl+Alt+V
// by hand.
hotkeyResetEl.addEventListener("click", () => {
  // While recording, hotkeyEl.value holds the "Press keys…" placeholder, not
  // the real current hotkey -- compare against previousHotkey instead so
  // canceling a recording via Reset doesn't spuriously mark the form dirty.
  const currentValue = recording ? previousHotkey : hotkeyEl.value;
  if (recording) {
    stopRecording();
  }
  if (currentValue !== DEFAULT_HOTKEY) markDirty();
  hotkeyEl.value = DEFAULT_HOTKEY;
  hotkeyErrorEl.textContent = "";
});
// Unlike Appearance's Reset buttons, Max items/Retention (days) share a
// single storageErrorEl -- resetting one field to its (always-valid)
// default must not leave a stale error referring to the other field intact,
// nor should it leave the just-fixed error lingering if both are now valid.
// Re-deriving via storageValidationMessage() (rather than just clearing on
// isValidStorageField) also keeps the message accurate -- not stuck
// blaming the just-reset field -- if the *other* field is still invalid.
resetMaxItemsEl.addEventListener("click", () => {
  if (maxItemsEl.value !== DEFAULT_MAX_ITEMS) markDirty();
  maxItemsEl.value = DEFAULT_MAX_ITEMS;
  if (storageErrorEl.textContent) storageErrorEl.textContent = storageValidationMessage();
});
resetRetentionDaysEl.addEventListener("click", () => {
  if (retentionEl.value !== DEFAULT_RETENTION_DAYS) markDirty();
  retentionEl.value = DEFAULT_RETENTION_DAYS;
  if (storageErrorEl.textContent) storageErrorEl.textContent = storageValidationMessage();
});

// Mirrors the submit handler's ordered Max items / Retention checks below,
// returning the message for whichever field is currently invalid (Max items
// first, matching submit's order) or "" if both are valid.
function storageValidationMessage(): string {
  if (maxItemsEl.value && !Number.isInteger(Number(maxItemsEl.value))) {
    return "Max items must be blank (unlimited) or a whole number";
  }
  if (maxItemsEl.value && Number(maxItemsEl.value) < 1) {
    return "Max items must be blank (unlimited) or at least 1";
  }
  if (maxItemsEl.value && Number(maxItemsEl.value) > Number.MAX_SAFE_INTEGER) {
    return `Max items must be blank (unlimited) or no greater than ${Number.MAX_SAFE_INTEGER.toLocaleString()}`;
  }
  if (retentionEl.value && !Number.isInteger(Number(retentionEl.value))) {
    return "Retention (days) must be blank (unlimited) or a whole number";
  }
  if (retentionEl.value && Number(retentionEl.value) < 1) {
    return "Retention (days) must be blank (unlimited) or at least 1";
  }
  if (retentionEl.value && Number(retentionEl.value) > Number.MAX_SAFE_INTEGER) {
    return `Retention (days) must be blank (unlimited) or no greater than ${Number.MAX_SAFE_INTEGER.toLocaleString()}`;
  }
  return "";
}

// Both fields share a single error message. Previously this only ever
// *cleared* the message once both fields became valid -- so fixing the
// field the error was actually about, while the other field was
// independently invalid, left the error text stuck blaming the wrong field.
// Re-deriving the message from the current field values (rather than just
// checking validity) keeps it accurate as the user edits either field.
for (const el of [maxItemsEl, retentionEl]) {
  el.addEventListener("input", () => {
    if (storageErrorEl.textContent) {
      storageErrorEl.textContent = storageValidationMessage();
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
  // hotkey_active reflects whether RegisterHotKey actually succeeded for the
  // stored combo (crate::watcher::HotkeyHandle::is_active) -- e.g. another
  // running app already owns the exact same combo. Previously that failure
  // only logged to stderr, so the hotkey could silently never fire with zero
  // indication anywhere in the UI.
  hotkeyInactiveWarningEl.classList.toggle("hidden", settings.hotkey_active !== false);
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
  updatePinAvailability();
  clearHistoryOnQuitEl.checked = settings.clear_history_on_quit;
  clearClipboardOnQuitEl.checked = settings.clear_clipboard_on_quit;
  clearClipboardOnQuitEl.disabled = !settings.clear_history_on_quit;
  hotkeyErrorEl.textContent = "";
  captureErrorEl.textContent = "";
  storageErrorEl.textContent = "";
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
  hotkeyRecordEl.setAttribute("aria-pressed", "false");
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
  hotkeyRecordEl.setAttribute("aria-pressed", "true");
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

// Right-click previously fell through to WebView2's default context menu
// (Reload / Inspect Element / etc.), which is meaningless -- and exposes
// devtools -- in this small fixed-size single-purpose Settings window,
// matching the same suppression applied to the popup window.
document.addEventListener("contextmenu", (e) => e.preventDefault());

document.addEventListener("keydown", (e) => {
  if (!recording) return;
  // Let Tab/Shift+Tab move focus natively instead of swallowing them --
  // the focusin listener above already treats focus leaving the hotkey
  // control as an implicit cancel, so blocking Tab here would trap
  // keyboard users on this control with no way out except Escape.
  if (e.key === "Tab") return;
  e.preventDefault();

  if (e.key === "Escape") {
    hotkeyEl.value = previousHotkey;
    hotkeyErrorEl.textContent = "";
    stopRecording();
    // Without this, the window-level Escape-to-close listener below would
    // also see this same keydown event and, since `recording` is now false
    // (just cleared by stopRecording() above), fall through and close the
    // whole Settings window on the same keystroke meant only to cancel the
    // recording.
    e.stopImmediatePropagation();
    return;
  }
  if (["Control", "Alt", "Shift", "Meta"].includes(e.key)) return;
  // Holding Shift remaps e.key to the shifted symbol on a digit (Shift+1 ->
  // "!"), which the UI explicitly advertises as a valid combo ("Ctrl/Alt/
  // Shift" hint). Prefer the physical key from e.code for letters/digits so
  // Shift+<digit> resolves to the digit instead of failing validation on the
  // shifted symbol.
  const physicalMatch = /^(?:Key([A-Z])|Digit(\d))$/.exec(e.code);
  const keyChar = physicalMatch ? physicalMatch[1] ?? physicalMatch[2] : e.key;
  if (!/^[a-zA-Z0-9]$/.test(keyChar)) {
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
  parts.push(keyChar.toUpperCase());

  const newHotkey = parts.join("+");
  hotkeyEl.value = newHotkey;
  hotkeyErrorEl.textContent = "";
  stopRecording();
  // hotkeyEl.value is assigned programmatically above, which doesn't fire a
  // native `input` event, so the delegated dirty-tracking listener on the
  // form wouldn't otherwise notice this change. Only mark dirty if the
  // recorded combo actually differs from what was previously saved --
  // re-recording the same shortcut shouldn't trigger a spurious "Discard
  // unsaved changes?" prompt.
  if (newHotkey !== previousHotkey) markDirty();
});

// Shared by Escape and the native titlebar close button (main.rs emits
// `close-requested` instead of hiding directly so both paths get the same
// unsaved-changes protection -- previously only Escape checked `dirty`,
// so clicking the OS window's X silently discarded in-progress edits.
async function closeIfConfirmed() {
  if (dirty) {
    if (!confirm("Discard unsaved changes?")) return;
    // The Settings webview is never destroyed/reloaded when hidden (main.rs
    // just show()/set_focus()s the same window from the tray), so without
    // this the discarded field values -- and the dirty flag itself -- would
    // still be sitting in the DOM the next time the window is reopened,
    // silently reintroducing the "unsaved changes" state and blocking
    // refreshLiveState()'s live autostart/hotkey_active refresh forever.
    await load();
  }
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

// The tray's Quit item used to call app.exit(0) directly, bypassing this
// window entirely and silently discarding unsaved edits -- main.rs now
// defers to this same confirm() (instead of hiding, actually exit via
// quit_app once resolved) whenever Settings is open when Quit is chosen.
onQuitRequested(async () => {
  if (dirty && !confirm("Discard unsaved changes and quit?")) return;
  await quitApp();
});

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
    maxItemsEl.focus();
    return;
  }
  if (maxItemsEl.value && Number(maxItemsEl.value) < 1) {
    activateTab(storageTab, false);
    storageErrorEl.textContent = "Max items must be blank (unlimited) or at least 1";
    maxItemsEl.focus();
    return;
  }
  // A whole number that's still too large for the backend's i64 field would
  // otherwise pass Number.isInteger() here but fail Tauri's IPC deserialization
  // before set_settings's own field-specific error handling ever runs, surfacing
  // as a raw/generic IPC error instead of this friendly message.
  if (maxItemsEl.value && Number(maxItemsEl.value) > Number.MAX_SAFE_INTEGER) {
    activateTab(storageTab, false);
    storageErrorEl.textContent = `Max items must be blank (unlimited) or no greater than ${Number.MAX_SAFE_INTEGER.toLocaleString()}`;
    maxItemsEl.focus();
    return;
  }
  if (retentionEl.value && !Number.isInteger(Number(retentionEl.value))) {
    activateTab(storageTab, false);
    storageErrorEl.textContent = "Retention (days) must be blank (unlimited) or a whole number";
    retentionEl.focus();
    return;
  }
  if (retentionEl.value && Number(retentionEl.value) < 1) {
    activateTab(storageTab, false);
    storageErrorEl.textContent = "Retention (days) must be blank (unlimited) or at least 1";
    retentionEl.focus();
    return;
  }
  if (retentionEl.value && Number(retentionEl.value) > Number.MAX_SAFE_INTEGER) {
    activateTab(storageTab, false);
    storageErrorEl.textContent = `Retention (days) must be blank (unlimited) or no greater than ${Number.MAX_SAFE_INTEGER.toLocaleString()}`;
    retentionEl.focus();
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
    captureTextEl.focus();
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
  // A successful save means `hotkey.rebind()` (called server-side before the
  // setting is persisted) succeeded, so the hotkey is now definitely active
  // -- clear any stale warning from a previous broken combo without waiting
  // for the next full load().
  hotkeyInactiveWarningEl.classList.add("hidden");
  status.classList.add("visible");
  // Guard against a stale timeout from an earlier save hiding this run's
  // status message early if Save is clicked again within the display window.
  clearTimeout(statusTimeout);
  statusTimeout = setTimeout(() => status.classList.remove("visible"), 1500);
});

let clearHistoryStatusTimeout: number | undefined;

clearHistoryBtn.addEventListener("click", async () => {
  // Disable immediately (before the async count fetch below) so a rapid
  // double-click can't fire this handler twice and surface two overlapping
  // confirm() prompts -- every other async button handler in this file
  // disables itself as its first synchronous step for the same reason.
  clearHistoryBtn.disabled = true;
  // The confirm prompt previously gave no sense of scale ("all clipboard
  // history" could mean 2 items or 2,000) -- showing the actual count makes
  // this destructive action's impact concrete before the user commits to it.
  // Uses countHistory() (unfiltered row count), not getHistory().length,
  // since getHistory applies the retention-days cutoff while clearHistory()
  // deletes every row unconditionally -- using the filtered count would
  // understate how many items actually get deleted.
  let count: number | undefined;
  try {
    count = await countHistory();
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
    clearHistoryBtn.disabled = false;
    return;
  }
  if (!confirm(message)) {
    clearHistoryBtn.disabled = false;
    return;
  }
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

// Fetched once at startup; the "Up to date" status previously gave no way
// to see which version is actually installed short of digging through the
// installer/uninstall entry, which matters when reporting a bug.
let currentVersion: string | null = null;
getVersion()
  .then((v: string) => {
    currentVersion = v;
    if (updateTextEl.textContent === "Up to date") updateTextEl.textContent = `Up to date (v${v})`;
  })
  .catch(() => {});

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
    updateTextEl.textContent = currentVersion ? `Up to date (v${currentVersion})` : "Up to date";
    updateActionEl.textContent = "Check for updates";
    updateActionEl.onclick = manualCheck;
    updateBannerEl.classList.remove("hidden");
    updateNotesEl.classList.add("hidden");
  }
}

// getUpdateStatus()/checkForUpdates() are triggered from several independent,
// overlappable places (startup, window-focus, the manual "Check for updates"
// button) with no coordination between them -- two in-flight calls can
// resolve out of order and the later-issued-but-earlier-resolving one would
// silently overwrite a fresher result (e.g. hiding a just-found update behind
// a stale "Up to date"). Mirrors popup.ts's refreshSeq guard for the same bug
// class.
let updateCheckSeq = 0;

async function manualCheck() {
  updateActionEl.disabled = true;
  const previousLabel = updateActionEl.textContent;
  updateActionEl.textContent = "Checking…";
  const seq = ++updateCheckSeq;
  try {
    const status = await checkForUpdates();
    if (seq !== updateCheckSeq) return;
    renderUpdateStatus(status);
  } catch {
    if (seq !== updateCheckSeq) return;
    updateBannerEl.classList.add("error");
    updateTextEl.textContent = "Check failed — try again later";
    updateActionEl.textContent = previousLabel;
  } finally {
    if (seq === updateCheckSeq) updateActionEl.disabled = false;
  }
}

async function installNow() {
  // Installing an update calls app.request_restart() on the backend, which
  // exits the process immediately -- the same "silently discards in-progress
  // edits" hazard that Escape/titlebar-close/tray-Quit already guard against
  // via `dirty`, but this exit path had no check at all.
  if (dirty && !confirm("Discard unsaved changes and update now?")) return;
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
  const seq = ++updateCheckSeq;
  // Unlike manualCheck() (triggered by a button the user can retry), this
  // runs unattended on every startup and window-focus -- if getUpdateStatus()
  // rejects (e.g. a transient store/IO error) without this guard, the
  // `await` throws before renderUpdateStatus() ever runs, leaving
  // updateActionEl permanently blank with no onclick bound at all (it's
  // only ever populated inside renderUpdateStatus()) and the banner stuck
  // hidden -- since the Settings webview is never recreated, that broken,
  // inert state would persist for the rest of the session.
  try {
    const status = await getUpdateStatus();
    if (seq !== updateCheckSeq) return;
    renderUpdateStatus(status);
  } catch {
    if (seq !== updateCheckSeq) return;
    // updateActionEl starts as an empty <button> in index.html with no
    // onclick bound -- it's only ever populated inside renderUpdateStatus(),
    // which we're bailing out of here. Without this, a failure on the very
    // first call (before renderUpdateStatus() ever ran once) would leave the
    // button permanently blank and inert with nothing to click to retry.
    // Bind it to the same manual-check path the "Check for updates" button
    // normally uses, rather than surfacing an error banner for a check the
    // user never explicitly asked for.
    if (!updateActionEl.onclick) {
      updateActionEl.textContent = "Check for updates";
      updateActionEl.onclick = manualCheck;
      updateBannerEl.classList.remove("hidden");
    }
  }
}

load();
loadUpdateStatus();

// autostart/hotkey_active reflect live OS state (see load()'s comment), not just
// the last-saved DB value, but the Settings window is only hidden/shown again
// via the tray (never recreated), so load() never reran after the initial call --
// reopening a long-hidden window could show a stale "Start with Windows" checkbox
// or hide a now-relevant hotkey-inactive warning. Re-check on focus, like
// loadUpdateStatus() already does, skipping while the user has unsaved edits so
// we don't clobber in-progress form changes.
async function refreshLiveState() {
  if (dirty) return;
  const settings = await getSettings();
  autostartEl.checked = settings.start_with_windows;
  hotkeyInactiveWarningEl.classList.toggle("hidden", settings.hotkey_active !== false);
}

onWindowFocusChanged((focused) => {
  if (focused) {
    loadUpdateStatus();
    refreshLiveState();
  }
});
