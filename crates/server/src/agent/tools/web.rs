//! The two web tools (spec/05 §7–§8, spec/07 §6–§7): `web_search` and `web_fetch`.
//!
//! Both do their network work in `prepare`, **outside** any transaction and cancellable by Stop
//! (spec/05 §1.2, D-024). `execute` only renders the already-fetched result; no tool holds a
//! transaction across a request.
//!
//! Every result is wrapped in `notice.web_content` (spec/01 §10.5): the model is told the text is
//! untrusted reference material whose instructions must be ignored. The wrapper is applied here,
//! once, so no tool can return raw web text by accident.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::web::WebError;
use crate::web::search;

use super::{
    PrepareCtx, Prepared, ToolCtx, ToolError, ToolErrorKind, ToolHandler, ToolKind, ToolOutcome,
};

// ───────────────────────────── shared helpers ─────────────────────────────

fn decode<T: for<'de> Deserialize<'de>>(input: Value) -> Result<T, ToolError> {
    serde_json::from_value(input).map_err(|err| {
        ToolError::new(ToolErrorKind::Validation).with("errors", vec![err.to_string()])
    })
}

fn internal() -> ToolError {
    ToolError::new(ToolErrorKind::Internal)
}

/// Wraps model-facing text in `notice.web_content` (spec/01 §10.5, spec/09 §2).
///
/// A missing template is a build defect; the text is returned unwrapped only then, so a broken
/// pack cannot make the tool unusable, but the failure is logged.
fn wrap_untrusted(ctx: &ToolCtx<'_>, body: &str) -> String {
    ctx.prompts
        .render("notice.web_content", &json!({ "body": body }))
        .map(|rendered| rendered.text)
        .unwrap_or_else(|err| {
            tracing::error!(error = %err, "cannot render notice.web_content");
            body.to_owned()
        })
}

/// Maps a [`WebError`] onto the tool error the model sees (spec/07 §6–§7, spec/09 §2).
fn web_error(error: &WebError) -> ToolError {
    match error {
        WebError::NotConfigured => ToolError::new(ToolErrorKind::SearchNotConfigured),
        WebError::Search { reason, status } => {
            let tool_error =
                ToolError::new(ToolErrorKind::SearchFailed).with("reason", reason.as_str());
            match status {
                Some(status) => tool_error.with("status", i64::from(*status)),
                None => tool_error,
            }
        }
    }
}

// ───────────────────────────── web_search ─────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WebSearchInput {
    query: String,
    #[serde(default)]
    max_results: Option<i64>,
}

/// `web_search` (spec/05 §7, spec/07 §6).
pub struct WebSearchTool;

#[async_trait::async_trait]
impl ToolHandler for WebSearchTool {
    fn name(&self) -> &'static str {
        "web_search"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Web
    }

    async fn prepare(&self, ctx: &PrepareCtx<'_>, input: &Value) -> Result<Prepared, ToolError> {
        let input: WebSearchInput = decode(input.clone())?;
        let query = input.query.trim().to_owned();
        if query.is_empty() {
            return Err(ToolError::new(ToolErrorKind::Validation)
                .with("errors", vec!["query must not be empty".to_owned()]));
        }
        // `app_settings["search"]`, with an `env:NAME` key resolved at call time (spec/01 §10.3).
        let config = crate::services::search_settings::config(ctx.state)
            .await
            .map_err(|error| {
                if error.code == crate::error::ErrorCode::ProviderAuth {
                    // An unset environment key behaves like a rejected one.
                    web_error(&WebError::Search {
                        reason: search::SearchFailure::Auth,
                        status: None,
                    })
                } else {
                    internal()
                }
            })?;
        if !search::is_configured(&config) {
            return Err(web_error(&WebError::NotConfigured));
        }
        let limit = match input.max_results {
            Some(requested) => usize::try_from(requested)
                .unwrap_or(crate::web::MAX_RESULTS_CAP)
                .clamp(1, crate::web::MAX_RESULTS_CAP),
            None => config.result_count(None),
        };
        let client = search::shared_client().map_err(|_| internal())?;
        let hits = search::search(client, &config, &query, limit)
            .await
            .map_err(|error| {
                web_error(&WebError::Search {
                    reason: error.reason,
                    status: error.status,
                })
            })?;
        Ok(Prepared::WebSearch(
            hits.into_iter()
                .map(|hit| super::WebHit {
                    title: hit.title,
                    url: hit.url,
                    snippet: hit.snippet,
                    published: hit.published,
                })
                .collect(),
        ))
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: WebSearchInput = decode(input)?;
        let query = input.query.trim().to_owned();
        let Prepared::WebSearch(hits) = prepared else {
            return Err(internal());
        };
        let rendered: Vec<Value> = hits
            .iter()
            .map(|hit| {
                json!({
                    "title": hit.title,
                    "url": hit.url,
                    "snippet": hit.snippet,
                    "published": hit.published.clone().unwrap_or_default(),
                })
            })
            .collect();
        let count = rendered.len();
        let text = ctx
            .prompts
            .render_value(
                "result.web_search",
                crate::prompts::template_value(&json!({ "query": query, "hits": rendered })),
            )
            .map_err(|_| internal())?
            .text;
        let wrapped = wrap_untrusted(ctx, &text);
        Ok(ToolOutcome::text(wrapped).with_ui(json!({
            "query": query,
            "results": hits
                .iter()
                .map(|hit| json!({ "title": hit.title, "url": hit.url }))
                .collect::<Vec<Value>>(),
            "count": count,
        })))
    }
}

// ───────────────────────────── registration ─────────────────────────────

/// The web tools registered so far (spec/05 §7).
pub fn all() -> Vec<std::sync::Arc<dyn ToolHandler>> {
    vec![std::sync::Arc::new(WebSearchTool)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::search::SearchFailure;

    fn error_code(error: &ToolError) -> &'static str {
        error.kind.code()
    }

    #[test]
    fn an_unconfigured_provider_is_search_not_configured() {
        let error = web_error(&WebError::NotConfigured);
        assert_eq!(error_code(&error), "search_not_configured");
        assert!(error.fields.is_empty());
    }

    #[test]
    fn search_failures_carry_their_reason_and_status() {
        for (reason, expected) in [
            (SearchFailure::Timeout, "timeout"),
            (SearchFailure::Auth, "auth"),
            (SearchFailure::Network, "network"),
        ] {
            let error = web_error(&WebError::Search {
                reason,
                status: None,
            });
            assert_eq!(error_code(&error), "search_failed");
            assert_eq!(error.fields["reason"], expected);
            assert!(!error.fields.contains_key("status"));
        }
        let status = web_error(&WebError::Search {
            reason: SearchFailure::Status,
            status: Some(503),
        });
        assert_eq!(status.fields["reason"], "status");
        assert_eq!(status.fields["status"], 503);
    }

    #[test]
    fn every_web_error_renders_through_the_prompt_template() {
        // The code and its fields are what `result.error` switches on (spec/09 §2), so a rendered
        // error must never fall back to the internal text.
        let prompts = crate::prompts::PromptRegistry::embedded()
            .expect("pack")
            .defaults_only();
        let cases = vec![
            web_error(&WebError::NotConfigured),
            web_error(&WebError::Search {
                reason: SearchFailure::Timeout,
                status: None,
            }),
            web_error(&WebError::Search {
                reason: SearchFailure::Status,
                status: Some(500),
            }),
        ];
        for case in cases {
            let outcome = ToolOutcome::error(&prompts, &case);
            assert!(outcome.is_error);
            assert!(
                !outcome.text.trim().is_empty(),
                "{} rendered nothing",
                case.kind.code()
            );
            assert!(
                !outcome.text.contains("{{"),
                "{} left a template placeholder: {}",
                case.kind.code(),
                outcome.text
            );
        }
    }

    #[test]
    fn a_search_result_is_wrapped_as_untrusted_content() {
        let prompts = crate::prompts::PromptRegistry::embedded()
            .expect("pack")
            .defaults_only();
        let wrapper = prompts
            .render("notice.web_content", &json!({ "body": "BODY" }))
            .expect("notice.web_content")
            .text;
        assert!(wrapper.contains("BODY"));
        assert!(wrapper.trim() != "BODY", "the wrapper adds its own text");
    }
}
