//! Preparation: the novel brief, the four artifacts, their dependencies and the chapter
//! plan (`docs/product-spec.md` §6).

use std::collections::BTreeSet;

use sea_orm::FromJsonQueryResult;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    field::{Field, multi_line_text, normalize_line_endings},
    ids::PlanItemId,
};

multi_line_text!(
    /// What the user wants the novel to be. Preparation starts from it.
    NovelBrief,
    Field::NovelBrief,
    20_000
);

/// One of the four preparation artifacts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ArtifactKind {
    Background,
    Research,
    Characters,
    Outline,
}

impl ArtifactKind {
    /// Every artifact, in the order of the review tabs and of the folded text.
    pub const ALL: [Self; 4] = [
        Self::Background,
        Self::Research,
        Self::Characters,
        Self::Outline,
    ];

    /// The artifacts this one is written from: research feeds everything else, the
    /// background feeds the characters and the outline, and the characters feed the outline.
    #[must_use]
    pub fn upstream(self) -> &'static [Self] {
        match self {
            Self::Research => &[],
            Self::Background => &[Self::Research],
            Self::Characters => &[Self::Research, Self::Background],
            Self::Outline => &[Self::Research, Self::Background, Self::Characters],
        }
    }
}

/// The longest artifact the agent may submit, in characters.
pub const ARTIFACT_MAX_CHARS: u32 = 100_000;
/// The most chapters a plan may have.
pub const PLAN_MAX_ITEMS: u32 = 500;
pub const PLAN_TITLE_MAX_CHARS: u32 = 200;
pub const PLAN_GOAL_MAX_CHARS: u32 = 2_000;

/// The ordered chapters the novel is planned to have. It is part of the outline artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, FromJsonQueryResult)]
#[ts(export)]
pub struct ChapterPlan(pub Vec<ChapterPlanItem>);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChapterPlanItem {
    pub id: PlanItemId,
    pub title: String,
    pub goal: String,
}

/// A preparation submission with the shape the submission tool requires. Its content has
/// not been checked against the business rules yet.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct PreparationSubmission {
    pub artifacts: Vec<ArtifactDraft>,
    #[serde(default)]
    pub chapter_plan: Option<Vec<PlanItemDraft>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ArtifactDraft {
    pub kind: ArtifactKind,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct PlanItemDraft {
    pub title: String,
    pub goal: String,
}

/// A submission that passed the business rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcceptedPreparation {
    /// The submitted artifacts with normalized content, in [`ArtifactKind::ALL`] order.
    pub artifacts: Vec<(ArtifactKind, String)>,
    /// Present exactly when the outline is submitted.
    pub chapter_plan: Option<ChapterPlan>,
}

/// Why a well-formed submission was rejected. The agent receives it and resubmits.
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
}

/// Checks a submission against the artifacts `required` this round (§6).
pub fn accept_preparation(
    required: &BTreeSet<ArtifactKind>,
    submission: PreparationSubmission,
) -> Result<AcceptedPreparation, SubmissionProblem> {
    let mut submitted = BTreeSet::new();
    for draft in &submission.artifacts {
        if !submitted.insert(draft.kind) {
            return Err(SubmissionProblem::DuplicateArtifact {
                artifact: draft.kind,
            });
        }
    }
    let missing: Vec<_> = required.difference(&submitted).copied().collect();
    if !missing.is_empty() {
        return Err(SubmissionProblem::MissingArtifacts { kinds: missing });
    }
    let unexpected: Vec<_> = submitted.difference(required).copied().collect();
    if !unexpected.is_empty() {
        return Err(SubmissionProblem::UnexpectedArtifacts { kinds: unexpected });
    }

    let mut artifacts = Vec::with_capacity(submission.artifacts.len());
    for draft in submission.artifacts {
        let content = normalize_line_endings(&draft.content);
        let content = content.trim();
        if content.is_empty() {
            return Err(SubmissionProblem::EmptyArtifact {
                artifact: draft.kind,
            });
        }
        if content.chars().count() > ARTIFACT_MAX_CHARS as usize {
            return Err(SubmissionProblem::ArtifactTooLong {
                artifact: draft.kind,
                max_chars: ARTIFACT_MAX_CHARS,
            });
        }
        artifacts.push((draft.kind, content.to_owned()));
    }
    artifacts.sort_by_key(|(kind, _)| *kind);

    let chapter_plan = match (
        required.contains(&ArtifactKind::Outline),
        submission.chapter_plan,
    ) {
        (true, None) => return Err(SubmissionProblem::ChapterPlanMissing),
        (false, Some(_)) => return Err(SubmissionProblem::ChapterPlanNotExpected),
        (false, None) => None,
        (true, Some(items)) => Some(accept_plan(items)?),
    };
    Ok(AcceptedPreparation {
        artifacts,
        chapter_plan,
    })
}

fn accept_plan(items: Vec<PlanItemDraft>) -> Result<ChapterPlan, SubmissionProblem> {
    if items.is_empty() {
        return Err(SubmissionProblem::ChapterPlanEmpty);
    }
    if items.len() > PLAN_MAX_ITEMS as usize {
        return Err(SubmissionProblem::ChapterPlanTooLong {
            max_items: PLAN_MAX_ITEMS,
        });
    }
    let mut accepted = Vec::with_capacity(items.len());
    for (index, item) in items.into_iter().enumerate() {
        let number = u32::try_from(index + 1).expect("the plan size is bounded");
        let title = item.title.trim();
        if title.is_empty() {
            return Err(SubmissionProblem::ChapterTitleEmpty { number });
        }
        if title.chars().any(char::is_control) {
            return Err(SubmissionProblem::ChapterTitleNotSingleLine { number });
        }
        if title.chars().count() > PLAN_TITLE_MAX_CHARS as usize {
            return Err(SubmissionProblem::ChapterTitleTooLong {
                number,
                max_chars: PLAN_TITLE_MAX_CHARS,
            });
        }
        let goal = normalize_line_endings(&item.goal);
        let goal = goal.trim();
        if goal.is_empty() {
            return Err(SubmissionProblem::ChapterGoalEmpty { number });
        }
        if goal.chars().count() > PLAN_GOAL_MAX_CHARS as usize {
            return Err(SubmissionProblem::ChapterGoalTooLong {
                number,
                max_chars: PLAN_GOAL_MAX_CHARS,
            });
        }
        accepted.push(ChapterPlanItem {
            id: PlanItemId::generate(),
            title: title.to_owned(),
            goal: goal.to_owned(),
        });
    }
    Ok(ChapterPlan(accepted))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(kind: ArtifactKind) -> ArtifactDraft {
        ArtifactDraft {
            kind,
            content: format!("{kind:?} text\r\n"),
        }
    }

    fn plan() -> Vec<PlanItemDraft> {
        vec![PlanItemDraft {
            title: "Arrival".to_owned(),
            goal: "The letter arrives.".to_owned(),
        }]
    }

    fn all() -> BTreeSet<ArtifactKind> {
        ArtifactKind::ALL.into_iter().collect()
    }

    #[test]
    fn a_complete_first_round_is_accepted_in_tab_order() {
        let submission = PreparationSubmission {
            artifacts: [
                ArtifactKind::Outline,
                ArtifactKind::Research,
                ArtifactKind::Background,
                ArtifactKind::Characters,
            ]
            .map(draft)
            .to_vec(),
            chapter_plan: Some(plan()),
        };

        let accepted = accept_preparation(&all(), submission).unwrap();

        let kinds: Vec<_> = accepted.artifacts.iter().map(|(kind, _)| *kind).collect();
        assert_eq!(kinds, ArtifactKind::ALL.to_vec());
        assert_eq!(accepted.artifacts[0].1, "Background text");
        assert_eq!(accepted.chapter_plan.unwrap().0[0].title, "Arrival");
    }

    #[test]
    fn a_revision_round_accepts_exactly_the_required_artifacts() {
        let required = BTreeSet::from([ArtifactKind::Characters]);
        let extra = PreparationSubmission {
            artifacts: vec![
                draft(ArtifactKind::Characters),
                draft(ArtifactKind::Background),
            ],
            chapter_plan: None,
        };
        assert_eq!(
            accept_preparation(&required, extra).unwrap_err(),
            SubmissionProblem::UnexpectedArtifacts {
                kinds: vec![ArtifactKind::Background]
            }
        );

        let missing = PreparationSubmission {
            artifacts: Vec::new(),
            chapter_plan: None,
        };
        assert_eq!(
            accept_preparation(&required, missing).unwrap_err(),
            SubmissionProblem::MissingArtifacts {
                kinds: vec![ArtifactKind::Characters]
            }
        );
    }

    #[test]
    fn the_chapter_plan_comes_with_the_outline_and_only_with_it() {
        let outline_only = BTreeSet::from([ArtifactKind::Outline]);
        let without_plan = PreparationSubmission {
            artifacts: vec![draft(ArtifactKind::Outline)],
            chapter_plan: None,
        };
        assert_eq!(
            accept_preparation(&outline_only, without_plan).unwrap_err(),
            SubmissionProblem::ChapterPlanMissing
        );

        let background_only = BTreeSet::from([ArtifactKind::Background]);
        let with_plan = PreparationSubmission {
            artifacts: vec![draft(ArtifactKind::Background)],
            chapter_plan: Some(plan()),
        };
        assert_eq!(
            accept_preparation(&background_only, with_plan).unwrap_err(),
            SubmissionProblem::ChapterPlanNotExpected
        );
    }

    #[test]
    fn empty_content_and_empty_plan_entries_are_rejected() {
        let outline_only = BTreeSet::from([ArtifactKind::Outline]);
        let empty_content = PreparationSubmission {
            artifacts: vec![ArtifactDraft {
                kind: ArtifactKind::Outline,
                content: " \n ".to_owned(),
            }],
            chapter_plan: Some(plan()),
        };
        assert_eq!(
            accept_preparation(&outline_only, empty_content).unwrap_err(),
            SubmissionProblem::EmptyArtifact {
                artifact: ArtifactKind::Outline
            }
        );

        let untitled = PreparationSubmission {
            artifacts: vec![draft(ArtifactKind::Outline)],
            chapter_plan: Some(vec![PlanItemDraft {
                title: " ".to_owned(),
                goal: "Something happens.".to_owned(),
            }]),
        };
        assert_eq!(
            accept_preparation(&outline_only, untitled).unwrap_err(),
            SubmissionProblem::ChapterTitleEmpty { number: 1 }
        );
    }

    #[test]
    fn every_artifact_reads_only_from_artifacts_before_it_in_dependency_order() {
        assert_eq!(ArtifactKind::Research.upstream(), &[] as &[ArtifactKind]);
        assert!(
            ArtifactKind::Outline
                .upstream()
                .contains(&ArtifactKind::Characters)
        );
        assert!(
            !ArtifactKind::Background
                .upstream()
                .contains(&ArtifactKind::Characters)
        );
    }
}
