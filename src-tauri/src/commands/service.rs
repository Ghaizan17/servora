//! Tauri commands for service status and control.
//!
//! Every command returns a structured payload. Status commands never fail in a
//! way that blanks the dashboard: when systemd is unreachable the response is
//! marked `degraded` so the UI can explain what happened.

use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::error::{AppError, ErrorPayload};
use crate::models::service::{
    BatchOperationResult, OperationResult, ServiceId, ServiceOperation, ServiceStatus,
    ServicesStatusResponse,
};
use crate::services::authorization::{authorize_operation, authorize_service};
use crate::services::batch;
use crate::services::logging;
use crate::services::systemd::now_ms;
use crate::state::AppState;

/// Event name used to stream batch progress to the UI.
pub const PROGRESS_EVENT: &str = "servora://batch-progress";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressPayload {
    operation: ServiceOperation,
    completed: usize,
    total: usize,
    result: OperationResult,
}

#[tauri::command]
pub async fn get_services_status(
    state: State<'_, AppState>,
) -> Result<ServicesStatusResponse, ErrorPayload> {
    match collect_statuses(&state).await {
        Ok(services) => Ok(ServicesStatusResponse::from_statuses(services, now_ms())),
        Err(error) => {
            logging::warn(&format!("status refresh failed: {error}"));
            state.reset_client().await;
            Ok(ServicesStatusResponse::degraded(error.to_payload(), now_ms()))
        }
    }
}

#[tauri::command]
pub async fn get_service_status(
    service: String,
    state: State<'_, AppState>,
) -> Result<ServiceStatus, ErrorPayload> {
    let service = authorize_service(&service).map_err(|error| error.to_payload())?;
    let client = state
        .client()
        .await
        .map_err(|error| error.to_payload())?;
    client
        .status(service)
        .await
        .map_err(|error| error.to_payload())
}

async fn collect_statuses(state: &State<'_, AppState>) -> Result<Vec<ServiceStatus>, AppError> {
    let client = state.client().await?;
    client.statuses().await
}

#[tauri::command]
pub async fn start_service(
    service: String,
    state: State<'_, AppState>,
) -> Result<OperationResult, ErrorPayload> {
    run_single(&state, &service, ServiceOperation::Start).await
}

#[tauri::command]
pub async fn stop_service(
    service: String,
    state: State<'_, AppState>,
) -> Result<OperationResult, ErrorPayload> {
    run_single(&state, &service, ServiceOperation::Stop).await
}

#[tauri::command]
pub async fn restart_service(
    service: String,
    state: State<'_, AppState>,
) -> Result<OperationResult, ErrorPayload> {
    run_single(&state, &service, ServiceOperation::Restart).await
}

async fn run_single(
    state: &State<'_, AppState>,
    service: &str,
    operation: ServiceOperation,
) -> Result<OperationResult, ErrorPayload> {
    let service = authorize_service(service).map_err(|error| error.to_payload())?;
    let client = state.client().await.map_err(|error| error.to_payload())?;

    let started = Instant::now();
    let outcome = client.operate(service, operation).await;
    let duration_ms = started.elapsed().as_millis() as u64;

    match outcome {
        Ok(status) => {
            logging::info(&format!("{operation} {} succeeded", service.unit()));
            Ok(OperationResult {
                service,
                unit: service.unit().to_string(),
                name: service.display_name().to_string(),
                operation,
                success: true,
                message: format!(
                    "{} {} successfully",
                    service.display_name(),
                    operation.past_tense()
                ),
                status,
                error: None,
                duration_ms,
            })
        }
        Err(error) => {
            logging::warn(&format!(
                "{operation} {} failed: {error}",
                service.unit()
            ));
            if matches!(
                error,
                AppError::DbusUnavailable(_) | AppError::SystemdUnavailable
            ) {
                state.reset_client().await;
            }
            Err(error.to_payload())
        }
    }
}

#[tauri::command]
pub async fn start_all_services(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<BatchOperationResult, ErrorPayload> {
    run_batch(&app, &state, ServiceOperation::Start).await
}

#[tauri::command]
pub async fn stop_all_services(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<BatchOperationResult, ErrorPayload> {
    run_batch(&app, &state, ServiceOperation::Stop).await
}

#[tauri::command]
pub async fn restart_all_services(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<BatchOperationResult, ErrorPayload> {
    run_batch(&app, &state, ServiceOperation::Restart).await
}

/// Run a batch operation, streaming one progress event per finished service.
pub async fn run_batch<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &State<'_, AppState>,
    operation: ServiceOperation,
) -> Result<BatchOperationResult, ErrorPayload> {
    let client = state.client().await.map_err(|error| error.to_payload())?;
    let plan = batch::plan(operation);
    let total = plan.len();

    let mut completed = 0usize;
    let result = batch::execute(&client, operation, plan, |partial| {
        completed += 1;
        let payload = ProgressPayload {
            operation,
            completed,
            total,
            result: partial.clone(),
        };
        let _ = app.emit(PROGRESS_EVENT, payload);
    })
    .await;

    logging::info(&format!(
        "batch {operation} finished: {}",
        result.summary()
    ));

    Ok(result)
}

/// Re-exported so the tray can reuse the same authorization gate.
pub fn gate(service: &str) -> Result<ServiceId, ErrorPayload> {
    authorize_service(service).map_err(|error| error.to_payload())
}

/// Re-exported so the tray can reuse operation parsing.
pub fn gate_operation(operation: &str) -> Result<ServiceOperation, ErrorPayload> {
    authorize_operation(operation).map_err(|error| error.to_payload())
}
