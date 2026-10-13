//! The input of a run's next request: what the stage is about and the folds of the stages
//! before it, assembled from stored data by the store and rendered into the request by the
//! agent (`docs/product-spec.md` §11).

use std::collections::BTreeSet;

use crate::{
    chapter::ChapterInput, document::DocumentRole, preparation::ArtifactKind,
    preparation::ChapterPlan,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StageInput {
    Preparation {
        /// The current novel brief.
        brief: String,
        /// The artifacts to submit in this round: all four first, then the ones that were
        /// rejected or became stale.
        required: BTreeSet<ArtifactKind>,
    },
    ChapterOutline {
        chapter: ChapterInput,
        guidance: String,
    },
    ChapterBody {
        chapter: ChapterInput,
        guidance: String,
        /// The chapter's current body.
        body: String,
    },
}

/// A completed stage's approved text, which later stages carry instead of its history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoldInput {
    pub stage: FoldStage,
    /// The approved revisions, in the order the fold presents them.
    pub documents: Vec<FoldDocument>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FoldStage {
    Preparation,
    ChapterOutline { chapter: ChapterInput },
    ChapterBody { chapter: ChapterInput },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoldDocument {
    pub role: DocumentRole,
    pub content: String,
    pub chapter_plan: Option<ChapterPlan>,
}
