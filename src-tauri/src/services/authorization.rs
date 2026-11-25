//! Authorization policy.
//!
//! Servora runs as the logged in desktop user. It never runs as root, never
//! stores passwords and never calls `sudo`. Privileged work is delegated to
//! systemd over the system bus, where Polkit (`org.freedesktop.systemd1.manage-units`)
//! asks the desktop session for approval.
//!
//! This module is deliberately small: it documents the policy, exposes the
//! policy information to the Settings page and enforces the two invariants the
//! application relies on (only allowlisted units, no elevation of the GUI).

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::models::service::{ServiceId, ServiceOperation};
use crate::models::settings::is_root;

/// Polkit action used by systemd for unit management.
pub const SYSTEMD_MANAGE_UNITS_ACTION: &str = "org.freedesktop.systemd1.manage-units";

/// Human readable description of the security model, shown in the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationInfo {
    pub action_id: String,
    pub mechanism: String,
    pub runs_as_root: bool,
    pub helper_installed: bool,
    pub notes: Vec<String>,
}

pub fn info() -> AuthorizationInfo {
    AuthorizationInfo {
        action_id: SYSTEMD_MANAGE_UNITS_ACTION.to_string(),
        mechanism: "systemd D-Bus + Polkit".to_string(),
        runs_as_root: is_root(),
        helper_installed: false,
        notes: vec![
            "Servora never runs as root and never stores credentials.".to_string(),
            "Service control is sent to systemd over the system bus; systemd asks Polkit for approval.".to_string(),
            "Only httpd.service, php-fpm.service and mariadb.service can be addressed.".to_string(),
            "Arguments are fixed in Rust. No shell is involved and no command text comes from the UI."
                .to_string(),
        ],
    }
}

/// Resolve a service slug coming from the frontend and refuse anything else.
///
/// This is the single gate every command passes through before systemd is
/// touched, so the unit allowlist cannot be bypassed by a crafted payload.
pub fn authorize_service(slug: &str) -> AppResult<ServiceId> {
    slug.parse::<ServiceId>()
        .map_err(|_| AppError::UnknownService(slug.trim().to_string()))
}

/// Resolve an operation slug coming from the frontend.
pub fn authorize_operation(slug: &str) -> AppResult<ServiceOperation> {
    slug.parse::<ServiceOperation>()
        .map_err(|_| AppError::InvalidInput(format!("unsupported operation `{slug}`")))
}

/// Refuse to run the GUI itself with elevated privileges instead of failing
/// later with an unusable session.
pub fn assert_not_elevated() -> AppResult<()> {
    if is_root() {
        return Err(AppError::InvalidInput(
            "Servora must not be started as root; run it as your desktop user so Polkit can authorize service actions"
                .into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_is_enforced() {
        assert_eq!(authorize_service("httpd").unwrap(), ServiceId::Httpd);
        assert_eq!(authorize_service("php-fpm").unwrap(), ServiceId::PhpFpm);
        assert_eq!(authorize_service("mariadb").unwrap(), ServiceId::MariaDb);
        assert!(authorize_service("sshd").is_err());
        assert!(authorize_service("rm -rf /").is_err());
        assert!(authorize_service("").is_err());
    }

    #[test]
    fn operation_allowlist_is_enforced() {
        assert_eq!(authorize_operation("start").unwrap(), ServiceOperation::Start);
        assert_eq!(authorize_operation("STOP").unwrap(), ServiceOperation::Stop);
        assert_eq!(
            authorize_operation("restart").unwrap(),
            ServiceOperation::Restart
        );
        assert!(authorize_operation("enable").is_err());
        assert!(authorize_operation("; reboot").is_err());
    }

    #[test]
    fn policy_documentation_is_exposed() {
        let info = info();
        assert_eq!(info.action_id, SYSTEMD_MANAGE_UNITS_ACTION);
        assert!(info.mechanism.contains("Polkit"));
        assert!(!info.notes.is_empty());
    }
}
