//! 写请求的信任边界：`Host` 必须是本机回环地址，带 `Origin` 时必须来自本机允许的来源。
//!
//! 从 `app/src/main.rs` 整块搬来（`docs/08` 的 P1.5），代码一行未改，只有可见性从私有放宽到
//! `pub(crate)`（`http::router` 要把 `require_trusted_write` 挂成中间件，`main` 要算一次允许
//! 来源）。`VITE_ORIGINS` 只被本模块使用，因此一并搬来，`main.rs` 不再保留它。

use std::collections::HashSet;

use axum::{
    extract::{Request, State},
    http::{HeaderMap, Method, header},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::http::error::ApiError;
use crate::http::state::AppState;

const VITE_ORIGINS: [&str; 2] = ["http://127.0.0.1:5173", "http://localhost:5173"];

/// 写请求的信任边界：`Host` 必须是本机回环地址，带 `Origin` 时必须来自本机允许的来源。
///
/// 它挂成一层中间件，而不是由每个 handler 记得调用：新增写端点时不会漏掉这条校验。
pub(crate) async fn require_trusted_write(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let is_write = !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    if is_write && let Err(error) = ensure_trusted_write_request(request.headers(), &state) {
        return error.into_response();
    }
    next.run(request).await
}

fn ensure_trusted_write_request(headers: &HeaderMap, state: &AppState) -> Result<(), ApiError> {
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .ok_or(ApiError::UntrustedHost)?;
    if !is_loopback_host(host) {
        return Err(ApiError::UntrustedHost);
    }

    if let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        && !state.allowed_write_origins.contains(origin)
    {
        return Err(ApiError::UntrustedOrigin);
    }
    Ok(())
}

pub(crate) fn is_loopback_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host == "localhost"
        || host.starts_with("localhost:")
        || host == "127.0.0.1"
        || host.starts_with("127.0.0.1:")
        || host == "[::1]"
        || host.starts_with("[::1]:")
}

pub(crate) fn allowed_write_origins(port: u16) -> HashSet<String> {
    let mut origins = HashSet::from([
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
    ]);
    origins.extend(VITE_ORIGINS.map(str::to_owned));
    origins
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use nekowriter_agent::engine::ACCUMULATION_ENGINES;
    use serde_json::{Value, json};
    use uuid::Uuid;

    use crate::http::tests::{accumulation_app, spawn_app};

    use super::*;

    #[test]
    fn loopback_host_validation_rejects_external_hosts() {
        assert!(is_loopback_host("127.0.0.1:47821"));
        assert!(is_loopback_host("[::1]:47821"));
        assert!(!is_loopback_host("nekowriter.example"));
        assert!(!is_loopback_host("127.0.0.10"));
    }

    /// 写请求的信任校验挂在路由层，而不是由每个 handler 自己记得调用。
    ///
    /// 这里用一个 handler 自己不做校验的路由：它只能被中间件拦下。
    #[tokio::test]
    async fn write_trust_is_enforced_by_the_router_not_by_handlers() {
        let (_dir, state) = accumulation_app(ACCUMULATION_ENGINES).await;
        let base = spawn_app(state);
        let client = reqwest::Client::new();

        let blocked = client
            .post(format!(
                "{base}/api/recycle-bin/knowledge_entry/{}/purge",
                Uuid::new_v4()
            ))
            .header("origin", "http://evil.example")
            .json(&json!({ "commandId": Uuid::new_v4() }))
            .send()
            .await
            .unwrap();
        assert_eq!(blocked.status(), StatusCode::FORBIDDEN);
        let body: Value = blocked.json().await.unwrap();
        assert_eq!(body["error"]["code"], "UNTRUSTED_ORIGIN");

        // `Host` 不是回环地址时也拦下：DNS 重绑定式的请求到不了 handler。
        let rebound = client
            .post(format!("{base}/api/settings"))
            .header("host", "evil.example")
            .json(&json!({ "commandId": Uuid::new_v4() }))
            .send()
            .await
            .unwrap();
        // 400 是这个错误码既有的状态码映射（`ApiError::UntrustedHost`），这里只钉住拦截本身。
        assert_eq!(rebound.status(), StatusCode::BAD_REQUEST);
        let body: Value = rebound.json().await.unwrap();
        assert_eq!(body["error"]["code"], "UNTRUSTED_HOST");

        // 读方法不经过这层：同一个不被信任的来源上 GET 仍然可用。
        let read = client
            .get(format!("{base}/api/settings"))
            .header("origin", "http://evil.example")
            .send()
            .await
            .unwrap();
        assert_eq!(read.status(), StatusCode::OK);
    }
}
