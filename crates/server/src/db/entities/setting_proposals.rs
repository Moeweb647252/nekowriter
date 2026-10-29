//! `setting_proposals` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

use crate::db::json::{ProposalStatus, ProposalTargetType, SettingDocKind};

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "setting_proposals")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub book_id: i32,
    pub target_type: ProposalTargetType,
    pub target_kind: Option<SettingDocKind>,
    pub target_id: Option<i32>,
    pub edits: String,
    pub rationale: String,
    pub status: ProposalStatus,
    pub chapter_id: Option<i32>,
    pub session_id: Option<i32>,
    pub created_at: DateTimeUtc,
    pub decided_at: Option<DateTimeUtc>,
    #[sea_orm(belongs_to, from = "book_id", to = "id")]
    pub book: BelongsTo<super::books::Entity>,
    #[sea_orm(belongs_to, from = "chapter_id", to = "id")]
    pub chapter: BelongsTo<Option<super::chapters::Entity>>,
    #[sea_orm(belongs_to, from = "session_id", to = "id")]
    pub session: BelongsTo<Option<super::agent_sessions::Entity>>,
}

impl ActiveModelBehavior for ActiveModel {}
