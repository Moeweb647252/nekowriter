//! 系统级写作模型配置。
//!
//! 凭据正文不进入 SQLite：`credential_ref` 只是应用层用来访问 OS 凭据库的
//! 不透明引用。模型能力、上下文窗口和默认模型 ID 则是可诊断的非敏感配置。

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, Eq, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "system_model_settings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub key: String,
    pub api_base_url: String,
    pub credential_ref: Option<String>,
    pub model_id: String,
    pub capabilities_json: String,
    pub max_context_tokens: i64,
    pub reserved_output_tokens: i64,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

impl ActiveModelBehavior for ActiveModel {}
