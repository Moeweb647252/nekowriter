//! 用户提示词模板。
//!
//! 模板是独立可复用内容，不会自动获得 system 消息权限；变量声明与正文一起保存，
//! 每次修改递增 `revision`。删除为软删除，为回收站恢复保留记录。

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, Eq, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "prompt_templates")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: TextUuid,
    pub title: String,
    pub body: String,
    /// `TemplateVariable` 数组的 JSON 文本；空数组保存为 `[]`。
    pub variables_json: String,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
}

impl ActiveModelBehavior for ActiveModel {}
