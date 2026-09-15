//! Agent 原始消息的不可变账本。
//!
//! `model_requests.context_snapshot` 记录某一次真正发送的完整输入；本模块则保留
//! 每个 Run 的原始消息序列，供后续请求组装、阶段折叠水位和中断诊断使用。Store
//! 不依赖具体模型协议，因此 `content` 是经过 JSON object 边界校验的传输消息包；
//! App/Agent 在写入和读取时负责将其编码为对应的模型消息类型。

use nekowriter_core::{RunId, decimal_i64, utc_now_millis};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::{
    Store, StoreError, db_id,
    entity::{agent_message, run},
    transaction_error,
};

const ROLE_SYSTEM: &str = "system";
const ROLE_USER: &str = "user";
const ROLE_ASSISTANT: &str = "assistant";
const ROLE_TOOL: &str = "tool";

/// `agent_messages.role` 的稳定值。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMessageRole {
    System,
    User,
    Assistant,
    Tool,
}

/// 待追加到某个 Run 的一条原始传输消息。
///
/// `content` 必须是 JSON object。它通常是 Agent 协议的完整消息 envelope，而不是
/// 散落的流式 token；凭据、日志上下文和任意可执行状态都不得写入此字段。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewAgentMessage {
    pub role: AgentMessageRole,
    pub content: Value,
    pub request_id: Option<Uuid>,
    pub tool_call_id: Option<String>,
}

/// 已持久化的 Agent 原始消息。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedAgentMessage {
    pub id: Uuid,
    pub run_id: RunId,
    #[serde(with = "decimal_i64")]
    pub sequence: i64,
    pub role: AgentMessageRole,
    pub content: Value,
    pub request_id: Option<Uuid>,
    pub tool_call_id: Option<String>,
    #[serde(with = "decimal_i64")]
    pub created_at_ms: i64,
}

impl Store {
    /// 以连续、不可变的序号原子追加一批 Agent 消息。
    ///
    /// # Errors
    ///
    /// Run 不存在或已终止、消息 envelope 不合法、序号溢出或数据库写入失败时返回
    /// 错误。调用成功后没有更新或删除接口；修订、折叠和请求快照都只引用这些已
    /// 封存的消息。
    pub async fn append_agent_messages(
        &self,
        run_id: RunId,
        messages: Vec<NewAgentMessage>,
    ) -> Result<Vec<PersistedAgentMessage>, StoreError> {
        validate_new_messages(&messages)?;
        let now = utc_now_millis();
        self.database
            .transaction::<_, _, StoreError>(|transaction| {
                Box::pin(async move {
                    let run_model = run::Entity::find_by_id(db_id(run_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    ensure_run_accepts_messages(&run_model)?;
                    append_agent_messages_for_run(transaction, &run_model, messages, now).await
                })
            })
            .await
            .map_err(transaction_error)
    }

    /// 以稳定 sequence 顺序读取某个 Run 的完整原始消息账本。
    ///
    /// # Errors
    ///
    /// Run 不存在、存储消息 JSON/UUID 损坏或数据库读取失败时返回错误。
    pub async fn list_agent_messages(
        &self,
        run_id: RunId,
    ) -> Result<Vec<PersistedAgentMessage>, StoreError> {
        if run::Entity::find_by_id(db_id(run_id))
            .one(&self.database)
            .await?
            .is_none()
        {
            return Err(StoreError::RunNotFound);
        }
        agent_message::Entity::find()
            .filter(agent_message::Column::RunId.eq(db_id(run_id)))
            .order_by_asc(agent_message::Column::Sequence)
            .all(&self.database)
            .await?
            .iter()
            .map(persisted_agent_message_from_model)
            .collect()
    }
}

pub(crate) fn validate_new_messages(messages: &[NewAgentMessage]) -> Result<(), StoreError> {
    if messages.is_empty() {
        return Err(StoreError::InvalidAgentMessage(
            "至少需要追加一条消息".to_owned(),
        ));
    }
    for message in messages {
        if !message.content.is_object() {
            return Err(StoreError::InvalidAgentMessage(
                "消息 content 必须是 JSON object".to_owned(),
            ));
        }
        if message
            .tool_call_id
            .as_deref()
            .is_some_and(|id| id.trim().is_empty())
        {
            return Err(StoreError::InvalidAgentMessage(
                "tool_call_id 不能是空字符串".to_owned(),
            ));
        }
        if message.role == AgentMessageRole::Tool && message.tool_call_id.is_none() {
            return Err(StoreError::InvalidAgentMessage(
                "tool 消息必须关联 tool_call_id".to_owned(),
            ));
        }
    }
    Ok(())
}

/// 阶段提交在消息账本里的两条记录：提交工具的调用，以及它配对的结果。
///
/// 阶段结果是一次声明过的提交工具调用，所以账本必须同时保留调用与结果：重放会
/// 用 `serialize_request_messages` 校验配对，只有调用而没有结果的序列会被判为
/// `MissingToolResult`。成功提交写 `applied` 结果；参数校验失败由
/// `record_model_protocol_correction` 以同一形态写错误结果。
///
/// # Errors
///
/// assistant 消息不是当前请求的一次工具调用，或缺少非空调用 ID 时返回错误。
pub(crate) fn phase_submission_messages(
    request_id: Uuid,
    assistant_message: NewAgentMessage,
) -> Result<Vec<NewAgentMessage>, StoreError> {
    let provider_call_id = phase_submission_call_id(request_id, &assistant_message)?;
    let content = serde_json::to_string(&serde_json::json!({
        "status": "applied",
        "code": "PHASE_SUBMISSION_ACCEPTED",
        "message": "阶段提交已接受，本阶段的结果已保存。",
    }))?;
    Ok(vec![
        assistant_message,
        NewAgentMessage {
            role: AgentMessageRole::Tool,
            content: serde_json::json!({
                "kind": "tool_result",
                "content": content,
            }),
            request_id: Some(request_id),
            tool_call_id: Some(provider_call_id),
        },
    ])
}

/// 测试用的阶段提交工具调用消息。
#[cfg(test)]
pub(crate) fn phase_submission_test_message(
    request_id: Uuid,
    tool_name: &str,
    arguments_json: &str,
) -> NewAgentMessage {
    NewAgentMessage {
        role: AgentMessageRole::Assistant,
        content: serde_json::json!({
            "kind": "assistant_tool_calls",
            "content": null,
            "toolCalls": [{
                "id": "call-submit-1",
                "name": tool_name,
                "arguments": serde_json::from_str::<Value>(arguments_json)
                    .expect("测试阶段提交参数必须是 JSON"),
            }],
        }),
        request_id: Some(request_id),
        tool_call_id: None,
    }
}

/// 校验一条 assistant 消息确实是当前请求的一次提交工具调用，并返回调用 ID。
///
/// 提交被拒绝（排队意见尚未投递、暂停恢复后发现提交不可应用）时写入的是错误工具
/// 结果，因此拒绝路径也只需要这条校验，而不是成功路径的 `applied` 结果。
pub(crate) fn phase_submission_call_id(
    request_id: Uuid,
    assistant_message: &NewAgentMessage,
) -> Result<String, StoreError> {
    if assistant_message.role != AgentMessageRole::Assistant
        || assistant_message.tool_call_id.is_some()
    {
        return Err(StoreError::InvalidAgentMessage(
            "阶段提交必须来自一条 assistant 工具调用消息".to_owned(),
        ));
    }
    if assistant_message.request_id != Some(request_id) {
        return Err(StoreError::InvalidAgentMessage(
            "阶段提交 assistant 消息必须关联当前 model_request".to_owned(),
        ));
    }
    if assistant_message
        .content
        .get("kind")
        .and_then(Value::as_str)
        != Some("assistant_tool_calls")
    {
        return Err(StoreError::InvalidAgentMessage(
            "阶段提交的 assistant 消息必须是工具调用".to_owned(),
        ));
    }
    assistant_message
        .content
        .get("toolCalls")
        .and_then(Value::as_array)
        .filter(|calls| calls.len() == 1)
        .and_then(|calls| calls[0].get("id"))
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            StoreError::InvalidAgentMessage(
                "阶段提交必须恰好包含一个带非空 id 的工具调用".to_owned(),
            )
        })
}

pub(crate) async fn append_agent_messages_for_run(
    transaction: &sea_orm::DatabaseTransaction,
    run_model: &run::Model,
    messages: Vec<NewAgentMessage>,
    now: i64,
) -> Result<Vec<PersistedAgentMessage>, StoreError> {
    let latest_sequence = agent_message::Entity::find()
        .filter(agent_message::Column::RunId.eq(run_model.id))
        .order_by_desc(agent_message::Column::Sequence)
        .one(transaction)
        .await?
        .map_or(0, |message| message.sequence);
    let mut sequence = latest_sequence;
    let mut persisted = Vec::with_capacity(messages.len());
    for message in messages {
        sequence = sequence.checked_add(1).ok_or_else(|| {
            StoreError::InvalidAgentMessage("消息 sequence 已达到 i64 上限".to_owned())
        })?;
        let model = agent_message::ActiveModel {
            id: Set(db_id(Uuid::new_v4())),
            run_id: Set(run_model.id),
            sequence: Set(sequence),
            role: Set(agent_message_role_name(message.role).to_owned()),
            content_json: Set(serde_json::to_string(&message.content)?),
            request_id: Set(message.request_id.map(|id| id.to_string())),
            tool_call_id: Set(message.tool_call_id),
            created_at: Set(now),
        }
        .insert(transaction)
        .await?;
        persisted.push(persisted_agent_message_from_model(&model)?);
    }
    Ok(persisted)
}

fn ensure_run_accepts_messages(run_model: &run::Model) -> Result<(), StoreError> {
    if matches!(
        run_model.status.as_str(),
        "queued" | "running" | "waiting" | "paused"
    ) {
        return Ok(());
    }
    Err(StoreError::InvalidAgentMessageState(format!(
        "Run {} 当前为 {}，不能追加 Agent 消息",
        run_model.id, run_model.status
    )))
}

fn agent_message_role_name(role: AgentMessageRole) -> &'static str {
    match role {
        AgentMessageRole::System => ROLE_SYSTEM,
        AgentMessageRole::User => ROLE_USER,
        AgentMessageRole::Assistant => ROLE_ASSISTANT,
        AgentMessageRole::Tool => ROLE_TOOL,
    }
}

fn agent_message_role_from_db(role: &str) -> Result<AgentMessageRole, StoreError> {
    match role {
        ROLE_SYSTEM => Ok(AgentMessageRole::System),
        ROLE_USER => Ok(AgentMessageRole::User),
        ROLE_ASSISTANT => Ok(AgentMessageRole::Assistant),
        ROLE_TOOL => Ok(AgentMessageRole::Tool),
        other => Err(StoreError::CorruptRecord(format!(
            "agent_message.role 不是已知角色：{other}"
        ))),
    }
}

fn persisted_agent_message_from_model(
    model: &agent_message::Model,
) -> Result<PersistedAgentMessage, StoreError> {
    if model.sequence <= 0 {
        return Err(StoreError::CorruptRecord(
            "agent_message.sequence 必须为正整数".to_owned(),
        ));
    }
    let content = serde_json::from_str::<Value>(&model.content_json)?;
    if !content.is_object() {
        return Err(StoreError::CorruptRecord(
            "agent_message.content_json 必须是 JSON object".to_owned(),
        ));
    }
    let role = agent_message_role_from_db(&model.role)?;
    let tool_call_id = model.tool_call_id.clone();
    if tool_call_id
        .as_deref()
        .is_some_and(|id| id.trim().is_empty())
    {
        return Err(StoreError::CorruptRecord(
            "agent_message.tool_call_id 不能是空字符串".to_owned(),
        ));
    }
    if role == AgentMessageRole::Tool && tool_call_id.is_none() {
        return Err(StoreError::CorruptRecord(
            "tool agent_message 缺少 tool_call_id".to_owned(),
        ));
    }
    let request_id = model
        .request_id
        .as_deref()
        .map(|value| {
            Uuid::parse_str(value).map_err(|error| {
                StoreError::CorruptRecord(format!("agent_message.request_id 不是 UUID：{error}"))
            })
        })
        .transpose()?;
    Ok(PersistedAgentMessage {
        id: model.id.into(),
        run_id: model.run_id.into(),
        sequence: model.sequence,
        role,
        content,
        request_id,
        tool_call_id,
        created_at_ms: model.created_at,
    })
}
