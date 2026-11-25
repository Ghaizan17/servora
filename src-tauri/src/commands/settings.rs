//! Tauri commands for reading and writing application settings.

use tauri::State;

use crate::error::ErrorPayload;
use crate::models::settings::{
    AppInfo, AppSettings, MAX_LOG_LINES, MAX_REFRESH_INTERVAL_MS, MIN_LOG_LINES,
    MIN_REFRESH_INTERVAL_MS,
};
use crate::services::authorization::{self, AuthorizationInfo};
use crate::services::config;
use crate::state::AppState;

/// Static limits so the UI can render sliders without duplicating constants.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsLimits {
    pub min_refresh_interval_ms: u64,
    pub max_refresh_interval_ms: u64,
    pub min_log_lines: u32,
    pub max_log_lines: u32,
}

#[tauri::command]
pub fn get_app_settings(state: State<'_, AppState>) -> AppSettings {
    state.settings()
}

#[tauri::command]
pub fn save_app_settings(
    settings: AppSettings,
    state: State<'_, AppState>,
) -> Result<AppSettings, ErrorPayload> {
    state
        .apply_settings(settings)
        .map_err(|error| error.to_payload())
}

#[tauri::command]
pub fn get_app_info() -> AppInfo {
    AppInfo::current()
}

#[tauri::command]
pub fn get_authorization_info() -> AuthorizationInfo {
    authorization::info()
}

#[tauri::command]
pub fn get_settings_limits() -> SettingsLimits {
    SettingsLimits {
        min_refresh_interval_ms: MIN_REFRESH_INTERVAL_MS,
        max_refresh_interval_ms: MAX_REFRESH_INTERVAL_MS,
        min_log_lines: MIN_LOG_LINES,
        max_log_lines: MAX_LOG_LINES,
    }
}

/// Reload the configuration from disk (used after editing the file manually).
#[tauri::command]
pub fn reload_app_settings(state: State<'_, AppState>) -> AppSettings {
    let loaded = config::load();
    let _ = state.apply_settings(loaded.clone());
    loaded
}
