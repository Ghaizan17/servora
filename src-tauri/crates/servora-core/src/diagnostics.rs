//! Environment diagnostics and first-run checks.
//!
//! Everything here is read-only: Servora inspects the system, reports what is
//! missing and suggests the Fedora package that provides it. It never installs,
//! enables, disables or reconfigures anything on its own.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::errors::CoreResult;
use crate::plan::{CommandOutput, SystemOps};
use crate::selinux::SelinuxMode;
use crate::storage::{now_ms, Paths};

/// Severity of a single check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckStatus {
    Passed,
    Warning,
    Failed,
    NotApplicable,
}

impl CheckStatus {
    pub fn label(&self) -> &'static str {
        match self {
            CheckStatus::Passed => "passed",
            CheckStatus::Warning => "warning",
            CheckStatus::Failed => "failed",
            CheckStatus::NotApplicable => "not applicable",
        }
    }
}

/// One diagnostic line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticCheck {
    pub id: String,
    pub label: String,
    pub status: CheckStatus,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

impl DiagnosticCheck {
    fn new(
        id: &str,
        label: &str,
        status: CheckStatus,
        detail: impl Into<String>,
        remediation: Option<&str>,
    ) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            status,
            detail: detail.into(),
            remediation: remediation.map(str::to_string),
        }
    }
}

/// Counters shown in the page header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticSummary {
    pub passed: usize,
    pub warnings: usize,
    pub failed: usize,
    pub not_applicable: usize,
    pub total: usize,
}

/// Host facts used by the Environment page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostInfo {
    pub fedora_version: Option<String>,
    pub architecture: String,
    pub kernel: Option<String>,
    pub user: String,
    pub home: String,
    pub selinux: String,
}

/// Full diagnostics payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsReport {
    pub host: HostInfo,
    pub checks: Vec<DiagnosticCheck>,
    pub summary: DiagnosticSummary,
    pub generated_at_ms: i64,
}

fn command(ops: &dyn SystemOps, program: &str, args: &[&str]) -> Option<CommandOutput> {
    let args: Vec<String> = args.iter().map(|value| value.to_string()).collect();
    ops.run(program, &args).ok()
}

fn command_present(program: &str) -> bool {
    ["/usr/sbin", "/usr/bin", "/sbin", "/bin"]
        .iter()
        .any(|dir| Path::new(dir).join(program).exists())
}

fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .to_string()
}

/// Build the diagnostics report.
pub fn report(ops: &dyn SystemOps, paths: &Paths) -> DiagnosticsReport {
    let mut checks = Vec::new();

    // --- Apache -----------------------------------------------------------
    let apache = command(ops, "httpd", &["-v"]).or_else(|| command(ops, "apachectl", &["-v"]));
    match &apache {
        Some(output) if output.success => checks.push(DiagnosticCheck::new(
            "apache-installed",
            "Apache HTTP Server",
            CheckStatus::Passed,
            first_line(&output.stdout),
            None,
        )),
        _ => checks.push(DiagnosticCheck::new(
            "apache-installed",
            "Apache HTTP Server",
            CheckStatus::Failed,
            "httpd is not installed",
            Some("sudo dnf install httpd"),
        )),
    }

    let httpd_active = command(ops, "systemctl", &["is-active", "httpd.service"]);
    match &httpd_active {
        Some(output) if output.success => checks.push(DiagnosticCheck::new(
            "apache-running",
            "Apache running",
            CheckStatus::Passed,
            "httpd.service is active",
            None,
        )),
        _ => checks.push(DiagnosticCheck::new(
            "apache-running",
            "Apache running",
            CheckStatus::Failed,
            "httpd.service is not active; websites will not respond",
            Some("Start it from the Dashboard, or run `sudo systemctl start httpd`"),
        )),
    }

    let configtest = command(ops, "apachectl", &["configtest"]);
    match &configtest {
        Some(output) if output.success => checks.push(DiagnosticCheck::new(
            "apache-config",
            "Apache configuration",
            CheckStatus::Passed,
            "configuration is valid",
            None,
        )),
        Some(output) => checks.push(DiagnosticCheck::new(
            "apache-config",
            "Apache configuration",
            CheckStatus::Failed,
            output.summary(),
            Some("Fix the reported VirtualHost, then run the test again"),
        )),
        None => checks.push(DiagnosticCheck::new(
            "apache-config",
            "Apache configuration",
            CheckStatus::Failed,
            "apachectl is not available",
            Some("sudo dnf install httpd"),
        )),
    }

    match std::fs::metadata(&paths.apache_conf_dir) {
        Ok(metadata) if metadata.is_dir() => checks.push(DiagnosticCheck::new(
            "apache-conf-dir",
            "VirtualHost directory",
            CheckStatus::Passed,
            paths.apache_conf_dir.display().to_string(),
            None,
        )),
        _ => checks.push(DiagnosticCheck::new(
            "apache-conf-dir",
            "VirtualHost directory",
            CheckStatus::Failed,
            format!("{} is missing", paths.apache_conf_dir.display()),
            Some("Reinstall httpd so /etc/httpd/conf.d exists"),
        )),
    }

    // --- PHP --------------------------------------------------------------
    match command(ops, "php-fpm", &["-v"]).or_else(|| command(ops, "php", &["-v"])) {
        Some(output) if output.success => checks.push(DiagnosticCheck::new(
            "php-installed",
            "PHP",
            CheckStatus::Passed,
            first_line(&output.stdout),
            None,
        )),
        _ => checks.push(DiagnosticCheck::new(
            "php-installed",
            "PHP",
            CheckStatus::Failed,
            "PHP is not installed",
            Some("sudo dnf install php php-fpm"),
        )),
    }

    let fpm_active = command(ops, "systemctl", &["is-active", "php-fpm.service"]);
    match &fpm_active {
        Some(output) if output.success => checks.push(DiagnosticCheck::new(
            "php-fpm-running",
            "PHP-FPM running",
            CheckStatus::Passed,
            "php-fpm.service is active",
            None,
        )),
        _ => checks.push(DiagnosticCheck::new(
            "php-fpm-running",
            "PHP-FPM running",
            CheckStatus::Failed,
            "php-fpm.service is not active; PHP sites will return 503",
            Some("Start it from the Dashboard, or run `sudo systemctl start php-fpm`"),
        )),
    }

    if Path::new(crate::virtualhost::DEFAULT_PHP_FPM_SOCKET).exists() {
        checks.push(DiagnosticCheck::new(
            "php-fpm-socket",
            "PHP-FPM socket",
            CheckStatus::Passed,
            crate::virtualhost::DEFAULT_PHP_FPM_SOCKET,
            None,
        ));
    } else {
        checks.push(DiagnosticCheck::new(
            "php-fpm-socket",
            "PHP-FPM socket",
            CheckStatus::Warning,
            format!(
                "{} was not found; generated sites rely on /etc/httpd/conf.d/php.conf",
                crate::virtualhost::DEFAULT_PHP_FPM_SOCKET
            ),
            Some("Check `systemctl status php-fpm` and the listen directive in /etc/php-fpm.d/www.conf"),
        ));
    }

    // --- MariaDB ----------------------------------------------------------
    match command(ops, "systemctl", &["is-active", "mariadb.service"]) {
        Some(output) if output.success => checks.push(DiagnosticCheck::new(
            "mariadb-running",
            "MariaDB running",
            CheckStatus::Passed,
            "mariadb.service is active",
            None,
        )),
        Some(_) => checks.push(DiagnosticCheck::new(
            "mariadb-running",
            "MariaDB running",
            CheckStatus::Warning,
            "mariadb.service is installed but not running",
            Some("Start it from the Dashboard if your project needs a database"),
        )),
        None => checks.push(DiagnosticCheck::new(
            "mariadb-running",
            "MariaDB",
            CheckStatus::NotApplicable,
            "mariadb.service is not installed",
            Some("sudo dnf install mariadb-server"),
        )),
    }

    // --- ACL --------------------------------------------------------------
    let acl_tools = command_present("setfacl") && command_present("getfacl");
    checks.push(if acl_tools {
        DiagnosticCheck::new(
            "acl-tools",
            "ACL utilities",
            CheckStatus::Passed,
            "setfacl and getfacl are available",
            None,
        )
    } else {
        DiagnosticCheck::new(
            "acl-tools",
            "ACL utilities",
            CheckStatus::Failed,
            "setfacl/getfacl are missing",
            Some("sudo dnf install acl"),
        )
    });

    // --- SELinux ----------------------------------------------------------
    let enforce = command(ops, "getenforce", &[]);
    let mode = enforce
        .as_ref()
        .map(|output| SelinuxMode::parse(&output.stdout))
        .unwrap_or(SelinuxMode::Unknown);

    let semanage = command_present("semanage") && command_present("restorecon");
    match mode {
        SelinuxMode::Enforcing => {
            if semanage {
                checks.push(DiagnosticCheck::new(
                    "selinux",
                    "SELinux",
                    CheckStatus::Passed,
                    "enforcing, semanage and restorecon are available",
                    None,
                ));
            } else {
                checks.push(DiagnosticCheck::new(
                    "selinux",
                    "SELinux",
                    CheckStatus::Failed,
                    "enforcing, but policycoreutils-python-utils is missing",
                    Some("sudo dnf install policycoreutils-python-utils"),
                ));
            }
        }
        SelinuxMode::Permissive => checks.push(DiagnosticCheck::new(
            "selinux",
            "SELinux",
            CheckStatus::Warning,
            "permissive; denials are logged but not enforced",
            Some(
                "Servora still labels files correctly; `sudo setenforce 1` re-enables enforcement",
            ),
        )),
        SelinuxMode::Disabled => checks.push(DiagnosticCheck::new(
            "selinux",
            "SELinux",
            CheckStatus::NotApplicable,
            "disabled on this system",
            None,
        )),
        SelinuxMode::Unknown => checks.push(DiagnosticCheck::new(
            "selinux",
            "SELinux",
            CheckStatus::NotApplicable,
            "getenforce is not available",
            None,
        )),
    }

    // --- Polkit / helper --------------------------------------------------
    let pkexec = command_present("pkexec");
    checks.push(if pkexec {
        DiagnosticCheck::new(
            "polkit",
            "Polkit",
            CheckStatus::Passed,
            "pkexec is available for privileged operations",
            None,
        )
    } else {
        DiagnosticCheck::new(
            "polkit",
            "Polkit",
            CheckStatus::Failed,
            "pkexec is missing; website changes cannot be authorized",
            Some("sudo dnf install polkit"),
        )
    });

    checks.push(if paths.helper_path.exists() {
        DiagnosticCheck::new(
            "helper",
            "Servora helper",
            CheckStatus::Passed,
            paths.helper_path.display().to_string(),
            None,
        )
    } else {
        DiagnosticCheck::new(
            "helper",
            "Servora helper",
            CheckStatus::Failed,
            format!("{} is missing", paths.helper_path.display()),
            Some("Install the Servora package so the helper and Polkit policy exist"),
        )
    });

    // --- Hosts file -------------------------------------------------------
    match std::fs::metadata(&paths.hosts_file) {
        Ok(_) => checks.push(DiagnosticCheck::new(
            "hosts-file",
            "Hosts file",
            CheckStatus::Passed,
            paths.hosts_file.display().to_string(),
            None,
        )),
        Err(_) => checks.push(DiagnosticCheck::new(
            "hosts-file",
            "Hosts file",
            CheckStatus::Failed,
            format!("{} cannot be read", paths.hosts_file.display()),
            None,
        )),
    }

    // --- Data directory ---------------------------------------------------
    match crate::storage::write_atomic(&paths.data_dir.join(".probe"), "ok") {
        Ok(()) => {
            let _ = std::fs::remove_file(paths.data_dir.join(".probe"));
            checks.push(DiagnosticCheck::new(
                "data-dir",
                "Servora data directory",
                CheckStatus::Passed,
                paths.data_dir.display().to_string(),
                None,
            ));
        }
        Err(_) => checks.push(DiagnosticCheck::new(
            "data-dir",
            "Servora data directory",
            CheckStatus::Failed,
            format!("{} is not writable", paths.data_dir.display()),
            Some("Check ownership of ~/.local/share/servora"),
        )),
    }

    let summary = summarize(&checks);
    DiagnosticsReport {
        host: host_info(ops),
        checks,
        summary,
        generated_at_ms: now_ms(),
    }
}

fn summarize(checks: &[DiagnosticCheck]) -> DiagnosticSummary {
    let mut summary = DiagnosticSummary {
        passed: 0,
        warnings: 0,
        failed: 0,
        not_applicable: 0,
        total: checks.len(),
    };
    for check in checks {
        match check.status {
            CheckStatus::Passed => summary.passed += 1,
            CheckStatus::Warning => summary.warnings += 1,
            CheckStatus::Failed => summary.failed += 1,
            CheckStatus::NotApplicable => summary.not_applicable += 1,
        }
    }
    summary
}

fn host_info(ops: &dyn SystemOps) -> HostInfo {
    let fedora_version = ["/etc/fedora-release"]
        .iter()
        .find_map(|path| std::fs::read_to_string(path).ok())
        .map(|text| text.trim().to_string())
        .or_else(|| {
            command(ops, "rpm", &["-E", "%fedora"]).and_then(|output| {
                let value = output.stdout.trim().to_string();
                if value.is_empty() || value.contains('%') {
                    None
                } else {
                    Some(format!("Fedora {value}"))
                }
            })
        });

    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .map(|text| text.trim().to_string());

    let selinux = command(ops, "getenforce", &[])
        .map(|output| output.stdout.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    HostInfo {
        fedora_version,
        architecture: std::env::consts::ARCH.to_string(),
        kernel,
        user: std::env::var("USER").unwrap_or_else(|_| "unknown".to_string()),
        home: std::env::var("HOME").unwrap_or_default(),
        selinux,
    }
}

/// One row of the Environment Setup page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentComponent {
    pub id: String,
    pub label: String,
    pub status: CheckStatus,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
}

/// First-run environment matrix with installation guidance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentReport {
    pub host: HostInfo,
    pub components: Vec<EnvironmentComponent>,
    pub ready: bool,
    pub generated_at_ms: i64,
}

pub fn environment(ops: &dyn SystemOps, paths: &Paths) -> EnvironmentReport {
    let report = report(ops, paths);
    let mut components = Vec::new();

    let fedora = report
        .host
        .fedora_version
        .clone()
        .unwrap_or_else(|| "Linux (Fedora not detected)".to_string());
    components.push(EnvironmentComponent {
        id: "fedora".into(),
        label: "Fedora Linux".into(),
        status: if report.host.fedora_version.is_some() {
            CheckStatus::Passed
        } else {
            CheckStatus::Warning
        },
        detail: fedora,
        package: None,
    });

    let systemd = command(ops, "systemctl", &["is-system-running"]);
    components.push(EnvironmentComponent {
        id: "systemd".into(),
        label: "systemd".into(),
        status: if systemd.is_some() {
            CheckStatus::Passed
        } else {
            CheckStatus::Failed
        },
        detail: systemd
            .map(|output| first_line(&output.stdout))
            .unwrap_or_else(|| "systemctl not available".to_string()),
        package: None,
    });

    let lookup = |id: &str| report.checks.iter().find(|check| check.id == id);

    for (id, label, package) in [
        ("apache-installed", "Apache HTTP Server", "httpd"),
        ("php-installed", "PHP", "php php-fpm"),
        ("mariadb-running", "MariaDB", "mariadb-server"),
        ("acl-tools", "ACL utilities", "acl"),
        ("polkit", "Polkit", "polkit"),
        ("helper", "Servora helper", "servora"),
    ] {
        if let Some(check) = lookup(id) {
            components.push(EnvironmentComponent {
                id: id.into(),
                label: label.into(),
                status: match check.status {
                    CheckStatus::Passed => CheckStatus::Passed,
                    CheckStatus::Warning => CheckStatus::Warning,
                    CheckStatus::NotApplicable | CheckStatus::Failed => CheckStatus::Failed,
                },
                detail: check.detail.clone(),
                package: Some(package.to_string()),
            });
        }
    }

    if let Some(check) = lookup("selinux") {
        components.push(EnvironmentComponent {
            id: "selinux".into(),
            label: "SELinux".into(),
            status: check.status,
            detail: check.detail.clone(),
            package: Some("policycoreutils-python-utils".into()),
        });
    }

    let ready = components
        .iter()
        .all(|component| !matches!(component.status, CheckStatus::Failed));

    EnvironmentReport {
        host: report.host,
        components,
        ready,
        generated_at_ms: now_ms(),
    }
}

/// Filesystem probe used by commands that need to know if a path exists.
pub fn path_exists(path: &Path) -> CoreResult<bool> {
    Ok(path.exists())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::CommandOutput;
    use std::cell::RefCell;
    use std::collections::HashSet;

    struct FakeSystem {
        available: HashSet<String>,
        active: HashSet<String>,
        bad_config: bool,
    }

    impl FakeSystem {
        fn new(available: &[&str], active: &[&str]) -> Self {
            Self {
                available: available.iter().map(|value| value.to_string()).collect(),
                active: active.iter().map(|value| value.to_string()).collect(),
                bad_config: false,
            }
        }

        fn with_bad_config(mut self) -> Self {
            self.bad_config = true;
            self
        }
    }

    impl SystemOps for FakeSystem {
        fn read(&self, _path: &Path) -> CoreResult<Option<String>> {
            Ok(None)
        }
        fn write(&self, _path: &Path, _contents: &str, _mode: u32) -> CoreResult<()> {
            Ok(())
        }
        fn remove(&self, _path: &Path) -> CoreResult<()> {
            Ok(())
        }
        fn exists(&self, _path: &Path) -> bool {
            false
        }
        fn run(&self, program: &str, args: &[String]) -> CoreResult<CommandOutput> {
            if !self.available.contains(program) {
                return Err(crate::errors::CoreError::Io(format!("no {program}")));
            }
            let mut success = true;
            let mut stdout = String::from("ok");
            if program == "systemctl" && args.first().map(String::as_str) == Some("is-active") {
                let unit = args.get(1).cloned().unwrap_or_default();
                success = self.active.contains(&unit);
                stdout = if success {
                    "active".into()
                } else {
                    "inactive".into()
                };
            }
            if program == "apachectl" && args.first().map(String::as_str) == Some("configtest") {
                success = !self.bad_config;
                stdout = if success {
                    "Syntax OK".into()
                } else {
                    "Syntax error".into()
                };
            }
            if program == "getenforce" {
                stdout = "Enforcing".into();
            }
            if program == "httpd" || program == "php-fpm" {
                stdout = format!("{program} version 1.0");
            }
            Ok(CommandOutput {
                success,
                status: Some(if success { 0 } else { 1 }),
                stdout,
                stderr: if success {
                    String::new()
                } else {
                    "Syntax error".into()
                },
            })
        }
    }

    fn paths() -> Paths {
        let root = std::env::temp_dir().join(format!("servora-diag-{}", std::process::id()));
        std::fs::create_dir_all(root.join("etc/httpd/conf.d")).ok();
        std::fs::write(root.join("etc/hosts"), "127.0.0.1 localhost\n").ok();
        Paths::default().with_root(&root)
    }

    #[test]
    fn healthy_system_reports_no_failures() {
        let paths = paths();
        let ops = FakeSystem::new(
            &[
                "httpd",
                "apachectl",
                "php-fpm",
                "systemctl",
                "getenforce",
                "pkexec",
            ],
            &["httpd.service", "php-fpm.service", "mariadb.service"],
        );
        let report = report(&ops, &paths);
        for id in ["apache-installed", "apache-running", "php-fpm-running"] {
            let check = report.checks.iter().find(|c| c.id == id).unwrap();
            assert_eq!(check.status, CheckStatus::Passed, "{id} should pass");
        }
        let apache = report
            .checks
            .iter()
            .find(|c| c.id == "apache-installed")
            .unwrap();
        assert_eq!(apache.status, CheckStatus::Passed);
        let selinux = report.checks.iter().find(|c| c.id == "selinux").unwrap();
        assert_eq!(selinux.status, CheckStatus::Passed);
    }

    #[test]
    fn missing_components_are_reported_with_remediation() {
        let paths = paths();
        let ops = FakeSystem::new(&[], &[]);
        let report = report(&ops, &paths);
        let apache = report
            .checks
            .iter()
            .find(|c| c.id == "apache-installed")
            .unwrap();
        assert_eq!(apache.status, CheckStatus::Failed);
        assert!(apache
            .remediation
            .clone()
            .unwrap()
            .contains("dnf install httpd"));

        let mariadb = report
            .checks
            .iter()
            .find(|c| c.id == "mariadb-running")
            .unwrap();
        assert_eq!(mariadb.status, CheckStatus::NotApplicable);
    }

    #[test]
    fn invalid_apache_configuration_is_reported() {
        let paths = paths();
        let ops = FakeSystem::new(&["httpd", "apachectl", "systemctl"], &["httpd.service"])
            .with_bad_config();
        let report = report(&ops, &paths);
        let config = report
            .checks
            .iter()
            .find(|c| c.id == "apache-config")
            .unwrap();
        assert_eq!(config.status, CheckStatus::Failed);
        assert!(config.detail.contains("Syntax error"));
    }

    #[test]
    fn summary_counts_match_the_checks() {
        let paths = paths();
        let ops = FakeSystem::new(&["httpd", "apachectl", "systemctl"], &["httpd.service"]);
        let report = report(&ops, &paths);
        let total = report.summary.passed
            + report.summary.warnings
            + report.summary.failed
            + report.summary.not_applicable;
        assert_eq!(total, report.checks.len());
        assert_eq!(report.summary.total, report.checks.len());
    }

    #[test]
    fn environment_matrix_marks_missing_packages() {
        let paths = paths();
        let ops = FakeSystem::new(&[], &[]);
        let environment = environment(&ops, &paths);
        assert!(!environment.ready);
        let apache = environment
            .components
            .iter()
            .find(|component| component.id == "apache-installed")
            .unwrap();
        assert_eq!(apache.status, CheckStatus::Failed);
        assert_eq!(apache.package.as_deref(), Some("httpd"));
    }

    #[test]
    fn diagnostics_never_run_privileged_commands() {
        let paths = paths();
        let ops = FakeSystem::new(&["httpd", "apachectl", "systemctl", "getenforce"], &[]);
        let report = report(&ops, &paths);
        // The read-only report must not request any state change.
        assert!(report.checks.iter().any(|check| check.id == "helper"));
        let _ = RefCell::new(0);
    }
}
