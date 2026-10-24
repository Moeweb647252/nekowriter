//! API router assembly, the host/origin guard and the root hint page (spec/01 §10).

pub mod app_settings;
pub mod books;
pub mod changes;
pub mod chapters;
pub mod dto;
pub mod events;
pub mod health;
pub mod prompts;
pub mod providers;
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
        .with_state(state.clone());

    Router::new()
        .nest("/api", api)
        .route("/", get(root_hint))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            host_origin_guard,
        ))
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

    if method != axum::http::Method::GET
        && method != axum::http::Method::HEAD
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
