import { invoke } from "@tauri-apps/api/core";
import {
  getUpdateStatus,
  checkForUpdates,
  installUpdate,
  onWindowFocusChanged,
  type UpdateStatusDto,
} from "../shared/bindings.ts";

interface SettingsDto {
  max_items: number | null;
  retention_days: number | null;
  start_with_windows: boolean;
}

const maxItemsEl = document.getElementById("maxItems") as HTMLInputElement;
const retentionEl = document.getElementById("retentionDays") as HTMLInputElement;
const autostartEl = document.getElementById("startWithWindows") as HTMLInputElement;
const form = document.getElementById("form") as HTMLFormElement;
const status = document.getElementById("status")!;
const updateBannerEl = document.getElementById("updateBanner")!;
const updateTextEl = document.getElementById("updateText")!;
const updateActionEl = document.getElementById("updateAction") as HTMLButtonElement;

async function load() {
  const settings = await invoke<SettingsDto>("get_settings");
  maxItemsEl.value = settings.max_items?.toString() ?? "";
  retentionEl.value = settings.retention_days?.toString() ?? "";
  autostartEl.checked = settings.start_with_windows;
}

form.addEventListener("submit", async (e) => {
  e.preventDefault();
  await invoke("set_settings", {
    settings: {
      max_items: maxItemsEl.value ? Number(maxItemsEl.value) : null,
      retention_days: retentionEl.value ? Number(retentionEl.value) : null,
      start_with_windows: autostartEl.checked,
    },
  });
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
