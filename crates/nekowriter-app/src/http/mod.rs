//! The HTTP layer: routes, request and response types, errors, the trust gate and the
//! embedded web UI.
//!
//! Every route under `/api` speaks JSON. Commands are `POST` requests that carry a
//! `commandId`; reads are `GET` requests. Everything else is the web UI.

use std::{path::PathBuf, sync::Arc};

use axum::{
    Router, middleware,
    routing::{get, post},
};
use nekowriter_store::Store;

use crate::lifecycle::Shutdown;

mod assets;
mod error;
mod extract;
mod gate;
mod handlers;

pub use error::{ApiErrorBody, ApiProblem};
pub use handlers::{
    events::{EventCursorView, EventStreamQuery, ServerMessage},
    projects::{
        CreateProjectRequest, ListProjectsQuery, ProjectRevisionRequest, RenameProjectRequest,
    },
    system::{HealthView, ShutdownStatus, ShutdownView},
};

/// Shared state of all handlers.
#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) store: Store,
    pub(crate) shutdown: Shutdown,
    pub(crate) diagnostics: Arc<Diagnostics>,
    /// The port the server listens on, for the trust gate's allowed origins.
    pub(crate) port: u16,
}

/// Facts about the running service that the diagnostics view shows.
#[derive(Debug)]
pub(crate) struct Diagnostics {
    pub(crate) data_dir: PathBuf,
    pub(crate) log_dir: PathBuf,
}

pub(crate) fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(handlers::system::health))
        .route("/system/shutdown", post(handlers::system::shutdown))
        .route("/events", get(handlers::events::stream))
        .route("/events/cursor", get(handlers::events::cursor))
        .route(
            "/projects",
            get(handlers::projects::list).post(handlers::projects::create),
        )
        .route("/projects/{id}/rename", post(handlers::projects::rename))
        .route("/projects/{id}/delete", post(handlers::projects::delete))
        .route("/projects/{id}/restore", post(handlers::projects::restore))
        .fallback(error::route_not_found);
    Router::new()
        .nest("/api", api)
        .fallback(assets::serve)
        .layer(middleware::from_fn_with_state(state.clone(), gate::trust))
        .with_state(state)
}

/// Whether the web UI is embedded in this build.
pub(crate) fn ui_embedded() -> bool {
    assets::ui_embedded()
}
