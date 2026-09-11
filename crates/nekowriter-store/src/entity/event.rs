use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, Eq, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "events")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub seq: i64,
    pub version_id: Option<String>,
    #[sea_orm(column_name = "type")]
    pub event_type: String,
    pub entity_id: TextUuid,
    pub revision: String,
    pub payload_json: String,
    pub occurred_at: i64,
    #[sea_orm(belongs_to, from = "version_id", to = "id")]
    pub version: BelongsTo<Option<super::novel_version::Entity>>,
}

impl ActiveModelBehavior for ActiveModel {}
