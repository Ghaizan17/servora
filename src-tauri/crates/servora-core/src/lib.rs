//! Servora core — the standalone business logic behind the Servora desktop app.
//!
//! This crate knows nothing about Tauri, the GUI or siteForge. It owns:
//!
//! * website records and their validation ([`website`]),
//! * project type and document root detection ([`project`]),
//! * domain validation and `/etc/hosts` management ([`domain`], [`hosts`]),
//! * Apache VirtualHost generation and parsing ([`virtualhost`]),
//! * ACL and SELinux planning ([`permissions`], [`selinux`]),
//! * transactional plans with rollback ([`plan`], [`operations`]),
//! * migration of existing siteForge/legacy sites ([`migration`]),
//! * environment diagnostics ([`diagnostics`]).
//!
//! Everything that mutates the system is expressed as a [`plan::Plan`], which is
//! executed either by the privileged helper (`servora-helper`) or, in tests,
//! against a temporary root.

pub mod diagnostics;
pub mod domain;
pub mod errors;
pub mod hosts;
pub mod migration;
pub mod operations;
pub mod permissions;
pub mod plan;
pub mod project;
pub mod selinux;
pub mod storage;
pub mod virtualhost;
pub mod website;

pub use errors::{CoreError, CoreErrorPayload, CoreResult};
pub use storage::Paths;

/// Version of the metadata/plan formats. Bumped when the on-disk shape changes.
pub const SCHEMA_VERSION: u32 = 1;

#[cfg(test)]
mod integration_tests {
    //! End-to-end behaviour of the core against a temporary system root.

    use crate::domain::DomainName;
    use crate::hosts::{HostsFile, HostsState};
    use crate::operations::{create_plan, live_status, remove_plan, update_plan, SiteContext};
    use crate::plan::{execute, PlanOutcome, StepAction};
    use crate::project::ProjectType;
    use crate::selinux::SelinuxMode;
    use crate::storage::Paths;
    use crate::website::{slug, ResolvedWebsite, Website, WebsiteOrigin, WebsiteStore};
    use std::fs;
    use std::path::{Path, PathBuf};

    /// Minimal `SystemOps` that writes into a temporary root directory.
    struct TempSystem {
        root: PathBuf,
    }

    impl crate::plan::SystemOps for TempSystem {
        fn read(&self, path: &Path) -> crate::CoreResult<Option<String>> {
            crate::storage::read_to_string_opt(path)
        }

        fn write(&self, path: &Path, contents: &str, _mode: u32) -> crate::CoreResult<()> {
            crate::storage::write_atomic(path, contents)
        }

        fn remove(&self, path: &Path) -> crate::CoreResult<()> {
            match fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(err) => Err(crate::CoreError::Io(err.to_string())),
            }
        }

        fn exists(&self, path: &Path) -> bool {
            path.exists()
        }

        fn run(
            &self,
            program: &str,
            args: &[String],
        ) -> crate::CoreResult<crate::plan::CommandOutput> {
            Ok(crate::plan::CommandOutput {
                success: true,
                status: Some(0),
                stdout: format!("{program} {:?}", args),
                stderr: String::new(),
            })
        }
    }

    fn setup(tag: &str) -> Paths {
        let dir =
            std::env::temp_dir().join(format!("servora-core-it-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("etc/httpd/conf.d")).unwrap();
        fs::write(dir.join("etc/hosts"), "127.0.0.1 localhost\n").unwrap();
        let project = dir.join("Work/e-perpus");
        fs::create_dir_all(project.join("public")).unwrap();
        fs::create_dir_all(project.join("storage")).unwrap();
        fs::write(project.join("artisan"), "x").unwrap();
        fs::write(project.join("public/index.php"), "<?php").unwrap();
        Paths::default().with_root(&dir)
    }

    fn context(paths: &Paths, existing: Option<String>) -> SiteContext {
        SiteContext {
            paths: paths.clone(),
            apache_user: "apache".into(),
            home: paths.data_dir.parent().unwrap().to_path_buf(),
            php_fpm_socket: None,
            selinux: SelinuxMode::Disabled,
            hosts_text: fs::read_to_string(&paths.hosts_file).unwrap(),
            existing_config: existing,
            writable_dirs: vec!["storage".into()],
        }
    }

    fn resolved(paths: &Paths, name: &str, domain: &str) -> ResolvedWebsite {
        let project = paths.data_dir.parent().unwrap().join("Work/e-perpus");
        ResolvedWebsite {
            id: slug(name),
            name: name.into(),
            domain: DomainName::parse(domain).unwrap(),
            project_path: project.clone(),
            document_root: project.join("public"),
            project_type: ProjectType::Laravel,
        }
    }

    fn apply(plan: &crate::plan::Plan, paths: &Paths) -> PlanOutcome {
        execute(
            plan,
            &TempSystem {
                root: paths.data_dir.clone(),
            },
            Some(paths),
        )
    }

    #[test]
    fn create_list_and_remove_full_cycle() {
        let paths = setup("cycle");
        let ctx = context(&paths, None);
        let website = resolved(&paths, "e-perpus", "e-perpus.test");

        let plan = create_plan(&website, &ctx).unwrap();
        let outcome = apply(&plan, &paths);
        assert!(outcome.success, "{:?}", outcome.steps);
        assert!(paths.apache_conf_dir.join("e-perpus.test.conf").exists());

        let hosts = fs::read_to_string(&paths.hosts_file).unwrap();
        assert_eq!(
            HostsFile::parse(&hosts).state("e-perpus.test"),
            HostsState::Managed
        );

        let mut store = WebsiteStore::load(&paths.websites_file()).unwrap();
        store
            .insert(WebsiteStore::new_record(
                &website,
                WebsiteOrigin::Created,
                1,
            ))
            .unwrap();
        store.save().unwrap();

        let status = live_status(&website_record(&website), &paths, &hosts);
        assert!(status.healthy, "{:?}", status.warnings);

        let remove = remove_plan(&website_record(&website), &context(&paths, None)).unwrap();
        let outcome = apply(&remove, &paths);
        assert!(outcome.success);
        assert!(!paths.apache_conf_dir.join("e-perpus.test.conf").exists());
        let hosts = fs::read_to_string(&paths.hosts_file).unwrap();
        assert_eq!(
            HostsFile::parse(&hosts).state("e-perpus.test"),
            HostsState::Absent
        );
        // The project is untouched.
        assert!(paths
            .data_dir
            .parent()
            .unwrap()
            .join("Work/e-perpus/public/index.php")
            .exists());

        fs::remove_dir_all(paths.data_dir.parent().unwrap()).ok();
    }

    fn website_record(resolved: &ResolvedWebsite) -> Website {
        WebsiteStore::new_record(resolved, WebsiteOrigin::Created, 1)
    }

    #[test]
    fn rename_keeps_the_previous_configuration_recoverable() {
        let paths = setup("rename-cycle");
        let ctx = context(&paths, None);
        let website = resolved(&paths, "e-perpus", "e-perpus.test");
        assert!(apply(&create_plan(&website, &ctx).unwrap(), &paths).success);

        let record = website_record(&website);
        let renamed = resolved(&paths, "e-perpus", "eperpus.local");
        let plan = update_plan(&record, &renamed, &context(&paths, None)).unwrap();
        let outcome = apply(&plan, &paths);
        assert!(outcome.success);

        assert!(paths.apache_conf_dir.join("eperpus.local.conf").exists());
        assert!(!paths.apache_conf_dir.join("e-perpus.test.conf").exists());
        let hosts = fs::read_to_string(&paths.hosts_file).unwrap();
        assert!(hosts.contains("eperpus.local"));
        assert!(!hosts.contains("e-perpus.test"));
        assert!(outcome
            .warnings
            .iter()
            .any(|warning| warning.starts_with("backup written to")));

        fs::remove_dir_all(paths.data_dir.parent().unwrap()).ok();
    }

    #[test]
    fn a_failing_step_leaves_the_system_as_it_was() {
        let paths = setup("rollback");
        let ctx = context(&paths, None);
        let website = resolved(&paths, "e-perpus", "e-perpus.test");
        let plan = create_plan(&website, &ctx).unwrap();

        // Make the hosts file unwritable by replacing it with a directory.
        fs::remove_file(&paths.hosts_file).unwrap();
        fs::create_dir_all(&paths.hosts_file).unwrap();

        let outcome = execute(
            &plan,
            &TempSystem {
                root: paths.data_dir.clone(),
            },
            None,
        );
        assert!(!outcome.success);
        assert_eq!(outcome.rollback, crate::plan::RollbackStatus::Completed);
        assert!(
            !paths.apache_conf_dir.join("e-perpus.test.conf").exists(),
            "the configuration written before the failure must be rolled back"
        );

        fs::remove_dir_all(paths.data_dir.parent().unwrap()).ok();
    }

    #[test]
    fn plans_only_touch_managed_paths() {
        let paths = setup("paths");
        let ctx = context(&paths, None);
        let plan = create_plan(&resolved(&paths, "a", "a.test"), &ctx).unwrap();
        plan.validate(&paths).unwrap();
        for step in &plan.steps {
            if let StepAction::WriteFile { path, .. } = &step.action {
                let path = PathBuf::from(path);
                assert!(
                    path.starts_with(&paths.apache_conf_dir) || path == paths.hosts_file,
                    "{path:?} is outside the managed locations"
                );
            }
        }
        fs::remove_dir_all(paths.data_dir.parent().unwrap()).ok();
    }
}
