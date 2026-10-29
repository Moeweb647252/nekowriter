//! `app_settings` entity (spec/02 §2). The primary key is a `TEXT` key.
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "app_settings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_name = "key")]
    pub key: String,
    pub value: String,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}
