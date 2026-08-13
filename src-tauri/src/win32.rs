// Raw Win32 FFI wrappers used by clipboard_io.rs (and, from Task 6 onward,
// by the clipboard watcher). This module owns only thin wrappers around the
// Windows API — no higher-level clipboard capture/write-back logic lives here.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP, DROPFILES};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

const CF_HDROP: u32 = 15;
const CF_UNICODETEXT: u32 = 13;

/// Clipboard access (`OpenClipboard`) commonly fails transiently right after
/// `WM_CLIPBOARDUPDATE` fires, while the source app still holds the
/// clipboard open. Retry a short, bounded number of times rather than
/// silently giving up on the capture.
const OPEN_CLIPBOARD_RETRY_ATTEMPTS: u32 = 5;
const OPEN_CLIPBOARD_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(10);

/// Calls `OpenClipboard`, retrying briefly on failure. Returns `true` once
/// open, `false` if it never succeeded within the retry budget.
unsafe fn open_clipboard_with_retry() -> bool {
    for attempt in 0..OPEN_CLIPBOARD_RETRY_ATTEMPTS {
        if OpenClipboard(HWND(std::ptr::null_mut())).is_ok() {
            return true;
        }
        if attempt + 1 < OPEN_CLIPBOARD_RETRY_ATTEMPTS {
            std::thread::sleep(OPEN_CLIPBOARD_RETRY_DELAY);
        }
    }
    false
}

/// Returns the current cursor position in screen coordinates.
pub fn cursor_position() -> (i32, i32) {
    let mut point = windows::Win32::Foundation::POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut point);
    }
    (point.x, point.y)
}

/// Returns the bounds (left, top, width, height) of the current foreground
/// window, in screen coordinates. Used for the "window center" popup
/// position mode. Returns `None` if there's no foreground window or
/// `GetWindowRect` fails (e.g. right after the desktop itself loses focus).
pub fn foreground_window_rect() -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowRect};
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return None;
        }
        Some((rect.left, rect.top, rect.right - rect.left, rect.bottom - rect.top))
    }
}

/// Checks whether the clipboard currently carries either of the two
/// well-known opt-out markers apps (e.g. password managers) use to exclude
/// their content from history tools:
/// - "ExcludeClipboardContentFromMonitorProcessing" registered format
///   (presence alone means excluded), or
/// - "CanIncludeInClipboardHistory" registered format present with a DWORD
///   value of 0 (explicitly means "do not include").
pub fn clipboard_has_exclude_format() -> bool {
    let exclude_name: Vec<u16> = "ExcludeClipboardContentFromMonitorProcessing\0"
        .encode_utf16()
        .collect();
    unsafe {
        let exclude_format = RegisterClipboardFormatW(PCWSTR(exclude_name.as_ptr()));
        if exclude_format != 0 && IsClipboardFormatAvailable(exclude_format).is_ok() {
            return true;
        }
    }

    let history_name: Vec<u16> = "CanIncludeInClipboardHistory\0".encode_utf16().collect();
    unsafe {
        let history_format = RegisterClipboardFormatW(PCWSTR(history_name.as_ptr()));
        if history_format == 0 || IsClipboardFormatAvailable(history_format).is_err() {
            return false;
        }

        if !open_clipboard_with_retry() {
            return false;
        }
        let excluded = (|| -> bool {
            let Ok(handle) = GetClipboardData(history_format) else { return false };
            let hglobal = HGLOBAL(handle.0);
            let ptr = GlobalLock(hglobal);
            if ptr.is_null() {
                return false;
            }
            let value = std::ptr::read_unaligned(ptr as *const u32);
            let _ = GlobalUnlock(hglobal);
            value == 0
        })();
        let _ = CloseClipboard();
        excluded
    }
}

/// Reads the list of file paths from the clipboard's CF_HDROP format, if
/// present.
pub fn read_hdrop() -> Option<Vec<String>> {
    unsafe {
        if !open_clipboard_with_retry() {
            return None;
        }

        let result = (|| {
            if !IsClipboardFormatAvailable(CF_HDROP).is_ok() {
                return None;
            }
            let handle = GetClipboardData(CF_HDROP).ok()?;
            let hdrop = HDROP(handle.0);

            let count = DragQueryFileW(hdrop, 0xFFFFFFFF, None);
            if count == 0 {
                return None;
            }

            let mut paths = Vec::with_capacity(count as usize);
            for i in 0..count {
                let len = DragQueryFileW(hdrop, i, None);
                if len == 0 {
                    continue;
                }
                let mut buf = vec![0u16; (len + 1) as usize];
                let written = DragQueryFileW(hdrop, i, Some(&mut buf));
                if written == 0 {
                    continue;
                }
                buf.truncate(written as usize);
                paths.push(String::from_utf16_lossy(&buf));
            }

            if paths.is_empty() {
                None
            } else {
                Some(paths)
            }
        })();

        let _ = CloseClipboard();
        result
    }
}

/// Writes a list of file paths to the clipboard as CF_HDROP.
pub fn write_hdrop(paths: &[String]) -> Result<(), String> {
    unsafe {
        // Build the double-null-terminated wide string list of paths.
        let mut wide_list: Vec<u16> = Vec::new();
        for path in paths {
            wide_list.extend(path.encode_utf16());
            wide_list.push(0);
        }
        wide_list.push(0); // final extra NUL terminates the whole list

        let dropfiles_size = std::mem::size_of::<DROPFILES>();
        let list_bytes = wide_list.len() * std::mem::size_of::<u16>();
        let total_size = dropfiles_size + list_bytes;

        let hglobal = GlobalAlloc(GMEM_MOVEABLE, total_size).map_err(|e| e.to_string())?;
        let ptr = GlobalLock(hglobal);
        if ptr.is_null() {
            let _ = GlobalFree(hglobal);
            return Err("GlobalLock returned null".into());
        }

        let dropfiles = DROPFILES {
            pFiles: dropfiles_size as u32,
            pt: windows::Win32::Foundation::POINT::default(),
            fNC: windows::Win32::Foundation::BOOL(0),
            fWide: windows::Win32::Foundation::BOOL(1),
        };
        std::ptr::write_unaligned(ptr as *mut DROPFILES, dropfiles);
        let list_ptr = (ptr as *mut u8).add(dropfiles_size) as *mut u16;
        std::ptr::copy_nonoverlapping(wide_list.as_ptr(), list_ptr, wide_list.len());

        let _ = GlobalUnlock(hglobal);

        if OpenClipboard(HWND(std::ptr::null_mut())).is_err() {
            let _ = GlobalFree(hglobal);
            return Err("OpenClipboard failed".into());
        }

        let result = (|| -> Result<(), String> {
            EmptyClipboard().map_err(|e| e.to_string())?;
            SetClipboardData(CF_HDROP, HANDLE(hglobal.0)).map_err(|e| e.to_string())?;
            Ok(())
        })();

        let _ = CloseClipboard();
        // Ownership of hglobal transfers to the clipboard on success; only
        // free it ourselves if SetClipboardData failed.
        if result.is_err() {
            let _ = GlobalFree(hglobal);
        }
        result
    }
}

/// Reads the complete raw "HTML Format" clipboard payload -- the whole
/// CF_HTML block (`Version`/`StartHTML`/`EndHTML`/`StartFragment`/
/// `EndFragment` header plus the fully wrapped document) -- rather than just
/// the inner Fragment. Many rich-text sources (Word, Outlook, Excel) declare
/// their CSS in a `<style>` block that lives *outside* the Fragment markers;
/// `arboard`'s `Clipboard::get().html()` only extracts the Fragment and
/// silently drops that styling. Capturing the raw payload verbatim and
/// writing it back the same way (see `write_html_full`) keeps formatting
/// intact end to end.
pub fn read_html_full() -> Option<String> {
    let name: Vec<u16> = "HTML Format\0".encode_utf16().collect();
    unsafe {
        let format = RegisterClipboardFormatW(PCWSTR(name.as_ptr()));
        if format == 0 {
            return None;
        }
        if !open_clipboard_with_retry() {
            return None;
        }

        let result = (|| {
            if !IsClipboardFormatAvailable(format).is_ok() {
                return None;
            }
            let handle = GetClipboardData(format).ok()?;
            let hglobal = HGLOBAL(handle.0);
            let ptr = GlobalLock(hglobal);
            if ptr.is_null() {
                return None;
            }
            let size = GlobalSize(hglobal);
            let bytes = std::slice::from_raw_parts(ptr as *const u8, size);
            // CF_HTML payloads are NUL-terminated; trim before decoding.
            let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
            let text = String::from_utf8_lossy(&bytes[..end]).into_owned();
            let _ = GlobalUnlock(hglobal);
            Some(text)
        })();

        let _ = CloseClipboard();
        result
    }
}

/// Writes a raw CF_HTML payload (as previously captured by
/// `read_html_full`) back to the clipboard verbatim, alongside a plain-text
/// fallback on CF_UNICODETEXT. Bypasses `arboard`'s `Set::html`, which
/// re-wraps only the inner Fragment through its own minimal header and so
/// would drop any styling declared outside the Fragment in the original
/// payload.
pub fn write_html_full(html: &str, alt: &str) -> Result<(), String> {
    let name: Vec<u16> = "HTML Format\0".encode_utf16().collect();
    unsafe {
        let format = RegisterClipboardFormatW(PCWSTR(name.as_ptr()));
        if format == 0 {
            return Err("failed to register HTML Format".into());
        }

        let alt_wide: Vec<u16> = alt.encode_utf16().chain(std::iter::once(0)).collect();
        let alt_hglobal = GlobalAlloc(GMEM_MOVEABLE, alt_wide.len() * std::mem::size_of::<u16>()).map_err(|e| e.to_string())?;
        let alt_ptr = GlobalLock(alt_hglobal);
        if alt_ptr.is_null() {
            let _ = GlobalFree(alt_hglobal);
            return Err("GlobalLock returned null".into());
        }
        std::ptr::copy_nonoverlapping(alt_wide.as_ptr(), alt_ptr as *mut u16, alt_wide.len());
        let _ = GlobalUnlock(alt_hglobal);

        let html_bytes = html.as_bytes();
        let html_hglobal = match GlobalAlloc(GMEM_MOVEABLE, html_bytes.len() + 1) {
            Ok(h) => h,
            Err(e) => {
                let _ = GlobalFree(alt_hglobal);
                return Err(e.to_string());
            }
        };
        let html_ptr = GlobalLock(html_hglobal);
        if html_ptr.is_null() {
            let _ = GlobalFree(alt_hglobal);
            let _ = GlobalFree(html_hglobal);
            return Err("GlobalLock returned null".into());
        }
        std::ptr::copy_nonoverlapping(html_bytes.as_ptr(), html_ptr as *mut u8, html_bytes.len());
        *((html_ptr as *mut u8).add(html_bytes.len())) = 0;
        let _ = GlobalUnlock(html_hglobal);

        if OpenClipboard(HWND(std::ptr::null_mut())).is_err() {
            let _ = GlobalFree(alt_hglobal);
            let _ = GlobalFree(html_hglobal);
            return Err("OpenClipboard failed".into());
        }

        // Track whether ownership of `alt_hglobal` has already transferred
        // to the clipboard, so a later failure doesn't free memory the
        // clipboard now owns.
        let mut alt_owned = false;
        let result = (|| -> Result<(), String> {
            EmptyClipboard().map_err(|e| e.to_string())?;
            SetClipboardData(CF_UNICODETEXT, HANDLE(alt_hglobal.0)).map_err(|e| e.to_string())?;
            alt_owned = true;
            SetClipboardData(format, HANDLE(html_hglobal.0)).map_err(|e| e.to_string())?;
            Ok(())
        })();

        let _ = CloseClipboard();
        if result.is_err() {
            if !alt_owned {
                let _ = GlobalFree(alt_hglobal);
            }
            let _ = GlobalFree(html_hglobal);
        }
        result
    }
}

/// The app's resolved data directory, set once at startup via
/// `set_app_data_dir` to the *same* directory `main.rs` uses for the
/// history DB (Tauri's `app.path().app_data_dir()`). This ensures images and
/// the DB always live under one root instead of two independently
/// hardcoded/derived paths that can silently diverge.
static APP_DATA_DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Records the app's resolved data directory. Must be called once, early at
/// startup (before any capture can happen), with the same directory used to
/// open the history DB.
pub fn set_app_data_dir(dir: std::path::PathBuf) {
    let _ = APP_DATA_DIR.set(dir);
}

/// Returns `<app_data_dir>\images`, where `app_data_dir` is the directory
/// set via `set_app_data_dir` (falling back to a hardcoded
/// `%APPDATA%\clipboard-manager` root only if that was never called, e.g. in
/// unit tests that exercise capture directly without going through
/// `main.rs`'s startup).
pub fn images_dir() -> std::path::PathBuf {
    let root = APP_DATA_DIR.get().cloned().unwrap_or_else(|| {
        dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("clipboard-manager")
    });
    root.join("images")
}
