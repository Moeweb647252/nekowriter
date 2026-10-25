//! `web_search` provider tests (PLAN 8.1, spec/07 §6, spec/10 §2.1, D-058, D-127).
//!
//! Every provider runs against `wiremock`: the Tavily and DeepSeek hosts are overridden through
//! the test-only `WebService::with_*_base_url` seams (both production addresses are fixed by the
//! spec), and SearXNG takes an author-supplied base URL, which is the `wiremock` server itself. No
//! real host is contacted.
//!
//! The harness is local because `TestApp` has no seam for replacing the web service, and adding one
//! would edit a file another task owns.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use nekowriter::config::Config;
use nekowriter::db::Db;
use nekowriter::events::EventBus;
use nekowriter::state::AppState;
use nekowriter::testing::assert_error;
use nekowriter::web::WebService;
use nekowriter::web::search::{self, SearchConfig, SearchFailure, SearchProviderKind};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::{bearer_token, body_string_contains, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A migrated temporary database and the app built over it, with a replaceable web service.
struct Harness {
    _dir: tempfile::TempDir,
    app: Router,
}

impl Harness {
    /// A fresh app whose Tavily provider calls `tavily_base` when given.
    async fn new(tavily_base: Option<String>) -> Self {
        Self::build(tavily_base, None).await
    }

    /// A fresh app whose DeepSeek provider calls `deepseek_base` when given.
    async fn with_deepseek(deepseek_base: Option<String>) -> Self {
        Self::build(None, deepseek_base).await
    }

    async fn build(tavily_base: Option<String>, deepseek_base: Option<String>) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let db = Db::connect(&dir.path().join("test.db"))
            .await
            .expect("connect");
        nekowriter_migration::up(db.conn()).await.expect("migrate");
        let vec_version = db.vec_version().await.expect("vec_version");
        let config = Config {
            bind: "127.0.0.1:0".parse().expect("bind"),
            data_dir: Some(dir.path().to_path_buf()),
            web_dir: None,
            enable_demo_provider: true,
            allow_remote: false,
        };
        let events = EventBus::new();
        let mut web = WebService::new();
        if let Some(base) = tavily_base {
            web = web.with_tavily_base_url(base);
        }
        if let Some(base) = deepseek_base {
            web = web.with_deepseek_base_url(base);
        }
        let state = AppState::new(db, config, vec_version)
            .with_events(events)
            .with_web(web);
        let app = nekowriter::api::build_app(state);
        Self { _dir: dir, app }
    }

    async fn request(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let builder = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "127.0.0.1:7878");
        let request = match body {
            Some(body) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
            None => builder.body(Body::empty()).expect("request"),
        };
        let response = self.app.clone().oneshot(request).await.expect("response");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
            .await
            .expect("body");
        let json = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, json)
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.request("GET", path, None).await
    }

    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.request("POST", path, Some(body)).await
    }

    async fn patch(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.request("PATCH", path, Some(body)).await
    }
}

// ───────────────────────────── helpers ─────────────────────────────

/// A harness whose Tavily provider calls `server`.
async fn app_with_tavily(server: &MockServer) -> Harness {
    Harness::new(Some(server.uri())).await
}

/// Saves a Tavily configuration pointing at the default (fixed) host.
async fn configure_tavily(app: &Harness, key: &str) {
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "search": { "provider": "tavily", "tavily_api_key": key } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "configure tavily: {body}");
}

/// Saves a SearXNG configuration pointing at `base_url`.
async fn configure_searxng(app: &Harness, base_url: &str) {
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "search": { "provider": "searxng", "searxng_base_url": base_url } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "configure searxng: {body}");
}

/// The provider configuration the service layer builds, for the direct provider tests.
fn config(server: &MockServer) -> SearchConfig {
    SearchConfig {
        provider: SearchProviderKind::Searxng,
        searxng_base_url: Some(server.uri()),
        ..SearchConfig::default()
    }
}

/// A harness whose DeepSeek provider calls `server`.
async fn app_with_deepseek(server: &MockServer) -> Harness {
    Harness::with_deepseek(Some(server.uri())).await
}

/// Saves a DeepSeek configuration.
async fn configure_deepseek(app: &Harness, key: &str) {
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "search": { "provider": "deepseek", "deepseek_api_key": key } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "configure deepseek: {body}");
}

fn tavily_response() -> Value {
    json!({
        "query": "song dynasty tea house",
        "results": [
            {
                "title": "Tea houses of the Song dynasty",
                "url": "https://example.com/tea",
                "content": "Tea houses served as social hubs.",
                "published_date": "2020-05-01",
            },
            {
                "title": "City life in Kaifeng",
                "url": "https://example.com/kaifeng",
                "content": "The capital had a bustling night market.",
                "published_date": null,
            },
        ],
    })
}

fn searx_response() -> Value {
    json!({
        "results": [
            {
                "title": "Song tea culture",
                "url": "https://example.org/tea",
                "content": "Tea drinking became a daily ritual.",
                "publishedDate": "2019-03-02",
            },
            {
                "title": "Kaifeng markets",
                "url": "https://example.org/markets",
                "content": "Night markets were common.",
                "publishedDate": null,
            },
        ],
    })
}

/// DeepSeek's response carries both a short `snippet` (for a front-end) and a long `summary` (for
/// a model); the provider must feed the latter into the prompt (spec/07 §6, D-127).
const DEEPSEEK_SUMMARY_FIRST: &str = "Tea houses were social hubs where the Song literati \
     gathered; the summary is the field DeepSeek documents for a model to read.";
const DEEPSEEK_SNIPPET_FIRST: &str = "Tea houses as social hubs";
const DEEPSEEK_SUMMARY_SECOND: &str =
    "Kaifeng's night market ran past the curfew and shaped the capital's consumer culture.";

fn deepseek_response() -> Value {
    json!({
        "results": [
            {
                "rerank_score": 2.1,
                "title": "Tea houses of the Song dynasty",
                "url": "https://example.com/deepseek-tea",
                "snippet": DEEPSEEK_SNIPPET_FIRST,
                "summary": DEEPSEEK_SUMMARY_FIRST,
                "content": "",
                "site_name": "Example",
                "site_icon": null,
                "date": "2021-07-09T00:00:00Z",
                "authority": 3,
            },
            {
                "rerank_score": 1.4,
                "title": "City life in Kaifeng",
                "url": "https://example.com/deepseek-kaifeng",
                "snippet": "Kaifeng night market",
                "summary": DEEPSEEK_SUMMARY_SECOND,
                "content": "",
                "site_name": null,
                "site_icon": null,
                "date": null,
                "authority": -1,
            },
        ],
    })
}

// ───────────────────────────── Tavily ─────────────────────────────

#[tokio::test]
async fn tavily_sends_the_documented_request_and_maps_its_results() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .and(bearer_token("tvly-test-key"))
        .and(body_string_contains("\"search_depth\":\"basic\""))
        .and(body_string_contains("\"include_answer\":false"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tavily_response()))
        .expect(1)
        .mount(&server)
        .await;

    let app = app_with_tavily(&server).await;
    configure_tavily(&app, "tvly-test-key").await;

    let (status, body) = app
        .post(
            "/api/app-settings/search/test",
            json!({ "query": "song tea house" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "search test: {body}");
    let hits = body["hits"].as_array().expect("hits");
    assert_eq!(hits.len(), 2, "{body}");
    assert_eq!(hits[0]["title"], "Tea houses of the Song dynasty");
    assert_eq!(hits[0]["url"], "https://example.com/tea");
    assert_eq!(hits[0]["snippet"], "Tea houses served as social hubs.");
    assert_eq!(hits[0]["published"], "2020-05-01");
    // A null `published_date` maps to an absent field, not the string "null".
    assert!(hits[1].get("published").is_none(), "{body}");

    // The request body carried the query and the configured result count.
    let requests = server.received_requests().await.expect("requests");
    let sent: Value = serde_json::from_slice(&requests[0].body).expect("json body");
    assert_eq!(sent["query"], "song tea house");
    assert_eq!(sent["max_results"], 6, "the settings default");
}

#[tokio::test]
async fn a_rejected_tavily_key_is_provider_auth() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(401).set_body_string("{\"error\":\"invalid key\"}"))
        .mount(&server)
        .await;

    let app = app_with_tavily(&server).await;
    configure_tavily(&app, "tvly-wrong-key").await;
    let (status, body) = app
        .post(
            "/api/app-settings/search/test",
            json!({ "query": "anything" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert_error(&body, "provider.auth");
}

#[tokio::test]
async fn other_tavily_failures_are_provider_unavailable() {
    for code in [500u16, 429, 404, 200] {
        let server = MockServer::start().await;
        let template = if code == 200 {
            // A 200 that is not the documented JSON shape.
            ResponseTemplate::new(200).set_body_string("not json")
        } else {
            ResponseTemplate::new(code).set_body_string("boom")
        };
        Mock::given(method("POST"))
            .and(path("/search"))
            .respond_with(template)
            .mount(&server)
            .await;

        let app = app_with_tavily(&server).await;
        configure_tavily(&app, "tvly-test-key").await;
        let (status, body) = app
            .post(
                "/api/app-settings/search/test",
                json!({ "query": "anything" }),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{code}: {body}");
        assert_error(&body, "provider.unavailable");
    }
}

// ───────────────────────────── DeepSeek ─────────────────────────────

/// The provider configuration for the direct DeepSeek provider tests.
fn deepseek_config(server: &MockServer) -> SearchConfig {
    SearchConfig {
        provider: SearchProviderKind::Deepseek,
        deepseek_api_key: Some("sk-deepseek-test".to_owned()),
        deepseek_base_url: server.uri(),
        ..SearchConfig::default()
    }
}

#[tokio::test]
async fn deepseek_sends_the_documented_request_and_maps_its_results() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/tools/search"))
        .and(bearer_token("sk-deepseek-test"))
        .and(body_string_contains("\"need_content\":false"))
        .respond_with(ResponseTemplate::new(200).set_body_json(deepseek_response()))
        .expect(1)
        .mount(&server)
        .await;

    let app = app_with_deepseek(&server).await;
    configure_deepseek(&app, "sk-deepseek-test").await;

    let (status, body) = app
        .post(
            "/api/app-settings/search/test",
            json!({ "query": "song tea house" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "search test: {body}");
    let hits = body["hits"].as_array().expect("hits");
    assert_eq!(hits.len(), 2, "{body}");
    assert_eq!(hits[0]["title"], "Tea houses of the Song dynasty");
    assert_eq!(hits[0]["url"], "https://example.com/deepseek-tea");
    // `SearchHit.snippet` is rendered into the model prompt, so it must carry DeepSeek's
    // `summary` (documented for a model), never its `snippet` (documented for a front-end).
    assert_eq!(hits[0]["snippet"], DEEPSEEK_SUMMARY_FIRST);
    assert_ne!(hits[0]["snippet"], DEEPSEEK_SNIPPET_FIRST);
    assert_eq!(hits[0]["published"], "2021-07-09T00:00:00Z");
    assert_eq!(hits[1]["snippet"], DEEPSEEK_SUMMARY_SECOND);
    // A null `date` maps to an absent field, not the string "null".
    assert!(hits[1].get("published").is_none(), "{body}");

    let requests = server.received_requests().await.expect("requests");
    assert_eq!(requests[0].url.path(), "/v1/tools/search");
    let sent: Value = serde_json::from_slice(&requests[0].body).expect("json body");
    assert_eq!(sent["query"], "song tea house");
    assert_eq!(sent["count"], 6, "the settings default");
    assert_eq!(sent["need_content"], false);
}

#[tokio::test]
async fn a_rejected_deepseek_key_is_provider_auth() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/tools/search"))
        .respond_with(ResponseTemplate::new(401).set_body_string("{\"error\":\"invalid key\"}"))
        .mount(&server)
        .await;

    let app = app_with_deepseek(&server).await;
    configure_deepseek(&app, "sk-deepseek-wrong-key").await;
    let (status, body) = app
        .post(
            "/api/app-settings/search/test",
            json!({ "query": "anything" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert_error(&body, "provider.auth");
}

#[tokio::test]
async fn other_deepseek_failures_are_provider_unavailable() {
    for code in [500u16, 429, 402] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/tools/search"))
            .respond_with(ResponseTemplate::new(code).set_body_string("boom"))
            .mount(&server)
            .await;

        let app = app_with_deepseek(&server).await;
        configure_deepseek(&app, "sk-deepseek-test").await;
        let (status, body) = app
            .post(
                "/api/app-settings/search/test",
                json!({ "query": "anything" }),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{code}: {body}");
        assert_error(&body, "provider.unavailable");
    }
}

#[tokio::test]
async fn a_rejected_deepseek_request_is_status_402_and_429() {
    let client = search::client().expect("client");
    for code in [402u16, 429] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/tools/search"))
            .respond_with(ResponseTemplate::new(code).set_body_string("rejected"))
            .mount(&server)
            .await;
        let error = search::search(&client, &deepseek_config(&server), "q", 5)
            .await
            .expect_err("the request is rejected");
        // The failure is a `status` failure that carries the number, not `auth` or `network`.
        assert_eq!(error.reason, SearchFailure::Status, "{code}");
        assert_eq!(error.status, Some(code), "{code}");
    }
}

#[test]
fn deepseek_is_not_configured_without_a_key() {
    let keyless = SearchConfig {
        provider: SearchProviderKind::Deepseek,
        ..SearchConfig::default()
    };
    assert!(!search::is_configured(&keyless));
    let empty = SearchConfig {
        deepseek_api_key: Some(String::new()),
        ..keyless.clone()
    };
    assert!(!search::is_configured(&empty));
    let keyed = SearchConfig {
        deepseek_api_key: Some("sk-deepseek-test".to_owned()),
        ..keyless
    };
    assert!(search::is_configured(&keyed));
}

#[tokio::test]
async fn a_deepseek_key_from_the_environment_is_read_at_call_time() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/tools/search"))
        .and(bearer_token("sk-deepseek-from-env"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "results": [] })))
        .expect(1)
        .mount(&server)
        .await;

    // Safety: the variable name is unique to this test, so no other test reads it.
    unsafe {
        std::env::set_var("NEKOWRITER_M8_DEEPSEEK_KEY", "sk-deepseek-from-env");
    }
    let app = app_with_deepseek(&server).await;
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "search": {
                "provider": "deepseek",
                "deepseek_api_key": "env:NEKOWRITER_M8_DEEPSEEK_KEY",
            } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "configure: {body}");
    // `env:NAME` is not a secret, so the DTO keeps it readable (spec/01 §10.3).
    assert_eq!(
        body["search"]["deepseek_api_key"],
        "env:NEKOWRITER_M8_DEEPSEEK_KEY"
    );

    let (status, body) = app
        .post("/api/app-settings/search/test", json!({ "query": "tea" }))
        .await;
    assert_eq!(status, StatusCode::OK, "search test: {body}");
    unsafe {
        std::env::remove_var("NEKOWRITER_M8_DEEPSEEK_KEY");
    }
}

#[tokio::test]
async fn deepseek_api_key_is_masked_and_a_round_tripped_mask_keeps_it() {
    const SECRET: &str = "sk-deepseek-secret-abcd";
    let server = MockServer::start().await;
    // The stored key survives the masked write-back only if the provider still sends it.
    Mock::given(method("POST"))
        .and(path("/v1/tools/search"))
        .and(bearer_token(SECRET))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "results": [] })))
        .expect(1)
        .mount(&server)
        .await;

    let app = app_with_deepseek(&server).await;
    configure_deepseek(&app, SECRET).await;

    let (status, body) = app.get("/api/app-settings").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["search"]["deepseek_api_key"], "…abcd");
    assert!(!body.to_string().contains(SECRET));

    // The UI round-trips the masked value: it must mean "unchanged", not a new key.
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "search": { "deepseek_api_key": "…abcd", "max_results": 8 } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["search"]["deepseek_api_key"], "…abcd");
    assert_eq!(body["search"]["provider"], "deepseek");
    assert_eq!(body["search"]["max_results"], 8);
    assert!(!body.to_string().contains(SECRET));

    // A search test now succeeds only because the stored secret was kept.
    let (status, body) = app
        .post("/api/app-settings/search/test", json!({ "query": "tea" }))
        .await;
    assert_eq!(status, StatusCode::OK, "search test: {body}");
}

#[tokio::test]
async fn a_deepseek_provider_without_a_key_is_rejected() {
    let app = Harness::with_deepseek(None).await;
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "search": { "provider": "deepseek" } }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_error(&body, "validation.failed");
    assert!(body["error"].is_object(), "{body}");
    // Nothing was stored: an unset provider remains the selected one.
    let (_, settings) = app.get("/api/app-settings").await;
    assert_eq!(settings["search"]["provider"], "none");
}

#[test]
fn the_deepseek_base_url_seam_is_test_only() {
    // Production `WebService::new()` always calls the documented host: the field, the builder and
    // the getter's override arm exist only under `cfg(any(test, feature = "testing"))`, so a
    // production build has no setting, environment variable or API field that can change it.
    assert_eq!(
        WebService::new().deepseek_base_url(),
        search::DEEPSEEK_BASE_URL
    );
    assert_eq!(
        WebService::new().deepseek_base_url(),
        "https://api.deepseek.com"
    );
    let overridden = WebService::new().with_deepseek_base_url("http://wiremock.invalid");
    assert_eq!(overridden.deepseek_base_url(), "http://wiremock.invalid");
    assert_eq!(
        SearchConfig::default().deepseek_base_url,
        search::DEEPSEEK_BASE_URL
    );
}

// ───────────────────────────── SearXNG ─────────────────────────────

#[tokio::test]
async fn searxng_sends_the_documented_query_and_maps_its_results() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .and(query_param("q", "宋代 茶馆"))
        .and(query_param("format", "json"))
        .and(query_param("safesearch", "0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(searx_response()))
        .expect(1)
        .mount(&server)
        .await;

    let app = Harness::new(None).await;
    configure_searxng(&app, &server.uri()).await;
    let (status, body) = app
        .post(
            "/api/app-settings/search/test",
            json!({ "query": "宋代 茶馆" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "search test: {body}");
    let hits = body["hits"].as_array().expect("hits");
    assert_eq!(hits.len(), 2, "{body}");
    assert_eq!(hits[0]["title"], "Song tea culture");
    assert_eq!(hits[0]["snippet"], "Tea drinking became a daily ritual.");
    assert_eq!(hits[0]["published"], "2019-03-02");
    assert!(hits[1].get("published").is_none(), "{body}");
}

#[tokio::test]
async fn a_rejected_searxng_key_is_provider_auth() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(403).set_body_string("forbidden"))
        .mount(&server)
        .await;
    let app = Harness::new(None).await;
    configure_searxng(&app, &server.uri()).await;
    let (status, body) = app
        .post("/api/app-settings/search/test", json!({ "query": "x" }))
        .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert_error(&body, "provider.auth");
}

#[tokio::test]
async fn a_search_test_without_a_provider_is_search_not_configured() {
    let app = Harness::new(None).await;
    let (status, body) = app
        .post("/api/app-settings/search/test", json!({ "query": "x" }))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_error(&body, "search.not_configured");
    // Choosing a provider but never saving its key also counts as unconfigured.
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "search": { "provider": "none" } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = app
        .post("/api/app-settings/search/test", json!({ "query": "x" }))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_error(&body, "search.not_configured");
}

#[tokio::test]
async fn an_empty_test_query_is_validation_failed() {
    let server = MockServer::start().await;
    let app = Harness::new(None).await;
    configure_searxng(&app, &server.uri()).await;
    let (status, body) = app
        .post("/api/app-settings/search/test", json!({ "query": "   " }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_error(&body, "validation.failed");
}

// ───────────────────────────── the provider layer ─────────────────────────────

#[tokio::test]
async fn results_are_deduplicated_by_url_and_capped() {
    let server = MockServer::start().await;
    let body = json!({
        "results": [
            { "title": "one", "url": "https://dup.example/a", "content": "a" },
            { "title": "two", "url": "https://dup.example/a", "content": "b" },
            { "title": "three", "url": "https://dup.example/c", "content": "c" },
            { "title": "four", "url": "https://dup.example/d", "content": "d" },
        ],
    });
    Mock::given(method("GET"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let client = search::client().expect("client");
    let hits = search::search(&client, &config(&server), "anything", 2)
        .await
        .expect("hits");
    assert_eq!(hits.len(), 2, "capped at the requested count");
    assert_eq!(hits[0].url, "https://dup.example/a");
    assert_eq!(hits[0].title, "one", "the first occurrence wins");
    assert_eq!(hits[1].url, "https://dup.example/c");

    let all = search::search(&client, &config(&server), "anything", 10)
        .await
        .expect("hits");
    assert_eq!(all.len(), 3, "the duplicate is dropped");
}

#[tokio::test]
async fn provider_failures_carry_the_documented_reason_codes() {
    let client = search::client().expect("client");

    // A status failure carries the status, and it is not `auth`.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(503).set_body_string("unavailable"))
        .mount(&server)
        .await;
    let error = search::search(&client, &config(&server), "q", 5)
        .await
        .expect_err("503");
    assert_eq!(error.reason, SearchFailure::Status);
    assert_eq!(error.status, Some(503));

    // A refused connection is a network failure. A port that was bound and released refuses
    // reliably on loopback.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let refused = format!("http://{}", listener.local_addr().expect("addr"));
    drop(listener);
    let config = SearchConfig {
        provider: SearchProviderKind::Searxng,
        searxng_base_url: Some(refused),
        ..SearchConfig::default()
    };
    let error = search::search(&client, &config, "q", 5)
        .await
        .expect_err("refused");
    assert_eq!(error.reason, SearchFailure::Network);
    assert_eq!(error.status, None);
}

#[tokio::test]
async fn a_tavily_key_from_the_environment_is_read_at_call_time() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .and(bearer_token("tvly-from-env"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tavily_response()))
        .expect(1)
        .mount(&server)
        .await;

    // Safety: the variable name is unique to this test, so no other test reads it.
    unsafe {
        std::env::set_var("NEKOWRITER_M8_TAVILY_KEY", "tvly-from-env");
    }
    let app = app_with_tavily(&server).await;
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "search": { "provider": "tavily", "tavily_api_key": "env:NEKOWRITER_M8_TAVILY_KEY" } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "configure: {body}");
    // `env:NAME` is not a secret, so the DTO keeps it readable (spec/01 §10.3).
    assert_eq!(
        body["search"]["tavily_api_key"],
        "env:NEKOWRITER_M8_TAVILY_KEY"
    );

    let (status, body) = app
        .post("/api/app-settings/search/test", json!({ "query": "tea" }))
        .await;
    assert_eq!(status, StatusCode::OK, "search test: {body}");
    unsafe {
        std::env::remove_var("NEKOWRITER_M8_TAVILY_KEY");
    }
}

#[tokio::test]
async fn an_unset_environment_key_makes_the_test_provider_auth() {
    let app = Harness::new(None).await;
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "search": { "provider": "tavily", "tavily_api_key": "env:NEKOWRITER_M8_ABSENT_KEY" } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "configure: {body}");
    let (status, body) = app
        .post("/api/app-settings/search/test", json!({ "query": "tea" }))
        .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert_error(&body, "provider.auth");
}

// ───────────────────────────── settings shape ─────────────────────────────

#[tokio::test]
async fn the_search_settings_shape_follows_spec_02() {
    let app = Harness::new(None).await;
    let (_, settings) = app.get("/api/app-settings").await;
    assert_eq!(settings["search"]["provider"], "none");
    assert_eq!(settings["search"]["max_results"], 6);
    assert!(settings["search"].get("tavily_api_key").is_none());

    // The stored key is masked like a provider key, and a round-tripped mask keeps it.
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "search": { "provider": "tavily", "tavily_api_key": "tvly-secret-abcd" } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["search"]["tavily_api_key"], "…abcd");
    assert!(!body.to_string().contains("tvly-secret-abcd"));
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "search": { "tavily_api_key": "…abcd", "max_results": 10 } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["search"]["tavily_api_key"], "…abcd");
    assert_eq!(body["search"]["max_results"], 10);

    // The default is capped at the tool's maximum (spec/05 §7).
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "search": { "max_results": 11 } }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_error(&body, "validation.failed");
}
