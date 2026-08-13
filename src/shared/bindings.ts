// Typed wrappers around Tauri invoke/listen calls. No UI logic here —
// consumers (popup.ts, settings.ts) own their own rendering/interaction.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface HistoryItemDto {
  id: number;
  kind: "text" | "image" | "files" | "richtext";
  preview: string;
  thumbnail: string | null;
  created_at: number;
}

export interface SettingsDto {
  max_items: number | null;
  retention_days: number | null;
  start_with_windows: boolean;
}

export const getHistory = () => invoke<HistoryItemDto[]>("get_history");
export const selectItem = (id: number) => invoke<void>("select_item", { id });
export const deleteItem = (id: number) => invoke<void>("delete_item", { id });
export const getSettings = () => invoke<SettingsDto>("get_settings");
export const setSettings = (settings: SettingsDto) => invoke<void>("set_settings", { settings });

export const onTogglePopup = (cb: (pos: { x: number; y: number }) => void) =>
  listen<{ x: number; y: number }>("toggle-popup", (e) => cb(e.payload));
export const onHistoryUpdated = (cb: () => void) => listen("history-updated", () => cb());
