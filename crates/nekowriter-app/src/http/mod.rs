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
use nekowriter_agent::client::ChatClient;
use nekowriter_store::Store;

use crate::{lifecycle::Shutdown, preview::Previews};

mod assets;
mod error;
mod extract;
mod gate;
mod handlers;

/// Shared state of all handlers.
#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) store: Store,
    pub(crate) shutdown: Shutdown,
    pub(crate) client: ChatClient,
    pub(crate) previews: Previews,
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
        .route(
            "/projects/{id}/settings",
            post(handlers::projects::update_settings),
        )
        .route(
            "/projects/{id}/versions",
            post(handlers::projects::create_version),
        )
        .route(
            "/projects/{id}/select-version",
            post(handlers::projects::select_version),
        )
        .route(
            "/settings",
            get(handlers::settings::get).post(handlers::settings::save),
        )
        .route("/settings/test-model", post(handlers::settings::test_model))
        .route("/dashboard", get(handlers::dashboard::get))
        .route("/versions/{id}/workspace", get(handlers::workspace::get))
        .route(
            "/versions/{id}/brief",
            post(handlers::workspace::save_brief),
        )
        .route(
            "/versions/{id}/preparation/start",
            post(handlers::workspace::start_preparation),
        )
        .route(
            "/reviews/{id}/decide",
            post(handlers::workspace::decide_review),
        )
        .route(
            "/versions/{id}/chapters/start",
            post(handlers::workspace::start_chapter),
        )
        .route(
            "/versions/{id}/chapters/next",
            post(handlers::workspace::next_chapter),
        )
        .route(
            "/versions/{id}/extra-chapters",
            post(handlers::workspace::add_extra_chapter),
        )
        .route(
            "/versions/{id}/complete",
            post(handlers::workspace::complete),
        )
        .route(
            "/versions/{id}/approval-mode",
            post(handlers::workspace::set_approval_mode),
        )
        .route("/revisions/{id}", get(handlers::workspace::revision))
        .route("/versions/{id}/export", get(handlers::export::export))
        .route(
            "/reviews/{id}/delegate",
            post(handlers::workspace::delegate_review),
        )
        .route(
            "/reviews/{id}/cancel-delegation",
            post(handlers::workspace::cancel_delegation),
        )
        .route(
            "/questions/{id}/answer",
            post(handlers::workspace::answer_questions),
        )
        .route(
            "/runs/{id}/continue",
            post(handlers::workspace::continue_run),
        )
        .route("/runs/{id}/stop", post(handlers::workspace::stop_run))
        .route(
            "/runs/{id}/feedback",
            post(handlers::workspace::submit_feedback),
        )
        .route("/runs/{id}/pause", post(handlers::workspace::pause_run))
        .route(
            "/runs/{id}/cancel-pause",
            post(handlers::workspace::cancel_pause),
        )
        .route("/runs/{id}/resume", post(handlers::workspace::resume_run))
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
