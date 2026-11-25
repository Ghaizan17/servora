//! `servora-helper` — the only privileged part of Servora.
//!
//! The desktop application never runs as root. When a website has to be created,
//! edited, imported or removed, the GUI hands a [`servora_core::plan::Plan`] to
//! this helper through `pkexec`, and Polkit
//! (`dev.servora.app.manage-sites`) decides whether the user may proceed.
//!
//! The helper is deliberately dumb and paranoid:
//!
//! * it accepts **only** plans that touch `/etc/httpd/conf.d` and `/etc/hosts`;
//! * every command it runs is hard coded inside `servora-core` (no shell, ever);
//! * it refuses oversized or empty plans;
//! * it prints a JSON result on stdout and diagnostics on stderr, so the GUI can
//!   report exactly which step failed and whether the rollback completed.
//!
//! Usage:
//!
//! ```text
//! servora-helper --version            print the helper version
//! servora-helper --apply-plan         read a plan as JSON on stdin, execute it
//! ```

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use servora_core::plan::{execute, LocalSystem, Plan, PlanOutcome};
use servora_core::storage::Paths;

/// Hard upper bound on the number of steps a single plan may carry.
const MAX_STEPS: usize = 256;

/// Locations the helper is willing to touch. These are compiled in on purpose:
/// they must not depend on the caller's environment.
const CONF_DIR: &str = "/etc/httpd/conf.d";
const HOSTS_FILE: &str = "/etc/hosts";

/// Where the helper keeps its backups. It runs as root with `HOME=/root`, so a
/// per-user directory would be wrong (and invisible to the user anyway).
const HELPER_DATA_DIR: &str = "/var/lib/servora";

fn production_paths() -> Paths {
    // Ignore SERVORA_TEST_ROOT: a privileged helper must never be retargeted.
    let mut paths = Paths::default();
    paths.apache_conf_dir = PathBuf::from(CONF_DIR);
    paths.hosts_file = PathBuf::from(HOSTS_FILE);
    paths.data_dir = PathBuf::from(HELPER_DATA_DIR);
    paths
}

/// Every mutation must stay inside the managed locations.
fn validate_paths(plan: &Plan) -> Result<(), String> {
    if plan.steps.is_empty() {
        return Err("the plan contains no steps".into());
    }
    if plan.steps.len() > MAX_STEPS {
        return Err(format!(
            "the plan has {} steps, at most {MAX_STEPS} are allowed",
            plan.steps.len()
        ));
    }
    for step in &plan.steps {
        if let servora_core::plan::StepAction::DeleteFcontext { rules } = &step.action {
            for rule in rules {
                // Only absolute SELinux patterns may be removed.
                if !rule.pattern.starts_with('/') || rule.pattern.len() > 512 {
                    return Err(format!(
                        "step `{}` removes an invalid SELinux pattern: {}",
                        step.id, rule.pattern
                    ));
                }
            }
        }
        if let servora_core::plan::StepAction::WriteFile { path, .. }
        | servora_core::plan::StepAction::RemoveFile { path, .. } = &step.action
        {
            let candidate = Path::new(path);
            let allowed = candidate == Path::new(HOSTS_FILE)
                || candidate.parent() == Some(Path::new(CONF_DIR));
            if !allowed {
                return Err(format!(
                    "step `{}` targets {}, which is outside {CONF_DIR} and {HOSTS_FILE}",
                    step.id, path
                ));
            }
            if candidate
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
            {
                return Err(format!("step `{}` targets a path with `..`", step.id));
            }
        }
    }
    Ok(())
}

fn run_plan(raw: &str) -> Result<PlanOutcome, String> {
    let plan: Plan =
        serde_json::from_str(raw).map_err(|err| format!("invalid plan JSON: {err}"))?;
    validate_paths(&plan)?;
    let paths = production_paths();
    let outcome = execute(&plan, &LocalSystem, Some(&paths));
    Ok(outcome)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("");

    match command {
        "--version" | "-v" => {
            println!("servora-helper {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        "--apply-plan" | "apply-plan" => {
            let mut raw = String::new();
            if let Err(err) = std::io::stdin().read_to_string(&mut raw) {
                eprintln!("servora-helper: could not read the plan: {err}");
                return ExitCode::FAILURE;
            }

            match run_plan(&raw) {
                Ok(outcome) => {
                    let success = outcome.success;
                    let json = serde_json::to_string(&outcome)
                        .unwrap_or_else(|_| "{\"success\":false}".to_string());
                    println!("{json}");
                    let _ = std::io::stdout().flush();
                    if success {
                        eprintln!(
                            "servora-helper: {} completed ({} steps)",
                            outcome.operation,
                            outcome.steps.len()
                        );
                        ExitCode::SUCCESS
                    } else {
                        eprintln!(
                            "servora-helper: {} failed: {}",
                            outcome.operation,
                            outcome.failure_message().unwrap_or_default()
                        );
                        ExitCode::FAILURE
                    }
                }
                Err(message) => {
                    eprintln!("servora-helper: {message}");
                    let payload = serde_json::json!({
                        "success": false,
                        "operation": "rejected",
                        "steps": [],
                        "rollback": {"notNeeded": null},
                        "warnings": [message],
                        "finishedAtMs": 0
                    });
                    println!("{payload}");
                    ExitCode::FAILURE
                }
            }
        }
        "" | "--help" | "-h" => {
            eprintln!("usage: servora-helper --apply-plan | --version");
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("servora-helper: unknown argument `{other}`");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use servora_core::plan::{Plan, Step, StepAction};

    fn plan_with(path: &str) -> Plan {
        Plan::new("create", Some("e-perpus".into())).push(Step::new(
            "write-config",
            "write the configuration",
            StepAction::WriteFile {
                path: path.into(),
                contents: "# Managed by Servora\n".into(),
                mode: 0o644,
                label: "vhost".into(),
            },
        ))
    }

    #[test]
    fn accepts_paths_inside_the_managed_locations() {
        assert!(validate_paths(&plan_with("/etc/httpd/conf.d/e-perpus.test.conf")).is_ok());
        assert!(validate_paths(&plan_with("/etc/hosts")).is_ok());
    }

    #[test]
    fn rejects_paths_outside_the_managed_locations() {
        for path in [
            "/etc/passwd",
            "/etc/httpd/conf/httpd.conf",
            "/etc/httpd/conf.d/../conf/httpd.conf",
            "/usr/libexec/servora-helper",
            "/tmp/evil.conf",
        ] {
            assert!(
                validate_paths(&plan_with(path)).is_err(),
                "{path} must be rejected"
            );
        }
    }

    #[test]
    fn rejects_empty_and_oversized_plans() {
        assert!(validate_paths(&Plan::new("noop", None)).is_err());

        let mut plan = Plan::new("big", None);
        for index in 0..(MAX_STEPS + 1) {
            plan = plan.push(Step::new(
                format!("step-{index}"),
                "write",
                StepAction::WriteFile {
                    path: "/etc/hosts".into(),
                    contents: String::new(),
                    mode: 0o644,
                    label: "hosts".into(),
                },
            ));
        }
        assert!(validate_paths(&plan).is_err());
    }

    #[test]
    fn rejects_selinux_patterns_that_are_not_absolute() {
        let mut plan = Plan::new("remove", Some("e-perpus".into()));
        plan = plan.push(Step::new(
            "selinux-cleanup",
            "clean up",
            StepAction::DeleteFcontext {
                rules: vec![servora_core::selinux::FcontextRule {
                    pattern: "relative/path(/.*)?".into(),
                    context: "httpd_sys_content_t".into(),
                    reason: "test".into(),
                }],
            },
        ));
        assert!(validate_paths(&plan).is_err());

        let mut ok = Plan::new("remove", Some("e-perpus".into()));
        ok = ok.push(Step::new(
            "selinux-cleanup",
            "clean up",
            StepAction::DeleteFcontext {
                rules: vec![servora_core::selinux::FcontextRule {
                    pattern: "/tmp/site(/.*)?".into(),
                    context: "httpd_sys_content_t".into(),
                    reason: "test".into(),
                }],
            },
        ));
        assert!(validate_paths(&ok).is_ok());
    }

    #[test]
    fn backups_go_to_a_root_owned_directory() {
        let paths = production_paths();
        assert_eq!(paths.data_dir, PathBuf::from("/var/lib/servora"));
        assert_eq!(
            paths.backups_dir(),
            PathBuf::from("/var/lib/servora/backups")
        );
    }

    #[test]
    fn rejects_a_plan_that_is_not_json() {
        assert!(run_plan("not json").is_err());
        assert!(run_plan("{}").is_err());
    }

    #[test]
    fn rejects_a_plan_targeting_an_unmanaged_path_before_executing_it() {
        let json = serde_json::to_string(&plan_with("/etc/passwd")).unwrap();
        let error = run_plan(&json).unwrap_err();
        assert!(error.contains("outside"), "{error}");
    }
}
