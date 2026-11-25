//! Live website roundtrip against the real machine.
//!
//! Creates a throwaway website (`servora-live-probe.test`) through the same code
//! path the GUI uses — `pkexec` → `servora-helper` → Polkit
//! (`dev.servora.app.manage-sites`) — verifies what landed on disk, then removes
//! it again and verifies the cleanup.
//!
//! Ignored by default because it writes to `/etc` and asks Polkit to
//! authenticate. Run it explicitly:
//!
//! ```text
//! SERVORA_ALLOW_SITE_TESTS=1 cargo test --release --test website_live -- --ignored --nocapture --test-threads=1
//! ```

use std::path::Path;

use servora_lib::services::sites::SiteService;
use servora_lib::servora_core::hosts::HostsFile;
use servora_lib::servora_core::website::WebsiteInput;

const DOMAIN: &str = "servora-live-probe.test";
const NAME: &str = "servora-live-probe";

fn fixture() -> std::path::PathBuf {
    let project = std::env::temp_dir().join(NAME);
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(project.join("public")).unwrap();
    std::fs::create_dir_all(project.join("storage/app")).unwrap();
    std::fs::create_dir_all(project.join("bootstrap/cache")).unwrap();
    std::fs::write(project.join("artisan"), "#!/usr/bin/env php\n").unwrap();
    std::fs::write(project.join("composer.json"), "{}\n").unwrap();
    std::fs::write(project.join("public/index.php"), "<?php echo 'probe';\n").unwrap();
    project
}

#[test]
#[ignore = "writes to /etc and requires Polkit authentication"]
fn create_then_remove_a_website_on_the_live_system() {
    if std::env::var("SERVORA_ALLOW_SITE_TESTS").as_deref() != Ok("1") {
        eprintln!("set SERVORA_ALLOW_SITE_TESTS=1 to run this test");
        return;
    }

    let service = SiteService::new();
    let paths = service.paths();
    assert!(
        paths.helper_path.exists(),
        "the helper must be installed at {}",
        paths.helper_path.display()
    );

    let project = fixture();
    let config = paths.apache_conf_dir.join(format!("{DOMAIN}.conf"));
    let runtime = tokio::runtime::Runtime::new().unwrap();

    // Never leave a previous run behind.
    if config.exists() {
        eprintln!("removing a leftover {DOMAIN} from an earlier run");
        let _ = runtime.block_on(service.remove(NAME));
    }

    // ---------------------------------------------------------------- create
    let input = WebsiteInput {
        name: NAME.into(),
        domain: DOMAIN.into(),
        project_path: project.display().to_string(),
        document_root: None,
        project_type: None,
    };

    let report = runtime
        .block_on(service.create(&input))
        .expect("create must not fail");
    println!(
        "create: success={} steps={} warnings={:?}",
        report.success,
        report.outcome.as_ref().map(|o| o.steps.len()).unwrap_or(0),
        report.warnings
    );
    assert!(report.success, "create failed: {:?}", report.message);
    for step in report.outcome.as_ref().unwrap().steps.iter() {
        println!("  step {}: {:?}", step.id, step.status);
    }

    // The VirtualHost must exist, carry the Servora marker and serve the domain.
    let config_text = std::fs::read_to_string(&config).expect("the VirtualHost must be written");
    assert!(
        config_text.contains("# Managed by Servora"),
        "{config_text}"
    );
    assert!(
        config_text.contains(&format!("ServerName {DOMAIN}")),
        "{config_text}"
    );
    assert!(
        config_text.contains(&project.join("public").display().to_string()),
        "the document root must be the detected public/ directory"
    );

    // The hosts entry must live inside the managed block.
    let hosts_text = std::fs::read_to_string(&paths.hosts_file).unwrap();
    let hosts = HostsFile::parse(&hosts_text);
    assert_eq!(
        hosts.state(DOMAIN),
        servora_lib::servora_core::hosts::HostsState::Managed,
        "expected a managed hosts entry"
    );

    // Apache must be able to read the project.
    let acl = std::process::Command::new("getfacl")
        .args(["-p", project.join("storage").to_str().unwrap()])
        .output()
        .expect("getfacl");
    let acl_text = String::from_utf8_lossy(&acl.stdout).to_string();
    println!("storage ACL:\n{acl_text}");
    assert!(
        acl_text.contains("apache"),
        "the project must be readable by Apache"
    );

    // And the list must now report it as live.
    let views = service.views().unwrap();
    let view = views
        .iter()
        .find(|view| view.website.name == NAME)
        .expect("the website must be listed");
    println!("state={} warnings={:?}", view.state, view.status.warnings);
    assert_eq!(view.state, "live", "{:?}", view.status.warnings);

    // The SELinux rule must be in effect for the project.
    let selinux = |path: &str| -> String {
        std::process::Command::new("matchpathcon")
            .args(["-n", path])
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
            .unwrap_or_default()
    };
    let context = selinux(project.to_str().unwrap());
    println!("SELinux context: {}", context.trim());
    if service.selinux_mode().requires_contexts() {
        assert!(
            context.contains("httpd_sys_content_t"),
            "expected httpd_sys_content_t, got {context}"
        );
    }

    // Apache must still accept its configuration.
    let configtest = service.validate_apache().unwrap();
    assert_eq!(configtest["valid"], true, "{configtest}");

    // ---------------------------------------------------------------- remove
    let removal = runtime
        .block_on(service.remove(NAME))
        .expect("remove must not fail");
    println!(
        "remove: success={} warnings={:?}",
        removal.success, removal.warnings
    );
    assert!(removal.success, "remove failed: {:?}", removal.message);

    assert!(!config.exists(), "the VirtualHost must be gone");
    let hosts_after = HostsFile::parse(&std::fs::read_to_string(&paths.hosts_file).unwrap());
    assert_eq!(
        hosts_after.state(DOMAIN),
        servora_lib::servora_core::hosts::HostsState::Absent,
        "the hosts entry must be gone"
    );
    assert!(
        Path::new(&project).is_dir(),
        "the project directory must never be deleted"
    );
    assert!(
        !service
            .views()
            .unwrap()
            .iter()
            .any(|view| view.website.name == NAME),
        "the website must no longer be listed"
    );

    // And Servora's SELinux rules must be gone again.
    if service.selinux_mode().requires_contexts() {
        let after = selinux(&project.display().to_string());
        println!("SELinux context after removal: {}", after.trim());
        assert!(
            !after.contains("httpd_sys_content_t"),
            "the SELinux rule must be removed again, got {after}"
        );
    }

    let _ = std::fs::remove_dir_all(&project);
}
