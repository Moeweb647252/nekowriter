//! `writing_skills` entity (spec/02 §2, design §4).

use sea_orm::entity::prelude::*;

/// Who authored a writing skill (spec/02 §3): built-in rows are read-only, user rows editable.
#[derive(Debug, Clone, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub enum WritingSkillAuthor {
    #[sea_orm(string_value = "builtin")]
    Builtin,
    #[sea_orm(string_value = "user")]
    User,
}

impl WritingSkillAuthor {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::User => "user",
        }
    }
}

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "writing_skills")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    /// Built-in only; `NULL` for user rows (design §4).
    #[sea_orm(unique)]
    pub key: Option<String>,
    pub name: String,
    pub category: String,
    pub author: WritingSkillAuthor,
    pub description: String,
    pub body: String,
    pub enabled_by_default: i32,
    pub retired: i32,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    #[sea_orm(has_many, relation_enum = "Book", via = "book_writing_skills::Skill")]
    pub books: HasMany<super::books::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
