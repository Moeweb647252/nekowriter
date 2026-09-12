//! 知识索引任务。
//!
//! 任务是「把某条目某个修订向量化到某个 Embedding profile」的幂等记录；同一
//! `entry_id + entry_revision + embedding_profile_id` 只有一行。任务完成后由
//! Store 依据条目当前修订判断是否仍可标记 ready，避免旧任务覆盖新修订。

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, Eq, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "knowledge_index_tasks")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: TextUuid,
    pub entry_id: TextUuid,
    pub entry_revision: i64,
    pub embedding_profile_id: String,
    /// `upsert`；保留字段以便未来区分物理清理等操作。
    pub operation: String,
    /// `pending` / `running` / `ready` / `failed`。
    pub status: String,
    pub error: Option<String>,
    pub checkpoint_json: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    #[sea_orm(belongs_to, from = "entry_id", to = "id")]
    pub entry: BelongsTo<super::knowledge_entry::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
