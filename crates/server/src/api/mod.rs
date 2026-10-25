//! API router assembly, the host/origin guard and the SPA fallback (spec/01 §10, §12).

pub mod app_settings;
pub mod books;
pub mod canon;
pub mod changes;
pub mod chapters;
#[cfg(debug_assertions)]
pub mod debug;
pub mod dto;
pub mod embedding;
pub mod events;
pub mod health;
pub mod knowledge;
pub mod prompts;
pub mod proposals;
pub mod providers;
pub mod research;
pub mod reviews;
pub mod search_settings;
pub mod sessions;
pub mod settings_materials;
pub mod spa;
pub mod tracks;

use std::net::IpAddr;
use std::str::FromStr;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post, put};

use crate::error::{AppError, ErrorCode};
use crate::state::AppState;

/// Hosts accepted by the guard: the API is loopback-only (spec/01 §10.2).
const ALLOWED_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "[::1]"];

/// Builds the whole application router. Tests call this directly.
pub fn build_app(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health::health))
        .route(
            "/app-settings",
            get(app_settings::get).patch(app_settings::patch),
        )
        .route("/app-settings/search/test", post(search_settings::test))
        .route("/books", get(books::list).post(books::create))
        .route(
            "/books/{book_id}",
            get(books::get).patch(books::patch).delete(books::delete),
        )
        .route("/books/{book_id}/overview", get(books::overview))
        .route(
            "/books/{book_id}/conception/start",
            post(books::conception_start),
        )
        .route(
            "/books/{book_id}/conception/finalize",
            post(books::conception_finalize),
        )
        .route("/books/{book_id}/canon/seed", post(books::canon_seed))
        .route(
            "/books/{book_id}/tracks",
            get(tracks::list).post(tracks::create),
        )
        .route(
            "/tracks/{track_id}",
            get(tracks::get).patch(tracks::patch).delete(tracks::delete),
        )
        .route(
            "/tracks/{track_id}/chapters",
            get(tracks::list_chapters).post(tracks::create_chapter),
        )
        .route(
            "/tracks/{track_id}/chapters/reorder",
            post(tracks::reorder_chapters),
        )
        .route(
            "/chapters/{chapter_id}",
            get(chapters::get)
                .patch(chapters::patch)
                .delete(chapters::delete),
        )
        .route(
            "/chapters/{chapter_id}/content",
            patch(chapters::patch_content),
        )
        .route(
            "/chapters/{chapter_id}/content/text",
            put(chapters::put_content_text),
        )
        .route("/chapters/{chapter_id}/write", post(chapters::write))
        .route("/chapters/{chapter_id}/submit", post(chapters::submit))
        .route("/chapters/{chapter_id}/approve", post(chapters::approve))
        .route("/chapters/{chapter_id}/review", post(reviews::review))
        .route("/chapters/{chapter_id}/send-back", post(reviews::send_back))
        .route("/chapters/{chapter_id}/reviews", get(reviews::reports))
        .route("/chapters/{chapter_id}/maintain", post(reviews::maintain))
        .route("/chapters/{chapter_id}/resync", post(reviews::resync))
        .route(
            "/review-reports/{report_id}/rereview",
            post(reviews::rereview),
        )
        .route(
            "/chapters/{chapter_id}/versions",
            get(chapters::list_versions).post(chapters::create_version),
        )
        .route("/chapter-versions/{version_id}", get(chapters::get_version))
        .route(
            "/chapter-versions/{version_id}/restore",
            post(chapters::restore_version),
        )
        .route(
            "/books/{book_id}/setting-docs",
            get(settings_materials::list_docs),
        )
        .route(
            "/books/{book_id}/setting-docs/{kind}",
            get(settings_materials::get_doc).put(settings_materials::put_doc),
        )
        .route(
            "/books/{book_id}/setting-docs/{kind}/history",
            get(settings_materials::doc_history),
        )
        .route(
            "/books/{book_id}/characters",
            get(settings_materials::list_characters).post(settings_materials::create_character),
        )
        .route(
            "/books/{book_id}/characters/reorder",
            post(settings_materials::reorder_characters),
        )
        .route("/books/{book_id}/proposals", get(proposals::list))
        .route("/proposals/{proposal_id}/accept", post(proposals::accept))
        .route("/proposals/{proposal_id}/reject", post(proposals::reject))
        .route(
            "/characters/{character_id}",
            get(settings_materials::get_character)
                .patch(settings_materials::patch_character)
                .delete(settings_materials::delete_character),
        )
        .route(
            "/characters/{character_id}/history",
            get(settings_materials::character_history),
        )
        .route("/books/{book_id}/changes", get(changes::list))
        .route("/changes/{change_id}", get(changes::get))
        .route("/changes/{change_id}/revert", post(canon::revert_change))
        // Canon (spec/10 §2.4).
        .route(
            "/books/{book_id}/entities",
            get(canon::list_entities).post(canon::create_entity),
        )
        .route(
            "/entities/{entity_id}",
            get(canon::get_entity)
                .patch(canon::patch_entity)
                .delete(canon::delete_entity),
        )
        .route("/entities/{entity_id}/merge", post(canon::merge_entity))
        .route(
            "/entities/{entity_id}/protagonist",
            post(canon::set_protagonist),
        )
        .route("/entities/{entity_id}/history", get(canon::entity_history))
        .route(
            "/books/{book_id}/relations",
            get(canon::list_relations).post(canon::create_relation),
        )
        .route(
            "/relations/{relation_id}",
            get(canon::get_relation)
                .patch(canon::patch_relation)
                .delete(canon::delete_relation),
        )
        .route(
            "/chapters/{chapter_id}/appearances",
            get(canon::chapter_appearances).put(canon::put_appearances),
        )
        .route(
            "/books/{book_id}/timeline",
            get(canon::list_timeline).post(canon::create_timeline_event),
        )
        .route(
            "/timeline-events/{event_id}",
            get(canon::get_timeline_event)
                .patch(canon::patch_timeline_event)
                .delete(canon::delete_timeline_event),
        )
        .route(
            "/timeline-events/{event_id}/move",
            post(canon::move_timeline_event),
        )
        .route(
            "/books/{book_id}/memory-notes",
            get(canon::list_memory_notes).post(canon::create_memory_note),
        )
        .route(
            "/memory-notes/{note_id}",
            get(canon::get_memory_note)
                .patch(canon::patch_memory_note)
                .delete(canon::delete_memory_note),
        )
        .route(
            "/books/{book_id}/foreshadowing",
            get(canon::list_foreshadowing).post(canon::create_foreshadowing),
        )
        .route(
            "/foreshadowing/{item_id}",
            get(canon::get_foreshadowing)
                .patch(canon::patch_foreshadowing)
                .delete(canon::delete_foreshadowing),
        )
        .route("/books/{book_id}/memory", get(canon::memory_snapshot))
        .route("/embedding/status", get(embedding::status))
        .route("/embedding/rebuild", post(embedding::rebuild))
        // Knowledge base (spec/10 §2.5).
        .route(
            "/knowledge/scopes",
            get(knowledge::list_scopes).post(knowledge::create_scope),
        )
        .route(
            "/knowledge/scopes/{scope_id}",
            get(knowledge::get_scope)
                .patch(knowledge::patch_scope)
                .delete(knowledge::delete_scope),
        )
        .route(
            "/knowledge/scopes/{scope_id}/entries",
            get(knowledge::list_entries).post(knowledge::create_entry),
        )
        .route(
            "/knowledge/entries/{entry_id}",
            get(knowledge::get_entry)
                .patch(knowledge::patch_entry)
                .delete(knowledge::delete_entry),
        )
        .route("/knowledge/search", get(knowledge::search))
        .route(
            "/research-tasks",
            get(research::list).post(research::create),
        )
        .route(
            "/research-tasks/{task_id}",
            get(research::get)
                .patch(research::patch)
                .delete(research::delete),
        )
        .route("/research-tasks/{task_id}/run", post(research::run))
        .route(
            "/books/{book_id}/scopes",
            get(knowledge::list_book_scopes).put(knowledge::put_book_scopes),
        )
        .route("/events", get(events::stream))
        .route("/providers", get(providers::list).post(providers::create))
        .route("/provider-presets", get(providers::presets))
        .route(
            "/providers/{provider_id}",
            get(providers::get)
                .patch(providers::patch)
                .delete(providers::delete),
        )
        .route("/providers/{provider_id}/test", post(providers::test))
        .route(
            "/providers/{provider_id}/remote-models",
            get(providers::remote_models),
        )
        .route(
            "/model-profiles",
            get(providers::list_models).post(providers::create_model),
        )
        .route(
            "/model-profiles/{model_profile_id}",
            get(providers::get_model)
                .patch(providers::patch_model)
                .delete(providers::delete_model),
        )
        .route("/prompts", get(prompts::list))
        .route("/prompts/{key}", put(prompts::save).delete(prompts::reset))
        .route("/books/{book_id}/sessions", get(sessions::list))
        .route(
            "/sessions/{session_id}",
            get(sessions::get).patch(sessions::patch),
        )
        .route("/sessions/{session_id}/messages", get(sessions::messages))
        .route("/sessions/{session_id}/pause", post(sessions::pause))
        .route("/sessions/{session_id}/resume", post(sessions::resume))
        .route("/sessions/{session_id}/stop", post(sessions::stop))
        .route(
            "/sessions/{session_id}/continue",
            post(sessions::continue_session),
        )
        .route("/sessions/{session_id}/abandon", post(sessions::abandon))
        .route(
            "/sessions/{session_id}/message",
            post(sessions::send_message),
        )
        .route("/sessions/{session_id}/answer", post(sessions::answer))
        // An unknown `/api` path must answer with the documented error envelope (spec/10 §1), not
        // with axum's empty-body 404 — and it must never fall through to the SPA's `index.html`.
        .fallback(api_not_found)
        .with_state(state.clone());

    // The debug endpoints exist only in debug builds (spec/10 §2.7).
    #[cfg(debug_assertions)]
    let api = api.route("/debug/sessions/echo", post(debug::echo));

    // `Json<T>`'s own rejection produces a bare 422 with a plain-text body, which breaks the
    // envelope contract of spec/10 §1/§4 (and the frontend's `ApiError`, spec/11 §4.1). Route the
    // extractor rejections of the API subtree through `AppError` instead.
    let api = api.layer(middleware::map_response(rewrite_extractor_rejection));

    Router::new()
        .nest("/api", api)
        // Everything else is the SPA: the embedded assets, `--web-dir`, or the hint page
        // (spec/01 §12). The fallback also covers `GET /`.
        .fallback(spa::serve)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            host_origin_guard,
        ))
        // Outermost, so the headers are also present on the guard's 403 and on 404s
        // (spec/01 §10.6).
        .layer(middleware::from_fn(security_headers))
        .with_state(state)
}

/// Rejects requests whose `Host` is not a loopback name — or, with `--allow-remote`, not an IP
/// literal — and non-GET requests whose `Origin` is neither a loopback origin nor, with
/// `--allow-remote`, the request's own host (DNS-rebinding and CSRF guard, spec/01 §10.2, D-126).
async fn host_origin_guard(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let headers = request.headers();
    let method = request.method().clone();
    let remote = state.config().allow_remote;

    match headers.get(header::HOST) {
        Some(host) if host_allowed(host) || (remote && remote_host_allowed(host)) => {}
        Some(host) => {
            return Err(AppError::new(
                ErrorCode::HostForbidden,
                format!(
                    "host {} is not allowed",
                    host.to_str().unwrap_or("<invalid>")
                ),
            ));
        }
        None => {
            return Err(AppError::new(
                ErrorCode::HostForbidden,
                "missing Host header",
            ));
        }
    }

    // spec/01 §10.2 checks `Origin` for every non-GET method; `HEAD` is not GET, so it is
    // checked too. Only GET is exempt (it is the method a plain navigation uses, and it carries
    // no `Origin`).
    if method != axum::http::Method::GET
        && let Some(origin) = headers.get(header::ORIGIN)
        && !origin_allowed(origin)
        && !(remote && origin_matches_host(origin, headers.get(header::HOST)))
    {
        return Err(AppError::new(
            ErrorCode::HostForbidden,
            format!(
                "origin {} is not allowed",
                origin.to_str().unwrap_or("<invalid>")
            ),
        ));
    }

    Ok(next.run(request).await)
}

/// Turns an extractor rejection into the documented error envelope.
///
/// axum answers a missing/invalid JSON body or a malformed path parameter itself, with
/// `422`/`400` and an empty body. spec/10 §1 fixes the shape of every error response to
/// `{ error: { code, message, details } }`, so rewrite those responses in place: a body problem
/// becomes `validation.failed` (400) with `details.fields`, matching the codes services raise.
async fn rewrite_extractor_rejection(response: Response) -> Response {
    if response.status().is_success() || response.status().is_redirection() {
        return response;
    }
    if response
        .extensions()
        .get::<crate::error::ErrorEnvelope>()
        .is_some()
    {
        return response;
    }
    let status = response.status();
    if status != axum::http::StatusCode::UNPROCESSABLE_ENTITY
        && status != axum::http::StatusCode::BAD_REQUEST
        && status != axum::http::StatusCode::UNSUPPORTED_MEDIA_TYPE
    {
        return response;
    }
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap_or_default();
    let detail = String::from_utf8_lossy(&body).trim().to_owned();
    let message = match status {
        axum::http::StatusCode::UNPROCESSABLE_ENTITY => "the request body is not valid JSON",
        axum::http::StatusCode::UNSUPPORTED_MEDIA_TYPE => {
            "the request body must be application/json"
        }
        _ => "the request is not valid",
    };
    let fields = if detail.is_empty() {
        serde_json::Map::new()
    } else {
        serde_json::Map::from_iter([("body".to_owned(), serde_json::Value::String(detail))])
    };
    AppError::invalid_fields(fields)
        .with_message(message)
        .into_response()
}

/// A `Host` header value is allowed when its host part is loopback (any port).
fn host_allowed(value: &HeaderValue) -> bool {
    let Ok(text) = value.to_str() else {
        return false;
    };
    ALLOWED_HOSTS.contains(&strip_port(text))
}

/// A `Host` header value is allowed in the `--allow-remote` case when its host part is an **IP
/// literal** (`10.2.0.7:8789`, `[2001:db8::1]:8789`, or either without a port).
///
/// Hostnames are deliberately not accepted here: DNS rebinding needs a hostname to point at the
/// server, so keeping every hostname forbidden preserves the guard's purpose (spec/01 §10.2,
/// D-126).
fn remote_host_allowed(value: &HeaderValue) -> bool {
    let Ok(text) = value.to_str() else {
        return false;
    };
    host_is_ip_literal(strip_port(text))
}

/// True when `host` (a host, not an authority) is a bare IP literal; `[v6]` brackets are
/// accepted and removed first.
fn host_is_ip_literal(host: &str) -> bool {
    let host = match host.strip_prefix('[') {
        Some(rest) => match rest.strip_suffix(']') {
            Some(inner) => inner,
            None => return false,
        },
        None => host,
    };
    IpAddr::from_str(host).is_ok()
}

/// An `Origin` value is allowed when it is `http(s)://<loopback>[:port]`.
fn origin_allowed(value: &HeaderValue) -> bool {
    let Ok(text) = value.to_str() else {
        return false;
    };
    let Some((scheme, rest)) = text.split_once("://") else {
        // `Origin: null` (sandboxed iframe, file://) is not a loopback origin.
        return false;
    };
    if scheme != "http" && scheme != "https" {
        return false;
    }
    ALLOWED_HOSTS.contains(&strip_port(rest))
}

/// An `Origin` is allowed in the `--allow-remote` case when it is `http(s)` and its authority is
/// **exactly** the request's `Host` header — the page that serves the SPA is the only one allowed
/// to drive the API.
///
/// Deliberately *not* "any IP-literal origin": a page on a different LAN address could otherwise
/// drive this API cross-origin (spec/01 §10.2, D-126).
fn origin_matches_host(origin: &HeaderValue, host: Option<&HeaderValue>) -> bool {
    let (Ok(origin), Some(Ok(host))) = (origin.to_str(), host.map(HeaderValue::to_str)) else {
        return false;
    };
    let Some((scheme, rest)) = origin.split_once("://") else {
        // `Origin: null` has no scheme and therefore never matches (spec/01 §10.2).
        return false;
    };
    if scheme != "http" && scheme != "https" {
        return false;
    }
    // Case-insensitive, because DNS names and the scheme are compared case-insensitively by the
    // browser; the host parts here are IP literals, so this cannot mask a real difference.
    rest.eq_ignore_ascii_case(host)
}

/// Strips a trailing `:port` from a host, keeping IPv6 bracket notation intact.
fn strip_port(host: &str) -> &str {
    if let Some(stripped) = host.strip_prefix("[::1]") {
        // `[::1]`, `[::1]:5173`
        return if stripped.is_empty() || stripped.starts_with(':') {
            "[::1]"
        } else {
            host
        };
    }
    match host.rsplit_once(':') {
        Some((name, port)) if port.chars().all(|c| c.is_ascii_digit()) => name,
        _ => host,
    }
}

/// The fallback for an unknown `/api` path: the standard error envelope (spec/10 §1, §4).
async fn api_not_found() -> AppError {
    AppError::new(ErrorCode::NotFound, "no such endpoint")
}

/// Adds the static-file security headers from spec/01 §10.6 to every response.
pub async fn security_headers(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'",
        ),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_parsing_accepts_loopback_names() {
        for host in [
            "localhost",
            "localhost:5173",
            "127.0.0.1",
            "127.0.0.1:7878",
            "[::1]",
            "[::1]:7878",
        ] {
            assert!(
                host_allowed(&HeaderValue::from_str(host).expect("header")),
                "{host} should be allowed"
            );
        }
    }

    #[test]
    fn host_parsing_rejects_other_names() {
        for host in [
            "evil.com",
            "evil.com:7878",
            "10.0.0.1",
            "localhost.evil.com",
        ] {
            assert!(
                !host_allowed(&HeaderValue::from_str(host).expect("header")),
                "{host} should be rejected"
            );
        }
    }

    #[test]
    fn origin_parsing_accepts_loopback_origins() {
        for origin in [
            "http://localhost:5173",
            "http://127.0.0.1:7878",
            "https://127.0.0.1",
            "http://[::1]:5173",
        ] {
            assert!(
                origin_allowed(&HeaderValue::from_str(origin).expect("header")),
                "{origin} should be allowed"
            );
        }
    }

    #[test]
    fn origin_parsing_rejects_other_origins() {
        for origin in [
            "http://evil.com",
            "null",
            "file://",
            "http://localhost.evil.com",
            "chrome-extension://abc",
        ] {
            assert!(
                !origin_allowed(&HeaderValue::from_str(origin).expect("header")),
                "{origin} should be rejected"
            );
        }
    }
}

#[cfg(test)]
mod guard_tests {
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    fn test_config(data_dir: &std::path::Path) -> crate::config::Config {
        crate::config::Config {
            bind: "127.0.0.1:0".parse().expect("bind"),
            data_dir: Some(data_dir.to_path_buf()),
            web_dir: None,
            enable_demo_provider: true,
            allow_remote: false,
        }
    }

    /// The `--allow-remote` variant: the same config with the flag on (spec/01 §5 step 10, D-126).
    fn test_config_allow_remote(data_dir: &std::path::Path) -> crate::config::Config {
        crate::config::Config {
            allow_remote: true,
            ..test_config(data_dir)
        }
    }

    /// Builds an app over a fresh migrated database with `config`.
    async fn test_app_with_config(
        config: impl FnOnce(&std::path::Path) -> crate::config::Config,
    ) -> (tempfile::TempDir, Router) {
        let dir = tempfile::tempdir().expect("temp dir");
        let db = crate::db::Db::connect(&dir.path().join("test.db"))
            .await
            .expect("db");
        nekowriter_migration::up(db.conn()).await.expect("migrate");
        let vec_version = db.vec_version().await.expect("vec");
        let state = AppState::new(db, config(dir.path()), vec_version);
        (dir, build_app(state))
    }

    /// The default app: loopback only, `--allow-remote` off.
    async fn test_app() -> (tempfile::TempDir, Router) {
        test_app_with_config(test_config).await
    }

    /// The `--allow-remote` app (spec/01 §5 step 10).
    async fn test_app_allow_remote() -> (tempfile::TempDir, Router) {
        test_app_with_config(test_config_allow_remote).await
    }

    /// Sends one request through the app and returns its status and headers.
    async fn send(
        app: Router,
        method: axum::http::Method,
        path: &str,
        origin: Option<&str>,
    ) -> (axum::http::StatusCode, axum::http::HeaderMap) {
        send_with_host(app, method, path, "127.0.0.1:7878", origin).await
    }

    /// Sends one request with an explicit `Host` header.
    async fn send_with_host(
        app: Router,
        method: axum::http::Method,
        path: &str,
        host: &str,
        origin: Option<&str>,
    ) -> (axum::http::StatusCode, axum::http::HeaderMap) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, host);
        if let Some(origin) = origin {
            request = request.header(header::ORIGIN, origin);
        }
        let response = app
            .oneshot(request.body(Body::empty()).expect("request"))
            .await
            .expect("response");
        (response.status(), response.headers().clone())
    }

    /// spec/01 §10.2: for **non-GET** methods a foreign `Origin` is rejected. `HEAD` is not GET,
    /// so it must be rejected too — otherwise `HEAD` is a CSRF hole around the guard.
    #[tokio::test]
    async fn head_with_foreign_origin_is_forbidden() {
        let (_dir, app) = test_app().await;
        let (status, _) = send(
            app,
            axum::http::Method::HEAD,
            "/api/health",
            Some("http://evil.com"),
        )
        .await;
        assert_eq!(
            status,
            axum::http::StatusCode::FORBIDDEN,
            "HEAD must be subject to the Origin check (spec/01 §10.2)"
        );
    }

    /// The other non-GET methods are rejected as well (spec/01 §10.2).
    #[tokio::test]
    async fn non_get_methods_with_foreign_origin_are_forbidden() {
        for method in [
            axum::http::Method::POST,
            axum::http::Method::PUT,
            axum::http::Method::PATCH,
            axum::http::Method::DELETE,
            axum::http::Method::OPTIONS,
        ] {
            let (_dir, app) = test_app().await;
            let (status, _) =
                send(app, method.clone(), "/api/health", Some("http://evil.com")).await;
            assert_eq!(
                status,
                axum::http::StatusCode::FORBIDDEN,
                "{method} with a foreign Origin must be forbidden"
            );
        }
    }

    /// `GET` stays exempt: a plain navigation carries no `Origin`, and the guard must not break
    /// reads (spec/01 §10.2 checks non-GET methods only).
    #[tokio::test]
    async fn get_is_exempt_from_the_origin_check() {
        let (_dir, app) = test_app().await;
        let (status, _) = send(
            app,
            axum::http::Method::GET,
            "/api/health",
            Some("http://evil.com"),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
    }

    /// A loopback `Origin` on a non-GET request is accepted (the Vite dev server case).
    #[tokio::test]
    async fn loopback_origin_on_a_mutation_is_accepted() {
        let (_dir, app) = test_app().await;
        let (status, _) = send(
            app,
            axum::http::Method::HEAD,
            "/api/health",
            Some("http://localhost:5173"),
        )
        .await;
        assert_ne!(status, axum::http::StatusCode::FORBIDDEN);
    }

    /// D-126: `--allow-remote` opens the bind to `0.0.0.0`, so a browser on another machine
    /// addresses the server by an **IP literal**. The guard must accept that Host, or the flag is
    /// useless (the reported bug: `Host: 10.2.0.7:8789` → 403 `host.forbidden`).
    #[tokio::test]
    async fn remote_ip_literal_hosts_are_accepted_with_allow_remote() {
        for host in [
            "10.2.0.7:8789",
            "[2001:db8::1]:8789",
            "10.2.0.7",
            "[2001:db8::1]",
            // The loopback names keep working while the flag is on.
            "localhost:8789",
            "127.0.0.1:8789",
            "[::1]:8789",
        ] {
            let (_dir, app) = test_app_allow_remote().await;
            let (status, _) =
                send_with_host(app, axum::http::Method::GET, "/api/health", host, None).await;
            assert_eq!(
                status,
                axum::http::StatusCode::OK,
                "Host {host} must be accepted with --allow-remote (D-126)"
            );
        }
    }

    /// D-126: the guard exists to stop DNS rebinding, and a rebinding attack **needs a hostname**
    /// (the browser sends `Host: evil.com`). Hostnames therefore stay forbidden even with the flag
    /// on, and the whole check is off for the flag's default.
    #[tokio::test]
    async fn hostnames_stay_forbidden_with_allow_remote() {
        for host in [
            "evil.com",
            "evil.com:8789",
            "localhost.evil.com",
            "10.2.0.7.nip.io",
        ] {
            let (_dir, app) = test_app_allow_remote().await;
            let (status, _) =
                send_with_host(app, axum::http::Method::GET, "/api/health", host, None).await;
            assert_eq!(
                status,
                axum::http::StatusCode::FORBIDDEN,
                "Host {host} must stay forbidden: a rebinding attack needs a hostname (D-126)"
            );
        }
    }

    /// The regression guard: without `--allow-remote` nothing changes — an IP-literal Host is
    /// still rejected exactly as before the fix (spec/01 §10.2, D-126).
    #[tokio::test]
    async fn ip_literal_hosts_are_forbidden_without_allow_remote() {
        for host in ["10.2.0.7:8789", "[2001:db8::1]:8789", "10.2.0.7"] {
            let (_dir, app) = test_app().await;
            let (status, _) =
                send_with_host(app, axum::http::Method::GET, "/api/health", host, None).await;
            assert_eq!(
                status,
                axum::http::StatusCode::FORBIDDEN,
                "Host {host} must stay forbidden by default (the flag is off)"
            );
        }
    }

    /// D-126: with `--allow-remote` a remote page's own origin is legitimate — its authority is
    /// exactly the request's `Host`. (A loopback origin is already accepted, so the Vite dev case
    /// is covered by `loopback_origin_on_a_mutation_is_accepted`.)
    #[tokio::test]
    async fn remote_origin_matching_the_host_is_accepted_with_allow_remote() {
        for (host, origin) in [
            ("10.2.0.7:8789", "http://10.2.0.7:8789"),
            ("10.2.0.7:8789", "https://10.2.0.7:8789"),
            ("[2001:db8::1]:8789", "http://[2001:db8::1]:8789"),
        ] {
            let (_dir, app) = test_app_allow_remote().await;
            let (status, _) = send_with_host(
                app,
                axum::http::Method::POST,
                "/api/health",
                host,
                Some(origin),
            )
            .await;
            assert_ne!(
                status,
                axum::http::StatusCode::FORBIDDEN,
                "Origin {origin} matching Host {host} must be accepted (D-126)"
            );
        }
    }

    /// D-126: the origin must equal the request's `Host`, not merely be an IP literal. A page
    /// served from another LAN address could otherwise drive this API cross-origin — that is the
    /// CSRF hole the Origin check exists to close.
    #[tokio::test]
    async fn a_foreign_lan_origin_is_forbidden_even_with_allow_remote() {
        let (_dir, app) = test_app_allow_remote().await;
        let (status, _) = send_with_host(
            app,
            axum::http::Method::POST,
            "/api/health",
            "10.2.0.7:8789",
            Some("http://192.168.1.99:8789"),
        )
        .await;
        assert_eq!(
            status,
            axum::http::StatusCode::FORBIDDEN,
            "an Origin from a different LAN address must not drive the API (D-126)"
        );
    }

    /// D-126: `Origin: null` (sandboxed iframe, local file) is never allowed, flag or not.
    #[tokio::test]
    async fn null_origin_stays_forbidden_with_allow_remote() {
        let (_dir, app) = test_app_allow_remote().await;
        let (status, _) = send_with_host(
            app,
            axum::http::Method::POST,
            "/api/health",
            "10.2.0.7:8789",
            Some("null"),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::FORBIDDEN);
    }

    /// The same non-GET request without `--allow-remote` keeps the pre-fix behaviour: the IP Host
    /// is rejected before `Origin` is even considered.
    #[tokio::test]
    async fn remote_origin_is_forbidden_without_allow_remote() {
        let (_dir, app) = test_app().await;
        let (status, _) = send_with_host(
            app,
            axum::http::Method::POST,
            "/api/health",
            "10.2.0.7:8789",
            Some("http://10.2.0.7:8789"),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::FORBIDDEN);
    }

    /// spec/01 §10.6: static files are served with `X-Content-Type-Options: nosniff` and a CSP
    /// that allows only self. The layer must cover API responses, the hint page and 404s.
    #[tokio::test]
    async fn security_headers_are_present_on_every_response() {
        for path in ["/api/health", "/", "/does-not-exist"] {
            let (_dir, app) = test_app().await;
            let (_, headers) = send(app, axum::http::Method::GET, path, None).await;
            assert_eq!(
                headers
                    .get(header::X_CONTENT_TYPE_OPTIONS)
                    .map(|value| value.to_str().expect("header")),
                Some("nosniff"),
                "{path} is missing X-Content-Type-Options (spec/01 §10.6)"
            );
            let csp = headers
                .get(header::CONTENT_SECURITY_POLICY)
                .map(|value| value.to_str().expect("header"))
                .unwrap_or_default();
            assert!(
                csp.contains("default-src 'self'"),
                "{path} is missing the CSP (got {csp:?})"
            );
        }
    }

    /// Sends one GET and returns the status, headers and body text.
    async fn get(
        app: Router,
        path: &str,
    ) -> (axum::http::StatusCode, axum::http::HeaderMap, String) {
        let request = Request::builder()
            .method(axum::http::Method::GET)
            .uri(path)
            .header(header::HOST, "127.0.0.1:7878");
        let response = app
            .oneshot(request.body(Body::empty()).expect("request"))
            .await
            .expect("response");
        let status = response.status();
        let headers = response.headers().clone();
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body");
        (status, headers, String::from_utf8_lossy(&body).into_owned())
    }

    /// Builds an app whose `--web-dir` points at a temporary directory holding `files`.
    async fn test_app_with_web_dir(
        files: &[(&str, &str)],
    ) -> (tempfile::TempDir, tempfile::TempDir, Router) {
        let assets = tempfile::tempdir().expect("assets dir");
        for (path, body) in files {
            let full = assets.path().join(path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).expect("asset dir");
            }
            std::fs::write(full, body).expect("write asset");
        }
        let data = tempfile::tempdir().expect("data dir");
        let db = crate::db::Db::connect(&data.path().join("test.db"))
            .await
            .expect("db");
        nekowriter_migration::up(db.conn()).await.expect("migrate");
        let vec_version = db.vec_version().await.expect("vec");
        let mut config = test_config(data.path());
        config.web_dir = Some(assets.path().to_path_buf());
        let state = AppState::new(db, config, vec_version);
        (data, assets, build_app(state))
    }

    const SHELL: &str = "<!doctype html><html><body><div id=\"root\"></div></body></html>";

    /// spec/01 §12: `--web-dir` serves the built SPA, unknown non-`/api` paths fall back to
    /// `index.html` (so a deep link reloads), and `/api` paths are never rewritten.
    #[tokio::test]
    async fn web_dir_serves_the_spa_with_fallback() {
        let (_data, _assets, app) = test_app_with_web_dir(&[
            ("index.html", SHELL),
            ("assets/app-abc123.js", "console.log(1)"),
            ("assets/app-abc123.css", "body{color:red}"),
        ])
        .await;

        let (status, headers, body) = get(app.clone(), "/").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(body, SHELL);
        assert_eq!(
            headers
                .get(header::CONTENT_TYPE)
                .map(|value| value.to_str().expect("header")),
            Some("text/html; charset=utf-8")
        );
        assert_eq!(
            headers
                .get(header::CACHE_CONTROL)
                .map(|value| value.to_str().expect("header")),
            Some("no-cache"),
            "the shell must revalidate so a rebuild is picked up"
        );

        // A deep link (the React Router route) has no file of its own.
        let (status, _, body) = get(app.clone(), "/books/1/chapters/2").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(body, SHELL);

        // A hashed asset is served as itself, with its type and an immutable cache policy.
        let (status, headers, body) = get(app.clone(), "/assets/app-abc123.js").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(body, "console.log(1)");
        assert_eq!(
            headers
                .get(header::CONTENT_TYPE)
                .map(|value| value.to_str().expect("header")),
            Some("text/javascript; charset=utf-8")
        );
        assert!(
            headers
                .get(header::CACHE_CONTROL)
                .map(|value| value.to_str().expect("header"))
                .unwrap_or_default()
                .contains("immutable")
        );

        let (status, headers, _) = get(app.clone(), "/assets/app-abc123.css").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(
            headers
                .get(header::CONTENT_TYPE)
                .map(|value| value.to_str().expect("header")),
            Some("text/css; charset=utf-8")
        );

        // The security headers cover static files too (spec/01 §10.6).
        let (_, headers, _) = get(app.clone(), "/assets/app-abc123.js").await;
        assert_eq!(
            headers
                .get(header::X_CONTENT_TYPE_OPTIONS)
                .map(|value| value.to_str().expect("header")),
            Some("nosniff")
        );

        // Traversal cannot escape the asset directory: the path is rejected and the request gets
        // the ordinary SPA fallback rather than a file outside the web dir.
        let (status, _, body) = get(app.clone(), "/../Cargo.toml").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(body, SHELL);
        assert!(!body.contains("[package]"));

        // An unknown `/api` path keeps the JSON error contract and is not turned into the shell.
        let (status, headers, body) = get(app.clone(), "/api/definitely-not-a-route").await;
        assert_eq!(status, axum::http::StatusCode::NOT_FOUND);
        assert_eq!(
            headers
                .get(header::CONTENT_TYPE)
                .map(|value| value.to_str().expect("header")),
            Some("application/json")
        );
        let envelope: serde_json::Value = serde_json::from_str(&body).expect("error envelope");
        assert_eq!(envelope["error"]["code"], "not_found");

        // A non-GET request to a page path is not a page request.
        let (status, _) = send(app, axum::http::Method::POST, "/books/1/chapters/2", None).await;
        assert_eq!(status, axum::http::StatusCode::METHOD_NOT_ALLOWED);
    }

    /// Without `--web-dir` and without the `embed-web` feature, `/` is the hint page and other
    /// paths are 404s (spec/01 §12).
    #[cfg(not(feature = "embed-web"))]
    #[tokio::test]
    async fn without_assets_the_root_is_a_hint_page() {
        let (_dir, app) = test_app().await;
        let (status, headers, body) = get(app.clone(), "/").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            body.contains("pnpm --dir web dev"),
            "the hint page must tell the reader how to start Vite"
        );
        assert_eq!(
            headers
                .get(header::CONTENT_TYPE)
                .map(|value| value.to_str().expect("header")),
            Some("text/html; charset=utf-8")
        );
        let (status, _, _) = get(app, "/books/1").await;
        assert_eq!(status, axum::http::StatusCode::NOT_FOUND);
    }

    /// The release binary embeds `web/dist` and serves it with the SPA fallback (spec/01 §12).
    #[cfg(feature = "embed-web")]
    #[tokio::test]
    async fn embedded_assets_serve_the_shell_and_deep_links() {
        let (_dir, app) = test_app().await;
        for path in ["/", "/books/1/chapters/2"] {
            let (status, headers, body) = get(app.clone(), path).await;
            assert_eq!(status, axum::http::StatusCode::OK, "{path}");
            assert_eq!(
                headers
                    .get(header::CONTENT_TYPE)
                    .map(|value| value.to_str().expect("header")),
                Some("text/html; charset=utf-8"),
                "{path}"
            );
            assert!(
                body.contains("<div id=\"root\""),
                "{path} is not the SPA shell"
            );
        }
    }
}
