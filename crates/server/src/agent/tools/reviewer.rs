//! Reviewer tool (spec/05 §5): `submit_review`.
//!
//! The handler creates the `review_reports` row in the call's transaction (D-033): the report,
//! the call's result and the completion of the session commit together. `content_version` is the
//! session's `base_content_version` — the version the reviewer actually read (D-026) — and
//! `user_comment` comes from `pending_comment`, which is cleared right after (spec/03 §6).
//!
//! Unknown pids are dropped from the issues and the result names them (spec/05 §5), so a review
//! written against a text the author edited meanwhile never carries dangling references.

use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::db::entities::{agent_sessions, review_reports};
use crate::db::json::{ChapterContent, CompletionKind, ReviewIssue, ReviewVerdict, ToolEffects};

use super::{Prepared, ToolCtx, ToolError, ToolErrorKind, ToolHandler, ToolKind, ToolOutcome};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SubmitReviewInput {
    verdict: ReviewVerdict,
    summary: String,
    issues: Vec<ReviewIssue>,
    feedback: String,
}

fn decode<T: for<'de> Deserialize<'de>>(input: Value) -> Result<T, ToolError> {
    serde_json::from_value(input).map_err(|err| {
        ToolError::new(ToolErrorKind::Validation).with("errors", vec![err.to_string()])
    })
}

/// `submit_review` (spec/05 §5).
pub struct SubmitReviewTool;

#[async_trait::async_trait]
impl ToolHandler for SubmitReviewTool {
    fn name(&self) -> &'static str {
        "submit_review"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Control
    }

    async fn execute(
        &self,
        ctx: &mut ToolCtx<'_>,
        input: Value,
        _prepared: Prepared,
    ) -> Result<ToolOutcome, ToolError> {
        let input: SubmitReviewInput = decode(input)?;
        let chapter = ctx
            .chapter
            .ok_or_else(|| ToolError::new(ToolErrorKind::Internal))?;
        let content: ChapterContent = crate::db::json::from_json_or_default(&chapter.content);

        // Unknown pids are dropped, with a note in the result (spec/05 §5).
        let mut dropped: Vec<String> = Vec::new();
        let mut issues = input.issues;
        for issue in &mut issues {
            issue.pids.retain(|pid| {
                if content
                    .paragraphs
                    .iter()
                    .any(|paragraph| &paragraph.pid == pid)
                {
                    true
                } else {
                    dropped.push(pid.clone());
                    false
                }
            });
        }
        dropped.sort();
        dropped.dedup();

        // `attempt` = count + 1 per chapter (spec/05 §5).
        let attempt = review_reports::Entity::find()
            .filter(review_reports::Column::ChapterId.eq(chapter.id))
            .order_by_desc(review_reports::Column::Attempt)
            .one(ctx.tx)
            .await
            .map_err(|_| ToolError::new(ToolErrorKind::Internal))?
            .map(|row| row.attempt + 1)
            .unwrap_or(1);

        let content_version = ctx
            .session
            .base_content_version
            .unwrap_or(chapter.content_version);
        let user_comment = ctx.session.pending_comment.clone().unwrap_or_default();
        let now = chrono::Utc::now();
        let report = review_reports::ActiveModel {
            chapter_id: Set(chapter.id),
            content_version: Set(content_version),
            session_id: Set(Some(ctx.session.id)),
            attempt: Set(attempt),
            verdict: Set(input.verdict),
            summary: Set(input.summary.trim().to_owned()),
            issues: Set(crate::db::json::to_json(&issues)
                .map_err(|_| ToolError::new(ToolErrorKind::Internal))?),
            feedback: Set(input.feedback.trim().to_owned()),
            user_comment: Set(user_comment),
            created_at: Set(now),
            ..Default::default()
        }
        .insert(ctx.tx)
        .await
        .map_err(|_| ToolError::new(ToolErrorKind::Internal))?;

        // The next report must not copy a comment that was already consumed (spec/03 §6).
        if ctx.session.pending_comment.is_some() {
            agent_sessions::ActiveModel {
                id: Set(ctx.session.id),
                pending_comment: Set(None),
                ..Default::default()
            }
            .update(ctx.tx)
            .await
            .map_err(|_| ToolError::new(ToolErrorKind::Internal))?;
        }

        let text = ctx
            .prompts
            .render_value(
                "result.submit_review",
                crate::prompts::template_value(&json!({
                    "attempt": attempt,
                    "issues": issues.len(),
                    "dropped_pids": dropped,
                })),
            )
            .map_err(|_| ToolError::new(ToolErrorKind::Internal))?
            .text;
        Ok(ToolOutcome::text(text)
            .with_ui(json!({
                "report_id": report.id,
                "verdict": input.verdict.as_str(),
                "issues": issues.len(),
            }))
            .with_effects(ToolEffects {
                complete: Some(CompletionKind::Review),
                ..ToolEffects::default()
            }))
    }
}

/// The reviewer role's handlers (spec/05 §2).
pub fn all() -> Vec<std::sync::Arc<dyn ToolHandler>> {
    vec![std::sync::Arc::new(SubmitReviewTool)]
}
