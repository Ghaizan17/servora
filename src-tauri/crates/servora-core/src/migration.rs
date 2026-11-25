//! Detecting and importing websites that were created by siteForge (or by hand).
//!
//! Detection is read-only. Servora parses the Apache configuration files and
//! `/etc/hosts`; it never executes siteForge and never needs it installed. The
//! import itself reuses the normal plan machinery, so every imported site gets a
//! backup of its original configuration.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::domain::DomainName;
use crate::errors::CoreResult;
use crate::hosts::{HostsFile, HostsState, DEFAULT_LOCAL_IP};
use crate::operations::SiteContext;
use crate::plan::{Plan, Step, StepAction};
use crate::project::{detect_document_root, detect_project_type, ProjectType};
use crate::storage::{now_ms, Paths};
use crate::virtualhost::{self, Owner, VHostSpec};
use crate::website::{slug, validate_name, WebsiteStore};

/// A configuration file that could be imported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationCandidate {
    pub name: String,
    pub domain: String,
    pub project_path: String,
    pub document_root: String,
    pub project_type: ProjectType,
    pub config_path: String,
    pub owner: String,
    pub importable: bool,
    pub already_imported: bool,
    pub hosts_state: HostsState,
    pub hosts_occurrences: usize,
    pub warnings: Vec<String>,
}

impl MigrationCandidate {
    pub fn id(&self) -> String {
        slug(&self.name)
    }
}

/// Result of scanning the system for importable sites.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationScan {
    pub scanned_at_ms: i64,
    pub conf_dir: String,
    pub siteforge_installed: bool,
    pub candidates: Vec<MigrationCandidate>,
    pub ignored: Vec<String>,
}

impl MigrationScan {
    pub fn importable(&self) -> Vec<&MigrationCandidate> {
        self.candidates
            .iter()
            .filter(|candidate| candidate.importable)
            .collect()
    }
}

fn name_from(domain: &str, document_root: &Path) -> String {
    // siteForge never stored a name, so prefer the project directory; generic
    // web roots fall back to the first domain label.
    const GENERIC: [&str; 8] = [
        "srv",
        "var",
        "www",
        "html",
        "htdocs",
        "public_html",
        "data",
        "public",
    ];
    let project = if document_root.file_name().and_then(|name| name.to_str()) == Some("public") {
        document_root.parent().unwrap_or(document_root)
    } else {
        document_root
    };

    if let Some(candidate) = project
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !GENERIC.contains(name))
    {
        if validate_name(candidate).is_ok() {
            return candidate.to_string();
        }
    }

    let label = domain.split('.').next().unwrap_or("site");
    if validate_name(label).is_ok() {
        return label.to_string();
    }
    slug(domain).replace('.', "-")
}

/// Scan every configuration file in the Apache directory.
pub fn scan(paths: &Paths, hosts_text: &str, store: &WebsiteStore) -> CoreResult<MigrationScan> {
    let hosts = HostsFile::parse(hosts_text);
    let mut candidates = Vec::new();
    let mut ignored = Vec::new();

    let entries = match std::fs::read_dir(&paths.apache_conf_dir) {
        Ok(entries) => entries,
        Err(_) => {
            return Ok(MigrationScan {
                scanned_at_ms: now_ms(),
                conf_dir: paths.apache_conf_dir.display().to_string(),
                siteforge_installed: siteforge_installed(),
                candidates,
                ignored,
            })
        }
    };

    let mut files: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("conf"))
        .collect();
    files.sort();

    for file in files {
        let file_name = file
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("site.conf")
            .to_string();
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(_) => {
                ignored.push(format!("{file_name} (unreadable)"));
                continue;
            }
        };
        let parsed = virtualhost::parse(&file_name, &text);

        // Only siteForge files and marker-less .test files are candidates.
        if !matches!(parsed.owner, Owner::SiteForge | Owner::LegacyTest) {
            ignored.push(format!("{file_name} ({})", parsed.owner.label()));
            continue;
        }

        let Some(domain_text) = parsed.primary_domain().map(str::to_string) else {
            ignored.push(format!("{file_name} (no ServerName)"));
            continue;
        };
        let domain = match DomainName::parse(&domain_text) {
            Ok(domain) => domain,
            Err(_) => {
                ignored.push(format!("{file_name} (invalid ServerName `{domain_text}`)"));
                continue;
            }
        };

        let Some(document_root) = parsed.primary_document_root().map(PathBuf::from) else {
            ignored.push(format!("{file_name} (no DocumentRoot)"));
            continue;
        };

        let project_path =
            if document_root.file_name().and_then(|name| name.to_str()) == Some("public") {
                document_root
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| document_root.clone())
            } else {
                document_root.clone()
            };

        let mut warnings = Vec::new();
        let name = name_from(domain.as_str(), &document_root);

        if !project_path.is_dir() {
            warnings.push(format!(
                "project directory {} does not exist",
                project_path.display()
            ));
        }
        if !document_root.is_dir() {
            warnings.push(format!(
                "document root {} does not exist",
                document_root.display()
            ));
        }
        let project_type = if project_path.is_dir() {
            detect_project_type(&project_path)
        } else {
            ProjectType::Unknown
        };
        if project_type == ProjectType::Unknown {
            warnings.push("project type could not be detected".to_string());
        }

        let detected_root = detect_document_root(&project_path);
        if let (Some(detected), true) = (&detected_root, project_path.is_dir()) {
            if detected != &document_root {
                warnings.push(format!(
                    "the tracked document root is {} but {} looks like the real one",
                    document_root.display(),
                    detected.display()
                ));
            }
        }

        let hosts_state = hosts.state(domain.as_str());
        let hosts_occurrences = hosts.entries_for(domain.as_str()).len();
        match &hosts_state {
            HostsState::Duplicate(count) => warnings.push(format!(
                "/etc/hosts lists {domain} {count} times (left over from an old siteForge rename)"
            )),
            HostsState::Absent => warnings.push(format!("/etc/hosts has no entry for {domain}")),
            _ => {}
        }

        let already_imported = store.by_domain(domain.as_str()).is_some();
        let name_conflict = store.name_taken(&name, None);
        if already_imported {
            warnings.push("already managed by Servora".to_string());
        } else if name_conflict {
            warnings.push(
                "another Servora website already uses this name; it will be imported under a suffix"
                    .to_string(),
            );
        }

        if parsed.owner == Owner::LegacyTest {
            warnings.push(
                "this file has no ownership marker; importing replaces it with a Servora-managed file (a backup is kept)"
                    .to_string(),
            );
        }

        candidates.push(MigrationCandidate {
            name,
            domain: domain.as_str().to_string(),
            project_path: project_path.display().to_string(),
            document_root: document_root.display().to_string(),
            project_type,
            config_path: file.display().to_string(),
            owner: parsed.owner.label().to_string(),
            importable: project_path.is_dir() && !already_imported,
            already_imported,
            hosts_state,
            hosts_occurrences,
            warnings,
        });
    }

    Ok(MigrationScan {
        scanned_at_ms: now_ms(),
        conf_dir: paths.apache_conf_dir.display().to_string(),
        siteforge_installed: siteforge_installed(),
        candidates,
        ignored,
    })
}

/// Whether the legacy CLI happens to exist. Informational only — Servora never
/// runs it and never requires it.
pub fn siteforge_installed() -> bool {
    ["/usr/local/bin/siteForge", "/usr/bin/siteForge"]
        .iter()
        .any(|path| Path::new(path).exists())
}

/// Plan that adopts a candidate into Servora.
pub fn import_plan(
    candidate: &MigrationCandidate,
    resolved_name: &str,
    ctx: &SiteContext,
) -> CoreResult<Plan> {
    let domain = DomainName::parse(&candidate.domain)?;
    let project_path = PathBuf::from(&candidate.project_path);
    let document_root = PathBuf::from(&candidate.document_root);
    if !project_path.is_dir() {
        return Err(crate::errors::CoreError::InvalidPath(format!(
            "{} is not available",
            project_path.display()
        )));
    }

    let spec = VHostSpec {
        website: resolved_name.to_string(),
        domain: domain.clone(),
        project_path: project_path.clone(),
        document_root: document_root.clone(),
        project_type: candidate.project_type,
        php_fpm_socket: ctx.php_fpm_socket.clone(),
    };
    let rendered = virtualhost::render(&spec);
    virtualhost::validate(&rendered, &domain)?;

    let hosts = HostsFile::parse(&ctx.hosts_text).with_domain(
        domain.as_str(),
        &slug(resolved_name),
        DEFAULT_LOCAL_IP,
    );

    Ok(Plan::new("import", Some(slug(resolved_name)))
        .push(Step::new(
            "write-config",
            format!("adopt {}", candidate.config_path),
            StepAction::WriteFile {
                path: candidate.config_path.clone(),
                contents: rendered,
                mode: 0o644,
                label: "import".into(),
            },
        ))
        .push(Step::new(
            "write-hosts",
            format!("register {} in the Servora block", domain),
            StepAction::WriteFile {
                path: ctx.paths.hosts_file.display().to_string(),
                contents: hosts,
                mode: 0o644,
                label: "hosts".into(),
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

/// One import that has been performed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationRecord {
    pub imported_at_ms: i64,
    pub name: String,
    pub domain: String,
    pub source: String,
    pub config_path: String,
}

/// Persisted migration history (proves where a site came from).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationLog {
    pub version: u32,
    #[serde(default)]
    pub records: Vec<MigrationRecord>,
}

impl Default for MigrationLog {
    fn default() -> Self {
        Self {
            version: 1,
            records: Vec::new(),
        }
    }
}

impl MigrationLog {
    pub fn load(path: &Path) -> CoreResult<Self> {
        match crate::storage::read_to_string_opt(path)? {
            None => Ok(Self::default()),
            Some(raw) => serde_json::from_str(&raw).map_err(|err| {
                crate::errors::CoreError::Storage(format!("{}: {err}", path.display()))
            }),
        }
    }

    pub fn save(&self, path: &Path) -> CoreResult<()> {
        crate::storage::write_atomic(path, &serde_json::to_string_pretty(self)?)
    }

    pub fn record(&mut self, record: MigrationRecord) {
        self.records.push(record);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selinux::SelinuxMode;
    use std::fs;

    fn setup(tag: &str) -> Paths {
        let dir = std::env::temp_dir().join(format!("servora-mig-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("etc/httpd/conf.d")).unwrap();
        fs::create_dir_all(dir.join("Work/e-perpus/public")).unwrap();
        fs::write(dir.join("Work/e-perpus/artisan"), "x").unwrap();
        fs::write(dir.join("Work/e-perpus/public/index.php"), "<?php").unwrap();
        fs::write(
            dir.join("etc/hosts"),
            "127.0.0.1 localhost\n127.0.0.1 e-perpus.test\n127.0.0.1 e-perpus.test\n",
        )
        .unwrap();
        fs::write(
            dir.join("etc/httpd/conf.d/e-perpus.test.conf"),
            "# Managed by siteForge\n<VirtualHost *:80>\n    ServerName e-perpus.test\n\n    DocumentRoot \"PUT_ROOT/Work/e-perpus/public\"\n\n    <Directory \"PUT_ROOT/Work/e-perpus/public\">\n        AllowOverride All\n        Require all granted\n    </Directory>\n</VirtualHost>\n"
                .replace("PUT_ROOT", dir.to_str().unwrap()),
        )
        .unwrap();
        fs::write(
            dir.join("etc/httpd/conf.d/other.test.conf"),
            format!(
                "<VirtualHost *:80>\n    ServerName other.test\n    DocumentRoot \"{}/Work/e-perpus/public\"\n</VirtualHost>\n",
                dir.to_str().unwrap()
            ),
        )
        .unwrap();
        fs::write(
            dir.join("etc/httpd/conf.d/welcome.conf"),
            "<VirtualHost *:80>\nServerName localhost\n</VirtualHost>\n",
        )
        .unwrap();
        Paths::default().with_root(&dir)
    }

    #[test]
    fn detects_siteforge_and_legacy_configurations() {
        let paths = setup("detect");
        let store = WebsiteStore::load(&paths.websites_file()).unwrap();
        let hosts = fs::read_to_string(&paths.hosts_file).unwrap();
        let scan = scan(&paths, &hosts, &store).unwrap();

        assert_eq!(scan.candidates.len(), 2, "siteForge + legacy .test file");
        let siteforge = scan
            .candidates
            .iter()
            .find(|candidate| candidate.domain == "e-perpus.test")
            .unwrap();
        assert_eq!(siteforge.owner, "managed by siteForge");
        assert!(siteforge.importable);
        assert_eq!(siteforge.project_type, ProjectType::Laravel);
        assert_eq!(siteforge.hosts_occurrences, 2);
        assert!(siteforge.warnings.iter().any(|w| w.contains("2 times")));

        let legacy = scan
            .candidates
            .iter()
            .find(|candidate| candidate.domain == "other.test")
            .unwrap();
        assert_eq!(legacy.owner, "legacy .test file");
        assert!(legacy
            .warnings
            .iter()
            .any(|w| w.contains("no ownership marker")));

        // Unrelated configuration is never a candidate.
        assert!(scan
            .ignored
            .iter()
            .any(|line| line.starts_with("welcome.conf")));
        fs::remove_dir_all(paths.data_dir.parent().unwrap()).ok();
    }

    #[test]
    fn import_plan_preserves_the_original_through_backups() {
        let paths = setup("import");
        let store = WebsiteStore::load(&paths.websites_file()).unwrap();
        let hosts = fs::read_to_string(&paths.hosts_file).unwrap();
        let scan = scan(&paths, &hosts, &store).unwrap();
        let candidate = scan
            .candidates
            .iter()
            .find(|candidate| candidate.domain == "e-perpus.test")
            .unwrap()
            .clone();

        let ctx = SiteContext {
            paths: paths.clone(),
            apache_user: "apache".into(),
            home: paths.data_dir.parent().unwrap().to_path_buf(),
            php_fpm_socket: None,
            selinux: SelinuxMode::Disabled,
            hosts_text: hosts.clone(),
            existing_config: Some(fs::read_to_string(&candidate.config_path).unwrap()),
            writable_dirs: vec!["storage".into()],
        };

        let plan = import_plan(&candidate, "e-perpus", &ctx).unwrap();
        assert_eq!(plan.operation, "import");
        assert!(plan.backup_targets().contains(&candidate.config_path));

        match &plan.steps[0].action {
            StepAction::WriteFile { contents, .. } => {
                assert!(contents.starts_with(crate::virtualhost::SERVORA_MARKER));
            }
            other => panic!("unexpected {other:?}"),
        }
        fs::remove_dir_all(paths.data_dir.parent().unwrap()).ok();
    }

    #[test]
    fn importing_twice_is_not_offered_again() {
        let paths = setup("twice");
        let mut store = WebsiteStore::load(&paths.websites_file()).unwrap();
        store
            .insert(crate::website::Website {
                id: "e-perpus".into(),
                name: "e-perpus".into(),
                domain: "e-perpus.test".into(),
                project_path: paths
                    .data_dir
                    .parent()
                    .unwrap()
                    .join("Work/e-perpus")
                    .display()
                    .to_string(),
                document_root: paths
                    .data_dir
                    .parent()
                    .unwrap()
                    .join("Work/e-perpus/public")
                    .display()
                    .to_string(),
                project_type: ProjectType::Laravel,
                origin: Some(crate::website::WebsiteOrigin::ImportedSiteForge),
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();

        let hosts = fs::read_to_string(&paths.hosts_file).unwrap();
        let scan = scan(&paths, &hosts, &store).unwrap();
        let candidate = scan
            .candidates
            .iter()
            .find(|candidate| candidate.domain == "e-perpus.test")
            .unwrap();
        assert!(candidate.already_imported);
        assert!(!candidate.importable);
        fs::remove_dir_all(paths.data_dir.parent().unwrap()).ok();
    }

    #[test]
    fn servora_configurations_are_not_candidates() {
        let paths = setup("servora-owned");
        fs::write(
            paths.apache_conf_dir.join("mine.test.conf"),
            format!(
                "{}\n<VirtualHost *:80>\n    ServerName mine.test\n    DocumentRoot \"{}/Work/e-perpus\"\n</VirtualHost>\n",
                crate::virtualhost::SERVORA_MARKER,
                paths.data_dir.parent().unwrap().to_str().unwrap()
            ),
        )
        .unwrap();
        let store = WebsiteStore::load(&paths.websites_file()).unwrap();
        let hosts = fs::read_to_string(&paths.hosts_file).unwrap();
        let scan = scan(&paths, &hosts, &store).unwrap();
        assert!(!scan.candidates.iter().any(|c| c.domain == "mine.test"));
        fs::remove_dir_all(paths.data_dir.parent().unwrap()).ok();
    }

    #[test]
    fn names_are_derived_from_the_project_directory() {
        assert_eq!(
            name_from(
                "e-perpus-skenda.com",
                Path::new("/home/u/Work/e-perpus/public")
            ),
            "e-perpus"
        );
        assert_eq!(
            name_from("shop.test", Path::new("/home/u/Work/red_shop")),
            "red_shop"
        );
        assert_eq!(name_from("shop.test", Path::new("/srv")), "shop");
    }

    #[test]
    fn migration_log_persists_history() {
        let dir = std::env::temp_dir().join(format!("servora-miglog-{}", std::process::id()));
        let path = dir.join("migrations.json");
        let mut log = MigrationLog::default();
        log.record(MigrationRecord {
            imported_at_ms: 1,
            name: "e-perpus".into(),
            domain: "e-perpus.test".into(),
            source: "siteforge".into(),
            config_path: "/etc/httpd/conf.d/e-perpus.test.conf".into(),
        });
        log.save(&path).unwrap();
        let loaded = MigrationLog::load(&path).unwrap();
        assert_eq!(loaded.records.len(), 1);
        assert_eq!(loaded.records[0].domain, "e-perpus.test");
        fs::remove_dir_all(&dir).ok();
    }
}
