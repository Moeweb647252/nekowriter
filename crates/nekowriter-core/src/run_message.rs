//! The conversation of a run with the model after its stage input
//! (`docs/product-spec.md` §8.2, §9.1).
//!
//! Messages are stored as data. Those the application writes (tool results, review
//! outcomes, corrections) are rendered into model text once, when the request that first
//! carries them is sealed, and never rendered again, so a message the model has seen never
//! changes.

use sea_orm::FromJsonQueryResult;
use serde::{Deserialize, Serialize};

use crate::{
    body::EditProblem,
    ids::{ReviewId, RevisionId, ToolCallId},
    preparation::ArtifactKind,
    review::ReviewKind,
    submission::SubmissionProblem,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, FromJsonQueryResult)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum RunMessageBody {
    /// What the model answered, as it answered it.
    Assistant {
        content: Option<String>,
        tool_calls: Vec<ModelToolCall>,
    },
    /// The result of one tool call, rendered from the call's outcome.
    ToolResult { tool_call_id: ToolCallId },
    /// The decisions of a preparation review round that rejected or invalidated artifacts.
    PreparationReviewOutcome { round: i32 },
    /// A rejected chapter outline or chapter, with the reviewer's feedback.
    ReviewRejected { review_id: ReviewId },
    /// Tells the model how to get back on protocol.
    Correction { correction: Correction },
}

/// A message as the model sees it. Assistant messages are stored this way when they
/// arrive; the others when they are rendered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, FromJsonQueryResult)]
#[serde(
    tag = "role",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum TranscriptMessage {
    Assistant {
        content: Option<String>,
        tool_calls: Vec<ModelToolCall>,
    },
    Tool {
        provider_call_id: String,
        content: String,
    },
    User {
        content: String,
    },
}

/// A message that has not been rendered yet, with the data it refers to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnsealedMessage {
    ToolResult {
        provider_call_id: String,
        outcome: ToolOutcome,
        /// The user's feedback that travels with this result, in order.
        feedback: Vec<String>,
    },
    PreparationReviewOutcome {
        rejected: Vec<RejectedArtifact>,
        stale: Vec<ArtifactKind>,
    },
    ReviewRejected {
        kind: ReviewKind,
        feedback: String,
    },
    Correction {
        correction: Correction,
    },
}

/// A rejected artifact and the reviewer's feedback on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RejectedArtifact {
    pub artifact: ArtifactKind,
    pub feedback: String,
}

/// A tool call exactly as the model sent it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelToolCall {
    /// The call's identifier, chosen by the model service.
    pub provider_id: String,
    pub name: String,
    /// The arguments as JSON text, unparsed.
    pub arguments: String,
}

/// Why the model receives a correction instead of progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Correction {
    /// The answer contained no tool call; the stage can only end through its submission
    /// tool.
    NoToolCall,
    /// A tool call broke the protocol; its result says how.
    ProtocolError,
    /// A submission was well formed but broke a business rule; its result says which.
    SubmissionRejected,
}

/// What happened to a tool call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, FromJsonQueryResult)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ToolOutcome {
    /// The stage submission was accepted and waits for review.
    SubmissionAccepted,
    /// The stage submission broke a business rule.
    SubmissionRejected { problem: SubmissionProblem },
    /// The call broke the protocol.
    ProtocolError { problem: ProtocolProblem },
    /// The body edit was committed as a new revision.
    EditApplied {
        base_revision: Option<RevisionId>,
        revision: RevisionId,
        word_count: u32,
    },
    /// The replacement equals the text it replaces; nothing changed.
    EditNoChange { word_count: u32 },
    /// The body edit could not be applied; the body is unchanged.
    EditRejected { problem: EditProblem },
}

/// How a tool call broke the protocol (§8.1, §8.4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ProtocolProblem {
    /// The tool is not declared in this round.
    UnknownTool { name: String },
    /// The answer contained several tool calls; none was run.
    ParallelToolCalls,
    /// The arguments do not match the tool's schema.
    InvalidArguments { problem: ArgumentsProblem },
}

/// Where and how tool arguments differ from the tool's schema.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ArgumentsProblem {
    /// The arguments are not valid JSON.
    NotJson,
    /// A required field is missing. `path` locates its parent object (empty for the
    /// arguments object itself).
    MissingField { path: String, field: String },
    /// A value has the wrong JSON type.
    WrongType { path: String, expected: JsonType },
    /// A value is not one of the allowed values.
    NotAllowed {
        path: String,
        value: String,
        allowed: Vec<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JsonType {
    Object,
    Array,
    String,
    Integer,
    Number,
    Boolean,
}
