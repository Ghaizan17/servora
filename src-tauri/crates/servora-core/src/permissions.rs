//! Access control planning for the Apache user.
//!
//! Apache (user `apache`) must be able to traverse the project path and read the
//! files it serves, plus write into the framework's runtime directories. Servora
//! expresses this as data ([`AclEntry`]) so the same plan can be rendered into
//! `setfacl` arguments by the helper and unit tested without touching the system.
//!
//! The strategy mirrors siteForge: traverse-only on the parent directories,
//! read/execute recursively on the project, read/write/execute (with default
//! entries) on `storage`, `bootstrap/cache` and `wp-content/uploads`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::errors::{CoreError, CoreResult};

/// One `setfacl` operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AclEntry {
    pub path: String,
    pub user: String,
    pub perms: String,
    pub recursive: bool,
    /// Also set the default ACL for new files (directories only).
    pub default: bool,
    pub reason: String,
}

impl AclEntry {
    /// `setfacl` arguments, always as an argv vector (never a shell string).
    pub fn setfacl_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if self.recursive {
            args.push("-R".to_string());
        }
        if self.default {
            args.push("-d".to_string());
        }
        args.push("-m".to_string());
        args.push(format!("u:{}:{}", self.user, self.perms));
        args.push(self.path.clone());
        args
    }

    /// `getfacl -p <path>` arguments used to verify the entry.
    pub fn getfacl_args(&self) -> Vec<String> {
        vec!["-p".to_string(), self.path.clone()]
    }
}

/// Build the ACL plan for a project.
pub fn plan(
    project: &Path,
    home: &Path,
    apache_user: &str,
    writable_dirs: &[String],
) -> CoreResult<Vec<AclEntry>> {
    let canonical = project
        .canonicalize()
        .map_err(|err| CoreError::InvalidPath(format!("{}: {err}", project.display())))?;

    let mut entries = Vec::new();

    // Traverse-only on the parents so Apache can reach the project without
    // being able to list the whole home directory.
    let mut parents: Vec<PathBuf> = canonical
        .ancestors()
        .skip(1)
        .take_while(|ancestor| ancestor.starts_with(home))
        .map(Path::to_path_buf)
        .collect();
    parents.reverse();
    for parent in parents {
        entries.push(AclEntry {
            path: parent.display().to_string(),
            user: apache_user.to_string(),
            perms: "--x".to_string(),
            recursive: false,
            default: false,
            reason: "let Apache traverse to the project".to_string(),
        });
    }

    entries.push(AclEntry {
        path: canonical.display().to_string(),
        user: apache_user.to_string(),
        perms: "rx".to_string(),
        recursive: true,
        default: false,
        reason: "let Apache read the project files".to_string(),
    });

    for directory in writable_dirs {
        let path = canonical.join(directory);
        if !path.is_dir() {
            continue;
        }
        entries.push(AclEntry {
            path: path.display().to_string(),
            user: apache_user.to_string(),
            perms: "rwx".to_string(),
            recursive: true,
            default: true,
            reason: format!("{directory} needs to be writable at runtime"),
        });
    }

    Ok(entries)
}

/// Presence of an ACL entry for `user` in `getfacl` output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AclStatus {
    Present(String),
    Absent,
    Unknown,
}

impl AclStatus {
    pub fn label(&self) -> &'static str {
        match self {
            AclStatus::Present(_) => "granted",
            AclStatus::Absent => "missing",
            AclStatus::Unknown => "unknown",
        }
    }

    pub fn is_present(&self) -> bool {
        matches!(self, AclStatus::Present(_))
    }
}

/// Read `user:<name>:<perms>` out of `getfacl` output.
pub fn status_from_getfacl(output: &str, user: &str) -> AclStatus {
    if output.trim().is_empty() {
        return AclStatus::Unknown;
    }
    for line in output.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("user:") {
            let mut parts = rest.split(':');
            let name = parts.next().unwrap_or_default();
            let perms = parts.next().unwrap_or_default();
            let perms = perms.split('#').next().unwrap_or("").trim();
            if name.eq_ignore_ascii_case(user) {
                return AclStatus::Present(perms.to_string());
            }
        }
    }
    AclStatus::Absent
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn project(root: &Path) -> PathBuf {
        let dir = root.join("Work/e-perpus");
        fs::create_dir_all(dir.join("public")).unwrap();
        fs::create_dir_all(dir.join("storage/logs")).unwrap();
        fs::create_dir_all(dir.join("bootstrap/cache")).unwrap();
        fs::write(dir.join("artisan"), "x").unwrap();
        dir
    }

    #[test]
    fn plan_grants_traverse_then_read_then_write() {
        let root = std::env::temp_dir().join(format!("servora-acl-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("Work")).unwrap();
        let dir = project(&root);
        let home = root.canonicalize().unwrap();

        let entries = plan(
            &dir,
            &home,
            "apache",
            &["storage".to_string(), "bootstrap/cache".to_string()],
        )
        .unwrap();

        assert!(entries.iter().any(|entry| entry.perms == "--x"));
        let read = entries
            .iter()
            .find(|entry| entry.perms == "rx")
            .expect("read entry");
        assert!(read.recursive);
        assert!(read.path.ends_with("e-perpus"));

        let storage = entries
            .iter()
            .find(|entry| entry.path.ends_with("storage"))
            .expect("storage entry");
        assert_eq!(storage.perms, "rwx");
        assert!(storage.default);

        assert!(entries.iter().all(|entry| entry.user == "apache"));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn setfacl_arguments_are_argv_safe() {
        let entry = AclEntry {
            path: "/home/u/Work/my project".into(),
            user: "apache".into(),
            perms: "rwx".into(),
            recursive: true,
            default: true,
            reason: "test".into(),
        };
        assert_eq!(
            entry.setfacl_args(),
            vec!["-R", "-d", "-m", "u:apache:rwx", "/home/u/Work/my project"]
        );
    }

    #[test]
    fn getfacl_output_is_parsed() {
        let sample = "# file: /home/u/Work/e-perpus\n# owner: u\n# group: u\n\
user::rwx\nuser:apache:r-x\ngroup::r-x\nmask::r-x\nother::r-x\n";
        assert_eq!(
            status_from_getfacl(sample, "apache"),
            AclStatus::Present("r-x".into())
        );
        assert_eq!(status_from_getfacl(sample, "nginx"), AclStatus::Absent);
        assert_eq!(status_from_getfacl("", "apache"), AclStatus::Unknown);
    }

    #[test]
    fn missing_project_is_reported() {
        let error = plan(
            Path::new("/nonexistent/servora-project"),
            Path::new("/home"),
            "apache",
            &[],
        )
        .unwrap_err();
        assert_eq!(error.code(), crate::errors::codes::INVALID_PATH);
    }
}
