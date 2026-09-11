//! 内建提示词的系统级与项目级覆盖。
//!
//! 覆盖以「system + user」一对为单位保存：覆盖内容里的空字符串是合法的实际空
//! 内容，未覆盖的键没有对应的行。项目级覆盖优先于系统级，重置通过删除行完成，
//! 因此本表不需要「是否覆盖」这样的布尔字段。

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, Eq, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "prompt_overrides")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: TextUuid,
    /// `system` 或 `project`。
    pub scope: String,
    /// 系统级覆盖为 NULL；项目级覆盖指向所属项目。
    pub project_id: Option<String>,
    /// 内建提示词的稳定键，例如 `chapter.write`。
    pub prompt_key: String,
    pub system_content: String,
    pub user_content: String,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    #[sea_orm(belongs_to, from = "project_id", to = "id")]
    pub project: BelongsTo<Option<super::project::Entity>>,
}

impl ActiveModelBehavior for ActiveModel {}
