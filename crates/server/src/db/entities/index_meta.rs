//! `index_meta` entity (spec/02 §2). Key/value bookkeeping for the active embedding model.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "index_meta")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_name = "key")]
    pub key: String,
    pub value: String,
}

impl ActiveModelBehavior for ActiveModel {}
