//! Website records and their metadata store.
//!
//! Metadata lives in `$XDG_DATA_HOME/servora/websites.json` and is *not* the
//! only source of truth: the Apache configuration is always re-read and
//! reconciled when the list is loaded (see [`crate::reconcile`]).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::domain::DomainName;
use crate::errors::{CoreError, CoreResult};
use crate::project::ProjectType;
use crate::storage::{now_ms, write_atomic};

/// Where a record came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WebsiteOrigin {
    Created,
    ImportedSiteForge,
    ImportedLegacy,
    /// Rebuilt from a `# Managed by Servora` VirtualHost when the stored record
    /// was missing.
    Recovered,
}

impl WebsiteOrigin {
    pub fn label(&self) -> &'static str {
        match self {
            WebsiteOrigin::Created => "created in Servora",
            WebsiteOrigin::ImportedSiteForge => "imported from siteForge",
            WebsiteOrigin::ImportedLegacy => "imported from a legacy VirtualHost",
            WebsiteOrigin::Recovered => "recovered from the live configuration",
        }
    }
}

/// One Servora-managed website.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Website {
    pub id: String,
    pub name: String,
    pub domain: String,
    pub project_path: String,
    pub document_root: String,
    pub project_type: ProjectType,
    #[serde(default)]
    pub origin: Option<WebsiteOrigin>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl Website {
    pub fn domain_name(&self) -> CoreResult<DomainName> {
        DomainName::parse(&self.domain)
    }

    pub fn project(&self) -> PathBuf {
        PathBuf::from(&self.project_path)
    }

    pub fn document_root_path(&self) -> PathBuf {
        PathBuf::from(&self.document_root)
    }

    /// `http://domain` for the UI.
    pub fn url(&self) -> String {
        format!("http://{}", self.domain)
    }

    /// Configuration file name Servora generates for this website.
    pub fn config_file_name(&self) -> String {
        format!("{}.conf", self.domain)
    }
}

/// User supplied values for create / update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebsiteInput {
    pub name: String,
    pub domain: String,
    pub project_path: String,
    /// Optional override; detection is used when omitted.
    #[serde(default)]
    pub document_root: Option<String>,
    /// Optional type override; detection is used when omitted.
    #[serde(default)]
    pub project_type: Option<String>,
}

/// Validated, normalised values ready to be planned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWebsite {
    pub id: String,
    pub name: String,
    pub domain: DomainName,
    pub project_path: PathBuf,
    pub document_root: PathBuf,
    pub project_type: ProjectType,
}

/// Validate a website name: letters, digits, dash, underscore and dot, must
/// start with a letter or digit, at most 64 characters.
pub fn validate_name(name: &str) -> CoreResult<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(CoreError::InvalidName("empty name".into()));
    }
    if trimmed.len() > 64 {
        return Err(CoreError::InvalidName(format!(
            "{trimmed} is longer than 64 characters"
        )));
    }
    let mut characters = trimmed.chars();
    let first = characters.next().unwrap_or(' ');
    if !first.is_ascii_alphanumeric() {
        return Err(CoreError::InvalidName(format!(
            "{trimmed} must start with a letter or a digit"
        )));
    }
    if !trimmed
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
    {
        return Err(CoreError::InvalidName(format!(
            "{trimmed} may only contain letters, digits, dot, dash and underscore"
        )));
    }
    Ok(trimmed.to_string())
}

/// Slug used as a stable record id: lowercase, dashes for separators.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for character in name.trim().to_ascii_lowercase().chars() {
        let allowed = character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.');
        if allowed {
            out.push(character);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

impl WebsiteInput {
    /// Validate every field and resolve it against the filesystem.
    pub fn resolve(&self) -> CoreResult<ResolvedWebsite> {
        let name = validate_name(&self.name)?;
        let domain = DomainName::parse(&self.domain)?;

        if self.project_path.trim().is_empty() {
            return Err(CoreError::InvalidPath("project directory is empty".into()));
        }
        let project_path = Path::new(&self.project_path);
        let inspection = crate::project::inspect(project_path)?;

        let document_root = match self.document_root.as_deref() {
            Some(raw) if !raw.trim().is_empty() => {
                crate::project::validate_document_root(project_path, Path::new(raw))?
            }
            _ => inspection.document_root_path().ok_or_else(|| {
                CoreError::InvalidPath(
                    "no document root could be detected; choose one explicitly".into(),
                )
            })?,
        };

        let project_type = match self.project_type.as_deref() {
            Some(raw) if !raw.trim().is_empty() => ProjectType::from_str_opt(raw)
                .ok_or_else(|| CoreError::InvalidInput(format!("unknown project type `{raw}`")))?,
            _ => inspection.project_type,
        };

        Ok(ResolvedWebsite {
            id: slug(&name),
            name,
            domain,
            project_path: PathBuf::from(&inspection.project_path),
            document_root,
            project_type,
        })
    }
}

/// Persisted metadata document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebsiteDocument {
    pub version: u32,
    #[serde(default)]
    pub websites: Vec<Website>,
}

impl Default for WebsiteDocument {
    fn default() -> Self {
        Self {
            version: 1,
            websites: Vec::new(),
        }
    }
}

/// In-memory view of the metadata file.
#[derive(Debug, Clone)]
pub struct WebsiteStore {
    path: PathBuf,
    document: WebsiteDocument,
}

impl WebsiteStore {
    /// Load the store, tolerating a missing file.
    pub fn load(path: &Path) -> CoreResult<Self> {
        let text = crate::storage::read_to_string_opt(path)?;
        let document = match text {
            None => WebsiteDocument::default(),
            Some(raw) => match serde_json::from_str::<WebsiteDocument>(&raw) {
                Ok(document) if document.version == 1 => document,
                Ok(document) => {
                    return Err(CoreError::Storage(format!(
                        "unsupported websites.json version {}",
                        document.version
                    )))
                }
                Err(err) => {
                    return Err(CoreError::Storage(format!(
                        "{} is not valid JSON: {err}",
                        path.display()
                    )))
                }
            },
        };
        Ok(Self {
            path: path.to_path_buf(),
            document,
        })
    }

    pub fn save(&self) -> CoreResult<()> {
        let json = serde_json::to_string_pretty(&self.document)?;
        write_atomic(&self.path, &json)
    }

    pub fn websites(&self) -> &[Website] {
        &self.document.websites
    }

    pub fn len(&self) -> usize {
        self.document.websites.len()
    }

    pub fn is_empty(&self) -> bool {
        self.document.websites.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<&Website> {
        self.document
            .websites
            .iter()
            .find(|website| website.id == id || website.name.eq_ignore_ascii_case(id))
    }

    pub fn by_domain(&self, domain: &str) -> Option<&Website> {
        let domain = domain.trim().to_ascii_lowercase();
        self.document
            .websites
            .iter()
            .find(|website| website.domain == domain)
    }

    pub fn by_project(&self, project: &Path) -> Option<&Website> {
        self.document
            .websites
            .iter()
            .find(|website| Path::new(&website.project_path) == project)
    }

    /// True when the name is already used by another record.
    pub fn name_taken(&self, name: &str, ignore: Option<&str>) -> bool {
        let slug = slug(name);
        self.document.websites.iter().any(|website| {
            Some(website.id.as_str()) != ignore
                && (website.id == slug || website.name.eq_ignore_ascii_case(name.trim()))
        })
    }

    /// True when the domain is already used by another record.
    pub fn domain_taken(&self, domain: &str, ignore: Option<&str>) -> bool {
        let domain = domain.trim().to_ascii_lowercase();
        self.document
            .websites
            .iter()
            .any(|website| Some(website.id.as_str()) != ignore && website.domain == domain)
    }

    pub fn insert(&mut self, website: Website) -> CoreResult<()> {
        if self.get(&website.id).is_some() {
            return Err(CoreError::Duplicate(website.name));
        }
        if self.by_domain(&website.domain).is_some() {
            return Err(CoreError::Duplicate(website.domain));
        }
        self.document.websites.push(website);
        self.document
            .websites
            .sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
        Ok(())
    }

    /// Insert the website, or overwrite the record with the same id.
    ///
    /// Used when reconciling: a recovered site must never collide with a record
    /// that is already there.
    pub fn upsert(&mut self, website: Website) -> CoreResult<()> {
        match self
            .document
            .websites
            .iter()
            .position(|existing| existing.id == website.id)
        {
            Some(position) => self.document.websites[position] = website,
            None => {
                if let Some(other) = self.by_domain(&website.domain) {
                    if other.id != website.id {
                        return Err(CoreError::Duplicate(website.domain));
                    }
                }
                self.document.websites.push(website);
            }
        }
        Ok(())
    }

    pub fn replace(&mut self, website: Website) -> CoreResult<()> {
        let position = self
            .document
            .websites
            .iter()
            .position(|existing| existing.id == website.id)
            .ok_or_else(|| CoreError::NotFound(website.id.clone()))?;
        self.document.websites[position] = website;
        self.document
            .websites
            .sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
        Ok(())
    }

    pub fn remove(&mut self, id: &str) -> CoreResult<Website> {
        let position = self
            .document
            .websites
            .iter()
            .position(|website| website.id == id)
            .ok_or_else(|| CoreError::NotFound(id.to_string()))?;
        Ok(self.document.websites.remove(position))
    }

    /// Build a record from resolved input.
    pub fn new_record(resolved: &ResolvedWebsite, origin: WebsiteOrigin, now: i64) -> Website {
        Website {
            id: resolved.id.clone(),
            name: resolved.name.clone(),
            domain: resolved.domain.as_str().to_string(),
            project_path: resolved.project_path.display().to_string(),
            document_root: resolved.document_root.display().to_string(),
            project_type: resolved.project_type,
            origin: Some(origin),
            created_at_ms: now,
            updated_at_ms: now,
        }
    }
}

/// Convenience: current timestamp.
pub fn timestamp() -> i64 {
    now_ms()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_paths(tag: &str) -> crate::storage::Paths {
        let root = std::env::temp_dir().join(format!("servora-store-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        crate::storage::Paths::default().with_root(&root)
    }

    fn laravel_project(root: &Path) -> PathBuf {
        let dir = root.join("project");
        fs::create_dir_all(dir.join("public")).unwrap();
        fs::create_dir_all(dir.join("bootstrap/cache")).unwrap();
        fs::write(dir.join("artisan"), "#!/usr/bin/env php\n").unwrap();
        fs::write(dir.join("public/index.php"), "<?php\n").unwrap();
        dir
    }

    fn sample_website(id: &str, domain: &str) -> Website {
        Website {
            id: id.into(),
            name: id.into(),
            domain: domain.into(),
            project_path: format!("/tmp/{id}"),
            document_root: format!("/tmp/{id}/public"),
            project_type: ProjectType::Laravel,
            origin: Some(WebsiteOrigin::Created),
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    #[test]
    fn names_are_validated() {
        assert_eq!(validate_name("e-perpus").unwrap(), "e-perpus");
        assert_eq!(validate_name("  red_shop.2  ").unwrap(), "red_shop.2");
        for bad in ["", "-leading", "with space", "slash/name", "emoji😀"] {
            assert!(validate_name(bad).is_err(), "{bad} should be rejected");
        }
        assert!(validate_name(&"a".repeat(65)).is_err());
    }

    #[test]
    fn slugs_are_stable() {
        assert_eq!(slug("E-Perpus"), "e-perpus");
        assert_eq!(slug("red shop"), "red-shop");
        assert_eq!(slug("  weird__name  "), "weird__name");
    }

    #[test]
    fn store_persists_and_reloads() {
        let paths = temp_paths("persist");
        let mut store = WebsiteStore::load(&paths.websites_file()).unwrap();
        store
            .insert(sample_website("e-perpus", "e-perpus.test"))
            .unwrap();
        store.save().unwrap();

        let reloaded = WebsiteStore::load(&paths.websites_file()).unwrap();
        assert_eq!(reloaded.len(), 1);
        assert_eq!(reloaded.get("e-perpus").unwrap().domain, "e-perpus.test");
        fs::remove_dir_all(paths.data_dir.parent().unwrap()).ok();
    }

    #[test]
    fn duplicate_names_and_domains_are_rejected() {
        let paths = temp_paths("dupes");
        let mut store = WebsiteStore::load(&paths.websites_file()).unwrap();
        store.insert(sample_website("a", "a.test")).unwrap();

        assert!(store.insert(sample_website("a", "b.test")).is_err());
        assert!(store.insert(sample_website("b", "a.test")).is_err());
        assert!(store.name_taken("A", None));
        assert!(store.domain_taken("A.test", None));
        assert!(!store.name_taken("A", Some("a")));
        assert!(!store.domain_taken("a.test", Some("a")));
        fs::remove_dir_all(paths.data_dir.parent().unwrap()).ok();
    }

    #[test]
    fn remove_and_replace_work() {
        let paths = temp_paths("remove");
        let mut store = WebsiteStore::load(&paths.websites_file()).unwrap();
        store.insert(sample_website("a", "a.test")).unwrap();
        store.insert(sample_website("b", "b.test")).unwrap();

        let mut record = store.get("a").unwrap().clone();
        record.name = "alpha".into();
        store.replace(record).unwrap();
        assert_eq!(store.get("a").unwrap().name, "alpha");

        let removed = store.remove("b").unwrap();
        assert_eq!(removed.id, "b");
        assert!(store.get("b").is_none());
        assert!(store.remove("b").is_err());
        fs::remove_dir_all(paths.data_dir.parent().unwrap()).ok();
    }

    #[test]
    fn corrupt_metadata_is_reported() {
        let paths = temp_paths("corrupt");
        fs::create_dir_all(&paths.data_dir).unwrap();
        fs::write(paths.websites_file(), "{not json").unwrap();
        let error = WebsiteStore::load(&paths.websites_file()).unwrap_err();
        assert_eq!(error.code(), crate::errors::codes::STORAGE_ERROR);
        fs::remove_dir_all(paths.data_dir.parent().unwrap()).ok();
    }

    #[test]
    fn input_resolution_detects_type_and_document_root() {
        let paths = temp_paths("resolve");
        let project = laravel_project(&paths.data_dir);

        let input = WebsiteInput {
            name: "e-perpus".into(),
            domain: "e-perpus.test".into(),
            project_path: project.display().to_string(),
            document_root: None,
            project_type: None,
        };
        let resolved = input.resolve().unwrap();
        assert_eq!(resolved.id, "e-perpus");
        assert_eq!(resolved.project_type, ProjectType::Laravel);
        assert!(resolved.document_root.ends_with("public"));

        let override_root = WebsiteInput {
            document_root: Some(project.join("bootstrap").display().to_string()),
            project_type: Some("php".into()),
            ..input.clone()
        }
        .resolve()
        .unwrap();
        assert!(override_root.document_root.ends_with("bootstrap"));
        assert_eq!(override_root.project_type, ProjectType::GenericPhp);

        fs::remove_dir_all(paths.data_dir.parent().unwrap()).ok();
    }

    #[test]
    fn input_resolution_rejects_bad_domains_and_paths() {
        let paths = temp_paths("badres");
        let project = laravel_project(&paths.data_dir);

        let bad_domain = WebsiteInput {
            name: "a".into(),
            domain: "not-a-domain".into(),
            project_path: project.display().to_string(),
            document_root: None,
            project_type: None,
        };
        assert!(bad_domain.resolve().is_err());

        let bad_path = WebsiteInput {
            name: "a".into(),
            domain: "a.test".into(),
            project_path: "/nonexistent/project".into(),
            document_root: None,
            project_type: None,
        };
        assert!(bad_path.resolve().is_err());

        let bad_type = WebsiteInput {
            project_type: Some("django".into()),
            ..WebsiteInput {
                name: "a".into(),
                domain: "a.test".into(),
                project_path: project.display().to_string(),
                document_root: None,
                project_type: None,
            }
        };
        assert!(bad_type.resolve().is_err());

        fs::remove_dir_all(paths.data_dir.parent().unwrap()).ok();
    }
}
