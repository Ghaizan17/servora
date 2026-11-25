//! Application settings, persisted as JSON in the XDG config directory.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Allowed refresh interval range, in milliseconds.
pub const MIN_REFRESH_INTERVAL_MS: u64 = 1000;
pub const MAX_REFRESH_INTERVAL_MS: u64 = 60_000;
pub const DEFAULT_REFRESH_INTERVAL_MS: u64 = 3000;

/// Allowed journal line range.
pub const MIN_LOG_LINES: u32 = 50;
pub const MAX_LOG_LINES: u32 = 5000;
pub const DEFAULT_LOG_LINES: u32 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Dark,
    Light,
    System,
}

/// Persisted user preferences. Never stores service state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AppSettings {
    pub theme: Theme,
    pub auto_refresh: bool,
    pub refresh_interval_ms: u64,
    pub minimize_to_tray: bool,
    pub start_minimized: bool,
    pub show_notifications: bool,
    pub log_line_limit: u32,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            auto_refresh: true,
            refresh_interval_ms: DEFAULT_REFRESH_INTERVAL_MS,
            minimize_to_tray: true,
            start_minimized: false,
            show_notifications: true,
            log_line_limit: DEFAULT_LOG_LINES,
        }
    }
}

impl AppSettings {
    /// Validate and clamp. Returns the list of human readable problems found.
    /// Values that can be clamped are clamped; out-of-range numbers are fixed up
    /// so a corrupt config file can never wedge the application.
    pub fn validate_and_repair(&mut self) -> Vec<String> {
        let mut problems = Vec::new();

        if self.refresh_interval_ms < MIN_REFRESH_INTERVAL_MS {
            problems.push(format!(
                "Refresh interval was raised to the minimum of {MIN_REFRESH_INTERVAL_MS} ms."
            ));
            self.refresh_interval_ms = MIN_REFRESH_INTERVAL_MS;
        }
        if self.refresh_interval_ms > MAX_REFRESH_INTERVAL_MS {
            problems.push(format!(
                "Refresh interval was lowered to the maximum of {MAX_REFRESH_INTERVAL_MS} ms."
            ));
            self.refresh_interval_ms = MAX_REFRESH_INTERVAL_MS;
        }
        if self.log_line_limit < MIN_LOG_LINES {
            problems.push(format!(
                "Log line limit was raised to the minimum of {MIN_LOG_LINES}."
            ));
            self.log_line_limit = MIN_LOG_LINES;
        }
        if self.log_line_limit > MAX_LOG_LINES {
            problems.push(format!(
                "Log line limit was lowered to the maximum of {MAX_LOG_LINES}."
            ));
            self.log_line_limit = MAX_LOG_LINES;
        }

        problems
    }

    /// Strict validation used when settings arrive from the frontend.
    pub fn validate(&self) -> Result<(), String> {
        if !(MIN_REFRESH_INTERVAL_MS..=MAX_REFRESH_INTERVAL_MS).contains(&self.refresh_interval_ms) {
            return Err(format!(
                "Refresh interval must be between {MIN_REFRESH_INTERVAL_MS} and {MAX_REFRESH_INTERVAL_MS} ms"
            ));
        }
        if !(MIN_LOG_LINES..=MAX_LOG_LINES).contains(&self.log_line_limit) {
            return Err(format!(
                "Log line limit must be between {MIN_LOG_LINES} and {MAX_LOG_LINES}"
            ));
        }
        Ok(())
    }
}

/// Where the settings file lives (`$XDG_CONFIG_HOME/servora/config.json`).
pub fn config_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(xdg).join("servora");
    }
    let home = std::env::var_os("HOME").unwrap_or_else(|| "/tmp".into());
    PathBuf::from(home).join(".config").join("servora")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

/// Directory used for the Servora application log.
pub fn state_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(xdg).join("servora");
    }
    let home = std::env::var_os("HOME").unwrap_or_else(|| "/tmp".into());
    PathBuf::from(home).join(".local").join("state").join("servora")
}

/// Application metadata surfaced on the Settings page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub identifier: String,
    pub description: String,
    pub repository: String,
    pub license: String,
    pub config_path: String,
    pub log_path: String,
    pub managed_units: Vec<String>,
    pub running_as_root: bool,
}

impl AppInfo {
    pub fn current() -> Self {
        Self {
            name: "Servora".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            identifier: "dev.servora.app".into(),
            description: "Local Development Service Manager".into(),
            repository: "https://github.com/Ghaizan17/servora".into(),
            license: "MIT".into(),
            config_path: config_path().to_string_lossy().into_owned(),
            log_path: state_dir()
                .join("servora.log")
                .to_string_lossy()
                .into_owned(),
            managed_units: crate::models::service::ServiceId::ALL
                .iter()
                .map(|s| s.unit().to_string())
                .collect(),
            running_as_root: is_root(),
        }
    }
}

pub fn is_root() -> bool {
    #[cfg(unix)]
    {
        // Effective uid 0
        unsafe { libc_geteuid() == 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

// Minimal `geteuid` binding so we do not pull in an extra crate for one call.
#[cfg(unix)]
extern "C" {
    #[link_name = "geteuid"]
    fn libc_geteuid() -> u32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        let settings = AppSettings::default();
        assert_eq!(settings.refresh_interval_ms, DEFAULT_REFRESH_INTERVAL_MS);
        assert_eq!(settings.theme, Theme::Dark);
        assert!(settings.auto_refresh);
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn validation_rejects_out_of_range_values() {
        let mut settings = AppSettings::default();
        settings.refresh_interval_ms = 10;
        assert!(settings.validate().is_err());

        settings.refresh_interval_ms = MAX_REFRESH_INTERVAL_MS + 1;
        assert!(settings.validate().is_err());

        settings.refresh_interval_ms = DEFAULT_REFRESH_INTERVAL_MS;
        settings.log_line_limit = 0;
        assert!(settings.validate().is_err());

        settings.log_line_limit = MAX_LOG_LINES + 1;
        assert!(settings.validate().is_err());
    }

    #[test]
    fn repair_clamps_and_reports() {
        let mut settings = AppSettings {
            refresh_interval_ms: 1,
            log_line_limit: 999_999,
            ..AppSettings::default()
        };
        let problems = settings.validate_and_repair();
        assert_eq!(problems.len(), 2);
        assert_eq!(settings.refresh_interval_ms, MIN_REFRESH_INTERVAL_MS);
        assert_eq!(settings.log_line_limit, MAX_LOG_LINES);
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn serialization_roundtrip_uses_camel_case() {
        let settings = AppSettings::default();
        let json = serde_json::to_string_pretty(&settings).unwrap();
        assert!(json.contains("\"refreshIntervalMs\""));
        assert!(json.contains("\"theme\": \"dark\""));
        let back: AppSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(settings, back);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let settings: AppSettings = serde_json::from_str("{\"theme\":\"light\"}").unwrap();
        assert_eq!(settings.theme, Theme::Light);
        assert_eq!(settings.refresh_interval_ms, DEFAULT_REFRESH_INTERVAL_MS);
    }

    #[test]
    fn config_path_is_xdg_compliant() {
        std::env::set_var("XDG_CONFIG_HOME", "/tmp/xdg-test");
        assert_eq!(config_path(), PathBuf::from("/tmp/xdg-test/servora/config.json"));
        std::env::remove_var("XDG_CONFIG_HOME");
        let fallback = config_path();
        assert!(fallback.ends_with(".config/servora/config.json"));
    }
}
