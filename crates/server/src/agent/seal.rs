//! Sealing: composing a tool call's delivered text once (spec/03 §1.3, §5.3).
//!
//! The text delivered for a tool call is `raw_result_text` followed by its appendices, separated
//! by a blank line, in the canonical order:
//!
//! 1. effect notices about the call itself (`writer.notice.revision_plan_accepted`, …);
//! 2. the edit report (`notice.user_edits` / `notice.setting_edits`);
//! 3. author feedback given on a held call (`notice.user_feedback`);
//! 4. inbox items in arrival order (`notice.user_message` and runtime notices).
//!
//! Within a kind, arrival order is preserved. `result_text` is written once and never modified
//! afterwards (append-only history, spec/03 §1.3).

use serde_json::json;

use crate::db::json::{Appendix, AppendixKind};
use crate::prompts::PromptSet;

/// The delivered text of an author message (D-160, spec/03 §5.3 slot 4).
///
/// The inbox row keeps the author's **raw** text (`agent_inbox.content`); only the delivered copy
/// is rendered through `notice.user_message` with the session's `PromptSet`, at the moment the row
/// is consumed. The rendered text is persisted once and never recomputed (spec/03 §1.3).
///
/// A broken override already falls back to the shipped default inside the registry (with the
/// `prompt.override_failed` warning); if the default itself cannot render, or the effective
/// template renders empty (a notice override may be saved empty), the raw text is delivered and
/// the failure is logged. The author's words are never dropped.
pub fn author_message_text(prompts: &PromptSet, content: &str) -> String {
    match prompts.render("notice.user_message", &json!({ "text": content })) {
        Ok(rendered) => {
            // The registry logs its `prompt.override_failed` warning itself; a consumed row has
            // no session warning channel, and the default text is what gets delivered.
            let _ = rendered.warnings();
            if rendered.text.trim().is_empty() {
                // An empty render is a failure, not a successful notice: on paths B and C an
                // empty block is dropped, which would silently swallow the author's message.
                tracing::warn!(
                    key = "notice.user_message",
                    "notice.user_message rendered empty; delivering the raw author text"
                );
                return content.to_owned();
            }
            rendered.text
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                "notice.user_message failed to render; delivering the raw author text"
            );
            content.to_owned()
        }
    }
}

/// Composes the delivered text of a sealed tool call (spec/03 §5.3).
///
/// Returns `None` when the call has no raw result text at all (a rejected call always has one).
pub fn compose(raw_result_text: Option<&str>, appendices: &[Appendix]) -> String {
    let mut ordered: Vec<&Appendix> = appendices.iter().collect();
    // `sort_by_key` is stable, so arrival order survives inside a kind.
    ordered.sort_by_key(|appendix| appendix.kind.canonical_rank());

    let mut parts: Vec<&str> = Vec::new();
    if let Some(raw) = raw_result_text
        && !raw.trim().is_empty()
    {
        parts.push(raw.trim());
    }
    for appendix in ordered {
        if !appendix.text.trim().is_empty() {
            parts.push(appendix.text.trim());
        }
    }
    parts.join("\n\n")
}

/// The `AppendixKind` of an inbox row: author messages are `inbox` items too, but the canonical
/// order treats every inbox item the same (spec/03 §5.3 slot 4).
pub fn inbox_appendix(text: String, inbox_id: i32) -> Appendix {
    Appendix {
        kind: AppendixKind::Inbox,
        text,
        inbox_id: Some(inbox_id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn appendix(kind: AppendixKind, text: &str) -> Appendix {
        Appendix {
            kind,
            text: text.to_owned(),
            inbox_id: None,
        }
    }

    #[test]
    fn compose_orders_by_kind_then_arrival() {
        let appendices = vec![
            appendix(AppendixKind::Inbox, "inbox one"),
            appendix(AppendixKind::UserFeedback, "feedback"),
            appendix(AppendixKind::Inbox, "inbox two"),
            appendix(AppendixKind::EffectNotice, "notice"),
            appendix(AppendixKind::EditReport, "edits"),
        ];
        let composed = compose(Some("raw"), &appendices);
        assert_eq!(
            composed,
            "raw\n\nnotice\n\nedits\n\nfeedback\n\ninbox one\n\ninbox two"
        );
    }

    #[test]
    fn compose_without_raw_text_starts_with_the_first_appendix() {
        let appendices = vec![appendix(AppendixKind::EditReport, "edits")];
        assert_eq!(compose(None, &appendices), "edits");
    }

    #[test]
    fn compose_drops_empty_parts_and_trims() {
        let appendices = vec![
            appendix(AppendixKind::EffectNotice, "  "),
            appendix(AppendixKind::Inbox, "  kept  "),
        ];
        assert_eq!(compose(Some(" raw \n"), &appendices), "raw\n\nkept");
    }

    #[test]
    fn compose_of_nothing_is_empty() {
        assert_eq!(compose(None, &[]), "");
        assert_eq!(compose(Some(""), &[]), "");
    }

    #[test]
    fn the_canonical_rank_matches_spec_03_section_5_3() {
        assert!(
            AppendixKind::EffectNotice.canonical_rank() < AppendixKind::EditReport.canonical_rank()
        );
        assert!(
            AppendixKind::EditReport.canonical_rank() < AppendixKind::UserFeedback.canonical_rank()
        );
        assert!(AppendixKind::UserFeedback.canonical_rank() < AppendixKind::Inbox.canonical_rank());
    }
}
