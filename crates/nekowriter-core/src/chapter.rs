//! Chapters: the units the novel is written in (`docs/product-spec.md` §7).

use sea_orm::{DeriveActiveEnum, DeriveValueType, EnumIter, sea_query::StringLen};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    field::{Field, FieldError, FieldProblem, multi_line, single_line_text},
    ids::ChapterId,
};

single_line_text!(
    /// The title of an extra chapter, which has no plan item to take it from.
    ChapterTitle,
    Field::ChapterTitle,
    200
);

/// What the user asks of a chapter before it is written. It may be empty.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS, DeriveValueType)]
#[ts(export)]
pub struct ChapterGuidance(String);

impl ChapterGuidance {
    pub const MAX_CHARS: u32 = 20_000;

    /// Validates like any multi-line text, except that empty guidance is allowed.
    pub fn parse(value: &str) -> Result<Self, FieldError> {
        match multi_line(Field::ChapterGuidance, value, Self::MAX_CHARS) {
            Ok(text) => Ok(Self(text)),
            Err(FieldError {
                problem: FieldProblem::Empty,
                ..
            }) => Ok(Self(String::new())),
            Err(error) => Err(error),
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a chapter's runs are told about the chapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChapterInput {
    pub id: ChapterId,
    pub kind: ChapterKind,
    /// The chapter's number among the chapters of its kind, starting at 1.
    pub number: i32,
    pub title: String,
    /// The plan item's goal; empty for extra chapters, whose guidance says what they are.
    pub goal: String,
}

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
pub enum ChapterKind {
    /// A chapter of the plan.
    Main,
    /// A chapter added after completion (§7).
    Extra,
}

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
pub enum ChapterStatus {
    NotStarted,
    InProgress,
    /// Its whole-chapter review approved its current body.
    Approved,
}
