use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, Eq, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "command_receipts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub command_id: TextUuid,
    pub method: String,
    pub target: String,
    pub payload_hash: String,
    pub response_status: i64,
    pub response_json: String,
    pub created_at: i64,
}

impl ActiveModelBehavior for ActiveModel {}
