//! The input of a run's next request: what the stage is about, assembled from stored data
//! by the store and rendered into the request by the agent (`docs/product-spec.md` §11).

use std::collections::BTreeSet;

use crate::preparation::ArtifactKind;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StageInput {
    Preparation {
        /// The current novel brief.
        brief: String,
        /// The artifacts to submit in this round: all four first, then the ones that were
        /// rejected or became stale.
        required: BTreeSet<ArtifactKind>,
    },
}
