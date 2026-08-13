#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod store;
mod hold_detector;
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
            let store = std::sync::Arc::new(std::sync::Mutex::new(
                store::HistoryStore::open(&app_data_dir.join("history.db"))?,
            ));
            watcher::spawn(app.handle().clone(), store.clone());
            app.manage(store);

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
