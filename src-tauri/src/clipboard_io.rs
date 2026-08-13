// Higher-level clipboard capture/write-back logic, built on top of
// `crate::win32` (Win32 FFI wrappers) and `arboard` (cross-platform
// text/image clipboard access).

use crate::store::{HistoryItem, NewItem};

const TEXT_CAP_BYTES: usize = 200_000;
const IMAGE_MAX_DIMENSION: u32 = 1600;
const THUMBNAIL_MAX_DIMENSION: u32 = 40;

pub fn is_excluded_from_history() -> bool {
    // Windows convention: apps (e.g. password managers) register a custom
    // clipboard format named "ExcludeClipboardContentFromMonitorProcessing"
    // to opt out of history tools. If present on the clipboard, skip capture.
    crate::win32::clipboard_has_exclude_format()
}

/// Clipboard reads can transiently fail right after `WM_CLIPBOARDUPDATE`
/// fires, while the source app still holds the clipboard open. Retry a
/// bounded number of times with a short sleep rather than silently dropping
/// the capture.
const CLIPBOARD_RETRY_ATTEMPTS: u32 = 5;
const CLIPBOARD_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(10);

fn retry<T, E>(mut f: impl FnMut() -> Result<T, E>) -> Result<T, E> {
    let mut last_err = None;
    for attempt in 0..CLIPBOARD_RETRY_ATTEMPTS {
        match f() {
            Ok(v) => return Ok(v),
            Err(e) => {
                last_err = Some(e);
                if attempt + 1 < CLIPBOARD_RETRY_ATTEMPTS {
                    std::thread::sleep(CLIPBOARD_RETRY_DELAY);
                }
            }
        }
    }
    Err(last_err.unwrap())
}

/// Hashes bytes with SHA-256, returning a hex digest suitable for use as a
/// dedup key and/or content-addressed filename.
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub fn capture_current_clipboard() -> Option<NewItem> {
    if is_excluded_from_history() {
        return None;
    }

    let mut clipboard = retry(arboard::Clipboard::new).ok()?;

    // Image data is checked before CF_HDROP: some apps (e.g. browsers'
    // "Copy image") place both a CF_HDROP pointing at a throwaway temp file
    // and real image bytes (CF_DIB/PNG) on the clipboard at once, to support
    // consumers that only handle one or the other. Checking hdrop first
    // would always win and capture the temp file path instead of the image
    // itself. Plain file copies (e.g. from File Explorer) don't place image
    // data on the clipboard, so this reordering doesn't affect that case.
    if let Ok(image) = retry(|| clipboard.get_image()) {
        let (w, h) = (image.width as u32, image.height as u32);
        let scale = (IMAGE_MAX_DIMENSION as f32 / w.max(h) as f32).min(1.0);
        let (out_w, out_h) = ((w as f32 * scale) as u32, (h as f32 * scale) as u32);
        let img_buf = image::RgbaImage::from_raw(w, h, image.bytes.into_owned())?;
        let resized = image::imageops::resize(&img_buf, out_w.max(1), out_h.max(1), image::imageops::FilterType::Triangle);

        // Dedup/filename are derived from an actual hash of the resized
        // pixel content, not a randomly generated UUID -- so the same
        // image copied twice (including writing an item back to the OS
        // clipboard on selection, which re-triggers capture) hashes to the
        // same key and reuses the same file instead of growing without
        // bound. Content-addressing the filename also means a duplicate
        // never gets written to disk twice: the `path.exists()` check
        // below skips the write entirely.
        let hash = sha256_hex(resized.as_raw());
        let dir = crate::win32::images_dir();
        std::fs::create_dir_all(&dir).ok()?;
        let path = dir.join(format!("{hash}.png"));
        if !path.exists() {
            resized.save(&path).ok()?;
        }

        let thumb_scale = (THUMBNAIL_MAX_DIMENSION as f32 / w.max(h) as f32).min(1.0);
        let (thumb_w, thumb_h) = ((w as f32 * thumb_scale) as u32, (h as f32 * thumb_scale) as u32);
        let thumbnail = image::imageops::resize(&img_buf, thumb_w.max(1), thumb_h.max(1), image::imageops::FilterType::Triangle);
        let thumb_path = dir.join(format!("{hash}_thumb.png"));
        if !thumb_path.exists() {
            thumbnail.save(&thumb_path).ok()?;
        }

        return Some(NewItem {
            kind: "image".into(),
            content: None,
            content_alt: None,
            image_path: Some(path.to_string_lossy().to_string()),
            thumb_path: Some(thumb_path.to_string_lossy().to_string()),
            preview: format!("Image ({out_w}x{out_h})"),
            dedup_source: format!("image:{hash}"),
        });
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
            content_alt: None,
            image_path: None,
            thumb_path: None,
            preview,
            dedup_source: format!("files:{joined}"),
        });
    }

    if let Ok(html) = clipboard.get().html() {
        if !html.trim().is_empty() {
            let truncated_html = truncate_to_byte_cap(&html, TEXT_CAP_BYTES);
            let alt = match retry(|| clipboard.get_text()) {
                Ok(text) => truncate_to_byte_cap(&text, TEXT_CAP_BYTES),
                Err(_) => truncate_to_byte_cap(&strip_html_tags(&truncated_html), TEXT_CAP_BYTES),
            };
            let preview: String = alt.chars().take(120).collect();
            return Some(NewItem {
                kind: "richtext".into(),
                content: Some(truncated_html),
                content_alt: Some(alt.clone()),
                image_path: None,
                thumb_path: None,
                preview,
                dedup_source: format!("richtext:{alt}"),
            });
        }
    }

    if let Ok(text) = retry(|| clipboard.get_text()) {
        let truncated = truncate_to_byte_cap(&text, TEXT_CAP_BYTES);
        let preview: String = truncated.chars().take(120).collect();
        return Some(NewItem {
            kind: "text".into(),
            content: Some(truncated.clone()),
            content_alt: None,
            image_path: None,
            thumb_path: None,
            preview,
            dedup_source: format!("text:{truncated}"),
        });
    }

    None
}

/// Last-resort plain-text fallback for a richtext capture when the
/// clipboard offers CF_HTML but no CF_UNICODETEXT alongside it (rare in
/// practice -- most rich sources set both). Strips anything between `<`
/// and `>` rather than parsing HTML properly, which is good enough for a
/// preview/dedup/alt-text string.
fn strip_html_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

/// Truncates `text` to at most `cap_bytes` UTF-8 bytes, cutting only on a
/// valid char boundary (never splitting a multi-byte character).
fn truncate_to_byte_cap(text: &str, cap_bytes: usize) -> String {
    if text.len() <= cap_bytes {
        return text.to_string();
    }
    let mut end = cap_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
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
        "richtext" => {
            let html = item.content.clone().unwrap_or_default();
            let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
            clipboard.set().html(html, item.content_alt.clone()).map_err(|e| e.to_string())
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
            content_alt: None,
            image_path: None,
            thumb_path: None,
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
            content_alt: None,
            image_path: None,
            thumb_path: None,
            preview: String::new(),
            created_at: 0,
        }).unwrap();
        let captured = capture_current_clipboard().expect("expected a captured item");
        assert!(captured.content.unwrap().len() <= 200_000);
    }

    #[test]
    fn duplicate_image_content_dedups_to_the_same_file() {
        let pixels = image::RgbaImage::from_pixel(4, 4, image::Rgba([10, 20, 30, 255]));
        let mut clipboard = arboard::Clipboard::new().unwrap();
        clipboard
            .set_image(arboard::ImageData { width: 4, height: 4, bytes: pixels.clone().into_raw().into() })
            .unwrap();
        let first = capture_current_clipboard().expect("expected first image capture");
        assert_eq!(first.kind, "image");

        // Simulate copying the exact same image content again (e.g. what
        // happens when selecting an image item writes it back to the OS
        // clipboard, which re-triggers the listener).
        clipboard
            .set_image(arboard::ImageData { width: 4, height: 4, bytes: pixels.into_raw().into() })
            .unwrap();
        let second = capture_current_clipboard().expect("expected second image capture");

        assert_eq!(
            first.dedup_source, second.dedup_source,
            "identical pixel content must hash to the same dedup key, not a random UUID-derived one"
        );
        assert_eq!(
            first.image_path, second.image_path,
            "identical pixel content must resolve to the same content-addressed file, not a new one"
        );
    }

    #[test]
    fn different_image_content_gets_different_dedup_and_path() {
        let a = image::RgbaImage::from_pixel(4, 4, image::Rgba([1, 2, 3, 255]));
        let b = image::RgbaImage::from_pixel(4, 4, image::Rgba([9, 9, 9, 255]));
        let mut clipboard = arboard::Clipboard::new().unwrap();
        clipboard.set_image(arboard::ImageData { width: 4, height: 4, bytes: a.into_raw().into() }).unwrap();
        let first = capture_current_clipboard().expect("expected first image capture");
        clipboard.set_image(arboard::ImageData { width: 4, height: 4, bytes: b.into_raw().into() }).unwrap();
        let second = capture_current_clipboard().expect("expected second image capture");
        assert_ne!(first.dedup_source, second.dedup_source);
        assert_ne!(first.image_path, second.image_path);
    }

    #[test]
    fn non_ascii_text_over_cap_is_truncated_by_bytes_not_chars() {
        // '中' is 3 bytes in UTF-8. 100_000 repeats = 300_000 bytes, well
        // over the 200_000-byte cap, but only 100_000 chars (under a
        // chars()-based cap of 200_000), so a char-counting truncation would
        // fail to cap this by bytes.
        write_item_to_clipboard(&HistoryItem {
            id: 1,
            kind: "text".into(),
            content: Some("中".repeat(100_000)),
            content_alt: None,
            image_path: None,
            thumb_path: None,
            preview: String::new(),
            created_at: 0,
        }).unwrap();
        let captured = capture_current_clipboard().expect("expected a captured item");
        let content = captured.content.unwrap();
        assert!(content.len() <= 200_000, "byte length {} exceeds cap", content.len());
        assert!(std::str::from_utf8(content.as_bytes()).is_ok(), "truncation must not split a char");
    }

    #[test]
    fn image_capture_produces_a_smaller_thumbnail_file() {
        let pixels = image::RgbaImage::from_pixel(200, 100, image::Rgba([50, 60, 70, 255]));
        let mut clipboard = arboard::Clipboard::new().unwrap();
        clipboard
            .set_image(arboard::ImageData { width: 200, height: 100, bytes: pixels.into_raw().into() })
            .unwrap();
        let captured = capture_current_clipboard().expect("expected an image capture");
        assert_eq!(captured.kind, "image");
        let thumb_path = captured.thumb_path.expect("image capture must produce a thumb_path");
        let thumb_meta = std::fs::metadata(&thumb_path).expect("thumbnail file must exist on disk");
        let full_meta = std::fs::metadata(captured.image_path.unwrap()).unwrap();
        assert!(thumb_meta.len() < full_meta.len(), "thumbnail file should be smaller than the full-size image");

        let thumb_img = image::open(&thumb_path).unwrap();
        assert!(
            thumb_img.width() <= 40 && thumb_img.height() <= 40,
            "thumbnail dimensions ({}, {}) must be capped at 40px",
            thumb_img.width(),
            thumb_img.height()
        );
    }

    #[test]
    fn html_on_clipboard_is_captured_as_richtext_with_plain_text_alt() {
        let mut clipboard = arboard::Clipboard::new().unwrap();
        clipboard.set().html("<b>hello</b>", Some("hello")).unwrap();
        let captured = capture_current_clipboard().expect("expected a richtext capture");
        assert_eq!(captured.kind, "richtext");
        assert_eq!(captured.content.as_deref(), Some("<b>hello</b>"));
        assert_eq!(captured.content_alt.as_deref(), Some("hello"));
        assert_eq!(captured.preview, "hello");
    }

    #[test]
    fn richtext_write_then_capture_round_trips() {
        let item = HistoryItem {
            id: 1,
            kind: "richtext".into(),
            content: Some("<i>styled</i>".into()),
            content_alt: Some("styled".into()),
            image_path: None,
            thumb_path: None,
            preview: "styled".into(),
            created_at: 0,
        };
        write_item_to_clipboard(&item).unwrap();
        let captured = capture_current_clipboard().expect("expected a richtext capture");
        assert_eq!(captured.kind, "richtext");
        assert_eq!(captured.content.as_deref(), Some("<i>styled</i>"));
    }
}
