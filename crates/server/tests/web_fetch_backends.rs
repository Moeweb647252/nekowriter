//! `web_fetch` backend tests (spec/07 §7, spec/02 §4.1, D-128).
//!
//! The direct fetcher's guard has its own adversarial tests in `web_fetch_guard.rs`; this file
//! proves the *second* backend: DeepSeek Fetch runs against `wiremock` through the test-only base
//! URL seam, shares the page cache with the direct fetcher, and never weakens the direct guard.
//! No real host is contacted.

use std::net::SocketAddr;

use axum::http::StatusCode;
use nekowriter::api::dto::settings::SearchSettingsDto;
use nekowriter::testing::{TestApp, assert_error, assert_error_detail};
use nekowriter::web::fetch::{self, FetchError, FetchFailure, Fetcher};
use nekowriter::web::search::DEEPSEEK_BASE_URL;
use nekowriter::web::{FetchBackend, FetchConfig, WebService};
use serde_json::{Value, json};
use wiremock::matchers::{bearer_token, body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "sk-deepseek-test";

/// A resolved DeepSeek fetch configuration pointing at `server`.
fn deepseek_config(server: &MockServer) -> FetchConfig {
    FetchConfig {
        backend: FetchBackend::Deepseek,
        deepseek_api_key: Some(KEY.to_owned()),
        deepseek_base_url: server.uri(),
    }
}

/// A direct configuration: the default backend, no key.
fn direct_config() -> FetchConfig {
    FetchConfig {
        backend: FetchBackend::Direct,
        deepseek_api_key: None,
        deepseek_base_url: DEEPSEEK_BASE_URL.to_owned(),
    }
}

// ───────────────────────────── DeepSeek Fetch ─────────────────────────────

#[tokio::test]
async fn deepseek_fetch_sends_the_documented_request_and_maps_the_page() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/tools/fetch"))
        .and(bearer_token(KEY))
        .and(body_string_contains(
            "\"url\":\"https://example.com/article\"",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "title": "An article",
            "content": "# Heading\n\nBody text.",
        })))
        .expect(1)
        .mount(&server)
        .await;

    let web = WebService::new().with_deepseek_base_url(server.uri());
    let document = web
        .fetch_document(&deepseek_config(&server), "https://example.com/article")
        .await
        .expect("fetched page");
    assert_eq!(document.title, "An article");
    assert_eq!(document.markdown, "# Heading\n\nBody text.");
    // DeepSeek does not report the URL it ended on, so the requested URL is reported.
    assert_eq!(document.url, "https://example.com/article");
    assert!(!document.cached);

    let requests = server.received_requests().await.expect("requests");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.path(), "/v1/tools/fetch");
    let sent: Value = serde_json::from_slice(&requests[0].body).expect("json body");
    assert_eq!(sent, json!({ "url": "https://example.com/article" }));
    let content_type = requests[0]
        .headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(
        content_type.starts_with("application/json"),
        "content type: {content_type}"
    );
}

#[tokio::test]
async fn deepseek_fetch_error_mapping() {
    let cases: &[(u16, FetchFailure, Option<u16>)] = &[
        (400, FetchFailure::Status, Some(400)),
        (401, FetchFailure::Status, Some(401)),
        (402, FetchFailure::Status, Some(402)),
        (403, FetchFailure::Status, Some(403)),
        (404, FetchFailure::Status, Some(404)),
        (408, FetchFailure::Timeout, None),
        (413, FetchFailure::TooLarge, None),
        (429, FetchFailure::Status, Some(429)),
        (500, FetchFailure::Status, Some(500)),
        (504, FetchFailure::Timeout, None),
    ];
    for (code, reason, carried) in cases {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/tools/fetch"))
            .respond_with(ResponseTemplate::new(*code).set_body_string("failure"))
            .mount(&server)
            .await;
        let web = WebService::new().with_deepseek_base_url(server.uri());
        let error = web
            .fetch_document(&deepseek_config(&server), "https://example.com/page")
            .await
            .expect_err("the endpoint failed");
        match error {
            FetchError::Failed {
                reason: actual,
                status,
                ..
            } => {
                assert_eq!(actual, *reason, "status {code}");
                assert_eq!(status, *carried, "status {code}");
            }
            other => panic!("status {code}: wrong error shape: {other:?}"),
        }
    }

    // A 200 without usable content is a network failure: there is no HTTP status to carry.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/tools/fetch"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "title": "",
            "content": "",
        })))
        .mount(&server)
        .await;
    let web = WebService::new().with_deepseek_base_url(server.uri());
    let error = web
        .fetch_document(&deepseek_config(&server), "https://example.com/empty")
        .await
        .expect_err("empty content");
    assert_eq!(error.reason(), Some(FetchFailure::Network));
    assert_eq!(error.status(), None);
}

#[tokio::test]
async fn deepseek_fetch_validates_the_url_shape_before_sending() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/tools/fetch"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "title": "unused",
            "content": "unused",
        })))
        .mount(&server)
        .await;
    let web = WebService::new().with_deepseek_base_url(server.uri());
    let too_long = format!("https://example.com/{}", "a".repeat(8_192));
    for url in [
        "not a url",
        "ftp://example.com/file",
        "https://user:secret@example.com/",
        "https://",
        too_long.as_str(),
    ] {
        let error = web
            .fetch_document(&deepseek_config(&server), url)
            .await
            .expect_err(url);
        assert_eq!(error.reason(), Some(FetchFailure::InvalidUrl), "{url}");
    }
    assert!(
        server
            .received_requests()
            .await
            .expect("requests")
            .is_empty(),
        "a malformed URL must not spend a request"
    );
}

#[tokio::test]
async fn fetching_through_deepseek_still_pages_and_caches() {
    // Longer than one page, so the tool has something to slice.
    let long_markdown = (0..1_200)
        .map(|index| format!("paragraph {index} of a long extracted page\n\n"))
        .collect::<String>();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/tools/fetch"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "title": "A long page",
            "content": long_markdown,
        })))
        .expect(1)
        .mount(&server)
        .await;

    let web = WebService::new().with_deepseek_base_url(server.uri());
    let config = deepseek_config(&server);
    let url = "https://example.com/long";
    let first = web.fetch_document(&config, url).await.expect("first fetch");
    assert!(!first.cached);
    assert_eq!(first.title, "A long page");
    let page = fetch::slice_page(&first.markdown, 0, fetch::DEFAULT_MAX_CHARS);
    assert_eq!(page.offset, 0);
    assert!(page.next_offset.is_some(), "the page continues");
    assert!(page.text.chars().count() < first.markdown.chars().count());

    // The second read of the same URL (what an `offset` call does) is a cache hit: the mock's
    // `expect(1)` fails the test if DeepSeek is asked again.
    let second = web.fetch_document(&config, url).await.expect("second read");
    assert!(second.cached, "the page was refetched: {second:?}");
    assert_eq!(second.markdown, first.markdown);
    let next = fetch::slice_page(
        &second.markdown,
        page.next_offset.expect("offset"),
        fetch::DEFAULT_MAX_CHARS,
    );
    assert!(!next.text.is_empty());
    assert_eq!(
        server.received_requests().await.expect("requests").len(),
        1,
        "offset reads are cache hits"
    );
}

// ───────────────────────────── the direct default and the guard ─────────────────────────────

#[tokio::test]
async fn the_direct_fetcher_remains_the_default() {
    // A stand-in public host for the guarded fetcher.
    let site = MockServer::start().await;
    let address: SocketAddr = *site.address();
    let fetcher = Fetcher::with_host_overrides(vec![(
        "public.example".to_owned(),
        SocketAddr::new(address.ip(), address.port()),
    )])
    .expect("fetcher");
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            "<html><head><title>Direct</title></head><body>\
             <article><p>Readability extracted this.</p></article></body></html>",
            "text/html; charset=utf-8",
        ))
        .mount(&site)
        .await;

    // The DeepSeek endpoint exists but must never be reached while `fetch_via` is `direct`.
    let deepseek = MockServer::start().await;
    let web = WebService::with_fetcher(fetcher).with_deepseek_base_url(deepseek.uri());
    let url = format!("http://public.example:{}/page", site.address().port());
    let document = web
        .fetch_document(&FetchConfig::default(), &url)
        .await
        .expect("direct fetch");
    assert_eq!(document.title, "Direct");
    assert!(document.markdown.contains("Readability extracted this."));
    assert!(
        deepseek
            .received_requests()
            .await
            .expect("requests")
            .is_empty(),
        "the DeepSeek backend must not run unless it was selected"
    );
}

#[tokio::test]
async fn the_private_address_exception_is_scoped_to_the_deepseek_backend() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/tools/fetch"))
        .and(body_string_contains("127.0.0.1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "title": "Loopback from DeepSeek's vantage point",
            "content": "Fetched by DeepSeek, not by us.",
        })))
        .expect(1)
        .mount(&server)
        .await;

    let web = WebService::new().with_deepseek_base_url(server.uri());
    // The direct backend still refuses the literal loopback address before any socket opens.
    let error = web
        .fetch_document(&direct_config(), "http://127.0.0.1/")
        .await
        .expect_err("the direct guard refuses loopback");
    assert_eq!(error.reason(), Some(FetchFailure::Blocked));

    // The DeepSeek backend sends the same URL as data to its fixed host (the test seam), because
    // our process never resolves or connects to the target.
    let document = web
        .fetch_document(&deepseek_config(&server), "http://127.0.0.1/")
        .await
        .expect("DeepSeek may reach loopback from its own network");
    assert_eq!(document.markdown, "Fetched by DeepSeek, not by us.");

    let requests = server.received_requests().await.expect("requests");
    assert_eq!(requests.len(), 1);
    let sent: Value = serde_json::from_slice(&requests[0].body).expect("json body");
    assert_eq!(sent["url"], "http://127.0.0.1/");
}

// ───────────────────────────── the setting ─────────────────────────────

#[tokio::test]
async fn fetch_via_defaults_to_direct() {
    let config = nekowriter::services::search_settings::to_config(&SearchSettingsDto::default())
        .expect("config");
    assert_eq!(config.fetch_via, FetchBackend::Direct);

    let app = TestApp::new().await;
    let (_, settings) = app.get("/api/app-settings").await;
    assert_eq!(settings["search"]["fetch_via"], "direct");
    // An unrelated save keeps the default.
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "search": { "max_results": 8 } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["search"]["fetch_via"], "direct");
}

#[tokio::test]
async fn fetch_via_deepseek_requires_a_key() {
    let app = TestApp::new().await;
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "search": { "fetch_via": "deepseek" } }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_error(&body, "validation.failed");
    assert_error_detail(&body, "validation.failed", "fields");
    assert!(
        body["error"]["details"]["fields"]
            .get("search.fetch_via")
            .is_some(),
        "the field error names search.fetch_via: {body}"
    );

    // With the DeepSeek key saved the same patch is accepted, and both values persist.
    let (status, body) = app
        .patch(
            "/api/app-settings",
            json!({ "search": {
                "provider": "tavily",
                "tavily_api_key": "tvly-search-key",
                "deepseek_api_key": "sk-deepseek-test",
                "fetch_via": "deepseek",
            } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["search"]["provider"], "tavily");
    assert_eq!(body["search"]["fetch_via"], "deepseek");
    assert_eq!(body["search"]["deepseek_api_key"], "…test");
}
