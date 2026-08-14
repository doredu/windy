#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod store;
mod position;
mod win32;
mod clipboard_io;
mod watcher;
mod commands;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_history,
            commands::select_item,
            commands::delete_item,
            commands::clear_history,
            commands::count_history,
            commands::get_settings,
            commands::set_settings,
            commands::get_update_status,
            commands::check_for_updates,
            commands::install_update,
            commands::quit_app,
        ])
        .setup(|app| {
            use tauri::Manager;

            let app_data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_data_dir)?;
            // Images and the history DB must resolve to the same root (see
            // finding 6 in the final review) -- record it once here before
            // anything can call `clipboard_io::capture_current_clipboard`.
            win32::set_app_data_dir(app_data_dir.clone());

            let db_path = app_data_dir.join("history.db");
            let opened = store::HistoryStore::open(&db_path).or_else(|open_err| {
                // Spec: "DB missing or corrupt on startup: recreate an empty
                // DB rather than crashing." Delete the (corrupt) file and any
                // SQLite sidecar files, then retry opening once before
                // giving up and letting `?` abort startup.
                eprintln!("main: failed to open history DB ({open_err}); recreating a fresh database");
                let _ = std::fs::remove_file(&db_path);
                let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
                let _ = std::fs::remove_file(db_path.with_extension("db-shm"));
                store::HistoryStore::open(&db_path)
            })?;
            let initial_hotkey = opened.get_setting("hotkey")?.unwrap_or_else(|| "Ctrl+Alt+V".into());
            let store = std::sync::Arc::new(std::sync::Mutex::new(opened));
            let hotkey_handle = watcher::spawn(app.handle().clone(), store.clone(), initial_hotkey.clone());
            // RegisterHotKey can fail silently at startup (e.g. another app
            // already owns the combo) -- capture whether it actually took
            // before `hotkey_handle` is moved into `app.manage`, so the tray
            // label below can tell the user rather than confidently
            // advertising a combo that won't fire.
            let hotkey_active = hotkey_handle.is_active();
            app.manage(hotkey_handle);
            app.manage(store);

            app.manage(commands::UpdateState::default());
            {
                let auto_check = app
                    .state::<commands::Store>()
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .get_setting("auto_check_updates")?
                    .map(|v| v == "true")
                    .unwrap_or(true);
                if auto_check {
                    let app_handle = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        use tauri::Manager;
                        let state = app_handle.state::<commands::UpdateState>();
                        let _ = commands::check_for_updates(app_handle.clone(), state).await;
                    });
                }
            }

            if let Some(settings_window) = app.get_webview_window("settings") {
                let settings_window_handle = settings_window.clone();
                settings_window.on_window_event(move |event| {
                    use tauri::Emitter;
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        // Always defer to the frontend rather than hiding
                        // unconditionally: settings.ts already prompts to
                        // discard unsaved changes on Escape, but the native
                        // titlebar X bypassed that check entirely, silently
                        // discarding in-progress edits. Emitting an event and
                        // letting JS call hide() itself (after its own dirty
                        // check) covers both close paths with one code path.
                        let _ = settings_window_handle.emit("close-requested", ());
                    }
                });
            }

            if let Some(popup_window) = app.get_webview_window("popup") {
                let popup_window_handle = popup_window.clone();
                popup_window.on_window_event(move |event| {
                    use tauri::Emitter;
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        // The popup has no titlebar/close button (decorations:
                        // false), but a focused top-level window still
                        // receives Alt+F4 as a native CloseRequested event.
                        // Without this guard, Tauri's default behavior lets
                        // the close proceed and destroys the webview window --
                        // after that, get_webview_window("popup") returns None
                        // forever, silently breaking both the hotkey and the
                        // tray's "Open History" item until the app is
                        // restarted. Emit the same close-requested event the
                        // settings window uses so popup.ts's existing hide()
                        // path handles it.
                        let _ = popup_window_handle.emit("close-requested", ());
                    }
                });
            }

            // Label includes the current hotkey combo (e.g. "Open History
            // (Ctrl+Alt+V)") so users can discover/recall the shortcut from
            // the tray without opening Settings; kept in sync afterward by
            // `set_settings` via the managed `TrayMenu` handle below. If
            // registration failed at startup, say so here too -- Settings
            // already shows a warning banner, but the tray is the first
            // place a user checks when the hotkey "isn't working", and it
            // shouldn't confidently advertise a combo that won't fire.
            let open_history_label = if hotkey_active {
                format!("Open History  ({initial_hotkey})")
            } else {
                format!("Open History  ({initial_hotkey} — inactive)")
            };
            let open_history = tauri::menu::MenuItemBuilder::with_id("open_history", open_history_label).build(app)?;
            let settings = tauri::menu::MenuItemBuilder::with_id("settings", "Settings").build(app)?;
            let quit = tauri::menu::MenuItemBuilder::with_id("quit", "Quit").build(app)?;
            let menu = tauri::menu::MenuBuilder::new(app).items(&[&open_history, &settings, &quit]).build()?;
            app.manage(commands::TrayMenu { open_history: open_history.clone() });

            let mut tray_builder = tauri::tray::TrayIconBuilder::new()
                // Without a tooltip, hovering the tray icon (e.g. to find it
                // among several similar-looking icons in the notification
                // area) gives no indication of which app it is.
                .tooltip("Clipboard Manager")
                .menu(&menu)
                .on_menu_event(move |app, event| match event.id().as_ref() {
                    "open_history" => {
                        use tauri::Manager;
                        if app.get_webview_window("popup").is_some() {
                            // Reuse the hotkey's cursor-relative positioning
                            // instead of a bare show()/set_focus(), so the
                            // popup doesn't appear stuck wherever it was last
                            // shown/hidden (e.g. off-screen after a monitor
                            // was unplugged) when opened from the tray menu.
                            let store = app.state::<commands::Store>();
                            watcher::emit_toggle_popup(app, &store);
                        } else {
                            eprintln!("tray: no window labeled 'popup' to show");
                        }
                    }
                    "settings" => {
                        if let Some(w) = app.get_webview_window("settings") {
                            // On Windows, show() alone doesn't restore a
                            // minimized window -- it stays minimized in the
                            // taskbar even though is_visible() reports true,
                            // so clicking the tray's "Settings" item again
                            // would silently appear to do nothing.
                            let _ = w.unminimize();
                            // Same idea as "open_history"'s cursor-relative
                            // repositioning below: if the window was last
                            // left on a monitor that's no longer connected
                            // (e.g. unplugged, or a display-arrangement
                            // change), it stays at those now off-screen
                            // coordinates -- show()/set_focus() alone won't
                            // move it back, so the user sees nothing happen.
                            if let (Ok(pos), Ok(size)) = (w.outer_position(), w.outer_size()) {
                                let on_screen = w
                                    .available_monitors()
                                    .map(|monitors| {
                                        monitors.iter().any(|m| {
                                            let m_pos = m.position();
                                            let m_size = m.size();
                                            pos.x + (size.width as i32) > m_pos.x
                                                && pos.x < m_pos.x + m_size.width as i32
                                                && pos.y + (size.height as i32) > m_pos.y
                                                && pos.y < m_pos.y + m_size.height as i32
                                        })
                                    })
                                    .unwrap_or(true);
                                if !on_screen {
                                    let _ = w.center();
                                }
                            }
                            let _ = w.show();
                            let _ = w.set_focus();
                        } else {
                            eprintln!("tray: settings window not implemented yet (Task 9)");
                        }
                    }
                    "quit" => {
                        use tauri::Manager;
                        // Every other way of leaving the Settings window
                        // (Escape, the titlebar X) already prompts to discard
                        // unsaved edits -- quitting via the tray bypassed
                        // that entirely and silently threw them away. Defer
                        // to the same frontend confirm() via a round-trip
                        // event instead of exiting immediately whenever
                        // Settings is open; commands::quit_app (called back
                        // once the prompt is resolved) does the actual exit.
                        let settings_open = app
                            .get_webview_window("settings")
                            .and_then(|w| w.is_visible().ok())
                            .unwrap_or(false);
                        if settings_open {
                            if let Some(w) = app.get_webview_window("settings") {
                                use tauri::Emitter;
                                // Same Windows quirk as the "settings" branch
                                // above: is_visible() reports true even while
                                // minimized, so without unminimize() the
                                // confirm() prompt below would appear on a
                                // still-minimized window the user can't see,
                                // making Quit look like it silently hangs.
                                let _ = w.unminimize();
                                let _ = w.set_focus();
                                let _ = w.emit("quit-requested", ());
                            }
                        } else {
                            let store = app.state::<commands::Store>();
                            commands::perform_quit(app, store.inner());
                        }
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    // Left-clicking the tray icon opens the popup, same as
                    // the "Open History" menu item. Right-click already
                    // opens the context menu by default; only handle Click
                    // here so we don't also react to Enter/Move/Leave.
                    if let tauri::tray::TrayIconEvent::Click {
                        button: tauri::tray::MouseButton::Left,
                        button_state: tauri::tray::MouseButtonState::Up,
                        ..
                    } = event
                    {
                        use tauri::Manager;
                        let app = tray.app_handle();
                        if app.get_webview_window("popup").is_some() {
                            let store = app.state::<commands::Store>();
                            watcher::emit_toggle_popup(app, &store);
                        }
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray_builder = tray_builder.icon(icon.clone());
            }
            let _tray = tray_builder.build(app)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
