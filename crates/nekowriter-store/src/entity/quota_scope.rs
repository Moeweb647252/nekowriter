//! 辅助工具预算作用域（细则 §7.1）。
//!
//! 一个作用域对应「准备阶段」或「某一章细纲」，只记录已用数。有效上限在每次工具
//! 执行前从项目最新设置读取，因此调整预算不会清空已用数，也不创建新的作用域。

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, Eq, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "quota_scopes")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: TextUuid,
    pub version_id: TextUuid,
    /// 准备阶段为 `preparation`，每章细纲为 `chapter_outline:<chapter_id>`。
    pub stage_key: String,
    pub used_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
    #[sea_orm(belongs_to, from = "version_id", to = "id")]
    pub version: BelongsTo<super::novel_version::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
