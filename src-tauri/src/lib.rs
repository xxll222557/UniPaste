#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_state;
mod clipboard;
mod commands;
mod config;
mod crypto;
mod discovery;
mod error;
mod sync {
    pub mod engine;
    pub mod protocol;
}

use app_state::ManagedState;
use commands::{
    approve_pair, clear_history, get_snapshot, reject_pair, remove_trusted_device, request_pair,
    set_sync_enabled, update_device_name, update_sync_preferences,
};
use config::load_or_create;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let config = load_or_create().expect("failed to load config");
    let state = ManagedState::new(
        config.path,
        config.history_path,
        config.identity,
        config.trusted_peers,
        config.history_entries,
    );

    tauri::Builder::default()
        .manage(state.clone())
        .setup(move |_app| {
            let discovery_runner = state.clone();
            tauri::async_runtime::spawn(async move {
                crate::discovery::run(discovery_runner).await;
            });

            crate::sync::engine::spawn_runtime(state.clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            request_pair,
            approve_pair,
            reject_pair,
            remove_trusted_device,
            set_sync_enabled,
            update_device_name,
            clear_history,
            update_sync_preferences
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
