//! The trust gate: the service answers only requests that come from this machine.
//!
//! - Every request must name a loopback host in its `Host` header. This defeats DNS
//!   rebinding, where a web page on another domain resolves its name to 127.0.0.1 to read
//!   local data.
//! - A request that changes data and carries an `Origin` must come from the web UI: this
//!   server itself or the Vite development server. This stops other web pages from
//!   submitting commands.

use axum::{
    extract::{Request, State},
    http::{HeaderMap, Method, header},
    middleware::Next,
    response::{IntoResponse, Response},
};

use super::{
    AppState,
    error::{ApiError, ApiProblem},
};

/// The origins of the Vite development server, which proxies `/api` to this server.
const VITE_DEV_PORT: u16 = 5173;

pub(crate) async fn trust(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if !host_is_loopback(request.headers()) {
        return ApiError::new(ApiProblem::UntrustedHost, host_header(request.headers()))
            .into_response();
    }
    let reads_only = matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    if !reads_only
        && let Some(origin) = request.headers().get(header::ORIGIN)
        && !origin
            .to_str()
            .is_ok_and(|origin| origin_is_allowed(origin, state.port))
    {
        let origin = String::from_utf8_lossy(origin.as_bytes()).into_owned();
        return ApiError::new(ApiProblem::UntrustedOrigin, origin).into_response();
    }
    next.run(request).await
}

fn host_header(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::HOST)
        .map(|host| String::from_utf8_lossy(host.as_bytes()).into_owned())
}

fn host_is_loopback(headers: &HeaderMap) -> bool {
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|host| host.to_str().ok())
    else {
        return false;
    };
    let name = if let Some(rest) = host.strip_prefix('[') {
        rest.split_once(']').map_or("", |(name, _)| name)
    } else {
        host.split_once(':').map_or(host, |(name, _)| name)
    };
    matches!(name, "127.0.0.1" | "localhost" | "::1")
}

fn origin_is_allowed(origin: &str, port: u16) -> bool {
    [port, VITE_DEV_PORT].iter().any(|port| {
        [
            format!("http://127.0.0.1:{port}"),
            format!("http://localhost:{port}"),
            format!("http://[::1]:{port}"),
        ]
        .iter()
        .any(|allowed| allowed == origin)
    })
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn headers_with_host(host: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_str(host).unwrap());
        headers
    }

    #[test]
    fn loopback_hosts_with_any_port_are_trusted() {
        for host in [
            "127.0.0.1:47821",
            "localhost:5173",
            "[::1]:47821",
            "localhost",
        ] {
            assert!(host_is_loopback(&headers_with_host(host)), "{host}");
        }
    }

    #[test]
    fn other_hosts_are_not_trusted() {
        for host in ["evil.example:47821", "127.0.0.1.evil.example", "[::2]:1"] {
            assert!(!host_is_loopback(&headers_with_host(host)), "{host}");
        }
        assert!(!host_is_loopback(&HeaderMap::new()));
    }

    #[test]
    fn only_the_web_ui_origins_are_allowed() {
        assert!(origin_is_allowed("http://127.0.0.1:47821", 47821));
        assert!(origin_is_allowed("http://localhost:5173", 47821));
        assert!(!origin_is_allowed("http://localhost:47822", 47821));
        assert!(!origin_is_allowed("https://evil.example", 47821));
    }
}
