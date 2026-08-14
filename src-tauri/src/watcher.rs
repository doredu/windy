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

use crate::position::{compute_popup_position, PopupPin, PopupPositionMode};
use crate::store::HistoryStore;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use tauri::{AppHandle, Emitter, Manager};

/// A pending rebind request handed to the hotkey thread: the new combo
/// string plus a oneshot channel the thread uses to report success/failure
/// back to whichever `tauri::command` requested the change.
struct PendingRebind {
    combo: String,
    result_tx: mpsc::Sender<Result<(), String>>,
}

/// Lets other threads (i.e. the `set_settings` command handler) ask the
/// dedicated hotkey thread to swap its registered combo. `RegisterHotKey`/
/// `UnregisterHotKey` are thread-affine, so the actual re-registration must
/// happen on `thread_id`'s own message loop -- this handle only queues the
/// request (`pending`) and wakes that loop with a custom thread message.
pub struct HotkeyHandle {
    thread_id: u32,
    pending: Arc<Mutex<Option<PendingRebind>>>,
    // Whether the toggle combo is actually registered with Windows right now.
    // RegisterHotKey can fail silently from the user's perspective -- e.g.
    // another running app already owns the exact same combo, or the stored
    // hotkey string is corrupt -- in which case the listener thread logs to
    // stderr (invisible to a normal user) and either exits early (startup) or
    // keeps the previous combo (rebind). Without this flag, Settings had no
    // way to tell the user their configured hotkey doesn't actually work.
    active: Arc<AtomicBool>,
}

impl HotkeyHandle {
    /// Whether the configured toggle hotkey is currently registered with
    /// Windows and will actually fire. False if `RegisterHotKey` failed at
    /// startup (e.g. the combo is already owned by another app) or the
    /// hotkey thread has exited.
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    /// Requests that the toggle hotkey be changed to `combo` (e.g.
    /// `"Ctrl+Alt+V"`), blocking briefly until the hotkey thread confirms
    /// the new registration succeeded (or reports why it didn't).
    pub fn rebind(&self, combo: String) -> Result<(), String> {
        let (result_tx, result_rx) = mpsc::channel();
        *self.pending.lock().unwrap_or_else(PoisonError::into_inner) = Some(PendingRebind { combo, result_tx });

        unsafe {
            use windows::Win32::Foundation::{LPARAM, WPARAM};
            use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;
            if PostThreadMessageW(self.thread_id, REBIND_HOTKEY_MSG, WPARAM(0), LPARAM(0)).is_err() {
                return Err("hotkey thread is no longer running".into());
            }
        }

        result_rx
            .recv_timeout(std::time::Duration::from_millis(500))
            .unwrap_or_else(|_| Err("hotkey thread did not respond in time".into()))
    }
}

/// Starts the two watcher threads:
/// - a hotkey thread that registers `initial_hotkey` as a system-wide
///   hotkey via `RegisterHotKey` and emits `"toggle-popup"` on each
///   `WM_HOTKEY`; its registration can be changed later via the returned
///   `HotkeyHandle`;
/// - a clipboard-listener thread that registers a hidden message-only
///   window for `WM_CLIPBOARDUPDATE` and captures clipboard changes into
///   `store`, emitting `"history-updated"` after each successful capture.
pub fn spawn(app_handle: AppHandle, store: Arc<Mutex<HistoryStore>>, initial_hotkey: String) -> HotkeyHandle {
    let pending: Arc<Mutex<Option<PendingRebind>>> = Arc::new(Mutex::new(None));
    // Optimistic default; the hotkey thread flips this to `false` if the
    // initial `RegisterHotKey` call fails before we get a chance to read it.
    let active = Arc::new(AtomicBool::new(true));

    // Hotkey thread: registers `initial_hotkey` and pumps a message loop to
    // receive WM_HOTKEY (and rebind requests). Owns the thread the hotkey is
    // registered on -- RegisterHotKey ties the registration to the calling
    // thread's message queue, so this must stay a dedicated thread with its
    // own loop.
    let thread_id = {
        let app_handle = app_handle.clone();
        let pending = pending.clone();
        let store = store.clone();
        let active = active.clone();
        let (thread_id_tx, thread_id_rx) = mpsc::channel();
        std::thread::spawn(move || unsafe {
            run_hotkey_listener(app_handle, store, initial_hotkey, pending, active, thread_id_tx);
        });
        // The hotkey thread reports its OS thread id as the first thing it
        // does, before entering the message loop -- block briefly here so
        // callers get back a `HotkeyHandle` that's immediately usable.
        thread_id_rx.recv().unwrap_or(0)
    };

    // Clipboard listener thread: hidden message-only window +
    // AddClipboardFormatListener.
    {
        std::thread::spawn(move || unsafe {
            run_clipboard_listener(app_handle, store);
        });
    }

    HotkeyHandle { thread_id, pending, active }
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

/// Computes the popup's cursor-relative position (respecting the user's
/// `popup_position`/`popup_pin` settings) and emits `"toggle-popup"` with it,
/// exactly as the global hotkey does. Also used by the tray icon/menu's
/// "open history" actions (main.rs) so opening the popup from the tray
/// positions it the same way instead of leaving it wherever it last was
/// shown/hidden.
pub fn emit_toggle_popup(app_handle: &AppHandle, store: &Arc<Mutex<HistoryStore>>) {
    let cursor = crate::win32::cursor_position();
    let popup_size = popup_physical_size(app_handle);
    let screen = monitor_bounds_at(cursor);
    let (mode, pin) = {
        let s = store.lock().unwrap_or_else(PoisonError::into_inner);
        (
            PopupPositionMode::parse(&s.get_setting("popup_position").ok().flatten().unwrap_or_default()),
            PopupPin::parse(&s.get_setting("popup_pin").ok().flatten().unwrap_or_default()),
        )
    };
    let foreground_window = crate::win32::foreground_window_rect();
    let (px, py) = compute_popup_position(mode, pin, cursor, foreground_window, popup_size, screen);
    let _ = app_handle.emit("toggle-popup", serde_json::json!({ "x": px, "y": py }));
}

/// Arbitrary id identifying our hotkey registration to `RegisterHotKey`/
/// `UnregisterHotKey` and matched against `WM_HOTKEY`'s wParam.
const TOGGLE_HOTKEY_ID: i32 = 1;

/// Custom thread message used to wake the hotkey thread's `GetMessageW`
/// loop when a rebind is requested (a plain `Arc<Mutex<..>>` write alone
/// wouldn't interrupt the blocking wait).
const REBIND_HOTKEY_MSG: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 1;

/// Parses a combo string like `"Ctrl+Alt+V"` into the `RegisterHotKey`
/// modifier flags and virtual-key code. Requires at least one modifier
/// (Ctrl/Alt/Shift/Win) plus exactly one trailing A-Z/0-9 key, matching what
/// the settings UI's press-to-record field can produce.
fn parse_hotkey(combo: &str) -> Result<(windows::Win32::UI::Input::KeyboardAndMouse::HOT_KEY_MODIFIERS, u32), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN};

    let mut mods: u32 = MOD_NOREPEAT.0;
    let mut vk: Option<u32> = None;

    for part in combo.split('+').map(str::trim) {
        if part.is_empty() {
            continue;
        }
        match part.to_ascii_uppercase().as_str() {
            "CTRL" | "CONTROL" => mods |= MOD_CONTROL.0,
            "ALT" => mods |= MOD_ALT.0,
            "SHIFT" => mods |= MOD_SHIFT.0,
            "WIN" | "SUPER" | "META" => mods |= MOD_WIN.0,
            key if key.len() == 1 && key.chars().next().unwrap().is_ascii_alphanumeric() => {
                if vk.is_some() {
                    return Err(format!("multiple keys in \"{combo}\" -- only one non-modifier key is supported"));
                }
                vk = Some(key.chars().next().unwrap() as u32);
            }
            other => return Err(format!("unsupported key \"{other}\" in \"{combo}\"")),
        }
    }

    let vk = vk.ok_or_else(|| format!("\"{combo}\" has no key, only modifiers"))?;
    if mods == MOD_NOREPEAT.0 {
        return Err(format!("\"{combo}\" needs at least one modifier (Ctrl/Alt/Shift/Win)"));
    }
    Ok((HOT_KEY_MODIFIERS(mods), vk))
}

/// Registers `initial_hotkey` as a system-wide hotkey and pumps a message
/// loop to receive `WM_HOTKEY` (and rebind requests posted via
/// `HotkeyHandle::rebind`) on the calling (dedicated) thread --
/// `RegisterHotKey` ties the registration to the calling thread's message
/// queue, so this must run on its own thread for the lifetime of the app.
unsafe fn run_hotkey_listener(
    app_handle: AppHandle,
    store: Arc<Mutex<HistoryStore>>,
    initial_hotkey: String,
    pending: Arc<Mutex<Option<PendingRebind>>>,
    active: Arc<AtomicBool>,
    thread_id_tx: mpsc::Sender<u32>,
) {
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey};
    use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY};

    let _ = thread_id_tx.send(GetCurrentThreadId());

    let (mut modifiers, mut vk) = match parse_hotkey(&initial_hotkey) {
        Ok(parsed) => parsed,
        Err(e) => {
            eprintln!("watcher: invalid stored hotkey \"{initial_hotkey}\" ({e}); the toggle is disabled");
            active.store(false, Ordering::SeqCst);
            return;
        }
    };
    if let Err(e) = RegisterHotKey(None, TOGGLE_HOTKEY_ID, modifiers, vk) {
        eprintln!("watcher: RegisterHotKey failed, the {initial_hotkey} toggle is disabled: {e}");
        active.store(false, Ordering::SeqCst);
        return;
    }

    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
        if msg.message == WM_HOTKEY && msg.wParam.0 as i32 == TOGGLE_HOTKEY_ID {
            emit_toggle_popup(&app_handle, &store);
        } else if msg.message == REBIND_HOTKEY_MSG {
            let Some(PendingRebind { combo, result_tx }) = pending.lock().unwrap_or_else(PoisonError::into_inner).take() else {
                continue;
            };
            let result = match parse_hotkey(&combo) {
                Err(e) => Err(e),
                Ok((new_modifiers, new_vk)) => {
                    let _ = UnregisterHotKey(None, TOGGLE_HOTKEY_ID);
                    match RegisterHotKey(None, TOGGLE_HOTKEY_ID, new_modifiers, new_vk) {
                        Ok(()) => {
                            modifiers = new_modifiers;
                            vk = new_vk;
                            active.store(true, Ordering::SeqCst);
                            Ok(())
                        }
                        Err(e) => {
                            // Best-effort: restore the previous combo so the
                            // toggle isn't left completely unbound. If even
                            // that re-registration fails (e.g. the combo was
                            // just taken by another app mid-rebind), the
                            // toggle is now genuinely unbound -- reflect that.
                            let restored = RegisterHotKey(None, TOGGLE_HOTKEY_ID, modifiers, vk).is_ok();
                            active.store(restored, Ordering::SeqCst);
                            Err(format!("couldn't register \"{combo}\": {e} (reverted to the previous hotkey)"))
                        }
                    }
                }
            };
            let _ = result_tx.send(result);
        }
    }
    let _ = UnregisterHotKey(None, TOGGLE_HOTKEY_ID);
}

thread_local! {
    static LISTENER_CTX: RefCell<Option<(AppHandle, Arc<Mutex<HistoryStore>>)>> = RefCell::new(None);
}

/// Whether `kind` (e.g. `"text"`/`"image"`/`"files"`/`"richtext"`) is
/// currently enabled by the `capture_types` setting. Defaults to capturing
/// everything if the setting is unset or empty (should not happen in
/// practice -- `seed_default_settings` always seeds it).
fn kind_is_captured(store: &HistoryStore, kind: &str) -> bool {
    match store.get_setting("capture_types").ok().flatten() {
        Some(types) if !types.is_empty() => types.split(',').any(|t| t == kind),
        _ => true,
    }
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
                        let store = store.lock().unwrap_or_else(PoisonError::into_inner);
                        if !kind_is_captured(&store, &item.kind) {
                            return;
                        }
                        let captured = store.capture(item).is_ok();
                        drop(store);
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

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT};

    #[test]
    fn parses_ctrl_alt_v() {
        let (mods, vk) = parse_hotkey("Ctrl+Alt+V").unwrap();
        assert_eq!(mods.0, MOD_CONTROL.0 | MOD_ALT.0 | MOD_NOREPEAT.0);
        assert_eq!(vk, 'V' as u32);
    }

    #[test]
    fn parses_lowercase_and_extra_whitespace() {
        let (mods, vk) = parse_hotkey(" ctrl + shift + 5 ").unwrap();
        assert_eq!(mods.0, MOD_CONTROL.0 | MOD_SHIFT.0 | MOD_NOREPEAT.0);
        assert_eq!(vk, '5' as u32);
    }

    #[test]
    fn rejects_combo_with_no_modifier() {
        assert!(parse_hotkey("V").is_err());
    }

    #[test]
    fn rejects_combo_with_no_key() {
        assert!(parse_hotkey("Ctrl+Alt").is_err());
    }

    #[test]
    fn rejects_combo_with_two_keys() {
        assert!(parse_hotkey("Ctrl+V+B").is_err());
    }

    #[test]
    fn rejects_unsupported_token() {
        assert!(parse_hotkey("Ctrl+F1").is_err());
    }
}
