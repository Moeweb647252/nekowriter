//! API router assembly, the host/origin guard and the root hint page (spec/01 §10).

pub mod dto;
pub mod health;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderValue, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;

use crate::error::{AppError, ErrorCode};
use crate::state::AppState;

/// Hosts accepted by the guard: the API is loopback-only (spec/01 §10).
const ALLOWED_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "[::1]"];

/// Builds the whole application router. Tests call this directly.
pub fn build_app(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health::health))
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
