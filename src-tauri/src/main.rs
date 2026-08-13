#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod store;
mod hold_detector;
mod position;
mod win32;
mod clipboard_io;
mod watcher;

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            use tauri::Manager;

            let app_data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_data_dir)?;
            let store = std::sync::Arc::new(std::sync::Mutex::new(
                store::HistoryStore::open(&app_data_dir.join("history.db"))?,
            ));
            watcher::spawn(app.handle().clone(), store.clone());
            app.manage(store);

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
