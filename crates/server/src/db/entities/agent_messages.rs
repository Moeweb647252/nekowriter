//! `agent_messages` entity (spec/02 §2).

use sea_orm::entity::prelude::*;

use crate::db::json::{
    JsonText, MessageKind, MessageRole, ProviderPayload, RequestMeta, StoredBlock, Usage,
};

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "agent_messages")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    #[sea_orm(unique_key = "session_seq")]
    pub session_id: i32,
    #[sea_orm(unique_key = "session_seq")]
    pub seq: i32,
    pub role: MessageRole,
    pub kind: MessageKind,
    pub content: JsonText<Vec<StoredBlock>>,
    pub provider_payload: Option<JsonText<ProviderPayload>>,
    pub request_meta: Option<JsonText<RequestMeta>>,
    pub stop_reason: Option<String>,
    pub usage: Option<JsonText<Usage>>,
    pub created_at: DateTimeUtc,
    #[sea_orm(belongs_to, from = "session_id", to = "id")]
    pub session: BelongsTo<super::agent_sessions::Entity>,
    #[sea_orm(has_many)]
    pub agent_inbox: HasMany<super::agent_inbox::Entity>,
    #[sea_orm(has_many)]
    pub agent_tool_calls: HasMany<super::agent_tool_calls::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
