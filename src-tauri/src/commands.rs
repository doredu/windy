// Thin Tauri command handlers exposing `store::HistoryStore` and
// `clipboard_io` to the frontend. No business logic lives here — it all
// delegates to `store.rs` / `clipboard_io.rs`.

use crate::store::HistoryStore;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, PoisonError};
use tauri::{AppHandle, State};

#[derive(Serialize)]
pub struct HistoryItemDto {
    pub id: i64,
    pub kind: String,
    pub preview: String,
    pub thumbnail: Option<String>,
    pub size: Option<String>,
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
    HistoryItemDto {
        id: item.id,
        kind: item.kind,
        preview: item.preview,
        thumbnail,
        size,
        created_at: item.created_at,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::HistoryItem;

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
        };
        assert_eq!(history_item_to_dto(item).size.as_deref(), Some("3 files"));
    }
}
