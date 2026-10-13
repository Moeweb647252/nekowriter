//! Projects and the values that describe them.

use sea_orm::{DeriveActiveEnum, EnumIter, sea_query::StringLen};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::field::{Field, single_line_text};

single_line_text!(
    /// The name of a project. A project name is not the title of its novel.
    ProjectName,
    Field::ProjectName,
    100
);

single_line_text!(
    /// The title of the novel a version contains.
    NovelTitle,
    Field::NovelTitle,
    200
);

/// The language the agents write a project's novel in.
///
/// This is independent of the language of the user interface: the UI language belongs to
/// the person using the app, the writing language belongs to the novel.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS, EnumIter, DeriveActiveEnum,
)]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::None)")]
#[ts(export)]
pub enum WritingLanguage {
    /// Simplified Chinese.
    #[serde(rename = "zh-CN")]
    #[sea_orm(string_value = "zh-CN")]
    ZhCn,
    /// English.
    #[serde(rename = "en")]
    #[sea_orm(string_value = "en")]
    En,
}
