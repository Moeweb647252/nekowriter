//! Token usage of a model request (`docs/product-spec.md` §15).

use sea_orm::FromJsonQueryResult;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The token counts the model service reported. A count it did not report is `None`,
/// which means unknown, never zero (§4 invariant 13).
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, FromJsonQueryResult,
)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TokenUsage {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    /// The part of the input served from the provider's cache.
    pub cached_input_tokens: Option<i64>,
    /// The part of the output spent on reasoning.
    pub reasoning_tokens: Option<i64>,
}
