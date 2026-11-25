//! Servora — Local Development Service Manager.
//!
//! Library entry point. `main.rs` only calls [`run`] so the whole application
//! can be exercised from tests.

pub mod commands;

/// Re-exported so downstream users (including integration tests) see exactly
/// the same `servora-core` build as the application.
pub use servora_core;
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
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
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
            // Website manager
            commands::site::get_websites,
            commands::site::get_website,
            commands::site::get_website_history,
            commands::site::get_virtualhost_status,
            commands::site::detect_project_type,
            commands::site::detect_document_root,
            commands::site::validate_domain,
            commands::site::check_domain_conflicts,
            commands::site::create_website,
            commands::site::update_website,
            commands::site::rename_website,
            commands::site::remove_website,
            commands::site::get_managed_domains,
            commands::site::preview_siteforge_migration,
            commands::site::import_siteforge_websites,
            commands::site::get_environment_diagnostics,
            commands::site::get_environment_setup,
            commands::site::validate_apache_configuration,
            commands::site::reload_apache,
            commands::site::get_site_paths,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            if let Err(err) = tray::setup(&handle) {
                services::logging::warn(&format!("tray icon unavailable on this desktop: {err}"));
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
