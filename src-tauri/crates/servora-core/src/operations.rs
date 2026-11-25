//! Website operations expressed as plans.
//!
//! The functions here are pure: they read the current state that the caller
//! passes in (`SiteContext`) and return a [`Plan`]. Nothing is written until the
//! plan is executed, so the same code powers the preview shown in the UI, the
//! helper that applies the change and the unit tests.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::domain::DomainName;
use crate::errors::CoreResult;
use crate::hosts::{HostsFile, HostsState, DEFAULT_LOCAL_IP};
use crate::permissions::AclEntry;
use crate::plan::{Plan, Step, StepAction};
use crate::selinux::{FcontextRule, SelinuxMode};
use crate::storage::Paths;
use crate::virtualhost::{self, VHostSpec};
use crate::website::{ResolvedWebsite, Website};

/// Everything the plan builders need to know about the system.
#[derive(Debug, Clone)]
pub struct SiteContext {
    pub paths: Paths,
    pub apache_user: String,
    pub home: PathBuf,
    pub php_fpm_socket: Option<String>,
    pub selinux: SelinuxMode,
    /// Current contents of `/etc/hosts`.
    pub hosts_text: String,
    /// Current contents of the target configuration file, when it exists.
    pub existing_config: Option<String>,
    /// Directories inside the project that need write access.
    pub writable_dirs: Vec<String>,
}

impl SiteContext {
    pub fn hosts(&self) -> HostsFile {
        HostsFile::parse(&self.hosts_text)
    }

    pub fn config_path(&self, domain: &DomainName) -> PathBuf {
        self.paths
            .apache_conf_dir
            .join(virtualhost::config_file_name(domain))
    }
}

/// Conflicts that block or warn about an operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictReport {
    pub name_taken: bool,
    pub domain_taken: bool,
    pub config_exists: bool,
    pub config_owner: String,
    pub hosts_state: HostsState,
    pub blocking: bool,
    pub warnings: Vec<String>,
}

/// Inspect the target state before creating or renaming a website.
pub fn check_conflicts(
    resolved: &ResolvedWebsite,
    store: &crate::website::WebsiteStore,
    ctx: &SiteContext,
    ignore_id: Option<&str>,
) -> ConflictReport {
    let mut warnings = Vec::new();
    let hosts_state = ctx.hosts().state(resolved.domain.as_str());
    let config_path = ctx.config_path(&resolved.domain);
    let existing = ctx.existing_config.clone().or_else(|| {
        std::fs::read_to_string(&config_path)
            .ok()
            .or_else(|| Some(String::new()))
    });
    let config_owner = existing
        .as_deref()
        .map(|text| {
            virtualhost::classify(
                config_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("site.conf"),
                text,
            )
        })
        .unwrap_or(virtualhost::Owner::External);

    let config_exists = ctx.existing_config.is_some() || config_path.exists();
    if config_exists && !config_owner.is_owned() {
        warnings.push(format!(
            "{} already exists and is {} — it will be backed up and replaced only if you confirm",
            config_path.display(),
            config_owner.label()
        ));
    }
    match &hosts_state {
        HostsState::Unmanaged => warnings.push(format!(
            "/etc/hosts already maps {} outside the Servora block; that line is left untouched",
            resolved.domain
        )),
        HostsState::Duplicate(count) => warnings.push(format!(
            "{} appears {} times in /etc/hosts (a known siteForge rename bug); Servora will add a single managed entry",
            resolved.domain, count
        )),
        _ => {}
    }
    if let Some(existing) = store.by_project(&resolved.project_path) {
        if Some(existing.id.as_str()) != ignore_id {
            warnings.push(format!(
                "{} is already registered as `{}`",
                resolved.project_path.display(),
                existing.name
            ));
        }
    }

    let name_taken = store.name_taken(&resolved.name, ignore_id);
    let domain_taken = store.domain_taken(resolved.domain.as_str(), ignore_id);

    ConflictReport {
        name_taken,
        domain_taken,
        config_exists,
        config_owner: config_owner.label().to_string(),
        hosts_state,
        blocking: name_taken || domain_taken,
        warnings,
    }
}

fn acl_step(project: &Path, ctx: &SiteContext) -> CoreResult<Step> {
    let entries: Vec<AclEntry> =
        crate::permissions::plan(project, &ctx.home, &ctx.apache_user, &ctx.writable_dirs)?;
    Ok(Step::new(
        "acl",
        format!("grant {} read access to the project", ctx.apache_user),
        StepAction::SetAcl { entries },
    ))
}

fn selinux_steps(project: &Path, ctx: &SiteContext) -> CoreResult<Vec<Step>> {
    if !ctx.selinux.requires_contexts() {
        return Ok(vec![Step::new(
            "selinux",
            "SELinux contexts",
            StepAction::Restorecon {
                path: project.display().to_string(),
            },
        )]);
    }
    let rules: Vec<FcontextRule> = crate::selinux::rules_for(project, &ctx.writable_dirs)?;
    Ok(vec![
        Step::new(
            "selinux-context",
            "label the project for Apache",
            StepAction::SetFcontext { rules },
        ),
        Step::new(
            "selinux-restore",
            "apply the SELinux contexts",
            StepAction::Restorecon {
                path: project.display().to_string(),
            },
        ),
    ])
}

/// Plan for creating a website.
pub fn create_plan(resolved: &ResolvedWebsite, ctx: &SiteContext) -> CoreResult<Plan> {
    let config_path = ctx.config_path(&resolved.domain);
    virtualhost::ensure_inside_conf_dir(&config_path, &ctx.paths.apache_conf_dir)?;

    let spec = VHostSpec {
        website: resolved.name.clone(),
        domain: resolved.domain.clone(),
        project_path: resolved.project_path.clone(),
        document_root: resolved.document_root.clone(),
        project_type: resolved.project_type,
        php_fpm_socket: ctx.php_fpm_socket.clone(),
    };
    let rendered = virtualhost::render(&spec);
    virtualhost::validate(&rendered, &resolved.domain)?;

    let hosts = ctx
        .hosts()
        .with_domain(resolved.domain.as_str(), &resolved.id, DEFAULT_LOCAL_IP);

    let mut plan = Plan::new("create", Some(resolved.id.clone()))
        .push(Step::new(
            "write-config",
            format!("write {}", config_path.display()),
            StepAction::WriteFile {
                path: config_path.display().to_string(),
                contents: rendered,
                mode: 0o644,
                label: "vhost".into(),
            },
        ))
        .push(Step::new(
            "write-hosts",
            format!("register {} in /etc/hosts", resolved.domain),
            StepAction::WriteFile {
                path: ctx.paths.hosts_file.display().to_string(),
                contents: hosts,
                mode: 0o644,
                label: "hosts".into(),
            },
        ))
        .push(acl_step(&resolved.project_path, ctx)?);

    for step in selinux_steps(&resolved.project_path, ctx)? {
        plan = plan.push(step);
    }

    Ok(plan
        .push(Step::new(
            "configtest",
            "validate the Apache configuration",
            StepAction::ApacheConfigTest,
        ))
        .push(Step::new(
            "reload",
            "reload Apache",
            StepAction::ReloadHttpd,
        )))
}

/// Plan for editing a website (any field may change).
pub fn update_plan(
    existing: &Website,
    resolved: &ResolvedWebsite,
    ctx: &SiteContext,
) -> CoreResult<Plan> {
    let old_domain = existing.domain_name()?;
    let domain_changed = old_domain != resolved.domain;
    let project_changed = Path::new(&existing.project_path) != resolved.project_path;

    let new_config_path = ctx.config_path(&resolved.domain);
    virtualhost::ensure_inside_conf_dir(&new_config_path, &ctx.paths.apache_conf_dir)?;

    let spec = VHostSpec {
        website: resolved.name.clone(),
        domain: resolved.domain.clone(),
        project_path: resolved.project_path.clone(),
        document_root: resolved.document_root.clone(),
        project_type: resolved.project_type,
        php_fpm_socket: ctx.php_fpm_socket.clone(),
    };
    let rendered = virtualhost::render(&spec);
    virtualhost::validate(&rendered, &resolved.domain)?;

    // Hosts: drop the old domain when it changes, then register the new one.
    let hosts = if domain_changed {
        let without = HostsFile::parse(&ctx.hosts_text).without_domain(old_domain.as_str());
        HostsFile::parse(&without).with_domain(
            resolved.domain.as_str(),
            &resolved.id,
            DEFAULT_LOCAL_IP,
        )
    } else {
        HostsFile::parse(&ctx.hosts_text).with_domain(
            resolved.domain.as_str(),
            &resolved.id,
            DEFAULT_LOCAL_IP,
        )
    };

    let mut plan = Plan::new("update", Some(resolved.id.clone()));

    if domain_changed {
        let old_config_path = ctx
            .paths
            .apache_conf_dir
            .join(format!("{}.conf", old_domain));
        plan = plan.push(Step::new(
            "remove-old-config",
            format!("remove {}", old_config_path.display()),
            StepAction::RemoveFile {
                path: old_config_path.display().to_string(),
                label: "vhost".into(),
            },
        ));
    }

    plan = plan
        .push(Step::new(
            "write-config",
            format!("write {}", new_config_path.display()),
            StepAction::WriteFile {
                path: new_config_path.display().to_string(),
                contents: rendered,
                mode: 0o644,
                label: "vhost".into(),
            },
        ))
        .push(Step::new(
            "write-hosts",
            "update /etc/hosts",
            StepAction::WriteFile {
                path: ctx.paths.hosts_file.display().to_string(),
                contents: hosts,
                mode: 0o644,
                label: "hosts".into(),
            },
        ));

    if project_changed {
        // The old location keeps no stale contexts.
        plan = plan.push(Step::new(
            "selinux-cleanup",
            format!(
                "remove the SELinux context rules for {}",
                existing.project_path
            ),
            StepAction::DeleteFcontext {
                rules: crate::selinux::rules_for(Path::new(&existing.project_path), &[])
                    .unwrap_or_default(),
            },
        ));
        plan = plan.push(acl_step(&resolved.project_path, ctx)?);
        for step in selinux_steps(&resolved.project_path, ctx)? {
            plan = plan.push(step);
        }
    }

    Ok(plan
        .push(Step::new(
            "configtest",
            "validate the Apache configuration",
            StepAction::ApacheConfigTest,
        ))
        .push(Step::new(
            "reload",
            "reload Apache",
            StepAction::ReloadHttpd,
        )))
}

/// Plan for removing a website: configuration and hosts entry only.
pub fn remove_plan(website: &Website, ctx: &SiteContext) -> CoreResult<Plan> {
    let domain = website.domain_name()?;
    let config_path = ctx.config_path(&domain);

    let hosts = if ctx
        .hosts()
        .entries_for(domain.as_str())
        .iter()
        .any(|entry| entry.managed)
    {
        HostsFile::parse(&ctx.hosts_text).without_domain(domain.as_str())
    } else {
        ctx.hosts_text.clone()
    };

    Ok(Plan::new("remove", Some(website.id.clone()))
        .push(Step::new(
            "remove-config",
            format!("remove {}", config_path.display()),
            StepAction::RemoveFile {
                path: config_path.display().to_string(),
                label: "vhost".into(),
            },
        ))
        .push(Step::new(
            "write-hosts",
            format!("unregister {} from /etc/hosts", domain),
            StepAction::WriteFile {
                path: ctx.paths.hosts_file.display().to_string(),
                contents: hosts,
                mode: 0o644,
                label: "hosts".into(),
            },
        ))
        .push(Step::new(
            "selinux-cleanup",
            format!(
                "remove the SELinux context rules for {}",
                website.project_path
            ),
            StepAction::DeleteFcontext {
                rules: crate::selinux::rules_for(Path::new(&website.project_path), &[])
                    .unwrap_or_default(),
            },
        ))
        .push(Step::new(
            "configtest",
            "validate the Apache configuration",
            StepAction::ApacheConfigTest,
        ))
        .push(Step::new(
            "reload",
            "reload Apache",
            StepAction::ReloadHttpd,
        )))
}

/// Live status of a website, reconciled against the real system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebsiteLiveStatus {
    pub config_path: String,
    pub config_exists: bool,
    pub config_owner: String,
    pub serves_domain: bool,
    pub document_root: String,
    pub document_root_exists: bool,
    pub hosts_state: HostsState,
    pub healthy: bool,
    pub warnings: Vec<String>,
}

/// Compare a stored record with what is actually on disk.
pub fn live_status(website: &Website, paths: &Paths, hosts_text: &str) -> WebsiteLiveStatus {
    let mut warnings = Vec::new();
    let config_path = paths.apache_conf_dir.join(website.config_file_name());
    let text = std::fs::read_to_string(&config_path).ok();
    let parsed = text.as_deref().map(|text| {
        virtualhost::parse(
            config_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("site.conf"),
            text,
        )
    });

    let config_exists = parsed.is_some();
    let serves_domain = parsed
        .as_ref()
        .map(|parsed| parsed.primary_domain() == Some(website.domain.as_str()))
        .unwrap_or(false);
    let config_owner = parsed
        .as_ref()
        .map(|parsed| parsed.owner.label().to_string())
        .unwrap_or_else(|| "missing".to_string());

    if !config_exists {
        warnings.push("VirtualHost configuration is missing".to_string());
    } else if !serves_domain {
        warnings.push("VirtualHost does not serve the recorded domain".to_string());
    }

    let document_root = PathBuf::from(&website.document_root);
    let document_root_exists = document_root.is_dir();
    if !document_root_exists {
        warnings.push(format!(
            "document root {} is missing",
            document_root.display()
        ));
    }

    let hosts_state = HostsFile::parse(hosts_text).state(&website.domain);
    match &hosts_state {
        HostsState::Absent => warnings.push("/etc/hosts has no entry for this domain".into()),
        HostsState::Unmanaged => warnings.push("/etc/hosts entry is not managed by Servora".into()),
        HostsState::Duplicate(count) => {
            warnings.push(format!("/etc/hosts lists the domain {count} times"))
        }
        HostsState::Managed => {}
    }

    let healthy =
        config_exists && serves_domain && document_root_exists && hosts_state.is_present();

    WebsiteLiveStatus {
        config_path: config_path.display().to_string(),
        config_exists,
        config_owner,
        serves_domain,
        document_root: document_root.display().to_string(),
        document_root_exists,
        hosts_state,
        healthy,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{inspect, ProjectType};
    use crate::website::{slug, WebsiteOrigin};
    use std::fs;

    fn root(tag: &str) -> Paths {
        let dir = std::env::temp_dir().join(format!("servora-ops-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("etc/httpd/conf.d")).unwrap();
        fs::write(dir.join("etc/hosts"), "127.0.0.1 localhost\n").unwrap();
        Paths::default().with_root(&dir)
    }

    fn laravel(paths: &Paths) -> PathBuf {
        let dir = paths.data_dir.parent().unwrap().join("Work/e-perpus");
        fs::create_dir_all(dir.join("public")).unwrap();
        fs::create_dir_all(dir.join("storage")).unwrap();
        fs::create_dir_all(dir.join("bootstrap/cache")).unwrap();
        fs::write(dir.join("artisan"), "x").unwrap();
        fs::write(dir.join("public/index.php"), "<?php").unwrap();
        dir
    }

    fn context(paths: &Paths, project: &Path, existing: Option<&str>) -> SiteContext {
        SiteContext {
            paths: paths.clone(),
            apache_user: "apache".into(),
            home: paths.data_dir.parent().unwrap().to_path_buf(),
            php_fpm_socket: None,
            selinux: SelinuxMode::Permissive,
            hosts_text: fs::read_to_string(&paths.hosts_file).unwrap(),
            existing_config: existing.map(str::to_string),
            writable_dirs: vec!["storage".into(), "bootstrap/cache".into()],
        }
    }

    fn resolved(project: &Path, name: &str, domain: &str) -> ResolvedWebsite {
        ResolvedWebsite {
            id: slug(name),
            name: name.into(),
            domain: DomainName::parse(domain).unwrap(),
            project_path: project.canonicalize().unwrap(),
            document_root: project.canonicalize().unwrap().join("public"),
            project_type: ProjectType::Laravel,
        }
    }

    #[test]
    fn create_plan_covers_every_required_step_in_order() {
        let paths = root("create");
        let project = laravel(&paths);
        let ctx = context(&paths, &project, None);
        let website = resolved(&project, "e-perpus", "e-perpus.test");

        let plan = create_plan(&website, &ctx).unwrap();
        let ids: Vec<&str> = plan.steps.iter().map(|step| step.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "write-config",
                "write-hosts",
                "acl",
                "selinux-context",
                "selinux-restore",
                "configtest",
                "reload"
            ]
        );
        plan.validate(&paths).unwrap();

        match &plan.steps[0].action {
            StepAction::WriteFile { contents, .. } => {
                assert!(contents.contains("ServerName e-perpus.test"));
                assert!(contents.contains("# website: e-perpus"));
            }
            other => panic!("unexpected action {other:?}"),
        }
        match &plan.steps[1].action {
            StepAction::WriteFile { contents, .. } => {
                assert!(contents.contains("e-perpus.test # servora:e-perpus"));
            }
            other => panic!("unexpected action {other:?}"),
        }
        fs::remove_dir_all(paths.data_dir.parent().unwrap()).ok();
    }

    #[test]
    fn create_plan_never_targets_unmanaged_paths() {
        let paths = root("guard");
        let project = laravel(&paths);
        let ctx = context(&paths, &project, None);
        let plan = create_plan(&resolved(&project, "a", "a.test"), &ctx).unwrap();
        for step in &plan.steps {
            if let StepAction::WriteFile { path, .. } = &step.action {
                assert!(
                    path.starts_with(paths.apache_conf_dir.to_str().unwrap())
                        || path == paths.hosts_file.to_str().unwrap()
                );
            }
        }
        fs::remove_dir_all(paths.data_dir.parent().unwrap()).ok();
    }

    #[test]
    fn update_plan_handles_a_domain_rename() {
        let paths = root("rename");
        let project = laravel(&paths);
        fs::write(
            paths.apache_conf_dir.join("old.test.conf"),
            virtualhost::render(&VHostSpec {
                website: "site".into(),
                domain: DomainName::parse("old.test").unwrap(),
                project_path: project.clone(),
                document_root: project.join("public"),
                project_type: ProjectType::Laravel,
                php_fpm_socket: None,
            }),
        )
        .unwrap();

        let ctx = context(&paths, &project, None);
        let existing = Website {
            id: "site".into(),
            name: "site".into(),
            domain: "old.test".into(),
            project_path: project.display().to_string(),
            document_root: project.join("public").display().to_string(),
            project_type: ProjectType::Laravel,
            origin: Some(WebsiteOrigin::Created),
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        let plan = update_plan(&existing, &resolved(&project, "site", "new.test"), &ctx).unwrap();
        let ids: Vec<&str> = plan.steps.iter().map(|step| step.id.as_str()).collect();
        assert!(ids.contains(&"remove-old-config"));
        assert!(ids.contains(&"write-config"));
        // Project did not change: no ACL or SELinux work.
        assert!(!ids.contains(&"acl"));

        let hosts = plan
            .steps
            .iter()
            .find_map(|step| match &step.action {
                StepAction::WriteFile { path, contents, .. }
                    if path == paths.hosts_file.to_str().unwrap() =>
                {
                    Some(contents.clone())
                }
                _ => None,
            })
            .unwrap();
        assert!(hosts.contains("new.test"));
        assert!(!hosts.contains("old.test"));
        fs::remove_dir_all(paths.data_dir.parent().unwrap()).ok();
    }

    #[test]
    fn update_plan_keeps_the_hosts_entry_for_the_same_domain() {
        let paths = root("same-domain");
        let project = laravel(&paths);
        let ctx = context(&paths, &project, None);
        let existing = Website {
            id: "site".into(),
            name: "site".into(),
            domain: "site.test".into(),
            project_path: project.display().to_string(),
            document_root: project.join("public").display().to_string(),
            project_type: ProjectType::Laravel,
            origin: Some(WebsiteOrigin::Created),
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        let plan = update_plan(&existing, &resolved(&project, "site", "site.test"), &ctx).unwrap();
        let ids: Vec<&str> = plan.steps.iter().map(|step| step.id.as_str()).collect();
        assert!(!ids.contains(&"remove-old-config"));
        fs::remove_dir_all(paths.data_dir.parent().unwrap()).ok();
    }

    #[test]
    fn remove_plan_drops_configuration_and_hosts_entry_only() {
        let paths = root("remove");
        let project = laravel(&paths);
        let hosts = HostsFile::parse(&fs::read_to_string(&paths.hosts_file).unwrap()).with_domain(
            "site.test",
            "site",
            DEFAULT_LOCAL_IP,
        );
        fs::write(&paths.hosts_file, &hosts).unwrap();
        fs::write(
            paths.apache_conf_dir.join("site.test.conf"),
            "# Managed by Servora\n<VirtualHost *:80>\nServerName site.test\n</VirtualHost>\n",
        )
        .unwrap();

        let ctx = context(&paths, &project, None);
        let website = Website {
            id: "site".into(),
            name: "site".into(),
            domain: "site.test".into(),
            project_path: project.display().to_string(),
            document_root: project.join("public").display().to_string(),
            project_type: ProjectType::Laravel,
            origin: Some(WebsiteOrigin::Created),
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        let plan = remove_plan(&website, &ctx).unwrap();
        let ids: Vec<&str> = plan.steps.iter().map(|step| step.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "remove-config",
                "write-hosts",
                "selinux-cleanup",
                "configtest",
                "reload"
            ]
        );

        // The SELinux context rules Servora added are taken out again.
        let cleanup = plan
            .steps
            .iter()
            .find(|step| step.id == "selinux-cleanup")
            .unwrap();
        let StepAction::DeleteFcontext { rules } = &cleanup.action else {
            panic!("expected a DeleteFcontext step");
        };
        assert!(!rules.is_empty(), "the project rules must be removed");
        for rule in rules {
            assert!(rule.pattern.starts_with('/'));
            assert_eq!(rule.delete_args()[1], "-d");
        }

        // No project deletion and no ACL reversal.
        assert!(!plan.steps.iter().any(|step| matches!(
            &step.action,
            StepAction::RemoveFile { path, .. } if path.contains("Work")
        )));
        fs::remove_dir_all(paths.data_dir.parent().unwrap()).ok();
    }

    #[test]
    fn conflicts_detect_names_domains_and_unmanaged_configs() {
        let paths = root("conflicts");
        let project = laravel(&paths);
        let ctx = context(&paths, &project, None);
        let mut store = crate::website::WebsiteStore::load(&paths.websites_file()).unwrap();
        let website = resolved(&project, "site", "site.test");
        store
            .insert(Website {
                id: website.id.clone(),
                name: website.name.clone(),
                domain: website.domain.as_str().to_string(),
                project_path: website.project_path.display().to_string(),
                document_root: website.document_root.display().to_string(),
                project_type: website.project_type,
                origin: Some(WebsiteOrigin::Created),
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();

        let duplicate = check_conflicts(&website, &store, &ctx, None);
        assert!(duplicate.name_taken);
        assert!(duplicate.domain_taken);
        assert!(duplicate.blocking);

        let renamed = check_conflicts(&website, &store, &ctx, Some("site"));
        assert!(!renamed.blocking);

        // A foreign configuration for the same domain is a warning, not a block.
        let external = context(
            &paths,
            &project,
            Some("<VirtualHost *:80>\nServerName other.test\n</VirtualHost>\n"),
        );
        let report = check_conflicts(
            &resolved(&project, "x", "other.test"),
            &store,
            &external,
            None,
        );
        assert!(!report.blocking);
        assert!(!report.config_owner.contains("Servora"));
        fs::remove_dir_all(paths.data_dir.parent().unwrap()).ok();
    }

    #[test]
    fn live_status_reports_drift() {
        let paths = root("live");
        let project = laravel(&paths);
        let hosts = HostsFile::parse(&fs::read_to_string(&paths.hosts_file).unwrap()).with_domain(
            "site.test",
            "site",
            DEFAULT_LOCAL_IP,
        );
        fs::write(&paths.hosts_file, &hosts).unwrap();

        let website = Website {
            id: "site".into(),
            name: "site".into(),
            domain: "site.test".into(),
            project_path: project.display().to_string(),
            document_root: project.join("public").display().to_string(),
            project_type: ProjectType::Laravel,
            origin: Some(WebsiteOrigin::Created),
            created_at_ms: 1,
            updated_at_ms: 1,
        };

        let missing = live_status(&website, &paths, &hosts);
        assert!(!missing.config_exists);
        assert!(!missing.healthy);

        let inspection = inspect(&project).unwrap();
        let rendered = virtualhost::render(&VHostSpec {
            website: "site".into(),
            domain: DomainName::parse("site.test").unwrap(),
            project_path: project.clone(),
            document_root: project.join("public"),
            project_type: inspection.project_type,
            php_fpm_socket: None,
        });
        fs::write(paths.apache_conf_dir.join("site.test.conf"), &rendered).unwrap();

        let healthy = live_status(&website, &paths, &hosts);
        assert!(healthy.config_exists);
        assert!(healthy.serves_domain);
        assert!(healthy.healthy);
        assert!(healthy.warnings.is_empty());
        fs::remove_dir_all(paths.data_dir.parent().unwrap()).ok();
    }
}
