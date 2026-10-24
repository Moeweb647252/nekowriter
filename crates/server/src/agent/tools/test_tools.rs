//! Test-only tool handlers (spec/10 §2.7 `POST /api/debug/sessions/echo`, PLAN M3.1).
//!
//! These exercise the runtime's paths that the real tools of later milestones will use:
//! `echo` (a plain retrieval-shaped call), `test_write` (a `WRITE_TOOLS` member, so `auto_pause`
//! holds it), `test_finish` (a completing call) and `test_slow` (a slow, cancellable `prepare`,
//! so a Stop during `prepare` returns the call to `pending`, D-024). They are never part of a
//! role's tool list.

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    FetchedPage, PrepareCtx, Prepared, ToolCtx, ToolError, ToolHandler, ToolKind, ToolOutcome,
};
use crate::db::json::{CompletionKind, ToolEffects};

/// Input of the test-only tools: everything is optional so a call can be minimal.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TestInput {
    #[serde(default)]
    text: Option<String>,
    /// How long `test_slow`'s prepare sleeps, in milliseconds.
    #[serde(default)]
    sleep_ms: Option<u64>,
    /// `test_slow` fails its prepare with this error code instead of returning.
    #[serde(default)]
    fail: Option<String>,
}

/// `echo`: returns the text it was given.
pub struct EchoTool;

#[async_trait::async_trait]
impl ToolHandler for EchoTool {
    fn name(&self) -> &'static str {
        "echo"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Debug
    }

    fn preview_field(&self) -> Option<&'static str> {
        Some("text")
    }

    async fn execute(
        &self,
        _ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: TestInput = decode(input)?;
        let text = input.text.unwrap_or_default();
        Ok(ToolOutcome::text(text.clone()).with_ui(json!({ "chars": text.chars().count() })))
    }
}

/// `test_write`: a text-editing tool, so `auto_pause` and the pause request hold it.
pub struct TestWriteTool;

#[async_trait::async_trait]
impl ToolHandler for TestWriteTool {
    fn name(&self) -> &'static str {
        "test_write"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Draft
    }

    fn preview_field(&self) -> Option<&'static str> {
        Some("text")
    }

    async fn execute(
        &self,
        _ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: TestInput = decode(input)?;
        let text = input.text.unwrap_or_default();
        Ok(ToolOutcome::text(text.clone()).with_ui(json!({
            "chars": text.chars().count(),
        })))
    }
}

/// `test_finish`: a completing call (`complete: draft`), so the session finishes and the rest of
/// its batch is `error.after_completion` (spec/03 §5.4, test 16).
pub struct TestFinishTool;

#[async_trait::async_trait]
impl ToolHandler for TestFinishTool {
    fn name(&self) -> &'static str {
        "test_finish"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Control
    }

    async fn execute(
        &self,
        _ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: TestInput = decode(input)?;
        let text = input.text.unwrap_or_else(|| "finished".to_owned());
        Ok(ToolOutcome::text(text).with_effects(ToolEffects {
            complete: Some(CompletionKind::Draft),
            ..ToolEffects::default()
        }))
    }
}

/// `test_slow`: a slow, cancellable `prepare` (spec/05 §1.2, D-024, spec/03 §15 test 17).
pub struct TestSlowTool;

#[async_trait::async_trait]
impl ToolHandler for TestSlowTool {
    fn name(&self) -> &'static str {
        "test_slow"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Web
    }

    async fn prepare(&self, _ctx: &PrepareCtx<'_>, input: &Value) -> Result<Prepared, ToolError> {
        let input: TestInput = serde_json::from_value(input.clone())
            .map_err(|_| ToolError::new(super::ToolErrorKind::Validation))?;
        let sleep = Duration::from_millis(input.sleep_ms.unwrap_or(250));
        tokio::time::sleep(sleep).await;
        if let Some(code) = input.fail {
            // A failing prepare becomes an error result, not a rolled-back transaction.
            let kind = match code.as_str() {
                "search_failed" => super::ToolErrorKind::SearchFailed,
                "fetch_failed" => super::ToolErrorKind::FetchFailed,
                "validation" => super::ToolErrorKind::Validation,
                _ => super::ToolErrorKind::Internal,
            };
            return Err(ToolError::new(kind).with("reason", "timeout"));
        }
        Ok(Prepared::WebPage(Box::new(FetchedPage {
            url: "https://example.invalid/test".to_owned(),
            title: "test".to_owned(),
            markdown: input.text.unwrap_or_default(),
            cached: false,
        })))
    }

    async fn execute(
        &self,
        _ctx: &mut ToolCtx<'_>,
        _input: Value,
        prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let Prepared::WebPage(page) = prepared else {
            return Err(ToolError::new(super::ToolErrorKind::Internal));
        };
        Ok(ToolOutcome::text(page.markdown.clone())
            .with_ui(json!({ "url": page.url, "chars": page.markdown.chars().count() })))
    }
}

/// Maps a serde failure into a validation tool error (spec/05 §1.4 step 3).
fn decode<T: serde::de::DeserializeOwned>(input: Value) -> Result<T, ToolError> {
    serde_json::from_value(input).map_err(|err| {
        ToolError::new(super::ToolErrorKind::Validation).with("errors", vec![err.to_string()])
    })
}

/// The JSON Schema of the test-only tools. They are not in `prompts/tools.toml`, because that
/// file is the model-facing pack (spec/09 §1) and these tools are never reachable from a real
/// session (spec/10 §2.7).
pub fn schemas() -> Vec<(&'static str, serde_json::Value)> {
    let schema = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "text": { "type": "string", "description": "text to echo or write" },
            "sleep_ms": { "type": "integer", "minimum": 0, "description": "prepare delay" },
            "fail": { "type": "string", "description": "fail the prepare with this code" },
        },
    });
    vec![
        ("echo", schema.clone()),
        ("test_write", schema.clone()),
        ("test_finish", schema.clone()),
        ("test_slow", schema),
    ]
}

/// Every test-only handler (they exist so the debug session has tools to drive).
pub fn all() -> Vec<Arc<dyn ToolHandler>> {
    vec![
        Arc::new(EchoTool),
        Arc::new(TestWriteTool),
        Arc::new(TestFinishTool),
        Arc::new(TestSlowTool),
    ]
}
