//! `AppState`: the shared handle every handler receives (spec/01 §3.1, §5 step 7).

use std::sync::Arc;

use crate::config::Config;
use crate::db::Db;
use crate::events::EventBus;

/// Shared application state. Cheap to clone; every field is behind an `Arc`.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

pub struct AppStateInner {
    pub db: Db,
    pub config: Config,
    /// Created at startup so `/health` can report it without a query.
    pub vec_version: String,
    /// The process-wide event bus (spec/10 §3).
    pub events: EventBus,
}

impl AppState {
    pub fn new(db: Db, config: Config, vec_version: String) -> Self {
        Self {
            inner: Arc::new(AppStateInner {
                db,
                config,
                vec_version,
                events: EventBus::new(),
            }),
        }
    }

    /// Replaces the event bus (tests subscribe before the app is built).
    pub fn with_events(self, events: EventBus) -> Self {
        Self {
            inner: Arc::new(AppStateInner {
                db: self.inner.db.clone(),
                config: self.inner.config.clone(),
                vec_version: self.inner.vec_version.clone(),
                events,
            }),
        }
    }

    pub fn db(&self) -> &Db {
        &self.inner.db
    }

    pub fn config(&self) -> &Config {
        &self.inner.config
    }

    pub fn vec_version(&self) -> &str {
        &self.inner.vec_version
    }

    pub fn events(&self) -> &EventBus {
        &self.inner.events
    }
}
