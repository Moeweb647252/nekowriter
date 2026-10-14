//! Citations: exact passages the user quotes in feedback (`docs/product-spec.md` §18.4).
//!
//! Offsets count Unicode scalar values and are half-open. A citation stays within one
//! text: a committed revision, or the candidate text of a pending proposal.

use sea_orm::FromJsonQueryResult;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    chapter::ChapterLabel,
    document::DocumentRole,
    ids::{RevisionId, ToolCallId},
};

/// The most citations one piece of feedback may carry.
pub const MAX_CITATIONS: usize = 20;

/// A passage as the user selected it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Citation {
    pub target: CitationTarget,
    pub start: u32,
    pub end: u32,
    pub quote: String,
}

/// The text a citation quotes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum CitationTarget {
    /// A committed revision of a document.
    Revision { revision_id: RevisionId },
    /// The body a pending proposal would produce, identified by its SHA-256.
    Proposal {
        call_id: ToolCallId,
        candidate_hash: String,
    },
}

/// Where a cited passage comes from, as the writer is told.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum CitationSource {
    Document {
        role: DocumentRole,
        chapter: Option<ChapterLabel>,
    },
    Proposal,
}

/// A citation whose quote matched its text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedCitation {
    pub citation: Citation,
    pub source: CitationSource,
}

/// Verified citations as a stored column.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, FromJsonQueryResult)]
pub struct Citations(pub Vec<VerifiedCitation>);

/// Why a citation was refused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum CitationProblem {
    /// More citations than one piece of feedback may carry.
    LimitExceeded { max: u32 },
    /// The range is empty or outside the text.
    BadRange,
    /// The text at the range is not the quote.
    QuoteMismatch,
    /// The cited text no longer exists or belongs to another version.
    TargetGone,
}

/// Checks that `text` holds `quote` at the citation's range.
pub fn check_quote(text: &str, citation: &Citation) -> Result<(), CitationProblem> {
    if citation.start >= citation.end {
        return Err(CitationProblem::BadRange);
    }
    let length = (citation.end - citation.start) as usize;
    let quoted: String = text
        .chars()
        .skip(citation.start as usize)
        .take(length)
        .collect();
    if quoted.chars().count() != length {
        return Err(CitationProblem::BadRange);
    }
    if quoted == citation.quote {
        Ok(())
    } else {
        Err(CitationProblem::QuoteMismatch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn citation(start: u32, end: u32, quote: &str) -> Citation {
        Citation {
            target: CitationTarget::Revision {
                revision_id: RevisionId::generate(),
            },
            start,
            end,
            quote: quote.to_owned(),
        }
    }

    #[test]
    fn quotes_are_checked_at_scalar_offsets() {
        // "Night rain fell." with two ideographs first.
        let text = "\u{591c}\u{96e8} fell.";
        assert_eq!(
            check_quote(text, &citation(0, 2, "\u{591c}\u{96e8}")),
            Ok(())
        );
        assert_eq!(check_quote(text, &citation(3, 7, "fell")), Ok(()));
        assert_eq!(
            check_quote(text, &citation(3, 7, "fall")),
            Err(CitationProblem::QuoteMismatch)
        );
        assert_eq!(
            check_quote(text, &citation(5, 99, "ll.")),
            Err(CitationProblem::BadRange)
        );
        assert_eq!(
            check_quote(text, &citation(2, 2, "")),
            Err(CitationProblem::BadRange)
        );
    }
}
