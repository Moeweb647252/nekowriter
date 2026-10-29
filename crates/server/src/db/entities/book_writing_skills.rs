//! `book_writing_skills` join table entity (spec/02 §2, design §4).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "book_writing_skills")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub book_id: i32,
    #[sea_orm(primary_key, auto_increment = false)]
    pub skill_id: i32,
    pub created_at: DateTimeUtc,
    #[sea_orm(belongs_to, from = "book_id", to = "id")]
    pub book: BelongsTo<super::books::Entity>,
    #[sea_orm(belongs_to, from = "skill_id", to = "id")]
    pub skill: BelongsTo<super::writing_skills::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
