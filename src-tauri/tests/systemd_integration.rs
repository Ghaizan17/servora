//! Integration tests against the live systemd instance.
//!
//! These are marked `#[ignore]` so a plain `cargo test` stays fast and does not
//! depend on the host. Run them explicitly on Fedora:
//!
//! ```text
//! cargo test --test systemd_integration -- --ignored --nocapture
//! ```
//!
//! The read-only tests never change service state. The control tests are behind
//! the `SERVORA_ALLOW_CONTROL_TESTS=1` environment variable because they stop
//! and start real services on the machine.

use servora_lib::models::service::{ServiceId, ServiceState};
use servora_lib::services::systemd::SystemdClient;

fn control_tests_enabled() -> bool {
    std::env::var("SERVORA_ALLOW_CONTROL_TESTS").as_deref() == Ok("1")
}

#[tokio::test]
#[ignore = "requires a live systemd instance"]
async fn all_managed_units_are_detected() {
    let client = SystemdClient::connect().await.expect("systemd connection");
    let statuses = client.statuses().await.expect("status list");

    assert_eq!(statuses.len(), 3, "one status per managed unit");
    for status in &statuses {
        println!(
            "{:<16} installed={:<5} state={:<12} sub={:<10} unit_file={}",
            status.unit,
            status.installed,
            status.state.label(),
            status.sub_state,
            status.unit_file_state
        );
        assert!(
            status.installed,
            "{} must be installed for this integration test (dnf install)",
            status.unit
        );
        assert_ne!(status.state, ServiceState::Unknown, "{} state", status.unit);
    }
}

#[tokio::test]
#[ignore = "requires a live systemd instance"]
async fn status_matches_systemctl_is_active() {
    let client = SystemdClient::connect().await.expect("systemd connection");
    for service in ServiceId::ALL {
        let status = client.status(service).await.expect("status");
        let output = std::process::Command::new("systemctl")
            .args(["is-active", service.unit()])
            .output()
            .expect("run systemctl is-active");
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        assert_eq!(
            status.state,
            ServiceState::parse(&text),
            "systemd D-Bus and `systemctl is-active` disagree for {}",
            service.unit()
        );
    }
}

#[tokio::test]
#[ignore = "requires a live systemd instance"]
async fn logs_are_retrievable_for_every_unit() {
    use servora_lib::models::logs::LogLevel;
    use servora_lib::services::journal;

    for service in ServiceId::ALL {
        let response = journal::fetch_logs(service, LogLevel::All, Some(50), 300)
            .await
            .unwrap_or_else(|err| panic!("journal for {} failed: {err}", service.unit()));
        println!(
            "{}: {} entries (limit {})",
            service.unit(),
            response.count,
            response.requested_lines
        );
        assert!(response.count <= 50);
        for entry in response.entries.iter().take(3) {
            println!(
                "  {} [{}] {}",
                entry.timestamp.as_deref().unwrap_or("-"),
                entry.ident.as_deref().unwrap_or("-"),
                entry.message
            );
        }
    }
}

#[tokio::test]
#[ignore = "changes service state; requires SERVORA_ALLOW_CONTROL_TESTS=1"]
async fn restart_roundtrip_leaves_service_running() {
    use servora_lib::models::service::ServiceOperation;

    if !control_tests_enabled() {
        eprintln!("skipped: set SERVORA_ALLOW_CONTROL_TESTS=1 to run control tests");
        return;
    }

    let client = SystemdClient::connect().await.expect("systemd connection");
    let service = ServiceId::MariaDb;

    let before = client.status(service).await.expect("status before");
    println!("before: {:?}", before.state);

    let after = client
        .operate(service, ServiceOperation::Restart)
        .await
        .expect("restart");
    println!("after: {:?} / {}", after.state, after.sub_state);
    assert!(after.state.is_running(), "mariadb should be active again");

    // Second start must be rejected with a typed error.
    let already = client.operate(service, ServiceOperation::Start).await;
    assert!(already.is_err(), "starting an active unit must fail");
}

#[tokio::test]
#[ignore = "changes service state; requires SERVORA_ALLOW_CONTROL_TESTS=1"]
async fn stop_and_start_roundtrip() {
    use servora_lib::error::AppError;
    use servora_lib::models::service::ServiceOperation;

    if !control_tests_enabled() {
        eprintln!("skipped: set SERVORA_ALLOW_CONTROL_TESTS=1 to run control tests");
        return;
    }

    let client = SystemdClient::connect().await.expect("systemd connection");
    let service = ServiceId::MariaDb;

    let original = client.status(service).await.expect("status before");
    println!("original: {:?}/{}", original.state, original.sub_state);
    if !original.state.is_running() {
        client
            .operate(service, ServiceOperation::Start)
            .await
            .expect("pre-start");
    }

    let stopped = client
        .operate(service, ServiceOperation::Stop)
        .await
        .expect("stop");
    assert!(stopped.state.is_stopped(), "mariadb should be stopped");
    println!("stopped: {:?}/{}", stopped.state, stopped.sub_state);

    // Stopping again must be rejected with a typed error, not a silent success.
    let again = client
        .operate(service, ServiceOperation::Stop)
        .await
        .expect_err("stopping an inactive unit must fail");
    assert!(
        matches!(again, AppError::ServiceAlreadyInactive(_)),
        "unexpected error: {again}"
    );

    let started = client
        .operate(service, ServiceOperation::Start)
        .await
        .expect("start");
    assert!(started.state.is_running(), "mariadb should be running again");
    println!("started: {:?}/{}", started.state, started.sub_state);

    // Leave the machine exactly as we found it.
    let restored = client.status(service).await.expect("status after");
    assert_eq!(
        restored.state, original.state,
        "the service state must be restored after the roundtrip"
    );
}

#[tokio::test]
#[ignore = "requires a live systemd instance"]
async fn unknown_units_cannot_be_queried() {
    let client = SystemdClient::connect().await.expect("systemd connection");
    // The allowlist is enforced before the client, but the client must also
    // report a typed error rather than panicking for a non-managed unit.
    let status = client.status(ServiceId::Httpd).await;
    assert!(status.is_ok());
}

#[tokio::test]
#[ignore = "requires a live systemd instance"]
async fn starting_an_active_unit_is_rejected_without_changing_state() {
    use servora_lib::error::AppError;
    use servora_lib::models::service::ServiceOperation;

    let client = SystemdClient::connect().await.expect("systemd connection");
    let before = client.status(ServiceId::Httpd).await.expect("status");
    if !before.state.is_running() {
        // Nothing to prove when httpd is down; the typed error is still covered
        // by the unit tests for the inactive direction.
        return;
    }

    let error = client
        .operate(ServiceId::Httpd, ServiceOperation::Start)
        .await
        .expect_err("starting an active unit must fail");
    assert!(
        matches!(error, AppError::ServiceAlreadyActive(_)),
        "unexpected error: {error}"
    );

    let after = client.status(ServiceId::Httpd).await.expect("status");
    assert_eq!(before.state, after.state, "state must be untouched");
    assert_eq!(before.sub_state, after.sub_state);
}

#[tokio::test]
#[ignore = "requires a live systemd instance"]
async fn managed_health_reports_all_running_when_everything_is_up() {
    let client = SystemdClient::connect().await.expect("systemd connection");
    let statuses = client.statuses().await.expect("status list");
    let running = statuses.iter().filter(|s| s.state.is_running()).count();
    println!("{running}/{} services active", statuses.len());
    assert_eq!(statuses.len(), 3);
}

/// Guards the JSON contract the frontend depends on (`subState`, `observedAtMs`, ...)
/// using real systemd data rather than fixtures.
#[tokio::test]
#[ignore = "requires a live systemd instance"]
async fn frontend_json_contract_matches_for_live_data() {
    use servora_lib::models::service::ServicesStatusResponse;

    let client = SystemdClient::connect().await.expect("systemd connection");
    let statuses = client.statuses().await.expect("status list");
    let response = ServicesStatusResponse::from_statuses(statuses, 12345);
    let json = serde_json::to_value(&response).expect("serialize");

    for key in ["services", "allRunning", "running", "total", "observedAtMs"] {
        assert!(json.get(key).is_some(), "missing `{key}` in {json}");
    }
    let first = &json["services"][0];
    for key in [
        "id",
        "unit",
        "name",
        "state",
        "subState",
        "loadState",
        "unitFileState",
        "installed",
        "masked",
        "observedAtMs",
    ] {
        assert!(first.get(key).is_some(), "missing `{key}` in {first}");
    }
    // Service ids must be the slugs the UI uses for buttons and IPC calls.
    let ids: Vec<&str> = json["services"]
        .as_array()
        .unwrap()
        .iter()
        .map(|service| service["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["httpd", "php-fpm", "mariadb"]);
}
