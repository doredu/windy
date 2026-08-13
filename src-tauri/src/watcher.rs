// Watcher thread wiring: owns only the OS hook/listener plumbing that
// triggers the popup toggle and feeds captured clipboard changes into
// `store::HistoryStore`.
//
// The Ctrl+Alt+V toggle is implemented with Windows' native `RegisterHotKey`
// API (WM_HOTKEY), not a hand-rolled `WH_KEYBOARD_LL` low-level hook. An
// earlier version of this file used a custom hook + edge-triggered state
// machine (see git history / hold_detector.rs's removal) to detect the
// combo, but that approach was found (empirically, via extensive manual
// testing) to be unreliable: Windows would intermittently fail to deliver
// key-up events for Ctrl/V once Alt was involved (Alt has special
// system-menu-tracking behavior in the low-level input pipeline), leaving
// the hand-rolled detector's internal state stuck and the hook occasionally
// unresponsive after a full press-release cycle. `RegisterHotKey` is the
// purpose-built OS API for "fire once when this exact combo is pressed" and
// sidesteps all of that: Windows does the chord-matching internally and
// delivers a single, reliable `WM_HOTKEY` message per press, with
// `MOD_NOREPEAT` suppressing re-fires while the combo is held down.
//
// No raw Win32 FFI primitives live here beyond what's needed to register the
// hotkey and the clipboard-format-listener window -- the reusable Win32
// wrappers (cursor position, hdrop, exclude-format checks, images_dir)
// already live in `crate::win32` (built in Task 5) and are reused here, not
// redefined.

use crate::position::clamp_popup_position;
use crate::store::HistoryStore;
use std::cell::RefCell;
use std::sync::{Arc, Mutex, PoisonError};
use tauri::{AppHandle, Emitter, Manager};

/// Starts the two watcher threads:
/// - a hotkey thread that registers Ctrl+Alt+V as a system-wide hotkey via
///   `RegisterHotKey` and emits `"toggle-popup"` on each `WM_HOTKEY`;
/// - a clipboard-listener thread that registers a hidden message-only
///   window for `WM_CLIPBOARDUPDATE` and captures clipboard changes into
///   `store`, emitting `"history-updated"` after each successful capture.
pub fn spawn(app_handle: AppHandle, store: Arc<Mutex<HistoryStore>>) {
    // Hotkey thread: registers Ctrl+Alt+V and pumps a message loop to
    // receive WM_HOTKEY. Owns the thread the hotkey is registered on --
    // RegisterHotKey ties the registration to the calling thread's message
    // queue, so this must stay a dedicated thread with its own loop.
    {
        let app_handle = app_handle.clone();
        std::thread::spawn(move || unsafe {
            run_hotkey_listener(app_handle);
        });
    }

    // Clipboard listener thread: hidden message-only window +
    // AddClipboardFormatListener.
    {
        std::thread::spawn(move || unsafe {
            run_clipboard_listener(app_handle, store);
        });
    }
}

/// tauri.conf.json's declared popup size, in *logical* pixels -- used only
/// as a last-resort fallback below if the popup window can't be looked up
/// (should not happen in practice, since the window is always declared).
const POPUP_LOGICAL_SIZE: (f64, f64) = (320.0, 340.0);

/// Returns the popup window's actual size in *physical* pixels, matching
/// the unit `GetCursorPos`/`clamp_popup_position` operate in. Uses the live
/// window's `outer_size()` (already physical) rather than a hardcoded
/// tuple, since a hardcoded logical-pixel guess can under-clamp on displays
/// scaled above 100%.
fn popup_physical_size(app_handle: &AppHandle) -> (i32, i32) {
    if let Some(window) = app_handle.get_webview_window("popup") {
        if let Ok(size) = window.outer_size() {
            return (size.width as i32, size.height as i32);
        }
        if let Ok(scale) = window.scale_factor() {
            return (
                (POPUP_LOGICAL_SIZE.0 * scale) as i32,
                (POPUP_LOGICAL_SIZE.1 * scale) as i32,
            );
        }
    }
    // Last resort: assume 100% scaling.
    (POPUP_LOGICAL_SIZE.0 as i32, POPUP_LOGICAL_SIZE.1 as i32)
}

/// Returns the work-area bounds (i.e. excluding the taskbar), in
/// virtual-screen coordinates, of whichever monitor contains `cursor`.
///
/// Falls back to a (0, 0, 0, 0) bounds box (which `clamp_popup_position`
/// will clamp the popup's origin into) if `GetMonitorInfoW` fails -- this
/// should never happen in practice since `MonitorFromPoint` with
/// `MONITOR_DEFAULTTONEAREST` always returns a valid monitor handle.
fn monitor_bounds_at(cursor: (i32, i32)) -> (i32, i32, i32, i32) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };

    unsafe {
        let point = POINT { x: cursor.0, y: cursor.1 };
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);

        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            eprintln!("watcher: GetMonitorInfoW failed; popup positioning may be wrong");
            return (0, 0, 0, 0);
        }

        let rc = info.rcWork;
        (rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top)
    }
}

/// Arbitrary id identifying our hotkey registration to `RegisterHotKey`/
/// `UnregisterHotKey` and matched against `WM_HOTKEY`'s wParam.
const TOGGLE_HOTKEY_ID: i32 = 1;

/// Registers Ctrl+Alt+V as a system-wide hotkey and pumps a message loop to
/// receive `WM_HOTKEY` on the calling (dedicated) thread -- `RegisterHotKey`
/// ties the registration to the calling thread's message queue, so this
/// must run on its own thread for the lifetime of the app.
unsafe fn run_hotkey_listener(app_handle: AppHandle) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY};

    const VK_V: u32 = 0x56;
    let modifiers = HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_ALT.0 | MOD_NOREPEAT.0);

    if let Err(e) = RegisterHotKey(None, TOGGLE_HOTKEY_ID, modifiers, VK_V) {
        eprintln!("watcher: RegisterHotKey failed, the Ctrl+Alt+V toggle is disabled: {e}");
        return;
    }

    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
        if msg.message == WM_HOTKEY && msg.wParam.0 as i32 == TOGGLE_HOTKEY_ID {
            let cursor = crate::win32::cursor_position();
            let popup_size = popup_physical_size(&app_handle);
            let (px, py) = clamp_popup_position(cursor, popup_size, monitor_bounds_at(cursor));
            let _ = app_handle.emit("toggle-popup", serde_json::json!({ "x": px, "y": py }));
        }
    }
    let _ = UnregisterHotKey(None, TOGGLE_HOTKEY_ID);
}

thread_local! {
    static LISTENER_CTX: RefCell<Option<(AppHandle, Arc<Mutex<HistoryStore>>)>> = RefCell::new(None);
}

/// Registers a hidden message-only window and `AddClipboardFormatListener`
/// on the calling (dedicated) thread, then pumps a message loop to keep it
/// alive. Must be called from the thread that will own the window -- the
/// thread-local is set here, on that same thread, before the window is
/// created, so `listener_wndproc` (dispatched on this thread) can reach it.
unsafe fn run_clipboard_listener(app_handle: AppHandle, store: Arc<Mutex<HistoryStore>>) {
    use windows::core::w;
    use windows::Win32::System::DataExchange::AddClipboardFormatListener;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DispatchMessageW, GetMessageW, RegisterClassW, TranslateMessage,
        HWND_MESSAGE, MSG, WNDCLASSW,
    };

    LISTENER_CTX.with(|cell| *cell.borrow_mut() = Some((app_handle, store)));

    let class_name = w!("ClipboardManagerListenerWindow");
    let hinstance = match GetModuleHandleW(None) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("watcher: GetModuleHandleW failed, clipboard capture is disabled: {e}");
            return;
        }
    };

    let wc = WNDCLASSW {
        lpfnWndProc: Some(listener_wndproc),
        hInstance: hinstance.into(),
        lpszClassName: class_name,
        ..Default::default()
    };
    if RegisterClassW(&wc) == 0 {
        eprintln!("watcher: RegisterClassW failed, clipboard capture is disabled");
        return;
    }

    let hwnd = match CreateWindowExW(
        Default::default(),
        class_name,
        w!("ClipboardManagerListener"),
        Default::default(),
        0,
        0,
        0,
        0,
        HWND_MESSAGE,
        None,
        hinstance,
        None,
    ) {
        Ok(hwnd) => hwnd,
        Err(e) => {
            eprintln!("watcher: CreateWindowExW failed, clipboard capture is disabled: {e}");
            return;
        }
    };

    if let Err(e) = AddClipboardFormatListener(hwnd) {
        eprintln!("watcher: AddClipboardFormatListener failed, clipboard capture is disabled: {e}");
        return;
    }

    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

/// Window procedure for the hidden clipboard-listener window. Runs on the
/// OS's behalf on the listener thread's message loop -- must never panic
/// (an unwind across this `extern "system"` boundary is undefined behavior)
/// and always falls through to `DefWindowProcW` for anything it doesn't
/// explicitly handle.
unsafe extern "system" fn listener_wndproc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{DefWindowProcW, WM_CLIPBOARDUPDATE};

    if msg == WM_CLIPBOARDUPDATE {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            LISTENER_CTX.with(|cell| {
                if let Some((app_handle, store)) = cell.borrow().as_ref() {
                    if let Some(item) = crate::clipboard_io::capture_current_clipboard() {
                        let captured = store
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .capture(item)
                            .is_ok();
                        if captured {
                            let _ = app_handle.emit("history-updated", ());
                        }
                    }
                }
            });
        }));
        if result.is_err() {
            eprintln!("watcher: listener_wndproc panicked and was caught; a clipboard change may not have been captured");
        }
        return windows::Win32::Foundation::LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}
