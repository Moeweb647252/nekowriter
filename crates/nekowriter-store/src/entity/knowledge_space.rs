//! 知识 Space。
//!
//! Space 只保存标题与修订号，条目通过外键归属它。删除 Space 前必须为空，因此不
//! 需要级联删除条目。

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, Eq, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "knowledge_spaces")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: TextUuid,
    pub title: String,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
    #[sea_orm(has_many)]
    pub entries: HasMany<super::knowledge_entry::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
