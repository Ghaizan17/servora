//! Strongly typed models for the three services Servora manages.
//!
//! The service allowlist lives here: no other unit name can ever reach systemd
//! because every entry point converts the incoming string through [`ServiceId`].

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// The only three systemd units Servora is allowed to touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ServiceId {
    #[serde(rename = "httpd")]
    Httpd,
    #[serde(rename = "php-fpm")]
    PhpFpm,
    #[serde(rename = "mariadb")]
    MariaDb,
}

impl ServiceId {
    /// Every managed service, in display order (web server first).
    pub const ALL: [ServiceId; 3] = [ServiceId::Httpd, ServiceId::PhpFpm, ServiceId::MariaDb];

    /// Start All order: database, then the application runtime, then the web server.
    pub const START_ALL_ORDER: [ServiceId; 3] =
        [ServiceId::MariaDb, ServiceId::PhpFpm, ServiceId::Httpd];

    /// Stop All order: the reverse of the start order.
    pub const STOP_ALL_ORDER: [ServiceId; 3] =
        [ServiceId::Httpd, ServiceId::PhpFpm, ServiceId::MariaDb];

    /// Restart All order matches Start All (dependencies come up first).
    pub const RESTART_ALL_ORDER: [ServiceId; 3] =
        [ServiceId::MariaDb, ServiceId::PhpFpm, ServiceId::Httpd];

    /// The systemd unit name, the only string ever handed to systemd.
    pub fn unit(self) -> &'static str {
        match self {
            ServiceId::Httpd => "httpd.service",
            ServiceId::PhpFpm => "php-fpm.service",
            ServiceId::MariaDb => "mariadb.service",
        }
    }

    /// Human readable name used in the UI and notifications.
    pub fn display_name(self) -> &'static str {
        match self {
            ServiceId::Httpd => "Apache HTTP Server",
            ServiceId::PhpFpm => "PHP-FPM",
            ServiceId::MariaDb => "MariaDB",
        }
    }

    /// Stable slug used by the UI as element id / config key.
    pub fn slug(self) -> &'static str {
        match self {
            ServiceId::Httpd => "httpd",
            ServiceId::PhpFpm => "php-fpm",
            ServiceId::MariaDb => "mariadb",
        }
    }

    /// Short label rendered on badges and tray items.
    pub fn short_name(self) -> &'static str {
        match self {
            ServiceId::Httpd => "HTTPD",
            ServiceId::PhpFpm => "PHP-FPM",
            ServiceId::MariaDb => "MariaDB",
        }
    }

    /// Resolve a systemd unit name back to a managed service.
    pub fn from_unit(unit: &str) -> Option<Self> {
        ServiceId::ALL
            .iter()
            .copied()
            .find(|service| service.unit() == unit)
    }
}

impl fmt::Display for ServiceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.slug())
    }
}

impl FromStr for ServiceId {
    type Err = String;

    /// Parses a slug (`httpd`, `php-fpm`, `mariadb`) or a unit name
    /// (`httpd.service`, ...). Anything else is rejected.
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let trimmed = input.trim();
        if let Some(service) = ServiceId::from_unit(trimmed) {
            return Ok(service);
        }
        ServiceId::ALL
            .iter()
            .copied()
            .find(|service| service.slug().eq_ignore_ascii_case(trimmed))
            .ok_or_else(|| format!("`{input}` is not a service managed by Servora"))
    }
}

/// systemd `ActiveState` values Servora understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceState {
    Active,
    Inactive,
    Activating,
    Deactivating,
    Failed,
    Unknown,
}

impl ServiceState {
    /// Parse a raw systemd `ActiveState` string.
    pub fn parse(raw: &str) -> Self {
        match raw.trim() {
            "active" | "reloading" => ServiceState::Active,
            "inactive" => ServiceState::Inactive,
            "activating" => ServiceState::Activating,
            "deactivating" => ServiceState::Deactivating,
            "failed" => ServiceState::Failed,
            _ => ServiceState::Unknown,
        }
    }

    /// True when the service is fully up.
    pub fn is_running(self) -> bool {
        matches!(self, ServiceState::Active)
    }

    /// True when the service is down (either cleanly or after a failure).
    pub fn is_stopped(self) -> bool {
        matches!(self, ServiceState::Inactive | ServiceState::Failed)
    }

    /// True while a transition is in flight.
    pub fn is_transitioning(self) -> bool {
        matches!(self, ServiceState::Activating | ServiceState::Deactivating)
    }

    pub fn label(self) -> &'static str {
        match self {
            ServiceState::Active => "Active",
            ServiceState::Inactive => "Inactive",
            ServiceState::Activating => "Activating",
            ServiceState::Deactivating => "Deactivating",
            ServiceState::Failed => "Failed",
            ServiceState::Unknown => "Unknown",
        }
    }
}

impl fmt::Display for ServiceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Snapshot of one managed unit, taken straight from systemd.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceStatus {
    pub id: ServiceId,
    pub unit: String,
    pub name: String,
    pub state: ServiceState,
    pub sub_state: String,
    pub load_state: String,
    /// `enabled` / `disabled` / `static` / `masked` / `""` when unknown.
    pub unit_file_state: String,
    pub description: String,
    /// True when systemd reports the unit file exists.
    pub installed: bool,
    /// True when the unit file is masked and therefore cannot be started.
    pub masked: bool,
    /// Observation time, epoch milliseconds.
    pub observed_at_ms: i64,
}

impl ServiceStatus {
    /// Fallback snapshot used when systemd cannot be queried at all.
    pub fn unknown(service: ServiceId, observed_at_ms: i64) -> Self {
        Self {
            id: service,
            unit: service.unit().to_string(),
            name: service.display_name().to_string(),
            state: ServiceState::Unknown,
            sub_state: String::new(),
            load_state: "unknown".to_string(),
            unit_file_state: String::new(),
            description: service.display_name().to_string(),
            installed: false,
            masked: false,
            observed_at_ms,
        }
    }
}

/// The three privileged actions Servora can perform on a unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceOperation {
    Start,
    Stop,
    Restart,
}

impl ServiceOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            ServiceOperation::Start => "start",
            ServiceOperation::Stop => "stop",
            ServiceOperation::Restart => "restart",
        }
    }

    pub fn past_tense(self) -> &'static str {
        match self {
            ServiceOperation::Start => "started",
            ServiceOperation::Stop => "stopped",
            ServiceOperation::Restart => "restarted",
        }
    }
}

impl fmt::Display for ServiceOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ServiceOperation {
    type Err = String;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        match input.trim().to_ascii_lowercase().as_str() {
            "start" => Ok(ServiceOperation::Start),
            "stop" => Ok(ServiceOperation::Stop),
            "restart" => Ok(ServiceOperation::Restart),
            other => Err(format!("unsupported operation `{other}`")),
        }
    }
}

/// Result of a single service operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationResult {
    pub service: ServiceId,
    pub unit: String,
    pub name: String,
    pub operation: ServiceOperation,
    pub success: bool,
    pub status: ServiceStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<crate::error::ErrorPayload>,
    pub message: String,
    pub duration_ms: u64,
}

/// Result of a batch (Start All / Stop All / Restart All) operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchOperationResult {
    pub operation: ServiceOperation,
    pub results: Vec<OperationResult>,
    pub succeeded: usize,
    pub failed: usize,
    pub all_succeeded: bool,
    pub started_at_ms: i64,
    pub finished_at_ms: i64,
}

impl BatchOperationResult {
    pub fn summary(&self) -> String {
        format!(
            "{} of {} services {}",
            self.succeeded,
            self.results.len(),
            self.operation.past_tense()
        )
    }
}

/// Response wrapper for a full dashboard refresh.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServicesStatusResponse {
    pub services: Vec<ServiceStatus>,
    /// True when all installed services are active.
    pub all_running: bool,
    /// Number of managed units that are currently active.
    pub running: usize,
    pub total: usize,
    pub systemd_available: bool,
    pub observed_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<crate::error::ErrorPayload>,
}

impl ServicesStatusResponse {
    pub fn from_statuses(services: Vec<ServiceStatus>, observed_at_ms: i64) -> Self {
        let total = services.len();
        let running = services.iter().filter(|s| s.state.is_running()).count();
        Self {
            all_running: running == total && total > 0,
            running,
            total,
            services,
            systemd_available: true,
            observed_at_ms,
            error: None,
        }
    }

    pub fn degraded(error: crate::error::ErrorPayload, observed_at_ms: i64) -> Self {
        let services = ServiceId::ALL
            .iter()
            .copied()
            .map(|service| ServiceStatus::unknown(service, observed_at_ms))
            .collect();
        Self {
            services,
            all_running: false,
            running: 0,
            total: ServiceId::ALL.len(),
            systemd_available: false,
            observed_at_ms,
            error: Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_mapping_is_stable() {
        assert_eq!(ServiceId::Httpd.unit(), "httpd.service");
        assert_eq!(ServiceId::PhpFpm.unit(), "php-fpm.service");
        assert_eq!(ServiceId::MariaDb.unit(), "mariadb.service");
    }

    #[test]
    fn slug_and_unit_roundtrip() {
        for service in ServiceId::ALL {
            assert_eq!(ServiceId::from_str(service.slug()).unwrap(), service);
            assert_eq!(ServiceId::from_str(service.unit()).unwrap(), service);
            assert_eq!(ServiceId::from_unit(service.unit()).unwrap(), service);
        }
    }

    #[test]
    fn arbitrary_units_are_rejected() {
        for bad in [
            "sshd.service",
            "httpd",
            "http.service",
            "mariadb.service ",
            "",
            "../../etc/passwd",
            "httpd.service; rm -rf /",
            "systemd-journald.service",
        ] {
            let parsed = ServiceId::from_str(bad);
            if bad == "mariadb.service " {
                // Whitespace is trimmed, so this one is legitimate.
                assert!(parsed.is_ok());
            } else if bad == "httpd" {
                // A bare slug is legitimate.
                assert!(parsed.is_ok());
            } else {
                assert!(parsed.is_err(), "`{bad}` must be rejected");
            }
        }
    }

    #[test]
    fn serde_uses_frontend_slugs() {
        assert_eq!(
            serde_json::to_string(&ServiceId::PhpFpm).unwrap(),
            "\"php-fpm\""
        );
        assert_eq!(
            serde_json::to_string(&ServiceId::MariaDb).unwrap(),
            "\"mariadb\""
        );
        let parsed: ServiceId = serde_json::from_str("\"httpd\"").unwrap();
        assert_eq!(parsed, ServiceId::Httpd);
        assert!(serde_json::from_str::<ServiceId>("\"sshd\"").is_err());
    }

    #[test]
    fn active_state_parsing() {
        assert_eq!(ServiceState::parse("active"), ServiceState::Active);
        assert_eq!(ServiceState::parse("reloading"), ServiceState::Active);
        assert_eq!(ServiceState::parse("inactive"), ServiceState::Inactive);
        assert_eq!(ServiceState::parse("activating"), ServiceState::Activating);
        assert_eq!(
            ServiceState::parse("deactivating"),
            ServiceState::Deactivating
        );
        assert_eq!(ServiceState::parse("failed"), ServiceState::Failed);
        assert_eq!(ServiceState::parse("maintenance"), ServiceState::Unknown);
        assert_eq!(ServiceState::parse(""), ServiceState::Unknown);
    }

    #[test]
    fn batch_order_is_dependency_safe() {
        assert_eq!(
            ServiceId::START_ALL_ORDER,
            [ServiceId::MariaDb, ServiceId::PhpFpm, ServiceId::Httpd]
        );
        assert_eq!(
            ServiceId::STOP_ALL_ORDER,
            [ServiceId::Httpd, ServiceId::PhpFpm, ServiceId::MariaDb]
        );
        let mut reversed = ServiceId::STOP_ALL_ORDER;
        reversed.reverse();
        assert_eq!(reversed, ServiceId::START_ALL_ORDER);
        assert_eq!(
            ServiceId::RESTART_ALL_ORDER,
            ServiceId::START_ALL_ORDER.to_vec()[..]
        );
    }

    #[test]
    fn frontend_contract_uses_camel_case() {
        let status = ServiceStatus::unknown(ServiceId::Httpd, 42);
        let json = serde_json::to_value(&status).unwrap();
        for key in [
            "subState",
            "loadState",
            "unitFileState",
            "observedAtMs",
            "installed",
            "masked",
        ] {
            assert!(json.get(key).is_some(), "missing key {key} in {json}");
        }
        assert!(json.get("sub_state").is_none());

        let response = ServicesStatusResponse::from_statuses(vec![status], 42);
        let json = serde_json::to_value(&response).unwrap();
        assert!(json.get("allRunning").is_some());
        assert!(json.get("observedAtMs").is_some());
    }

    #[test]
    fn status_response_aggregation() {
        let now = 1_700_000_000_000;
        let mut services: Vec<ServiceStatus> = ServiceId::ALL
            .iter()
            .copied()
            .map(|id| ServiceStatus::unknown(id, now))
            .collect();
        services[0].state = ServiceState::Active;
        let response = ServicesStatusResponse::from_statuses(services.clone(), now);
        assert_eq!(response.running, 1);
        assert_eq!(response.total, 3);
        assert!(!response.all_running);

        for service in services.iter_mut() {
            service.state = ServiceState::Active;
        }
        let response = ServicesStatusResponse::from_statuses(services, now);
        assert!(response.all_running);
        assert_eq!(response.running, 3);
    }
}
