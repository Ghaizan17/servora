//! Project inspection: type detection and document root discovery.
//!
//! Detection is purely filesystem based — no Composer, no PHP and no network
//! access are required, so it works on any project the user can read.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::errors::{CoreError, CoreResult};

/// Supported project kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProjectType {
    Laravel,
    Wordpress,
    GenericPhp,
    Unknown,
}

impl ProjectType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProjectType::Laravel => "laravel",
            ProjectType::Wordpress => "wordpress",
            ProjectType::GenericPhp => "php",
            ProjectType::Unknown => "unknown",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ProjectType::Laravel => "Laravel",
            ProjectType::Wordpress => "WordPress",
            ProjectType::GenericPhp => "Generic PHP",
            ProjectType::Unknown => "Unknown",
        }
    }

    pub fn from_str_opt(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "laravel" => Some(ProjectType::Laravel),
            "wordpress" | "wp" => Some(ProjectType::Wordpress),
            "php" | "generic" | "generic-php" => Some(ProjectType::GenericPhp),
            "unknown" => Some(ProjectType::Unknown),
            _ => None,
        }
    }
}

/// Result of inspecting a project directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInspection {
    pub project_path: String,
    pub project_type: ProjectType,
    /// Detected document root (`…/public` or the project root); `None` when the
    /// user has to choose one.
    pub document_root: Option<String>,
    pub detected_by: Vec<String>,
    pub has_artisan: bool,
    pub has_composer: bool,
    pub has_public_index: bool,
    pub has_root_index: bool,
    pub has_wp_config: bool,
    /// Laravel directories that need write access (relative to the project).
    pub writable_dirs: Vec<String>,
    pub warnings: Vec<String>,
}

impl ProjectInspection {
    pub fn document_root_path(&self) -> Option<PathBuf> {
        self.document_root.as_ref().map(PathBuf::from)
    }
}

fn exists(path: &Path) -> bool {
    path.exists()
}

fn is_dir(path: &Path) -> bool {
    path.is_dir()
}

/// Detect the project type from filesystem markers.
pub fn detect_project_type(project: &Path) -> ProjectType {
    let artisan = exists(&project.join("artisan"));
    let public_index = exists(&project.join("public/index.php"));
    let bootstrap_app = exists(&project.join("bootstrap/app.php"));
    let wp_config = exists(&project.join("wp-config.php"));
    let wp_admin = is_dir(&project.join("wp-admin"));
    let wp_includes = is_dir(&project.join("wp-includes"));

    if wp_config || (wp_admin && wp_includes) {
        return ProjectType::Wordpress;
    }
    if (artisan && (public_index || bootstrap_app)) || (bootstrap_app && public_index) {
        return ProjectType::Laravel;
    }
    if public_index || exists(&project.join("index.php")) {
        return ProjectType::GenericPhp;
    }
    ProjectType::Unknown
}

/// Detect the document root: `public/` when it contains `index.php`, otherwise
/// the project root when it contains `index.php`.
pub fn detect_document_root(project: &Path) -> Option<PathBuf> {
    if exists(&project.join("public/index.php")) {
        return Some(project.join("public"));
    }
    if exists(&project.join("index.php")) {
        return Some(project.to_path_buf());
    }
    None
}

/// Full inspection used by the Create / Edit flows.
pub fn inspect(project: &Path) -> CoreResult<ProjectInspection> {
    if !project.exists() {
        return Err(CoreError::InvalidPath(format!(
            "{} does not exist",
            project.display()
        )));
    }
    if !project.is_dir() {
        return Err(CoreError::InvalidPath(format!(
            "{} is not a directory",
            project.display()
        )));
    }
    let canonical = project
        .canonicalize()
        .map_err(|err| CoreError::InvalidPath(format!("{}: {err}", project.display())))?;

    let project_type = detect_project_type(&canonical);
    let document_root = detect_document_root(&canonical);

    let mut detected_by = Vec::new();
    let has_artisan = exists(&canonical.join("artisan"));
    let has_composer = exists(&canonical.join("composer.json"));
    let has_public_index = exists(&canonical.join("public/index.php"));
    let has_root_index = exists(&canonical.join("index.php"));
    let has_wp_config = exists(&canonical.join("wp-config.php"));

    if has_artisan {
        detected_by.push("artisan".to_string());
    }
    if has_composer {
        detected_by.push("composer.json".to_string());
    }
    if has_public_index {
        detected_by.push("public/index.php".to_string());
    }
    if has_root_index {
        detected_by.push("index.php".to_string());
    }
    if has_wp_config {
        detected_by.push("wp-config.php".to_string());
    }
    if is_dir(&canonical.join("wp-admin")) {
        detected_by.push("wp-admin/".to_string());
    }
    if is_dir(&canonical.join("wp-includes")) {
        detected_by.push("wp-includes/".to_string());
    }

    let mut writable_dirs = Vec::new();
    for candidate in ["storage", "bootstrap/cache", "wp-content/uploads"] {
        if is_dir(&canonical.join(candidate)) {
            writable_dirs.push(candidate.to_string());
        }
    }

    let mut warnings = Vec::new();
    if document_root.is_none() {
        warnings.push(
            "No index.php found in the project root or public/; choose a document root manually."
                .to_string(),
        );
    }
    if project_type == ProjectType::Unknown {
        warnings.push(
            "Project type could not be detected from filesystem markers; it will be served as a static/PHP site."
                .to_string(),
        );
    }

    Ok(ProjectInspection {
        project_path: canonical.display().to_string(),
        project_type,
        document_root: document_root.map(|path| path.display().to_string()),
        detected_by,
        has_artisan,
        has_composer,
        has_public_index,
        has_root_index,
        has_wp_config,
        writable_dirs,
        warnings,
    })
}

/// Validate a user supplied document root: it must live inside the project and
/// exist as a directory.
pub fn validate_document_root(project: &Path, document_root: &Path) -> CoreResult<PathBuf> {
    let project = project
        .canonicalize()
        .map_err(|err| CoreError::InvalidPath(format!("{}: {err}", project.display())))?;
    let root = document_root
        .canonicalize()
        .map_err(|err| CoreError::InvalidPath(format!("{}: {err}", document_root.display())))?;

    if !root.is_dir() {
        return Err(CoreError::InvalidPath(format!(
            "{} is not a directory",
            root.display()
        )));
    }
    if !root.starts_with(&project) {
        return Err(CoreError::InvalidPath(format!(
            "{} is outside the project {}",
            root.display(),
            project.display()
        )));
    }
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_project(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("servora-project-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, "<?php\n").unwrap();
    }

    #[test]
    fn detects_laravel_projects() {
        let dir = temp_project("laravel");
        touch(&dir.join("artisan"));
        touch(&dir.join("public/index.php"));
        fs::create_dir_all(dir.join("bootstrap/cache")).unwrap();
        fs::create_dir_all(dir.join("storage/logs")).unwrap();

        let inspection = inspect(&dir).unwrap();
        assert_eq!(inspection.project_type, ProjectType::Laravel);
        assert_eq!(
            inspection.document_root.as_deref(),
            Some(dir.join("public").to_string_lossy().as_ref())
        );
        assert!(inspection.writable_dirs.contains(&"storage".to_string()));
        assert!(inspection
            .writable_dirs
            .contains(&"bootstrap/cache".to_string()));
        assert!(inspection.warnings.is_empty());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn detects_wordpress_projects() {
        let dir = temp_project("wordpress");
        touch(&dir.join("wp-config.php"));
        touch(&dir.join("index.php"));
        fs::create_dir_all(dir.join("wp-admin")).unwrap();
        fs::create_dir_all(dir.join("wp-includes")).unwrap();
        fs::create_dir_all(dir.join("wp-content/uploads")).unwrap();

        let inspection = inspect(&dir).unwrap();
        assert_eq!(inspection.project_type, ProjectType::Wordpress);
        assert_eq!(
            inspection.document_root.as_deref(),
            Some(dir.to_string_lossy().as_ref())
        );
        assert!(inspection
            .writable_dirs
            .contains(&"wp-content/uploads".to_string()));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn detects_generic_php_with_public_directory() {
        let dir = temp_project("generic-public");
        touch(&dir.join("public/index.php"));
        let inspection = inspect(&dir).unwrap();
        assert_eq!(inspection.project_type, ProjectType::GenericPhp);
        assert_eq!(
            inspection.document_root.as_deref(),
            Some(dir.join("public").to_string_lossy().as_ref())
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn detects_generic_php_in_the_project_root() {
        let dir = temp_project("generic-root");
        touch(&dir.join("index.php"));
        let inspection = inspect(&dir).unwrap();
        assert_eq!(inspection.project_type, ProjectType::GenericPhp);
        assert_eq!(
            inspection.document_root.as_deref(),
            Some(dir.to_string_lossy().as_ref())
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reports_unknown_projects_without_an_entry_point() {
        let dir = temp_project("empty");
        fs::write(dir.join("README.md"), "hello").unwrap();
        let inspection = inspect(&dir).unwrap();
        assert_eq!(inspection.project_type, ProjectType::Unknown);
        assert!(inspection.document_root.is_none());
        assert_eq!(inspection.warnings.len(), 2);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_missing_and_non_directory_paths() {
        let missing = Path::new("/nonexistent-servora-project");
        assert!(inspect(missing).is_err());

        let dir = temp_project("file");
        let file = dir.join("index.php");
        touch(&file);
        assert!(inspect(&file).is_err());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn document_root_must_live_inside_the_project() {
        let dir = temp_project("docroot");
        touch(&dir.join("public/index.php"));
        fs::create_dir_all(dir.join("assets")).unwrap();

        assert!(validate_document_root(&dir, &dir.join("public")).is_ok());
        assert!(validate_document_root(&dir, &dir.join("assets")).is_ok());
        assert!(validate_document_root(&dir, Path::new("/tmp")).is_err());
        assert!(validate_document_root(&dir, &dir.join("missing")).is_err());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn project_type_strings_round_trip() {
        for value in [
            ProjectType::Laravel,
            ProjectType::Wordpress,
            ProjectType::GenericPhp,
            ProjectType::Unknown,
        ] {
            assert_eq!(ProjectType::from_str_opt(value.as_str()), Some(value));
        }
        assert_eq!(
            ProjectType::from_str_opt("wp"),
            Some(ProjectType::Wordpress)
        );
        assert!(ProjectType::from_str_opt("django").is_none());
    }
}
