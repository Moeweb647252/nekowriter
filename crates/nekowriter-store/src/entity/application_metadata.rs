use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, Eq, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "application_metadata")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub key: String,
    pub value: String,
}

impl ActiveModelBehavior for ActiveModel {}
