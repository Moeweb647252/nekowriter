//! API-level tests for the health endpoint and the host/origin guard (PLAN M0.2).

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use nekowriter::api::build_app;
use nekowriter::config::Config;
use nekowriter::db::Db;
use nekowriter::state::AppState;
use tower::ServiceExt;

fn test_config(data_dir: &std::path::Path) -> Config {
    Config {
        bind: "127.0.0.1:0".parse().expect("bind"),
        data_dir: Some(data_dir.to_path_buf()),
        web_dir: None,
        enable_demo_provider: true,
        allow_remote: false,
    }
}

async fn test_app() -> (tempfile::TempDir, axum::Router) {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Db::connect(&dir.path().join("test.db")).await.expect("db");
    nekowriter_migration::up(db.conn()).await.expect("migrate");
    let vec_version = db.vec_version().await.expect("vec");
    let state = AppState::new(db, test_config(dir.path()), vec_version);
    (dir, build_app(state))
}

#[tokio::test]
async fn health_returns_ok() {
    let (_dir, app) = test_app().await;
    let response = app
        .oneshot(
            Request::get("/api/health")
                .header(header::HOST, "127.0.0.1:7878")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(json["ok"], true);
    assert!(
        !json["vec_version"]
            .as_str()
            .expect("vec version")
            .is_empty()
    );
}

#[tokio::test]
async fn foreign_host_is_forbidden() {
    let (_dir, app) = test_app().await;
    let response = app
        .oneshot(
            Request::get("/api/health")
                .header(header::HOST, "evil.com")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(json["error"]["code"], "host.forbidden");
}

#[tokio::test]
async fn foreign_origin_on_mutation_is_forbidden() {
    let (_dir, app) = test_app().await;
    let response = app
        .oneshot(
            Request::post("/api/health")
                .header(header::HOST, "127.0.0.1:7878")
                .header(header::ORIGIN, "http://evil.com")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
