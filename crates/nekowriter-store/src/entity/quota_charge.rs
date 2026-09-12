//! 辅助工具预算消耗记录（细则 §7.1）。
//!
//! 以 `(scope_id, tool_call_id)` 为唯一键：同一次工具调用即使在重放或重试中被重复
//! 处理，也只登记一次消耗。

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, Eq, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "quota_charges")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: TextUuid,
    pub scope_id: TextUuid,
    /// 供应商返回的工具调用 ID，重放时保持不变。
    pub tool_call_id: String,
    pub tool_name: String,
    pub charged_at: i64,
    #[sea_orm(belongs_to, from = "scope_id", to = "id")]
    pub scope: BelongsTo<super::quota_scope::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
