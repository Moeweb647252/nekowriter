//! API router assembly, the host/origin guard and the root hint page (spec/01 §10).

pub mod app_settings;
pub mod books;
pub mod changes;
pub mod chapters;
#[cfg(debug_assertions)]
pub mod debug;
pub mod dto;
pub mod events;
pub mod health;
pub mod prompts;
pub mod providers;
pub mod sessions;
pub mod settings_materials;
pub mod tracks;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderValue, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, patch, post, put};

use crate::error::{AppError, ErrorCode};
use crate::state::AppState;

/// Hosts accepted by the guard: the API is loopback-only (spec/01 §10).
const ALLOWED_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "[::1]"];

/// Builds the whole application router. Tests call this directly.
pub fn build_app(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health::health))
        .route(
            "/app-settings",
            get(app_settings::get).patch(app_settings::patch),
        )
        .route("/books", get(books::list).post(books::create))
        .route(
            "/books/{book_id}",
            get(books::get).patch(books::patch).delete(books::delete),
        )
        .route("/books/{book_id}/overview", get(books::overview))
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
        .route("/chapters/{chapter_id}/submit", post(chapters::submit))
        .route("/chapters/{chapter_id}/approve", post(chapters::approve))
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
        .route("/", get(root_hint))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            host_origin_guard,
        ))
        // Outermost, so the headers are also present on the guard's 403 and on 404s
        // (spec/01 §10.6).
        .layer(middleware::from_fn(security_headers))
        .with_state(state)
}

/// Rejects requests whose `Host` is not a loopback name, and non-GET requests whose `Origin` is
/// not a loopback origin (DNS-rebinding and CSRF guard, spec/01 §10.2).
async fn host_origin_guard(request: Request, next: Next) -> Result<Response, AppError> {
    let headers = request.headers();
    let method = request.method().clone();

    match headers.get(header::HOST) {
        Some(host) if host_allowed(host) => {}
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

/// `GET /` when no SPA assets are configured: a plain page telling the reader to start Vite
/// (spec/01 §12).
async fn root_hint() -> Response {
    Html(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <title>NekoWriter</title></head><body style=\"font-family: sans-serif; margin: 3rem\">\
         <h1>NekoWriter API is running</h1>\
         <p>Run the Vite dev server to use the studio:</p>\
         <pre>pnpm --dir web dev</pre>\
         <p>Then open <a href=\"http://localhost:5173\">http://localhost:5173</a>.</p>\
         <p>Health: <a href=\"/api/health\">/api/health</a></p>\
         </body></html>",
    )
    .into_response()
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

    async fn test_app() -> (tempfile::TempDir, Router) {
        let dir = tempfile::tempdir().expect("temp dir");
        let db = crate::db::Db::connect(&dir.path().join("test.db"))
            .await
            .expect("db");
        nekowriter_migration::up(db.conn()).await.expect("migrate");
        let vec_version = db.vec_version().await.expect("vec");
        let state = AppState::new(db, test_config(dir.path()), vec_version);
        (dir, build_app(state))
    }

    /// Sends one request through the app and returns its status and headers.
    async fn send(
        app: Router,
        method: axum::http::Method,
        path: &str,
        origin: Option<&str>,
    ) -> (axum::http::StatusCode, axum::http::HeaderMap) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "127.0.0.1:7878");
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
}
