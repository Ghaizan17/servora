//! Shared application state.
//!
//! The systemd connection is created lazily and cached. If it drops (systemd
//! restarted, D-Bus hiccup) the caller can [`AppState::reset_client`] and the
//! next request reconnects.

use std::sync::RwLock;
use std::time::Duration;

use tokio::sync::Mutex;

use crate::error::AppResult;
use crate::models::settings::AppSettings;
use crate::services::config;
use crate::services::systemd::{now_ms, SystemdClient};

pub struct AppState {
    settings: RwLock<AppSettings>,
    client: Mutex<Option<SystemdClient>>,
    started_at_ms: i64,
}

impl AppState {
    pub fn new(settings: AppSettings) -> Self {
        Self {
            settings: RwLock::new(settings),
            client: Mutex::new(None),
            started_at_ms: now_ms(),
        }
    }

    pub fn started_at_ms(&self) -> i64 {
        self.started_at_ms
    }

    /// Current settings (cheap clone).
    pub fn settings(&self) -> AppSettings {
        self.settings
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    pub fn refresh_interval(&self) -> Duration {
        Duration::from_millis(self.settings().refresh_interval_ms)
    }

    /// Validate, persist and apply new settings.
    pub fn apply_settings(&self, settings: AppSettings) -> AppResult<AppSettings> {
        settings.validate().map_err(crate::error::AppError::InvalidInput)?;
        config::save(&settings)?;
        if let Ok(mut guard) = self.settings.write() {
            *guard = settings.clone();
        }
        Ok(settings)
    }

    /// Get a cached systemd client, connecting on first use.
    pub async fn client(&self) -> AppResult<SystemdClient> {
        let mut guard = self.client.lock().await;
        if let Some(client) = guard.as_ref() {
            return Ok(client.clone());
        }
        let client = SystemdClient::connect().await?;
        *guard = Some(client.clone());
        Ok(client)
    }

    /// Drop the cached connection so the next request reconnects.
    pub async fn reset_client(&self) {
        let mut guard = self.client.lock().await;
        *guard = None;
    }
}
