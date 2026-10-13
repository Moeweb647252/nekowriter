//! Documents: the reviewable, citable content of a version.

use sea_orm::{DeriveActiveEnum, EnumIter, sea_query::StringLen};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::preparation::ArtifactKind;

/// What a document holds. A version has one document of each version-level role; each
/// chapter has one of each chapter-level role.
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
pub enum DocumentRole {
    NovelBrief,
    Background,
    Research,
    Characters,
    Outline,
    /// What the user asked of a chapter.
    ChapterGuidance,
    ChapterOutline,
    ChapterBody,
}

impl DocumentRole {
    /// The roles every chapter has from its creation.
    pub const CHAPTER_ROLES: [Self; 3] = [
        Self::ChapterGuidance,
        Self::ChapterOutline,
        Self::ChapterBody,
    ];

    /// The roles every version has from its creation.
    pub const VERSION_ROLES: [Self; 5] = [
        Self::NovelBrief,
        Self::Background,
        Self::Research,
        Self::Characters,
        Self::Outline,
    ];
}

impl From<ArtifactKind> for DocumentRole {
    fn from(kind: ArtifactKind) -> Self {
        match kind {
            ArtifactKind::Background => Self::Background,
            ArtifactKind::Research => Self::Research,
            ArtifactKind::Characters => Self::Characters,
            ArtifactKind::Outline => Self::Outline,
        }
    }
}
