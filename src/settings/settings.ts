import { invoke } from "@tauri-apps/api/core";

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

load();
