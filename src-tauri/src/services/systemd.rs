//! systemd integration over the system D-Bus.
//!
//! Servora never shells out and never builds command lines. Every privileged
//! action is a D-Bus method call on `org.freedesktop.systemd1.Manager`, which
//! makes systemd itself the authorization boundary: Polkit checks
//! `org.freedesktop.systemd1.manage-units` and shows the desktop
//! authentication dialog for the calling session.
//!
//! Only unit names coming from [`ServiceId`] can reach this module, so no
//! arbitrary unit or command can be requested through the IPC layer.

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use zbus::proxy::MethodFlags;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};
use zbus::{Connection, Proxy};

use crate::error::{AppError, AppResult};
use crate::models::service::{ServiceId, ServiceOperation, ServiceState, ServiceStatus};

const SYSTEMD_DESTINATION: &str = "org.freedesktop.systemd1";
const SYSTEMD_MANAGER: &str = "org.freedesktop.systemd1.Manager";
const SYSTEMD_UNIT: &str = "org.freedesktop.systemd1.Unit";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const SYSTEMD_MANAGER_PATH: &str = "/org/freedesktop/systemd1";

/// How long a D-Bus method may block waiting for the Polkit dialog.
const AUTHORIZATION_TIMEOUT: Duration = Duration::from_secs(120);
/// How long we wait for the unit to reach the requested state.
const TRANSITION_TIMEOUT: Duration = Duration::from_secs(30);
/// Poll interval while waiting for a transition.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// Subset of `ListUnits` entries we care about. Kept for completeness and for
/// diagnostics; status lookups use `LoadUnit` + property reads.
#[derive(Debug, Deserialize, zbus::zvariant::Type)]
#[allow(dead_code)]
struct UnitInfo {
    name: String,
    description: String,
    load_state: String,
    active_state: String,
    sub_state: String,
    following: String,
    path: OwnedObjectPath,
    job_id: u32,
    job_type: String,
    job_path: OwnedObjectPath,
}

/// Thin, typed wrapper around the systemd manager.
///
/// The manager proxy is created once and always called with the
/// `AllowInteractiveAuth` flag, so Polkit is permitted to show the desktop
/// authentication dialog instead of failing with "interactive authentication
/// has not been enabled by the calling program".
#[derive(Clone)]
pub struct SystemdClient {
    connection: Connection,
    manager: Proxy<'static>,
}

impl SystemdClient {
    /// Connect to the system bus. Fails fast when systemd / D-Bus is missing.
    pub async fn connect() -> AppResult<Self> {
        let connection = tokio::time::timeout(Duration::from_secs(10), Connection::system())
            .await
            .map_err(|_| AppError::DbusUnavailable("connection attempt timed out".into()))?
            .map_err(|err| classify_connection_error(&err))?;

        let manager = Proxy::new_owned(
            connection.clone(),
            SYSTEMD_DESTINATION,
            SYSTEMD_MANAGER_PATH,
            SYSTEMD_MANAGER,
        )
        .await
        .map_err(|err| classify_connection_error(&err))?;

        Ok(Self {
            connection,
            manager,
        })
    }

    /// Read-only status for one managed service.
    pub async fn status(&self, service: ServiceId) -> AppResult<ServiceStatus> {
        let unit = service.unit();
        let path = match self.load_unit(unit).await {
            Ok(path) => path,
            Err(AppError::ServiceNotInstalled(_)) | Err(AppError::ServiceNotFound(_)) => {
                return Ok(ServiceStatus::unknown(service, now_ms()));
            }
            Err(other) => return Err(other),
        };
        let properties = self.unit_properties(&path).await?;
        Ok(build_status(service, &properties))
    }

    /// Read-only status for all managed services, in display order.
    ///
    /// A unit that is not installed is reported as `unknown` instead of
    /// failing the whole refresh, so one missing package never blanks the UI.
    pub async fn statuses(&self) -> AppResult<Vec<ServiceStatus>> {
        let mut result = Vec::with_capacity(ServiceId::ALL.len());
        for service in ServiceId::ALL {
            match self.status(service).await {
                Ok(status) => result.push(status),
                Err(AppError::DbusUnavailable(message)) => {
                    return Err(AppError::DbusUnavailable(message))
                }
                Err(AppError::SystemdUnavailable) => return Err(AppError::SystemdUnavailable),
                Err(error) => {
                    log::warn!("status for {} failed: {error}", service.unit());
                    result.push(ServiceStatus::unknown(service, now_ms()));
                }
            }
        }
        Ok(result)
    }

    /// Perform start / stop / restart and wait until the unit settles.
    pub async fn operate(
        &self,
        service: ServiceId,
        operation: ServiceOperation,
    ) -> AppResult<ServiceStatus> {
        let unit = service.unit();
        let path = self.load_unit(unit).await?;
        let properties = self.unit_properties(&path).await?;
        let before = build_status(service, &properties);

        if before.masked {
            return Err(AppError::OperationFailed(
                unit.to_string(),
                "the unit is masked and cannot be controlled".into(),
            ));
        }

        match operation {
            ServiceOperation::Start => {
                if before.state.is_running() || before.state == ServiceState::Activating {
                    return Err(AppError::ServiceAlreadyActive(service.display_name().into()));
                }
                if before.state == ServiceState::Deactivating {
                    return Err(AppError::OperationFailed(
                        unit.to_string(),
                        "the service is currently stopping, try again in a moment".into(),
                    ));
                }
            }
            ServiceOperation::Stop => {
                if before.state == ServiceState::Inactive {
                    return Err(AppError::ServiceAlreadyInactive(
                        service.display_name().into(),
                    ));
                }
            }
            ServiceOperation::Restart => {}
        }

        let method = match operation {
            ServiceOperation::Start => "StartUnit",
            ServiceOperation::Stop => "StopUnit",
            ServiceOperation::Restart => "RestartUnit",
        };

        self.call_manager(method, &(unit, "replace"), unit).await?;

        self.wait_for_settled(service, operation).await
    }

    /// `LoadUnit` returns the object path of a unit, loading it if necessary.
    async fn load_unit(&self, unit: &str) -> AppResult<OwnedObjectPath> {
        self.call_manager("LoadUnit", &(unit,), unit).await
    }

    /// Invoke a `org.freedesktop.systemd1.Manager` method that may require
    /// Polkit approval. All three job methods return the job object path.
    async fn call_manager<B>(&self, method: &str, body: &B, unit: &str) -> AppResult<OwnedObjectPath>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        let call = self.manager.call_with_flags(
            method,
            MethodFlags::AllowInteractiveAuth.into(),
            body,
        );
        match tokio::time::timeout(AUTHORIZATION_TIMEOUT, call).await {
            Ok(Ok(Some(path))) => Ok(path),
            Ok(Ok(None)) => Err(AppError::Internal(format!(
                "systemd returned no object path for {method}"
            ))),
            Ok(Err(err)) => Err(classify_dbus_error(unit, &err)),
            Err(_) => Err(AppError::OperationTimeout(unit.to_string())),
        }
    }

    async fn unit_properties(&self, path: &OwnedObjectPath) -> AppResult<HashMap<String, OwnedValue>> {
        let call = self.connection.call_method(
            Some(SYSTEMD_DESTINATION),
            path.as_str(),
            Some(PROPERTIES),
            "GetAll",
            &(SYSTEMD_UNIT,),
        );
        let reply = match tokio::time::timeout(AUTHORIZATION_TIMEOUT, call).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(err)) => return Err(classify_dbus_error(path.as_str(), &err)),
            Err(_) => return Err(AppError::OperationTimeout(path.as_str().to_string())),
        };
        reply
            .body()
            .deserialize()
            .map_err(|err| AppError::Internal(format!("unexpected GetAll reply: {err}")))
    }

    /// Poll the unit until the operation is visibly finished.
    async fn wait_for_settled(
        &self,
        service: ServiceId,
        operation: ServiceOperation,
    ) -> AppResult<ServiceStatus> {
        let deadline = Instant::now() + TRANSITION_TIMEOUT;
        let mut last = self.status(service).await?;

        loop {
            let settled = match operation {
                ServiceOperation::Start | ServiceOperation::Restart => {
                    last.state.is_running() || last.state == ServiceState::Failed
                }
                ServiceOperation::Stop => last.state.is_stopped(),
            };

            if settled {
                break;
            }
            if Instant::now() >= deadline {
                return Err(AppError::OperationTimeout(service.display_name().into()));
            }
            tokio::time::sleep(POLL_INTERVAL).await;
            last = self.status(service).await?;
        }

        if last.state == ServiceState::Failed && operation != ServiceOperation::Stop {
            return Err(AppError::OperationFailed(
                service.unit().to_string(),
                format!(
                    "unit entered the `failed` state ({})",
                    if last.sub_state.is_empty() {
                        "no sub-state"
                    } else {
                        last.sub_state.as_str()
                    }
                ),
            ));
        }

        Ok(last)
    }
}

/// Turn the raw `GetAll` property map into a [`ServiceStatus`].
fn build_status(service: ServiceId, properties: &HashMap<String, OwnedValue>) -> ServiceStatus {
    let active_state = prop_string(properties, "ActiveState").unwrap_or_default();
    let sub_state = prop_string(properties, "SubState").unwrap_or_default();
    let load_state = prop_string(properties, "LoadState").unwrap_or_else(|| "loaded".into());
    let unit_file_state = prop_string(properties, "UnitFileState").unwrap_or_default();
    let description = prop_string(properties, "Description")
        .unwrap_or_else(|| service.display_name().to_string());

    ServiceStatus {
        id: service,
        unit: service.unit().to_string(),
        name: service.display_name().to_string(),
        state: ServiceState::parse(&active_state),
        sub_state,
        masked: load_state == "masked" || unit_file_state.starts_with("masked"),
        installed: load_state != "not-found" && load_state != "error",
        load_state,
        unit_file_state,
        description,
        observed_at_ms: now_ms(),
    }
}

fn prop_string(properties: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    let value = properties.get(key)?;
    if let Ok(text) = value.downcast_ref::<&str>() {
        return Some(text.to_string());
    }
    if let Ok(text) = value.downcast_ref::<String>() {
        return Some(text.clone());
    }
    String::try_from(value.clone()).ok()
}

/// Map a D-Bus method error onto a user facing [`AppError`].
fn classify_dbus_error(unit: &str, err: &zbus::Error) -> AppError {
    if let zbus::Error::MethodError(name, detail, _) = err {
        let name = name.as_str();
        let detail = detail.clone().unwrap_or_default();
        let lower = detail.to_lowercase();
        return match name {
            "org.freedesktop.systemd1.NoSuchUnit"
            | "org.freedesktop.systemd1.NoSuchUnitFile"
            | "org.freedesktop.systemd1.LoadFailed" => AppError::ServiceNotInstalled(unit.into()),
            "org.freedesktop.DBus.Error.ServiceUnknown" => AppError::SystemdUnavailable,
            "org.freedesktop.DBus.Error.AccessDenied"
            | "org.freedesktop.DBus.Error.AuthFailed"
            | "org.freedesktop.DBus.Error.InteractiveAuthorizationRequired" => {
                if lower.contains("cancel")
                    || lower.contains("dismiss")
                    || lower.contains("not authorized")
                        && lower.contains("interactive")
                {
                    AppError::AuthorizationCancelled(unit.into())
                } else if name == "org.freedesktop.DBus.Error.InteractiveAuthorizationRequired" {
                    AppError::PermissionDenied(unit.into())
                } else {
                    AppError::PermissionDenied(unit.into())
                }
            }
            "org.freedesktop.DBus.Error.NoReply"
            | "org.freedesktop.DBus.Error.Timeout"
            | "org.freedesktop.DBus.Error.TimedOut" => AppError::OperationTimeout(unit.into()),
            "org.freedesktop.DBus.Error.Disconnected"
            | "org.freedesktop.DBus.Error.NoServer"
            | "org.freedesktop.DBus.Error.ConnectionClosed" => {
                AppError::DbusUnavailable(detail)
            }
            other => AppError::OperationFailed(unit.into(), format!("{other}: {detail}")),
        };
    }

    let text = err.to_string();
    let lower = text.to_lowercase();
    if lower.contains("timed out") || lower.contains("timeout") {
        AppError::OperationTimeout(unit.into())
    } else if lower.contains("connection")
        && (lower.contains("refused")
            || lower.contains("closed")
            || lower.contains("reset")
            || lower.contains("no such file"))
    {
        AppError::DbusUnavailable(text)
    } else if lower.contains("no such file or directory") {
        AppError::SystemdUnavailable
    } else {
        AppError::Internal(format!("D-Bus call failed: {text}"))
    }
}

fn classify_connection_error(err: &zbus::Error) -> AppError {
    let text = err.to_string();
    let lower = text.to_lowercase();
    if lower.contains("no such file") || lower.contains("not found") {
        AppError::SystemdUnavailable
    } else {
        AppError::DbusUnavailable(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::TryFrom;
    use zbus::zvariant::Value;

    fn props(pairs: &[(&str, &str)]) -> HashMap<String, OwnedValue> {
        pairs
            .iter()
            .map(|(key, value)| {
                let owned = OwnedValue::try_from(Value::from(*value)).unwrap();
                ((*key).to_string(), owned)
            })
            .collect()
    }

    #[test]
    fn builds_status_from_properties() {
        let status = build_status(
            ServiceId::MariaDb,
            &props(&[
                ("ActiveState", "active"),
                ("SubState", "running"),
                ("LoadState", "loaded"),
                ("UnitFileState", "enabled"),
                ("Description", "MariaDB 10.11 database server"),
            ]),
        );
        assert_eq!(status.state, ServiceState::Active);
        assert_eq!(status.unit, "mariadb.service");
        assert_eq!(status.sub_state, "running");
        assert!(status.installed);
        assert!(!status.masked);
        assert_eq!(status.description, "MariaDB 10.11 database server");
        assert!(status.unit_file_state == "enabled");
    }

    #[test]
    fn detects_masked_and_missing_units() {
        let masked = build_status(
            ServiceId::Httpd,
            &props(&[("ActiveState", "inactive"), ("LoadState", "masked")]),
        );
        assert!(masked.masked);
        // A masked unit still exists on disk, it just refuses to start.
        assert!(masked.installed);

        let missing = build_status(
            ServiceId::PhpFpm,
            &props(&[("ActiveState", ""), ("LoadState", "not-found")]),
        );
        assert!(!missing.installed);
        assert_eq!(missing.state, ServiceState::Unknown);

        let error = build_status(ServiceId::PhpFpm, &props(&[("LoadState", "error")]));
        assert!(!error.installed);
    }

    #[test]
    fn missing_properties_are_tolerated() {
        let status = build_status(ServiceId::Httpd, &HashMap::new());
        assert_eq!(status.state, ServiceState::Unknown);
        assert_eq!(status.description, "Apache HTTP Server");
        assert!(status.installed);
    }

    #[test]
    fn no_such_unit_maps_to_not_installed() {
        let err = zbus::Error::MethodError(
            zbus::names::OwnedErrorName::try_from("org.freedesktop.systemd1.NoSuchUnit").unwrap(),
            Some("Unit nope.service not found.".into()),
            zbus::Message::method_call("/", "Ping")
                .unwrap()
                .build(&())
                .unwrap(),
        );
        match classify_dbus_error("nope.service", &err) {
            AppError::ServiceNotInstalled(unit) => assert_eq!(unit, "nope.service"),
            other => panic!("unexpected mapping: {other:?}"),
        }
    }

    #[test]
    fn access_denied_maps_to_permission_denied() {
        let err = zbus::Error::MethodError(
            zbus::names::OwnedErrorName::try_from("org.freedesktop.DBus.Error.AccessDenied")
                .unwrap(),
            Some("Access denied".into()),
            zbus::Message::method_call("/", "Ping")
                .unwrap()
                .build(&())
                .unwrap(),
        );
        assert!(matches!(
            classify_dbus_error("httpd.service", &err),
            AppError::PermissionDenied(_)
        ));
    }

    #[test]
    fn dismissed_polkit_dialog_maps_to_cancelled() {
        let err = zbus::Error::MethodError(
            zbus::names::OwnedErrorName::try_from("org.freedesktop.DBus.Error.AccessDenied")
                .unwrap(),
            Some("Authentication cancelled by the user".into()),
            zbus::Message::method_call("/", "Ping")
                .unwrap()
                .build(&())
                .unwrap(),
        );
        assert!(matches!(
            classify_dbus_error("httpd.service", &err),
            AppError::AuthorizationCancelled(_)
        ));
    }

    #[test]
    fn operation_timeout_mapping() {
        let err = zbus::Error::MethodError(
            zbus::names::OwnedErrorName::try_from("org.freedesktop.DBus.Error.NoReply").unwrap(),
            Some("Did not receive a reply".into()),
            zbus::Message::method_call("/", "Ping")
                .unwrap()
                .build(&())
                .unwrap(),
        );
        assert!(matches!(
            classify_dbus_error("mariadb.service", &err),
            AppError::OperationTimeout(_)
        ));
    }
}
