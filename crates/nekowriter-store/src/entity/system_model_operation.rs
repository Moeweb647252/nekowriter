//! 系统模型操作。
//!
//! 连接测试这类请求真的会发出一次模型调用，但它们不属于任何稿本，因此不能写进
//! 以 Run 为主键的 `model_requests`，也不能从统计里消失。它们单独成表，并带上与
//! 对话请求相同的用量口径（来源、状态、原始字段），使 Dashboard 能把它们单列。

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, Eq, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "system_model_operations")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: TextUuid,
    /// 操作类型；目前只有 `connection_test`。
    pub kind: String,
    pub model_id: String,
    /// 实际请求的服务根地址，不含凭据；用于说明这次测的是哪个服务。
    pub api_base_url: String,
    /// `succeeded` / `failed` / `unknown`。`unknown` 表示连接或超时失败，
    /// 请求可能已经到达供应商。
    pub status: String,
    pub provider_request_id: Option<String>,
    pub usage_json: Option<String>,
    /// 失败时为 `{"kind": ..., "message": ...}`。
    pub error_json: Option<String>,
    pub created_at: i64,
    pub completed_at: Option<i64>,
}

impl ActiveModelBehavior for ActiveModel {}
