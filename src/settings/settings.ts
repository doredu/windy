import {
  getSettings,
  setSettings,
  getUpdateStatus,
  checkForUpdates,
  installUpdate,
  onWindowFocusChanged,
  type CaptureType,
  type SettingsDto,
  type UpdateStatusDto,
} from "../shared/bindings.ts";

const maxItemsEl = document.getElementById("maxItems") as HTMLInputElement;
const retentionEl = document.getElementById("retentionDays") as HTMLInputElement;
const autostartEl = document.getElementById("startWithWindows") as HTMLInputElement;
const hotkeyEl = document.getElementById("hotkey") as HTMLInputElement;
const hotkeyRecordEl = document.getElementById("hotkeyRecord") as HTMLButtonElement;
const hotkeyErrorEl = document.getElementById("hotkeyError")!;
const autoCheckUpdatesEl = document.getElementById("autoCheckUpdates") as HTMLInputElement;
const sortModeEl = document.getElementById("sortMode") as HTMLSelectElement;
const captureTextEl = document.getElementById("captureText") as HTMLInputElement;
const captureImageEl = document.getElementById("captureImage") as HTMLInputElement;
const captureFilesEl = document.getElementById("captureFiles") as HTMLInputElement;
const captureRichtextEl = document.getElementById("captureRichtext") as HTMLInputElement;
const opacityEl = document.getElementById("popupOpacity") as HTMLInputElement;
const opacityValueEl = document.getElementById("popupOpacityValue")!;
const bgColorEl = document.getElementById("popupBgColor") as HTMLInputElement;
const accentColorEl = document.getElementById("popupAccentColor") as HTMLInputElement;
const popupPositionEl = document.getElementById("popupPosition") as HTMLSelectElement;
const popupPinEl = document.getElementById("popupPin") as HTMLSelectElement;
const clearHistoryOnQuitEl = document.getElementById("clearHistoryOnQuit") as HTMLInputElement;
const clearClipboardOnQuitEl = document.getElementById("clearClipboardOnQuit") as HTMLInputElement;
const form = document.getElementById("form") as HTMLFormElement;
const status = document.getElementById("status")!;
const updateBannerEl = document.getElementById("updateBanner")!;
const updateTextEl = document.getElementById("updateText")!;
const updateActionEl = document.getElementById("updateAction") as HTMLButtonElement;

const captureCheckboxes: Record<CaptureType, HTMLInputElement> = {
  text: captureTextEl,
  image: captureImageEl,
  files: captureFilesEl,
  richtext: captureRichtextEl,
};

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
  accentColorEl.value = settings.popup_accent_color;
  popupPositionEl.value = settings.popup_position;
  popupPinEl.value = settings.popup_pin;
  clearHistoryOnQuitEl.checked = settings.clear_history_on_quit;
  clearClipboardOnQuitEl.checked = settings.clear_clipboard_on_quit;
  clearClipboardOnQuitEl.disabled = !settings.clear_history_on_quit;
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

document.addEventListener("keydown", (e) => {
  if (!recording) return;
  e.preventDefault();

  if (e.key === "Escape") {
    hotkeyEl.value = previousHotkey;
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
});

form.addEventListener("submit", async (e) => {
  e.preventDefault();
  hotkeyErrorEl.textContent = "";
  const settings: SettingsDto = {
    max_items: maxItemsEl.value ? Number(maxItemsEl.value) : null,
    retention_days: retentionEl.value ? Number(retentionEl.value) : null,
    start_with_windows: autostartEl.checked,
    hotkey: hotkeyEl.value,
    auto_check_updates: autoCheckUpdatesEl.checked,
    sort_mode: sortModeEl.value as SettingsDto["sort_mode"],
    capture_types: (Object.entries(captureCheckboxes) as [CaptureType, HTMLInputElement][])
      .filter(([, el]) => el.checked)
      .map(([kind]) => kind),
    popup_opacity: Number(opacityEl.value) / 100,
    popup_bg_color: bgColorEl.value,
    popup_accent_color: accentColorEl.value,
    popup_position: popupPositionEl.value as SettingsDto["popup_position"],
    popup_pin: popupPinEl.value as SettingsDto["popup_pin"],
    clear_history_on_quit: clearHistoryOnQuitEl.checked,
    clear_clipboard_on_quit: clearClipboardOnQuitEl.checked,
  };
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
  }
  status.classList.add("visible");
  setTimeout(() => status.classList.remove("visible"), 1500);
});

function renderUpdateStatus(status: UpdateStatusDto) {
  updateBannerEl.classList.remove("error");
  if (status.available) {
    updateTextEl.textContent = `Update available: v${status.version}`;
    updateActionEl.textContent = "Update";
    updateActionEl.onclick = installNow;
    updateBannerEl.classList.remove("hidden");
  } else {
    updateTextEl.textContent = "Up to date";
    updateActionEl.textContent = "Check for updates";
    updateActionEl.onclick = manualCheck;
    updateBannerEl.classList.remove("hidden");
  }
}

async function manualCheck() {
  updateActionEl.disabled = true;
  try {
    const status = await checkForUpdates();
    renderUpdateStatus(status);
  } catch {
    updateBannerEl.classList.add("error");
    updateTextEl.textContent = "Check failed — try again later";
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
