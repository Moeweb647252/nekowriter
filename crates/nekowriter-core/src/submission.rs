//! Stage submissions: what the agent submits to end a stage, and the business rules a
//! well-formed submission must pass (`docs/product-spec.md` §8.4). Breaking a rule is a
//! rejected round: the agent is told why and submits again.

use serde::{Deserialize, Serialize};

use crate::{field::normalize_line_endings, ids::ChapterId, preparation::ArtifactKind};

/// The longest chapter outline, in characters.
pub const OUTLINE_MAX_CHARS: u32 = 20_000;
/// The longest chapter summary, in characters.
pub const SUMMARY_MAX_CHARS: u32 = 2_000;

/// Why a well-formed submission was rejected. The agent receives it and submits again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SubmissionProblem {
    MissingArtifacts {
        kinds: Vec<ArtifactKind>,
    },
    UnexpectedArtifacts {
        kinds: Vec<ArtifactKind>,
    },
    DuplicateArtifact {
        artifact: ArtifactKind,
    },
    EmptyArtifact {
        artifact: ArtifactKind,
    },
    ArtifactTooLong {
        artifact: ArtifactKind,
        max_chars: u32,
    },
    ChapterPlanMissing,
    ChapterPlanNotExpected,
    ChapterPlanEmpty,
    ChapterPlanTooLong {
        max_items: u32,
    },
    ChapterTitleEmpty {
        number: u32,
    },
    ChapterTitleNotSingleLine {
        number: u32,
    },
    ChapterTitleTooLong {
        number: u32,
        max_chars: u32,
    },
    ChapterGoalEmpty {
        number: u32,
    },
    ChapterGoalTooLong {
        number: u32,
        max_chars: u32,
    },
    OutlineEmpty,
    OutlineTooLong {
        max_chars: u32,
    },
    SummaryEmpty,
    SummaryTooLong {
        max_chars: u32,
    },
    /// A chapter cannot be submitted before it has any body text.
    BodyEmpty,
    /// The submission names another chapter than the one being written.
    WrongChapter,
}

/// The arguments of `submit_chapter_outline`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct OutlineSubmission {
    pub chapter_id: ChapterId,
    pub content: String,
}

/// The arguments of `submit_chapter`. The summary describes the chapter for its review;
/// it is not body text.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ChapterSubmission {
    pub chapter_id: ChapterId,
    pub summary: String,
}

/// The outline text to store: line endings normalized and surrounding whitespace trimmed.
pub fn accept_outline(content: &str) -> Result<String, SubmissionProblem> {
    accept_text(
        content,
        OUTLINE_MAX_CHARS,
        SubmissionProblem::OutlineEmpty,
        SubmissionProblem::OutlineTooLong {
            max_chars: OUTLINE_MAX_CHARS,
        },
    )
}

/// The summary to store, if the chapter has body text to submit.
pub fn accept_chapter(summary: &str, body: &str) -> Result<String, SubmissionProblem> {
    if body.trim().is_empty() {
        return Err(SubmissionProblem::BodyEmpty);
    }
    accept_text(
        summary,
        SUMMARY_MAX_CHARS,
        SubmissionProblem::SummaryEmpty,
        SubmissionProblem::SummaryTooLong {
            max_chars: SUMMARY_MAX_CHARS,
        },
    )
}

fn accept_text(
    value: &str,
    max_chars: u32,
    empty: SubmissionProblem,
    too_long: SubmissionProblem,
) -> Result<String, SubmissionProblem> {
    let value = normalize_line_endings(value);
    let value = value.trim();
    if value.is_empty() {
        return Err(empty);
    }
    if value.chars().count() > max_chars as usize {
        return Err(too_long);
    }
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_outline_needs_text() {
        assert_eq!(
            accept_outline("  Scene one.\r\n "),
            Ok("Scene one.".to_owned())
        );
        assert_eq!(accept_outline(" \n"), Err(SubmissionProblem::OutlineEmpty));
    }

    #[test]
    fn a_chapter_needs_body_text_and_a_summary() {
        assert_eq!(
            accept_chapter("The lamp fails.", "\n"),
            Err(SubmissionProblem::BodyEmpty)
        );
        assert_eq!(
            accept_chapter(" ", "Text."),
            Err(SubmissionProblem::SummaryEmpty)
        );
        assert_eq!(
            accept_chapter("The lamp fails.", "Text."),
            Ok("The lamp fails.".to_owned())
        );
    }
}
