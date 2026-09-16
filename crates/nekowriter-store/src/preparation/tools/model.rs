//! 工具体的传输中立类型、参数解析与结果构造。
//!
//! 从 `preparation/mod.rs` 的这层原样搬出（`docs/08` 的 P3.4），只移动：函数体逐字不变，
//! 只改了可见性前缀（被块外引用的项改 `pub(crate)`）。`preparation/mod.rs` 用
//! `pub use tools::model::{…}`（四个公开 DTO）与 `pub(crate) use tools::model::{…}` 把名字接回，
//! 所以兄弟模块与 app 侧的既有引用一行都不用改。

use nekowriter_core::{
    AppendChapterContent, ChapterBodyEditError, HeldChapterBodyTool, PendingChapterBodyProposal,
    PendingUserQuestion, ReplaceChapterContent, UserQuestionSchema, WorkflowStage,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::StoreError;
use crate::entity::{question, tool_call};

pub(crate) enum ChapterBodyMutation {
    Append(AppendChapterContent),
    Replace(ReplaceChapterContent),
}

/// 一次已经通过 Agent 协议白名单校验、即将由正文 `ToolHost` 执行的调用。
///
/// Store 不依赖 `nekowriter-agent` crate，因此采用这个传输中立的形状；调用 ID、
/// 名称和 JSON 参数仍会原样作为审计记录持久化。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChapterBodyAgentToolCall {
    pub provider_call_id: String,
    pub name: String,
    pub arguments: Value,
}

/// 正文工具调用经同一事务处理后的模型可见结果。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppliedChapterBodyToolCall {
    pub provider_call_id: String,
    pub result: Value,
    /// `true` 仅表示本次回调刚刚推进到下一轮；重复回调读取既有工具结果时为
    /// `false`，调度器不应再额外投递一次同 Run 请求。
    pub newly_resolved: bool,
    /// 本次结果是否把 Run 推进到可以发送下一次模型请求的队列状态。
    ///
    /// 第三次连续的工具协议错误会停止 Run；它仍是已解析的新调用，但不能继续
    /// 投递请求。
    pub requeue: bool,
}

/// `ask_user` 工具调用的传输中立输入。
///
/// Store 自行解析参数、校验问题模式并持久化问题，因此上层不把尚未验证的 JSON
/// 直接变成等待用户的工作流状态。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentQuestionToolCall {
    pub provider_call_id: String,
    pub arguments: Value,
}

/// `ask_user` 的两种稳定终态：等待用户，或参数无效后已经形成可回送模型的错误结果。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppliedAgentQuestionToolCall {
    AwaitingAnswer {
        provider_call_id: String,
        question: PendingUserQuestion,
        newly_created: bool,
    },
    Resolved {
        provider_call_id: String,
        result: Value,
        newly_resolved: bool,
        /// 无效问题参数达到连续协议错误上限时为 `false`。
        requeue: bool,
    },
}
pub(crate) enum AgentChapterBodyMutation {
    Append(AppendChapterContent),
    Replace(ReplaceChapterContent),
}

pub(crate) struct AgentBodyToolError {
    code: &'static str,
    message: String,
}

impl AgentBodyToolError {
    pub(crate) fn into_result(self) -> Value {
        agent_tool_error_result(self.code, &self.message)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAppendChapterBodyToolArguments {
    content: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawReplaceChapterBodyToolArguments {
    old_str: String,
    new_str: String,
}

pub(crate) fn validate_agent_body_tool_call(
    call: &ChapterBodyAgentToolCall,
) -> Result<(), StoreError> {
    if call.provider_call_id.trim().is_empty() {
        return Err(StoreError::InvalidModelRequest(
            "正文工具调用 provider_call_id 不能为空".to_owned(),
        ));
    }
    if call.name.trim().is_empty() {
        return Err(StoreError::InvalidModelRequest(
            "正文工具调用名称不能为空".to_owned(),
        ));
    }
    if !call.arguments.is_object() {
        return Err(StoreError::InvalidModelRequest(
            "正文工具调用参数必须是 JSON object".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn parse_agent_body_mutation(
    call: &ChapterBodyAgentToolCall,
) -> Result<AgentChapterBodyMutation, AgentBodyToolError> {
    match call.name.as_str() {
        "append_content" => {
            let raw: RawAppendChapterBodyToolArguments =
                serde_json::from_value(call.arguments.clone()).map_err(|error| {
                    AgentBodyToolError {
                        code: "TOOL_ARGUMENTS_INVALID",
                        message: format!("append_content 参数无效：{error}"),
                    }
                })?;
            AppendChapterContent::new(raw.content)
                .map(AgentChapterBodyMutation::Append)
                .map_err(|error| AgentBodyToolError {
                    code: "VALIDATION_ERROR",
                    message: error.to_string(),
                })
        }
        "replace_content" => {
            let raw: RawReplaceChapterBodyToolArguments =
                serde_json::from_value(call.arguments.clone()).map_err(|error| {
                    AgentBodyToolError {
                        code: "TOOL_ARGUMENTS_INVALID",
                        message: format!("replace_content 参数无效：{error}"),
                    }
                })?;
            ReplaceChapterContent::new(raw.old_str, raw.new_str)
                .map(AgentChapterBodyMutation::Replace)
                .map_err(|error| AgentBodyToolError {
                    code: "VALIDATION_ERROR",
                    message: error.to_string(),
                })
        }
        _ => Err(AgentBodyToolError {
            code: "TOOL_NOT_AVAILABLE",
            message: format!("正文阶段不支持工具 {}", call.name),
        }),
    }
}

pub(crate) fn agent_body_edit_error_result(error: &ChapterBodyEditError) -> Value {
    let message = error.to_string();
    let code = match error {
        ChapterBodyEditError::EmptyOldText => "VALIDATION_ERROR",
        ChapterBodyEditError::NoMatch => "EDIT_NO_MATCH",
        ChapterBodyEditError::MultipleMatches => "EDIT_AMBIGUOUS_MATCH",
        ChapterBodyEditError::NoChange => "EDIT_NO_CHANGE",
    };
    agent_tool_error_result(code, &message)
}

pub(crate) fn agent_tool_error_result(code: &'static str, message: &str) -> Value {
    json!({
        "status": "error",
        "code": code,
        "message": message,
    })
}

pub(crate) fn question_tool_result(
    provider_call_id: &str,
    answers: &[nekowriter_core::UserQuestionAnswer],
) -> Value {
    json!({
        "tool_call_id": provider_call_id,
        "status": "answered",
        "answers": answers,
    })
}

pub(crate) fn question_tool_error_result(
    provider_call_id: &str,
    code: &'static str,
    message: &str,
) -> Value {
    let mut result = agent_tool_error_result(code, message);
    result
        .as_object_mut()
        .expect("agent_tool_error_result 必须产生 JSON object")
        .insert(
            "tool_call_id".to_owned(),
            Value::String(provider_call_id.to_owned()),
        );
    result
}

pub(crate) fn validate_agent_question_tool_call(
    call: &AgentQuestionToolCall,
) -> Result<(), StoreError> {
    if call.provider_call_id.trim().is_empty() {
        return Err(StoreError::InvalidModelRequest(
            "ask_user provider_call_id 不能为空".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn parse_agent_question_schema(
    arguments: &Value,
) -> Result<UserQuestionSchema, StoreError> {
    let schema: UserQuestionSchema =
        serde_json::from_value(arguments.clone()).map_err(|error| {
            StoreError::InvalidQuestion(format!("ask_user 参数不符合问题模式：{error}"))
        })?;
    schema
        .validate()
        .map_err(|error| StoreError::InvalidQuestion(error.to_string()))?;
    Ok(schema)
}

pub(crate) fn pending_user_question_from_model(
    model: &question::Model,
) -> Result<PendingUserQuestion, StoreError> {
    if model.status != "open" || model.answer_json.is_some() {
        return Err(StoreError::CorruptRecord(
            "待答问题的状态与答案字段不一致".to_owned(),
        ));
    }
    let schema: UserQuestionSchema = serde_json::from_str(&model.schema_json)?;
    schema
        .validate()
        .map_err(|error| StoreError::CorruptRecord(format!("持久问题模式无效：{error}")))?;
    Ok(PendingUserQuestion {
        id: model.id.into(),
        tool_call_id: model.tool_call_id.into(),
        schema,
        created_at_ms: model.created_at,
    })
}

pub(crate) fn question_stage_matches_run_kind(stage: &WorkflowStage, run_kind: &str) -> bool {
    matches!(
        (stage, run_kind),
        (
            WorkflowStage::PreparingGenerating | WorkflowStage::PreparingRevising,
            "preparation"
        ) | (WorkflowStage::WritingOutlineGenerating, "chapter_outline")
            | (WorkflowStage::WritingDrafting, "chapter_body")
    )
}

pub(crate) fn awaiting_body_proposal_result(proposal: &PendingChapterBodyProposal) -> Value {
    json!({
        "status": "awaiting_review",
        "proposalId": proposal.tool_call_id.to_string(),
        "baseRevisionId": proposal.base_revision_id.map(|revision_id| revision_id.to_string()),
    })
}

pub(crate) fn held_body_tool_result(held: &HeldChapterBodyTool) -> Value {
    json!({
        "status": "held",
        "tool_call_id": held.provider_call_id,
        "base_revision_id": held.base_revision_id.map(|revision_id| revision_id.to_string()),
    })
}

/// 恢复暂停所需的不可变工具边界快照。
///
/// `result_if_resumed` 仅表示工具参数或候选正文在边界上已经可判定的稳定业务错误；
/// 有候选正文时必须为空，避免恢复流程重新解析或猜测模型当时传入的内容。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct HeldChapterBodyToolPayload {
    pub(crate) held: HeldChapterBodyTool,
    pub(crate) result_if_resumed: Option<Value>,
}
pub(crate) fn held_chapter_body_tool_payload_from_model(
    model: &tool_call::Model,
) -> Result<HeldChapterBodyToolPayload, StoreError> {
    let held_json = model
        .held_json
        .as_deref()
        .ok_or_else(|| StoreError::CorruptRecord("已暂停正文工具调用缺少 held_json".to_owned()))?;
    let payload: HeldChapterBodyToolPayload = serde_json::from_str(held_json)?;
    let tool_call_id: Uuid = model.id.into();
    let held = &payload.held;
    let expected_candidate_hash = held
        .candidate_content
        .as_deref()
        .map(candidate_content_hash);
    if held.tool_call_id != tool_call_id
        || held.provider_call_id != model.provider_call_id
        || held.tool_name != model.name
        || held
            .base_revision_id
            .map(|revision_id| revision_id.to_string())
            .as_deref()
            != model.base_revision_id.as_deref()
        || held.candidate_hash.as_deref() != expected_candidate_hash.as_deref()
        || (held.candidate_content.is_some() && payload.result_if_resumed.is_some())
        || (held.candidate_content.is_none() && payload.result_if_resumed.is_none())
    {
        return Err(StoreError::CorruptRecord(
            "已暂停正文工具调用与工具审计字段不一致".to_owned(),
        ));
    }
    if payload
        .result_if_resumed
        .as_ref()
        .is_some_and(|result| !result.is_object())
    {
        return Err(StoreError::CorruptRecord(
            "已暂停正文工具调用的 result_if_resumed 必须是 JSON object".to_owned(),
        ));
    }
    Ok(payload)
}

pub(crate) fn pending_chapter_body_proposal_from_tool_call(
    model: &tool_call::Model,
) -> Result<PendingChapterBodyProposal, StoreError> {
    let proposal_json = model.proposal_json.as_deref().ok_or_else(|| {
        StoreError::CorruptRecord("待审正文工具调用缺少 proposal_json".to_owned())
    })?;
    let proposal: PendingChapterBodyProposal = serde_json::from_str(proposal_json)?;
    let tool_call_id: Uuid = model.id.into();
    if proposal.tool_call_id != tool_call_id
        || proposal.provider_call_id != model.provider_call_id
        || proposal.tool_name != model.name
        || proposal
            .base_revision_id
            .map(|revision_id| revision_id.to_string())
            .as_deref()
            != model.base_revision_id.as_deref()
        || proposal.candidate_hash != candidate_content_hash(&proposal.candidate_content)
    {
        return Err(StoreError::CorruptRecord(
            "待审正文提案与工具调用审计字段不一致".to_owned(),
        ));
    }
    Ok(proposal)
}

pub(crate) fn candidate_content_hash(content: &str) -> String {
    format!("{:x}", Sha256::digest(content.as_bytes()))
}
