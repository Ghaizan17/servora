//! Privileged plan execution.
//!
//! The GUI process never writes to `/etc`. Plans produced by `servora-core` are
//! serialised to JSON and handed to `servora-helper` through `pkexec`, which
//! asks Polkit (`dev.servora.app.manage-sites`) for authorization. If the user
//! dismisses the dialog nothing is written and the caller receives a typed
//! `authorization_cancelled` error.

use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;

use servora_core::plan::{Plan, PlanOutcome};

use crate::error::{AppError, AppResult};
use crate::services::logging;

/// How long an authenticated helper run may take (includes Apache reload).
const HELPER_TIMEOUT: Duration = Duration::from_secs(180);

/// pkexec exit codes documented in `pkexec(1)`.
const PKEXEC_AUTH_FAILED: i32 = 126;
const PKEXEC_NOT_AUTHORIZED: i32 = 127;

/// Is the helper installed where the Polkit policy expects it?
pub fn helper_available(helper_path: &std::path::Path) -> bool {
    helper_path.exists()
}

/// Execute `plan` through the privileged helper.
pub async fn apply(plan: &Plan, helper_path: &std::path::Path) -> AppResult<PlanOutcome> {
    if !helper_available(helper_path) {
        return Err(AppError::from(servora_core::CoreError::HelperMissing));
    }

    let payload = serde_json::to_string(plan)
        .map_err(|err| AppError::Internal(format!("could not encode the plan: {err}")))?;

    logging::info(&format!(
        "requesting authorization for {} on {}",
        plan.operation,
        plan.website.as_deref().unwrap_or("(no website)")
    ));

    let mut child = Command::new("pkexec")
        .arg(helper_path)
        .arg("--apply-plan")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| {
            AppError::from(servora_core::CoreError::HelperFailed(format!(
                "could not start pkexec: {err}"
            )))
        })?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(payload.as_bytes()).await.map_err(|err| {
            AppError::from(servora_core::CoreError::HelperFailed(format!(
                "could not send the plan: {err}"
            )))
        })?;
        stdin.shutdown().await.ok();
    }

    let output = match timeout(HELPER_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(err)) => {
            return Err(AppError::from(servora_core::CoreError::HelperFailed(
                format!("the helper could not be waited for: {err}"),
            )))
        }
        Err(_) => {
            return Err(AppError::OperationTimeout(
                plan.website
                    .clone()
                    .unwrap_or_else(|| plan.operation.clone()),
            ))
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let status = output.status.code();

    if !stderr.is_empty() {
        logging::warn(&format!("servora-helper: {stderr}"));
    }

    // Polkit refusals happen before the helper ever runs.
    if status == Some(PKEXEC_AUTH_FAILED) {
        return Err(AppError::AuthorizationCancelled(
            plan.website
                .clone()
                .unwrap_or_else(|| plan.operation.clone()),
        ));
    }
    if status == Some(PKEXEC_NOT_AUTHORIZED) {
        return Err(AppError::PermissionDenied(
            plan.website
                .clone()
                .unwrap_or_else(|| plan.operation.clone()),
        ));
    }

    match serde_json::from_str::<PlanOutcome>(&stdout) {
        Ok(outcome) => Ok(outcome),
        Err(err) => Err(AppError::from(servora_core::CoreError::HelperFailed(
            if stderr.is_empty() {
                format!("the helper returned no usable result ({err})")
            } else {
                stderr
            },
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn missing_helper_is_reported_as_such() {
        assert!(!helper_available(Path::new("/nonexistent/servora-helper")));
    }

    #[tokio::test]
    async fn applying_without_the_helper_fails_cleanly() {
        let plan = Plan::new("create", Some("e-perpus".into()));
        let error = apply(&plan, Path::new("/nonexistent/servora-helper"))
            .await
            .unwrap_err();
        assert_eq!(error.code(), crate::error::codes::HELPER_MISSING);
        assert!(error.hint().is_some());
    }
}
