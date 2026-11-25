//! Persistence for the application settings.
//!
//! The file lives at `$XDG_CONFIG_HOME/servora/config.json`
//! (`~/.config/servora/config.json` by default) and is written atomically:
//! a temporary file is created, flushed and then renamed over the target so a
//! crash can never leave a half written configuration behind.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};
use crate::models::settings::{config_dir, config_path, AppSettings};

/// Load the settings, falling back to defaults when the file is missing.
///
/// A corrupt file is preserved as `config.json.invalid` and the defaults are
/// returned, so the application always starts.
pub fn load() -> AppSettings {
    let path = config_path();
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return AppSettings::default(),
        Err(err) => {
            log::warn!("could not read {}: {err}", path.display());
            return AppSettings::default();
        }
    };

    match serde_json::from_str::<AppSettings>(&raw) {
        Ok(mut settings) => {
            for problem in settings.validate_and_repair() {
                log::warn!("settings repaired: {problem}");
            }
            settings
        }
        Err(err) => {
            log::warn!("invalid configuration at {}: {err}", path.display());
            let backup = path.with_extension("json.invalid");
            let _ = fs::rename(&path, &backup);
            AppSettings::default()
        }
    }
}

/// Persist the settings atomically.
pub fn save(settings: &AppSettings) -> AppResult<PathBuf> {
    settings.validate().map_err(AppError::Config)?;
    let path = config_path();
    write_atomic(&path, &settings)
}

fn write_atomic(path: &Path, settings: &AppSettings) -> AppResult<PathBuf> {
    let directory = config_dir();
    fs::create_dir_all(&directory).map_err(|err| {
        AppError::Config(format!("could not create {}: {err}", directory.display()))
    })?;

    let json = serde_json::to_string_pretty(settings)
        .map_err(|err| AppError::Config(format!("could not serialize settings: {err}")))?;

    let temp_path = path.with_extension("json.tmp");
    {
        let mut file = fs::File::create(&temp_path).map_err(|err| {
            AppError::Config(format!("could not write {}: {err}", temp_path.display()))
        })?;
        file.write_all(json.as_bytes())
            .map_err(|err| AppError::Config(format!("could not write settings: {err}")))?;
        file.write_all(b"\n")
            .map_err(|err| AppError::Config(format!("could not write settings: {err}")))?;
        file.sync_all()
            .map_err(|err| AppError::Config(format!("could not flush settings: {err}")))?;
    }

    fs::rename(&temp_path, path).map_err(|err| {
        let _ = fs::remove_file(&temp_path);
        AppError::Config(format!("could not replace {}: {err}", path.display()))
    })?;

    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::settings::Theme;

    /// `std::env` is process global, so the config tests must not overlap.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_temp_config<F: FnOnce(&Path)>(test: F) {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "servora-config-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("XDG_CONFIG_HOME", &dir);
        test(&dir.join("servora"));
        std::env::remove_var("XDG_CONFIG_HOME");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_yields_defaults() {
        with_temp_config(|_| {
            assert_eq!(load(), AppSettings::default());
        });
    }

    #[test]
    fn saves_and_reloads_settings() {
        with_temp_config(|dir| {
            let settings = AppSettings {
                theme: Theme::Light,
                refresh_interval_ms: 5000,
                log_line_limit: 400,
                ..AppSettings::default()
            };
            let path = save(&settings).unwrap();
            assert_eq!(path, dir.join("config.json"));
            assert!(path.exists());
            assert_eq!(load(), settings);
        });
    }

    #[test]
    fn rejects_invalid_settings() {
        with_temp_config(|_| {
            let settings = AppSettings {
                refresh_interval_ms: 5,
                ..AppSettings::default()
            };
            assert!(save(&settings).is_err());
        });
    }

    #[test]
    fn corrupt_file_is_preserved_and_defaults_return() {
        with_temp_config(|dir| {
            fs::create_dir_all(dir).unwrap();
            fs::write(dir.join("config.json"), b"{ not json").unwrap();
            assert_eq!(load(), AppSettings::default());
            assert!(dir.join("config.json.invalid").exists());
        });
    }

    #[test]
    fn out_of_range_values_are_repaired_on_load() {
        with_temp_config(|dir| {
            fs::create_dir_all(dir).unwrap();
            fs::write(
                dir.join("config.json"),
                br#"{"theme":"dark","refreshIntervalMs":10,"logLineLimit":90000}"#,
            )
            .unwrap();
            let settings = load();
            assert_eq!(
                settings.refresh_interval_ms,
                crate::models::settings::MIN_REFRESH_INTERVAL_MS
            );
            assert_eq!(
                settings.log_line_limit,
                crate::models::settings::MAX_LOG_LINES
            );
        });
    }
}
