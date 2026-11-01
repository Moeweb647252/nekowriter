//! Sealing: composing a tool call's delivered text once (spec/03 §1.3, §5.3).
//!
//! The text delivered for a tool call is `raw_result_text` followed by its appendices, separated
//! by a blank line, in the canonical order:
//!
//! 1. effect notices about the call itself (`writer.notice.outline_accepted`, …);
//! 2. the edit report (`notice.user_edits` / `notice.setting_edits`);
//! 3. author feedback given on a held call (`notice.user_feedback`);
//! 4. inbox items in arrival order (`notice.user_message` and runtime notices).
//!
//! Within a kind, arrival order is preserved. `result_text` is written once and never modified
//! afterwards (append-only history, spec/03 §1.3).

use crate::db::json::{Appendix, AppendixKind};

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
