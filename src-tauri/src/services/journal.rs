//! Journal retrieval through `journalctl`.
//!
//! The command line is built here from fixed arguments only: the unit name
//! comes from [`ServiceId`], the line count is clamped to a hard range and the
//! priority filter is one of three literals. No string from the frontend is
//! ever interpolated into a shell — the process is spawned directly.

use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::error::{AppError, AppResult};
use crate::models::logs::{LogEntry, LogLevel, LogResponse, MAX_LOG_BYTES};
use crate::models::service::ServiceId;
use crate::models::settings::{MAX_LOG_LINES, MIN_LOG_LINES};
use crate::services::systemd::now_ms;

const JOURNALCTL_TIMEOUT: Duration = Duration::from_secs(20);
const JOURNALCTL: &str = "journalctl";

/// Clamp a requested line count into the supported range.
pub fn clamp_limit(requested: Option<u32>, fallback: u32) -> u32 {
    let value = requested.unwrap_or(fallback);
    value.clamp(MIN_LOG_LINES, MAX_LOG_LINES)
}

/// Build the argument vector. Kept separate so it can be asserted in tests.
pub fn build_args(service: ServiceId, level: LogLevel, limit: u32) -> Vec<String> {
    let mut args = vec![
        "--unit".to_string(),
        service.unit().to_string(),
        "--lines".to_string(),
        limit.to_string(),
        "--no-pager".to_string(),
        "--output=short-iso".to_string(),
        "--quiet".to_string(),
    ];
    if let Some(priority) = level.priority_arg() {
        args.push("--priority".to_string());
        args.push(priority.to_string());
    }
    args
}

/// Retrieve journal entries for one managed service.
pub async fn fetch_logs(
    service: ServiceId,
    level: LogLevel,
    requested_limit: Option<u32>,
    fallback_limit: u32,
) -> AppResult<LogResponse> {
    let limit = clamp_limit(requested_limit, fallback_limit);
    let args = build_args(service, level, limit);

    let mut child = Command::new(JOURNALCTL)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| {
            AppError::LogRetrievalFailed(format!("could not run `{JOURNALCTL}`: {err}"))
        })?;

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::LogRetrievalFailed("journalctl stdout was unavailable".into()))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| AppError::LogRetrievalFailed("journalctl stderr was unavailable".into()))?;

    let mut buffer: Vec<u8> = Vec::new();
    let read = async {
        let mut chunk = [0u8; 8192];
        loop {
            match stdout.read(&mut chunk).await {
                Ok(0) => break,
                Ok(n) => {
                    if buffer.len() < MAX_LOG_BYTES {
                        let remaining = MAX_LOG_BYTES - buffer.len();
                        let take = n.min(remaining);
                        buffer.extend_from_slice(&chunk[..take]);
                    }
                }
                Err(err) => return Err(err),
            }
        }
        Ok::<_, std::io::Error>(())
    };

    let read_result = tokio::time::timeout(JOURNALCTL_TIMEOUT, read).await;
    match read_result {
        Ok(Ok(())) => {}
        Ok(Err(err)) => {
            let _ = child.kill().await;
            return Err(AppError::LogRetrievalFailed(format!(
                "could not read journal output: {err}"
            )));
        }
        Err(_) => {
            let _ = child.kill().await;
            return Err(AppError::OperationTimeout(service.unit().into()));
        }
    }

    let mut err_buffer = Vec::new();
    let _ = stderr.read_to_end(&mut err_buffer).await;
    let status = child
        .wait()
        .await
        .map_err(|err| AppError::LogRetrievalFailed(format!("journalctl did not exit: {err}")))?;

    if !status.success() {
        let message = String::from_utf8_lossy(&err_buffer).trim().to_string();
        let message = if message.is_empty() {
            format!("journalctl exited with {status}")
        } else {
            message
        };
        return Err(AppError::LogRetrievalFailed(message));
    }

    let truncated = buffer.len() >= MAX_LOG_BYTES;
    let text = String::from_utf8_lossy(&buffer);
    let mut entries: Vec<LogEntry> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(LogEntry::parse_line)
        .collect();

    // Defence in depth: never hand more than the requested number of lines back.
    if entries.len() > limit as usize {
        entries = entries.split_off(entries.len() - limit as usize);
    }

    let count = entries.len();
    Ok(LogResponse {
        service,
        unit: service.unit().to_string(),
        level,
        entries,
        count,
        truncated,
        requested_lines: limit,
        retrieved_at_ms: now_ms(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_are_fixed_and_safe() {
        let args = build_args(ServiceId::Httpd, LogLevel::All, 200);
        assert_eq!(args[0], "--unit");
        assert_eq!(args[1], "httpd.service");
        assert_eq!(args[2], "--lines");
        assert_eq!(args[3], "200");
        assert!(!args.iter().any(|arg| arg == "-f"));
        assert!(!args.iter().any(|arg| arg.contains(';')));
        assert!(!args.iter().any(|arg| arg.contains("shell")));
    }

    #[test]
    fn priority_filter_is_opt_in() {
        let args = build_args(ServiceId::MariaDb, LogLevel::Errors, 100);
        let position = args.iter().position(|arg| arg == "--priority").unwrap();
        assert_eq!(args[position + 1], "err");

        let args = build_args(ServiceId::MariaDb, LogLevel::Warnings, 100);
        let position = args.iter().position(|arg| arg == "--priority").unwrap();
        assert_eq!(args[position + 1], "warning");

        let args = build_args(ServiceId::MariaDb, LogLevel::All, 100);
        assert!(!args.iter().any(|arg| arg == "--priority"));
    }

    #[test]
    fn every_managed_unit_can_be_queried() {
        for service in ServiceId::ALL {
            let args = build_args(service, LogLevel::All, 10);
            assert_eq!(args[1], service.unit());
        }
    }

    #[test]
    fn limits_are_clamped() {
        assert_eq!(clamp_limit(None, 300), 300);
        assert_eq!(clamp_limit(Some(0), 300), MIN_LOG_LINES);
        assert_eq!(clamp_limit(Some(1), 300), MIN_LOG_LINES);
        assert_eq!(clamp_limit(Some(10_000_000), 300), MAX_LOG_LINES);
        assert_eq!(clamp_limit(Some(500), 300), 500);
    }
}
