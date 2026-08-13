// Higher-level clipboard capture/write-back logic, built on top of
// `crate::win32` (Win32 FFI wrappers) and `arboard` (cross-platform
// text/image clipboard access).

use crate::store::{HistoryItem, NewItem};

const TEXT_CAP_BYTES: usize = 200_000;
const IMAGE_MAX_DIMENSION: u32 = 1600;

pub fn is_excluded_from_history() -> bool {
    // Windows convention: apps (e.g. password managers) register a custom
    // clipboard format named "ExcludeClipboardContentFromMonitorProcessing"
    // to opt out of history tools. If present on the clipboard, skip capture.
    crate::win32::clipboard_has_exclude_format()
}

pub fn capture_current_clipboard() -> Option<NewItem> {
    if is_excluded_from_history() {
        return None;
    }

    if let Some(paths) = crate::win32::read_hdrop() {
        let joined = paths.join("\n");
        let preview = if paths.len() == 1 {
            paths[0].clone()
        } else {
            format!("{} files", paths.len())
        };
        let content = serde_json::to_string(&paths).ok()?;
        return Some(NewItem {
            kind: "files".into(),
            content: Some(content),
            image_path: None,
            preview,
            dedup_source: format!("files:{joined}"),
        });
    }

    let mut clipboard = arboard::Clipboard::new().ok()?;

    if let Ok(image) = clipboard.get_image() {
        let (w, h) = (image.width as u32, image.height as u32);
        let scale = (IMAGE_MAX_DIMENSION as f32 / w.max(h) as f32).min(1.0);
        let (out_w, out_h) = ((w as f32 * scale) as u32, (h as f32 * scale) as u32);
        let img_buf = image::RgbaImage::from_raw(w, h, image.bytes.into_owned())?;
        let resized = image::imageops::resize(&img_buf, out_w.max(1), out_h.max(1), image::imageops::FilterType::Triangle);
        let id = uuid::Uuid::new_v4();
        let dir = crate::win32::images_dir();
        std::fs::create_dir_all(&dir).ok()?;
        let path = dir.join(format!("{id}.png"));
        resized.save(&path).ok()?;
        return Some(NewItem {
            kind: "image".into(),
            content: None,
            image_path: Some(path.to_string_lossy().to_string()),
            preview: format!("Image ({out_w}x{out_h})"),
            dedup_source: format!("image:{}", path.to_string_lossy()),
        });
    }

    if let Ok(text) = clipboard.get_text() {
        let truncated: String = text.chars().take(TEXT_CAP_BYTES).collect();
        let preview: String = truncated.chars().take(120).collect();
        return Some(NewItem {
            kind: "text".into(),
            content: Some(truncated.clone()),
            image_path: None,
            preview,
            dedup_source: format!("text:{truncated}"),
        });
    }

    None
}

pub fn write_item_to_clipboard(item: &HistoryItem) -> Result<(), String> {
    match item.kind.as_str() {
        "text" => {
            let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
            clipboard.set_text(item.content.clone().unwrap_or_default()).map_err(|e| e.to_string())
        }
        "image" => {
            let path = item.image_path.clone().ok_or("missing image_path")?;
            let img = image::open(&path).map_err(|e| e.to_string())?.to_rgba8();
            let (w, h) = img.dimensions();
            let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
            clipboard
                .set_image(arboard::ImageData { width: w as usize, height: h as usize, bytes: img.into_raw().into() })
                .map_err(|e| e.to_string())
        }
        "files" => {
            let paths: Vec<String> = serde_json::from_str(item.content.as_deref().unwrap_or("[]")).map_err(|e| e.to_string())?;
            crate::win32::write_hdrop(&paths)
        }
        other => Err(format!("unknown item kind: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::HistoryItem;

    #[test]
    fn write_then_capture_round_trips_text() {
        let item = HistoryItem {
            id: 1,
            kind: "text".into(),
            content: Some("clipboard round trip".into()),
            image_path: None,
            preview: "clipboard round trip".into(),
            created_at: 0,
        };
        write_item_to_clipboard(&item).unwrap();
        let captured = capture_current_clipboard().expect("expected a captured item");
        assert_eq!(captured.kind, "text");
        assert_eq!(captured.content.as_deref(), Some("clipboard round trip"));
    }

    #[test]
    fn text_over_cap_is_truncated_on_capture() {
        write_item_to_clipboard(&HistoryItem {
            id: 1,
            kind: "text".into(),
            content: Some("x".repeat(300_000)),
            image_path: None,
            preview: String::new(),
            created_at: 0,
        }).unwrap();
        let captured = capture_current_clipboard().expect("expected a captured item");
        assert!(captured.content.unwrap().len() <= 200_000);
    }
}
