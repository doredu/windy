// Raw Win32 FFI wrappers used by clipboard_io.rs (and, from Task 6 onward,
// by the clipboard watcher). This module owns only thin wrappers around the
// Windows API — no higher-level clipboard capture/write-back logic lives here.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP, DROPFILES};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

const CF_HDROP: u32 = 15;

/// Returns the current cursor position in screen coordinates.
pub fn cursor_position() -> (i32, i32) {
    let mut point = windows::Win32::Foundation::POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut point);
    }
    (point.x, point.y)
}

/// Checks whether the clipboard currently carries the well-known
/// "ExcludeClipboardContentFromMonitorProcessing" registered format, which
/// apps (e.g. password managers) use to opt out of history tools.
pub fn clipboard_has_exclude_format() -> bool {
    let name: Vec<u16> = "ExcludeClipboardContentFromMonitorProcessing\0"
        .encode_utf16()
        .collect();
    unsafe {
        let format = RegisterClipboardFormatW(PCWSTR(name.as_ptr()));
        if format == 0 {
            return false;
        }
        IsClipboardFormatAvailable(format).is_ok()
    }
}

/// Reads the list of file paths from the clipboard's CF_HDROP format, if
/// present.
pub fn read_hdrop() -> Option<Vec<String>> {
    unsafe {
        if OpenClipboard(HWND(std::ptr::null_mut())).is_err() {
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

/// Returns `%APPDATA%\clipboard-manager\images`.
pub fn images_dir() -> std::path::PathBuf {
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("clipboard-manager")
        .join("images")
}
