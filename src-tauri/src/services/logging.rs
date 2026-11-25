//! Minimal application logger.
//!
//! Messages are written to stderr *and* to
//! `$XDG_STATE_HOME/servora/servora.log` (rotated at ~1 MiB). Nothing here ever
//! records credentials: callers only pass unit names, operation names and error
//! descriptions.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::models::settings::state_dir;

const MAX_LOG_SIZE: u64 = 1024 * 1024;

static LOG_FILE: OnceLock<Option<PathBuf>> = OnceLock::new();

pub fn log_path() -> PathBuf {
    state_dir().join("servora.log")
}

/// Initialise the logger. Safe to call more than once.
pub fn init() {
    LOG_FILE.get_or_init(|| {
        let directory = state_dir();
        if let Err(err) = fs::create_dir_all(&directory) {
            eprintln!("[servora] could not create state directory: {err}");
            return None;
        }
        let path = log_path();
        if let Ok(metadata) = fs::metadata(&path) {
            if metadata.len() > MAX_LOG_SIZE {
                let backup = path.with_extension("log.1");
                let _ = fs::rename(&path, backup);
            }
        }
        Some(path)
    });
}

/// Append a single line to the log file (best effort).
pub fn write_line(level: &str, message: &str) {
    let timestamp = format_timestamp_utc(now_seconds());
    let line = format!("{timestamp} [{level}] {message}\n");
    eprint!("{line}");

    if let Some(Some(path)) = LOG_FILE.get() {
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = file.write_all(line.as_bytes());
        }
    }
}

pub fn error(message: &str) {
    write_line("ERROR", message);
}

pub fn warn(message: &str) {
    write_line("WARN", message);
}

pub fn info(message: &str) {
    write_line("INFO", message);
}

fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

/// Format epoch seconds as `YYYY-MM-DDThh:mm:ssZ` without pulling in a date crate.
pub fn format_timestamp_utc(epoch_seconds: i64) -> String {
    let days = epoch_seconds.div_euclid(86_400);
    let seconds_of_day = epoch_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3600;
    let minute = (seconds_of_day % 3600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Howard Hinnant's `civil_from_days`.
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_known_timestamps() {
        assert_eq!(format_timestamp_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_timestamp_utc(1_000_000_000), "2001-09-09T01:46:40Z");
        assert_eq!(format_timestamp_utc(1_700_000_000), "2023-11-14T22:13:20Z");
    }

    #[test]
    fn handles_leap_days() {
        // 2024-02-29T12:00:00Z
        assert_eq!(format_timestamp_utc(1_709_208_000), "2024-02-29T12:00:00Z");
    }
}
