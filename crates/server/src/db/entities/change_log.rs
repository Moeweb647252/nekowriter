//! `change_log` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "change_log")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub book_id: i32,
    pub track_id: Option<i32>,
    pub chapter_id: Option<i32>,
    pub session_id: Option<i32>,
    pub actor: String,
    pub target_type: String,
    pub target_id: i32,
    pub op: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub note: String,
    pub reverts_id: Option<i32>,
    pub reverted_by_id: Option<i32>,
    pub created_at: DateTimeUtc,
    #[sea_orm(belongs_to, from = "book_id", to = "id")]
    pub book: BelongsTo<super::books::Entity>,
    #[sea_orm(belongs_to, from = "track_id", to = "id")]
    pub track: BelongsTo<Option<super::tracks::Entity>>,
    #[sea_orm(belongs_to, from = "chapter_id", to = "id")]
    pub chapter: BelongsTo<Option<super::chapters::Entity>>,
    #[sea_orm(belongs_to, from = "session_id", to = "id")]
    pub session: BelongsTo<Option<super::agent_sessions::Entity>>,
    #[sea_orm(self_ref, relation_enum = "Reverts", from = "reverts_id", to = "id")]
    pub reverts: BelongsTo<Option<Entity>>,
    #[sea_orm(
        self_ref,
        relation_enum = "RevertedBy",
        from = "reverted_by_id",
        to = "id"
    )]
    pub reverted_by: BelongsTo<Option<Entity>>,
}

impl ActiveModelBehavior for ActiveModel {}
