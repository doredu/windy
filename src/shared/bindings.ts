// Typed wrappers around Tauri invoke/listen calls. No UI logic here —
// consumers (popup.ts, settings.ts) own their own rendering/interaction.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

export interface HistoryItemDto {
  id: number;
  kind: "text" | "image" | "files" | "richtext";
  preview: string;
  thumbnail: string | null;
  size: string | null;
  created_at: number;
  first_copied_at: number;
  copy_count: number;
  search_text: string | null;
}

export type SortMode = "last_copied" | "first_copied" | "most_copied";
export type CaptureType = "text" | "image" | "files" | "richtext";
export type PopupPosition = "cursor" | "window_center" | "screen_center" | "bottom_right";
export type PopupPin = "top" | "bottom";

export interface SettingsDto {
  max_items: number | null;
  retention_days: number | null;
  start_with_windows: boolean;
  hotkey: string;
  // Only meaningful on the response from `get_settings` -- reflects whether
  // the configured hotkey is actually registered with Windows right now, not
  // a persisted setting. `set_settings` ignores it, so it's optional here
  // rather than forcing every SettingsDto construction (e.g. the submit
  // handler) to fill in a meaningless value just to satisfy the type.
  hotkey_active?: boolean;
  auto_check_updates: boolean;
  sort_mode: SortMode;
  capture_types: CaptureType[];
  popup_opacity: number;
  popup_bg_color: string;
  popup_accent_color: string;
  popup_position: PopupPosition;
  popup_pin: PopupPin;
  clear_history_on_quit: boolean;
  clear_clipboard_on_quit: boolean;
}

export interface UpdateStatusDto {
  available: boolean;
  version: string | null;
  notes: string | null;
}

export const getHistory = () => invoke<HistoryItemDto[]>("get_history");
export const selectItem = (id: number) => invoke<void>("select_item", { id });
export const deleteItem = (id: number) => invoke<void>("delete_item", { id });
export const clearHistory = () => invoke<void>("clear_history");
export const getSettings = () => invoke<SettingsDto>("get_settings");
export const setSettings = (settings: SettingsDto) => invoke<void>("set_settings", { settings });
export const getUpdateStatus = () => invoke<UpdateStatusDto>("get_update_status");
export const checkForUpdates = () => invoke<UpdateStatusDto>("check_for_updates");
export const installUpdate = () => invoke<void>("install_update");
export const quitApp = () => invoke<void>("quit_app");

export const onTogglePopup = (cb: (pos: { x: number; y: number }) => void) =>
  listen<{ x: number; y: number }>("toggle-popup", (e) => cb(e.payload));
export const onHistoryUpdated = (cb: () => void) => listen("history-updated", () => cb());
export const onSettingsUpdated = (cb: () => void) => listen("settings-updated", () => cb());
export const onCloseRequested = (cb: () => void) => listen("close-requested", () => cb());
export const onQuitRequested = (cb: () => void) => listen("quit-requested", () => cb());
export const onWindowFocusChanged = (cb: (focused: boolean) => void) =>
  getCurrentWindow().onFocusChanged(({ payload }) => cb(payload));
