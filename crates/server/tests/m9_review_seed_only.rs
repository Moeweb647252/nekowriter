//! M9 review findings on the canon-seed tool restrictions (spec/05 §6, spec/06 §7).

mod review_harness;

use std::time::Duration;

use nekowriter::db::json::SessionKind;
use nekowriter_llm::mock::ScriptStep;
use review_harness::{prepare_in_review, runtime, start_archivist};
use serde_json::json;

/// `is_protagonist` belongs to the initial seed only (spec/05 §6: "canon_seed only, otherwise
/// `error.seed_only`"). Passing it at all — even as `false`, which an archivist session might copy
/// from an entity shape — must be refused outside `canon_seed`, not silently accepted.
#[tokio::test]
async fn is_protagonist_is_refused_outside_the_seed_in_every_form() {
    let runtime = runtime(vec![
        ScriptStep::new().tool(
            "create_entity",
            json!({
                "kind": "character",
                "name": "林凡",
                "summary": "身怀残破剑灵的青云宗弟子",
                "is_protagonist": false,
            }),
        ),
        ScriptStep::new().tool("finish_maintenance", json!({ "report": "done" })),
    ])
    .await;
    let _ = prepare_in_review(&runtime).await;
    let session_id = start_archivist(
        &runtime,
        runtime.chapter_id,
        SessionKind::ChapterMaintenance,
    )
    .await;

    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let call = loop {
        if let Some(call) = runtime.calls(session_id).await.into_iter().find(|call| {
            call.name == "create_entity"
                && matches!(
                    call.status.as_str(),
                    "done" | "error" | "held" | "awaiting_input"
                )
        }) {
            break call;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the scripted create_entity never ran"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert!(
        call.is_error,
        "is_protagonist is seed-only outside canon_seed: status={} raw={:?} input={}",
        call.status, call.raw_result_text, call.input
    );
    // The rendered error is `raw_result_text` until the call is sealed; the compose step only
    // adds appendices, so either carries the same model-facing text.
    let text = call
        .result_text
        .clone()
        .or(call.raw_result_text.clone())
        .unwrap_or_default();
    assert!(
        text.contains("is_protagonist"),
        "the error names the parameter: {text}"
    );
    assert!(
        text.contains("建立初始档案"),
        "it is the seed_only branch of result.error: {text}"
    );
}
