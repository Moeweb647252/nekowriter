//! 系统级 Embedding 配置。
//!
//! 与写作模型设置分开保存：Embedding 的服务地址、模型与输出维度共同决定向量
//! 空间，改变任一项都必须建立新的 profile 索引。凭据正文不进入 SQLite，只保存
//! OS 凭据库的不透明引用。

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, Eq, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "embedding_settings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub key: String,
    pub api_base_url: String,
    pub credential_ref: Option<String>,
    pub model_id: String,
    pub dimension: i64,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

impl ActiveModelBehavior for ActiveModel {}
