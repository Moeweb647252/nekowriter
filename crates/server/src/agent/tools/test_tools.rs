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
    /// `test_ask` records this phase change in its deferred effects.
    #[serde(default)]
    phase: Option<String>,
    /// `test_ask` records a `complete: draft` effect when true.
    #[serde(default)]
    complete: Option<bool>,
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
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: TestInput = decode(input)?;
        let text = input.text.unwrap_or_default();
        let mut outcome = ToolOutcome::text(text.clone()).with_ui(json!({
            "chars": text.chars().count(),
        }));
        // A debug session may have no chapter; then there is nothing to stage.
        if ctx.chapter.is_some() {
            // A write tool stages its content like the real ones (D-139), so the hold/resume path
            // carries a real pending write.
            let mut content: crate::db::json::ChapterContent = ctx
                .chapter
                .map(|chapter| chapter.content.clone().into_inner())
                .unwrap_or_default();
            let paragraph = crate::db::json::Paragraph {
                pid: crate::text::allocate_pid(&mut content),
                text: text.clone(),
            };
            content.paragraphs.push(paragraph);
            outcome = outcome
                .with_pending_content(crate::agent::tools::writer::stage_content(ctx, content)?);
        }
        // A write tool may also request a completion, so the hold/resume path can carry both
        // halves of the deferred effects (spec/03 §5.4). `with_effects` replaces the whole set, so
        // the pending content is attached afterwards.
        if input.complete.unwrap_or(false) {
            return Ok(outcome.with_effects(ToolEffects {
                complete: Some(CompletionKind::Draft),
                ..ToolEffects::default()
            }));
        }
        Ok(outcome)
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

/// `test_ask`: an awaiting call that also records a phase change and/or a completion in its
/// deferred effects.
///
/// The M3 runtime review found that the transactions finalizing an answered/held call applied
/// only part of the recorded `ToolEffects`. No M3 tool set `await_input`, so nothing could reach
/// the answer path; this tool makes the path observable without shipping a real one (it is only
/// frozen into a debug session, D-072).
pub struct TestAskTool;

#[async_trait::async_trait]
impl ToolHandler for TestAskTool {
    fn name(&self) -> &'static str {
        "test_ask"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Debug
    }

    async fn execute(
        &self,
        _ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: TestInput = decode(input)?;
        let phase = input.phase.as_deref().and_then(|raw| match raw {
            "planning" => Some(crate::db::json::Phase::Planning),
            "drafting" => Some(crate::db::json::Phase::Drafting),
            "revision_planning" => Some(crate::db::json::Phase::RevisionPlanning),
            "revising" => Some(crate::db::json::Phase::Revising),
            _ => None,
        });
        let complete = input
            .complete
            .unwrap_or(false)
            .then_some(CompletionKind::Draft);
        let text = input.text.unwrap_or_else(|| "test_ask".to_owned());
        // `with_await_input` also records the effect, so the deferred-effects record is complete.
        let outcome = ToolOutcome::text(text)
            .with_ui(json!({}))
            .with_await_input(crate::db::json::AwaitInputKind::AskUser);
        Ok(outcome.with_effects(ToolEffects {
            phase_to: phase,
            complete,
            await_input: Some(crate::db::json::AwaitInputKind::AskUser),
            ..ToolEffects::default()
        }))
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
            "complete": { "type": "boolean", "description": "deferred completion" },
        },
    });
    let ask_schema = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "text": { "type": "string", "description": "answer placeholder" },
            "phase": {
                "type": "string",
                "enum": ["planning", "drafting", "revision_planning", "revising"],
                "description": "deferred phase change",
            },
            "complete": { "type": "boolean", "description": "deferred completion" },
        },
    });
    vec![
        ("echo", schema.clone()),
        ("test_write", schema.clone()),
        ("test_finish", schema.clone()),
        ("test_slow", schema),
        ("test_ask", ask_schema),
    ]
}

/// Every test-only handler (they exist so the debug session has tools to drive).
pub fn all() -> Vec<Arc<dyn ToolHandler>> {
    vec![
        Arc::new(EchoTool),
        Arc::new(TestWriteTool),
        Arc::new(TestFinishTool),
        Arc::new(TestSlowTool),
        Arc::new(TestAskTool),
    ]
}
