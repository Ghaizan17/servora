//! Structured errors shared by the Servora core, the helper and the GUI.
//!
//! Every variant maps to a stable `code` string that the frontend groups on,
//! plus an optional actionable `hint`. Nothing here ever embeds credentials or
//! environment dumps.

use serde::{Deserialize, Serialize};

/// Stable machine readable error codes.
pub mod codes {
    pub const INVALID_NAME: &str = "invalid_name";
    pub const INVALID_DOMAIN: &str = "invalid_domain";
    pub const INVALID_PATH: &str = "invalid_path";
    pub const NOT_FOUND: &str = "not_found";
    pub const DUPLICATE: &str = "duplicate";
    pub const CONFLICT: &str = "conflict";
    pub const IO_ERROR: &str = "io_error";
    pub const STORAGE_ERROR: &str = "storage_error";
    pub const HELPER_MISSING: &str = "helper_missing";
    pub const HELPER_FAILED: &str = "helper_failed";
    pub const APACHE_CONFIG_INVALID: &str = "apache_config_invalid";
    pub const STEP_FAILED: &str = "step_failed";
    pub const UNSUPPORTED: &str = "unsupported";
    pub const INVALID_INPUT: &str = "invalid_input";
    pub const INTERNAL: &str = "internal_error";
    pub const PERMISSION_DENIED: &str = "permission_denied";
}

/// Error type for every core operation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CoreError {
    #[error("`{0}` is not a valid website name")]
    InvalidName(String),

    #[error("`{0}` is not a valid domain name")]
    InvalidDomain(String),

    #[error("`{0}` is not a usable project directory")]
    InvalidPath(String),

    #[error("`{0}` could not be found")]
    NotFound(String),

    #[error("`{0}` already exists")]
    Duplicate(String),

    #[error("{0}")]
    Conflict(String),

    #[error("I/O error: {0}")]
    Io(String),

    #[error("Metadata could not be read or written: {0}")]
    Storage(String),

    #[error("The Servora helper is not installed")]
    HelperMissing,

    #[error("The Servora helper failed: {0}")]
    HelperFailed(String),

    #[error("Apache rejected the configuration: {0}")]
    ApacheConfigInvalid(String),

    #[error("Step `{step}` failed: {message}")]
    StepFailed { step: String, message: String },

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("{0}")]
    Unsupported(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Permission denied for `{0}`")]
    PermissionDenied(String),
}

impl CoreError {
    /// Stable code shared with the frontend.
    pub fn code(&self) -> &'static str {
        match self {
            CoreError::InvalidName(_) => codes::INVALID_NAME,
            CoreError::InvalidDomain(_) => codes::INVALID_DOMAIN,
            CoreError::InvalidPath(_) => codes::INVALID_PATH,
            CoreError::NotFound(_) => codes::NOT_FOUND,
            CoreError::Duplicate(_) => codes::DUPLICATE,
            CoreError::Conflict(_) => codes::CONFLICT,
            CoreError::Io(_) => codes::IO_ERROR,
            CoreError::Storage(_) => codes::STORAGE_ERROR,
            CoreError::HelperMissing => codes::HELPER_MISSING,
            CoreError::HelperFailed(_) => codes::HELPER_FAILED,
            CoreError::ApacheConfigInvalid(_) => codes::APACHE_CONFIG_INVALID,
            CoreError::StepFailed { .. } => codes::STEP_FAILED,
            CoreError::InvalidInput(_) => codes::INVALID_INPUT,
            CoreError::Unsupported(_) => codes::UNSUPPORTED,
            CoreError::Internal(_) => codes::INTERNAL,
            CoreError::PermissionDenied(_) => codes::PERMISSION_DENIED,
        }
    }

    /// Actionable remediation shown under the error in the UI.
    pub fn hint(&self) -> Option<String> {
        let hint = match self {
            CoreError::InvalidName(_) => {
                "Use letters, digits, dots and dashes, starting with a letter or digit."
            }
            CoreError::InvalidDomain(_) => {
                "A local domain looks like `my-project.test` or `shop.local`. Do not include a scheme or a path."
            }
            CoreError::InvalidPath(_) => {
                "Pick an existing directory (for example `~/Work/my-project`) that your user can read."
            }
            CoreError::Duplicate(_) => {
                "Pick another name or domain, or remove the existing website first."
            }
            CoreError::Conflict(_) => {
                "Another VirtualHost or hosts entry already uses this domain. Repair the conflict first."
            }
            CoreError::HelperMissing => {
                "Install the Servora package (`sudo dnf install servora`) so `/usr/libexec/servora-helper` and its Polkit policy exist."
            }
            CoreError::HelperFailed(_) => {
                "Run the action again and approve the Polkit prompt. If it keeps failing, check `journalctl -t servora-helper`."
            }
            CoreError::ApacheConfigInvalid(_) => {
                "Run `apachectl configtest` as root to see the exact error, then fix the reported VirtualHost."
            }
            CoreError::PermissionDenied(_) => {
                "Servora asks Polkit for permission. Make sure your user is allowed to manage local development sites."
            }
            CoreError::Io(_) | CoreError::Storage(_) => {
                "Check that `~/.local/share/servora` is writable and that the directory was not moved."
            }
            _ => return None,
        };
        Some(hint.to_string())
    }
}

impl From<std::io::Error> for CoreError {
    fn from(value: std::io::Error) -> Self {
        CoreError::Io(value.to_string())
    }
}

impl From<serde_json::Error> for CoreError {
    fn from(value: serde_json::Error) -> Self {
        CoreError::Storage(value.to_string())
    }
}

/// Payload handed to the frontend (mirrors `AppError::ErrorPayload`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoreErrorPayload {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl From<&CoreError> for CoreErrorPayload {
    fn from(value: &CoreError) -> Self {
        Self {
            code: value.code().to_string(),
            message: value.to_string(),
            hint: value.hint(),
        }
    }
}

impl From<CoreError> for CoreErrorPayload {
    fn from(value: CoreError) -> Self {
        Self::from(&value)
    }
}

pub type CoreResult<T> = Result<T, CoreError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_has_a_code() {
        let errors = vec![
            CoreError::InvalidName("x".into()),
            CoreError::InvalidDomain("x".into()),
            CoreError::InvalidPath("/nope".into()),
            CoreError::NotFound("x".into()),
            CoreError::Duplicate("x".into()),
            CoreError::Conflict("x".into()),
            CoreError::Io("x".into()),
            CoreError::Storage("x".into()),
            CoreError::HelperMissing,
            CoreError::HelperFailed("x".into()),
            CoreError::ApacheConfigInvalid("x".into()),
            CoreError::StepFailed {
                step: "write".into(),
                message: "x".into(),
            },
            CoreError::InvalidInput("x".into()),
            CoreError::Unsupported("x".into()),
            CoreError::Internal("x".into()),
            CoreError::PermissionDenied("x".into()),
        ];
        for error in errors {
            assert!(!error.code().is_empty());
            assert!(!error.to_string().is_empty());
        }
    }

    #[test]
    fn payload_carries_hint_for_user_facing_errors() {
        let payload = CoreErrorPayload::from(&CoreError::Duplicate("e-perpus".into()));
        assert_eq!(payload.code, codes::DUPLICATE);
        assert!(payload.hint.is_some());
    }

    #[test]
    fn internal_errors_have_no_hint() {
        assert!(CoreError::Internal("boom".into()).hint().is_none());
    }
}
