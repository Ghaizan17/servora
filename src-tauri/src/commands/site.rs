//! Tauri commands for the Website Manager, Domain Manager and migration.
//!
//! Every handler is a thin adapter: validate the arguments, call
//! [`crate::services::sites::SiteService`] and translate errors into the
//! `{code, message, hint}` payload the frontend already understands.

use serde::{Deserialize, Serialize};
use tauri::State;

use servora_core::migration::MigrationScan;
use servora_core::operations::ConflictReport;
use servora_core::project::ProjectInspection;
use servora_core::virtualhost::ParsedVHost;
use servora_core::website::{Website, WebsiteInput};

use crate::error::ErrorPayload;
use crate::services::sites::{ImportReport, SiteOperationReport, SiteService, WebsiteView};
use crate::state::AppState;

fn service(state: &State<'_, AppState>) -> SiteService {
    SiteService::with_paths(state.site_paths())
}

/// Request body for create / update.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebsiteRequest {
    pub name: String,
    pub domain: String,
    pub project_path: String,
    #[serde(default)]
    pub document_root: Option<String>,
    #[serde(default)]
    pub project_type: Option<String>,
}

impl From<WebsiteRequest> for WebsiteInput {
    fn from(value: WebsiteRequest) -> Self {
        WebsiteInput {
            name: value.name,
            domain: value.domain,
            project_path: value.project_path,
            document_root: value.document_root,
            project_type: value.project_type,
        }
    }
}

/// Body for `rename_website`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameRequest {
    pub website: String,
    pub domain: String,
}

/// Body for `import_siteforge_websites`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    #[serde(default)]
    pub domains: Vec<String>,
    /// Import every importable candidate.
    #[serde(default)]
    pub all: bool,
}

/// Body for project inspection.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathRequest {
    pub path: String,
}

// ------------------------------------------------------------------ reading

#[tauri::command]
pub async fn get_websites(state: State<'_, AppState>) -> Result<Vec<WebsiteView>, ErrorPayload> {
    service(&state).views().map_err(|error| error.to_payload())
}

#[tauri::command]
pub async fn get_website(
    website: String,
    state: State<'_, AppState>,
) -> Result<WebsiteView, ErrorPayload> {
    service(&state)
        .website_view(&website)
        .map_err(|error| error.to_payload())
}

#[tauri::command]
pub async fn get_virtualhost_status(
    domain: String,
    state: State<'_, AppState>,
) -> Result<Option<ParsedVHost>, ErrorPayload> {
    service(&state)
        .virtualhost(&domain)
        .map_err(|error| error.to_payload())
}

#[tauri::command]
pub async fn get_website_history(
    state: State<'_, AppState>,
) -> Result<Vec<crate::services::sites::HistoryEntry>, ErrorPayload> {
    Ok(service(&state).history())
}

// --------------------------------------------------------------- validation

#[tauri::command]
pub async fn detect_project_type(
    request: PathRequest,
    state: State<'_, AppState>,
) -> Result<ProjectInspection, ErrorPayload> {
    service(&state)
        .inspect_project(std::path::Path::new(&request.path))
        .map_err(|error| error.to_payload())
}

#[tauri::command]
pub async fn detect_document_root(
    request: PathRequest,
    state: State<'_, AppState>,
) -> Result<ProjectInspection, ErrorPayload> {
    service(&state)
        .inspect_project(std::path::Path::new(&request.path))
        .map_err(|error| error.to_payload())
}

#[tauri::command]
pub async fn validate_domain(
    domain: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, ErrorPayload> {
    service(&state)
        .validate_domain(&domain)
        .map_err(|error| error.to_payload())
}

#[tauri::command]
pub async fn check_domain_conflicts(
    request: WebsiteRequest,
    website: Option<String>,
    state: State<'_, AppState>,
) -> Result<ConflictReport, ErrorPayload> {
    service(&state)
        .conflicts(&WebsiteInput::from(request), website.as_deref())
        .map_err(|error| error.to_payload())
}

// --------------------------------------------------------------- operations

#[tauri::command]
pub async fn create_website(
    request: WebsiteRequest,
    state: State<'_, AppState>,
) -> Result<SiteOperationReport, ErrorPayload> {
    let _guard = state.site_lock().await;
    service(&state)
        .create(&WebsiteInput::from(request))
        .await
        .map_err(|error| error.to_payload())
}

#[tauri::command]
pub async fn update_website(
    website: String,
    request: WebsiteRequest,
    state: State<'_, AppState>,
) -> Result<SiteOperationReport, ErrorPayload> {
    let _guard = state.site_lock().await;
    service(&state)
        .update(&website, &WebsiteInput::from(request))
        .await
        .map_err(|error| error.to_payload())
}

#[tauri::command]
pub async fn rename_website(
    request: RenameRequest,
    state: State<'_, AppState>,
) -> Result<SiteOperationReport, ErrorPayload> {
    let _guard = state.site_lock().await;
    service(&state)
        .rename(&request.website, &request.domain)
        .await
        .map_err(|error| error.to_payload())
}

#[tauri::command]
pub async fn remove_website(
    website: String,
    state: State<'_, AppState>,
) -> Result<SiteOperationReport, ErrorPayload> {
    let _guard = state.site_lock().await;
    service(&state)
        .remove(&website)
        .await
        .map_err(|error| error.to_payload())
}

// ------------------------------------------------------------------ domains

#[tauri::command]
pub async fn get_managed_domains(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, ErrorPayload> {
    service(&state)
        .managed_domains()
        .map_err(|error| error.to_payload())
}

// ---------------------------------------------------------------- migration

#[tauri::command]
pub async fn preview_siteforge_migration(
    state: State<'_, AppState>,
) -> Result<MigrationScan, ErrorPayload> {
    service(&state)
        .preview_migration()
        .map_err(|error| error.to_payload())
}

#[tauri::command]
pub async fn import_siteforge_websites(
    request: ImportRequest,
    state: State<'_, AppState>,
) -> Result<ImportReport, ErrorPayload> {
    let _guard = state.site_lock().await;
    let service = service(&state);
    let domains = if request.all {
        service
            .preview_migration()
            .map_err(|error| error.to_payload())?
            .importable()
            .into_iter()
            .map(|candidate| candidate.domain.clone())
            .collect()
    } else {
        request.domains
    };
    service
        .import(&domains)
        .await
        .map_err(|error| error.to_payload())
}

// -------------------------------------------------------------- diagnostics

#[tauri::command]
pub async fn get_environment_diagnostics(
    state: State<'_, AppState>,
) -> Result<servora_core::diagnostics::DiagnosticsReport, ErrorPayload> {
    service(&state)
        .diagnostics()
        .map_err(|error| error.to_payload())
}

#[tauri::command]
pub async fn get_environment_setup(
    state: State<'_, AppState>,
) -> Result<servora_core::diagnostics::EnvironmentReport, ErrorPayload> {
    service(&state)
        .environment()
        .map_err(|error| error.to_payload())
}

#[tauri::command]
pub async fn validate_apache_configuration(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, ErrorPayload> {
    service(&state)
        .validate_apache()
        .map_err(|error| error.to_payload())
}

#[tauri::command]
pub async fn reload_apache(
    state: State<'_, AppState>,
) -> Result<servora_core::plan::PlanOutcome, ErrorPayload> {
    let _guard = state.site_lock().await;
    service(&state)
        .reload_apache()
        .await
        .map_err(|error| error.to_payload())
}

// ------------------------------------------------------------------- helper

/// Paths the UI needs (data directory, Apache configuration directory, …).
#[tauri::command]
pub async fn get_site_paths(state: State<'_, AppState>) -> Result<serde_json::Value, ErrorPayload> {
    let paths = state.site_paths();
    Ok(serde_json::json!({
        "configDir": paths.config_dir.display().to_string(),
        "dataDir": paths.data_dir.display().to_string(),
        "backupsDir": paths.backups_dir().display().to_string(),
        "apacheConfDir": paths.apache_conf_dir.display().to_string(),
        "hostsFile": paths.hosts_file.display().to_string(),
        "helperPath": paths.helper_path.display().to_string(),
        "helperInstalled": paths.helper_path.exists(),
    }))
}

/// Helper used by tests to build a deterministic request body.
impl WebsiteRequest {
    pub fn new(name: &str, domain: &str, project_path: &str, document_root: Option<&str>) -> Self {
        Self {
            name: name.into(),
            domain: domain.into(),
            project_path: project_path.into(),
            document_root: document_root.map(str::to_string),
            project_type: None,
        }
    }
}

#[allow(dead_code)]
fn _assert_website_serialises(website: &Website) -> String {
    serde_json::to_string(website).unwrap_or_default()
}
