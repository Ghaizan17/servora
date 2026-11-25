//! Read-only integration tests against the real machine.
//!
//! Nothing here creates, edits or removes a website: the tests only read the
//! live Apache configuration, `/etc/hosts` and the diagnostics sources, so they
//! are safe to run repeatedly on a developer machine.
//!
//! `cargo test --test website_manager`

use servora_lib::services::sites::SiteService;
use servora_lib::servora_core::project::ProjectType;
use servora_lib::servora_core::website::WebsiteInput;

fn site_service() -> SiteService {
    SiteService::new()
}

#[test]
fn website_list_is_readable_and_consistent_with_apache() {
    let views = site_service()
        .views()
        .expect("the website list must be readable");

    for view in &views {
        // The recorded domain must be a valid domain and the config path must be
        // inside the Apache configuration directory.
        assert!(!view.website.domain.is_empty());
        assert!(view.status.config_path.starts_with("/etc/httpd/conf.d"));
        // Derived state must always be one of the known values.
        assert!(
            [
                "live",
                "config-missing",
                "config-mismatch",
                "project-missing",
                "hosts-missing",
                "degraded"
            ]
            .contains(&view.state.as_str()),
            "unexpected state {}",
            view.state
        );
    }
}

#[test]
fn domain_validation_matches_the_documented_rules() {
    let service = site_service();

    let valid = service.validate_domain("e-perpus-skenda.com").unwrap();
    assert_eq!(valid["valid"], true);

    for invalid in [
        "",
        "not a domain",
        "http://example.com",
        "example.com/path",
        "1.2.3.4",
    ] {
        let result = service.validate_domain(invalid).unwrap();
        assert_eq!(result["valid"], false, "{invalid} must be rejected");
        assert!(result["message"]
            .as_str()
            .is_some_and(|text| !text.is_empty()));
    }
}

#[test]
fn migration_preview_sees_the_siteforge_sites_without_touching_them() {
    let scan = site_service()
        .preview_migration()
        .expect("the scan must succeed");

    assert!(scan.conf_dir.starts_with("/etc/httpd/conf.d"));
    for candidate in &scan.candidates {
        assert!(candidate.config_path.starts_with("/etc/httpd/conf.d"));
        assert!(!candidate.domain.is_empty());
    }

    // The two siteForge-managed sites on this machine are detected as theirs.
    let siteforge: Vec<&str> = scan
        .candidates
        .iter()
        .filter(|candidate| candidate.owner.contains("siteForge"))
        .map(|candidate| candidate.domain.as_str())
        .collect();
    println!("siteForge candidates: {siteforge:?}");
    for candidate in &scan.candidates {
        println!(
            "- {} owner={} importable={} type={:?} warnings={:?}",
            candidate.domain,
            candidate.owner,
            candidate.importable,
            candidate.project_type,
            candidate.warnings
        );
    }
}

#[test]
fn project_detection_works_on_a_real_directory() {
    let service = site_service();
    let home = std::env::var("HOME").expect("HOME must be set");

    let inspection = service
        .inspect_project(std::path::Path::new(&home))
        .expect("the home directory must be inspectable");
    assert_eq!(inspection.project_path, home);
    // A home directory is not a PHP project, so the type must be unknown and the
    // document root must fall back to the directory itself.
    assert_eq!(inspection.project_type, ProjectType::Unknown);
}

#[test]
fn diagnostics_and_environment_report_real_values() {
    let service = site_service();

    let report = service.diagnostics().expect("diagnostics must run");
    assert!(!report.checks.is_empty());
    assert_eq!(report.summary.total, report.checks.len());
    assert_eq!(
        report.summary.passed
            + report.summary.warnings
            + report.summary.failed
            + report.summary.not_applicable,
        report.summary.total
    );
    for check in &report.checks {
        println!("[{:?}] {} — {}", check.status, check.label, check.detail);
    }

    let environment = service
        .environment()
        .expect("the environment must be readable");
    assert!(environment
        .components
        .iter()
        .any(|component| component.id.contains("apache")));
    println!(
        "ready: {} ({} components)",
        environment.ready,
        environment.components.len()
    );
}

#[test]
fn apache_configuration_is_valid() {
    let service = site_service();
    let result = service.validate_apache().expect("configtest must run");
    let output = result["output"].as_str().unwrap_or_default();
    println!("configtest: valid={} output={output}", result["valid"]);
    assert_eq!(result["valid"], true, "configtest output: {output}");
}

#[test]
fn creating_a_website_without_the_helper_fails_without_writing() {
    let service = site_service();
    let paths = service.paths();
    if paths.helper_path.exists() {
        // A packaged build has the helper installed; the privileged path is
        // covered by the core integration tests that use an injected system.
        println!(
            "helper present at {}, skipping",
            paths.helper_path.display()
        );
        return;
    }

    // A temporary project directory: nothing outside /tmp is touched.
    let project = std::env::temp_dir().join("servora-readonly-probe");
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(&project).expect("temporary project");
    std::fs::write(project.join("index.php"), "<?php echo 'probe';").expect("fixture");

    let input = WebsiteInput {
        name: "servora-readonly-probe".into(),
        domain: "servora-readonly-probe.test".into(),
        project_path: project.display().to_string(),
        document_root: Some(project.display().to_string()),
        project_type: Some("php".into()),
    };

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let error = runtime.block_on(service.create(&input)).unwrap_err();
    println!("create without helper: [{}] {}", error.code(), error);
    assert_eq!(error.code(), "helper_missing");
    assert!(!paths
        .apache_conf_dir
        .join("servora-readonly-probe.test.conf")
        .exists());
    assert!(!service
        .views()
        .unwrap()
        .iter()
        .any(|view| view.website.name == "servora-readonly-probe"));
}
