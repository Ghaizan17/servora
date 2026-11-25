//! Live siteForge import.
//!
//! Adopts the domains listed in `SERVORA_IMPORT_DOMAINS` (comma separated)
//! through the same code path as the Import dialog: scan → import one website →
//! write the Servora configuration, hosts block and SELinux rules → reload.
//!
//! Ignored by default because it rewrites real Apache configuration.
//!
//! ```text
//! SERVORA_IMPORT_DOMAINS=e-perpus-skenda.com \
//!   cargo test --release --test siteforge_import_live -- --ignored --nocapture
//! ```

use servora_lib::services::sites::SiteService;

#[test]
#[ignore = "rewrites real Apache configuration and requires Polkit authentication"]
fn import_the_requested_domains() {
    let Ok(list) = std::env::var("SERVORA_IMPORT_DOMAINS") else {
        eprintln!("set SERVORA_IMPORT_DOMAINS=domain1,domain2 to import");
        return;
    };
    let domains: Vec<String> = list
        .split(',')
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    assert!(!domains.is_empty(), "no domains given");

    let service = SiteService::new();
    let scan = service.preview_migration().expect("the scan must succeed");
    for candidate in &scan.candidates {
        println!(
            "candidate {} owner={} importable={} project={}",
            candidate.domain, candidate.owner, candidate.importable, candidate.project_path
        );
    }

    let runtime = tokio::runtime::Runtime::new().unwrap();

    // A domain Servora already manages is refreshed instead of re-imported:
    // saving it again rewrites the configuration, the hosts block (removing any
    // leftover duplicate lines) and the SELinux rules.
    let existing = service.views().unwrap();
    let mut already_managed: Vec<String> = Vec::new();
    let mut fresh: Vec<String> = Vec::new();
    for domain in &domains {
        match existing.iter().find(|view| &view.website.domain == domain) {
            Some(view) => {
                already_managed.push(view.website.id.clone());
                let website = view.website.clone();
                let input = servora_lib::servora_core::website::WebsiteInput {
                    name: website.name.clone(),
                    domain: website.domain.clone(),
                    project_path: website.project_path.clone(),
                    document_root: Some(website.document_root.clone()),
                    project_type: Some(
                        servora_lib::servora_core::project::ProjectType::as_str(
                            &website.project_type,
                        )
                        .to_string(),
                    ),
                };
                let refreshed = runtime
                    .block_on(service.update(&website.id, &input))
                    .expect("the refresh must run");
                println!(
                    "refreshed {} -> success={} warnings={:?}",
                    website.name, refreshed.success, refreshed.warnings
                );
                assert!(refreshed.success, "{:?}", refreshed.message);
            }
            None => fresh.push(domain.clone()),
        }
    }

    let report = if fresh.is_empty() {
        servora_lib::services::sites::ImportReport {
            imported: Vec::new(),
            skipped: Vec::new(),
            failed: Vec::new(),
        }
    } else {
        runtime
            .block_on(service.import(&fresh))
            .expect("the import must run")
    };

    for operation in &report.imported {
        println!(
            "imported {} -> success={} warnings={:?}",
            operation.website_name.as_deref().unwrap_or("?"),
            operation.success,
            operation.warnings
        );
        if let Some(outcome) = &operation.outcome {
            for step in &outcome.steps {
                println!("  step {}: {:?}", step.id, step.status);
            }
        }
    }
    for skipped in &report.skipped {
        println!("skipped: {skipped}");
    }
    for failure in &report.failed {
        println!(
            "failed: {} — {:?}",
            failure.website_name.as_deref().unwrap_or("?"),
            failure.message
        );
    }

    assert!(
        report.failed.is_empty(),
        "{} import(s) failed",
        report.failed.len()
    );
    assert_eq!(
        report.imported.len() + already_managed.len(),
        domains.len(),
        "every requested domain must be adopted or refreshed"
    );

    // Every imported website must now be listed as live.
    let views = service.views().unwrap();
    for domain in &domains {
        let view = views
            .iter()
            .find(|view| &view.website.domain == domain)
            .unwrap_or_else(|| panic!("{domain} must be listed after the import"));
        println!(
            "{domain}: state={} warnings={:?}",
            view.state, view.status.warnings
        );
        assert_eq!(view.state, "live", "{domain}: {:?}", view.status.warnings);
    }

    let configtest = service.validate_apache().unwrap();
    assert_eq!(configtest["valid"], true, "{configtest}");
}
