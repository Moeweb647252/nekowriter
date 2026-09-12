//! 知识条目。
//!
//! 正文与关键词属于 SQLite 主数据；向量只存在于 LanceDB，可由正文重建。条目每次
//! 内容变化都递增 `revision`，索引按该修订建立。

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, Eq, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "knowledge_entries")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: TextUuid,
    pub space_id: TextUuid,
    pub title: String,
    /// 关键词数组的 JSON 文本；空数组保存为 `[]`。
    pub keywords_json: String,
    pub content: String,
    /// 用户填写的来源说明，可为空。
    pub source_json: Option<String>,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
    #[sea_orm(belongs_to, from = "space_id", to = "id")]
    pub space: BelongsTo<super::knowledge_space::Entity>,
    #[sea_orm(has_many)]
    pub index_tasks: HasMany<super::knowledge_index_task::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
