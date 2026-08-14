// Thin Tauri command handlers exposing `store::HistoryStore` and
// `clipboard_io` to the frontend. No business logic lives here — it all
// delegates to `store.rs` / `clipboard_io.rs`.

use crate::store::HistoryStore;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, PoisonError};
use tauri::{AppHandle, Emitter, State};

#[derive(Serialize)]
pub struct HistoryItemDto {
    pub id: i64,
    pub kind: String,
    pub preview: String,
    pub thumbnail: Option<String>,
    pub size: Option<String>,
    pub created_at: i64,
    pub copy_count: i64,
}

#[derive(Serialize, Deserialize)]
pub struct SettingsDto {
    pub max_items: Option<i64>,
    pub retention_days: Option<i64>,
    pub start_with_windows: bool,
    pub hotkey: String,
    pub auto_check_updates: bool,
    pub sort_mode: String,
    pub capture_types: Vec<String>,
    pub popup_opacity: f64,
    pub popup_bg_color: String,
    pub popup_accent_color: String,
    pub popup_position: String,
    pub popup_pin: String,
    pub clear_history_on_quit: bool,
    pub clear_clipboard_on_quit: bool,
}

#[derive(Serialize, Clone)]
pub struct UpdateStatusDto {
    pub available: bool,
    pub version: Option<String>,
    pub notes: Option<String>,
}

impl UpdateStatusDto {
    fn none() -> Self {
        Self { available: false, version: None, notes: None }
    }

    fn from_update(update: &tauri_plugin_updater::Update) -> Self {
        Self {
            available: true,
            version: Some(update.version.clone()),
            notes: update.body.clone(),
        }
    }
}

pub type UpdateState = std::sync::Mutex<Option<tauri_plugin_updater::Update>>;

pub type Store = Arc<Mutex<HistoryStore>>;

#[tauri::command]
pub fn get_history(store: State<Store>) -> Result<Vec<HistoryItemDto>, String> {
    store
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_history()
        .map(|items| items.into_iter().map(history_item_to_dto).collect())
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
pub fn clear_history(app: AppHandle, store: State<Store>) -> Result<(), String> {
    let paths = store.lock().unwrap_or_else(PoisonError::into_inner).clear_all().map_err(|e| e.to_string())?;
    for path in paths {
        let _ = std::fs::remove_file(path);
    }
    // Without this, a popup window already open at the time history is
    // cleared from Settings would keep showing the now-deleted rows until
    // the next capture event or the next time it's toggled open.
    let _ = app.emit("history-updated", ());
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
        hotkey: s.get_setting("hotkey").map_err(|e| e.to_string())?.unwrap_or_else(|| "Ctrl+Alt+V".into()),
        auto_check_updates: s
            .get_setting("auto_check_updates")
            .map_err(|e| e.to_string())?
            .map(|v| v == "true")
            .unwrap_or(true),
        sort_mode: s.get_setting("sort_mode").map_err(|e| e.to_string())?.unwrap_or_else(|| "last_copied".into()),
        capture_types: s
            .get_setting("capture_types")
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| "text,image,files,richtext".into())
            .split(',')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect(),
        popup_opacity: s
            .get_setting("popup_opacity")
            .map_err(|e| e.to_string())?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.9),
        popup_bg_color: s.get_setting("popup_bg_color").map_err(|e| e.to_string())?.unwrap_or_else(|| "#1e1e22".into()),
        popup_accent_color: s
            .get_setting("popup_accent_color")
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| "#ffffff".into()),
        popup_position: s.get_setting("popup_position").map_err(|e| e.to_string())?.unwrap_or_else(|| "cursor".into()),
        popup_pin: s.get_setting("popup_pin").map_err(|e| e.to_string())?.unwrap_or_else(|| "bottom".into()),
        clear_history_on_quit: s
            .get_setting("clear_history_on_quit")
            .map_err(|e| e.to_string())?
            .map(|v| v == "true")
            .unwrap_or(false),
        clear_clipboard_on_quit: s
            .get_setting("clear_clipboard_on_quit")
            .map_err(|e| e.to_string())?
            .map(|v| v == "true")
            .unwrap_or(false),
    })
}

#[tauri::command]
pub fn set_settings(
    settings: SettingsDto,
    store: State<Store>,
    hotkey: State<crate::watcher::HotkeyHandle>,
    app: AppHandle,
) -> Result<(), String> {
    let s = store.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(v) = settings.max_items {
        s.set_setting("max_items", &v.to_string()).map_err(|e| e.to_string())?;
    }
    if let Some(v) = settings.retention_days {
        s.set_setting("retention_days", &v.to_string()).map_err(|e| e.to_string())?;
    }
    s.set_setting("start_with_windows", if settings.start_with_windows { "true" } else { "false" })
        .map_err(|e| e.to_string())?;

    // Applied to the live hotkey listener thread first -- if the combo is
    // invalid or already claimed by another app, bail before persisting it
    // so settings never disagree with what's actually registered.
    hotkey.rebind(settings.hotkey.clone())?;
    s.set_setting("hotkey", &settings.hotkey).map_err(|e| e.to_string())?;

    s.set_setting("auto_check_updates", if settings.auto_check_updates { "true" } else { "false" })
        .map_err(|e| e.to_string())?;
    s.set_setting("sort_mode", &settings.sort_mode).map_err(|e| e.to_string())?;
    s.set_setting("capture_types", &settings.capture_types.join(",")).map_err(|e| e.to_string())?;

    s.set_setting("popup_opacity", &settings.popup_opacity.to_string()).map_err(|e| e.to_string())?;
    s.set_setting("popup_bg_color", &settings.popup_bg_color).map_err(|e| e.to_string())?;
    s.set_setting("popup_accent_color", &settings.popup_accent_color).map_err(|e| e.to_string())?;
    s.set_setting("popup_position", &settings.popup_position).map_err(|e| e.to_string())?;
    s.set_setting("popup_pin", &settings.popup_pin).map_err(|e| e.to_string())?;

    s.set_setting("clear_history_on_quit", if settings.clear_history_on_quit { "true" } else { "false" })
        .map_err(|e| e.to_string())?;
    s.set_setting("clear_clipboard_on_quit", if settings.clear_clipboard_on_quit { "true" } else { "false" })
        .map_err(|e| e.to_string())?;
    drop(s);

    use tauri_plugin_autostart::ManagerExt;
    let autostart = app.autolaunch();
    if settings.start_with_windows {
        let _ = autostart.enable();
    } else {
        let _ = autostart.disable();
    }

    let _ = app.emit("settings-updated", ());
    Ok(())
}

/// Converts a stored `HistoryItem` into its wire DTO, reading and
/// base64-encoding the (already-tiny, precomputed) thumbnail file for image
/// items. Kept as a free function, separate from the `#[tauri::command]`
/// wrapper, so it's testable without a `tauri::State`/running app.
fn history_item_to_dto(item: crate::store::HistoryItem) -> HistoryItemDto {
    let thumbnail = item.thumb_path.as_deref().and_then(|path| {
        std::fs::read(path)
            .ok()
            .map(|bytes| format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
    });
    let size = item_size(&item);
    let copy_count = item.copy_count;
    HistoryItemDto {
        id: item.id,
        kind: item.kind,
        preview: item.preview,
        thumbnail,
        size,
        created_at: item.created_at,
        copy_count,
    }
}

/// Formats a byte count as a short human-readable size, e.g. `"512 B"`,
/// `"3.4 KB"`, `"1.2 MB"`.
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Computes a short display size for a history item.
///
/// - `text`/`richtext`: byte length of the captured content.
/// - `image`: on-disk size of the (already app-resized) full image file --
///   local and cheap to stat, unlike arbitrary file-list entries below.
/// - `files`: item count rather than a byte size, since stat-ing arbitrary
///   (possibly slow/network) paths on every list render isn't worth it.
fn item_size(item: &crate::store::HistoryItem) -> Option<String> {
    match item.kind.as_str() {
        "image" => item
            .image_path
            .as_deref()
            .and_then(|path| std::fs::metadata(path).ok())
            .map(|meta| format_size(meta.len())),
        "files" => {
            let paths: Vec<String> = serde_json::from_str(item.content.as_deref().unwrap_or("[]")).ok()?;
            Some(format!("{} file{}", paths.len(), if paths.len() == 1 { "" } else { "s" }))
        }
        _ => item.content.as_ref().map(|c| format_size(c.len() as u64)),
    }
}

use tauri_plugin_updater::UpdaterExt;

async fn run_check(app: &AppHandle, state: &State<'_, UpdateState>) -> Result<UpdateStatusDto, String> {
    let updater = app.updater().map_err(|e| {
        eprintln!("update check failed: {e}");
        e.to_string()
    })?;
    match updater.check().await {
        Ok(Some(update)) => {
            let dto = UpdateStatusDto::from_update(&update);
            *state.lock().unwrap_or_else(PoisonError::into_inner) = Some(update);
            Ok(dto)
        }
        Ok(None) => {
            *state.lock().unwrap_or_else(PoisonError::into_inner) = None;
            Ok(UpdateStatusDto::none())
        }
        Err(e) => {
            eprintln!("update check failed: {e}");
            Ok(UpdateStatusDto::none())
        }
    }
}

#[tauri::command]
pub async fn get_update_status(state: State<'_, UpdateState>) -> Result<UpdateStatusDto, String> {
    let cached = state.lock().unwrap_or_else(PoisonError::into_inner).as_ref().map(UpdateStatusDto::from_update);
    Ok(cached.unwrap_or_else(UpdateStatusDto::none))
}

#[tauri::command]
pub async fn check_for_updates(app: AppHandle, state: State<'_, UpdateState>) -> Result<UpdateStatusDto, String> {
    run_check(&app, &state).await
}

#[tauri::command]
pub async fn install_update(app: AppHandle, state: State<'_, UpdateState>) -> Result<(), String> {
    let update = state.lock().unwrap_or_else(PoisonError::into_inner).clone().ok_or("no update available")?;
    update.download_and_install(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
    app.request_restart();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::HistoryItem;

    #[test]
    fn update_status_dto_none_reports_unavailable() {
        let dto = UpdateStatusDto::none();
        assert!(!dto.available);
        assert!(dto.version.is_none());
        assert!(dto.notes.is_none());
    }

    #[test]
    fn dto_conversion_includes_base64_thumbnail_when_thumb_path_set() {
        let dir = std::env::temp_dir().join(format!("cm-dto-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let thumb_path = dir.join("thumb.png");
        std::fs::write(&thumb_path, b"fake-png-bytes").unwrap();

        let item = HistoryItem {
            id: 1,
            kind: "image".into(),
            content: None,
            content_alt: None,
            image_path: Some("full.png".into()),
            thumb_path: Some(thumb_path.to_string_lossy().to_string()),
            preview: "Image (10x10)".into(),
            created_at: 0,
            first_copied_at: 0,
            copy_count: 1,
        };
        let dto = history_item_to_dto(item);
        let expected = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(b"fake-png-bytes")
        );
        assert_eq!(dto.thumbnail, Some(expected));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dto_conversion_has_no_thumbnail_when_thumb_path_absent() {
        let item = HistoryItem {
            id: 1,
            kind: "text".into(),
            content: Some("hi".into()),
            content_alt: None,
            image_path: None,
            thumb_path: None,
            preview: "hi".into(),
            created_at: 0,
            first_copied_at: 0,
            copy_count: 1,
        };
        let dto = history_item_to_dto(item);
        assert!(dto.thumbnail.is_none());
    }

    #[test]
    fn dto_conversion_reports_byte_size_for_text_and_richtext() {
        let item = HistoryItem {
            id: 1,
            kind: "text".into(),
            content: Some("hello".into()),
            content_alt: None,
            image_path: None,
            thumb_path: None,
            preview: "hello".into(),
            created_at: 0,
            first_copied_at: 0,
            copy_count: 1,
        };
        assert_eq!(history_item_to_dto(item).size.as_deref(), Some("5 B"));
    }

    #[test]
    fn dto_conversion_reports_kb_size_for_large_content() {
        let item = HistoryItem {
            id: 1,
            kind: "richtext".into(),
            content: Some("x".repeat(2048)),
            content_alt: Some("x".repeat(2048)),
            image_path: None,
            thumb_path: None,
            preview: "x".into(),
            created_at: 0,
            first_copied_at: 0,
            copy_count: 1,
        };
        assert_eq!(history_item_to_dto(item).size.as_deref(), Some("2.0 KB"));
    }

    #[test]
    fn dto_conversion_reports_file_size_for_image() {
        let dir = std::env::temp_dir().join(format!("cm-dto-size-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let image_path = dir.join("full.png");
        std::fs::write(&image_path, b"fake-png-bytes-here").unwrap();

        let item = HistoryItem {
            id: 1,
            kind: "image".into(),
            content: None,
            content_alt: None,
            image_path: Some(image_path.to_string_lossy().to_string()),
            thumb_path: None,
            preview: "Image (10x10)".into(),
            created_at: 0,
            first_copied_at: 0,
            copy_count: 1,
        };
        assert_eq!(history_item_to_dto(item).size.as_deref(), Some("19 B"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dto_conversion_reports_file_count_for_files() {
        let item = HistoryItem {
            id: 1,
            kind: "files".into(),
            content: Some(r#"["a.txt","b.txt","c.txt"]"#.into()),
            content_alt: None,
            image_path: None,
            thumb_path: None,
            preview: "3 files".into(),
            created_at: 0,
            first_copied_at: 0,
            copy_count: 1,
        };
        assert_eq!(history_item_to_dto(item).size.as_deref(), Some("3 files"));
    }
}
