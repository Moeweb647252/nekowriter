//! Tool calls as data: what a valid call asks for (`docs/product-spec.md` §8.2, §9.1).
//!
//! A call is stored with its parsed request, so a call that is held (by a pause, and
//! later by per-edit review or a question) can be carried out later exactly as the model
//! made it.

use sea_orm::FromJsonQueryResult;
use serde::{Deserialize, Serialize};

use crate::{
    body::BodyEdit,
    preparation::PreparationSubmission,
    question::Question,
    submission::{ChapterSubmission, OutlineSubmission, VerdictSubmission},
};

/// A tool call that passed the protocol checks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, FromJsonQueryResult)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum CallRequest {
    /// `append_content` or `replace_content`.
    Edit { edit: BodyEdit },
    /// `ask_user`: the run waits for the user's answers.
    Ask { questions: Vec<Question> },
    /// `query_knowledge`: a semantic search of the knowledge base, in the named spaces or
    /// in all of them. The application carries it out between rounds (§12.1, §12.2).
    QueryKnowledge { query: String, spaces: Vec<String> },
    /// The stage's submission tool.
    Submit { submission: StageSubmission },
}

/// A submission that ends a stage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "stage",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum StageSubmission {
    Preparation { submission: PreparationSubmission },
    ChapterOutline { submission: OutlineSubmission },
    Chapter { submission: ChapterSubmission },
    Review { verdict: VerdictSubmission },
}
