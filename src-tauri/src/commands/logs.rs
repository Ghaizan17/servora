//! Tauri commands for the journal log viewer.

use serde::Serialize;
use tauri::State;

use crate::error::ErrorPayload;
use crate::models::logs::{LogRequest, LogResponse};
use crate::models::settings::{MAX_LOG_LINES, MIN_LOG_LINES};
use crate::services::journal;
use crate::services::logging;
use crate::state::AppState;

/// Extra context for the Logs page.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogsContext {
    pub default_line_limit: u32,
    pub min_line_limit: u32,
    pub max_line_limit: u32,
}

#[tauri::command]
pub async fn get_service_logs(
    request: LogRequest,
    state: State<'_, AppState>,
) -> Result<LogResponse, ErrorPayload> {
    let settings = state.settings();
    match journal::fetch_logs(
        request.service,
        request.level,
        request.limit,
        settings.log_line_limit,
    )
    .await
    {
        Ok(response) => Ok(response),
        Err(error) => {
            logging::warn(&format!(
                "log retrieval for {} failed: {error}",
                request.service.unit()
            ));
            Err(error.to_payload())
        }
    }
}

#[tauri::command]
pub fn get_logs_context(state: State<'_, AppState>) -> LogsContext {
    LogsContext {
        default_line_limit: state.settings().log_line_limit,
        min_line_limit: MIN_LOG_LINES,
        max_line_limit: MAX_LOG_LINES,
    }
}
