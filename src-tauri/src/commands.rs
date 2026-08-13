// Thin Tauri command handlers exposing `store::HistoryStore` and
// `clipboard_io` to the frontend. No business logic lives here — it all
// delegates to `store.rs` / `clipboard_io.rs`.

use crate::store::HistoryStore;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, PoisonError};
use tauri::{AppHandle, State};

#[derive(Serialize)]
pub struct HistoryItemDto {
    pub id: i64,
    pub kind: String,
    pub preview: String,
    pub created_at: i64,
}

#[derive(Serialize, Deserialize)]
pub struct SettingsDto {
    pub max_items: Option<i64>,
    pub retention_days: Option<i64>,
    pub start_with_windows: bool,
}

pub type Store = Arc<Mutex<HistoryStore>>;

#[tauri::command]
pub fn get_history(store: State<Store>) -> Result<Vec<HistoryItemDto>, String> {
    store
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_history()
        .map(|items| {
            items
                .into_iter()
                .map(|i| HistoryItemDto {
                    id: i.id,
                    kind: i.kind,
                    preview: i.preview,
                    created_at: i.created_at,
                })
                .collect()
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn select_item(id: i64, store: State<Store>) -> Result<(), String> {
    let history = store.lock().unwrap_or_else(PoisonError::into_inner).get_history().map_err(|e| e.to_string())?;
    let item = history.into_iter().find(|i| i.id == id).ok_or("item not found")?;
    crate::clipboard_io::write_item_to_clipboard(&item)
}

#[tauri::command]
pub fn delete_item(id: i64, store: State<Store>) -> Result<(), String> {
    let paths = store.lock().unwrap_or_else(PoisonError::into_inner).delete_item(id).map_err(|e| e.to_string())?;
    for path in paths {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

#[tauri::command]
pub fn get_settings(store: State<Store>) -> Result<SettingsDto, String> {
    let s = store.lock().unwrap_or_else(PoisonError::into_inner);
    Ok(SettingsDto {
        max_items: s.get_setting("max_items").map_err(|e| e.to_string())?.and_then(|v| v.parse().ok()),
        retention_days: s.get_setting("retention_days").map_err(|e| e.to_string())?.and_then(|v| v.parse().ok()),
        start_with_windows: s
            .get_setting("start_with_windows")
            .map_err(|e| e.to_string())?
            .map(|v| v == "true")
            .unwrap_or(false),
    })
}

#[tauri::command]
pub fn set_settings(settings: SettingsDto, store: State<Store>, app: AppHandle) -> Result<(), String> {
    let s = store.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(v) = settings.max_items {
        s.set_setting("max_items", &v.to_string()).map_err(|e| e.to_string())?;
    }
    if let Some(v) = settings.retention_days {
        s.set_setting("retention_days", &v.to_string()).map_err(|e| e.to_string())?;
    }
    s.set_setting("start_with_windows", if settings.start_with_windows { "true" } else { "false" })
        .map_err(|e| e.to_string())?;
    drop(s);

    use tauri_plugin_autostart::ManagerExt;
    let autostart = app.autolaunch();
    if settings.start_with_windows {
        let _ = autostart.enable();
    } else {
        let _ = autostart.disable();
    }
    Ok(())
}
