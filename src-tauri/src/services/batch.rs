//! Batch operations (Start All / Stop All / Restart All).
//!
//! Ordering is dependency aware: the database comes up first and goes down
//! last. Each service is handled independently so a single failure never
//! prevents the rest of the batch from running, and every service always gets
//! its own result entry.

use std::time::Instant;

use crate::error::AppError;
use crate::models::service::{
    BatchOperationResult, OperationResult, ServiceId, ServiceOperation, ServiceStatus,
};
use crate::services::systemd::{now_ms, SystemdClient};

/// Services in the order they should be processed for a batch operation.
pub fn plan(operation: ServiceOperation) -> Vec<ServiceId> {
    match operation {
        ServiceOperation::Start => ServiceId::START_ALL_ORDER.to_vec(),
        ServiceOperation::Stop => ServiceId::STOP_ALL_ORDER.to_vec(),
        ServiceOperation::Restart => ServiceId::RESTART_ALL_ORDER.to_vec(),
    }
}

/// Best effort status snapshot used to fill in results that could not run.
async fn status_or_unknown(client: &SystemdClient, service: ServiceId) -> ServiceStatus {
    client
        .status(service)
        .await
        .unwrap_or_else(|_| ServiceStatus::unknown(service, now_ms()))
}

/// Run every service in `services` and collect a result for each one.
///
/// `on_progress` is invoked after every service so the UI can render progress
/// while the batch is still running.
pub async fn execute<F>(
    client: &SystemdClient,
    operation: ServiceOperation,
    services: Vec<ServiceId>,
    mut on_progress: F,
) -> BatchOperationResult
where
    F: FnMut(&OperationResult),
{
    let started_at_ms = now_ms();
    let mut results = Vec::with_capacity(services.len());

    for service in services {
        let started = Instant::now();
        let outcome = client.operate(service, operation).await;
        let duration_ms = started.elapsed().as_millis() as u64;

        let result = match outcome {
            Ok(status) => OperationResult {
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
            },
            Err(error) => {
                let status = status_or_unknown(client, service).await;
                let already_in_target_state = matches!(
                    (&error, operation),
                    (AppError::ServiceAlreadyActive(_), ServiceOperation::Start)
                        | (AppError::ServiceAlreadyInactive(_), ServiceOperation::Stop)
                );
                OperationResult {
                    service,
                    unit: service.unit().to_string(),
                    name: service.display_name().to_string(),
                    operation,
                    // Being already started/stopped is not a batch failure.
                    success: already_in_target_state,
                    message: if already_in_target_state {
                        format!(
                            "{} was already in the requested state",
                            service.display_name()
                        )
                    } else {
                        format!(
                            "{} could not be {}: {}",
                            service.display_name(),
                            operation.past_tense(),
                            error
                        )
                    },
                    status,
                    error: if already_in_target_state {
                        None
                    } else {
                        Some(error.to_payload())
                    },
                    duration_ms,
                }
            }
        };

        on_progress(&result);
        results.push(result);
    }

    let succeeded = results.iter().filter(|result| result.success).count();
    let failed = results.len() - succeeded;

    BatchOperationResult {
        operation,
        results,
        succeeded,
        failed,
        all_succeeded: failed == 0,
        started_at_ms,
        finished_at_ms: now_ms(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_plan_is_dependency_ordered() {
        assert_eq!(
            plan(ServiceOperation::Start),
            vec![ServiceId::MariaDb, ServiceId::PhpFpm, ServiceId::Httpd]
        );
    }

    #[test]
    fn stop_plan_reverses_dependency_order() {
        assert_eq!(
            plan(ServiceOperation::Stop),
            vec![ServiceId::Httpd, ServiceId::PhpFpm, ServiceId::MariaDb]
        );
    }

    #[test]
    fn restart_plan_starts_dependencies_first() {
        assert_eq!(
            plan(ServiceOperation::Restart),
            vec![ServiceId::MariaDb, ServiceId::PhpFpm, ServiceId::Httpd]
        );
    }

    #[test]
    fn every_plan_covers_all_managed_services_exactly_once() {
        for operation in [
            ServiceOperation::Start,
            ServiceOperation::Stop,
            ServiceOperation::Restart,
        ] {
            let planned = plan(operation);
            assert_eq!(planned.len(), ServiceId::ALL.len());
            for service in ServiceId::ALL {
                assert_eq!(
                    planned
                        .iter()
                        .filter(|candidate| **candidate == service)
                        .count(),
                    1,
                    "{service} must appear exactly once in the {operation} plan"
                );
            }
        }
    }

    #[test]
    fn batch_result_reports_partial_failure() {
        let now = now_ms();
        let make = |service: ServiceId, success: bool| OperationResult {
            service,
            unit: service.unit().to_string(),
            name: service.display_name().to_string(),
            operation: ServiceOperation::Start,
            success,
            status: ServiceStatus::unknown(service, now),
            error: if success {
                None
            } else {
                Some(AppError::PermissionDenied(service.unit().into()).to_payload())
            },
            message: String::new(),
            duration_ms: 1,
        };
        let results = vec![
            make(ServiceId::MariaDb, true),
            make(ServiceId::PhpFpm, false),
        ];
        let succeeded = results.iter().filter(|r| r.success).count();
        let failed = results.len() - succeeded;
        let batch = BatchOperationResult {
            operation: ServiceOperation::Start,
            results,
            succeeded,
            failed,
            all_succeeded: failed == 0,
            started_at_ms: now,
            finished_at_ms: now,
        };
        assert_eq!(batch.succeeded, 1);
        assert_eq!(batch.failed, 1);
        assert!(!batch.all_succeeded);
        assert!(batch.summary().contains("1 of 2"));
    }
}
