//! Servora — Local Development Service Manager.
//!
//! Library entry point. `main.rs` only calls [`run`] so the whole application
//! can be exercised from tests.

pub mod commands;
pub mod error;
pub mod models;
pub mod services;
pub mod state;
pub mod tray;

use tauri::{Manager, WindowEvent};

use state::AppState;

/// Build and run the Tauri application.
pub fn run() {
    services::logging::init();
    services::logging::info("servora starting");

    let settings = services::config::load();
    let start_minimized = settings.start_minimized;

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .manage(AppState::new(settings))
        .invoke_handler(tauri::generate_handler![
            commands::service::get_services_status,
            commands::service::get_service_status,
            commands::service::start_service,
            commands::service::stop_service,
            commands::service::restart_service,
            commands::service::start_all_services,
            commands::service::stop_all_services,
            commands::service::restart_all_services,
            commands::logs::get_service_logs,
            commands::logs::get_logs_context,
            commands::settings::get_app_settings,
            commands::settings::save_app_settings,
            commands::settings::get_app_info,
            commands::settings::get_authorization_info,
            commands::settings::get_settings_limits,
            commands::settings::reload_app_settings,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            if let Err(err) = tray::setup(&handle) {
                services::logging::warn(&format!(
                    "tray icon unavailable on this desktop: {err}"
                ));
            }
            if start_minimized {
                if let Some(window) = handle.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                let minimize_to_tray = app.state::<AppState>().settings().minimize_to_tray;
                if minimize_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                    services::logging::info("window hidden to tray; services left running");
                } else {
                    services::logging::info("window closed; services left running");
                }
            }
        });

    if let Err(err) = builder.run(tauri::generate_context!()) {
        services::logging::error(&format!("fatal error: {err}"));
        std::process::exit(1);
    }
}
