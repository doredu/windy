// Watcher thread wiring: owns only the OS hook/listener plumbing that drives
// the pure `hold_detector::HoldDetector` state machine and feeds captured
// clipboard changes into `store::HistoryStore`.
//
// No raw Win32 FFI primitives live here beyond what's needed to install the
// low-level keyboard hook and the clipboard-format-listener window -- the
// reusable Win32 wrappers (cursor position, hdrop, exclude-format checks,
// images_dir) already live in `crate::win32` (built in Task 5) and are
// reused here, not redefined.

use crate::hold_detector::HoldDetector;
use crate::position::clamp_popup_position;
use crate::store::HistoryStore;
use std::cell::RefCell;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

/// Starts the three watcher threads:
/// - a 100ms timer thread that polls the shared `HoldDetector` and emits
///   `"show-popup"` when a 3s Ctrl+C hold is detected;
/// - a keyboard-hook thread that installs a `WH_KEYBOARD_LL` hook and feeds
///   key state into the shared `HoldDetector`;
/// - a clipboard-listener thread that registers a hidden message-only
///   window for `WM_CLIPBOARDUPDATE` and captures clipboard changes into
///   `store`, emitting `"history-updated"` after each successful capture.
pub fn spawn(app_handle: AppHandle, store: Arc<Mutex<HistoryStore>>) {
    let detector = Arc::new(Mutex::new(HoldDetector::new()));

    // Timer thread: polls the shared detector every 100ms and asks it to fire.
    {
        let detector = detector.clone();
        let app_handle = app_handle.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(100));
            let fired = {
                let mut d = detector.lock().unwrap_or_else(PoisonError::into_inner);
                d.check(Instant::now(), Duration::from_secs(3))
            };
            if fired {
                let cursor = crate::win32::cursor_position();
                let (px, py) = clamp_popup_position(cursor, (260, 320), monitor_bounds_at(cursor));
                let _ = app_handle.emit("show-popup", serde_json::json!({ "x": px, "y": py }));
            }
        });
    }

    // Keyboard hook thread: installs WH_KEYBOARD_LL, feeds key state into
    // `detector`. Runs its own message loop (required for a low-level hook
    // to receive callbacks) and owns the thread the thread-local is set on.
    {
        let detector = detector.clone();
        std::thread::spawn(move || unsafe {
            install_keyboard_hook(detector);
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

/// Returns the work-area bounds (i.e. excluding the taskbar), in
/// virtual-screen coordinates, of whichever monitor contains `cursor`. Not
/// pure (calls `MonitorFromPoint`/`GetMonitorInfoW`), so no unit test --
/// covered by manual verification only.
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

thread_local! {
    static HOOK_DETECTOR: RefCell<Option<Arc<Mutex<HoldDetector>>>> = RefCell::new(None);
}

/// Installs the low-level keyboard hook on the calling (dedicated) thread
/// and pumps a message loop to keep it alive. Must be called from the
/// thread that will own the hook -- the thread-local is set here, on that
/// same thread, before the hook is installed, so `hook_proc` (which also
/// runs on this thread) can reach it.
unsafe fn install_keyboard_hook(detector: Arc<Mutex<HoldDetector>>) {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetMessageW, SetWindowsHookExW, UnhookWindowsHookEx, MSG, WH_KEYBOARD_LL,
    };

    HOOK_DETECTOR.with(|cell| *cell.borrow_mut() = Some(detector));

    let hook = match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("watcher: SetWindowsHookExW failed, hold-to-open is disabled: {e}");
            return;
        }
    };

    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).0 > 0 {}
    let _ = UnhookWindowsHookEx(hook);
}

/// `WH_KEYBOARD_LL` hook callback. Runs on the OS's behalf on the hook
/// thread's message loop -- it must never panic (a panic unwinding across
/// this `extern "system"` boundary is undefined behavior) and must ALWAYS
/// call `CallNextHookEx`, on every path, so this hook never blocks key
/// delivery anywhere else on the system.
unsafe extern "system" fn hook_proc(
    code: i32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{
        KBDLLHOOKSTRUCT, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    // Guard the whole body in catch_unwind: any panic inside (mutex logic,
    // etc.) is swallowed here rather than unwinding across the FFI boundary.
    // CallNextHookEx below always runs regardless.
    let result = std::panic::catch_unwind(|| {
        if code >= 0 {
            let data = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
            let up = matches!(wparam.0 as u32, WM_KEYUP | WM_SYSKEYUP);
            if down || up {
                HOOK_DETECTOR.with(|cell| {
                    if let Some(detector) = cell.borrow().as_ref() {
                        let mut d = detector.lock().unwrap_or_else(PoisonError::into_inner);
                        match data.vkCode {
                            0xA2 | 0xA3 => d.set_ctrl(down), // VK_LCONTROL, VK_RCONTROL (WH_KEYBOARD_LL never reports the generic 0x11)
                            0x43 => d.set_c(down),           // VK_C
                            _ => {}
                        }
                    }
                });
            }
        }
    });
    if result.is_err() {
        eprintln!("watcher: hook_proc panicked and was caught; key state may be stale");
    }

    windows::Win32::UI::WindowsAndMessaging::CallNextHookEx(None, code, wparam, lparam)
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
/// (see `hook_proc` for why) and always falls through to `DefWindowProcW`
/// for anything it doesn't explicitly handle.
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
