#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod store;
mod hold_detector;
mod position;
mod win32;
mod clipboard_io;

fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
