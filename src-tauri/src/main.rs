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
        .invoke_handler(tauri::generate_handler![
            commands::get_history,
            commands::select_item,
            commands::delete_item,
            commands::get_settings,
            commands::set_settings,
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
            let store = std::sync::Arc::new(std::sync::Mutex::new(opened));
            watcher::spawn(app.handle().clone(), store.clone());
            app.manage(store);

            if let Some(settings_window) = app.get_webview_window("settings") {
                let settings_window_handle = settings_window.clone();
                settings_window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = settings_window_handle.hide();
                    }
                });
            }

            let open_history = tauri::menu::MenuItemBuilder::with_id("open_history", "Open History").build(app)?;
            let settings = tauri::menu::MenuItemBuilder::with_id("settings", "Settings").build(app)?;
            let quit = tauri::menu::MenuItemBuilder::with_id("quit", "Quit").build(app)?;
            let menu = tauri::menu::MenuBuilder::new(app).items(&[&open_history, &settings, &quit]).build()?;

            let mut tray_builder = tauri::tray::TrayIconBuilder::new()
                .menu(&menu)
                .on_menu_event(move |app, event| match event.id().as_ref() {
                    "open_history" => {
                        if let Some(w) = app.get_webview_window("popup") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        } else {
                            eprintln!("tray: no window labeled 'popup' to show");
                        }
                    }
                    "settings" => {
                        if let Some(w) = app.get_webview_window("settings") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        } else {
                            eprintln!("tray: settings window not implemented yet (Task 9)");
                        }
                    }
                    "quit" => app.exit(0),
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
                        let app = tray.app_handle();
                        if let Some(w) = app.get_webview_window("popup") {
                            let _ = w.show();
                            let _ = w.set_focus();
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
