//! Transactional plans.
//!
//! Every mutation Servora performs is first expressed as a [`Plan`]: an ordered
//! list of [`Step`]s with fixed arguments. The plan is executed either by the
//! privileged helper (production) or directly against a temporary root (tests).
//! File writes and removals record their previous content so a failure can be
//! rolled back before the user ever sees a half-applied change.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::errors::{CoreError, CoreResult};
use crate::permissions::AclEntry;
use crate::selinux::FcontextRule;
use crate::storage::{backup_file, now_ms, Paths};

/// A single filesystem or service action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum StepAction {
    WriteFile {
        path: String,
        contents: String,
        #[serde(default = "default_mode")]
        mode: u32,
        label: String,
    },
    RemoveFile {
        path: String,
        label: String,
    },
    SetAcl {
        entries: Vec<AclEntry>,
    },
    SetFcontext {
        rules: Vec<FcontextRule>,
    },
    Restorecon {
        path: String,
    },
    /// `apachectl configtest`
    ApacheConfigTest,
    /// `systemctl reload httpd`
    ReloadHttpd,
}

fn default_mode() -> u32 {
    0o644
}

/// A step with a human readable description and a stable id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub id: String,
    pub description: String,
    pub action: StepAction,
}

impl Step {
    pub fn new(id: impl Into<String>, description: impl Into<String>, action: StepAction) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
            action,
        }
    }
}

/// The complete ordered change set for one user action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub operation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    pub steps: Vec<Step>,
}

impl Plan {
    pub fn new(operation: impl Into<String>, website: Option<String>) -> Self {
        Self {
            operation: operation.into(),
            website,
            steps: Vec::new(),
        }
    }

    pub fn push(mut self, step: Step) -> Self {
        self.steps.push(step);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Refuse to execute a plan that would touch a path outside the managed
    /// locations. This is the last line of defence before the helper runs.
    pub fn validate(&self, paths: &Paths) -> CoreResult<()> {
        for step in &self.steps {
            match &step.action {
                StepAction::WriteFile { path, .. } | StepAction::RemoveFile { path, .. } => {
                    let candidate = Path::new(path);
                    if candidate == paths.hosts_file
                        || candidate.starts_with(&paths.apache_conf_dir)
                    {
                        continue;
                    }
                    return Err(CoreError::InvalidPath(format!(
                        "step `{}` targets {} which Servora does not manage",
                        step.id, path
                    )));
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Paths that should be backed up before the plan runs.
    pub fn backup_targets(&self) -> Vec<String> {
        let mut targets = Vec::new();
        for step in &self.steps {
            match &step.action {
                StepAction::WriteFile { path, .. } | StepAction::RemoveFile { path, .. } => {
                    if !targets.contains(path) {
                        targets.push(path.clone());
                    }
                }
                _ => {}
            }
        }
        targets
    }
}

/// Status of a single executed step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StepStatus {
    Ok,
    Failed,
    Skipped,
}

/// Result of a single executed step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepResult {
    pub id: String,
    pub description: String,
    pub status: StepStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl StepResult {
    pub fn ok(step: &Step) -> Self {
        Self {
            id: step.id.clone(),
            description: step.description.clone(),
            status: StepStatus::Ok,
            message: None,
        }
    }

    pub fn failed(step: &Step, message: impl Into<String>) -> Self {
        Self {
            id: step.id.clone(),
            description: step.description.clone(),
            status: StepStatus::Failed,
            message: Some(message.into()),
        }
    }

    pub fn skipped(step: &Step, message: impl Into<String>) -> Self {
        Self {
            id: step.id.clone(),
            description: step.description.clone(),
            status: StepStatus::Skipped,
            message: Some(message.into()),
        }
    }
}

/// Rollback outcome reported back to the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RollbackStatus {
    /// Nothing had to be undone.
    NotNeeded,
    /// Every recorded change was reverted.
    Completed,
    /// Some changes could not be reverted; details are listed.
    Partial { problems: Vec<String> },
}

/// Full outcome of a plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanOutcome {
    pub success: bool,
    pub operation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    pub steps: Vec<StepResult>,
    pub rollback: RollbackStatus,
    pub warnings: Vec<String>,
    pub finished_at_ms: i64,
}

impl PlanOutcome {
    pub fn failure_message(&self) -> Option<String> {
        self.steps
            .iter()
            .find(|step| step.status == StepStatus::Failed)
            .map(|step| {
                format!(
                    "{}: {}",
                    step.description,
                    step.message.clone().unwrap_or_default()
                )
            })
    }
}

/// Output of an external command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandOutput {
    pub success: bool,
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    pub fn summary(&self) -> String {
        let text = if self.stderr.trim().is_empty() {
            self.stdout.trim()
        } else {
            self.stderr.trim()
        };
        let text = text.lines().take(6).collect::<Vec<_>>().join("\n");
        if text.is_empty() {
            format!("exit status {:?}", self.status)
        } else {
            text
        }
    }
}

/// Abstraction over the system so plans can run against a fake in tests.
pub trait SystemOps {
    fn read(&self, path: &Path) -> CoreResult<Option<String>>;
    fn write(&self, path: &Path, contents: &str, mode: u32) -> CoreResult<()>;
    fn remove(&self, path: &Path) -> CoreResult<()>;
    fn exists(&self, path: &Path) -> bool;
    fn run(&self, program: &str, args: &[String]) -> CoreResult<CommandOutput>;
}

/// Real system implementation (used by the helper).
#[derive(Debug, Default, Clone, Copy)]
pub struct LocalSystem;

impl SystemOps for LocalSystem {
    fn read(&self, path: &Path) -> CoreResult<Option<String>> {
        crate::storage::read_to_string_opt(path)
    }

    fn write(&self, path: &Path, contents: &str, mode: u32) -> CoreResult<()> {
        use std::os::unix::fs::PermissionsExt;
        crate::storage::write_atomic(path, contents)?;
        let permissions = std::fs::Permissions::from_mode(mode);
        std::fs::set_permissions(path, permissions)
            .map_err(|err| CoreError::Io(format!("chmod {}: {err}", path.display())))?;
        Ok(())
    }

    fn remove(&self, path: &Path) -> CoreResult<()> {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(CoreError::Io(format!("remove {}: {err}", path.display()))),
        }
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn run(&self, program: &str, args: &[String]) -> CoreResult<CommandOutput> {
        let output = std::process::Command::new(program)
            .args(args)
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .env("LANG", "C.UTF-8")
            .output()
            .map_err(|err| CoreError::Io(format!("could not run {program}: {err}")))?;
        Ok(CommandOutput {
            success: output.status.success(),
            status: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }
}

/// Snapshot of a file taken before it is modified.
struct Snapshot {
    path: PathBuf,
    contents: Option<String>,
}

/// Execute `plan`, rolling the file changes back if any step fails.
pub fn execute(plan: &Plan, ops: &dyn SystemOps, backups: Option<&Paths>) -> PlanOutcome {
    let mut results = Vec::new();
    let mut snapshots: Vec<Snapshot> = Vec::new();
    let mut warnings = Vec::new();
    let mut failed = false;

    for step in &plan.steps {
        if failed {
            results.push(StepResult::skipped(
                step,
                "skipped after an earlier failure",
            ));
            continue;
        }

        let outcome = run_step(step, ops, &mut snapshots, backups, &mut warnings);
        match outcome {
            Ok(result) => {
                if result.status == StepStatus::Failed {
                    failed = true;
                }
                results.push(result);
            }
            Err(error) => {
                failed = true;
                results.push(StepResult::failed(step, error.to_string()));
            }
        }
    }

    let rollback = if failed {
        rollback(snapshots, ops)
    } else {
        RollbackStatus::NotNeeded
    };

    PlanOutcome {
        success: !failed,
        operation: plan.operation.clone(),
        website: plan.website.clone(),
        steps: results,
        rollback,
        warnings,
        finished_at_ms: now_ms(),
    }
}

fn run_step(
    step: &Step,
    ops: &dyn SystemOps,
    snapshots: &mut Vec<Snapshot>,
    backups: Option<&Paths>,
    warnings: &mut Vec<String>,
) -> CoreResult<StepResult> {
    match &step.action {
        StepAction::WriteFile {
            path,
            contents,
            mode,
            label,
        } => {
            let target = PathBuf::from(path);
            let previous = ops.read(&target)?;
            if previous.as_deref() == Some(contents.as_str()) {
                return Ok(StepResult::skipped(step, "already up to date"));
            }
            if let Some(paths) = backups {
                if let Ok(Some(stored)) = backup_file(paths, &target, label) {
                    warnings.push(format!("backup written to {}", stored.display()));
                }
            }
            snapshots.push(Snapshot {
                path: target.clone(),
                contents: previous,
            });
            ops.write(&target, contents, *mode)?;
            Ok(StepResult::ok(step))
        }
        StepAction::RemoveFile { path, label } => {
            let target = PathBuf::from(path);
            let previous = ops.read(&target)?;
            match previous {
                None => Ok(StepResult::skipped(step, "file is already gone")),
                Some(contents) => {
                    if let Some(paths) = backups {
                        if let Ok(Some(stored)) = backup_file(paths, &target, label) {
                            warnings.push(format!("backup written to {}", stored.display()));
                        }
                    }
                    snapshots.push(Snapshot {
                        path: target.clone(),
                        contents: Some(contents),
                    });
                    ops.remove(&target)?;
                    Ok(StepResult::ok(step))
                }
            }
        }
        StepAction::SetAcl { entries } => {
            for entry in entries {
                let output = ops.run("setfacl", &entry.setfacl_args())?;
                if !output.success {
                    return Ok(StepResult::failed(
                        step,
                        format!("setfacl {}: {}", entry.path, output.summary()),
                    ));
                }
            }
            Ok(StepResult::ok(step))
        }
        StepAction::SetFcontext { rules } => {
            let listing = ops.run("semanage", &["fcontext".into(), "-l".into(), "-C".into()])?;
            for rule in rules {
                let exists = crate::selinux::rule_exists(&listing.stdout, &rule.pattern);
                let args = if exists {
                    rule.modify_args()
                } else {
                    rule.add_args()
                };
                let output = ops.run("semanage", &args)?;
                if !output.success {
                    return Ok(StepResult::failed(
                        step,
                        format!("semanage {}: {}", rule.pattern, output.summary()),
                    ));
                }
            }
            Ok(StepResult::ok(step))
        }
        StepAction::Restorecon { path } => {
            let output = ops.run(
                "restorecon",
                &crate::selinux::restorecon_args(Path::new(path)),
            )?;
            if !output.success {
                warnings.push(format!("restorecon {}: {}", path, output.summary()));
            }
            Ok(StepResult::ok(step))
        }
        StepAction::ApacheConfigTest => {
            let output = ops.run("apachectl", &["configtest".into()])?;
            if output.success {
                Ok(StepResult::ok(step))
            } else {
                Ok(StepResult::failed(step, output.summary()))
            }
        }
        StepAction::ReloadHttpd => {
            let output = ops.run("systemctl", &["reload".into(), "httpd.service".into()])?;
            if output.success {
                Ok(StepResult::ok(step))
            } else {
                Ok(StepResult::failed(step, output.summary()))
            }
        }
    }
}

fn rollback(snapshots: Vec<Snapshot>, ops: &dyn SystemOps) -> RollbackStatus {
    let mut problems = Vec::new();
    for snapshot in snapshots.into_iter().rev() {
        let result = match &snapshot.contents {
            Some(contents) => ops.write(&snapshot.path, contents, 0o644),
            None => ops.remove(&snapshot.path),
        };
        if let Err(error) = result {
            problems.push(format!("{}: {error}", snapshot.path.display()));
        }
    }
    if problems.is_empty() {
        RollbackStatus::Completed
    } else {
        RollbackStatus::Partial { problems }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// In-memory system used by the plan tests.
    struct FakeSystem {
        files: RefCell<HashMap<String, String>>,
        failing: Vec<String>,
        commands: RefCell<Vec<Vec<String>>>,
    }

    impl FakeSystem {
        fn new() -> Self {
            Self {
                files: RefCell::new(HashMap::new()),
                failing: Vec::new(),
                commands: RefCell::new(Vec::new()),
            }
        }

        fn with_file(self, path: &str, contents: &str) -> Self {
            self.files
                .borrow_mut()
                .insert(path.to_string(), contents.to_string());
            self
        }

        fn fail_on(mut self, path: &str) -> Self {
            self.failing.push(path.to_string());
            self
        }

        fn get(&self, path: &str) -> Option<String> {
            self.files.borrow().get(path).cloned()
        }
    }

    impl SystemOps for FakeSystem {
        fn read(&self, path: &Path) -> CoreResult<Option<String>> {
            Ok(self.get(&path.display().to_string()))
        }

        fn write(&self, path: &Path, contents: &str, _mode: u32) -> CoreResult<()> {
            let key = path.display().to_string();
            if self.failing.contains(&key) {
                return Err(CoreError::Io(format!("refusing to write {key}")));
            }
            self.files.borrow_mut().insert(key, contents.to_string());
            Ok(())
        }

        fn remove(&self, path: &Path) -> CoreResult<()> {
            self.files.borrow_mut().remove(&path.display().to_string());
            Ok(())
        }

        fn exists(&self, path: &Path) -> bool {
            self.get(&path.display().to_string()).is_some()
        }

        fn run(&self, program: &str, args: &[String]) -> CoreResult<CommandOutput> {
            self.commands.borrow_mut().push(
                std::iter::once(program.to_string())
                    .chain(args.iter().cloned())
                    .collect(),
            );
            Ok(CommandOutput {
                success: true,
                status: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    fn write_step(path: &str, contents: &str) -> Step {
        Step::new(
            format!("write:{path}"),
            format!("write {path}"),
            StepAction::WriteFile {
                path: path.into(),
                contents: contents.into(),
                mode: 0o644,
                label: "site".into(),
            },
        )
    }

    fn test_paths() -> Paths {
        let root = std::env::temp_dir().join(format!("servora-plan-{}", std::process::id()));
        Paths::default().with_root(&root)
    }

    #[test]
    fn successful_plan_writes_every_step() {
        let system = FakeSystem::new();
        let plan = Plan::new("create", Some("e-perpus".into()))
            .push(write_step("/etc/httpd/conf.d/a.conf", "one"))
            .push(write_step("/etc/hosts", "two"));

        let outcome = execute(&plan, &system, None);
        assert!(outcome.success);
        assert_eq!(outcome.steps.len(), 2);
        assert!(outcome
            .steps
            .iter()
            .all(|step| step.status == StepStatus::Ok));
        assert_eq!(outcome.rollback, RollbackStatus::NotNeeded);
        assert_eq!(system.get("/etc/hosts").as_deref(), Some("two"));
    }

    #[test]
    fn failure_rolls_back_previous_writes() {
        let system = FakeSystem::new()
            .with_file("/etc/httpd/conf.d/a.conf", "original")
            .fail_on("/etc/hosts");
        let plan = Plan::new("create", None)
            .push(write_step("/etc/httpd/conf.d/a.conf", "changed"))
            .push(write_step("/etc/hosts", "added"));

        let outcome = execute(&plan, &system, None);
        assert!(!outcome.success);
        assert_eq!(outcome.steps[0].status, StepStatus::Ok);
        assert_eq!(outcome.steps[1].status, StepStatus::Failed);
        assert_eq!(outcome.rollback, RollbackStatus::Completed);
        assert_eq!(
            system.get("/etc/httpd/conf.d/a.conf").as_deref(),
            Some("original"),
            "the first write must be reverted"
        );
        assert!(system.get("/etc/hosts").is_none());
        assert!(outcome.failure_message().is_some());
    }

    #[test]
    fn rollback_removes_files_that_did_not_exist_before() {
        let system = FakeSystem::new().fail_on("/etc/hosts");
        let plan = Plan::new("create", None)
            .push(write_step("/etc/httpd/conf.d/new.conf", "created"))
            .push(write_step("/etc/hosts", "boom"));

        let outcome = execute(&plan, &system, None);
        assert!(!outcome.success);
        assert!(system.get("/etc/httpd/conf.d/new.conf").is_none());
    }

    #[test]
    fn remaining_steps_are_skipped_after_a_failure() {
        let system = FakeSystem::new().fail_on("/etc/hosts");
        let plan = Plan::new("create", None)
            .push(write_step("/etc/hosts", "boom"))
            .push(write_step("/etc/httpd/conf.d/b.conf", "never"));

        let outcome = execute(&plan, &system, None);
        assert_eq!(outcome.steps[1].status, StepStatus::Skipped);
        assert!(system.get("/etc/httpd/conf.d/b.conf").is_none());
    }

    #[test]
    fn identical_content_is_skipped() {
        let system = FakeSystem::new().with_file("/etc/hosts", "same");
        let plan = Plan::new("noop", None).push(write_step("/etc/hosts", "same"));
        let outcome = execute(&plan, &system, None);
        assert!(outcome.success);
        assert_eq!(outcome.steps[0].status, StepStatus::Skipped);
        assert_eq!(outcome.rollback, RollbackStatus::NotNeeded);
    }

    #[test]
    fn commands_are_invoked_with_fixed_argv() {
        let system = FakeSystem::new();
        let plan = Plan::new("test", None)
            .push(Step::new(
                "configtest",
                "validate",
                StepAction::ApacheConfigTest,
            ))
            .push(Step::new("reload", "reload", StepAction::ReloadHttpd));

        let outcome = execute(&plan, &system, None);
        assert!(outcome.success);
        let calls = system.commands.borrow();
        assert_eq!(calls[0], vec!["apachectl", "configtest"]);
        assert_eq!(calls[1], vec!["systemctl", "reload", "httpd.service"]);
    }

    #[test]
    fn plan_rejects_paths_outside_the_managed_locations() {
        let paths = test_paths();
        let inside = Plan::new("ok", None).push(write_step(
            &paths.apache_conf_dir.join("a.conf").display().to_string(),
            "x",
        ));
        assert!(inside.validate(&paths).is_ok());

        let outside = Plan::new("bad", None).push(write_step("/etc/passwd", "x"));
        let error = outside.validate(&paths).unwrap_err();
        assert_eq!(error.code(), crate::errors::codes::INVALID_PATH);
    }

    #[test]
    fn backup_targets_are_unique() {
        let plan = Plan::new("create", None)
            .push(write_step("/etc/hosts", "a"))
            .push(write_step("/etc/hosts", "b"))
            .push(write_step("/etc/httpd/conf.d/a.conf", "c"));
        let targets = plan.backup_targets();
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0], "/etc/hosts");
    }

    #[test]
    fn plan_serialises_as_json() {
        let plan = Plan::new("create", Some("e-perpus".into())).push(Step::new(
            "write",
            "write the configuration",
            StepAction::WriteFile {
                path: "/etc/httpd/conf.d/a.conf".into(),
                contents: "x".into(),
                mode: 0o644,
                label: "site".into(),
            },
        ));
        let json = serde_json::to_string(&plan).unwrap();
        assert!(json.contains("\"kind\":\"writeFile\""));
        let parsed: Plan = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, plan);
    }
}
