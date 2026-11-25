//! Models for the journal log viewer.

use serde::{Deserialize, Serialize};

use crate::models::service::ServiceId;

/// Journal priority filters exposed in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    /// Every priority, no `-p` argument.
    #[default]
    All,
    /// `-p warning` (warning and more severe).
    Warnings,
    /// `-p err` (errors and more severe).
    Errors,
}

impl LogLevel {
    /// journalctl `-p` argument, if the level needs one.
    pub fn priority_arg(self) -> Option<&'static str> {
        match self {
            LogLevel::All => None,
            LogLevel::Warnings => Some("warning"),
            LogLevel::Errors => Some("err"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LogLevel::All => "All levels",
            LogLevel::Warnings => "Warnings and errors",
            LogLevel::Errors => "Errors only",
        }
    }
}

/// Request payload for `get_service_logs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogRequest {
    pub service: ServiceId,
    #[serde(default)]
    pub level: LogLevel,
    /// Requested number of lines; clamped to [`MIN_LOG_LINES`..`MAX_LOG_LINES`].
    #[serde(default)]
    pub limit: Option<u32>,
}

/// One parsed journal line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    /// ISO-8601 timestamp as printed by journalctl, when it could be parsed.
    pub timestamp: Option<String>,
    /// Ident (process name) when present.
    pub ident: Option<String>,
    /// Message body.
    pub message: String,
}

/// A raw journal line that did not match the expected format.
impl LogEntry {
    /// Parse a `journalctl --output=short-iso` line:
    /// `2026-05-01T10:20:30+0200 host ident[123]: message`
    pub fn parse_line(line: &str) -> Self {
        let mut rest = line;
        let mut timestamp = None;
        let mut ident = None;

        if let Some(first_space) = line.find(' ') {
            let candidate = &line[..first_space];
            if looks_like_timestamp(candidate) {
                timestamp = Some(candidate.to_string());
                rest = &line[first_space + 1..];
                // Drop the hostname token.
                if let Some(second_space) = rest.find(' ') {
                    rest = &rest[second_space + 1..];
                }
            }
        }

        // Identify `ident[pid]: ` prefix.
        if let Some(colon) = rest.find(": ") {
            let head = &rest[..colon];
            if !head.contains(' ') && head.len() <= 64 {
                let name = head.split('[').next().unwrap_or(head);
                if !name.is_empty() {
                    ident = Some(name.to_string());
                }
                rest = &rest[colon + 2..];
            }
        }

        if timestamp.is_none() && ident.is_none() {
            // Not a standard line: keep the original text untouched.
            return Self {
                timestamp: None,
                ident: None,
                message: line.to_string(),
            };
        }

        Self {
            timestamp,
            ident,
            message: rest.to_string(),
        }
    }
}

fn looks_like_timestamp(token: &str) -> bool {
    let bytes = token.as_bytes();
    token.len() >= 19
        && bytes.len() > 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && token[..4].chars().all(|c| c.is_ascii_digit())
}

/// Response for `get_service_logs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogResponse {
    pub service: ServiceId,
    pub unit: String,
    pub level: LogLevel,
    pub entries: Vec<LogEntry>,
    /// Number of lines actually returned.
    pub count: usize,
    /// True when the byte cap cut the output short.
    pub truncated: bool,
    /// Number of lines requested.
    pub requested_lines: u32,
    pub retrieved_at_ms: i64,
}

impl LogResponse {
    pub fn empty(service: ServiceId, level: LogLevel, requested_lines: u32, now: i64) -> Self {
        Self {
            service,
            unit: service.unit().to_string(),
            level,
            entries: Vec::new(),
            count: 0,
            truncated: false,
            requested_lines,
            retrieved_at_ms: now,
        }
    }
}

/// Maximum bytes of journal output kept in memory.
pub const MAX_LOG_BYTES: usize = 512 * 1024;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::settings::{MAX_LOG_LINES, MIN_LOG_LINES};

    #[test]
    fn parses_short_iso_line() {
        let entry = LogEntry::parse_line(
            "2026-05-01T10:20:30+0200 fedora httpd[1234]: AH00094: Command line: '/usr/sbin/httpd'",
        );
        assert_eq!(entry.timestamp.as_deref(), Some("2026-05-01T10:20:30+0200"));
        assert_eq!(entry.ident.as_deref(), Some("httpd"));
        assert_eq!(entry.message, "AH00094: Command line: '/usr/sbin/httpd'");
    }

    #[test]
    fn parses_systemd_boot_line() {
        let entry = LogEntry::parse_line(
            "2026-05-01T08:00:00+0000 fedora systemd[1]: Started MariaDB 10.11 database server.",
        );
        assert_eq!(entry.ident.as_deref(), Some("systemd"));
        assert_eq!(entry.message, "Started MariaDB 10.11 database server.");
    }

    #[test]
    fn keeps_unparseable_lines_intact() {
        let entry = LogEntry::parse_line("-- Logs begin at Mon 2026-05-01 --");
        assert_eq!(entry.timestamp, None);
        assert_eq!(entry.ident, None);
        assert_eq!(entry.message, "-- Logs begin at Mon 2026-05-01 --");
    }

    #[test]
    fn priority_arguments() {
        assert_eq!(LogLevel::All.priority_arg(), None);
        assert_eq!(LogLevel::Errors.priority_arg(), Some("err"));
        assert_eq!(LogLevel::Warnings.priority_arg(), Some("warning"));
    }

    #[test]
    fn log_limits_sanity() {
        assert!(MIN_LOG_LINES < MAX_LOG_LINES);
        assert_eq!(MAX_LOG_BYTES, 512 * 1024);
    }

    #[test]
    fn request_deserializes_from_frontend_shape() {
        let request: LogRequest =
            serde_json::from_str(r#"{"service":"php-fpm","level":"errors","limit":120}"#).unwrap();
        assert_eq!(request.service, ServiceId::PhpFpm);
        assert_eq!(request.level, LogLevel::Errors);
        assert_eq!(request.limit, Some(120));

        let minimal: LogRequest = serde_json::from_str(r#"{"service":"httpd"}"#).unwrap();
        assert_eq!(minimal.level, LogLevel::All);
        assert_eq!(minimal.limit, None);

        assert!(serde_json::from_str::<LogRequest>(r#"{"service":"sshd"}"#).is_err());
    }
}
