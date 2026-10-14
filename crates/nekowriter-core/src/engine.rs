//! Research and knowledge accumulation engines (`docs/product-spec.md` §12.3, D11): the
//! contracts an engine fulfils and what the application tells clients about them. The
//! MVP registers no engine.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{ids::KnowledgeSpaceId, project::WritingLanguage, time::Timestamp, usage::TokenUsage};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum EngineKind {
    /// Answers questions from the web; it never writes novel text.
    Research,
    /// Finds knowledge entries from the user's hints.
    Accumulation,
}

/// A registered engine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EngineInfo {
    pub id: String,
    pub name: String,
}

/// The engines of one kind, and why there are none when the list is empty.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EngineList {
    pub engines: Vec<EngineInfo>,
    pub unavailable: Option<EngineUnavailable>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum EngineUnavailable {
    /// This build registers no engine of the kind.
    NoneRegistered,
}

/// What a research engine is asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResearchRequest {
    pub question: String,
    /// What the user suggested looking at.
    pub hints: String,
    pub language: WritingLanguage,
    /// What the run is working on, so the answer fits it.
    pub context: String,
}

/// What a research engine found, with the sources and what it could not establish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResearchReport {
    pub findings: String,
    pub sources: Vec<Source>,
    pub limitations: String,
    pub usage: Option<TokenUsage>,
}

/// Where a finding comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub url: String,
    pub title: String,
    pub accessed_at: Timestamp,
}

/// What an accumulation engine is asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccumulationRequest {
    pub hints: String,
    pub space: KnowledgeSpaceId,
}

/// An entry an accumulation engine proposes; it is validated like a manual entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub title: String,
    pub keywords: Vec<String>,
    pub content: String,
    pub sources: Vec<Source>,
}
