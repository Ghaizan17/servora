//! Recover websites from the live Apache configuration.
//!
//! `websites.json` is a cache of intent, not the source of truth. When it is
//! missing, moved or emptied (a fresh profile, a restored home directory, a
//! manual `rm -rf ~/.local/share/servora`) the sites themselves are still on
//! disk: a `# Managed by Servora` VirtualHost with the website name, project path
//! and type written into its header.
//!
//! This module rebuilds `Website` records from those files so the Websites page
//! lists what Apache actually serves instead of an empty list while the Domain
//! manager still shows the domains.

use std::path::Path;

use crate::domain::DomainName;
use crate::errors::CoreResult;
use crate::project::ProjectType;
use crate::storage::{now_ms, Paths};
use crate::virtualhost::{self, Owner};
use crate::website::{slug, validate_name, Website, WebsiteOrigin};

/// A website rebuilt from a Servora-managed VirtualHost.
#[derive(Debug, Clone)]
pub struct RecoveredSite {
    pub website: Website,
    pub config_path: String,
}

/// Read the header comments Servora writes into every configuration it owns.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ServletMarkers {
    pub website: Option<String>,
    pub project: Option<String>,
    pub project_type: Option<String>,
}

impl ServletMarkers {
    pub fn parse(text: &str) -> Self {
        let mut markers = ServletMarkers::default();
        for line in text.lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix('#') else {
                continue;
            };
            let rest = rest.trim();
            if let Some(value) = rest.strip_prefix("website:") {
                let value = value.trim();
                if !value.is_empty() {
                    markers.website = Some(value.to_string());
                }
            } else if let Some(value) = rest.strip_prefix("project:") {
                let value = value.trim();
                if !value.is_empty() {
                    markers.project = Some(value.to_string());
                }
            } else if let Some(value) = rest.strip_prefix("type:") {
                let value = value.trim();
                if !value.is_empty() {
                    markers.project_type = Some(value.to_string());
                }
            }
        }
        markers
    }
}

/// Rebuild the websites Servora owns from `/etc/httpd/conf.d`.
///
/// Files without the Servora marker are ignored: they belong to another tool and
/// are offered through the import flow instead.
pub fn recover_sites(paths: &Paths) -> CoreResult<Vec<RecoveredSite>> {
    let dir = &paths.apache_conf_dir;
    if !dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut entries: Vec<_> = std::fs::read_dir(dir)?.filter_map(Result::ok).collect();
    entries.sort_by_key(|entry| entry.file_name());

    let mut recovered = Vec::new();
    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("conf") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("site.conf")
            .to_string();
        if virtualhost::classify(&file_name, &text) != Owner::Servora {
            continue;
        }

        let parsed = virtualhost::parse(&file_name, &text);
        let Some(domain) = parsed.primary_domain().map(str::to_string) else {
            continue;
        };
        let Ok(domain_name) = DomainName::parse(&domain) else {
            continue;
        };

        let markers = ServletMarkers::parse(&text);
        let name = markers
            .website
            .filter(|value| validate_name(value).is_ok())
            .unwrap_or_else(|| slug(domain_name.as_str()));

        let document_root = parsed
            .primary_document_root()
            .map(str::to_string)
            .or_else(|| markers.project.clone());
        let project_path = markers
            .project
            .clone()
            .or_else(|| document_root.clone());
        let Some(project_path) = project_path else {
            // Without a project path there is nothing safe to manage: the file is
            // listed as external through the import flow instead.
            continue;
        };

        let project_type = markers
            .project_type
            .as_deref()
            .and_then(ProjectType::from_str_opt)
            .unwrap_or_else(|| {
                let path = Path::new(&project_path);
                if path.is_dir() {
                    crate::project::detect_project_type(path)
                } else {
                    ProjectType::Unknown
                }
            });

        let updated_at_ms = entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|modified| {
                modified
                    .duration_since(std::time::UNIX_EPOCH)
                    .ok()
                    .map(|elapsed| elapsed.as_millis() as i64)
            })
            .unwrap_or_else(now_ms);

        recovered.push(RecoveredSite {
            website: Website {
                id: slug(&name),
                name,
                domain: domain_name.as_str().to_string(),
                project_path,
                document_root: document_root.unwrap_or_default(),
                project_type,
                origin: Some(WebsiteOrigin::Recovered),
                created_at_ms: updated_at_ms,
                updated_at_ms,
            },
            config_path: path.display().to_string(),
        });
    }

    Ok(recovered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::virtualhost::{render, VHostSpec};
    use std::fs;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "servora-recover-{}-{tag}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn paths_with(root: &Path) -> Paths {
        Paths::system().with_root(root)
    }

    fn write_vhost(paths: &Paths, name: &str, domain: &str, project: &str, root: &str) {
        let spec = VHostSpec {
            website: name.to_string(),
            domain: DomainName::parse(domain).unwrap(),
            project_path: project.into(),
            document_root: root.into(),
            project_type: ProjectType::Laravel,
            php_fpm_socket: None,
        };
        fs::create_dir_all(&paths.apache_conf_dir).unwrap();
        fs::write(
            paths.apache_conf_dir.join(format!("{domain}.conf")),
            render(&spec),
        )
        .unwrap();
    }

    #[test]
    fn reads_the_header_markers() {
        let text = "# Managed by Servora\n# website: e-perpus\n# project: /home/u/e-perpus\n# type: laravel\n";
        let markers = ServletMarkers::parse(text);
        assert_eq!(markers.website.as_deref(), Some("e-perpus"));
        assert_eq!(markers.project.as_deref(), Some("/home/u/e-perpus"));
        assert_eq!(markers.project_type.as_deref(), Some("laravel"));
    }

    #[test]
    fn recovers_a_site_from_its_configuration() {
        let root = scratch("site");
        let paths = paths_with(&root);
        write_vhost(
            &paths,
            "e-perpus",
            "e-perpus-skenda.com",
            "/home/u/e-perpus",
            "/home/u/e-perpus/public",
        );

        let recovered = recover_sites(&paths).unwrap();
        assert_eq!(recovered.len(), 1);
        let site = &recovered[0].website;
        assert_eq!(site.name, "e-perpus");
        assert_eq!(site.domain, "e-perpus-skenda.com");
        assert_eq!(site.project_path, "/home/u/e-perpus");
        assert_eq!(site.document_root, "/home/u/e-perpus/public");
        assert_eq!(site.project_type, ProjectType::Laravel);
        assert_eq!(site.origin, Some(WebsiteOrigin::Recovered));
    }

    #[test]
    fn ignores_configuration_owned_by_another_tool() {
        let root = scratch("foreign");
        let paths = paths_with(&root);
        fs::create_dir_all(&paths.apache_conf_dir).unwrap();
        fs::write(
            paths.apache_conf_dir.join("legacy.test.conf"),
            "# Managed by siteForge\n<VirtualHost *:80>\n  ServerName legacy.test\n  DocumentRoot \"/srv/legacy\"\n</VirtualHost>\n",
        )
        .unwrap();

        assert!(recover_sites(&paths).unwrap().is_empty());
    }

    #[test]
    fn a_missing_conf_directory_is_not_an_error() {
        let root = scratch("missing");
        let paths = paths_with(&root);
        assert!(recover_sites(&paths).unwrap().is_empty());
    }
}
