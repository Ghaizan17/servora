//! Native tray icon integration.
//!
//! The tray shows live service states, offers Start All / Stop All, opens the
//! dashboard and quits the application. Quitting never touches running
//! services: systemd owns their lifecycle, Servora only observes and controls.
//!
//! Linux desktops differ a lot here. On GNOME the tray needs the AppIndicator
//! extension; when the tray cannot be created we log it and keep the window
//! fully usable instead of failing to start.

use serde::Serialize;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::commands::service::run_batch;
use crate::models::service::{ServiceId, ServiceOperation, ServiceState};
use crate::models::settings::AppSettings;
use crate::services::logging;
use crate::state::AppState;

/// Event emitted when tray state changes so the dashboard can refresh early.
pub const TRAY_REFRESH_EVENT: &str = "servora://tray-refresh";

/// Menu ids.
const ID_BRAND: &str = "brand";
const ID_HTTPD: &str = "status-httpd";
const ID_PHP: &str = "status-php-fpm";
const ID_MARIADB: &str = "status-mariadb";
const ID_START_ALL: &str = "start-all";
const ID_STOP_ALL: &str = "stop-all";
const ID_REFRESH: &str = "refresh";
const ID_OPEN: &str = "open-dashboard";
const ID_QUIT: &str = "quit";

/// Handles kept alive (and updated later) for the tray menu entries.
pub struct TrayMenu<R: Runtime> {
    pub httpd: MenuItem<R>,
    pub php: MenuItem<R>,
    pub mariadb: MenuItem<R>,
}

impl<R: Runtime> TrayMenu<R> {
    fn item_for(&self, service: ServiceId) -> &MenuItem<R> {
        match service {
            ServiceId::Httpd => &self.httpd,
            ServiceId::PhpFpm => &self.php,
            ServiceId::MariaDb => &self.mariadb,
        }
    }

    /// Update one entry, e.g. `HTTPD — Active`.
    pub fn set_service(&self, service: ServiceId, text: &str) {
        let _ = self.item_for(service).set_text(text);
    }
}

fn status_text(service: ServiceId, state: Option<ServiceState>) -> String {
    let label = match state {
        Some(ServiceState::Active) => "Active",
        Some(ServiceState::Inactive) => "Inactive",
        Some(ServiceState::Activating) => "Activating",
        Some(ServiceState::Deactivating) => "Deactivating",
        Some(ServiceState::Failed) => "Failed",
        Some(ServiceState::Unknown) | None => "Unknown",
    };
    format!("{} — {}", service.short_name(), label)
}

/// Build the tray icon and start the background updater.
pub fn setup<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let brand = MenuItem::with_id(app, ID_BRAND, "Servora", false, None::<&str>)?;
    let httpd = MenuItem::with_id(
        app,
        ID_HTTPD,
        status_text(ServiceId::Httpd, None),
        false,
        None::<&str>,
    )?;
    let php = MenuItem::with_id(
        app,
        ID_PHP,
        status_text(ServiceId::PhpFpm, None),
        false,
        None::<&str>,
    )?;
    let mariadb = MenuItem::with_id(
        app,
        ID_MARIADB,
        status_text(ServiceId::MariaDb, None),
        false,
        None::<&str>,
    )?;
    let separator_one = PredefinedMenuItem::separator(app)?;
    let start_all = MenuItem::with_id(app, ID_START_ALL, "Start All", true, None::<&str>)?;
    let stop_all = MenuItem::with_id(app, ID_STOP_ALL, "Stop All", true, None::<&str>)?;
    let separator_two = PredefinedMenuItem::separator(app)?;
    let refresh = MenuItem::with_id(app, ID_REFRESH, "Refresh status", true, None::<&str>)?;
    let open = MenuItem::with_id(app, ID_OPEN, "Open Dashboard", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, ID_QUIT, "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &brand,
            &httpd,
            &php,
            &mariadb,
            &separator_one,
            &start_all,
            &stop_all,
            &separator_two,
            &refresh,
            &open,
            &quit,
        ],
    )?;

    // Embed the icon so the tray never depends on the bundle configuration.
    let icon = tauri::include_image!("icons/128x128.png");

    let mut builder = TrayIconBuilder::with_id("servora-tray")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("Servora — Local Development Service Manager")
        .on_menu_event(move |app, event| handle_menu(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_dashboard(tray.app_handle());
            }
        });

    builder = builder.icon(icon);

    builder.build(app)?;
    app.manage(TrayMenu {
        httpd,
        php,
        mariadb,
    });

    logging::info("tray icon created");
    start_status_updater(app.clone());
    Ok(())
}

fn handle_menu<R: Runtime>(app: &AppHandle<R>, id: &str) {
    match id {
        ID_OPEN => show_dashboard(app),
        ID_QUIT => {
            logging::info("quit requested from tray; leaving services running");
            app.exit(0);
        }
        ID_START_ALL => spawn_batch(app.clone(), ServiceOperation::Start),
        ID_STOP_ALL => spawn_batch(app.clone(), ServiceOperation::Stop),
        ID_REFRESH => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                refresh_tray_once(&app).await;
                let _ = app.emit(TRAY_REFRESH_EVENT, ());
            });
        }
        _ => {}
    }
}

/// Bring the dashboard to the front (creating nothing, the window already exists).
pub fn show_dashboard<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn spawn_batch<R: Runtime>(app: AppHandle<R>, operation: ServiceOperation) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        match run_batch(&app, &state, operation).await {
            Ok(result) => {
                logging::info(&format!("tray batch {}: {}", operation, result.summary()));
                notify_result(&app, operation, &result.summary());
                let _ = app.emit(TRAY_REFRESH_EVENT, ());
            }
            Err(error) => {
                logging::warn(&format!("tray batch {operation} failed: {}", error.message));
                notify_result(&app, operation, &error.message);
            }
        }
        refresh_tray_once(&app).await;
    });
}

fn notify_result<R: Runtime>(app: &AppHandle<R>, operation: ServiceOperation, message: &str) {
    let settings: AppSettings = app.state::<AppState>().settings();
    if !settings.show_notifications {
        return;
    }
    use tauri_plugin_notification::NotificationExt;
    let title = match operation {
        ServiceOperation::Start => "Servora — Start All",
        ServiceOperation::Stop => "Servora — Stop All",
        ServiceOperation::Restart => "Servora — Restart All",
    };
    if let Err(err) = app
        .notification()
        .builder()
        .title(title)
        .body(message)
        .show()
    {
        logging::warn(&format!("could not show notification: {err}"));
    }
}

/// One status refresh: query systemd and update the three menu labels.
pub async fn refresh_tray_once<R: Runtime>(app: &AppHandle<R>) {
    let Some(menu) = app.try_state::<TrayMenu<R>>() else {
        return;
    };
    let state = app.state::<AppState>();
    let statuses = match state.client().await {
        Ok(client) => match client.statuses().await {
            Ok(statuses) => statuses,
            Err(err) => {
                logging::warn(&format!("tray status refresh failed: {err}"));
                statuses_fallback()
            }
        },
        Err(err) => {
            logging::warn(&format!("tray could not reach systemd: {err}"));
            statuses_fallback()
        }
    };

    for status in statuses {
        menu.set_service(status.id, &status_text(status.id, Some(status.state)));
    }
}

/// Neutral snapshot used when systemd cannot be queried.
fn statuses_fallback() -> Vec<crate::models::service::ServiceStatus> {
    ServiceId::ALL
        .iter()
        .copied()
        .map(|service| {
            crate::models::service::ServiceStatus::unknown(
                service,
                crate::services::systemd::now_ms(),
            )
        })
        .collect()
}

/// Background loop that keeps the tray labels honest.
fn start_status_updater<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        loop {
            let interval = app.state::<AppState>().refresh_interval();
            refresh_tray_once(&app).await;
            let interval = interval.max(std::time::Duration::from_secs(1));
            tokio::time::sleep(interval).await;
        }
    });
}

/// Payload mirrored to the frontend when the tray changes something.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayStatusSummary {
    pub httpd: String,
    pub php_fpm: String,
    pub mariadb: String,
}

impl TrayStatusSummary {
    pub fn from_statuses(statuses: &[crate::models::service::ServiceStatus]) -> Self {
        let label = |id: ServiceId| {
            statuses
                .iter()
                .find(|status| status.id == id)
                .map(|status| status.state.label().to_string())
                .unwrap_or_else(|| "Unknown".to_string())
        };
        Self {
            httpd: label(ServiceId::Httpd),
            php_fpm: label(ServiceId::PhpFpm),
            mariadb: label(ServiceId::MariaDb),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::service::ServiceStatus;

    #[test]
    fn status_text_format() {
        assert_eq!(status_text(ServiceId::Httpd, None), "HTTPD — Unknown");
        assert_eq!(
            status_text(ServiceId::PhpFpm, Some(ServiceState::Active)),
            "PHP-FPM — Active"
        );
        assert_eq!(
            status_text(ServiceId::MariaDb, Some(ServiceState::Failed)),
            "MariaDB — Failed"
        );
    }

    #[test]
    fn summary_maps_every_service() {
        let now = 1;
        let mut statuses: Vec<ServiceStatus> = ServiceId::ALL
            .iter()
            .copied()
            .map(|id| ServiceStatus::unknown(id, now))
            .collect();
        statuses[2].state = ServiceState::Active;
        let summary = TrayStatusSummary::from_statuses(&statuses);
        assert_eq!(summary.mariadb, "Active");
        assert_eq!(summary.httpd, "Unknown");
    }
}
