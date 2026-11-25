//! Website manager orchestration.
//!
//! This module glues `servora-core` to the Tauri application:
//!
//! 1. read the current system state (hosts file, Apache configuration, SELinux),
//! 2. ask the core to build a plan,
//! 3. execute it through the privileged helper,
//! 4. reconcile the metadata store with the result.
//!
//! Nothing here re-implements business rules — validation, rendering and
//! rollback all live in the core crate.

use std::path::Path;

use serde::{Deserialize, Serialize};

use servora_core::diagnostics::{self, DiagnosticsReport, EnvironmentReport};
use servora_core::hosts::HostsFile;
use servora_core::migration::{
    self, MigrationCandidate, MigrationLog, MigrationRecord, MigrationScan,
};
use servora_core::operations::{
    check_conflicts, create_plan, live_status, remove_plan, update_plan, ConflictReport,
    SiteContext, WebsiteLiveStatus,
};
use servora_core::plan::{Plan, PlanOutcome};
use servora_core::project::{self, ProjectInspection};
use servora_core::selinux::SelinuxMode;
use servora_core::storage::{now_ms, Paths};
use servora_core::virtualhost::{self, ParsedVHost};
use servora_core::website::{ResolvedWebsite, Website, WebsiteInput, WebsiteOrigin, WebsiteStore};

use crate::error::{AppError, AppResult};
use crate::services::{logging, privileged};

/// User the Apache worker runs as.
pub const DEFAULT_APACHE_USER: &str = "apache";

/// A website plus the state Servora observed on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebsiteView {
    #[serde(flatten)]
    pub website: Website,
    pub url: String,
    pub status: WebsiteLiveStatus,
    /// Single word the UI can switch on: `live` or the first thing that is wrong.
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl WebsiteView {
    fn build(website: Website, paths: &Paths, hosts: &str) -> Self {
        let status = live_status(&website, paths, hosts);
        let state = if status.healthy {
            "live"
        } else if !status.config_exists {
            "config-missing"
        } else if !status.serves_domain {
            "config-mismatch"
        } else if !status.document_root_exists {
            "project-missing"
        } else if !status.hosts_state.is_present() {
            "hosts-missing"
        } else {
            "degraded"
        };
        WebsiteView {
            url: website.url(),
            detail: status.warnings.first().cloned(),
            status,
            state: state.to_string(),
            website,
        }
    }
}

/// Result of a create / update / remove / import.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteOperationReport {
    pub operation: String,
    pub website_id: Option<String>,
    pub website_name: Option<String>,
    pub success: bool,
    pub outcome: Option<PlanOutcome>,
    pub website: Option<Website>,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Result of an import run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub imported: Vec<SiteOperationReport>,
    pub skipped: Vec<String>,
    pub failed: Vec<SiteOperationReport>,
}

/// One history entry (proves what Servora did and when).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub at_ms: i64,
    pub operation: String,
    pub website: String,
    pub success: bool,
    pub summary: String,
}

/// Facade over the core used by the Tauri commands.
#[derive(Debug, Clone)]
pub struct SiteService {
    paths: Paths,
}

impl Default for SiteService {
    fn default() -> Self {
        Self::new()
    }
}

impl SiteService {
    pub fn new() -> Self {
        Self {
            paths: Paths::system(),
        }
    }

    pub fn with_paths(paths: Paths) -> Self {
        Self { paths }
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    // ---------------------------------------------------------------- state

    pub fn store(&self) -> AppResult<WebsiteStore> {
        self.paths.ensure_dirs()?;
        Ok(WebsiteStore::load(&self.paths.websites_file())?)
    }

    pub fn hosts_text(&self) -> AppResult<String> {
        Ok(
            servora_core::storage::read_to_string_opt(&self.paths.hosts_file)?
                .unwrap_or_else(|| "127.0.0.1 localhost\n".to_string()),
        )
    }

    /// Apache worker user; detected from `httpd.conf` when possible.
    pub fn apache_user(&self) -> String {
        let configured = std::fs::read_to_string("/etc/httpd/conf/httpd.conf")
            .ok()
            .and_then(|text| {
                text.lines().find_map(|line| {
                    let line = line.trim();
                    line.strip_prefix("User ")
                        .map(|value| value.trim().to_string())
                })
            });
        configured.unwrap_or_else(|| DEFAULT_APACHE_USER.to_string())
    }

    pub fn selinux_mode(&self) -> SelinuxMode {
        std::process::Command::new("getenforce")
            .output()
            .ok()
            .map(|output| SelinuxMode::parse(&String::from_utf8_lossy(&output.stdout)))
            .unwrap_or(SelinuxMode::Unknown)
    }

    pub fn php_socket(&self) -> Option<String> {
        let socket = virtualhost::DEFAULT_PHP_FPM_SOCKET;
        if Path::new(socket).exists() {
            Some(socket.to_string())
        } else {
            None
        }
    }

    fn writable_dirs(&self, project: &Path) -> Vec<String> {
        project::inspect(project)
            .map(|inspection| inspection.writable_dirs)
            .unwrap_or_default()
    }

    pub fn context(&self, existing_config: Option<String>) -> AppResult<SiteContext> {
        Ok(SiteContext {
            paths: self.paths.clone(),
            apache_user: self.apache_user(),
            home: std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_default(),
            php_fpm_socket: self.php_socket(),
            selinux: self.selinux_mode(),
            hosts_text: self.hosts_text()?,
            existing_config,
            writable_dirs: Vec::new(),
        })
    }

    fn context_for(&self, project: &Path) -> AppResult<SiteContext> {
        let mut context = self.context(None)?;
        context.writable_dirs = self.writable_dirs(project);
        Ok(context)
    }

    // ------------------------------------------------------------- reading

    pub fn views(&self) -> AppResult<Vec<WebsiteView>> {
        let store = self.store()?;
        let hosts = self.hosts_text()?;
        Ok(store
            .websites()
            .iter()
            .map(|website| WebsiteView::build(website.clone(), &self.paths, &hosts))
            .collect())
    }

    pub fn website_view(&self, id: &str) -> AppResult<WebsiteView> {
        let store = self.store()?;
        let website = store
            .get(id)
            .ok_or_else(|| AppError::from(servora_core::CoreError::NotFound(id.to_string())))?
            .clone();
        let hosts = self.hosts_text()?;
        Ok(WebsiteView::build(website, &self.paths, &hosts))
    }

    /// Parse a live VirtualHost file, for the details page.
    pub fn virtualhost(&self, domain: &str) -> AppResult<Option<ParsedVHost>> {
        let path = self.paths.apache_conf_dir.join(format!("{domain}.conf"));
        let Some(text) = servora_core::storage::read_to_string_opt(&path)? else {
            return Ok(None);
        };
        Ok(Some(virtualhost::parse(
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("site.conf"),
            &text,
        )))
    }

    pub fn inspect_project(&self, path: &Path) -> AppResult<ProjectInspection> {
        Ok(project::inspect(path)?)
    }

    pub fn conflicts(
        &self,
        input: &WebsiteInput,
        ignore: Option<&str>,
    ) -> AppResult<ConflictReport> {
        let resolved = input.resolve()?;
        let store = self.store()?;
        let context = self.context_for(&resolved.project_path)?;
        Ok(check_conflicts(&resolved, &store, &context, ignore))
    }

    pub fn validate_domain(&self, domain: &str) -> AppResult<serde_json::Value> {
        let store = self.store()?;
        let hosts = self.hosts_text()?;
        match servora_core::domain::DomainName::parse(domain) {
            Ok(parsed) => {
                let taken = store.domain_taken(parsed.as_str(), None);
                let hosts_state = HostsFile::parse(&hosts).state(parsed.as_str());
                Ok(serde_json::json!({
                    "valid": true,
                    "domain": parsed.as_str(),
                    "isLocalDevelopment": parsed.is_local_development(),
                    "suffix": parsed.local_suffix(),
                    "url": parsed.url(),
                    "takenByServora": taken,
                    "hostsState": hosts_state,
                }))
            }
            Err(error) => Ok(serde_json::json!({
                "valid": false,
                "domain": domain,
                "message": error.to_string(),
                "hint": error.hint(),
            })),
        }
    }

    pub fn managed_domains(&self) -> AppResult<serde_json::Value> {
        let hosts = HostsFile::parse(&self.hosts_text()?);
        let store = self.store()?;
        let mut domains = Vec::new();
        for entry in hosts.entries() {
            if !entry.managed {
                continue;
            }
            let website = entry
                .website
                .as_deref()
                .and_then(|id| store.get(id))
                .or_else(|| store.by_domain(&entry.domain));
            domains.push(serde_json::json!({
                "domain": entry.domain,
                "ip": entry.ip,
                "website": website.map(|w| w.name.clone()),
                "websiteId": website.map(|w| w.id.clone()),
                "managed": true,
                "line": entry.line,
            }));
        }
        Ok(serde_json::json!({
            "domains": domains,
            "hostsPath": self.paths.hosts_file.display().to_string(),
            "servoraWebsites": store.websites().iter().map(|w| serde_json::json!({
                "domain": w.domain,
                "website": w.name,
                "websiteId": w.id,
            })).collect::<Vec<_>>(),
        }))
    }

    // ---------------------------------------------------------- operations

    pub async fn create(&self, input: &WebsiteInput) -> AppResult<SiteOperationReport> {
        let resolved = input.resolve()?;
        let mut store = self.store()?;

        let context = self.context_for(&resolved.project_path)?;
        let conflicts = check_conflicts(&resolved, &store, &context, None);
        if conflicts.name_taken {
            return Err(AppError::from(servora_core::CoreError::Duplicate(
                resolved.name.clone(),
            )));
        }
        if conflicts.domain_taken {
            return Err(AppError::from(servora_core::CoreError::Duplicate(
                resolved.domain.to_string(),
            )));
        }

        let plan = create_plan(&resolved, &context)?;
        let outcome = self.run(&plan).await?;

        let mut report = SiteOperationReport {
            operation: "create".into(),
            website_id: Some(resolved.id.clone()),
            website_name: Some(resolved.name.clone()),
            success: outcome.success,
            warnings: outcome.warnings.clone(),
            outcome: Some(outcome.clone()),
            website: None,
            message: outcome.failure_message(),
        };

        if outcome.success {
            let record = WebsiteStore::new_record(&resolved, WebsiteOrigin::Created, now_ms());
            store.insert(record.clone())?;
            store.save()?;
            report.website = Some(record);
            report.warnings.extend(conflicts.warnings);
        }
        self.record_history("create", &resolved.name, outcome.success, &outcome);
        Ok(report)
    }

    pub async fn update(&self, id: &str, input: &WebsiteInput) -> AppResult<SiteOperationReport> {
        let existing = self
            .store()?
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::from(servora_core::CoreError::NotFound(id.to_string())))?;
        let resolved = input.resolve()?;
        let mut store = self.store()?;

        let context = self.context_for(&resolved.project_path)?;
        let conflicts = check_conflicts(&resolved, &store, &context, Some(&existing.id));
        if conflicts.name_taken {
            return Err(AppError::from(servora_core::CoreError::Duplicate(
                resolved.name.clone(),
            )));
        }
        if conflicts.domain_taken {
            return Err(AppError::from(servora_core::CoreError::Duplicate(
                resolved.domain.to_string(),
            )));
        }

        let plan = update_plan(&existing, &resolved, &context)?;
        let outcome = self.run(&plan).await?;

        let mut report = SiteOperationReport {
            operation: "update".into(),
            website_id: Some(existing.id.clone()),
            website_name: Some(resolved.name.clone()),
            success: outcome.success,
            warnings: outcome.warnings.clone(),
            outcome: Some(outcome.clone()),
            website: None,
            message: outcome.failure_message(),
        };

        if outcome.success {
            let updated = Website {
                id: existing.id.clone(),
                name: resolved.name.clone(),
                domain: resolved.domain.to_string(),
                project_path: resolved.project_path.display().to_string(),
                document_root: resolved.document_root.display().to_string(),
                project_type: resolved.project_type,
                origin: existing.origin,
                created_at_ms: existing.created_at_ms,
                updated_at_ms: now_ms(),
            };
            store.replace(updated.clone())?;
            store.save()?;
            report.website = Some(updated);
        }
        self.record_history("update", &resolved.name, outcome.success, &outcome);
        Ok(report)
    }

    pub async fn rename(&self, id: &str, domain: &str) -> AppResult<SiteOperationReport> {
        let existing = self
            .store()?
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::from(servora_core::CoreError::NotFound(id.to_string())))?;
        let input = WebsiteInput {
            name: existing.name.clone(),
            domain: domain.to_string(),
            project_path: existing.project_path.clone(),
            document_root: Some(existing.document_root.clone()),
            project_type: Some(existing.project_type.as_str().to_string()),
        };
        let mut report = self.update(id, &input).await?;
        report.operation = "rename".into();
        Ok(report)
    }

    pub async fn remove(&self, id: &str) -> AppResult<SiteOperationReport> {
        let mut store = self.store()?;
        let website = store
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::from(servora_core::CoreError::NotFound(id.to_string())))?;

        let context = self.context(None)?;
        let plan = remove_plan(&website, &context)?;
        let outcome = self.run(&plan).await?;

        let mut report = SiteOperationReport {
            operation: "remove".into(),
            website_id: Some(website.id.clone()),
            website_name: Some(website.name.clone()),
            success: outcome.success,
            warnings: outcome.warnings.clone(),
            outcome: Some(outcome.clone()),
            website: None,
            message: outcome.failure_message(),
        };

        if outcome.success {
            store.remove(&website.id)?;
            store.save()?;
            report.warnings.push(
                "source files, database contents and databases were left untouched; the ACLs and SELinux rules Servora added were removed".to_string(),
            );
        }
        self.record_history("remove", &website.name, outcome.success, &outcome);
        Ok(report)
    }

    // ----------------------------------------------------------- migration

    pub fn preview_migration(&self) -> AppResult<MigrationScan> {
        let store = self.store()?;
        let hosts = self.hosts_text()?;
        Ok(migration::scan(&self.paths, &hosts, &store)?)
    }

    pub async fn import(&self, domains: &[String]) -> AppResult<ImportReport> {
        let scan = self.preview_migration()?;
        let mut report = ImportReport {
            imported: Vec::new(),
            skipped: Vec::new(),
            failed: Vec::new(),
        };

        for domain in domains {
            let Some(candidate) = scan
                .candidates
                .iter()
                .find(|candidate| candidate.domain.eq_ignore_ascii_case(domain))
            else {
                report
                    .skipped
                    .push(format!("{domain}: not found or not importable"));
                continue;
            };
            if !candidate.importable {
                report.skipped.push(format!(
                    "{}: {}",
                    candidate.domain,
                    candidate.warnings.join("; ")
                ));
                continue;
            }

            match self.import_one(candidate).await {
                Ok(operation) if operation.success => report.imported.push(operation),
                Ok(operation) => report.failed.push(operation),
                Err(error) => report
                    .skipped
                    .push(format!("{}: {}", candidate.domain, error)),
            }
        }

        Ok(report)
    }

    async fn import_one(&self, candidate: &MigrationCandidate) -> AppResult<SiteOperationReport> {
        let mut store = self.store()?;

        // Resolve a free name for the record.
        let base = candidate.name.clone();
        let mut name = base.clone();
        let mut counter = 2;
        while store.name_taken(&name, None) {
            name = format!("{base}-{counter}");
            counter += 1;
        }

        let project_path = Path::new(&candidate.project_path);
        let context = self.context_for(project_path)?;
        let existing =
            servora_core::storage::read_to_string_opt(Path::new(&candidate.config_path))?;
        let mut context = context;
        context.existing_config = existing;

        let plan = migration::import_plan(candidate, &name, &context)?;
        let outcome = self.run(&plan).await?;

        let mut report = SiteOperationReport {
            operation: "import".into(),
            website_id: Some(servora_core::website::slug(&name)),
            website_name: Some(name.clone()),
            success: outcome.success,
            warnings: outcome.warnings.clone(),
            outcome: Some(outcome.clone()),
            website: None,
            message: outcome.failure_message(),
        };

        if outcome.success {
            let resolved = ResolvedWebsite {
                id: servora_core::website::slug(&name),
                name: name.clone(),
                domain: servora_core::domain::DomainName::parse(&candidate.domain)?,
                project_path: std::path::PathBuf::from(&candidate.project_path),
                document_root: std::path::PathBuf::from(&candidate.document_root),
                project_type: candidate.project_type,
            };
            let record =
                WebsiteStore::new_record(&resolved, WebsiteOrigin::ImportedSiteForge, now_ms());
            store.insert(record.clone())?;
            store.save()?;

            let mut log = MigrationLog::load(&self.paths.migrations_file())?;
            log.record(MigrationRecord {
                imported_at_ms: now_ms(),
                name: name.clone(),
                domain: candidate.domain.clone(),
                source: candidate.owner.clone(),
                config_path: candidate.config_path.clone(),
            });
            log.save(&self.paths.migrations_file())?;
            report.website = Some(record);
        }
        self.record_history("import", &name, outcome.success, &outcome);
        Ok(report)
    }

    // --------------------------------------------------------- diagnostics

    pub fn diagnostics(&self) -> AppResult<DiagnosticsReport> {
        Ok(diagnostics::report(
            &servora_core::plan::LocalSystem,
            &self.paths,
        ))
    }

    pub fn environment(&self) -> AppResult<EnvironmentReport> {
        Ok(diagnostics::environment(
            &servora_core::plan::LocalSystem,
            &self.paths,
        ))
    }

    /// `apachectl configtest` without touching any file.
    pub fn validate_apache(&self) -> AppResult<serde_json::Value> {
        use servora_core::plan::SystemOps;
        let output = servora_core::plan::LocalSystem.run("apachectl", &["configtest".into()])?;
        Ok(serde_json::json!({
            "valid": output.success,
            "output": if output.stderr.trim().is_empty() { output.stdout } else { output.stderr },
        }))
    }

    /// Reload Apache through the helper (needs authorization).
    pub async fn reload_apache(&self) -> AppResult<PlanOutcome> {
        let plan = Plan::new("reload-apache", None)
            .push(servora_core::plan::Step::new(
                "configtest",
                "validate the Apache configuration",
                servora_core::plan::StepAction::ApacheConfigTest,
            ))
            .push(servora_core::plan::Step::new(
                "reload",
                "reload Apache",
                servora_core::plan::StepAction::ReloadHttpd,
            ));
        self.run(&plan).await
    }

    // ------------------------------------------------------------- helpers

    async fn run(&self, plan: &Plan) -> AppResult<PlanOutcome> {
        plan.validate(&self.paths)?;
        let outcome = privileged::apply(plan, &self.paths.helper_path).await?;
        if !outcome.success {
            logging::warn(&format!(
                "{} failed: {}",
                plan.operation,
                outcome.failure_message().unwrap_or_default()
            ));
        }
        Ok(outcome)
    }

    fn record_history(&self, operation: &str, website: &str, success: bool, outcome: &PlanOutcome) {
        let mut entries = self.history();
        entries.push(HistoryEntry {
            at_ms: now_ms(),
            operation: operation.to_string(),
            website: website.to_string(),
            success,
            summary: if success {
                format!("{} steps completed", outcome.steps.len())
            } else {
                outcome.failure_message().unwrap_or_default()
            },
        });
        if entries.len() > 200 {
            let start = entries.len() - 200;
            entries.drain(0..start);
        }
        let path = self.paths.history_file();
        if let Ok(json) = serde_json::to_string_pretty(&entries) {
            let _ = servora_core::storage::write_atomic(&path, &json);
        }
    }

    /// Operation history, newest first.
    pub fn history(&self) -> Vec<HistoryEntry> {
        let path = self.paths.history_file();
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let mut entries: Vec<HistoryEntry> = serde_json::from_str(&text).unwrap_or_default();
        entries.sort_by_key(|entry| entry.at_ms);
        entries.reverse();
        entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn service(tag: &str) -> SiteService {
        let root = std::env::temp_dir().join(format!("servora-sites-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("etc/httpd/conf.d")).unwrap();
        fs::write(root.join("etc/hosts"), "127.0.0.1 localhost\n").unwrap();
        let project = root.join("Work/e-perpus");
        fs::create_dir_all(project.join("public")).unwrap();
        fs::create_dir_all(project.join("storage")).unwrap();
        fs::write(project.join("artisan"), "x").unwrap();
        fs::write(project.join("public/index.php"), "<?php").unwrap();
        SiteService::with_paths(Paths::default().with_root(&root))
    }

    fn input(service: &SiteService) -> WebsiteInput {
        let project = service
            .paths()
            .data_dir
            .parent()
            .unwrap()
            .join("Work/e-perpus");
        WebsiteInput {
            name: "e-perpus".into(),
            domain: "e-perpus.test".into(),
            project_path: project.display().to_string(),
            document_root: None,
            project_type: None,
        }
    }

    #[test]
    fn empty_store_starts_with_no_websites() {
        let service = service("empty");
        assert!(service.views().unwrap().is_empty());
        assert!(service.history().is_empty());
    }

    #[test]
    fn projects_are_inspected_and_conflicts_detected() {
        let service = service("inspect");
        let inspection = service
            .inspect_project(Path::new(&input(&service).project_path))
            .unwrap();
        assert_eq!(
            inspection.project_type,
            servora_core::project::ProjectType::Laravel
        );
        assert!(inspection.document_root.unwrap().ends_with("public"));

        let conflicts = service.conflicts(&input(&service), None).unwrap();
        assert!(!conflicts.blocking);
        assert!(!conflicts.config_exists);
    }

    #[test]
    fn domain_validation_reports_specifics() {
        let service = service("domain");
        let valid = service.validate_domain("e-perpus.test").unwrap();
        assert_eq!(valid["valid"], true);
        assert_eq!(valid["isLocalDevelopment"], true);

        let invalid = service.validate_domain("not a domain").unwrap();
        assert_eq!(invalid["valid"], false);
        assert!(invalid["message"].as_str().unwrap().contains("valid"));
    }

    #[test]
    fn managed_domains_reflect_the_hosts_block() {
        let service = service("domains");
        let hosts = HostsFile::parse(&service.hosts_text().unwrap()).with_domain(
            "e-perpus.test",
            "e-perpus",
            servora_core::hosts::DEFAULT_LOCAL_IP,
        );
        fs::write(service.paths().hosts_file.clone(), hosts).unwrap();

        let payload = service.managed_domains().unwrap();
        let domains = payload["domains"].as_array().unwrap();
        assert_eq!(domains.len(), 1);
        assert_eq!(domains[0]["domain"], "e-perpus.test");
    }

    #[test]
    fn diagnostics_and_environment_run_without_side_effects() {
        let service = service("diagnostics");
        let report = service.diagnostics().unwrap();
        assert!(!report.checks.is_empty());
        let environment = service.environment().unwrap();
        assert!(environment
            .components
            .iter()
            .any(|component| component.id == "apache-installed"));
    }

    #[test]
    fn missing_helper_fails_website_creation_with_a_typed_error() {
        let service = service("nohelper");
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let error = runtime
            .block_on(service.create(&input(&service)))
            .unwrap_err();
        assert_eq!(error.code(), crate::error::codes::HELPER_MISSING);
        // Nothing was written and no record was stored.
        assert!(service.views().unwrap().is_empty());
        assert!(!service
            .paths()
            .apache_conf_dir
            .join("e-perpus.test.conf")
            .exists());
    }

    #[test]
    fn website_view_serialises_the_shape_the_frontend_expects() {
        let service = service("contract");
        let mut store = service.store().unwrap();
        let resolved = ResolvedWebsite {
            id: "e-perpus".into(),
            name: "e-perpus".into(),
            domain: servora_core::domain::DomainName::parse("e-perpus.test").unwrap(),
            project_path: std::path::PathBuf::from(&input(&service).project_path),
            document_root: std::path::PathBuf::from(&input(&service).project_path).join("public"),
            project_type: servora_core::project::ProjectType::Laravel,
        };
        let record = WebsiteStore::new_record(&resolved, WebsiteOrigin::Created, now_ms());
        store.insert(record).unwrap();
        store.save().unwrap();

        let views = service.views().unwrap();
        let json = serde_json::to_value(&views[0]).unwrap();

        for key in [
            "id",
            "name",
            "domain",
            "projectPath",
            "documentRoot",
            "projectType",
            "origin",
            "createdAtMs",
            "updatedAtMs",
            "url",
            "state",
            "status",
        ] {
            assert!(json.get(key).is_some(), "missing key {key} in {json}");
        }
        assert_eq!(json["url"], "http://e-perpus.test");
        assert_eq!(json["state"], "config-missing");
        assert_eq!(json["status"]["hostsState"]["state"], "absent");
        assert_eq!(json["status"]["configExists"], false);
        assert!(json["status"]["warnings"].as_array().unwrap().len() >= 2);
    }

    #[test]
    fn import_report_serialises_expected_keys() {
        let report = ImportReport {
            imported: Vec::new(),
            skipped: vec!["a.test: not found".into()],
            failed: Vec::new(),
        };
        let json = serde_json::to_value(&report).unwrap();
        assert!(json.get("imported").is_some());
        assert!(json.get("skipped").is_some());
        assert!(json.get("failed").is_some());
    }

    #[test]
    fn preview_reports_importable_siteforge_sites() {
        let service = service("preview");
        let project = service
            .paths()
            .data_dir
            .parent()
            .unwrap()
            .join("Work/e-perpus");
        fs::write(
            service.paths().apache_conf_dir.join("e-perpus.test.conf"),
            format!(
                "# Managed by siteForge\n<VirtualHost *:80>\n    ServerName e-perpus.test\n    DocumentRoot \"{}/public\"\n</VirtualHost>\n",
                project.display()
            ),
        )
        .unwrap();
        fs::write(
            service.paths().hosts_file.clone(),
            "127.0.0.1 localhost\n127.0.0.1 e-perpus.test\n",
        )
        .unwrap();

        let scan = service.preview_migration().unwrap();
        assert_eq!(scan.candidates.len(), 1);
        let candidate = &scan.candidates[0];
        assert_eq!(candidate.name, "e-perpus");
        assert!(candidate.importable);
        assert_eq!(candidate.owner, "managed by siteForge");
    }
}
