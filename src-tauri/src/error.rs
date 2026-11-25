//! Structured error handling for every fallible operation exposed through Tauri IPC.
//!
//! Every error carries a stable machine-readable `code`, a human readable
//! `message` and an optional `hint` that the UI can surface to the user.
//! Errors never carry credentials, tokens or environment dumps.

use serde::{Deserialize, Serialize};

/// Stable error codes shared with the frontend (`src/lib/errors.js`).
pub mod codes {
    pub const UNKNOWN_SERVICE: &str = "unknown_service";
    pub const SERVICE_NOT_INSTALLED: &str = "service_not_installed";
    pub const SERVICE_NOT_FOUND: &str = "service_not_found";
    pub const SERVICE_ALREADY_ACTIVE: &str = "service_already_active";
    pub const SERVICE_ALREADY_INACTIVE: &str = "service_already_inactive";
    pub const PERMISSION_DENIED: &str = "permission_denied";
    pub const AUTHORIZATION_CANCELLED: &str = "authorization_cancelled";
    pub const SYSTEMD_UNAVAILABLE: &str = "systemd_unavailable";
    pub const DBUS_UNAVAILABLE: &str = "dbus_unavailable";
    pub const OPERATION_TIMEOUT: &str = "operation_timeout";
    pub const OPERATION_FAILED: &str = "operation_failed";
    pub const LOG_RETRIEVAL_FAILED: &str = "log_retrieval_failed";
    pub const CONFIG_ERROR: &str = "config_error";
    pub const INVALID_INPUT: &str = "invalid_input";
    // Website manager
    pub const INVALID_NAME: &str = "invalid_name";
    pub const INVALID_DOMAIN: &str = "invalid_domain";
    pub const INVALID_PATH: &str = "invalid_path";
    pub const NOT_FOUND: &str = "not_found";
    pub const DUPLICATE: &str = "duplicate";
    pub const CONFLICT: &str = "conflict";
    pub const STORAGE_ERROR: &str = "storage_error";
    pub const HELPER_MISSING: &str = "helper_missing";
    pub const HELPER_FAILED: &str = "helper_failed";
    pub const APACHE_CONFIG_INVALID: &str = "apache_config_invalid";
    pub const NOT_IMPLEMENTED_ON_PLATFORM: &str = "not_supported";
    pub const INTERNAL_ERROR: &str = "internal_error";
}

/// Serialized error payload returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorPayload {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl ErrorPayload {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            hint: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

/// Application level error type.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum AppError {
    #[error("`{0}` is not a service managed by Servora")]
    UnknownService(String),

    #[error("Unit `{0}` is not installed on this system")]
    ServiceNotInstalled(String),

    #[error("Unit `{0}` could not be found in systemd")]
    ServiceNotFound(String),

    #[error("`{0}` is already running")]
    ServiceAlreadyActive(String),

    #[error("`{0}` is already stopped")]
    ServiceAlreadyInactive(String),

    #[error("Authorization was denied for `{0}`")]
    PermissionDenied(String),

    #[error("Authorization request for `{0}` was cancelled")]
    AuthorizationCancelled(String),

    #[error("systemd is not available on this system")]
    SystemdUnavailable,

    #[error("Could not reach the system D-Bus: {0}")]
    DbusUnavailable(String),

    #[error("The operation on `{0}` timed out")]
    OperationTimeout(String),

    #[error("The operation on `{0}` failed: {1}")]
    OperationFailed(String, String),

    #[error("Logs could not be retrieved: {0}")]
    LogRetrievalFailed(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Internal error: {0}")]
    Internal(String),

    /// Error raised by `servora-core` with its own code and hint.
    #[error("{message}")]
    Site {
        code: String,
        message: String,
        hint: Option<String>,
    },
}

impl AppError {
    /// Stable machine readable code for the frontend and for logs.
    pub fn code(&self) -> String {
        if let AppError::Site { code, .. } = self {
            return code.clone();
        }
        self.static_code().to_string()
    }

    fn static_code(&self) -> &'static str {
        match self {
            AppError::UnknownService(_) => codes::UNKNOWN_SERVICE,
            AppError::ServiceNotInstalled(_) => codes::SERVICE_NOT_INSTALLED,
            AppError::ServiceNotFound(_) => codes::SERVICE_NOT_FOUND,
            AppError::ServiceAlreadyActive(_) => codes::SERVICE_ALREADY_ACTIVE,
            AppError::ServiceAlreadyInactive(_) => codes::SERVICE_ALREADY_INACTIVE,
            AppError::PermissionDenied(_) => codes::PERMISSION_DENIED,
            AppError::AuthorizationCancelled(_) => codes::AUTHORIZATION_CANCELLED,
            AppError::SystemdUnavailable => codes::SYSTEMD_UNAVAILABLE,
            AppError::DbusUnavailable(_) => codes::DBUS_UNAVAILABLE,
            AppError::OperationTimeout(_) => codes::OPERATION_TIMEOUT,
            AppError::OperationFailed(_, _) => codes::OPERATION_FAILED,
            AppError::LogRetrievalFailed(_) => codes::LOG_RETRIEVAL_FAILED,
            AppError::Config(_) => codes::CONFIG_ERROR,
            AppError::InvalidInput(_) => codes::INVALID_INPUT,
            AppError::Internal(_) => codes::INTERNAL_ERROR,
            AppError::Site { .. } => codes::INTERNAL_ERROR,
        }
    }

    /// Actionable hint shown under the error message in the UI.
    pub fn hint(&self) -> Option<String> {
        if let AppError::Site { hint, .. } = self {
            return hint.clone();
        }
        let hint = match self {
            AppError::ServiceNotInstalled(_) => {
                "Install the package first, for example `sudo dnf install httpd php-fpm mariadb-server`."
            }
            AppError::PermissionDenied(_) => {
                "Servora asks Polkit for permission. Make sure your user is allowed to manage system units."
            }
            AppError::AuthorizationCancelled(_) => {
                "The Polkit dialog was dismissed. Run the action again and authenticate to continue."
            }
            AppError::SystemdUnavailable => {
                "This system does not run systemd (containers usually do not). Servora needs systemd to manage services."
            }
            AppError::DbusUnavailable(_) => {
                "The system D-Bus could not be reached. Check that `dbus-broker` or `dbus-daemon` is running."
            }
            AppError::OperationTimeout(_) => {
                "The request timed out. If a Polkit authentication dialog was left unanswered, run the action again and authenticate. Otherwise check `journalctl -u <unit>` for details."
            }
            AppError::LogRetrievalFailed(_) => {
                "Make sure `journalctl` is available and your user can read the journal (`usermod -aG systemd-journal $USER`)."
            }
            AppError::Config(_) => "Fix or delete the configuration file and try again.",
            _ => return None,
        };
        Some(hint.to_string())
    }

    pub fn to_payload(&self) -> ErrorPayload {
        let mut payload = ErrorPayload::new(self.code(), self.to_string());
        if let Some(hint) = self.hint() {
            payload = payload.with_hint(hint);
        }
        payload
    }
}

impl From<serde_json::Error> for AppError {
    fn from(value: serde_json::Error) -> Self {
        AppError::Config(format!("could not read or write JSON: {value}"))
    }
}

impl From<std::io::Error> for AppError {
    fn from(value: std::io::Error) -> Self {
        AppError::Internal(format!("I/O error: {value}"))
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_has_a_code_and_message() {
        let errors = vec![
            AppError::UnknownService("foo.service".into()),
            AppError::ServiceNotInstalled("httpd.service".into()),
            AppError::ServiceNotFound("httpd.service".into()),
            AppError::ServiceAlreadyActive("httpd".into()),
            AppError::ServiceAlreadyInactive("httpd".into()),
            AppError::PermissionDenied("httpd".into()),
            AppError::AuthorizationCancelled("httpd".into()),
            AppError::SystemdUnavailable,
            AppError::DbusUnavailable("no bus".into()),
            AppError::OperationTimeout("httpd".into()),
            AppError::OperationFailed("httpd".into(), "exit 1".into()),
            AppError::LogRetrievalFailed("boom".into()),
            AppError::Config("bad".into()),
            AppError::InvalidInput("negative interval".into()),
            AppError::Internal("oops".into()),
        ];

        for error in errors {
            let payload = error.to_payload();
            assert!(!payload.code.is_empty(), "missing code for {error}");
            assert!(!payload.message.is_empty(), "missing message for {error}");
        }
    }

    #[test]
    fn serializes_to_code_and_message() {
        let payload = AppError::PermissionDenied("httpd.service".into()).to_payload();
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["code"], codes::PERMISSION_DENIED);
        assert!(json["message"].as_str().unwrap().contains("httpd.service"));
        assert!(json.get("hint").is_some());
    }

    #[test]
    fn technical_errors_do_not_suggest_a_hint() {
        let payload = AppError::UnknownService("x".into()).to_payload();
        assert!(payload.hint.is_none());
    }
}

impl From<servora_core::CoreError> for AppError {
    fn from(value: servora_core::CoreError) -> Self {
        AppError::Site {
            code: value.code().to_string(),
            message: value.to_string(),
            hint: value.hint(),
        }
    }
}

#[cfg(test)]
mod site_error_tests {
    use super::*;

    #[test]
    fn core_errors_keep_their_code_and_hint() {
        let error = AppError::from(servora_core::CoreError::Duplicate("e-perpus".into()));
        assert_eq!(error.code(), codes::DUPLICATE);
        assert_eq!(error.to_payload().code, codes::DUPLICATE);
        assert!(error.hint().is_some());
    }

    #[test]
    fn service_errors_still_use_their_static_codes() {
        assert_eq!(
            AppError::PermissionDenied("httpd".into()).code(),
            codes::PERMISSION_DENIED
        );
    }
}
