//! SELinux integration.
//!
//! Servora never disables SELinux. When it is enforcing, the project needs
//! `httpd_sys_content_t` (readable by Apache) and the runtime directories need
//! `httpd_sys_rw_content_t`. Rules are expressed as data so they can be applied
//! by the helper (`semanage fcontext`, `restorecon`) and tested without root.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::errors::{CoreError, CoreResult};

pub const CONTENT_TYPE: &str = "httpd_sys_content_t";
pub const RW_TYPE: &str = "httpd_sys_rw_content_t";

/// SELinux enforcement mode as reported by `getenforce`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SelinuxMode {
    Enforcing,
    Permissive,
    Disabled,
    Unknown,
}

impl SelinuxMode {
    pub fn parse(output: &str) -> Self {
        match output.trim().to_ascii_lowercase().as_str() {
            "enforcing" => SelinuxMode::Enforcing,
            "permissive" => SelinuxMode::Permissive,
            "disabled" => SelinuxMode::Disabled,
            _ => SelinuxMode::Unknown,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            SelinuxMode::Enforcing => "enforcing",
            SelinuxMode::Permissive => "permissive",
            SelinuxMode::Disabled => "disabled",
            SelinuxMode::Unknown => "unknown",
        }
    }

    /// Rules only matter when SELinux is actually labelling files.
    pub fn requires_contexts(&self) -> bool {
        matches!(self, SelinuxMode::Enforcing | SelinuxMode::Permissive)
    }
}

/// One `semanage fcontext` rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FcontextRule {
    /// Regular expression accepted by `semanage fcontext`.
    pub pattern: String,
    pub context: String,
    pub reason: String,
}

impl FcontextRule {
    /// Arguments for creating the rule.
    pub fn add_args(&self) -> Vec<String> {
        vec![
            "fcontext".to_string(),
            "-a".to_string(),
            "-t".to_string(),
            self.context.clone(),
            self.pattern.clone(),
        ]
    }

    /// Arguments for removing the rule again.
    pub fn delete_args(&self) -> Vec<String> {
        vec![
            "fcontext".to_string(),
            "-d".to_string(),
            self.pattern.clone(),
        ]
    }

    /// Arguments for modifying an existing rule (used when it is already present).
    pub fn modify_args(&self) -> Vec<String> {
        vec![
            "fcontext".to_string(),
            "-m".to_string(),
            "-t".to_string(),
            self.context.clone(),
            self.pattern.clone(),
        ]
    }
}

/// Rules for a project, mirroring the siteForge conventions:
/// the project tree is readable, the runtime directories are writable.
pub fn rules_for(project: &Path, writable_dirs: &[String]) -> CoreResult<Vec<FcontextRule>> {
    let canonical = project
        .canonicalize()
        .map_err(|err| CoreError::InvalidPath(format!("{}: {err}", project.display())))?;
    let base = canonical.display().to_string();
    if base.contains('"') || base.contains('\n') {
        return Err(CoreError::InvalidPath(format!(
            "{base} contains characters SELinux patterns cannot express"
        )));
    }

    let mut rules = vec![FcontextRule {
        pattern: format!("{base}(/.*)?"),
        context: CONTENT_TYPE.to_string(),
        reason: "Apache must read the project files".to_string(),
    }];

    for directory in writable_dirs {
        if !canonical.join(directory).is_dir() {
            continue;
        }
        rules.push(FcontextRule {
            pattern: format!("{base}/{directory}(/.*)?"),
            context: RW_TYPE.to_string(),
            reason: format!("{directory} is written at runtime"),
        });
    }

    Ok(rules)
}

/// `restorecon` arguments for a project tree.
pub fn restorecon_args(project: &Path) -> Vec<String> {
    vec!["-RF".to_string(), project.display().to_string()]
}

/// True when a rule with this pattern is already customised on the system.
pub fn rule_exists(listing: &str, pattern: &str) -> bool {
    listing.lines().any(|line| line.contains(pattern))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn enforcement_modes_are_parsed() {
        assert_eq!(SelinuxMode::parse("Enforcing\n"), SelinuxMode::Enforcing);
        assert_eq!(SelinuxMode::parse("permissive"), SelinuxMode::Permissive);
        assert_eq!(SelinuxMode::parse("Disabled"), SelinuxMode::Disabled);
        assert_eq!(SelinuxMode::parse("nonsense"), SelinuxMode::Unknown);
        assert!(SelinuxMode::Enforcing.requires_contexts());
        assert!(!SelinuxMode::Disabled.requires_contexts());
    }

    #[test]
    fn rules_cover_the_project_and_writable_directories() {
        let root = std::env::temp_dir().join(format!("servora-selinux-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let project = root.join("e-perpus");
        fs::create_dir_all(project.join("storage")).unwrap();
        fs::create_dir_all(project.join("public")).unwrap();

        let rules = rules_for(&project, &["storage".to_string(), "missing".to_string()]).unwrap();
        assert_eq!(rules.len(), 2, "missing directories are skipped");
        assert_eq!(rules[0].context, CONTENT_TYPE);
        assert!(rules[0].pattern.ends_with("e-perpus(/.*)?"));
        assert_eq!(rules[1].context, RW_TYPE);
        assert!(rules[1].pattern.ends_with("e-perpus/storage(/.*)?"));

        assert_eq!(
            rules[0].add_args(),
            vec!["fcontext", "-a", "-t", CONTENT_TYPE, &rules[0].pattern]
        );
        assert_eq!(rules[0].modify_args()[1], "-m");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn existing_rules_are_detected() {
        let listing = format!(
            "/home/u/Work/e-perpus(/.*)?  all files  system_u:object_r:{CONTENT_TYPE}:s0\n"
        );
        assert!(rule_exists(&listing, "/home/u/Work/e-perpus(/.*)?"));
        assert!(!rule_exists(&listing, "/home/u/Work/other(/.*)?"));
    }

    #[test]
    fn restorecon_arguments_are_argv_safe() {
        assert_eq!(
            restorecon_args(Path::new("/home/u/Work/my project")),
            vec!["-RF", "/home/u/Work/my project"]
        );
    }

    #[test]
    fn missing_project_is_rejected() {
        assert!(rules_for(Path::new("/nonexistent/project"), &[]).is_err());
    }
}

/// `semanage fcontext -d` reports a missing rule as an error; that is not a
/// failure for Servora, which only ever removes what it may have added.
pub fn is_missing_rule(stderr: &str) -> bool {
    let text = stderr.to_ascii_lowercase();
    text.contains("not defined") || text.contains("no such file") || text.contains("does not exist")
}
