//! 系统级写作模型配置。
//!
//! `api_key` 是模型服务的密钥正文，只保存在本地 SQLite 中，且从不进入 HTTP
//! 响应、SSE 事件、模型请求快照或 tracing 字段。模型能力、上下文窗口和默认模型
//! ID 则是可诊断的非敏感配置。

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, Eq, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "system_model_settings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub key: String,
    pub api_base_url: String,
    pub api_key: Option<String>,
    pub model_id: String,
    pub capabilities_json: String,
    pub max_context_tokens: i64,
    pub reserved_output_tokens: i64,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

impl ActiveModelBehavior for ActiveModel {}
