//! Feedback the user gives a writer while it works (`docs/product-spec.md` §9.2).

use sea_orm::{DeriveActiveEnum, EnumIter, sea_query::StringLen};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::field::{Field, multi_line_text};

multi_line_text!(
    /// What the user tells the writer.
    FeedbackText,
    Field::Feedback,
    20_000
);

/// Where a piece of feedback is on its way to the model.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS, EnumIter, DeriveActiveEnum,
)]
#[sea_orm(
    rs_type = "String",
    db_type = "String(StringLen::None)",
    rename_all = "snake_case"
)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FeedbackStatus {
    /// Waiting for a tool result to travel with.
    Queued,
    /// Attached to a tool result that has not been sent yet.
    Bound,
    /// Sealed into a request; this says nothing about what the model made of it.
    Included,
}

/// What became of the request that carried a piece of feedback.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FeedbackDelivery {
    /// The request is in flight.
    Sent,
    Failed,
    /// The application stopped while the request was in flight.
    Unknown,
    /// The user stopped the run while the request was in flight.
    Cancelled,
}
