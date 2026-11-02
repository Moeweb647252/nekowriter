//! The web tools through a full agent session (PLAN 11 hardening, task-24 item 8).
//!
//! The M8 review noted a coverage gap: `web_fetch`'s `prepare`/`execute` split was tested through
//! the tool path alone, never through a runner that freezes the tool list, waits for the preview
//! stream and seals the result. This test drives a real researcher session with a scripted
//! provider against a `wiremock` stand-in host (the D-096 seam), so the production pipeline runs:
//! guard → fetch → extract → prompt template → sealed tool result.

use std::net::SocketAddr;

use axum::http::StatusCode;
use nekowriter::testing::TestApp;
use nekowriter::web::WebService;
use nekowriter::web::fetch::Fetcher;
use nekowriter_llm::mock::{ScriptStep, ScriptedProvider};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The stand-in public host the fetcher resolves to the local server.
const HOST: &str = "example.test";

#[tokio::test]
async fn a_researcher_session_fetches_a_page_and_seals_it_into_the_history() {
    let server = MockServer::start().await;
    let body = "<html><head><title>Gate rules</title></head><body><main><h1>Gate rules</h1>\
                <p>The night watch changes at the third bell.</p></main></body></html>";
    Mock::given(method("GET"))
        .and(path("/gate"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/html; charset=utf-8"))
        .mount(&server)
        .await;
    let address: SocketAddr = *server.address();
    let web = WebService::with_fetcher(
        Fetcher::with_host_overrides(vec![(
            HOST.to_owned(),
            SocketAddr::new(address.ip(), address.port()),
        )])
        .expect("fetcher"),
    );

    let mut app = TestApp::new().await;
    app.set_web(web);
    // The researcher freezes `web_fetch` in its tool list (spec/05 §2).
    let scripted = ScriptedProvider::new(vec![
        ScriptStep::new().tool(
            "web_fetch",
            json!({ "url": format!("http://{HOST}:{}/gate", server.address().port()) }),
        ),
        ScriptStep::new().tool("finish_research", json!({ "report": "已取回城门守则。" })),
    ])
    .fail_when_exhausted(nekowriter_llm::LlmError::BadRequest(
        "script exhausted".to_owned(),
    ));
    app.set_provider(scripted);

    // A model and the researcher role, a book, and a research task to run.
    let (status, provider) = app
        .post(
            "/api/providers",
            json!({ "name": "Demo", "protocol": "mock", "base_url": "mock://demo", "extra_headers": {} }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "provider: {provider}");
    let (status, profile) = app
        .post(
            "/api/model-profiles",
            json!({
                "provider_id": provider["id"],
                "display_name": "Demo model",
                "model": "demo-model",
                "context_window": 200_000,
                "max_output_tokens": 8_192,
                "effort_levels": [],
                "default_effort": null,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "profile: {profile}");
    let (status, _) = app
        .patch(
            "/api/app-settings",
            json!({ "roles_defaults": { "researcher": profile["id"] } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let book = app.create_book("Book").await;
    let book_id = book["id"].as_i64().expect("book id") as i32;
    let (status, task) = app
        .post(
            "/api/research-tasks",
            json!({ "book_id": book_id, "scope_name": "考据", "topic": "城门守则" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "task: {task}");
    let task_id = task["id"].as_i64().expect("task id") as i32;

    let (status, run) = app
        .post(&format!("/api/research-tasks/{task_id}/run"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "run: {run}");
    let session_id = run["session_id"].as_i64().expect("session") as i32;

    // The session finishes through the sealed history.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let snapshot = loop {
        let (_, snapshot) = app.get(&format!("/api/sessions/{session_id}")).await;
        let status = snapshot["session"]["status"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if status == "completed" {
            break snapshot;
        }
        assert!(
            status != "failed" && status != "abandoned" && status != "superseded",
            "the session ended as {status}: {snapshot}"
        );
        assert!(
            std::time::Instant::now() < deadline,
            "the session did not finish"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };

    // The tool call went through prepare (fetch) and execute (render), and was sealed.
    let call = snapshot["tool_calls"]
        .as_array()
        .expect("tool calls")
        .iter()
        .find(|call| call["name"] == "web_fetch")
        .expect("the web_fetch call is in the history");
    assert_eq!(call["status"], "done", "call: {call}");
    assert_eq!(call["is_error"], false);
    assert_eq!(call["sealed"], true, "a completed session seals its calls");
    let result = call["result_text"].as_str().expect("result text");
    assert!(
        result.contains("The night watch changes at the third bell."),
        "the extracted body reaches the model: {result}"
    );
    assert!(
        result.contains("Gate rules"),
        "the title is part of the result: {result}"
    );
    // The task itself completed, so the runner's completion handler ran.
    let (_, finished) = app.get(&format!("/api/research-tasks/{task_id}")).await;
    assert_eq!(finished["status"], "done", "task: {finished}");
}
