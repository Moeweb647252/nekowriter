//! 准备阶段及其后首章细纲的持久化命令。
//!
//! 这里的每个写操作都在 SQLite 短事务中完成；模型调用不属于这一层。即使当前
//! 通过人工提交产物，也沿用与后续 Agent 提交相同的文档修订、审核和折叠数据模型。

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use nekowriter_core::{
    AppendChapterContent, ApprovalMode, ChapterBodyEditError, ChapterBodySnapshot,
    ChapterEditReview, ChapterFinalReview, ChapterFinalReviewCompletion, ChapterId,
    ChapterOutlineReview, ChapterOutlineReviewCompletion, ChapterOutlineSnapshot, ChapterPlan,
    ChapterPlanDraft, ContextFold, CopiedContextFoldStage, CurrentChapterBody,
    CurrentChapterGuidance, CurrentChapterOutline, CurrentNovelBrief, CurrentPreparationArtifact,
    DeliveredChapterBodyFeedback, DocumentId, DocumentRevisionView, ExecutionStatus,
    FeedbackCitation, FeedbackDeliveryStatus, FeedbackStatus, HeldChapterBodySubmission,
    HeldChapterBodyTool, HumanReviewDecision, MAX_CONSECUTIVE_PROTOCOL_ERRORS,
    MAX_REVIEW_ISSUES_LEN, NewChapterGuidance, NewChapterOutline, NewExtraChapter, NewNovelBrief,
    PendingChapterBodyFeedback, PendingChapterBodyProposal, PendingUserQuestion,
    PreparationArtifactInput, PreparationArtifactKind, PreparationReview,
    PreparationReviewCompletion, PreparationSnapshot, QuestionId, ReplaceChapterContent, ReviewId,
    ReviewStatus, RevisionId, RunId, UserQuestionAnswers, UserQuestionSchema, WorkflowSnapshot,
    WorkflowStage, begin_chapter_outline, begin_preparation,
    cancel_chapter_body_pause_for_feedback, confirm_novel_completion, continue_after_agent_tool,
    continue_after_user_answer, continue_chapter_body_after_proposal,
    continue_chapter_body_after_protocol_correction, continue_chapter_body_after_tool,
    continue_writing_run, enter_extra_chapter, enter_next_chapter, finish_chapter_final_review,
    finish_chapter_outline_review, finish_extra_chapter_final_review, finish_preparation_review,
    hold_chapter_body_proposal, hold_chapter_body_submission_for_pause,
    hold_chapter_body_tool_for_pause, interrupt_writing_run, non_whitespace_char_count,
    record_chapter_body_edit, record_chapter_body_feedback, record_chapter_guidance,
    record_preparation_review_decision, render_preparation_artifact_plaintext,
    replace_unique_chapter_content, schedule_chapter_body_run, stop_writing_after_protocol_error,
    stop_writing_run as stop_writing_workflow_run, submit_chapter_for_review,
    submit_chapter_outline_for_review, submit_preparation_for_review, wait_for_user_answer,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, Set, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::agent_messages::{
    append_agent_messages_for_run, phase_submission_call_id, phase_submission_messages,
};
use super::budget::ensure_quota_scope;
use super::entity::{
    agent_message, chapter, document, document_revision, event, feedback, model_request,
    novel_version, question, review, revision_dependency, run, tool_call, workflow_instance,
};
use super::model_requests::{
    model_request_status_from_db, next_protocol_error_count, protocol_error_tool_result_messages,
};
use super::{
    AgentMessageRole, CHAPTER_BODY_ROLE, CHAPTER_GUIDANCE_ROLE, CHAPTER_OUTLINE_ROLE, Command,
    ModelRequestStatus, NOVEL_BRIEF_ROLE, NewAgentMessage, Notify, ProjectId, Store, StoreError,
    VersionId, approval_mode_name, db_id, insert_receipt, parse_id, parse_optional_id,
    payload_hash, replay_or_conflict, transaction_error, utc_now_millis, workflow_from_model,
};

const SAVE_NOVEL_BRIEF_METHOD: &str = "PUT";
const START_PREPARATION_METHOD: &str = "POST";
const SUBMIT_PREPARATION_ARTIFACTS_METHOD: &str = "POST";
const DECIDE_PREPARATION_REVIEW_METHOD: &str = "POST";
const SAVE_CHAPTER_GUIDANCE_METHOD: &str = "PUT";
const START_CHAPTER_OUTLINE_METHOD: &str = "POST";
const SUBMIT_CHAPTER_OUTLINE_METHOD: &str = "POST";
const DECIDE_CHAPTER_OUTLINE_REVIEW_METHOD: &str = "POST";
const APPEND_CHAPTER_BODY_METHOD: &str = "POST";
const REPLACE_CHAPTER_BODY_METHOD: &str = "POST";
const SET_APPROVAL_MODE_METHOD: &str = "POST";
const DECIDE_CHAPTER_BODY_PROPOSAL_METHOD: &str = "POST";
const SUBMIT_RUN_FEEDBACK_METHOD: &str = "POST";
const REQUEST_CHAPTER_BODY_PAUSE_METHOD: &str = "POST";
const RESUME_CHAPTER_BODY_METHOD: &str = "POST";
const ANSWER_QUESTION_METHOD: &str = "POST";
const CONTINUE_WRITING_RUN_METHOD: &str = "POST";
const STOP_WRITING_RUN_METHOD: &str = "POST";
const REQUEST_ALLOWANCE_STEP: i64 = 100;
const CHAPTER_BODY_FEEDBACK_PROTOCOL_CORRECTION: &str = "本章仍有未投递的自由意见，不能直接提交终稿审核。请继续正文阶段，并使用 append_content 或 replace_content 形成下一条完整工具结果，让意见随该结果投递。";
const CHAPTER_BODY_SUBMISSION_PROTOCOL_CORRECTION: &str = "当前直接提交无法应用。请继续正文阶段，并使用 append_content 或 replace_content 先形成一条有效正文修订；只有正文依赖当前已通过的章节细纲后才能提交终稿审核。";
const SUBMIT_CHAPTER_FINAL_REVIEW_METHOD: &str = "POST";
const DECIDE_CHAPTER_FINAL_REVIEW_METHOD: &str = "POST";
const ADVANCE_TO_NEXT_CHAPTER_METHOD: &str = "POST";
const CONFIRM_NOVEL_COMPLETION_METHOD: &str = "POST";
const ADD_EXTRA_CHAPTER_METHOD: &str = "POST";
const DELEGATE_REVIEW_METHOD: &str = "POST";
const CANCEL_REVIEW_DELEGATION_METHOD: &str = "POST";

#[derive(Serialize)]
struct SaveNovelBriefPayload<'a> {
    content: &'a str,
}

#[derive(Serialize)]
struct StartPreparationPayload {
    expected_state_revision: i64,
}

#[derive(Serialize)]
struct SubmitPreparationArtifactsPayload<'a> {
    expected_state_revision: i64,
    artifacts: &'a [PreparationArtifactInput],
    chapter_plan: Option<&'a [ChapterPlanDraft]>,
}

#[derive(Serialize)]
struct DecidePreparationReviewPayload<'a> {
    expected_state_revision: i64,
    decision: &'a HumanReviewDecision,
    citations: &'a [FeedbackCitation],
}

#[derive(Serialize)]
struct SaveChapterGuidancePayload<'a> {
    expected_state_revision: i64,
    content: &'a str,
}

#[derive(Serialize)]
struct StartChapterOutlinePayload {
    expected_state_revision: i64,
}

#[derive(Serialize)]
struct SubmitChapterOutlinePayload<'a> {
    expected_state_revision: i64,
    content: &'a str,
}

#[derive(Serialize)]
struct AppendChapterBodyPayload<'a> {
    expected_state_revision: i64,
    content: &'a str,
}

#[derive(Serialize)]
struct ReplaceChapterBodyPayload<'a> {
    expected_state_revision: i64,
    old_str: &'a str,
    new_str: &'a str,
}

#[derive(Serialize)]
struct SetApprovalModePayload {
    expected_state_revision: i64,
    mode: ApprovalMode,
}

#[derive(Serialize)]
struct DecideChapterBodyProposalPayload<'a> {
    expected_state_revision: i64,
    feedback: &'a str,
    citations: &'a [FeedbackCitation],
}

#[derive(Serialize)]
struct SubmitRunFeedbackPayload<'a> {
    expected_state_revision: i64,
    text: &'a str,
    citations: &'a [FeedbackCitation],
}

#[derive(Serialize)]
struct AnswerQuestionPayload<'a> {
    expected_state_revision: i64,
    answers: &'a UserQuestionAnswers,
}

#[derive(Serialize)]
struct RunWorkflowControlPayload {
    expected_state_revision: i64,
}

#[derive(Serialize)]
struct SubmitChapterFinalReviewPayload {
    expected_state_revision: i64,
}

#[derive(Serialize)]
struct AdvanceToNextChapterPayload {
    expected_state_revision: i64,
}

#[derive(Serialize)]
struct ConfirmNovelCompletionPayload {
    expected_state_revision: i64,
}

#[derive(Serialize)]
struct AddExtraChapterPayload<'a> {
    expected_state_revision: i64,
    title: &'a str,
}

#[derive(Serialize)]
struct DelegateReviewPayload<'a> {
    expected_state_revision: i64,
    notes: &'a str,
    citations: &'a [FeedbackCitation],
}

#[derive(Serialize)]
struct CancelReviewDelegationPayload {
    expected_state_revision: i64,
}

enum ChapterBodyMutation {
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

/// 回答问题后的最小命令结果。
///
/// 前端依据其中的工作流刷新所属阶段快照；HTTP 层同时把 queued Run 交还给调度器。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionAnswerResult {
    pub question_id: QuestionId,
    pub run_id: RunId,
    pub workflow: WorkflowSnapshot,
}

/// 显式停止一条顶层写作 Run 的结果。
///
/// `cancelled_request_ids` 是已经在同一 SQLite 事务中作废的 prepared/dispatched
/// attempt；应用层据此中止仍在内存中执行的 HTTP 任务。回执保留这些 ID，使响应
/// 丢失后的重试仍可再次发出相同的本地取消信号。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StopWritingRunResult {
    pub workflow: WorkflowSnapshot,
    pub cancelled_request_ids: Vec<Uuid>,
}

/// 为什么要收束自动执行中的 Run。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterruptionSource {
    /// 上次进程以非正常方式结束，本次启动时收束。
    StartupRecovery,
    /// 本次退出服务，收尾在途请求。
    ProcessShutdown,
}

impl InterruptionSource {
    /// 持久事件里的 `reason`，也写在请求错误的 `kind` 上。
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::StartupRecovery => "startup_recovery",
            Self::ProcessShutdown => "process_shutdown",
        }
    }

    /// 用户可见原因文案的前缀。
    const fn prefix(self) -> &'static str {
        match self {
            Self::StartupRecovery => "服务重启",
            Self::ProcessShutdown => "退出服务",
        }
    }
}

/// 一次收束的结果：被中断的写作 Run 与审核委托数量。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InterruptionOutcome {
    pub writing_runs: usize,
    pub review_delegations: usize,
}

/// 一条已经绑定到正文工具结果的自由意见，供调度器把基础工具结果物化为实际模型
/// 消息。`included` 记录也会返回，因为后续请求重放历史 tool 消息时必须保持首次
/// 发出时的同一反馈内容。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedChapterBodyFeedback {
    pub provider_tool_call_id: String,
    pub id: Uuid,
    pub sequence: i64,
    pub text: String,
    pub citations: Vec<FeedbackCitation>,
    pub status: FeedbackStatus,
}

enum AgentChapterBodyMutation {
    Append(AppendChapterContent),
    Replace(ReplaceChapterContent),
}

struct AgentBodyToolError {
    code: &'static str,
    message: String,
}

impl AgentBodyToolError {
    fn into_result(self) -> Value {
        agent_tool_error_result(self.code, self.message)
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

fn validate_agent_body_tool_call(call: &ChapterBodyAgentToolCall) -> Result<(), StoreError> {
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

fn parse_agent_body_mutation(
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

fn agent_body_edit_error_result(error: ChapterBodyEditError) -> Value {
    let message = error.to_string();
    let code = match error {
        ChapterBodyEditError::EmptyOldText => "VALIDATION_ERROR",
        ChapterBodyEditError::NoMatch => "EDIT_NO_MATCH",
        ChapterBodyEditError::MultipleMatches => "EDIT_AMBIGUOUS_MATCH",
        ChapterBodyEditError::NoChange => "EDIT_NO_CHANGE",
    };
    agent_tool_error_result(code, message)
}

fn agent_tool_error_result(code: &'static str, message: String) -> Value {
    json!({
        "status": "error",
        "code": code,
        "message": message,
    })
}

fn question_tool_result(
    provider_call_id: &str,
    answers: &[nekowriter_core::UserQuestionAnswer],
) -> Value {
    json!({
        "tool_call_id": provider_call_id,
        "status": "answered",
        "answers": answers,
    })
}

fn question_tool_error_result(
    provider_call_id: &str,
    code: &'static str,
    message: String,
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

fn validate_agent_question_tool_call(call: &AgentQuestionToolCall) -> Result<(), StoreError> {
    if call.provider_call_id.trim().is_empty() {
        return Err(StoreError::InvalidModelRequest(
            "ask_user provider_call_id 不能为空".to_owned(),
        ));
    }
    Ok(())
}

fn parse_agent_question_schema(arguments: &Value) -> Result<UserQuestionSchema, StoreError> {
    let schema: UserQuestionSchema =
        serde_json::from_value(arguments.clone()).map_err(|error| {
            StoreError::InvalidQuestion(format!("ask_user 参数不符合问题模式：{error}"))
        })?;
    schema
        .validate()
        .map_err(|error| StoreError::InvalidQuestion(error.to_string()))?;
    Ok(schema)
}

fn pending_user_question_from_model(
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

fn question_stage_matches_run_kind(stage: &WorkflowStage, run_kind: &str) -> bool {
    matches!(
        (stage, run_kind),
        (
            WorkflowStage::PreparingGenerating | WorkflowStage::PreparingRevising,
            "preparation"
        ) | (WorkflowStage::WritingOutlineGenerating, "chapter_outline")
            | (WorkflowStage::WritingDrafting, "chapter_body")
    )
}

fn awaiting_body_proposal_result(proposal: &PendingChapterBodyProposal) -> Value {
    json!({
        "status": "awaiting_review",
        "proposalId": proposal.tool_call_id.to_string(),
        "baseRevisionId": proposal.base_revision_id.map(|revision_id| revision_id.to_string()),
    })
}

fn held_body_tool_result(held: &HeldChapterBodyTool) -> Value {
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
struct HeldChapterBodyToolPayload {
    held: HeldChapterBodyTool,
    result_if_resumed: Option<Value>,
}

/// 模型直接提交章节时命中暂停边界的 Run 检查点。
///
/// 提交工具调用直到恢复后才进入 Agent 消息账本；这样暂停不会把“已提交章节”
/// 提前作为已完成事实注入下一轮上下文。原始工具参数属于该 assistant 消息，
/// 恢复时与它的配对工具结果一起写回。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct HeldChapterBodySubmissionCheckpoint {
    held: HeldChapterBodySubmission,
    assistant_message: NewAgentMessage,
}

fn held_chapter_body_tool_payload_from_model(
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

fn pending_chapter_body_proposal_from_tool_call(
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

fn candidate_content_hash(content: &str) -> String {
    format!("{:x}", Sha256::digest(content.as_bytes()))
}

/// 正文自由意见只能引用当前稿本中的正式文档，或当前 Run 尚未落库的候选正文。
///
/// 前端提交的字符位置始终被当作 Unicode scalar value 序号处理；这里先把它们映射
/// 回服务端快照中的 UTF-8 边界，再逐字校验 quote。这样 emoji、组合字符和多字节
/// 中文都不会因为把 UTF-16/UTF-8 偏移混用而引用到错误位置。
#[derive(Clone, Copy)]
enum CandidateCitationScope {
    HeldCurrentRun,
    Proposal { tool_call_id: Uuid },
}

async fn validate_chapter_body_feedback_citations(
    transaction: &DatabaseTransaction,
    citations: &[FeedbackCitation],
    version_id: VersionId,
    active_chapter_id: ChapterId,
    run_id: RunId,
    candidate_scope: CandidateCitationScope,
) -> Result<(), StoreError> {
    for citation in citations {
        match citation {
            FeedbackCitation::Document {
                document_id,
                revision_id,
                range,
                chapter_id,
            } => {
                validate_document_citation(
                    transaction,
                    *document_id,
                    *revision_id,
                    range,
                    *chapter_id,
                    version_id,
                    Some(active_chapter_id),
                )
                .await?;
            }
            FeedbackCitation::Candidate {
                tool_call_id,
                candidate_hash,
                candidate_range,
            } => {
                let (candidate_content, actual_hash) = match candidate_scope {
                    CandidateCitationScope::HeldCurrentRun => {
                        held_candidate_for_feedback_citation(transaction, run_id, *tool_call_id)
                            .await?
                    }
                    CandidateCitationScope::Proposal {
                        tool_call_id: proposal_tool_call_id,
                    } => {
                        if *tool_call_id != proposal_tool_call_id {
                            return Err(StoreError::InvalidCitation(
                                "待审编辑的引用必须指向当前待审工具调用".to_owned(),
                            ));
                        }
                        proposal_candidate_for_feedback_citation(
                            transaction,
                            run_id,
                            proposal_tool_call_id,
                        )
                        .await?
                    }
                };
                if candidate_hash != &actual_hash {
                    return Err(StoreError::InvalidCitation(
                        "候选正文已变化，请重新选择要引用的文本".to_owned(),
                    ));
                }
                validate_text_citation_range(&candidate_content, candidate_range)?;
            }
        }
    }
    Ok(())
}

/// 校验一条正式文档引用是否逐字锚定在本稿本中的不可变修订上。
///
/// 正文意见、委托审核与人工审核决定共用同一套判定：修订存在、`document_id` 与
/// `revision_id` 匹配、文档未被删除、属于当前稿本、`chapter_id` 与文档归属一致，
/// 最后把引用范围映射回 UTF-8 边界逐字复核 quote。`required_chapter_id` 为 `Some`
/// 时表示本次操作只允许引用该章节的文档；为 `None` 时允许本稿本任意文档（准备
/// 阶段审核）。
async fn validate_document_citation(
    transaction: &DatabaseTransaction,
    document_id: nekowriter_core::DocumentId,
    revision_id: RevisionId,
    range: &nekowriter_core::TextCitationRange,
    cited_chapter_id: Option<ChapterId>,
    version_id: VersionId,
    required_chapter_id: Option<ChapterId>,
) -> Result<(), StoreError> {
    let revision_model = document_revision::Entity::find_by_id(db_id(revision_id))
        .one(transaction)
        .await?
        .ok_or_else(|| StoreError::InvalidCitation("被引用的修订不存在".to_owned()))?;
    if revision_model.document_id != db_id(document_id) {
        return Err(StoreError::InvalidCitation(
            "引用中的 document_id 与 revision_id 不匹配".to_owned(),
        ));
    }
    let document_model = document::Entity::find_by_id(db_id(document_id))
        .filter(document::Entity::COLUMN.deleted_at.is_null())
        .one(transaction)
        .await?
        .ok_or_else(|| StoreError::InvalidCitation("被引用的文档不存在或已删除".to_owned()))?;
    if document_model.version_id != db_id(version_id) {
        return Err(StoreError::InvalidCitation(
            "引用必须属于当前稿本".to_owned(),
        ));
    }
    let actual_chapter_id =
        parse_optional_id(document_model.chapter_id, "citation.document.chapter_id")?;
    if actual_chapter_id != cited_chapter_id {
        return Err(StoreError::InvalidCitation(
            "引用中的 chapter_id 与文档所属章节不匹配".to_owned(),
        ));
    }
    if required_chapter_id.is_some_and(|required| actual_chapter_id != Some(required)) {
        return Err(StoreError::InvalidCitation(
            "引用必须指向本次操作所在章节的文档".to_owned(),
        ));
    }
    validate_text_citation_range(&revision_model.content, range)
}

/// 校验一批人工审核决定携带的引用。
///
/// 决定针对的是已经进入审核的产物或正文修订，因此只能引用已持久化的正式文档；
/// 尚未落库的正文候选不属于审核目标，不能借决定绕过候选哈希校验。
async fn validate_review_decision_citations(
    transaction: &DatabaseTransaction,
    citations: &[FeedbackCitation],
    version_id: VersionId,
    required_chapter_id: Option<ChapterId>,
) -> Result<(), StoreError> {
    for citation in citations {
        match citation {
            FeedbackCitation::Document {
                document_id,
                revision_id,
                range,
                chapter_id,
            } => {
                validate_document_citation(
                    transaction,
                    *document_id,
                    *revision_id,
                    range,
                    *chapter_id,
                    version_id,
                    required_chapter_id,
                )
                .await?;
            }
            FeedbackCitation::Candidate { .. } => {
                return Err(StoreError::InvalidCitation(
                    "人工审核决定只能引用已持久化的修订".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

/// 退回审核可以由文字或已逐字验证的引用构成；这项判断必须留在引用校验之后，
/// 否则一个伪造引用会错误地让空反馈通过。
fn validate_review_decision_feedback(
    decision: &HumanReviewDecision,
    citations: &[FeedbackCitation],
) -> Result<(), StoreError> {
    decision
        .validate_feedback(!citations.is_empty())
        .map_err(|error| StoreError::InvalidReviewDecision(error.to_string()))
}

async fn held_candidate_for_feedback_citation(
    transaction: &DatabaseTransaction,
    run_id: RunId,
    tool_call_id: Uuid,
) -> Result<(String, String), StoreError> {
    let model = tool_call::Entity::find_by_id(db_id(tool_call_id))
        .filter(tool_call::Entity::COLUMN.run_id.eq(db_id(run_id)))
        .filter(tool_call::Entity::COLUMN.status.eq("held"))
        .one(transaction)
        .await?
        .ok_or_else(|| {
            StoreError::InvalidCitation("候选引用必须来自当前 Run 中已暂停的工具调用".to_owned())
        })?;
    let held = held_chapter_body_tool_payload_from_model(&model)?.held;
    let candidate_content = held.candidate_content.ok_or_else(|| {
        StoreError::InvalidCitation("该暂停工具调用没有可引用的正文候选".to_owned())
    })?;
    let candidate_hash = held
        .candidate_hash
        .ok_or_else(|| StoreError::CorruptRecord("暂停正文候选缺少 content hash".to_owned()))?;
    Ok((candidate_content, candidate_hash))
}

async fn proposal_candidate_for_feedback_citation(
    transaction: &DatabaseTransaction,
    run_id: RunId,
    tool_call_id: Uuid,
) -> Result<(String, String), StoreError> {
    let model = tool_call::Entity::find_by_id(db_id(tool_call_id))
        .filter(tool_call::Entity::COLUMN.run_id.eq(db_id(run_id)))
        .filter(tool_call::Entity::COLUMN.status.eq("awaiting_review"))
        .one(transaction)
        .await?
        .ok_or_else(|| StoreError::InvalidCitation("候选引用必须来自当前待审编辑".to_owned()))?;
    let proposal = pending_chapter_body_proposal_from_tool_call(&model)?;
    Ok((proposal.candidate_content, proposal.candidate_hash))
}

fn validate_text_citation_range(
    content: &str,
    range: &nekowriter_core::TextCitationRange,
) -> Result<(), StoreError> {
    if range.start_char < 0 || range.end_char < range.start_char {
        return Err(StoreError::InvalidCitation(
            "引用范围必须是非负的左闭右开区间".to_owned(),
        ));
    }
    let start = usize::try_from(range.start_char)
        .map_err(|_| StoreError::InvalidCitation("引用起点超出当前平台可表示范围".to_owned()))?;
    let end = usize::try_from(range.end_char)
        .map_err(|_| StoreError::InvalidCitation("引用终点超出当前平台可表示范围".to_owned()))?;
    let scalar_count = content.chars().count();
    if start >= end || end > scalar_count {
        return Err(StoreError::InvalidCitation(
            "引用范围不在原文的 Unicode 字符边界内".to_owned(),
        ));
    }
    let start_byte = byte_offset_for_unicode_scalar(content, start).ok_or_else(|| {
        StoreError::InvalidCitation("引用起点不在原文的 Unicode 字符边界内".to_owned())
    })?;
    let end_byte = byte_offset_for_unicode_scalar(content, end).ok_or_else(|| {
        StoreError::InvalidCitation("引用终点不在原文的 Unicode 字符边界内".to_owned())
    })?;
    if content[start_byte..end_byte] != range.quote {
        return Err(StoreError::InvalidCitation(
            "引用文字与原始修订不一致，请重新选择".to_owned(),
        ));
    }
    Ok(())
}

fn byte_offset_for_unicode_scalar(content: &str, scalar_offset: usize) -> Option<usize> {
    if scalar_offset == content.chars().count() {
        Some(content.len())
    } else {
        content
            .char_indices()
            .nth(scalar_offset)
            .map(|(byte_offset, _)| byte_offset)
    }
}

fn feedback_citations_from_json(value: &str) -> Result<Vec<FeedbackCitation>, StoreError> {
    serde_json::from_str(value).map_err(|error| {
        StoreError::CorruptRecord(format!("自由意见 citations_json 无法解析：{error}"))
    })
}

/// 候选正文被接受为正式修订前，把仍未封存的候选引用重定位到该修订。
///
/// 已封存意见不在这里改写：它们已经进入某个不可变请求快照。当前调用点只会处理
/// 等待投递的同一批意见，因此后续模型消息得到稳定的正式修订锚点，同时不会回写
/// 已发送的历史。
async fn resolve_pending_candidate_feedback_citations(
    transaction: &DatabaseTransaction,
    run_id: RunId,
    tool_call_id: Uuid,
    candidate_content: &str,
    document_id: nekowriter_core::DocumentId,
    revision_id: RevisionId,
    chapter_id: ChapterId,
    now: i64,
) -> Result<(), StoreError> {
    let expected_hash = candidate_content_hash(candidate_content);
    let models = feedback::Entity::find()
        .filter(feedback::Entity::COLUMN.run_id.eq(db_id(run_id)))
        .filter(feedback::Entity::COLUMN.status.is_in(["queued", "bound"]))
        .all(transaction)
        .await?;
    for model in models {
        let mut citations = feedback_citations_from_json(&model.citations_json)?;
        let mut changed = false;
        for citation in &mut citations {
            let FeedbackCitation::Candidate {
                tool_call_id: cited_tool_call_id,
                candidate_hash,
                candidate_range,
            } = citation
            else {
                continue;
            };
            if *cited_tool_call_id != tool_call_id {
                continue;
            }
            if *candidate_hash != expected_hash {
                return Err(StoreError::CorruptRecord(
                    "待投递意见的候选引用 hash 与被接受候选不一致".to_owned(),
                ));
            }
            validate_text_citation_range(candidate_content, candidate_range).map_err(|error| {
                StoreError::CorruptRecord(format!("待投递意见的候选引用无效：{error}"))
            })?;
            *citation = FeedbackCitation::Document {
                document_id,
                revision_id,
                range: candidate_range.clone(),
                chapter_id: Some(chapter_id),
            };
            changed = true;
        }
        if changed {
            let mut active = model.into_active_model();
            active.citations_json = Set(serde_json::to_string(&citations)?);
            active.updated_at = Set(now);
            active.update(transaction).await?;
        }
    }
    Ok(())
}

struct FeedbackBatchAnchors {
    after_tool_sequence: i64,
    preferred_tool_call_id: Option<String>,
    bound_tool_call_id: Option<String>,
    status: FeedbackStatus,
}

fn feedback_status_name(status: FeedbackStatus) -> &'static str {
    match status {
        FeedbackStatus::Queued => "queued",
        FeedbackStatus::Bound => "bound",
        FeedbackStatus::Included => "included",
    }
}

fn feedback_status_from_db(status: &str) -> Result<FeedbackStatus, StoreError> {
    match status {
        "queued" => Ok(FeedbackStatus::Queued),
        "bound" => Ok(FeedbackStatus::Bound),
        "included" => Ok(FeedbackStatus::Included),
        _ => Err(StoreError::CorruptRecord(format!(
            "未知的自由意见状态：{status}"
        ))),
    }
}

fn feedback_batch_anchors(
    models: &[feedback::Model],
) -> Result<Option<FeedbackBatchAnchors>, StoreError> {
    let Some(first) = models.first() else {
        return Ok(None);
    };
    let status = feedback_status_from_db(&first.status)?;
    if status == FeedbackStatus::Included {
        return Err(StoreError::CorruptRecord(
            "未封存自由意见批次包含 included 记录".to_owned(),
        ));
    }
    if (status == FeedbackStatus::Queued && first.bound_tool_call_id.is_some())
        || (status == FeedbackStatus::Bound && first.bound_tool_call_id.is_none())
    {
        return Err(StoreError::CorruptRecord(
            "自由意见批次的状态与工具绑定不一致".to_owned(),
        ));
    }
    let anchors = FeedbackBatchAnchors {
        after_tool_sequence: first.after_tool_sequence,
        preferred_tool_call_id: first.preferred_tool_call_id.clone(),
        bound_tool_call_id: first.bound_tool_call_id.clone(),
        status,
    };
    for model in models.iter().skip(1) {
        if feedback_status_from_db(&model.status)? != anchors.status
            || model.after_tool_sequence != anchors.after_tool_sequence
            || model.preferred_tool_call_id != anchors.preferred_tool_call_id
            || model.bound_tool_call_id != anchors.bound_tool_call_id
        {
            return Err(StoreError::CorruptRecord(
                "同一 Run 的未封存自由意见没有共享批次锚点".to_owned(),
            ));
        }
    }
    Ok(Some(anchors))
}

/// 一条刚排进未封存批次的自由意见。
struct QueuedChapterBodyFeedback {
    id: Uuid,
    sequence: i64,
    status: FeedbackStatus,
    /// 意见已归属的工具投放（同一批次共享），未归属时为 `None`。
    bound_tool_call_id: Option<String>,
}

/// 把一条自由意见排进正文 Run 的未封存批次。
///
/// 与 [`Store::submit_run_feedback`] 共用同一套锚点规则（见 [`feedback_batch_anchors`]）：
/// 已经有未封存批次时继承它的锚点，让同一批意见随同一个工具结果一起交付；否则挂到
/// 最后一条已有结果的工具调用之后，等模型产生序号更大的下一条工具结果时由
/// [`bind_pending_feedback_to_tool`] 绑定，随该结果的 `feedback` 数组在下一次请求里
/// 交给模型。未封存期间这条意见同时挡住直接终稿提交（见
/// `commit_chapter_body_model_submission`），因此模型必须先产生一条正文编辑。
///
/// 调用方负责把工作流留在能继续写正文的状态，并写 `feedback.changed` 事件。
async fn queue_chapter_body_feedback(
    transaction: &DatabaseTransaction,
    body_run: &run::Model,
    text: String,
    citations: &[FeedbackCitation],
    now: i64,
) -> Result<QueuedChapterBodyFeedback, StoreError> {
    let existing_batch = feedback::Entity::find()
        .filter(feedback::Entity::COLUMN.run_id.eq(body_run.id))
        .filter(feedback::Entity::COLUMN.status.is_in(["queued", "bound"]))
        .order_by_asc(feedback::Entity::COLUMN.sequence)
        .all(transaction)
        .await?;
    let anchors = feedback_batch_anchors(&existing_batch)?;
    let (after_tool_sequence, preferred_tool_call_id, bound_tool_call_id, status) =
        if let Some(anchors) = anchors {
            (
                anchors.after_tool_sequence,
                anchors.preferred_tool_call_id,
                anchors.bound_tool_call_id,
                anchors.status,
            )
        } else {
            let after_tool_sequence = tool_call::Entity::find()
                .filter(tool_call::Entity::COLUMN.run_id.eq(body_run.id))
                .filter(tool_call::Entity::COLUMN.result_json.is_not_null())
                .order_by_desc(tool_call::Entity::COLUMN.sequence)
                .one(transaction)
                .await?
                .map_or(0, |tool| tool.sequence);
            (after_tool_sequence, None, None, FeedbackStatus::Queued)
        };
    let sequence = feedback::Entity::find()
        .filter(feedback::Entity::COLUMN.run_id.eq(body_run.id))
        .order_by_desc(feedback::Entity::COLUMN.sequence)
        .one(transaction)
        .await?
        .map_or(Ok(1), |previous| {
            previous.sequence.checked_add(1).ok_or_else(|| {
                StoreError::InvalidChapterState("自由意见 sequence 已达到 i64 上限".to_owned())
            })
        })?;
    let id = Uuid::new_v4();
    feedback::ActiveModel {
        id: Set(db_id(id)),
        run_id: Set(body_run.id),
        version_id: Set(body_run.version_id),
        sequence: Set(sequence),
        text: Set(text),
        citations_json: Set(serde_json::to_string(citations)?),
        after_tool_sequence: Set(after_tool_sequence),
        preferred_tool_call_id: Set(preferred_tool_call_id),
        bound_tool_call_id: Set(bound_tool_call_id.clone()),
        included_request_id: Set(None),
        status: Set(feedback_status_name(status).to_owned()),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(transaction)
    .await?;
    Ok(QueuedChapterBodyFeedback {
        id,
        sequence,
        status,
        bound_tool_call_id,
    })
}

fn chapter_body_run_status_matches_execution(
    run_status: &str,
    execution: &ExecutionStatus,
) -> bool {
    matches!(
        (run_status, execution),
        ("queued", ExecutionStatus::Queued)
            | (
                "running",
                ExecutionStatus::RequestInFlight | ExecutionStatus::ToolBoundary
            )
            | (
                "waiting",
                ExecutionStatus::WaitingAnswer | ExecutionStatus::WaitingReview
            )
            | ("paused", ExecutionStatus::Paused)
    )
}

fn writing_run_status_for_execution(execution: &ExecutionStatus) -> Option<&'static str> {
    match execution {
        ExecutionStatus::Queued => Some("queued"),
        ExecutionStatus::RequestInFlight | ExecutionStatus::ToolBoundary => Some("running"),
        ExecutionStatus::WaitingAnswer | ExecutionStatus::WaitingReview => Some("waiting"),
        ExecutionStatus::Paused => Some("paused"),
        _ => None,
    }
}

/// 在一个新工具结果落库时，为唯一未封存的自由意见批次选择该结果。
///
/// 工具基础结果保持不可变；这里只保存绑定关系。下一次模型请求会在封存快照时
/// 把同一批次的所有意见按序物化到该结果的 `feedback` 数组，故已绑定批次绝不能
/// 被之后的工具调用悄悄改绑。
async fn bind_pending_feedback_to_tool<C: ConnectionTrait>(
    connection: &C,
    run_id: RunId,
    tool_call_id: Uuid,
    tool_sequence: i64,
    now: i64,
) -> Result<Vec<Uuid>, StoreError> {
    let models = feedback::Entity::find()
        .filter(feedback::Entity::COLUMN.run_id.eq(db_id(run_id)))
        .filter(feedback::Entity::COLUMN.status.is_in(["queued", "bound"]))
        .order_by_asc(feedback::Entity::COLUMN.sequence)
        .all(connection)
        .await?;
    let Some(anchors) = feedback_batch_anchors(&models)? else {
        return Ok(Vec::new());
    };
    let tool_call_id_text = tool_call_id.to_string();
    match anchors.status {
        FeedbackStatus::Included => Err(StoreError::CorruptRecord(
            "未封存自由意见批次不应为 included".to_owned(),
        )),
        FeedbackStatus::Bound => {
            if anchors.bound_tool_call_id.as_deref() != Some(tool_call_id_text.as_str()) {
                return Err(StoreError::CorruptRecord(
                    "存在尚未封存的自由意见批次时又产生了另一条正文工具结果".to_owned(),
                ));
            }
            Ok(Vec::new())
        }
        FeedbackStatus::Queued if tool_sequence <= anchors.after_tool_sequence => Ok(Vec::new()),
        FeedbackStatus::Queued => {
            let bound_tool_call_id = tool_call_id_text;
            let mut changed_ids = Vec::with_capacity(models.len());
            for model in models {
                let id: Uuid = model.id.into();
                let mut active = model.into_active_model();
                active.status = Set("bound".to_owned());
                active.preferred_tool_call_id = Set(Some(bound_tool_call_id.clone()));
                active.bound_tool_call_id = Set(Some(bound_tool_call_id.clone()));
                active.updated_at = Set(now);
                active.update(connection).await?;
                changed_ids.push(id);
            }
            Ok(changed_ids)
        }
    }
}

/// 找到尚未随任何请求封存的最近正文工具结果。
///
/// 这是“暂停尚未命中时提交意见”的唯一可回填目标；已封存的旧结果绝不能被新的
/// 反馈改写。`error` 也是完整、可交付的工具结果，因此与 `resolved` 同等处理。
async fn latest_unsealed_chapter_body_tool_result<C: ConnectionTrait>(
    connection: &C,
    run_id: &sea_orm::prelude::TextUuid,
) -> Result<Option<(Uuid, i64)>, StoreError> {
    tool_call::Entity::find()
        .filter(tool_call::Entity::COLUMN.run_id.eq(*run_id))
        .filter(
            tool_call::Entity::COLUMN
                .status
                .is_in(["resolved", "error"]),
        )
        .filter(tool_call::Entity::COLUMN.result_json.is_not_null())
        .filter(tool_call::Entity::COLUMN.included_request_id.is_null())
        .order_by_desc(tool_call::Entity::COLUMN.sequence)
        .one(connection)
        .await?
        .map(|tool| Ok((tool.id.into(), tool.sequence)))
        .transpose()
}

/// 暂停尚未命中工具边界时，把整个未封存意见批次回填到最近的可交付结果。
///
/// 正常的“下一条工具结果”规则要求序号严格大于 `after_tool_sequence`；暂停特例
/// 则刻意允许等于它，因为意见抵达时最近结果已经形成但尚未随请求发送。所有
/// 记录同步写入相同的 preferred/bound 锚点，保证随后追加的意见不会拆批。
async fn bind_pending_feedback_to_existing_tool_for_pause<C: ConnectionTrait>(
    connection: &C,
    run_id: RunId,
    tool_call_id: Uuid,
    now: i64,
) -> Result<Vec<Uuid>, StoreError> {
    let models = feedback::Entity::find()
        .filter(feedback::Entity::COLUMN.run_id.eq(db_id(run_id)))
        .filter(feedback::Entity::COLUMN.status.is_in(["queued", "bound"]))
        .order_by_asc(feedback::Entity::COLUMN.sequence)
        .all(connection)
        .await?;
    let Some(anchors) = feedback_batch_anchors(&models)? else {
        return Ok(Vec::new());
    };
    let tool_call_id_text = tool_call_id.to_string();
    match anchors.status {
        FeedbackStatus::Included => Err(StoreError::CorruptRecord(
            "未封存自由意见批次不应为 included".to_owned(),
        )),
        FeedbackStatus::Bound => {
            if anchors.bound_tool_call_id.as_deref() != Some(tool_call_id_text.as_str()) {
                return Err(StoreError::CorruptRecord(
                    "暂停回填意见时批次已经绑定到另一条工具结果".to_owned(),
                ));
            }
            Ok(Vec::new())
        }
        FeedbackStatus::Queued => {
            let mut changed_ids = Vec::with_capacity(models.len());
            for model in models {
                let id: Uuid = model.id.into();
                let mut active = model.into_active_model();
                active.status = Set("bound".to_owned());
                active.preferred_tool_call_id = Set(Some(tool_call_id_text.clone()));
                active.bound_tool_call_id = Set(Some(tool_call_id_text.clone()));
                active.updated_at = Set(now);
                active.update(connection).await?;
                changed_ids.push(id);
            }
            Ok(changed_ids)
        }
    }
}

/// 一次由调用方算好结果的自由工具调用。
#[derive(Clone, Debug)]
pub struct FreeAgentToolCall {
    pub provider_call_id: String,
    pub name: String,
    pub arguments: Value,
    /// 已算好的工具结果，会被原样回送给模型。
    pub result: Value,
    /// `true` 表示这是一个稳定错误结果，落库状态为 `error`。
    pub failed: bool,
    pub error_code: Option<String>,
}

/// 自由工具调用经事务落库后的结果。
///
/// `newly_resolved` 用来抑制供应商重放；`requeue` 则表达这次新调用是否仍可继续
/// 同一 Run。两者必须区分，否则第三次连续工具错误仍会被调度器错误地再次投递。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppliedFreeAgentToolCall {
    pub result: Value,
    pub newly_resolved: bool,
    pub requeue: bool,
}

/// 一次顶层写作 Run 中、由调用方提供结果的自由工具调用的落库上下文。
///
/// 校验只存在这一处：只有 `dispatched` 请求、顶层 `preparation`/`chapter_outline`/
/// `chapter_body` Run、工作流正在等待该阶段模型响应且 Run 为 `running` 时才允许落库。
struct FreeAgentToolContext {
    request: model_request::Model,
    run_model: run::Model,
    run_id: RunId,
    version_id: VersionId,
    run_kind: String,
    workflow: WorkflowSnapshot,
    sequence: i64,
    tool_call_id: Uuid,
    provider_call_id: String,
    args_json: String,
    assistant_message: NewAgentMessage,
}

async fn prepare_free_agent_tool_call(
    transaction: &DatabaseTransaction,
    request_id: Uuid,
    provider_call_id: &str,
    name: &str,
    arguments: &Value,
) -> Result<FreeAgentToolContext, StoreError> {
    let request = model_request::Entity::find_by_id(db_id(request_id))
        .one(transaction)
        .await?
        .ok_or(StoreError::ModelRequestNotFound)?;
    if request.status != "dispatched" {
        return Err(StoreError::InvalidModelRequestState(format!(
            "只有 dispatched 请求可以执行 {name}，当前为 {}",
            request.status
        )));
    }
    let run_model = run::Entity::find_by_id(request.run_id)
        .one(transaction)
        .await?
        .ok_or(StoreError::RunNotFound)?;
    if run_model.parent_run_id.is_some()
        || !matches!(
            run_model.kind.as_str(),
            "preparation" | "chapter_outline" | "chapter_body"
        )
    {
        return Err(StoreError::InvalidModelRequestState(
            "该模型请求不属于顶层写作 Run".to_owned(),
        ));
    }
    let run_id: RunId = run_model.id.into();
    let version_id = run_model.version_id.into();
    let run_kind = run_model.kind.clone();
    let workflow = workflow_for_version(transaction, version_id).await?;
    if workflow.active_run_id != Some(run_id)
        || workflow.execution != ExecutionStatus::RequestInFlight
        || !question_stage_matches_run_kind(&workflow.stage, &run_kind)
    {
        return Err(StoreError::InvalidModelRequestState(format!(
            "当前工作流不接受 {name} 工具调用"
        )));
    }
    if run_model.status != "running" {
        return Err(StoreError::CorruptRecord(format!(
            "{name} 在途请求的 Run 未处于 running 状态"
        )));
    }
    let sequence = next_agent_tool_sequence(transaction, &run_model.id).await?;
    let tool_call_id = Uuid::new_v4();
    let assistant_message = NewAgentMessage {
        role: AgentMessageRole::Assistant,
        content: json!({
            "kind": "assistant_tool_calls",
            "content": Value::Null,
            "toolCalls": [{
                "id": provider_call_id,
                "name": name,
                "arguments": arguments,
            }],
        }),
        request_id: Some(request_id),
        tool_call_id: None,
    };
    Ok(FreeAgentToolContext {
        request,
        run_model,
        run_id,
        version_id,
        run_kind,
        workflow,
        sequence,
        tool_call_id,
        provider_call_id: provider_call_id.to_owned(),
        args_json: serde_json::to_string(arguments)?,
        assistant_message,
    })
}

/// 一次自由工具调用的可回送结果。
struct FreeAgentToolResult<'a> {
    name: &'a str,
    result: &'a Value,
    /// `resolved` 或 `error`。
    status: &'a str,
    /// 失败时的稳定错误码，正常结果时为 `None`。
    error_code: Option<&'a str>,
}

/// 提交一次自由工具调用：写入 assistant/tool 审计与工具调用记录，更新本次模型请求
/// 与 Run，推进工作流到下一轮并发出事件。
///
/// 返回值表示调度器是否可以继续投递同一 Run。工具返回稳定错误说明模型没有形成
/// 可推进动作，必须与其他协议纠正共用三次工程上限。
async fn commit_free_agent_tool_call(
    transaction: &DatabaseTransaction,
    context: FreeAgentToolContext,
    request_id: Uuid,
    provider_request_id: Option<String>,
    usage: &Value,
    outcome: FreeAgentToolResult<'_>,
    now: i64,
) -> Result<bool, StoreError> {
    let FreeAgentToolContext {
        request,
        run_model,
        run_id,
        version_id,
        run_kind,
        workflow,
        sequence,
        tool_call_id,
        provider_call_id,
        args_json,
        assistant_message,
    } = context;
    if run_model.consecutive_protocol_errors < 0 {
        return Err(StoreError::CorruptRecord(
            "Run consecutive_protocol_errors 不能为负数".to_owned(),
        ));
    }
    let is_protocol_error = outcome.status == "error";
    let next_error_count = if is_protocol_error {
        next_protocol_error_count(run_model.consecutive_protocol_errors)?
    } else {
        0
    };
    let stop = is_protocol_error && next_error_count >= MAX_CONSECUTIVE_PROTOCOL_ERRORS;
    let next_workflow = if stop {
        stop_writing_after_protocol_error(&workflow, run_id)?
    } else {
        continue_after_agent_tool(&workflow, run_id)?
    };
    let run_status = if stop { "stopped" } else { "queued" };
    let result_json = serde_json::to_string(outcome.result)?;
    let mut messages = vec![assistant_message];
    messages.push(NewAgentMessage {
        role: AgentMessageRole::Tool,
        content: json!({
            "kind": "tool_result",
            "content": result_json,
        }),
        request_id: Some(request_id),
        tool_call_id: Some(provider_call_id.clone()),
    });
    append_agent_messages_for_run(transaction, &run_model, messages, now).await?;
    tool_call::ActiveModel {
        id: Set(db_id(tool_call_id)),
        request_id: Set(request.id),
        run_id: Set(run_model.id),
        provider_call_id: Set(provider_call_id.clone()),
        sequence: Set(sequence),
        name: Set(outcome.name.to_owned()),
        args_json: Set(args_json),
        base_revision_id: Set(None),
        proposal_json: Set(None),
        held_json: Set(None),
        status: Set(outcome.status.to_owned()),
        result_json: Set(Some(result_json)),
        included_request_id: Set(None),
        created_at: Set(now),
        resolved_at: Set(Some(now)),
        ..Default::default()
    }
    .insert(transaction)
    .await?;
    let bound_feedback_ids = if run_kind == "chapter_body" {
        bind_pending_feedback_to_tool(transaction, run_id, tool_call_id, sequence, now).await?
    } else {
        Vec::new()
    };
    let mut active_request = request.into_active_model();
    active_request.status = Set("succeeded".to_owned());
    active_request.provider_request_id = Set(provider_request_id);
    active_request.usage_json = Set(Some(serde_json::to_string(usage)?));
    active_request.error_json = Set(None);
    active_request.updated_at = Set(now);
    active_request.completed_at = Set(Some(now));
    active_request.update(transaction).await?;
    let request_count = run_model.request_count;
    let mut active_run = run_model.into_active_model();
    active_run.status = Set(run_status.to_owned());
    active_run.consecutive_protocol_errors = Set(next_error_count);
    active_run.ended_at = Set(if stop { Some(now) } else { None });
    active_run.updated_at = Set(now);
    active_run.update(transaction).await?;
    persist_workflow(transaction, &next_workflow).await?;
    let mut tool_event = json!({
        "run_id": run_id,
        "chapter_id": next_workflow.active_chapter_id,
        "name": outcome.name,
        "provider_call_id": provider_call_id,
        "status": outcome.status,
    });
    if let Some(code) = outcome.error_code {
        tool_event
            .as_object_mut()
            .expect("工具事件必须是 JSON object")
            .insert("code".to_owned(), Value::String(code.to_owned()));
    }
    if is_protocol_error {
        tool_event
            .as_object_mut()
            .expect("工具事件必须是 JSON object")
            .insert(
                "consecutive_protocol_errors".to_owned(),
                Value::String(next_error_count.to_string()),
            );
    }
    insert_event(
        transaction,
        version_id,
        "tool.resolved",
        tool_call_id,
        next_workflow.state_revision,
        tool_event,
        now,
    )
    .await?;
    for feedback_id in bound_feedback_ids {
        insert_event(
            transaction,
            version_id,
            "feedback.changed",
            feedback_id,
            next_workflow.state_revision,
            json!({
                "chapter_id": next_workflow.active_chapter_id,
                "run_id": run_id,
                "status": "bound",
                "bound_tool_call_id": tool_call_id,
            }),
            now,
        )
        .await?;
    }
    let mut run_event = json!({
        "kind": run_kind,
        "status": run_status,
        "request_id": request_id,
        "request_count": request_count.to_string(),
    });
    if is_protocol_error {
        let event = run_event
            .as_object_mut()
            .expect("Run 事件必须是 JSON object");
        event.insert("reason".to_owned(), Value::String("tool_error".to_owned()));
        event.insert(
            "consecutive_protocol_errors".to_owned(),
            Value::String(next_error_count.to_string()),
        );
        event.insert(
            "wait_reason".to_owned(),
            if stop {
                Value::String("protocol_error_limit".to_owned())
            } else {
                Value::Null
            },
        );
    }
    insert_event(
        transaction,
        version_id,
        "run.changed",
        run_id,
        next_workflow.state_revision,
        run_event,
        now,
    )
    .await?;
    insert_event(
        transaction,
        version_id,
        "workflow.changed",
        version_id,
        next_workflow.state_revision,
        json!({
            "stage": next_workflow.stage,
            "execution": next_workflow.execution,
            "run_id": run_id,
            "chapter_id": next_workflow.active_chapter_id,
            "request_id": request_id,
            "pause_requested": next_workflow.pause_requested,
            "protocol_correction": is_protocol_error,
            "consecutive_protocol_errors": next_error_count.to_string(),
        }),
        now,
    )
    .await?;
    Ok(!stop)
}

async fn next_agent_tool_sequence<C: ConnectionTrait>(
    connection: &C,
    run_id: &sea_orm::prelude::TextUuid,
) -> Result<i64, StoreError> {
    tool_call::Entity::find()
        .filter(tool_call::Entity::COLUMN.run_id.eq(*run_id))
        .order_by_desc(tool_call::Entity::COLUMN.sequence)
        .one(connection)
        .await?
        .map_or(Ok(1), |tool| {
            tool.sequence.checked_add(1).ok_or_else(|| {
                StoreError::InvalidModelRequest("正文工具调用 sequence 已达到 i64 上限".to_owned())
            })
        })
}

/// 一个已挂起正文工具调用恢复后的持久化结果。
///
/// 该结构只在同一 Store 事务内传递；它让“显式继续”和“提交意见后自动继续”共享
/// 同一条正文/提案/反馈线性化路径，而不是各自复制一套候选应用逻辑。
struct ResolvedHeldChapterBodyTool {
    tool_call_id: Uuid,
    provider_call_id: String,
    tool_name: String,
    tool_result: Value,
    next_workflow: WorkflowSnapshot,
    run_status: String,
    request_count: i64,
    committed_revision: Option<(RevisionId, i64, String)>,
    awaiting_review: bool,
    bound_feedback_ids: Vec<Uuid>,
}

/// 在暂停命中的完整工具边界上恢复正文调用。
///
/// 调用方已经验证该 Run 是当前正文 Run 且工作流确实处于 `paused`。此函数负责余下
/// 的全部原子事实：基础修订复验、候选正文或提案、工具结果、反馈绑定、消息账本、
/// Run 状态与工作流。事件和命令回执仍由外层命令写入，以保留各命令自己的审计语义。
async fn resolve_held_chapter_body_tool(
    transaction: &DatabaseTransaction,
    workflow: &WorkflowSnapshot,
    body_run: run::Model,
    chapter_id: ChapterId,
    now: i64,
) -> Result<ResolvedHeldChapterBodyTool, StoreError> {
    let run_id: RunId = body_run.id.into();
    let version_id: VersionId = body_run.version_id.into();
    let (chapter_model, body_document) =
        chapter_and_document(transaction, chapter_id, CHAPTER_BODY_ROLE, "章节正文").await?;
    if chapter_model.status != "in_progress" {
        return Err(StoreError::InvalidChapterState(
            "章节尚未处于可编辑正文状态".to_owned(),
        ));
    }
    let chapter_version_id: VersionId = chapter_model.version_id.into();
    if chapter_version_id != version_id {
        return Err(StoreError::CorruptRecord(
            "已暂停正文工具调用章节与 Run 稿本不一致".to_owned(),
        ));
    }
    let held_call = tool_call::Entity::find()
        .filter(tool_call::Entity::COLUMN.run_id.eq(body_run.id))
        .filter(tool_call::Entity::COLUMN.status.eq("held"))
        .one(transaction)
        .await?
        .ok_or_else(|| {
            StoreError::InvalidChapterState("暂停状态缺少可恢复的完整工具调用".to_owned())
        })?;
    let held_payload = held_chapter_body_tool_payload_from_model(&held_call)?;
    let held = held_payload.held.clone();
    let current_body = current_chapter_body(transaction, chapter_id).await?;
    if current_body.revision_id != held.base_revision_id {
        return Err(StoreError::ProposalBaseRevisionConflict {
            expected: held.base_revision_id,
            actual: current_body.revision_id,
        });
    }
    let tool_call_id: Uuid = held_call.id.into();
    let request_id: Uuid = held_call.request_id.into();
    let held_sequence = held_call.sequence;
    let (mut tool_result, committed_revision, proposal) = match (
        held.candidate_content.clone(),
        held_payload.result_if_resumed,
    ) {
        (Some(candidate_content), None)
            if workflow.approval_mode == ApprovalMode::ApproveRequired =>
        {
            if tool_call::Entity::find()
                .filter(tool_call::Entity::COLUMN.run_id.eq(body_run.id))
                .filter(tool_call::Entity::COLUMN.status.eq("awaiting_review"))
                .one(transaction)
                .await?
                .is_some()
            {
                return Err(StoreError::InvalidChapterState(
                    "当前正文 Run 已有待审编辑提案".to_owned(),
                ));
            }
            let proposal = PendingChapterBodyProposal {
                tool_call_id,
                provider_call_id: held.provider_call_id.clone(),
                tool_name: held.tool_name.clone(),
                base_revision_id: held.base_revision_id,
                base_revision_no: held.base_revision_no,
                candidate_hash: candidate_content_hash(&candidate_content),
                candidate_content,
            };
            (
                awaiting_body_proposal_result(&proposal),
                None,
                Some(proposal),
            )
        }
        (Some(candidate_content), None) => {
            let dependency_revision_ids = chapter_body_revision_dependency_ids(
                transaction,
                version_id,
                chapter_id,
                held.base_revision_id,
            )
            .await?;
            let (revision_id, revision_no) = insert_document_revision_with_payload_and_source(
                transaction,
                body_document.id.into(),
                &candidate_content,
                None,
                "agent",
                held.base_revision_id,
                now,
            )
            .await?;
            for dependency_revision_id in &dependency_revision_ids {
                insert_revision_dependency(
                    transaction,
                    revision_id,
                    *dependency_revision_id,
                    "chapter_body_input",
                )
                .await?;
            }
            resolve_pending_candidate_feedback_citations(
                transaction,
                run_id,
                tool_call_id,
                &candidate_content,
                body_document.id.into(),
                revision_id,
                chapter_id,
                now,
            )
            .await?;
            (
                json!({
                    "status": "applied",
                    "tool_call_id": held.provider_call_id,
                    "base_revision_id": held.base_revision_id.map(|revision_id| revision_id.to_string()),
                    "result_revision_id": revision_id.to_string(),
                    "data": {
                        "revision_no": revision_no.to_string(),
                        "word_count": non_whitespace_char_count(&candidate_content).to_string(),
                    },
                    "feedback": [],
                }),
                Some((revision_id, revision_no, candidate_content)),
                None,
            )
        }
        (None, Some(result)) => (result, None, None),
        _ => {
            return Err(StoreError::CorruptRecord(
                "已暂停正文工具调用缺少候选内容或稳定错误结果".to_owned(),
            ));
        }
    };
    let result_object = tool_result
        .as_object_mut()
        .ok_or_else(|| StoreError::CorruptRecord("正文工具结果必须是 JSON object".to_owned()))?;
    result_object
        .entry("tool_call_id".to_owned())
        .or_insert_with(|| Value::String(held.provider_call_id.clone()));
    result_object
        .entry("base_revision_id".to_owned())
        .or_insert_with(|| {
            held.base_revision_id.map_or(Value::Null, |revision_id| {
                Value::String(revision_id.to_string())
            })
        });
    result_object
        .entry("feedback".to_owned())
        .or_insert_with(|| Value::Array(Vec::new()));
    let awaiting_review = proposal.is_some();
    let next_workflow = if awaiting_review {
        nekowriter_core::resume_chapter_body_to_proposal(workflow, chapter_id, run_id)?
    } else {
        nekowriter_core::resume_chapter_body_after_pause(workflow, chapter_id, run_id)?
    };
    let mut active_call = held_call.into_active_model();
    active_call.proposal_json = Set(proposal.as_ref().map(serde_json::to_string).transpose()?);
    active_call.held_json = Set(None);
    active_call.status = Set(if awaiting_review {
        "awaiting_review"
    } else if tool_result["status"] == "applied" {
        "resolved"
    } else {
        "error"
    }
    .to_owned());
    active_call.result_json = Set((!awaiting_review)
        .then(|| serde_json::to_string(&tool_result))
        .transpose()?);
    active_call.included_request_id = Set(None);
    active_call.resolved_at = Set((!awaiting_review).then_some(now));
    active_call.update(transaction).await?;
    if awaiting_review {
        let dependency_revision_ids = chapter_body_revision_dependency_ids(
            transaction,
            version_id,
            chapter_id,
            held.base_revision_id,
        )
        .await?;
        insert_chapter_edit_review(
            transaction,
            version_id,
            tool_call_id,
            held.base_revision_id,
            &dependency_revision_ids,
            now,
        )
        .await?;
    }
    let bound_feedback_ids = if awaiting_review {
        Vec::new()
    } else {
        bind_pending_feedback_to_tool(transaction, run_id, tool_call_id, held_sequence, now).await?
    };
    if !awaiting_review {
        append_agent_messages_for_run(
            transaction,
            &body_run,
            vec![NewAgentMessage {
                role: AgentMessageRole::Tool,
                content: json!({
                    "kind": "tool_result",
                    "content": serde_json::to_string(&tool_result)?,
                }),
                request_id: Some(request_id),
                tool_call_id: Some(held.provider_call_id.clone()),
            }],
            now,
        )
        .await?;
    }
    let run_status = if awaiting_review { "waiting" } else { "queued" }.to_owned();
    let request_count = body_run.request_count;
    let mut active_run = body_run.into_active_model();
    active_run.status = Set(run_status.clone());
    active_run.ended_at = Set(None);
    active_run.updated_at = Set(now);
    active_run.update(transaction).await?;
    persist_workflow(transaction, &next_workflow).await?;
    Ok(ResolvedHeldChapterBodyTool {
        tool_call_id,
        provider_call_id: held.provider_call_id,
        tool_name: held.tool_name,
        tool_result,
        next_workflow,
        run_status,
        request_count,
        committed_revision,
        awaiting_review,
        bound_feedback_ids,
    })
}

async fn insert_resolved_held_chapter_body_tool_events(
    transaction: &DatabaseTransaction,
    version_id: VersionId,
    chapter_id: ChapterId,
    run_id: RunId,
    resolved: &ResolvedHeldChapterBodyTool,
    source: &'static str,
    now: i64,
) -> Result<(), StoreError> {
    if let Some((revision_id, revision_no, content)) = &resolved.committed_revision {
        insert_event(
            transaction,
            version_id,
            "document.committed",
            *revision_id,
            *revision_no,
            json!({
                "chapter_id": chapter_id,
                "role": CHAPTER_BODY_ROLE,
                "word_count": non_whitespace_char_count(content),
                "source": source,
            }),
            now,
        )
        .await?;
    }
    insert_event(
        transaction,
        version_id,
        if resolved.awaiting_review {
            "tool.proposed"
        } else {
            "tool.resolved"
        },
        resolved.tool_call_id,
        resolved.next_workflow.state_revision,
        json!({
            "chapter_id": chapter_id,
            "name": resolved.tool_name,
            "provider_call_id": resolved.provider_call_id,
            "status": resolved.tool_result["status"],
            "resumed": true,
            "source": source,
        }),
        now,
    )
    .await?;
    for feedback_id in &resolved.bound_feedback_ids {
        insert_event(
            transaction,
            version_id,
            "feedback.changed",
            *feedback_id,
            resolved.next_workflow.state_revision,
            json!({
                "chapter_id": chapter_id,
                "run_id": run_id,
                "status": "bound",
                "bound_tool_call_id": resolved.tool_call_id,
            }),
            now,
        )
        .await?;
    }
    insert_event(
        transaction,
        version_id,
        "run.changed",
        run_id,
        resolved.next_workflow.state_revision,
        json!({
            "chapter_id": chapter_id,
            "kind": "chapter_body",
            "status": resolved.run_status,
            "request_count": resolved.request_count.to_string(),
        }),
        now,
    )
    .await?;
    insert_event(
        transaction,
        version_id,
        "workflow.changed",
        version_id,
        resolved.next_workflow.state_revision,
        json!({
            "stage": resolved.next_workflow.stage,
            "execution": resolved.next_workflow.execution,
            "chapter_id": chapter_id,
            "run_id": run_id,
            "pause_requested": false,
            "source": source,
        }),
        now,
    )
    .await?;
    Ok(())
}

/// 已挂起结构化章节提交恢复后的终态。
struct ResolvedHeldChapterBodySubmission {
    next_workflow: WorkflowSnapshot,
    run_status: String,
    request_id: Uuid,
    request_count: i64,
    review_id: Option<ReviewId>,
    protocol_correction: bool,
}

/// 恢复一个在“提交章节”边界命中的暂停。
///
/// 如果仍有未投递意见，或当前没有可审核的正文修订，恢复不会假装阶段已完成：它把
/// 原始助手文本与协议纠正消息写回消息账本，清除暂停并重新排队，等待新的合法正文
/// 工具结果。只有提交实际可应用时才创建终稿审核。两条路径都会清除 Run 检查点中的
/// 挂起提交。
async fn resolve_held_chapter_body_submission(
    transaction: &DatabaseTransaction,
    workflow: &WorkflowSnapshot,
    body_run: run::Model,
    chapter_id: ChapterId,
    now: i64,
) -> Result<ResolvedHeldChapterBodySubmission, StoreError> {
    let run_id: RunId = body_run.id.into();
    let version_id: VersionId = body_run.version_id.into();
    let mut checkpoint: RunCheckpoint = serde_json::from_str(&body_run.checkpoint_json)?;
    if checkpoint.chapter_id != Some(chapter_id) {
        return Err(StoreError::CorruptRecord(
            "已暂停章节提交的 Run 检查点与当前章节不匹配".to_owned(),
        ));
    }
    let held = checkpoint
        .held_chapter_body_submission
        .take()
        .ok_or_else(|| {
            StoreError::InvalidChapterState("暂停状态缺少可恢复的章节提交".to_owned())
        })?;
    if held.held.summary.trim().is_empty() {
        return Err(StoreError::CorruptRecord(
            "已暂停章节提交缺少摘要".to_owned(),
        ));
    }
    phase_submission_call_id(held.held.request_id, &held.assistant_message)?;
    let request = model_request::Entity::find_by_id(db_id(held.held.request_id))
        .one(transaction)
        .await?
        .ok_or(StoreError::ModelRequestNotFound)?;
    if request.run_id != body_run.id || request.status != "succeeded" {
        return Err(StoreError::CorruptRecord(
            "已暂停章节提交的请求状态与 Run 不一致".to_owned(),
        ));
    }
    let (chapter_model, _) =
        chapter_and_document(transaction, chapter_id, CHAPTER_BODY_ROLE, "章节正文").await?;
    let chapter_version_id: VersionId = chapter_model.version_id.into();
    if chapter_model.status != "in_progress" || chapter_version_id != version_id {
        return Err(StoreError::InvalidChapterState(
            "当前章节不属于可恢复提交的活跃稿本".to_owned(),
        ));
    }
    let has_pending_feedback = feedback::Entity::find()
        .filter(feedback::Entity::COLUMN.run_id.eq(body_run.id))
        .filter(feedback::Entity::COLUMN.status.is_in(["queued", "bound"]))
        .one(transaction)
        .await?
        .is_some();
    let mut submission_problem = None;
    let mut applicable_submission = None;
    if !has_pending_feedback {
        let body = current_chapter_body(transaction, chapter_id).await?;
        if let Some(body_revision_id) = body.revision_id {
            let body_content = body
                .content
                .ok_or_else(|| StoreError::CorruptRecord("章节正文当前修订缺少内容".to_owned()))?;
            if let Err(error) = nekowriter_core::validate_document_content(
                "章节正文",
                &body_content,
                nekowriter_core::MAX_ARTIFACT_CONTENT_LEN,
            ) {
                submission_problem = Some(error.to_string());
            }
            if submission_problem.is_none() {
                let outline_revision_id = match chapter_body_outline_dependency_revision_id(
                    transaction,
                    version_id,
                    chapter_id,
                )
                .await
                {
                    Ok(revision_id) => Some(revision_id),
                    Err(StoreError::InvalidChapterState(message)) => {
                        submission_problem = Some(message);
                        None
                    }
                    Err(error) => return Err(error),
                };
                if let Some(outline_revision_id) = outline_revision_id {
                    let dependency_revision_ids =
                        revision_dependency_ids(transaction, body_revision_id).await?;
                    if dependency_revision_ids.contains(&outline_revision_id) {
                        applicable_submission = Some((body_revision_id, dependency_revision_ids));
                    } else {
                        submission_problem = Some("当前正文未依赖已通过的章节细纲".to_owned());
                    }
                }
            }
        } else {
            submission_problem = Some("章节正文不能为空".to_owned());
        }
    }
    let correction = if has_pending_feedback {
        Some(CHAPTER_BODY_FEEDBACK_PROTOCOL_CORRECTION.to_owned())
    } else {
        submission_problem.map(|reason| {
            format!("{CHAPTER_BODY_SUBMISSION_PROTOCOL_CORRECTION}\n本次无法应用的原因：{reason}")
        })
    };
    let (next_workflow, run_status, review_id, protocol_correction) = if let Some(correction) =
        correction
    {
        let next_workflow =
            nekowriter_core::resume_chapter_body_after_pause(workflow, chapter_id, run_id)?;
        let mut messages = vec![held.assistant_message.clone()];
        messages.extend(protocol_error_tool_result_messages(
            held.held.request_id,
            &held.assistant_message,
            &correction,
            "PHASE_SUBMISSION_REJECTED",
        )?);
        messages.push(NewAgentMessage {
            role: AgentMessageRole::User,
            content: json!({
                "kind": "protocol_correction",
                "content": correction,
            }),
            request_id: Some(held.held.request_id),
            tool_call_id: None,
        });
        append_agent_messages_for_run(transaction, &body_run, messages, now).await?;
        (next_workflow, "queued".to_owned(), None, true)
    } else {
        let (body_revision_id, dependency_revision_ids) = applicable_submission
            .ok_or_else(|| StoreError::CorruptRecord("暂停章节提交缺少可应用性判定".to_owned()))?;
        let next_workflow = nekowriter_core::resume_chapter_body_submission_after_pause(
            workflow, chapter_id, run_id,
        )?;
        let review_id = insert_chapter_final_review(
            transaction,
            version_id,
            body_revision_id,
            &dependency_revision_ids,
            now,
        )
        .await?;
        append_agent_messages_for_run(
            transaction,
            &body_run,
            phase_submission_messages(held.held.request_id, held.assistant_message.clone())?,
            now,
        )
        .await?;
        (next_workflow, "waiting".to_owned(), Some(review_id), false)
    };
    let request_count = body_run.request_count;
    let mut active_run = body_run.into_active_model();
    active_run.status = Set(run_status.clone());
    active_run.checkpoint_json = Set(serde_json::to_string(&checkpoint)?);
    active_run.ended_at = Set(None);
    active_run.updated_at = Set(now);
    active_run.update(transaction).await?;
    persist_workflow(transaction, &next_workflow).await?;
    Ok(ResolvedHeldChapterBodySubmission {
        next_workflow,
        run_status,
        request_id: held.held.request_id,
        request_count,
        review_id,
        protocol_correction,
    })
}

async fn insert_resolved_held_chapter_body_submission_events(
    transaction: &DatabaseTransaction,
    version_id: VersionId,
    chapter_id: ChapterId,
    run_id: RunId,
    resolved: &ResolvedHeldChapterBodySubmission,
    source: &'static str,
    now: i64,
) -> Result<(), StoreError> {
    if let Some(review_id) = resolved.review_id {
        insert_event(
            transaction,
            version_id,
            "review.changed",
            review_id,
            resolved.next_workflow.state_revision,
            json!({
                "chapter_id": chapter_id,
                "role": CHAPTER_BODY_ROLE,
                "status": "open",
                "source": source,
            }),
            now,
        )
        .await?;
    }
    insert_event(
        transaction,
        version_id,
        "run.changed",
        run_id,
        resolved.next_workflow.state_revision,
        json!({
            "chapter_id": chapter_id,
            "kind": "chapter_body",
            "status": resolved.run_status,
            "request_id": resolved.request_id,
            "request_count": resolved.request_count.to_string(),
            "protocol_correction": resolved.protocol_correction,
        }),
        now,
    )
    .await?;
    insert_event(
        transaction,
        version_id,
        "workflow.changed",
        version_id,
        resolved.next_workflow.state_revision,
        json!({
            "stage": resolved.next_workflow.stage,
            "execution": resolved.next_workflow.execution,
            "chapter_id": chapter_id,
            "run_id": run_id,
            "request_id": resolved.request_id,
            "pause_requested": false,
            "source": source,
        }),
        now,
    )
    .await?;
    Ok(())
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub(crate) struct RunCheckpoint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    chapter_id: Option<ChapterId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context_fold: Option<ContextFold>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    held_chapter_body_submission: Option<HeldChapterBodySubmissionCheckpoint>,
}

/// 暂停的正文 Run 在任一时刻只能保留一种边界事实。
///
/// 工具调用和直接阶段提交都可能成为暂停的线性化点。将判定收敛在这里，避免恢复
/// 命令与“提交意见后恢复”各自假定暂停总是由工具调用造成，并使损坏的双重检查点
/// 在写入任何新状态前失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HeldChapterBodyPauseBoundary {
    Tool,
    Submission,
}

async fn held_chapter_body_pause_boundary<C: ConnectionTrait>(
    connection: &C,
    body_run: &run::Model,
    chapter_id: ChapterId,
) -> Result<HeldChapterBodyPauseBoundary, StoreError> {
    let checkpoint: RunCheckpoint = serde_json::from_str(&body_run.checkpoint_json)?;
    if checkpoint.chapter_id != Some(chapter_id) {
        return Err(StoreError::CorruptRecord(
            "正文暂停 Run 检查点与当前章节不匹配".to_owned(),
        ));
    }
    let held_tools = tool_call::Entity::find()
        .filter(tool_call::Entity::COLUMN.run_id.eq(body_run.id))
        .filter(tool_call::Entity::COLUMN.status.eq("held"))
        .all(connection)
        .await?;
    match (
        held_tools.len(),
        checkpoint.held_chapter_body_submission.is_some(),
    ) {
        (1, false) => Ok(HeldChapterBodyPauseBoundary::Tool),
        (0, true) => Ok(HeldChapterBodyPauseBoundary::Submission),
        (0, false) => Err(StoreError::InvalidChapterState(
            "暂停状态缺少可恢复的正文工具调用或章节提交".to_owned(),
        )),
        _ => Err(StoreError::CorruptRecord(
            "正文暂停状态同时保留了多个恢复边界".to_owned(),
        )),
    }
}

/// 已完成阶段可供后续 Agent 请求使用的纯文本折叠来源。
///
/// Store 只描述持久化 Run、章节顺序和折叠本身；App 将其映射为 agent crate 的
/// `CompletedStageContext`，避免 store 反向依赖模型协议层。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletedRunContextStage {
    Preparation,
    ChapterOutline,
    ChapterBody,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletedRunContextFold {
    pub stage: CompletedRunContextStage,
    pub chapter_order: Option<i64>,
    pub source_run_id: RunId,
    pub fold: ContextFold,
}

impl Store {
    pub async fn get_preparation_snapshot(
        &self,
        version_id: VersionId,
    ) -> Result<PreparationSnapshot, StoreError> {
        self.database
            .transaction::<_, _, StoreError>(|transaction| {
                Box::pin(async move { preparation_snapshot(transaction, version_id).await })
            })
            .await
            .map_err(transaction_error)
    }

    /// 读取当前稿本所有已通过阶段的折叠来源，供 App 在构建下一模型请求时按固定
    /// 顺序注入。原始消息仍保存在 `agent_messages`，不会因为此查询而被删除。
    pub async fn list_completed_run_context_folds(
        &self,
        version_id: VersionId,
    ) -> Result<Vec<CompletedRunContextFold>, StoreError> {
        self.database
            .transaction::<_, _, StoreError>(|transaction| {
                Box::pin(async move { completed_run_context_folds(transaction, version_id).await })
            })
            .await
            .map_err(transaction_error)
    }

    pub async fn save_novel_brief(
        &self,
        version_id: VersionId,
        command_id: Uuid,
        input: NewNovelBrief,
    ) -> Result<CurrentNovelBrief, StoreError> {
        let target = format!("/api/versions/{version_id}/brief");
        let payload_hash = payload_hash(&SaveNovelBriefPayload {
            content: &input.content,
        })?;
        let command = Command::new(command_id, SAVE_NOVEL_BRIEF_METHOD, &target, &payload_hash);

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    if workflow.stage != WorkflowStage::Initializing {
                        return Err(StoreError::InvalidPreparationState(
                            "小说目标只能在准备阶段开始前修改".to_owned(),
                        ));
                    }
                    let brief = current_novel_brief(transaction, version_id).await?;
                    let now = utc_now_millis();
                    let (revision_id, revision_no) = insert_document_revision(
                        transaction,
                        brief.document_id,
                        &input.content,
                        "user",
                        now,
                    )
                    .await?;
                    let response = CurrentNovelBrief {
                        document_id: brief.document_id,
                        revision_id: Some(revision_id),
                        content: Some(input.content),
                    };
                    insert_event(
                        transaction,
                        version_id,
                        "document.committed",
                        revision_id,
                        revision_no,
                        json!({ "role": NOVEL_BRIEF_ROLE, "word_count": non_whitespace_char_count(response.content.as_deref().unwrap_or_default()) }),
                        now,
                    )
                    .await?;
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        SAVE_NOVEL_BRIEF_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    pub async fn start_preparation(
        &self,
        version_id: VersionId,
        command_id: Uuid,
        expected_state_revision: i64,
    ) -> Result<PreparationSnapshot, StoreError> {
        let target = format!("/api/versions/{version_id}/preparation/start");
        let payload_hash = payload_hash(&StartPreparationPayload {
            expected_state_revision,
        })?;
        let command = Command::new(command_id, START_PREPARATION_METHOD, &target, &payload_hash);

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    if workflow.stage != WorkflowStage::Initializing {
                        return Err(StoreError::InvalidPreparationState(
                            "准备阶段已经开始或稿本不再处于初始化状态".to_owned(),
                        ));
                    }
                    let brief = current_novel_brief(transaction, version_id).await?;
                    if brief
                        .content
                        .as_deref()
                        .is_none_or(|content| content.trim().is_empty())
                    {
                        return Err(StoreError::NovelBriefRequired);
                    }

                    let now = utc_now_millis();
                    let run_id = Uuid::new_v4();
                    let project_id = project_id_for_version(transaction, version_id).await?;
                    ensure_no_active_top_level_write_run(transaction, project_id).await?;
                    insert_preparation_run(transaction, run_id, project_id, version_id, now)
                        .await?;
                    let next_workflow = begin_preparation(&workflow, run_id)?;
                    persist_workflow(transaction, &next_workflow).await?;
                    insert_event(
                        transaction,
                        version_id,
                        "run.changed",
                        run_id,
                        next_workflow.state_revision,
                        json!({ "kind": "preparation", "status": "queued" }),
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        version_id,
                        "workflow.changed",
                        version_id,
                        next_workflow.state_revision,
                        json!({ "stage": next_workflow.stage, "run_id": run_id }),
                        now,
                    )
                    .await?;
                    let response = preparation_snapshot(transaction, version_id).await?;
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        START_PREPARATION_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    pub async fn submit_preparation_artifacts(
        &self,
        version_id: VersionId,
        command_id: Uuid,
        expected_state_revision: i64,
        artifacts: Vec<PreparationArtifactInput>,
        chapter_plan: Option<Vec<ChapterPlanDraft>>,
    ) -> Result<PreparationSnapshot, StoreError> {
        let target = format!("/api/versions/{version_id}/preparation/artifacts");
        let payload_hash = payload_hash(&SubmitPreparationArtifactsPayload {
            expected_state_revision,
            artifacts: &artifacts,
            chapter_plan: chapter_plan.as_deref(),
        })?;
        let command = Command::new(
            command_id,
            SUBMIT_PREPARATION_ARTIFACTS_METHOD,
            &target,
            &payload_hash,
        );

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    if workflow.execution == ExecutionStatus::RequestInFlight {
                        return Err(StoreError::ResourceBusy(
                            "模型正在生成准备产物，不能同时手动提交".to_owned(),
                        ));
                    }
                    let now = utc_now_millis();
                    let next_workflow = persist_preparation_submission(
                        transaction,
                        version_id,
                        &workflow,
                        &artifacts,
                        chapter_plan.as_deref(),
                        "user",
                        now,
                    )
                    .await?;
                    let active_run_id = next_workflow.active_run_id.ok_or_else(|| {
                        StoreError::CorruptRecord("准备阶段缺少活跃 Run".to_owned())
                    })?;
                    update_preparation_run_status(
                        transaction,
                        active_run_id,
                        "waiting",
                        None,
                        None,
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        version_id,
                        "run.changed",
                        active_run_id,
                        next_workflow.state_revision,
                        json!({ "kind": "preparation", "status": "waiting" }),
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        version_id,
                        "workflow.changed",
                        version_id,
                        next_workflow.state_revision,
                        json!({ "stage": next_workflow.stage }),
                        now,
                    )
                    .await?;

                    let response = preparation_snapshot(transaction, version_id).await?;
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        SUBMIT_PREPARATION_ARTIFACTS_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    pub async fn decide_preparation_review(
        &self,
        review_id: ReviewId,
        command_id: Uuid,
        expected_state_revision: i64,
        decision: HumanReviewDecision,
        citations: Vec<FeedbackCitation>,
    ) -> Result<PreparationSnapshot, StoreError> {
        let target = format!("/api/reviews/{review_id}/decision");
        let payload_hash = payload_hash(&DecidePreparationReviewPayload {
            expected_state_revision,
            decision: &decision,
            citations: &citations,
        })?;
        let command = Command::new(
            command_id,
            DECIDE_PREPARATION_REVIEW_METHOD,
            &target,
            &payload_hash,
        );

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let review_target = preparation_review_target(transaction, review_id).await?;
                    if review_target.status != ReviewStatus::Open {
                        return Err(StoreError::ReviewAlreadyResolved);
                    }
                    let workflow =
                        workflow_for_version(transaction, review_target.version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    if workflow.stage != WorkflowStage::PreparingReviewing {
                        return Err(StoreError::InvalidPreparationState(
                            "只有准备审核中状态可以作出准备产物审核决定".to_owned(),
                        ));
                    }
                    if review_target.current_revision_id != review_target.target_revision_id {
                        return Err(StoreError::InvalidPreparationState(
                            "审核目标已不是当前修订，不能对旧修订作出决定".to_owned(),
                        ));
                    }
                    // 准备阶段审核可以引用本稿本任意已持久化文档。
                    validate_review_decision_citations(
                        transaction,
                        &citations,
                        review_target.version_id,
                        None,
                    )
                    .await?;
                    validate_review_decision_feedback(&decision, &citations)?;

                    let now = utc_now_millis();
                    persist_human_review_decision(
                        transaction,
                        review_id,
                        &decision,
                        &citations,
                        now,
                    )
                    .await?;
                    let current_artifacts =
                        current_preparation_artifacts(transaction, review_target.version_id)
                            .await?;
                    let has_open_reviews = current_artifacts.iter().any(|artifact| {
                        matches!(
                            artifact.review.as_ref().map(|review| &review.status),
                            Some(ReviewStatus::Open | ReviewStatus::Delegated)
                        )
                    });

                    let (next_workflow, run_status) = if has_open_reviews {
                        (record_preparation_review_decision(&workflow)?, None)
                    } else if current_preparation_is_approved(transaction, &current_artifacts)
                        .await?
                    {
                        let active_run_id = workflow.active_run_id.ok_or_else(|| {
                            StoreError::CorruptRecord("准备审核缺少对应的活跃 Run".to_owned())
                        })?;
                        let context_fold = build_preparation_context_fold(
                            transaction,
                            active_run_id,
                            &current_artifacts,
                        )
                        .await?;
                        let first_chapter_id = materialize_chapter_plan(
                            transaction,
                            review_target.version_id,
                            &current_artifacts,
                            now,
                        )
                        .await?;
                        let checkpoint = RunCheckpoint {
                            chapter_id: None,
                            context_fold: Some(context_fold),
                            held_chapter_body_submission: None,
                        };
                        update_preparation_run_status(
                            transaction,
                            active_run_id,
                            "succeeded",
                            Some(&checkpoint),
                            Some(now),
                            now,
                        )
                        .await?;
                        let mut next_workflow = finish_preparation_review(
                            &workflow,
                            PreparationReviewCompletion::AllApproved,
                        )?;
                        next_workflow.active_chapter_id = Some(first_chapter_id);
                        (next_workflow, Some((active_run_id, "succeeded")))
                    } else {
                        let active_run_id = workflow.active_run_id.ok_or_else(|| {
                            StoreError::CorruptRecord("准备审核缺少对应的活跃 Run".to_owned())
                        })?;
                        update_preparation_run_status(
                            transaction,
                            active_run_id,
                            "queued",
                            None,
                            None,
                            now,
                        )
                        .await?;
                        (
                            finish_preparation_review(
                                &workflow,
                                PreparationReviewCompletion::RevisionsRequired,
                            )?,
                            Some((active_run_id, "queued")),
                        )
                    };
                    persist_workflow(transaction, &next_workflow).await?;
                    insert_event(
                        transaction,
                        review_target.version_id,
                        "review.changed",
                        review_id,
                        next_workflow.state_revision,
                        json!({
                            "role": review_target.kind.stable_key(),
                            "passed": decision.passed,
                            "status": if decision.passed { "approved" } else { "rejected" },
                        }),
                        now,
                    )
                    .await?;
                    if let Some((run_id, status)) = run_status {
                        insert_event(
                            transaction,
                            review_target.version_id,
                            "run.changed",
                            run_id,
                            next_workflow.state_revision,
                            json!({ "kind": "preparation", "status": status }),
                            now,
                        )
                        .await?;
                    }
                    insert_event(
                        transaction,
                        review_target.version_id,
                        "workflow.changed",
                        review_target.version_id,
                        next_workflow.state_revision,
                        json!({ "stage": next_workflow.stage }),
                        now,
                    )
                    .await?;

                    let response =
                        preparation_snapshot(transaction, review_target.version_id).await?;
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        DECIDE_PREPARATION_REVIEW_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    pub async fn get_chapter_guidance(
        &self,
        chapter_id: ChapterId,
    ) -> Result<CurrentChapterGuidance, StoreError> {
        current_chapter_guidance(&self.database, chapter_id).await
    }

    pub async fn save_chapter_guidance(
        &self,
        chapter_id: ChapterId,
        command_id: Uuid,
        expected_state_revision: i64,
        input: NewChapterGuidance,
    ) -> Result<CurrentChapterGuidance, StoreError> {
        let target = format!("/api/chapters/{chapter_id}/guidance");
        let payload_hash = payload_hash(&SaveChapterGuidancePayload {
            expected_state_revision,
            content: &input.content,
        })?;
        let command = Command::new(
            command_id,
            SAVE_CHAPTER_GUIDANCE_METHOD,
            &target,
            &payload_hash,
        );

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let (chapter_model, guidance_document) =
                        chapter_and_guidance_document(transaction, chapter_id).await?;
                    let version_id: VersionId = chapter_model.version_id.into();
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    let next_workflow = record_chapter_guidance(&workflow, chapter_id)?;
                    let now = utc_now_millis();
                    let (revision_id, revision_no) = insert_document_revision_with_payload(
                        transaction,
                        guidance_document.id.into(),
                        &input.content,
                        None,
                        "user",
                        now,
                    )
                    .await?;
                    persist_workflow(transaction, &next_workflow).await?;
                    insert_event(
                        transaction,
                        version_id,
                        "document.committed",
                        revision_id,
                        revision_no,
                        json!({
                            "chapter_id": chapter_id,
                            "role": "chapter_guidance",
                            "word_count": non_whitespace_char_count(&input.content),
                        }),
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        version_id,
                        "workflow.changed",
                        version_id,
                        next_workflow.state_revision,
                        json!({
                            "stage": next_workflow.stage,
                            "chapter_id": chapter_id,
                        }),
                        now,
                    )
                    .await?;
                    let response = CurrentChapterGuidance {
                        document_id: guidance_document.id.into(),
                        revision_id: Some(revision_id),
                        revision_no: Some(revision_no),
                        content: Some(input.content),
                    };
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        SAVE_CHAPTER_GUIDANCE_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    pub async fn start_chapter_outline(
        &self,
        chapter_id: ChapterId,
        command_id: Uuid,
        expected_state_revision: i64,
    ) -> Result<WorkflowSnapshot, StoreError> {
        let target = format!("/api/chapters/{chapter_id}/outline/start");
        let payload_hash = payload_hash(&StartChapterOutlinePayload {
            expected_state_revision,
        })?;
        let command = Command::new(
            command_id,
            START_CHAPTER_OUTLINE_METHOD,
            &target,
            &payload_hash,
        );

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let (chapter_model, guidance_document) =
                        chapter_and_guidance_document(transaction, chapter_id).await?;
                    if chapter_model.status != "planned" {
                        return Err(StoreError::InvalidChapterState(
                            "章节已启动或不再可开始细纲".to_owned(),
                        ));
                    }
                    let version_id: VersionId = chapter_model.version_id.into();
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    let run_id = Uuid::new_v4();
                    let next_workflow = begin_chapter_outline(&workflow, chapter_id, run_id)?;
                    let now = utc_now_millis();
                    let empty_guidance_revision = if guidance_document.current_revision_id.is_none()
                    {
                        Some(
                            insert_document_revision_with_payload(
                                transaction,
                                guidance_document.id.into(),
                                "",
                                None,
                                "user",
                                now,
                            )
                            .await?,
                        )
                    } else {
                        None
                    };
                    let project_id = project_id_for_version(transaction, version_id).await?;
                    ensure_no_active_top_level_write_run(transaction, project_id).await?;
                    insert_chapter_outline_run(
                        transaction,
                        run_id,
                        project_id,
                        version_id,
                        chapter_id,
                        now,
                    )
                    .await?;
                    let chapter_next_revision = chapter_model.revision + 1;
                    let mut active_chapter = chapter_model.into_active_model();
                    active_chapter.status = Set("in_progress".to_owned());
                    active_chapter.revision = Set(chapter_next_revision);
                    active_chapter.updated_at = Set(now);
                    active_chapter.update(transaction).await?;
                    persist_workflow(transaction, &next_workflow).await?;
                    if let Some((revision_id, revision_no)) = empty_guidance_revision {
                        insert_event(
                            transaction,
                            version_id,
                            "document.committed",
                            revision_id,
                            revision_no,
                            json!({
                                "chapter_id": chapter_id,
                                "role": "chapter_guidance",
                                "word_count": 0,
                            }),
                            now,
                        )
                        .await?;
                    }
                    insert_event(
                        transaction,
                        version_id,
                        "run.changed",
                        run_id,
                        next_workflow.state_revision,
                        json!({
                            "chapter_id": chapter_id,
                            "kind": "chapter_outline",
                            "status": "queued",
                        }),
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        version_id,
                        "workflow.changed",
                        version_id,
                        next_workflow.state_revision,
                        json!({
                            "stage": next_workflow.stage,
                            "chapter_id": chapter_id,
                            "run_id": run_id,
                        }),
                        now,
                    )
                    .await?;
                    let response_json = serde_json::to_string(&next_workflow)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        START_CHAPTER_OUTLINE_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(next_workflow)
                })
            })
            .await
    }

    /// 读取当前章节的细纲文档、审核状态和已通过时的阶段折叠。
    pub async fn get_chapter_outline(
        &self,
        chapter_id: ChapterId,
    ) -> Result<ChapterOutlineSnapshot, StoreError> {
        self.database
            .transaction::<_, _, StoreError>(|transaction| {
                Box::pin(async move { chapter_outline_snapshot(transaction, chapter_id).await })
            })
            .await
            .map_err(transaction_error)
    }

    /// 切换正文 Agent 的编辑审核模式。
    ///
    /// 模式是稿本级工作流控制数据。既有待审提案不会因切换而被接受、拒绝或删除；
    /// 只有之后完整接收的新编辑调用读取新的模式。
    pub async fn set_approval_mode(
        &self,
        version_id: VersionId,
        command_id: Uuid,
        expected_state_revision: i64,
        approval_mode: ApprovalMode,
    ) -> Result<WorkflowSnapshot, StoreError> {
        let target = format!("/api/versions/{version_id}/approval-mode");
        let payload_hash = payload_hash(&SetApprovalModePayload {
            expected_state_revision,
            mode: approval_mode,
        })?;
        let command = Command::new(command_id, SET_APPROVAL_MODE_METHOD, &target, &payload_hash);
        // 两段式站点：通知与否要看事务结果，收尾走 `finish`，而 `finish` 不做预查，
        // 所以事务外的预查仍由这里承担。
        if let Some(replayed) = command.replay_if_recorded(self).await? {
            return Ok(replayed);
        }

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let now = utc_now_millis();
        let result = self
            .database
            .transaction::<_, _, StoreError>(move |transaction| {
                Box::pin(async move {
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    let next_workflow =
                        nekowriter_core::set_approval_mode(&workflow, approval_mode);
                    let changed = next_workflow != workflow;
                    if changed {
                        persist_workflow(transaction, &next_workflow).await?;
                        insert_event(
                            transaction,
                            version_id,
                            "workflow.changed",
                            version_id,
                            next_workflow.state_revision,
                            json!({
                                "stage": next_workflow.stage,
                                "execution": next_workflow.execution,
                                "approval_mode": next_workflow.approval_mode,
                            }),
                            now,
                        )
                        .await?;
                    }
                    let response_json = serde_json::to_string(&next_workflow)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        SET_APPROVAL_MODE_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok((next_workflow, changed))
                })
            })
            .await
            .map_err(transaction_error);
        // no-op 命令不通知：审批模式没变就不该让界面重读。
        let notify = Notify::on_change(matches!(&result, Ok((_, changed)) if *changed));
        let result = result.map(|(workflow, _)| workflow);
        command.finish(self, notify, result).await
    }

    /// 请求在当前正文 Agent 的下一个完整工具调用边界暂停。
    ///
    /// 该命令不取消模型网络调用，也不会先写入半截工具参数。实际进入 `paused` 的
    /// 线性化点位于 `execute_chapter_body_agent_tool_call` 收到完整调用时。
    pub async fn request_chapter_body_pause(
        &self,
        run_id: RunId,
        command_id: Uuid,
        expected_state_revision: i64,
    ) -> Result<ChapterBodySnapshot, StoreError> {
        let target = format!("/api/runs/{run_id}/pause");
        let payload_hash = payload_hash(&RunWorkflowControlPayload {
            expected_state_revision,
        })?;
        let command = Command::new(
            command_id,
            REQUEST_CHAPTER_BODY_PAUSE_METHOD,
            &target,
            &payload_hash,
        );
        // 两段式站点：通知与否要看事务结果，收尾走 `finish`，而 `finish` 不做预查，
        // 所以事务外的预查仍由这里承担。
        if let Some(replayed) = command.replay_if_recorded(self).await? {
            return Ok(replayed);
        }
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let now = utc_now_millis();
        let result = self
            .database
            .transaction::<_, _, StoreError>(move |transaction| {
                Box::pin(async move {
                    let run_model = run::Entity::find_by_id(db_id(run_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    if run_model.parent_run_id.is_some() || run_model.kind != "chapter_body" {
                        return Err(StoreError::InvalidChapterState(
                            "只有当前章节正文 Run 可以请求暂停".to_owned(),
                        ));
                    }
                    let version_id: VersionId = run_model.version_id.into();
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    let chapter_id = workflow.active_chapter_id.ok_or_else(|| {
                        StoreError::InvalidChapterState("当前正文 Run 缺少活动章节".to_owned())
                    })?;
                    if workflow.active_run_id != Some(run_id)
                        || workflow.stage != WorkflowStage::WritingDrafting
                    {
                        return Err(StoreError::InvalidChapterState(
                            "当前工作流不接受正文暂停请求".to_owned(),
                        ));
                    }
                    let body_run = chapter_body_run(transaction, run_id, chapter_id).await?;
                    if !chapter_body_run_status_matches_execution(
                        &body_run.status,
                        &workflow.execution,
                    ) {
                        return Err(StoreError::CorruptRecord(
                            "正文 Run 状态与工作流执行状态不一致".to_owned(),
                        ));
                    }
                    let next_workflow =
                        nekowriter_core::request_chapter_body_pause(&workflow, chapter_id, run_id)?;
                    let changed = next_workflow != workflow;
                    if changed {
                        persist_workflow(transaction, &next_workflow).await?;
                        insert_event(
                            transaction,
                            version_id,
                            "workflow.changed",
                            version_id,
                            next_workflow.state_revision,
                            json!({
                                "stage": next_workflow.stage,
                                "execution": next_workflow.execution,
                                "chapter_id": chapter_id,
                                "run_id": run_id,
                                "pause_requested": true,
                            }),
                            now,
                        )
                        .await?;
                    }
                    let response = chapter_body_snapshot(transaction, chapter_id).await?;
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        REQUEST_CHAPTER_BODY_PAUSE_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok((response, changed))
                })
            })
            .await
            .map_err(transaction_error);
        // no-op 命令不通知：暂停已经请求过就不该再让界面重读。
        let notify = Notify::on_change(matches!(&result, Ok((_, changed)) if *changed));
        let result = result.map(|(response, _)| response);
        command.finish(self, notify, result).await
    }

    /// 恢复一个已经命中工具边界的正文暂停。
    ///
    /// 挂起工具调用、正文修订或提案、工具消息、反馈绑定、Run/工作流与命令回执在
    /// 同一事务中完成。恢复时重新读取审核模式：自动接受会应用候选，逐次确认会转为
    /// 待审提案；两者都不会绕过原始基础修订检查。
    pub async fn resume_chapter_body_pause(
        &self,
        run_id: RunId,
        command_id: Uuid,
        expected_state_revision: i64,
    ) -> Result<ChapterBodySnapshot, StoreError> {
        let target = format!("/api/runs/{run_id}/resume");
        let payload_hash = payload_hash(&RunWorkflowControlPayload {
            expected_state_revision,
        })?;
        let command = Command::new(
            command_id,
            RESUME_CHAPTER_BODY_METHOD,
            &target,
            &payload_hash,
        );
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let now = utc_now_millis();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let run_model = run::Entity::find_by_id(db_id(run_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    if run_model.parent_run_id.is_some() || run_model.kind != "chapter_body" {
                        return Err(StoreError::InvalidChapterState(
                            "只有当前章节正文 Run 可以恢复暂停".to_owned(),
                        ));
                    }
                    let version_id: VersionId = run_model.version_id.into();
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    let chapter_id = workflow.active_chapter_id.ok_or_else(|| {
                        StoreError::InvalidChapterState("当前正文 Run 缺少活动章节".to_owned())
                    })?;
                    if workflow.active_run_id != Some(run_id)
                        || workflow.stage != WorkflowStage::WritingDrafting
                        || workflow.execution != ExecutionStatus::Paused
                        || !workflow.pause_requested
                    {
                        return Err(StoreError::InvalidChapterState(
                            "当前工作流不在可恢复的正文暂停状态".to_owned(),
                        ));
                    }
                    let body_run = chapter_body_run(transaction, run_id, chapter_id).await?;
                    if body_run.status != "paused" {
                        return Err(StoreError::CorruptRecord(
                            "已暂停正文工具调用的 Run 未处于 paused 状态".to_owned(),
                        ));
                    }
                    match held_chapter_body_pause_boundary(transaction, &body_run, chapter_id)
                        .await?
                    {
                        HeldChapterBodyPauseBoundary::Tool => {
                            let resolved = resolve_held_chapter_body_tool(
                                transaction,
                                &workflow,
                                body_run,
                                chapter_id,
                                now,
                            )
                            .await?;
                            insert_resolved_held_chapter_body_tool_events(
                                transaction,
                                version_id,
                                chapter_id,
                                run_id,
                                &resolved,
                                "agent_tool_after_pause",
                                now,
                            )
                            .await?;
                        }
                        HeldChapterBodyPauseBoundary::Submission => {
                            let resolved = resolve_held_chapter_body_submission(
                                transaction,
                                &workflow,
                                body_run,
                                chapter_id,
                                now,
                            )
                            .await?;
                            insert_resolved_held_chapter_body_submission_events(
                                transaction,
                                version_id,
                                chapter_id,
                                run_id,
                                &resolved,
                                "chapter_submission_after_pause",
                                now,
                            )
                            .await?;
                        }
                    }
                    let response = chapter_body_snapshot(transaction, chapter_id).await?;
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        RESUME_CHAPTER_BODY_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    /// 显式停止当前顶层写作 Run，并作废尚未形成稳定结果的请求 attempt。
    ///
    /// 完整停止事实、取消标记、Run/工作流、事件和命令回执在同一事务中提交。事务
    /// 成功后应用层才会根据返回的 request ID 中止本地 HTTP 任务；因此即使进程在
    /// 两步之间退出，迟到回调也会因 request 已取消而无法提交正文或推进工作流。
    pub async fn stop_writing_run(
        &self,
        run_id: RunId,
        command_id: Uuid,
        expected_state_revision: i64,
    ) -> Result<StopWritingRunResult, StoreError> {
        let target = format!("/api/runs/{run_id}/stop");
        let payload_hash = payload_hash(&RunWorkflowControlPayload {
            expected_state_revision,
        })?;
        let command = Command::new(command_id, STOP_WRITING_RUN_METHOD, &target, &payload_hash);
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let now = utc_now_millis();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let run_model = run::Entity::find_by_id(db_id(run_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    if run_model.parent_run_id.is_some()
                        || !matches!(
                            run_model.kind.as_str(),
                            "preparation" | "chapter_outline" | "chapter_body"
                        )
                    {
                        return Err(StoreError::InvalidModelRequestState(
                            "只有顶层写作 Run 可以显式停止".to_owned(),
                        ));
                    }
                    let version_id: VersionId = run_model.version_id.into();
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    let expected_run_status = writing_run_status_for_execution(&workflow.execution)
                        .ok_or_else(|| {
                            StoreError::InvalidModelRequestState(
                                "当前 Run 不在可显式停止的执行状态".to_owned(),
                            )
                        })?;
                    if run_model.status != expected_run_status {
                        return Err(StoreError::CorruptRecord(
                            "Run 状态与工作流执行状态不一致".to_owned(),
                        ));
                    }
                    let stopped_from_execution = workflow.execution.clone();
                    let next_workflow = stop_writing_workflow_run(&workflow, run_id)?;

                    let outstanding_requests = model_request::Entity::find()
                        .filter(model_request::Column::RunId.eq(run_model.id))
                        .filter(
                            model_request::Column::Status.is_in(["prepared", "dispatched"]),
                        )
                        .all(transaction)
                        .await?;
                    if outstanding_requests.len() > 1 {
                        return Err(StoreError::CorruptRecord(
                            "一个顶层写作 Run 同时存在多个未终态模型请求".to_owned(),
                        ));
                    }
                    let mut cancelled_request_ids = Vec::with_capacity(outstanding_requests.len());
                    let mut cancelled_attempt_nos = Vec::with_capacity(outstanding_requests.len());
                    for request in outstanding_requests {
                        let request_id: Uuid = request.id.into();
                        let attempt_no = request.attempt_no;
                        let was_dispatched = request.status == "dispatched";
                        let mut active_request = request.into_active_model();
                        active_request.status = Set("cancelled".to_owned());
                        active_request.error_json = Set(was_dispatched.then(|| {
                            serde_json::to_string(&json!({
                                "kind": "user_stopped",
                                "message": "用户在模型响应完成前显式停止了当前 Run",
                            }))
                        }).transpose()?);
                        active_request.updated_at = Set(now);
                        active_request.completed_at = Set(Some(now));
                        active_request.update(transaction).await?;
                        cancelled_request_ids.push(request_id);
                        cancelled_attempt_nos.push(attempt_no);
                        insert_event(
                            transaction,
                            version_id,
                            "model_request.cancelled",
                            request_id,
                            next_workflow.state_revision,
                            json!({
                                "run_id": run_id,
                                "attempt_no": attempt_no.to_string(),
                                "was_dispatched": was_dispatched,
                                "reason": "user_stopped",
                            }),
                            now,
                        )
                        .await?;
                    }

                    let mut checkpoint: Value = serde_json::from_str(&run_model.checkpoint_json)?;
                    let checkpoint = checkpoint.as_object_mut().ok_or_else(|| {
                        StoreError::CorruptRecord("Run 检查点必须是 JSON object".to_owned())
                    })?;
                    checkpoint.insert(
                        "stop".to_owned(),
                        json!({
                            "reason": "user_stopped",
                            "stopped_from_execution": stopped_from_execution,
                            "cancelled_request_ids": &cancelled_request_ids,
                            "cancelled_attempt_nos": cancelled_attempt_nos.iter().map(ToString::to_string).collect::<Vec<_>>(),
                        }),
                    );

                    let run_kind = run_model.kind.clone();
                    let request_count = run_model.request_count;
                    let mut active_run = run_model.into_active_model();
                    active_run.status = Set("stopped".to_owned());
                    active_run.checkpoint_json = Set(serde_json::to_string(&checkpoint)?);
                    active_run.ended_at = Set(Some(now));
                    active_run.updated_at = Set(now);
                    active_run.update(transaction).await?;
                    persist_workflow(transaction, &next_workflow).await?;
                    insert_event(
                        transaction,
                        version_id,
                        "run.changed",
                        run_id,
                        next_workflow.state_revision,
                        json!({
                            "kind": run_kind,
                            "status": "stopped",
                            "reason": "user_stopped",
                            "request_count": request_count.to_string(),
                            "cancelled_request_ids": &cancelled_request_ids,
                            "cancelled_attempt_nos": cancelled_attempt_nos.iter().map(ToString::to_string).collect::<Vec<_>>(),
                        }),
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        version_id,
                        "workflow.changed",
                        version_id,
                        next_workflow.state_revision,
                        json!({
                            "stage": next_workflow.stage,
                            "execution": next_workflow.execution,
                            "run_id": run_id,
                            "chapter_id": next_workflow.active_chapter_id,
                            "pause_requested": next_workflow.pause_requested,
                            "wait_reason": next_workflow.wait_reason,
                            "resume_execution": next_workflow.resume_execution,
                            "reason": "user_stopped",
                        }),
                        now,
                    )
                    .await?;
                    let response = StopWritingRunResult {
                        workflow: next_workflow,
                        cancelled_request_ids,
                    };
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        STOP_WRITING_RUN_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    /// 显式继续一个失败、停止或因服务重启而中断的顶层写作 Run。
    ///
    /// 这不是正文暂停恢复：只有异常终态才能进入这里，并会在同一事务中重新授予
    /// 一轮请求额度、增加 `resume_count`、清零连续协议错误。用户停止在待答问题、
    /// 待审提案或已暂停边界时，Core 会恢复原等待状态而非重新排队，绝不借此入口
    /// 绕过原有的人机决定。
    pub async fn continue_writing_run(
        &self,
        run_id: RunId,
        command_id: Uuid,
        expected_state_revision: i64,
    ) -> Result<WorkflowSnapshot, StoreError> {
        let target = format!("/api/runs/{run_id}/continue");
        let payload_hash = payload_hash(&RunWorkflowControlPayload {
            expected_state_revision,
        })?;
        let command = Command::new(
            command_id,
            CONTINUE_WRITING_RUN_METHOD,
            &target,
            &payload_hash,
        );
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let now = utc_now_millis();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let run_model = run::Entity::find_by_id(db_id(run_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    if run_model.parent_run_id.is_some()
                        || !matches!(
                            run_model.kind.as_str(),
                            "preparation" | "chapter_outline" | "chapter_body"
                        )
                    {
                        return Err(StoreError::InvalidModelRequestState(
                            "只有顶层写作 Run 可以显式继续".to_owned(),
                        ));
                    }
                    let version_id: VersionId = run_model.version_id.into();
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    let expected_run_status = match workflow.execution {
                        ExecutionStatus::Interrupted => "interrupted",
                        ExecutionStatus::Failed => "failed",
                        ExecutionStatus::Stopped => "stopped",
                        _ => {
                            return Err(StoreError::InvalidModelRequestState(
                                "当前 Run 不在可显式继续的异常终态".to_owned(),
                            ));
                        }
                    };
                    if run_model.status != expected_run_status {
                        return Err(StoreError::CorruptRecord(
                            "Run 状态与工作流异常终态不一致".to_owned(),
                        ));
                    }
                    let next_workflow = continue_writing_run(&workflow, run_id)?;
                    let next_run_status = writing_run_status_for_execution(
                        &next_workflow.execution,
                    )
                    .ok_or_else(|| {
                        StoreError::CorruptRecord(
                            "显式继续没有恢复到可持久化的写作 Run 状态".to_owned(),
                        )
                    })?;
                    let next_allowance_end = run_model
                        .request_count
                        .checked_add(REQUEST_ALLOWANCE_STEP)
                        .ok_or_else(|| {
                            StoreError::InvalidModelRequest(
                                "Run request_allowance_end 已达到 i64 上限".to_owned(),
                            )
                        })?;
                    let next_resume_count =
                        run_model.resume_count.checked_add(1).ok_or_else(|| {
                            StoreError::InvalidModelRequest(
                                "Run resume_count 已达到 i64 上限".to_owned(),
                            )
                        })?;
                    let run_kind = run_model.kind.clone();
                    let request_count = run_model.request_count;
                    let mut active_run = run_model.into_active_model();
                    active_run.status = Set(next_run_status.to_owned());
                    active_run.resume_count = Set(next_resume_count);
                    active_run.request_allowance_end = Set(next_allowance_end);
                    active_run.consecutive_protocol_errors = Set(0);
                    active_run.ended_at = Set(None);
                    active_run.updated_at = Set(now);
                    active_run.update(transaction).await?;
                    persist_workflow(transaction, &next_workflow).await?;
                    insert_event(
                        transaction,
                        version_id,
                        "run.changed",
                        run_id,
                        next_workflow.state_revision,
                        json!({
                            "kind": run_kind,
                            "status": next_run_status,
                            "resumed": true,
                            "resume_count": next_resume_count.to_string(),
                            "request_count": request_count.to_string(),
                            "request_allowance_end": next_allowance_end.to_string(),
                        }),
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        version_id,
                        "workflow.changed",
                        version_id,
                        next_workflow.state_revision,
                        json!({
                            "stage": next_workflow.stage,
                            "execution": next_workflow.execution,
                            "run_id": run_id,
                            "chapter_id": next_workflow.active_chapter_id,
                            "pause_requested": next_workflow.pause_requested,
                            "resume_execution": next_workflow.resume_execution,
                        }),
                        now,
                    )
                    .await?;
                    let response_json = serde_json::to_string(&next_workflow)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        CONTINUE_WRITING_RUN_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(next_workflow)
                })
            })
            .await
    }

    /// 向当前正文 Run 追加一条自由意见。
    ///
    /// 意见以独立实体保存，而非改写已经完成的工具结果或已发出的模型请求。一个
    /// Run 同时只有一个未封存批次：后续意见继承它的工具锚点，等目标工具结果被
    /// 下一次请求封存时再一起交给模型。
    pub async fn submit_run_feedback(
        &self,
        run_id: RunId,
        command_id: Uuid,
        expected_state_revision: i64,
        text: String,
        citations: Vec<FeedbackCitation>,
    ) -> Result<ChapterBodySnapshot, StoreError> {
        if text.trim().is_empty() && citations.is_empty() {
            return Err(StoreError::EmptyFeedback);
        }
        if text.chars().count() > nekowriter_core::MAX_REVIEW_ISSUES_LEN {
            return Err(StoreError::InvalidChapterBodyEdit(format!(
                "自由意见超过 {} 个字符",
                nekowriter_core::MAX_REVIEW_ISSUES_LEN
            )));
        }
        let target = format!("/api/runs/{run_id}/feedback");
        let payload_hash = payload_hash(&SubmitRunFeedbackPayload {
            expected_state_revision,
            text: &text,
            citations: &citations,
        })?;
        let command = Command::new(
            command_id,
            SUBMIT_RUN_FEEDBACK_METHOD,
            &target,
            &payload_hash,
        );

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let now = utc_now_millis();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                let text = text.clone();
                let citations = citations.clone();
                Box::pin(async move {
                    let run_model = run::Entity::find_by_id(db_id(run_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    if run_model.parent_run_id.is_some() || run_model.kind != "chapter_body" {
                        return Err(StoreError::InvalidChapterState(
                            "自由意见只能投递给当前章节正文 Run".to_owned(),
                        ));
                    }
                    let version_id: VersionId = run_model.version_id.into();
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    let chapter_id = workflow.active_chapter_id.ok_or_else(|| {
                        StoreError::InvalidChapterState("当前正文 Run 缺少活动章节".to_owned())
                    })?;
                    if workflow.active_run_id != Some(run_id)
                        || workflow.stage != WorkflowStage::WritingDrafting
                    {
                        return Err(StoreError::InvalidChapterState(
                            "当前工作流不接受正文自由意见".to_owned(),
                        ));
                    }
                    validate_chapter_body_feedback_citations(
                        transaction,
                        &citations,
                        version_id,
                        chapter_id,
                        run_id,
                        CandidateCitationScope::HeldCurrentRun,
                    )
                    .await?;
                    let body_run = chapter_body_run(transaction, run_id, chapter_id).await?;
                    if !chapter_body_run_status_matches_execution(
                        &body_run.status,
                        &workflow.execution,
                    ) {
                        return Err(StoreError::CorruptRecord(
                            "正文 Run 状态与工作流执行状态不一致".to_owned(),
                        ));
                    }
                    let held_boundary = if workflow.execution == ExecutionStatus::Paused {
                        Some(
                            held_chapter_body_pause_boundary(transaction, &body_run, chapter_id)
                                .await?,
                        )
                    } else {
                        None
                    };
                    let resumes_held_boundary = held_boundary.is_some();
                    let cancels_pending_pause = workflow.pause_requested && !resumes_held_boundary;
                    let feedback_workflow = if resumes_held_boundary {
                        None
                    } else if cancels_pending_pause {
                        Some(cancel_chapter_body_pause_for_feedback(
                            &workflow, chapter_id, run_id,
                        )?)
                    } else {
                        Some(record_chapter_body_feedback(&workflow, chapter_id, run_id)?)
                    };

                    let QueuedChapterBodyFeedback {
                        id: feedback_id,
                        sequence,
                        status,
                        bound_tool_call_id,
                    } = queue_chapter_body_feedback(transaction, &body_run, text, &citations, now)
                        .await?;
                    match held_boundary {
                        Some(HeldChapterBodyPauseBoundary::Tool) => {
                            let resolved = resolve_held_chapter_body_tool(
                                transaction,
                                &workflow,
                                body_run.clone(),
                                chapter_id,
                                now,
                            )
                            .await?;
                            if !resolved.bound_feedback_ids.contains(&feedback_id) {
                                insert_event(
                                    transaction,
                                    version_id,
                                    "feedback.changed",
                                    feedback_id,
                                    resolved.next_workflow.state_revision,
                                    json!({
                                        "chapter_id": chapter_id,
                                        "run_id": run_id,
                                        "status": feedback_status_name(status),
                                        "sequence": sequence.to_string(),
                                        "bound_tool_call_id": bound_tool_call_id,
                                    }),
                                    now,
                                )
                                .await?;
                            }
                            insert_resolved_held_chapter_body_tool_events(
                                transaction,
                                version_id,
                                chapter_id,
                                run_id,
                                &resolved,
                                "feedback_resumes_held_tool",
                                now,
                            )
                            .await?;
                        }
                        Some(HeldChapterBodyPauseBoundary::Submission) => {
                            let resolved = resolve_held_chapter_body_submission(
                                transaction,
                                &workflow,
                                body_run.clone(),
                                chapter_id,
                                now,
                            )
                            .await?;
                            insert_event(
                                transaction,
                                version_id,
                                "feedback.changed",
                                feedback_id,
                                resolved.next_workflow.state_revision,
                                json!({
                                    "chapter_id": chapter_id,
                                    "run_id": run_id,
                                    "status": feedback_status_name(status),
                                    "sequence": sequence.to_string(),
                                    "bound_tool_call_id": bound_tool_call_id,
                                }),
                                now,
                            )
                            .await?;
                            insert_resolved_held_chapter_body_submission_events(
                                transaction,
                                version_id,
                                chapter_id,
                                run_id,
                                &resolved,
                                "feedback_resumes_held_submission",
                                now,
                            )
                            .await?;
                        }
                        None => {
                            let next_workflow = feedback_workflow.as_ref().ok_or_else(|| {
                                StoreError::CorruptRecord("自由意见缺少下一工作流状态".to_owned())
                            })?;
                            let bound_feedback_ids = if cancels_pending_pause {
                                match latest_unsealed_chapter_body_tool_result(
                                    transaction,
                                    &body_run.id,
                                )
                                .await?
                                {
                                    Some((tool_call_id, _)) => {
                                        bind_pending_feedback_to_existing_tool_for_pause(
                                            transaction,
                                            run_id,
                                            tool_call_id,
                                            now,
                                        )
                                        .await?
                                    }
                                    None => Vec::new(),
                                }
                            } else {
                                Vec::new()
                            };
                            persist_workflow(transaction, next_workflow).await?;
                            if !bound_feedback_ids.contains(&feedback_id) {
                                let final_status = if bound_tool_call_id.is_some() {
                                    FeedbackStatus::Bound
                                } else {
                                    status
                                };
                                insert_event(
                                    transaction,
                                    version_id,
                                    "feedback.changed",
                                    feedback_id,
                                    next_workflow.state_revision,
                                    json!({
                                        "chapter_id": chapter_id,
                                        "run_id": run_id,
                                        "status": feedback_status_name(final_status),
                                        "sequence": sequence.to_string(),
                                        "bound_tool_call_id": bound_tool_call_id,
                                    }),
                                    now,
                                )
                                .await?;
                            }
                            for bound_feedback_id in bound_feedback_ids {
                                insert_event(
                                    transaction,
                                    version_id,
                                    "feedback.changed",
                                    bound_feedback_id,
                                    next_workflow.state_revision,
                                    json!({
                                        "chapter_id": chapter_id,
                                        "run_id": run_id,
                                        "status": "bound",
                                    }),
                                    now,
                                )
                                .await?;
                            }
                            insert_event(
                                transaction,
                                version_id,
                                "workflow.changed",
                                version_id,
                                next_workflow.state_revision,
                                json!({
                                    "stage": next_workflow.stage,
                                    "execution": next_workflow.execution,
                                    "chapter_id": chapter_id,
                                    "run_id": run_id,
                                    "pause_requested": next_workflow.pause_requested,
                                }),
                                now,
                            )
                            .await?;
                        }
                    }
                    let response = chapter_body_snapshot(transaction, chapter_id).await?;
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        SUBMIT_RUN_FEEDBACK_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    /// 读取当前正文 Run 已经绑定到工具结果的自由意见。
    ///
    /// Store 返回原始工具调用 ID 而非数据库 UUID，使应用层只需在重放 `tool` 消息
    /// 时把这些条目附入对应 JSON 结果。实际“included”状态仍在请求封存事务中改变，
    /// 本读取不会提前声称意见已发出。
    pub async fn list_materialized_chapter_body_feedback(
        &self,
        run_id: RunId,
    ) -> Result<Vec<MaterializedChapterBodyFeedback>, StoreError> {
        self.database
            .transaction::<_, _, StoreError>(|transaction| {
                Box::pin(async move {
                    let run_model = run::Entity::find_by_id(db_id(run_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    if run_model.parent_run_id.is_some() || run_model.kind != "chapter_body" {
                        return Err(StoreError::InvalidChapterState(
                            "只有章节正文 Run 可以读取自由意见".to_owned(),
                        ));
                    }
                    let tools = tool_call::Entity::find()
                        .filter(tool_call::Entity::COLUMN.run_id.eq(run_model.id))
                        .all(transaction)
                        .await?
                        .into_iter()
                        .map(|tool| (tool.id.clone().to_string(), tool))
                        .collect::<HashMap<_, _>>();
                    let models = feedback::Entity::find()
                        .filter(feedback::Entity::COLUMN.run_id.eq(run_model.id))
                        .filter(feedback::Entity::COLUMN.status.is_in(["bound", "included"]))
                        .order_by_asc(feedback::Entity::COLUMN.sequence)
                        .all(transaction)
                        .await?;
                    models
                        .iter()
                        .map(|model| {
                            let status = feedback_status_from_db(&model.status)?;
                            let bound_tool_call_id =
                                model.bound_tool_call_id.as_ref().ok_or_else(|| {
                                    StoreError::CorruptRecord(
                                        "已绑定自由意见缺少 bound_tool_call_id".to_owned(),
                                    )
                                })?;
                            if (status == FeedbackStatus::Bound
                                && model.included_request_id.is_some())
                                || (status == FeedbackStatus::Included
                                    && model.included_request_id.is_none())
                            {
                                return Err(StoreError::CorruptRecord(
                                    "自由意见状态与 included_request_id 不一致".to_owned(),
                                ));
                            }
                            let tool = tools.get(bound_tool_call_id).ok_or_else(|| {
                                StoreError::CorruptRecord(
                                    "自由意见绑定了不属于当前 Run 的工具调用".to_owned(),
                                )
                            })?;
                            if tool.result_json.is_none() {
                                return Err(StoreError::CorruptRecord(
                                    "自由意见绑定的工具调用尚无可物化结果".to_owned(),
                                ));
                            }
                            let citations = feedback_citations_from_json(&model.citations_json)?;
                            Ok(MaterializedChapterBodyFeedback {
                                provider_tool_call_id: tool.provider_call_id.clone(),
                                id: model.id.into(),
                                sequence: model.sequence,
                                text: model.text.clone(),
                                citations,
                                status,
                            })
                        })
                        .collect()
                })
            })
            .await
            .map_err(transaction_error)
    }

    /// 返回本轮即将首次纳入下一请求快照的写作工具结果调用 ID。
    ///
    /// 调度器将该列表写入不可变请求诊断快照；Store 在 capture 事务中再次精确校验
    /// 并标记结果已经 included，避免暂停特例把新意见回填到早已发出的历史结果。
    pub async fn list_unsealed_agent_tool_result_provider_ids(
        &self,
        run_id: RunId,
    ) -> Result<Vec<String>, StoreError> {
        self.database
            .transaction::<_, _, StoreError>(|transaction| {
                Box::pin(async move {
                    let run_model = run::Entity::find_by_id(db_id(run_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    if run_model.parent_run_id.is_some()
                        || !matches!(
                            run_model.kind.as_str(),
                            "preparation" | "chapter_outline" | "chapter_body"
                        )
                    {
                        return Err(StoreError::InvalidChapterState(
                            "只有顶层写作 Run 可以读取未封存工具结果".to_owned(),
                        ));
                    }
                    tool_call::Entity::find()
                        .filter(tool_call::Entity::COLUMN.run_id.eq(run_model.id))
                        .filter(
                            tool_call::Entity::COLUMN
                                .status
                                .is_in(["resolved", "error"]),
                        )
                        .filter(tool_call::Entity::COLUMN.result_json.is_not_null())
                        .filter(tool_call::Entity::COLUMN.included_request_id.is_null())
                        .order_by_asc(tool_call::Entity::COLUMN.sequence)
                        .all(transaction)
                        .await?
                        .into_iter()
                        .map(|tool| {
                            if tool.provider_call_id.trim().is_empty() {
                                Err(StoreError::CorruptRecord(
                                    "未封存写作工具结果缺少 provider_call_id".to_owned(),
                                ))
                            } else {
                                Ok(tool.provider_call_id)
                            }
                        })
                        .collect()
                })
            })
            .await
            .map_err(transaction_error)
    }

    /// 执行一条完整到达的 `ask_user` 调用。
    ///
    /// 合法问题会把模型请求收束为 `waiting_answer`，并原子写入 assistant/tool 审计、
    /// Question、Run 和工作流。参数无效时不会让模型调用消失：同样持久化一条稳定
    /// 错误工具结果并重新排队，让 Agent 能在下一轮自行修正。
    pub async fn execute_agent_question_tool_call(
        &self,
        request_id: Uuid,
        provider_request_id: Option<String>,
        usage: Value,
        call: AgentQuestionToolCall,
    ) -> Result<AppliedAgentQuestionToolCall, StoreError> {
        validate_agent_question_tool_call(&call)?;
        if !usage.is_object() {
            return Err(StoreError::InvalidModelRequest(
                "ask_user 模型响应的 usage 必须是 JSON object".to_owned(),
            ));
        }
        if provider_request_id
            .as_deref()
            .is_some_and(|provider_request_id| provider_request_id.trim().is_empty())
        {
            return Err(StoreError::InvalidModelRequest(
                "provider_request_id 不能是空字符串".to_owned(),
            ));
        }
        let args_json = serde_json::to_string(&call.arguments)?;
        let now = utc_now_millis();
        let result = self
            .database
            .transaction::<_, _, StoreError>(|transaction| {
                let call = call.clone();
                let args_json = args_json.clone();
                let provider_request_id = provider_request_id.clone();
                let usage = usage.clone();
                Box::pin(async move {
                    let request = model_request::Entity::find_by_id(db_id(request_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::ModelRequestNotFound)?;
                    if let Some(existing) = tool_call::Entity::find()
                        .filter(tool_call::Entity::COLUMN.request_id.eq(request.id))
                        .filter(
                            tool_call::Entity::COLUMN
                                .provider_call_id
                                .eq(call.provider_call_id.clone()),
                        )
                        .one(transaction)
                        .await?
                    {
                        if existing.name != "ask_user" || existing.args_json != args_json {
                            return Err(StoreError::InvalidModelRequestState(
                                "同一 provider tool call ID 的名称或参数发生变化".to_owned(),
                            ));
                        }
                        let provider_call_id = existing.provider_call_id.clone();
                        return match existing.status.as_str() {
                            "awaiting_answer" => {
                                let question_model = question::Entity::find()
                                    .filter(question::Entity::COLUMN.tool_call_id.eq(existing.id))
                                    .one(transaction)
                                    .await?
                                    .ok_or_else(|| {
                                        StoreError::CorruptRecord(
                                            "待答 ask_user 工具调用缺少问题记录".to_owned(),
                                        )
                                    })?;
                                Ok(AppliedAgentQuestionToolCall::AwaitingAnswer {
                                    provider_call_id,
                                    question: pending_user_question_from_model(&question_model)?,
                                    newly_created: false,
                                })
                            }
                            "resolved" | "error" | "included" => {
                                let result_json = existing.result_json.ok_or_else(|| {
                                    StoreError::CorruptRecord(
                                        "已完成 ask_user 工具调用缺少 result_json".to_owned(),
                                    )
                                })?;
                                let result: Value = serde_json::from_str(&result_json)?;
                                if !result.is_object() {
                                    return Err(StoreError::CorruptRecord(
                                        "ask_user 工具结果必须是 JSON object".to_owned(),
                                    ));
                                }
                                Ok(AppliedAgentQuestionToolCall::Resolved {
                                    provider_call_id,
                                    result,
                                    newly_resolved: false,
                                    requeue: false,
                                })
                            }
                            _ => Err(StoreError::CorruptRecord(
                                "ask_user 工具调用处于未知状态".to_owned(),
                            )),
                        };
                    }
                    let FreeAgentToolContext {
                        request,
                        run_model,
                        run_id,
                        version_id,
                        run_kind,
                        workflow,
                        sequence,
                        tool_call_id,
                        provider_call_id,
                        args_json,
                        assistant_message,
                    } = prepare_free_agent_tool_call(
                        transaction,
                        request_id,
                        &call.provider_call_id,
                        "ask_user",
                        &call.arguments,
                    )
                    .await?;
                    match parse_agent_question_schema(&call.arguments) {
                        Ok(schema) => {
                            let next_workflow = wait_for_user_answer(&workflow, run_id)?;
                            let question_id = Uuid::new_v4();
                            let pending_question = PendingUserQuestion {
                                id: question_id,
                                tool_call_id,
                                schema: schema.clone(),
                                created_at_ms: now,
                            };
                            append_agent_messages_for_run(
                                transaction,
                                &run_model,
                                vec![assistant_message],
                                now,
                            )
                            .await?;
                            tool_call::ActiveModel {
                                id: Set(db_id(tool_call_id)),
                                request_id: Set(request.id),
                                run_id: Set(run_model.id),
                                provider_call_id: Set(call.provider_call_id.clone()),
                                sequence: Set(sequence),
                                name: Set("ask_user".to_owned()),
                                args_json: Set(args_json),
                                base_revision_id: Set(None),
                                proposal_json: Set(None),
                                held_json: Set(None),
                                status: Set("awaiting_answer".to_owned()),
                                result_json: Set(None),
                                included_request_id: Set(None),
                                created_at: Set(now),
                                resolved_at: Set(None),
                                ..Default::default()
                            }
                            .insert(transaction)
                            .await?;
                            question::ActiveModel {
                                id: Set(db_id(question_id)),
                                tool_call_id: Set(db_id(tool_call_id)),
                                schema_json: Set(serde_json::to_string(&schema)?),
                                answer_json: Set(None),
                                status: Set("open".to_owned()),
                                created_at: Set(now),
                                updated_at: Set(now),
                                ..Default::default()
                            }
                            .insert(transaction)
                            .await?;
                            let mut active_request = request.into_active_model();
                            active_request.status = Set("succeeded".to_owned());
                            active_request.provider_request_id = Set(provider_request_id);
                            active_request.usage_json = Set(Some(serde_json::to_string(&usage)?));
                            active_request.error_json = Set(None);
                            active_request.updated_at = Set(now);
                            active_request.completed_at = Set(Some(now));
                            active_request.update(transaction).await?;
                            let request_count = run_model.request_count;
                            let mut active_run = run_model.into_active_model();
                            active_run.status = Set("waiting".to_owned());
                            active_run.consecutive_protocol_errors = Set(0);
                            active_run.ended_at = Set(None);
                            active_run.updated_at = Set(now);
                            active_run.update(transaction).await?;
                            persist_workflow(transaction, &next_workflow).await?;
                            insert_event(
                                transaction,
                                version_id,
                                "question.created",
                                question_id,
                                next_workflow.state_revision,
                                json!({
                                    "run_id": run_id,
                                    "tool_call_id": tool_call_id,
                                    "provider_call_id": call.provider_call_id,
                                    "question_count": schema.questions.len(),
                                }),
                                now,
                            )
                            .await?;
                            insert_event(
                                transaction,
                                version_id,
                                "run.changed",
                                run_id,
                                next_workflow.state_revision,
                                json!({
                                    "kind": run_kind.clone(),
                                    "status": "waiting",
                                    "request_id": request_id,
                                    "request_count": request_count.to_string(),
                                    "wait_reason": "ask_user",
                                }),
                                now,
                            )
                            .await?;
                            insert_event(
                                transaction,
                                version_id,
                                "workflow.changed",
                                version_id,
                                next_workflow.state_revision,
                                json!({
                                    "stage": next_workflow.stage,
                                    "execution": next_workflow.execution,
                                    "run_id": run_id,
                                    "chapter_id": next_workflow.active_chapter_id,
                                    "request_id": request_id,
                                    "pause_requested": next_workflow.pause_requested,
                                }),
                                now,
                            )
                            .await?;
                            Ok(AppliedAgentQuestionToolCall::AwaitingAnswer {
                                provider_call_id: call.provider_call_id,
                                question: pending_question,
                                newly_created: true,
                            })
                        }
                        Err(StoreError::InvalidQuestion(message)) => {
                            let tool_result = question_tool_error_result(
                                &call.provider_call_id,
                                "TOOL_ARGUMENTS_INVALID",
                                message,
                            );
                            let requeue = commit_free_agent_tool_call(
                                transaction,
                                FreeAgentToolContext {
                                    request,
                                    run_model,
                                    run_id,
                                    version_id,
                                    run_kind,
                                    workflow,
                                    sequence,
                                    tool_call_id,
                                    provider_call_id,
                                    args_json,
                                    assistant_message,
                                },
                                request_id,
                                provider_request_id,
                                &usage,
                                FreeAgentToolResult {
                                    name: "ask_user",
                                    result: &tool_result,
                                    status: "error",
                                    error_code: Some("TOOL_ARGUMENTS_INVALID"),
                                },
                                now,
                            )
                            .await?;
                            Ok(AppliedAgentQuestionToolCall::Resolved {
                                provider_call_id: call.provider_call_id,
                                result: tool_result,
                                newly_resolved: true,
                                requeue,
                            })
                        }
                        Err(error) => Err(error),
                    }
                })
            })
            .await
            .map_err(transaction_error);
        match result {
            Ok(execution) => {
                if matches!(
                    execution,
                    AppliedAgentQuestionToolCall::AwaitingAnswer {
                        newly_created: true,
                        ..
                    } | AppliedAgentQuestionToolCall::Resolved {
                        newly_resolved: true,
                        ..
                    }
                ) {
                    self.notify_event_listeners();
                }
                Ok(execution)
            }
            Err(error) => Err(error),
        }
    }

    /// 读取一次自由工具调用已经落库的结果。
    ///
    /// 供应商回调重放时调用方先用它跳过重复的检索与重复扣费。
    ///
    /// # Errors
    ///
    /// 工具结果不是 JSON object 或数据库读取失败时返回错误。
    pub async fn free_agent_tool_result(
        &self,
        request_id: Uuid,
        provider_call_id: &str,
    ) -> Result<Option<Value>, StoreError> {
        let request = model_request::Entity::find_by_id(db_id(request_id))
            .one(&self.database)
            .await?
            .ok_or(StoreError::ModelRequestNotFound)?;
        let Some(existing) = tool_call::Entity::find()
            .filter(tool_call::Entity::COLUMN.request_id.eq(request.id))
            .filter(
                tool_call::Entity::COLUMN
                    .provider_call_id
                    .eq(provider_call_id.to_owned()),
            )
            .one(&self.database)
            .await?
        else {
            return Ok(None);
        };
        let Some(result_json) = existing.result_json else {
            return Ok(None);
        };
        let result: Value = serde_json::from_str(&result_json)?;
        if !result.is_object() {
            return Err(StoreError::CorruptRecord(
                "自由工具结果必须是 JSON object".to_owned(),
            ));
        }
        Ok(Some(result))
    }

    /// 提交一次由调用方算好结果的自由工具调用（目前只有 `query_knowledge`）。
    ///
    /// 结果落库与工作流推进在一个事务里完成；同一 `provider_call_id` 的重放只返回
    /// 已提交结果，不重复写入、不重复推进。
    ///
    /// # Errors
    ///
    /// 参数无效、请求或 Run 不存在、工作流不接受该工具、同一调用 ID 的名称或参数
    /// 发生变化，或数据库操作失败时返回错误。
    pub async fn execute_free_agent_tool_call(
        &self,
        request_id: Uuid,
        provider_request_id: Option<String>,
        usage: Value,
        call: FreeAgentToolCall,
    ) -> Result<AppliedFreeAgentToolCall, StoreError> {
        if call.provider_call_id.trim().is_empty() {
            return Err(StoreError::InvalidModelRequest(
                "自由工具 provider_call_id 不能为空".to_owned(),
            ));
        }
        if !usage.is_object() {
            return Err(StoreError::InvalidModelRequest(
                "自由工具调用的模型 usage 必须是 JSON object".to_owned(),
            ));
        }
        if provider_request_id
            .as_deref()
            .is_some_and(|provider_request_id| provider_request_id.trim().is_empty())
        {
            return Err(StoreError::InvalidModelRequest(
                "provider_request_id 不能是空字符串".to_owned(),
            ));
        }
        let args_json = serde_json::to_string(&call.arguments)?;
        let now = utc_now_millis();
        let result = self
            .database
            .transaction::<_, _, StoreError>(|transaction| {
                let call = call.clone();
                let args_json = args_json.clone();
                let provider_request_id = provider_request_id.clone();
                let usage = usage.clone();
                Box::pin(async move {
                    if let Some(request) = model_request::Entity::find_by_id(db_id(request_id))
                        .one(transaction)
                        .await?
                        && let Some(existing) = tool_call::Entity::find()
                            .filter(tool_call::Entity::COLUMN.request_id.eq(request.id))
                            .filter(
                                tool_call::Entity::COLUMN
                                    .provider_call_id
                                    .eq(call.provider_call_id.clone()),
                            )
                            .one(transaction)
                            .await?
                    {
                        if existing.name != call.name || existing.args_json != args_json {
                            return Err(StoreError::InvalidModelRequestState(
                                "同一 provider tool call ID 的名称或参数发生变化".to_owned(),
                            ));
                        }
                        let result_json = existing.result_json.ok_or_else(|| {
                            StoreError::CorruptRecord(
                                "已完成自由工具调用缺少 result_json".to_owned(),
                            )
                        })?;
                        return Ok((
                            AppliedFreeAgentToolCall {
                                result: serde_json::from_str(&result_json)?,
                                newly_resolved: false,
                                requeue: false,
                            },
                            false,
                        ));
                    }
                    let context = prepare_free_agent_tool_call(
                        transaction,
                        request_id,
                        &call.provider_call_id,
                        &call.name,
                        &call.arguments,
                    )
                    .await?;
                    let requeue = commit_free_agent_tool_call(
                        transaction,
                        context,
                        request_id,
                        provider_request_id,
                        &usage,
                        FreeAgentToolResult {
                            name: &call.name,
                            result: &call.result,
                            status: if call.failed { "error" } else { "resolved" },
                            error_code: call.error_code.as_deref(),
                        },
                        now,
                    )
                    .await?;
                    Ok((
                        AppliedFreeAgentToolCall {
                            result: call.result.clone(),
                            newly_resolved: true,
                            requeue,
                        },
                        true,
                    ))
                })
            })
            .await
            .map_err(transaction_error)?;
        if result.1 {
            self.notify_event_listeners();
        }
        Ok(result.0)
    }

    /// 验证并提交一组 `ask_user` 答案，形成同一工具调用的可回送结果。
    pub async fn answer_user_question(
        &self,
        question_id: QuestionId,
        command_id: Uuid,
        expected_state_revision: i64,
        answers: UserQuestionAnswers,
    ) -> Result<QuestionAnswerResult, StoreError> {
        let target = format!("/api/questions/{question_id}/answer");
        let payload_hash = payload_hash(&AnswerQuestionPayload {
            expected_state_revision,
            answers: &answers,
        })?;
        let command = Command::new(command_id, ANSWER_QUESTION_METHOD, &target, &payload_hash);
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let now = utc_now_millis();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                let answers = answers.clone();
                Box::pin(async move {
                    let question_model = question::Entity::find_by_id(db_id(question_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::QuestionNotFound)?;
                    let tool_model = tool_call::Entity::find_by_id(question_model.tool_call_id)
                        .one(transaction)
                        .await?
                        .ok_or_else(|| {
                            StoreError::CorruptRecord("问题引用的工具调用不存在".to_owned())
                        })?;
                    let run_model = run::Entity::find_by_id(tool_model.run_id)
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    if run_model.parent_run_id.is_some()
                        || !matches!(
                            run_model.kind.as_str(),
                            "preparation" | "chapter_outline" | "chapter_body"
                        )
                    {
                        return Err(StoreError::QuestionNotFound);
                    }
                    let run_id: RunId = run_model.id.into();
                    let version_id = run_model.version_id.into();
                    let run_kind = run_model.kind.clone();
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    if workflow.active_run_id != Some(run_id)
                        || workflow.execution != ExecutionStatus::WaitingAnswer
                        || !question_stage_matches_run_kind(&workflow.stage, &run_kind)
                    {
                        return Err(StoreError::InvalidModelRequestState(
                            "当前工作流不接受该问题的答案".to_owned(),
                        ));
                    }
                    if run_model.status != "waiting" {
                        return Err(StoreError::CorruptRecord(
                            "待答问题的 Run 未处于 waiting 状态".to_owned(),
                        ));
                    }
                    if question_model.status == "answered" {
                        return Err(StoreError::QuestionAlreadyAnswered);
                    }
                    if question_model.status != "open" || question_model.answer_json.is_some() {
                        return Err(StoreError::CorruptRecord(
                            "待答问题的状态与答案字段不一致".to_owned(),
                        ));
                    }
                    if tool_model.status != "awaiting_answer" || tool_model.result_json.is_some() {
                        return Err(StoreError::CorruptRecord(
                            "待答问题的工具调用状态不一致".to_owned(),
                        ));
                    }
                    let schema: UserQuestionSchema =
                        serde_json::from_str(&question_model.schema_json)?;
                    let normalized_answers = schema
                        .validate_answers(&answers.answers)
                        .map_err(|error| StoreError::InvalidQuestion(error.to_string()))?;
                    let normalized = UserQuestionAnswers {
                        answers: normalized_answers.clone(),
                    };
                    let tool_result =
                        question_tool_result(&tool_model.provider_call_id, &normalized_answers);
                    let next_workflow = continue_after_user_answer(&workflow, run_id)?;
                    let mut active_question = question_model.into_active_model();
                    active_question.answer_json = Set(Some(serde_json::to_string(&normalized)?));
                    active_question.status = Set("answered".to_owned());
                    active_question.updated_at = Set(now);
                    active_question.update(transaction).await?;
                    let tool_call_id: Uuid = tool_model.id.into();
                    let tool_sequence = tool_model.sequence;
                    let provider_call_id = tool_model.provider_call_id.clone();
                    let tool_request_id: Uuid = tool_model.request_id.into();
                    let mut active_tool = tool_model.into_active_model();
                    active_tool.status = Set("resolved".to_owned());
                    active_tool.result_json = Set(Some(serde_json::to_string(&tool_result)?));
                    active_tool.included_request_id = Set(None);
                    active_tool.resolved_at = Set(Some(now));
                    active_tool.update(transaction).await?;
                    append_agent_messages_for_run(
                        transaction,
                        &run_model,
                        vec![NewAgentMessage {
                            role: AgentMessageRole::Tool,
                            content: json!({
                                "kind": "tool_result",
                                "content": serde_json::to_string(&tool_result)?,
                            }),
                            request_id: Some(tool_request_id),
                            tool_call_id: Some(provider_call_id.clone()),
                        }],
                        now,
                    )
                    .await?;
                    let bound_feedback_ids = if run_kind == "chapter_body" {
                        bind_pending_feedback_to_tool(
                            transaction,
                            run_id,
                            tool_call_id,
                            tool_sequence,
                            now,
                        )
                        .await?
                    } else {
                        Vec::new()
                    };
                    let request_count = run_model.request_count;
                    let mut active_run = run_model.into_active_model();
                    active_run.status = Set("queued".to_owned());
                    active_run.ended_at = Set(None);
                    active_run.updated_at = Set(now);
                    active_run.update(transaction).await?;
                    persist_workflow(transaction, &next_workflow).await?;
                    insert_event(
                        transaction,
                        version_id,
                        "question.answered",
                        question_id,
                        next_workflow.state_revision,
                        json!({
                            "run_id": run_id,
                            "tool_call_id": tool_call_id,
                            "answer_count": normalized_answers.len(),
                        }),
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        version_id,
                        "tool.resolved",
                        tool_call_id,
                        next_workflow.state_revision,
                        json!({
                            "run_id": run_id,
                            "chapter_id": next_workflow.active_chapter_id,
                            "name": "ask_user",
                            "provider_call_id": provider_call_id,
                            "status": "answered",
                        }),
                        now,
                    )
                    .await?;
                    for feedback_id in bound_feedback_ids {
                        insert_event(
                            transaction,
                            version_id,
                            "feedback.changed",
                            feedback_id,
                            next_workflow.state_revision,
                            json!({
                                "chapter_id": next_workflow.active_chapter_id,
                                "run_id": run_id,
                                "status": "bound",
                                "bound_tool_call_id": tool_call_id,
                            }),
                            now,
                        )
                        .await?;
                    }
                    insert_event(
                        transaction,
                        version_id,
                        "run.changed",
                        run_id,
                        next_workflow.state_revision,
                        json!({
                            "kind": run_kind.clone(),
                            "status": "queued",
                            "request_count": request_count.to_string(),
                        }),
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        version_id,
                        "workflow.changed",
                        version_id,
                        next_workflow.state_revision,
                        json!({
                            "stage": next_workflow.stage,
                            "execution": next_workflow.execution,
                            "run_id": run_id,
                            "chapter_id": next_workflow.active_chapter_id,
                            "pause_requested": next_workflow.pause_requested,
                        }),
                        now,
                    )
                    .await?;
                    let response = QuestionAnswerResult {
                        question_id,
                        run_id,
                        workflow: next_workflow,
                    };
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        ANSWER_QUESTION_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    /// 读取当前章节的正文、终稿审核状态以及两个阶段的纯文本折叠。
    pub async fn get_chapter_body(
        &self,
        chapter_id: ChapterId,
    ) -> Result<ChapterBodySnapshot, StoreError> {
        self.database
            .transaction::<_, _, StoreError>(|transaction| {
                Box::pin(async move { chapter_body_snapshot(transaction, chapter_id).await })
            })
            .await
            .map_err(transaction_error)
    }

    /// 读取一条不可变文档修订，用于引用回看与历史核对。
    ///
    /// 引用锚定在提交时的具体修订上，因此该修订即使已经不是文档当前修订，也必须
    /// 仍可读取；这里只做只读查询，不修改任何文档状态。
    ///
    /// # Errors
    ///
    /// 修订不存在、与 `document_id` 不匹配、或所属文档已删除时返回
    /// [`StoreError::DocumentRevisionNotFound`]。
    pub async fn get_document_revision(
        &self,
        document_id: DocumentId,
        revision_id: RevisionId,
    ) -> Result<DocumentRevisionView, StoreError> {
        let Some(revision) = document_revision::Entity::find_by_id(db_id(revision_id))
            .one(&self.database)
            .await?
        else {
            return Err(StoreError::DocumentRevisionNotFound);
        };
        if revision.document_id != db_id(document_id) {
            return Err(StoreError::DocumentRevisionNotFound);
        }
        let document = document::Entity::find_by_id(db_id(document_id))
            .filter(document::Entity::COLUMN.deleted_at.is_null())
            .one(&self.database)
            .await?
            .ok_or(StoreError::DocumentRevisionNotFound)?;
        let is_current = document
            .current_revision_id
            .as_deref()
            .and_then(|value| Uuid::parse_str(value).ok())
            == Some(revision_id);
        Ok(DocumentRevisionView {
            document_id,
            revision_id,
            revision_no: revision.revision_no,
            role: document.role,
            chapter_id: parse_optional_id(document.chapter_id, "document.chapter_id")?,
            word_count: revision.word_count,
            content: revision.content,
            is_current,
            created_at_ms: revision.created_at,
        })
    }

    /// 以自动接受模式追加当前章节正文。
    pub async fn append_chapter_body(
        &self,
        chapter_id: ChapterId,
        command_id: Uuid,
        expected_state_revision: i64,
        input: AppendChapterContent,
    ) -> Result<ChapterBodySnapshot, StoreError> {
        let target = format!("/api/chapters/{chapter_id}/body/append");
        let payload_hash = payload_hash(&AppendChapterBodyPayload {
            expected_state_revision,
            content: &input.content,
        })?;
        self.mutate_chapter_body(
            chapter_id,
            command_id,
            expected_state_revision,
            target,
            payload_hash,
            APPEND_CHAPTER_BODY_METHOD,
            ChapterBodyMutation::Append(input),
        )
        .await
    }

    /// 以自动接受模式精确替换当前章节正文的一段文本。
    pub async fn replace_chapter_body(
        &self,
        chapter_id: ChapterId,
        command_id: Uuid,
        expected_state_revision: i64,
        input: ReplaceChapterContent,
    ) -> Result<ChapterBodySnapshot, StoreError> {
        let target = format!("/api/chapters/{chapter_id}/body/replace");
        let payload_hash = payload_hash(&ReplaceChapterBodyPayload {
            expected_state_revision,
            old_str: &input.old_str,
            new_str: &input.new_str,
        })?;
        self.mutate_chapter_body(
            chapter_id,
            command_id,
            expected_state_revision,
            target,
            payload_hash,
            REPLACE_CHAPTER_BODY_METHOD,
            ChapterBodyMutation::Replace(input),
        )
        .await
    }

    /// 在同一事务中执行一条正文 Agent 工具调用。
    ///
    /// 这不是 HTTP 命令的包装：`request_id + provider_call_id` 是模型回调的幂等键。
    /// 一次成功的事务会同时完成模型请求、assistant/tool 消息、工具调用审计、可选
    /// 正文修订或待审提案、Run 状态、工作流状态和持久事件。自动接受或业务校验
    /// 失败后会为下一轮请求恢复排队；逐次确认模式的实际正文变更则停在待审提案。
    pub async fn execute_chapter_body_agent_tool_call(
        &self,
        request_id: Uuid,
        chapter_id: ChapterId,
        provider_request_id: Option<String>,
        usage: Value,
        call: ChapterBodyAgentToolCall,
    ) -> Result<AppliedChapterBodyToolCall, StoreError> {
        validate_agent_body_tool_call(&call)?;
        if !usage.is_object() {
            return Err(StoreError::InvalidModelRequest(
                "正文工具调用的模型 usage 必须是 JSON object".to_owned(),
            ));
        }
        if provider_request_id
            .as_deref()
            .is_some_and(|provider_request_id| provider_request_id.trim().is_empty())
        {
            return Err(StoreError::InvalidModelRequest(
                "provider_request_id 不能是空字符串".to_owned(),
            ));
        }

        let args_json = serde_json::to_string(&call.arguments)?;
        let now = utc_now_millis();
        let result = self
            .database
            .transaction::<_, _, StoreError>(|transaction| {
                let call = call.clone();
                let args_json = args_json.clone();
                let provider_request_id = provider_request_id.clone();
                let usage = usage.clone();
                Box::pin(async move {
                    let request = model_request::Entity::find_by_id(db_id(request_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::ModelRequestNotFound)?;

                    if let Some(existing) = tool_call::Entity::find()
                        .filter(tool_call::Entity::COLUMN.request_id.eq(request.id))
                        .filter(
                            tool_call::Entity::COLUMN
                                .provider_call_id
                                .eq(call.provider_call_id.clone()),
                        )
                        .one(transaction)
                        .await?
                    {
                        if existing.name != call.name || existing.args_json != args_json {
                            return Err(StoreError::InvalidModelRequestState(
                                "同一 provider tool call ID 的名称或参数发生变化".to_owned(),
                            ));
                        }
                        let result = if existing.status == "awaiting_review" {
                            let proposal = pending_chapter_body_proposal_from_tool_call(&existing)?;
                            awaiting_body_proposal_result(&proposal)
                        } else if existing.status == "held" {
                            let held = held_chapter_body_tool_payload_from_model(&existing)?;
                            held_body_tool_result(&held.held)
                        } else {
                            let result_json = existing.result_json.ok_or_else(|| {
                                StoreError::CorruptRecord(
                                    "已解析正文工具调用缺少 result_json".to_owned(),
                                )
                            })?;
                            let result: Value = serde_json::from_str(&result_json)?;
                            if !result.is_object() {
                                return Err(StoreError::CorruptRecord(
                                    "已解析正文工具调用的 result_json 必须是 JSON object"
                                        .to_owned(),
                                ));
                            }
                            result
                        };
                        return Ok((
                            AppliedChapterBodyToolCall {
                                provider_call_id: call.provider_call_id,
                                result,
                                newly_resolved: false,
                                requeue: false,
                            },
                            false,
                        ));
                    }

                    if request.status != "dispatched" {
                        return Err(StoreError::InvalidModelRequestState(format!(
                            "只有 dispatched 请求可以执行正文工具，当前为 {}",
                            request.status
                        )));
                    }
                    let run_model = run::Entity::find_by_id(request.run_id)
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    if run_model.parent_run_id.is_some() || run_model.kind != "chapter_body" {
                        return Err(StoreError::InvalidModelRequestState(
                            "该模型请求不属于顶层章节正文 Run".to_owned(),
                        ));
                    }
                    let run_id: RunId = run_model.id.into();
                    let version_id: VersionId = run_model.version_id.into();
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    if workflow.active_run_id != Some(run_id)
                        || workflow.active_chapter_id != Some(chapter_id)
                        || workflow.stage != WorkflowStage::WritingDrafting
                        || workflow.execution != ExecutionStatus::RequestInFlight
                    {
                        return Err(StoreError::InvalidModelRequestState(
                            "章节正文工作流已不接受此工具调用".to_owned(),
                        ));
                    }

                    let (chapter_model, body_document) = chapter_and_document(
                        transaction,
                        chapter_id,
                        CHAPTER_BODY_ROLE,
                        "章节正文",
                    )
                    .await?;
                    if chapter_model.status != "in_progress" {
                        return Err(StoreError::InvalidChapterState(
                            "章节尚未处于可编辑正文状态".to_owned(),
                        ));
                    }
                    let chapter_version_id: VersionId = chapter_model.version_id.into();
                    if chapter_version_id != version_id {
                        return Err(StoreError::CorruptRecord(
                            "正文工具调用章节与 Run 稿本不一致".to_owned(),
                        ));
                    }
                    chapter_body_run(transaction, run_id, chapter_id).await?;

                    let current_body = current_chapter_body(transaction, chapter_id).await?;
                    let base_revision_id = current_body.revision_id;
                    let base_revision_no = current_body.revision_no;
                    let current_content = current_body.content.unwrap_or_default();
                    let candidate = match parse_agent_body_mutation(&call) {
                        Ok(AgentChapterBodyMutation::Append(input)) => {
                            Ok(format!("{current_content}{}", input.content))
                        }
                        Ok(AgentChapterBodyMutation::Replace(input)) => replace_unique_chapter_content(
                            &current_content,
                            &input.old_str,
                            &input.new_str,
                        )
                        .map_err(agent_body_edit_error_result),
                        Err(error) => Err(error.into_result()),
                    };
                    let tool_call_id = Uuid::new_v4();
                    let sequence =
                        next_agent_tool_sequence(transaction, &run_model.id).await?;
                    if workflow.pause_requested {
                        if tool_call::Entity::find()
                            .filter(tool_call::Entity::COLUMN.run_id.eq(run_model.id))
                            .filter(tool_call::Entity::COLUMN.status.eq("held"))
                            .one(transaction)
                            .await?
                            .is_some()
                        {
                            return Err(StoreError::InvalidModelRequestState(
                                "当前正文 Run 已有挂起的工具调用".to_owned(),
                            ));
                        }
                        let (candidate_content, result_if_resumed) = match candidate {
                            Ok(next_content)
                                if next_content.chars().count()
                                    > nekowriter_core::MAX_ARTIFACT_CONTENT_LEN =>
                            {
                                (
                                    None,
                                    Some(agent_tool_error_result(
                                        "VALIDATION_ERROR",
                                        format!(
                                            "章节正文超过 {} 个字符",
                                            nekowriter_core::MAX_ARTIFACT_CONTENT_LEN
                                        ),
                                    )),
                                )
                            }
                            Ok(next_content) => (Some(next_content), None),
                            Err(error) => (None, Some(error)),
                        };
                        let candidate_hash = candidate_content
                            .as_deref()
                            .map(candidate_content_hash);
                        let held = HeldChapterBodyTool {
                            tool_call_id,
                            provider_call_id: call.provider_call_id.clone(),
                            tool_name: call.name.clone(),
                            base_revision_id,
                            base_revision_no,
                            candidate_content,
                            candidate_hash,
                        };
                        let held_payload = HeldChapterBodyToolPayload {
                            held: held.clone(),
                            result_if_resumed,
                        };
                        let next_workflow =
                            hold_chapter_body_tool_for_pause(&workflow, chapter_id, run_id)?;
                        append_agent_messages_for_run(
                            transaction,
                            &run_model,
                            vec![NewAgentMessage {
                                role: AgentMessageRole::Assistant,
                                content: json!({
                                    "kind": "assistant_tool_calls",
                                    "content": Value::Null,
                                    "toolCalls": [{
                                        "id": call.provider_call_id.clone(),
                                        "name": call.name.clone(),
                                        "arguments": call.arguments.clone(),
                                    }],
                                }),
                                request_id: Some(request_id),
                                tool_call_id: None,
                            }],
                            now,
                        )
                        .await?;
                        tool_call::ActiveModel {
                            id: Set(db_id(tool_call_id)),
                            request_id: Set(request.id),
                            run_id: Set(run_model.id),
                            provider_call_id: Set(call.provider_call_id.clone()),
                            sequence: Set(sequence),
                            name: Set(call.name.clone()),
                            args_json: Set(args_json),
                            base_revision_id: Set(base_revision_id.map(|id| id.to_string())),
                            proposal_json: Set(None),
                            held_json: Set(Some(serde_json::to_string(&held_payload)?)),
                            status: Set("held".to_owned()),
                            result_json: Set(None),
                            included_request_id: Set(None),
                            created_at: Set(now),
                            resolved_at: Set(None),
                            ..Default::default()
                        }
                        .insert(transaction)
                        .await?;
                        let mut active_request = request.into_active_model();
                        active_request.status = Set("succeeded".to_owned());
                        active_request.provider_request_id = Set(provider_request_id);
                        active_request.usage_json = Set(Some(serde_json::to_string(&usage)?));
                        active_request.error_json = Set(None);
                        active_request.updated_at = Set(now);
                        active_request.completed_at = Set(Some(now));
                        active_request.update(transaction).await?;
                        let request_count = run_model.request_count;
                        let mut active_run = run_model.into_active_model();
                        active_run.status = Set("paused".to_owned());
                        active_run.consecutive_protocol_errors = Set(0);
                        active_run.ended_at = Set(None);
                        active_run.updated_at = Set(now);
                        active_run.update(transaction).await?;
                        persist_workflow(transaction, &next_workflow).await?;
                        insert_event(
                            transaction,
                            version_id,
                            "run.changed",
                            run_id,
                            next_workflow.state_revision,
                            json!({
                                "chapter_id": chapter_id,
                                "kind": "chapter_body",
                                "status": "paused",
                                "request_id": request_id,
                                "request_count": request_count.to_string(),
                                "held_tool_call_id": tool_call_id,
                            }),
                            now,
                        )
                        .await?;
                        insert_event(
                            transaction,
                            version_id,
                            "workflow.changed",
                            version_id,
                            next_workflow.state_revision,
                            json!({
                                "stage": next_workflow.stage,
                                "execution": next_workflow.execution,
                                "chapter_id": chapter_id,
                                "run_id": run_id,
                                "request_id": request_id,
                            }),
                            now,
                        )
                        .await?;
                        return Ok((
                            AppliedChapterBodyToolCall {
                                provider_call_id: call.provider_call_id,
                                result: held_body_tool_result(&held),
                                newly_resolved: false,
                                requeue: false,
                            },
                            true,
                        ));
                    }
                    let mut proposal = None;
                    let (mut tool_result, committed_revision) = match candidate {
                        Ok(next_content)
                            if next_content.chars().count()
                                > nekowriter_core::MAX_ARTIFACT_CONTENT_LEN =>
                        {
                            (
                                agent_tool_error_result(
                                    "VALIDATION_ERROR",
                                    format!(
                                        "章节正文超过 {} 个字符",
                                        nekowriter_core::MAX_ARTIFACT_CONTENT_LEN
                                    ),
                                ),
                                None,
                            )
                        }
                        Ok(next_content) if workflow.approval_mode == ApprovalMode::ApproveRequired => {
                            if tool_call::Entity::find()
                                .filter(tool_call::Entity::COLUMN.run_id.eq(run_model.id))
                                .filter(
                                    tool_call::Entity::COLUMN
                                        .status
                                        .eq("awaiting_review"),
                                )
                                .one(transaction)
                                .await?
                                .is_some()
                            {
                                return Err(StoreError::InvalidModelRequestState(
                                    "当前正文 Run 已有待审编辑提案".to_owned(),
                                ));
                            }
                            let pending = PendingChapterBodyProposal {
                                tool_call_id,
                                provider_call_id: call.provider_call_id.clone(),
                                tool_name: call.name.clone(),
                                base_revision_id,
                                base_revision_no,
                                candidate_hash: candidate_content_hash(&next_content),
                                candidate_content: next_content,
                            };
                            let result = awaiting_body_proposal_result(&pending);
                            proposal = Some(pending);
                            (result, None)
                        }
                        Ok(next_content) => {
                            let dependency_revision_ids = chapter_body_revision_dependency_ids(
                                transaction,
                                version_id,
                                chapter_id,
                                base_revision_id,
                            )
                            .await?;
                            let (revision_id, revision_no) =
                                insert_document_revision_with_payload_and_source(
                                    transaction,
                                    body_document.id.into(),
                                    &next_content,
                                    None,
                                    "agent",
                                    base_revision_id,
                                    now,
                                )
                                .await?;
                            for dependency_revision_id in &dependency_revision_ids {
                                insert_revision_dependency(
                                    transaction,
                                    revision_id,
                                    *dependency_revision_id,
                                    "chapter_body_input",
                                )
                                .await?;
                            }
                            (
                                json!({
                                    "status": "applied",
                                    "tool_call_id": call.provider_call_id,
                                    "base_revision_id": base_revision_id.map(|revision_id| revision_id.to_string()),
                                    "result_revision_id": revision_id.to_string(),
                                    "data": {
                                        "revision_no": revision_no.to_string(),
                                        "word_count": non_whitespace_char_count(&next_content).to_string(),
                                    },
                                    "feedback": [],
                                }),
                                Some((revision_id, revision_no, next_content)),
                            )
                        }
                        Err(error) => (error, None),
                    };
                    let result_object = tool_result.as_object_mut().ok_or_else(|| {
                        StoreError::CorruptRecord("正文工具结果必须是 JSON object".to_owned())
                    })?;
                    result_object
                        .entry("tool_call_id".to_owned())
                        .or_insert_with(|| Value::String(call.provider_call_id.clone()));
                    result_object
                        .entry("base_revision_id".to_owned())
                        .or_insert_with(|| {
                            base_revision_id
                                .map_or(Value::Null, |revision_id| Value::String(revision_id.to_string()))
                        });
                    result_object
                        .entry("feedback".to_owned())
                        .or_insert_with(|| Value::Array(Vec::new()));
                    let awaiting_review = proposal.is_some();
                    if run_model.consecutive_protocol_errors < 0 {
                        return Err(StoreError::CorruptRecord(
                            "Run consecutive_protocol_errors 不能为负数".to_owned(),
                        ));
                    }
                    // `awaiting_review` 是一个已形成的业务边界；其余非 applied 工具
                    // 结果都没有推进正文，和非法 ask_user 一样必须受同一工程上限约束。
                    let is_protocol_error = !awaiting_review && tool_result["status"] != "applied";
                    let next_error_count = if is_protocol_error {
                        next_protocol_error_count(run_model.consecutive_protocol_errors)?
                    } else {
                        0
                    };
                    let stop = is_protocol_error
                        && next_error_count >= MAX_CONSECUTIVE_PROTOCOL_ERRORS;
                    let next_workflow = if awaiting_review {
                        hold_chapter_body_proposal(&workflow, chapter_id, run_id)?
                    } else if stop {
                        stop_writing_after_protocol_error(&workflow, run_id)?
                    } else {
                        continue_chapter_body_after_tool(&workflow, chapter_id, run_id)?
                    };
                    let assistant_message = NewAgentMessage {
                        role: AgentMessageRole::Assistant,
                        content: json!({
                            "kind": "assistant_tool_calls",
                            "content": Value::Null,
                            "toolCalls": [{
                                "id": call.provider_call_id.clone(),
                                "name": call.name.clone(),
                                "arguments": call.arguments.clone(),
                            }],
                        }),
                        request_id: Some(request_id),
                        tool_call_id: None,
                    };
                    let mut messages = vec![assistant_message];
                    if !awaiting_review {
                        let model_visible_result = serde_json::to_string(&tool_result)?;
                        messages.push(NewAgentMessage {
                            role: AgentMessageRole::Tool,
                            content: json!({
                                "kind": "tool_result",
                                "content": model_visible_result,
                            }),
                            request_id: Some(request_id),
                            tool_call_id: Some(call.provider_call_id.clone()),
                        });
                    }
                    append_agent_messages_for_run(
                        transaction,
                        &run_model,
                        messages,
                        now,
                    )
                    .await?;

                    tool_call::ActiveModel {
                        id: Set(db_id(tool_call_id)),
                        request_id: Set(request.id),
                        run_id: Set(run_model.id),
                        provider_call_id: Set(call.provider_call_id.clone()),
                        sequence: Set(sequence),
                        name: Set(call.name.clone()),
                        args_json: Set(args_json),
                        base_revision_id: Set(base_revision_id.map(|id| id.to_string())),
                        proposal_json: Set(
                            proposal
                                .as_ref()
                                .map(serde_json::to_string)
                                .transpose()?,
                        ),
                        held_json: Set(None),
                        status: Set(
                            if awaiting_review {
                                "awaiting_review"
                            } else if tool_result["status"] == "applied" {
                                "resolved"
                            } else {
                                "error"
                            }
                            .to_owned(),
                        ),
                        result_json: Set((!awaiting_review)
                            .then(|| serde_json::to_string(&tool_result))
                            .transpose()?),
                        included_request_id: Set(None),
                        created_at: Set(now),
                        resolved_at: Set((!awaiting_review).then_some(now)),
                        ..Default::default()
                    }
                    .insert(transaction)
                    .await?;

                    if awaiting_review {
                        let dependency_revision_ids = chapter_body_revision_dependency_ids(
                            transaction,
                            version_id,
                            chapter_id,
                            base_revision_id,
                        )
                        .await?;
                        insert_chapter_edit_review(
                            transaction,
                            version_id,
                            tool_call_id,
                            base_revision_id,
                            &dependency_revision_ids,
                            now,
                        )
                        .await?;
                    }

                    let bound_feedback_ids = if awaiting_review {
                        Vec::new()
                    } else {
                        bind_pending_feedback_to_tool(
                            transaction,
                            run_id,
                            tool_call_id,
                            sequence,
                            now,
                        )
                        .await?
                    };

                    let mut active_request = request.into_active_model();
                    active_request.status = Set("succeeded".to_owned());
                    active_request.provider_request_id = Set(provider_request_id);
                    active_request.usage_json = Set(Some(serde_json::to_string(&usage)?));
                    active_request.error_json = Set(None);
                    active_request.updated_at = Set(now);
                    active_request.completed_at = Set(Some(now));
                    active_request.update(transaction).await?;
                    let request_count = run_model.request_count;
                    let run_status = if awaiting_review {
                        "waiting"
                    } else if stop {
                        "stopped"
                    } else {
                        "queued"
                    };
                    let mut active_run = run_model.into_active_model();
                    active_run.status = Set(run_status.to_owned());
                    active_run.consecutive_protocol_errors = Set(next_error_count);
                    active_run.ended_at = Set(if stop { Some(now) } else { None });
                    active_run.updated_at = Set(now);
                    active_run.update(transaction).await?;
                    persist_workflow(transaction, &next_workflow).await?;

                    if let Some((revision_id, revision_no, content)) = committed_revision {
                        insert_event(
                            transaction,
                            version_id,
                            "document.committed",
                            revision_id,
                            revision_no,
                            json!({
                                "chapter_id": chapter_id,
                                "role": CHAPTER_BODY_ROLE,
                                "word_count": non_whitespace_char_count(&content),
                                "source": "agent_tool",
                            }),
                            now,
                        )
                        .await?;
                    }
                    let mut tool_event = json!({
                        "chapter_id": chapter_id,
                        "name": call.name.clone(),
                        "provider_call_id": call.provider_call_id.clone(),
                        "status": tool_result["status"],
                        "proposal_id": proposal.as_ref().map(|proposal| proposal.tool_call_id),
                    });
                    if is_protocol_error {
                        tool_event
                            .as_object_mut()
                            .expect("工具事件必须是 JSON object")
                            .insert(
                                "consecutive_protocol_errors".to_owned(),
                                Value::String(next_error_count.to_string()),
                            );
                    }
                    insert_event(
                        transaction,
                        version_id,
                        if awaiting_review {
                            "tool.proposed"
                        } else {
                            "tool.resolved"
                        },
                        tool_call_id,
                        next_workflow.state_revision,
                        tool_event,
                        now,
                    )
                    .await?;
                    for feedback_id in bound_feedback_ids {
                        insert_event(
                            transaction,
                            version_id,
                            "feedback.changed",
                            feedback_id,
                            next_workflow.state_revision,
                            json!({
                                "chapter_id": chapter_id,
                                "run_id": run_id,
                                "status": "bound",
                                "bound_tool_call_id": tool_call_id,
                            }),
                            now,
                        )
                        .await?;
                    }
                    let mut run_event = json!({
                        "chapter_id": chapter_id,
                        "kind": "chapter_body",
                        "status": run_status,
                        "request_id": request_id,
                        "request_count": request_count.to_string(),
                    });
                    if is_protocol_error {
                        let event = run_event
                            .as_object_mut()
                            .expect("Run 事件必须是 JSON object");
                        event.insert("reason".to_owned(), Value::String("tool_error".to_owned()));
                        event.insert(
                            "consecutive_protocol_errors".to_owned(),
                            Value::String(next_error_count.to_string()),
                        );
                        event.insert(
                            "wait_reason".to_owned(),
                            if stop {
                                Value::String("protocol_error_limit".to_owned())
                            } else {
                                Value::Null
                            },
                        );
                    }
                    insert_event(
                        transaction,
                        version_id,
                        "run.changed",
                        run_id,
                        next_workflow.state_revision,
                        run_event,
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        version_id,
                        "workflow.changed",
                        version_id,
                        next_workflow.state_revision,
                        json!({
                            "stage": next_workflow.stage,
                            "execution": next_workflow.execution,
                            "chapter_id": chapter_id,
                            "run_id": run_id,
                            "request_id": request_id,
                            "protocol_correction": is_protocol_error,
                            "consecutive_protocol_errors": next_error_count.to_string(),
                        }),
                        now,
                    )
                    .await?;
                    Ok((
                        AppliedChapterBodyToolCall {
                            provider_call_id: call.provider_call_id,
                            result: tool_result,
                            newly_resolved: !awaiting_review,
                            requeue: !awaiting_review && !stop,
                        },
                        true,
                    ))
                })
            })
            .await
            .map_err(transaction_error);
        match result {
            Ok((execution, changed)) => {
                if changed {
                    self.notify_event_listeners();
                }
                Ok(execution)
            }
            Err(error) => Err(error),
        }
    }

    /// 对一条待审正文提案作出决定。
    ///
    /// 空白意见表示接受，非空意见表示拒绝并作为下一轮工具结果反馈给 Agent。两种
    /// 决定都在同一事务里完成基础修订复核、工具结果、消息账本、Run/工作流重新
    /// 排队、事件和命令回执；因此同一 command ID 的网络重试不会重复创建正文修订。
    pub async fn decide_chapter_body_proposal(
        &self,
        chapter_id: ChapterId,
        tool_call_id: Uuid,
        command_id: Uuid,
        expected_state_revision: i64,
        feedback: String,
        citations: Vec<FeedbackCitation>,
    ) -> Result<ChapterBodySnapshot, StoreError> {
        if feedback.chars().count() > nekowriter_core::MAX_REVIEW_ISSUES_LEN {
            return Err(StoreError::InvalidChapterBodyEdit(format!(
                "编辑反馈超过 {} 个字符",
                nekowriter_core::MAX_REVIEW_ISSUES_LEN
            )));
        }
        let target = format!("/api/chapters/{chapter_id}/body/proposals/{tool_call_id}/decision");
        let payload_hash = payload_hash(&DecideChapterBodyProposalPayload {
            expected_state_revision,
            feedback: &feedback,
            citations: &citations,
        })?;
        let command = Command::new(
            command_id,
            DECIDE_CHAPTER_BODY_PROPOSAL_METHOD,
            &target,
            &payload_hash,
        );

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let now = utc_now_millis();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                let feedback = feedback.clone();
                let citations = citations.clone();
                Box::pin(async move {
                    let (chapter_model, body_document) = chapter_and_document(
                        transaction,
                        chapter_id,
                        CHAPTER_BODY_ROLE,
                        "章节正文",
                    )
                    .await?;
                    if chapter_model.status != "in_progress" {
                        return Err(StoreError::InvalidChapterState(
                            "章节尚未处于可编辑正文状态".to_owned(),
                        ));
                    }
                    let version_id: VersionId = chapter_model.version_id.into();
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    let run_id = workflow.active_run_id.ok_or_else(|| {
                        StoreError::InvalidChapterState("正文提案缺少活跃 Run".to_owned())
                    })?;
                    if workflow.active_chapter_id != Some(chapter_id)
                        || workflow.stage != WorkflowStage::WritingDrafting
                        || workflow.execution != ExecutionStatus::WaitingReview
                    {
                        return Err(StoreError::InvalidChapterState(
                            "当前工作流不在正文编辑提案审核状态".to_owned(),
                        ));
                    }
                    let run_model = chapter_body_run(transaction, run_id, chapter_id).await?;
                    if run_model.status != "waiting" {
                        return Err(StoreError::CorruptRecord(
                            "待审正文提案的 Run 未处于 waiting 状态".to_owned(),
                        ));
                    }
                    let proposal_call = tool_call::Entity::find_by_id(db_id(tool_call_id))
                        .filter(tool_call::Entity::COLUMN.run_id.eq(run_model.id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::ProposalNotFound)?;
                    if proposal_call.status != "awaiting_review" {
                        return Err(StoreError::ProposalAlreadyResolved);
                    }
                    let proposal = pending_chapter_body_proposal_from_tool_call(&proposal_call)?;
                    let chapter_edit_review = review::Entity::find()
                        .filter(review::Entity::COLUMN.version_id.eq(db_id(version_id)))
                        .filter(review::Entity::COLUMN.review_type.eq("chapter_edit"))
                        .filter(
                            review::Entity::COLUMN
                                .target_tool_call_id
                                .eq(tool_call_id.to_string()),
                        )
                        .filter(review::Entity::COLUMN.status.eq("open"))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::ReviewAlreadyResolved)?;
                    validate_chapter_body_feedback_citations(
                        transaction,
                        &citations,
                        version_id,
                        chapter_id,
                        run_id,
                        CandidateCitationScope::Proposal {
                            tool_call_id,
                        },
                    )
                    .await?;
                    let current_body = current_chapter_body(transaction, chapter_id).await?;
                    if current_body.revision_id != proposal.base_revision_id {
                        return Err(StoreError::ProposalBaseRevisionConflict {
                            expected: proposal.base_revision_id,
                            actual: current_body.revision_id,
                        });
                    }
                    let accepted = feedback.trim().is_empty() && citations.is_empty();
                    let (tool_result, committed_revision) = if accepted {
                        let dependency_revision_ids = chapter_body_revision_dependency_ids(
                            transaction,
                            version_id,
                            chapter_id,
                            proposal.base_revision_id,
                        )
                        .await?;
                        let (revision_id, revision_no) =
                            insert_document_revision_with_payload_and_source(
                                transaction,
                                body_document.id.into(),
                                &proposal.candidate_content,
                                None,
                                "agent",
                                proposal.base_revision_id,
                                now,
                            )
                            .await?;
                        for dependency_revision_id in &dependency_revision_ids {
                            insert_revision_dependency(
                                transaction,
                                revision_id,
                                *dependency_revision_id,
                                "chapter_body_input",
                            )
                            .await?;
                        }
                        resolve_pending_candidate_feedback_citations(
                            transaction,
                            run_id,
                            tool_call_id,
                            &proposal.candidate_content,
                            body_document.id.into(),
                            revision_id,
                            chapter_id,
                            now,
                        )
                        .await?;
                        (
                            json!({
                                "status": "applied",
                                "tool_call_id": proposal.provider_call_id,
                                "base_revision_id": proposal.base_revision_id.map(|revision_id| revision_id.to_string()),
                                "result_revision_id": revision_id.to_string(),
                                "data": {
                                    "word_count": non_whitespace_char_count(&proposal.candidate_content).to_string(),
                                },
                                "feedback": [],
                            }),
                            Some((revision_id, revision_no)),
                        )
                    } else {
                        (
                            json!({
                                "status": "rejected",
                                "tool_call_id": proposal.provider_call_id,
                                "base_revision_id": proposal.base_revision_id.map(|revision_id| revision_id.to_string()),
                                "feedback": [{
                                    "text": feedback.clone(),
                                    "citations": citations.clone(),
                                }],
                            }),
                            None,
                        )
                    };
                    let request_id: Uuid = proposal_call.request_id.into();
                    let proposal_sequence = proposal_call.sequence;
                    let mut active_call = proposal_call.into_active_model();
                    // 调用生命周期到这里都已完成；“rejected”是返回给模型的业务
                    // 结果，而不是把 ToolCall 留在另一种未完成的数据库状态。
                    active_call.status = Set("resolved".to_owned());
                    active_call.result_json = Set(Some(serde_json::to_string(&tool_result)?));
                    active_call.resolved_at = Set(Some(now));
                    active_call.update(transaction).await?;
                    let bound_feedback_ids = bind_pending_feedback_to_tool(
                        transaction,
                        run_id,
                        tool_call_id,
                        proposal_sequence,
                        now,
                    )
                    .await?;
                    append_agent_messages_for_run(
                        transaction,
                        &run_model,
                        vec![NewAgentMessage {
                            role: AgentMessageRole::Tool,
                            content: json!({
                                "kind": "tool_result",
                                "content": serde_json::to_string(&tool_result)?,
                            }),
                            request_id: Some(request_id),
                            tool_call_id: Some(proposal.provider_call_id.clone()),
                        }],
                        now,
                    )
                    .await?;
                    let next_workflow =
                        continue_chapter_body_after_proposal(&workflow, chapter_id, run_id)?;
                    update_chapter_body_run_status(
                        transaction,
                        run_id,
                        chapter_id,
                        "queued",
                        None,
                        None,
                        now,
                    )
                    .await?;
                    persist_workflow(transaction, &next_workflow).await?;
                    let chapter_edit_review_id: ReviewId = chapter_edit_review.id.into();
                    let mut active_review = chapter_edit_review.into_active_model();
                    active_review.status = Set(
                        if accepted { "approved" } else { "rejected" }.to_owned(),
                    );
                    active_review.decision_source = Set(Some("human".to_owned()));
                    active_review.passed = Set(Some(accepted));
                    active_review.confidence = Set(None);
                    active_review.issues_json = Set(
                        (!feedback.is_empty())
                            .then(|| serde_json::to_string(&feedback))
                            .transpose()?,
                    );
                    active_review.user_input_json = Set(Some(serde_json::to_string(&json!({
                        "source": "user",
                        "feedback": feedback,
                        "citations": citations,
                    }))?));
                    active_review.updated_at = Set(now);
                    active_review.update(transaction).await?;
                    if let Some((revision_id, revision_no)) = committed_revision {
                        insert_event(
                            transaction,
                            version_id,
                            "document.committed",
                            revision_id,
                            revision_no,
                            json!({
                                "chapter_id": chapter_id,
                                "role": CHAPTER_BODY_ROLE,
                                "word_count": non_whitespace_char_count(&proposal.candidate_content),
                                "source": "agent_proposal",
                            }),
                            now,
                        )
                        .await?;
                    }
                    insert_event(
                        transaction,
                        version_id,
                        "tool.resolved",
                        tool_call_id,
                        next_workflow.state_revision,
                        json!({
                            "chapter_id": chapter_id,
                            "name": proposal.tool_name,
                            "provider_call_id": proposal.provider_call_id,
                            "status": tool_result["status"],
                        }),
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        version_id,
                        "review.changed",
                        chapter_edit_review_id,
                        next_workflow.state_revision,
                        json!({
                            "status": if accepted { "approved" } else { "rejected" },
                            "source": "human",
                            "tool_call_id": tool_call_id,
                        }),
                        now,
                    )
                    .await?;
                    for feedback_id in bound_feedback_ids {
                        insert_event(
                            transaction,
                            version_id,
                            "feedback.changed",
                            feedback_id,
                            next_workflow.state_revision,
                            json!({
                                "chapter_id": chapter_id,
                                "run_id": run_id,
                                "status": "bound",
                                "bound_tool_call_id": tool_call_id,
                            }),
                            now,
                        )
                        .await?;
                    }
                    insert_event(
                        transaction,
                        version_id,
                        "run.changed",
                        run_id,
                        next_workflow.state_revision,
                        json!({
                            "chapter_id": chapter_id,
                            "kind": "chapter_body",
                            "status": "queued",
                        }),
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        version_id,
                        "workflow.changed",
                        version_id,
                        next_workflow.state_revision,
                        json!({
                            "stage": next_workflow.stage,
                            "execution": next_workflow.execution,
                            "chapter_id": chapter_id,
                            "run_id": run_id,
                        }),
                        now,
                    )
                    .await?;
                    let response = chapter_body_snapshot(transaction, chapter_id).await?;
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        DECIDE_CHAPTER_BODY_PROPOSAL_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    /// 将正文 Agent 的阶段提交、模型请求终态和整章终稿审核原子落库。
    ///
    /// 正文内容不从 `summary` 读取；它只能来自此前已经解析并执行的正文工具调用。
    /// 因此模型无法通过提交工具绕过正文修订、依赖或整章审核门槛。
    pub async fn commit_chapter_body_model_submission(
        &self,
        request_id: Uuid,
        chapter_id: ChapterId,
        provider_request_id: Option<String>,
        usage: Value,
        assistant_message: NewAgentMessage,
        summary: String,
    ) -> Result<ChapterBodySnapshot, StoreError> {
        // 提交是一次工具调用：成功路径写配对的应用结果，被拒路径写配对的错误结果。
        let submission_messages = phase_submission_messages(request_id, assistant_message.clone())?;
        if !usage.is_object() {
            return Err(StoreError::InvalidModelRequest(
                "章节正文模型响应的 usage 必须是 JSON object".to_owned(),
            ));
        }
        if provider_request_id
            .as_deref()
            .is_some_and(|provider_request_id| provider_request_id.trim().is_empty())
        {
            return Err(StoreError::InvalidModelRequest(
                "provider_request_id 不能是空字符串".to_owned(),
            ));
        }
        nekowriter_core::validate_document_content(
            "章节摘要",
            &summary,
            nekowriter_core::MAX_ARTIFACT_CONTENT_LEN,
        )
        .map_err(|error| StoreError::InvalidModelRequest(error.to_string()))?;

        let now = utc_now_millis();
        let result = self
            .database
            .transaction::<_, _, StoreError>(|transaction| {
                let provider_request_id = provider_request_id.clone();
                let usage = usage.clone();
                let assistant_message = assistant_message.clone();
                let summary = summary.clone();
                Box::pin(async move {
                    let request = model_request::Entity::find_by_id(db_id(request_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::ModelRequestNotFound)?;
                    let run_model = run::Entity::find_by_id(request.run_id)
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    if run_model.parent_run_id.is_some() || run_model.kind != "chapter_body" {
                        return Err(StoreError::InvalidModelRequestState(
                            "该模型请求不属于顶层章节正文 Run".to_owned(),
                        ));
                    }
                    let run_id: RunId = run_model.id.into();
                    let version_id: VersionId = run_model.version_id.into();
                    let workflow = workflow_for_version(transaction, version_id).await?;

                    if request.status == "succeeded" {
                        if workflow.active_run_id == Some(run_id)
                            && workflow.active_chapter_id == Some(chapter_id)
                            && ((workflow.stage == WorkflowStage::WritingChapterReviewing
                                && workflow.execution == ExecutionStatus::WaitingReview)
                                || (workflow.stage == WorkflowStage::WritingDrafting
                                    && workflow.execution == ExecutionStatus::Paused
                                    && workflow.pause_requested))
                        {
                            return Ok((
                                chapter_body_snapshot(transaction, chapter_id).await?,
                                false,
                            ));
                        }
                        return Err(StoreError::InvalidModelRequestState(
                            "已成功的正文请求不对应当前整章审核状态".to_owned(),
                        ));
                    }
                    if request.status != "dispatched" {
                        return Err(StoreError::InvalidModelRequestState(format!(
                            "只有 dispatched 请求可以提交章节正文，当前为 {}",
                            request.status
                        )));
                    }
                    if workflow.active_run_id != Some(run_id)
                        || workflow.active_chapter_id != Some(chapter_id)
                        || workflow.stage != WorkflowStage::WritingDrafting
                        || workflow.execution != ExecutionStatus::RequestInFlight
                    {
                        return Err(StoreError::InvalidModelRequestState(
                            "章节正文工作流已不接受此模型提交".to_owned(),
                        ));
                    }

                    let (chapter_model, _) = chapter_and_document(
                        transaction,
                        chapter_id,
                        CHAPTER_BODY_ROLE,
                        "章节正文",
                    )
                    .await?;
                    let chapter_version_id: VersionId = chapter_model.version_id.into();
                    if chapter_model.status != "in_progress" || chapter_version_id != version_id {
                        return Err(StoreError::InvalidChapterState(
                            "当前章节不属于可提交正文的活跃稿本".to_owned(),
                        ));
                    }
                    let body_run = chapter_body_run(transaction, run_id, chapter_id).await?;
                    if workflow.pause_requested {
                        let mut checkpoint: RunCheckpoint =
                            serde_json::from_str(&body_run.checkpoint_json)?;
                        if checkpoint.chapter_id != Some(chapter_id)
                            || checkpoint.held_chapter_body_submission.is_some()
                        {
                            return Err(StoreError::CorruptRecord(
                                "正文暂停提交的 Run 检查点无效".to_owned(),
                            ));
                        }
                        let next_workflow =
                            hold_chapter_body_submission_for_pause(&workflow, chapter_id, run_id)?;
                        checkpoint.held_chapter_body_submission =
                            Some(HeldChapterBodySubmissionCheckpoint {
                                held: HeldChapterBodySubmission {
                                    request_id,
                                    summary: summary.clone(),
                                },
                                assistant_message: assistant_message.clone(),
                            });
                        let mut active_request = request.into_active_model();
                        active_request.status = Set("succeeded".to_owned());
                        active_request.provider_request_id = Set(provider_request_id);
                        active_request.usage_json = Set(Some(serde_json::to_string(&usage)?));
                        active_request.error_json = Set(None);
                        active_request.updated_at = Set(now);
                        active_request.completed_at = Set(Some(now));
                        active_request.update(transaction).await?;
                        let request_count = body_run.request_count;
                        let mut active_run = body_run.into_active_model();
                        active_run.status = Set("paused".to_owned());
                        active_run.consecutive_protocol_errors = Set(0);
                        active_run.checkpoint_json = Set(serde_json::to_string(&checkpoint)?);
                        active_run.ended_at = Set(None);
                        active_run.updated_at = Set(now);
                        active_run.update(transaction).await?;
                        persist_workflow(transaction, &next_workflow).await?;
                        insert_event(
                            transaction,
                            version_id,
                            "run.changed",
                            run_id,
                            next_workflow.state_revision,
                            json!({
                                "chapter_id": chapter_id,
                                "kind": "chapter_body",
                                "status": "paused",
                                "request_id": request_id,
                                "request_count": request_count.to_string(),
                                "held_submission": true,
                            }),
                            now,
                        )
                        .await?;
                        insert_event(
                            transaction,
                            version_id,
                            "workflow.changed",
                            version_id,
                            next_workflow.state_revision,
                            json!({
                                "stage": next_workflow.stage,
                                "execution": next_workflow.execution,
                                "chapter_id": chapter_id,
                                "run_id": run_id,
                                "request_id": request_id,
                                "pause_requested": true,
                            }),
                            now,
                        )
                        .await?;
                        return Ok((chapter_body_snapshot(transaction, chapter_id).await?, true));
                    }
                    let has_pending_feedback = feedback::Entity::find()
                        .filter(feedback::Entity::COLUMN.run_id.eq(body_run.id))
                        .filter(feedback::Entity::COLUMN.status.is_in(["queued", "bound"]))
                        .one(transaction)
                        .await?
                        .is_some();
                    if has_pending_feedback {
                        // 这是业务守卫的拒绝，不是协议错误：§36.1 已把四条阶段提交路径统一成
                        // 「被拒回合」——立即重排、不推进 `consecutive_protocol_errors`、永不触发
                        // 上限，非进展回合的上限由请求额度承担。反例现场（2026-09-15 第二章正文
                        // Run）：两个纯文本回合已占 2/3 计数，这次守卫拒绝把计数推到 3，Run 当场
                        // 停下，模型刚收到的「请用 append_content / replace_content 投递意见」再没有
                        // 执行机会，未投递意见永远留在队列里。
                        let next_workflow = continue_chapter_body_after_protocol_correction(
                            &workflow, chapter_id, run_id,
                        )?;
                        let mut messages = vec![assistant_message.clone()];
                        messages.extend(protocol_error_tool_result_messages(
                            request_id,
                            &assistant_message,
                            CHAPTER_BODY_FEEDBACK_PROTOCOL_CORRECTION,
                            "PHASE_SUBMISSION_REJECTED",
                        )?);
                        messages.push(NewAgentMessage {
                            role: AgentMessageRole::User,
                            content: json!({
                                "kind": "protocol_correction",
                                "content": CHAPTER_BODY_FEEDBACK_PROTOCOL_CORRECTION,
                            }),
                            request_id: Some(request_id),
                            tool_call_id: None,
                        });
                        append_agent_messages_for_run(transaction, &body_run, messages, now)
                            .await?;
                        let mut active_request = request.into_active_model();
                        active_request.status = Set("succeeded".to_owned());
                        active_request.provider_request_id = Set(provider_request_id);
                        active_request.usage_json = Set(Some(serde_json::to_string(&usage)?));
                        active_request.error_json = Set(None);
                        active_request.updated_at = Set(now);
                        active_request.completed_at = Set(Some(now));
                        active_request.update(transaction).await?;
                        let request_count = body_run.request_count;
                        let mut active_run = body_run.into_active_model();
                        active_run.status = Set("queued".to_owned());
                        active_run.ended_at = Set(None);
                        active_run.updated_at = Set(now);
                        active_run.update(transaction).await?;
                        persist_workflow(transaction, &next_workflow).await?;
                        insert_event(
                            transaction,
                            version_id,
                            "run.changed",
                            run_id,
                            next_workflow.state_revision,
                            json!({
                                "chapter_id": chapter_id,
                                "kind": "chapter_body",
                                "status": "queued",
                                "request_id": request_id,
                                "request_count": request_count.to_string(),
                                "reason": "pending_feedback_requires_tool_result",
                            }),
                            now,
                        )
                        .await?;
                        insert_event(
                            transaction,
                            version_id,
                            "workflow.changed",
                            version_id,
                            next_workflow.state_revision,
                            json!({
                                "stage": next_workflow.stage,
                                "execution": next_workflow.execution,
                                "chapter_id": chapter_id,
                                "run_id": run_id,
                                "request_id": request_id,
                            }),
                            now,
                        )
                        .await?;
                        return Ok((chapter_body_snapshot(transaction, chapter_id).await?, true));
                    }
                    let body = current_chapter_body(transaction, chapter_id).await?;
                    let body_revision_id = body.revision_id.ok_or_else(|| {
                        StoreError::InvalidChapterBodyEdit("章节正文不能为空".to_owned())
                    })?;
                    let body_content = body.content.ok_or_else(|| {
                        StoreError::CorruptRecord("章节正文当前修订缺少内容".to_owned())
                    })?;
                    nekowriter_core::validate_document_content(
                        "章节正文",
                        &body_content,
                        nekowriter_core::MAX_ARTIFACT_CONTENT_LEN,
                    )
                    .map_err(|error| StoreError::InvalidChapterBodyEdit(error.to_string()))?;
                    let outline_revision_id = chapter_body_outline_dependency_revision_id(
                        transaction,
                        version_id,
                        chapter_id,
                    )
                    .await?;
                    let dependency_revision_ids =
                        revision_dependency_ids(transaction, body_revision_id).await?;
                    if !dependency_revision_ids.contains(&outline_revision_id) {
                        return Err(StoreError::InvalidChapterState(
                            "当前正文未依赖已通过的章节细纲".to_owned(),
                        ));
                    }

                    let next_workflow = submit_chapter_for_review(&workflow, chapter_id)?;
                    let review_id = insert_chapter_final_review(
                        transaction,
                        version_id,
                        body_revision_id,
                        &dependency_revision_ids,
                        now,
                    )
                    .await?;
                    append_agent_messages_for_run(
                        transaction,
                        &run_model,
                        submission_messages.clone(),
                        now,
                    )
                    .await?;
                    let mut active_request = request.into_active_model();
                    active_request.status = Set("succeeded".to_owned());
                    active_request.provider_request_id = Set(provider_request_id);
                    active_request.usage_json = Set(Some(serde_json::to_string(&usage)?));
                    active_request.error_json = Set(None);
                    active_request.updated_at = Set(now);
                    active_request.completed_at = Set(Some(now));
                    active_request.update(transaction).await?;
                    let request_count = run_model.request_count;
                    let mut active_run = run_model.into_active_model();
                    active_run.status = Set("waiting".to_owned());
                    active_run.consecutive_protocol_errors = Set(0);
                    active_run.updated_at = Set(now);
                    active_run.update(transaction).await?;
                    persist_workflow(transaction, &next_workflow).await?;
                    insert_event(
                        transaction,
                        version_id,
                        "review.changed",
                        review_id,
                        next_workflow.state_revision,
                        json!({
                            "chapter_id": chapter_id,
                            "role": CHAPTER_BODY_ROLE,
                            "status": "open",
                            "source": "agent_submission",
                        }),
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        version_id,
                        "run.changed",
                        run_id,
                        next_workflow.state_revision,
                        json!({
                            "chapter_id": chapter_id,
                            "kind": "chapter_body",
                            "status": "waiting",
                            "request_id": request_id,
                            "request_count": request_count.to_string(),
                        }),
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        version_id,
                        "workflow.changed",
                        version_id,
                        next_workflow.state_revision,
                        json!({
                            "stage": next_workflow.stage,
                            "execution": next_workflow.execution,
                            "chapter_id": chapter_id,
                            "run_id": run_id,
                            "request_id": request_id,
                        }),
                        now,
                    )
                    .await?;
                    Ok((chapter_body_snapshot(transaction, chapter_id).await?, true))
                })
            })
            .await
            .map_err(transaction_error);
        match result {
            Ok((snapshot, changed)) => {
                if changed {
                    self.notify_event_listeners();
                }
                Ok(snapshot)
            }
            Err(error) => Err(error),
        }
    }

    async fn mutate_chapter_body(
        &self,
        chapter_id: ChapterId,
        command_id: Uuid,
        expected_state_revision: i64,
        target: String,
        payload_hash: String,
        method: &'static str,
        mutation: ChapterBodyMutation,
    ) -> Result<ChapterBodySnapshot, StoreError> {
        let command = Command::new(command_id, method, &target, &payload_hash);

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let (chapter_model, body_document) = chapter_and_document(
                        transaction,
                        chapter_id,
                        CHAPTER_BODY_ROLE,
                        "章节正文",
                    )
                    .await?;
                    if chapter_model.status != "in_progress" {
                        return Err(StoreError::InvalidChapterState(
                            "章节尚未处于可编辑正文状态".to_owned(),
                        ));
                    }
                    let version_id: VersionId = chapter_model.version_id.into();
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    if workflow.approval_mode != ApprovalMode::AutoApprove {
                        return Err(StoreError::InvalidChapterState(
                            "逐次确认模式下不能绕过待审提案直接修改正文".to_owned(),
                        ));
                    }
                    if workflow.execution == ExecutionStatus::WaitingReview {
                        return Err(StoreError::InvalidChapterState(
                            "当前正文仍有待决定的编辑提案".to_owned(),
                        ));
                    }
                    if workflow.pause_requested || workflow.execution == ExecutionStatus::Paused {
                        return Err(StoreError::InvalidChapterState(
                            "暂停请求尚未完成，不能绕过挂起工具直接修改正文".to_owned(),
                        ));
                    }
                    // 即使候选文本不变，也必须先确认当前章节、阶段和 Run 没有被其他
                    // 命令替换；但 NO_CHANGE 不写修订、不递增工作流版本。
                    let _ = record_chapter_body_edit(&workflow, chapter_id)?;
                    let run_id = workflow.active_run_id.ok_or_else(|| {
                        StoreError::CorruptRecord("正文编辑缺少对应的活跃 Run".to_owned())
                    })?;
                    chapter_body_run(transaction, run_id, chapter_id).await?;
                    let current_body = current_chapter_body(transaction, chapter_id).await?;
                    let base_revision_id = current_body.revision_id;
                    let current_content = current_body.content.unwrap_or_default();
                    let next_content = match mutation {
                        ChapterBodyMutation::Append(input) => {
                            Some(format!("{current_content}{}", input.content))
                        }
                        ChapterBodyMutation::Replace(input) => {
                            match replace_unique_chapter_content(
                                &current_content,
                                &input.old_str,
                                &input.new_str,
                            ) {
                                Ok(content) => Some(content),
                                Err(ChapterBodyEditError::NoChange) => None,
                                Err(error) => return Err(StoreError::ChapterBodyEdit(error)),
                            }
                        }
                    };
                    let now = utc_now_millis();
                    if let Some(next_content) = next_content {
                        if next_content.chars().count() > nekowriter_core::MAX_ARTIFACT_CONTENT_LEN
                        {
                            return Err(StoreError::InvalidChapterBodyEdit(format!(
                                "章节正文超过 {} 个字符",
                                nekowriter_core::MAX_ARTIFACT_CONTENT_LEN
                            )));
                        }
                        let dependency_revision_ids = chapter_body_revision_dependency_ids(
                            transaction,
                            version_id,
                            chapter_id,
                            base_revision_id,
                        )
                        .await?;
                        let (revision_id, revision_no) =
                            insert_document_revision_with_payload_and_source(
                                transaction,
                                body_document.id.into(),
                                &next_content,
                                None,
                                "user",
                                base_revision_id,
                                now,
                            )
                            .await?;
                        for dependency_revision_id in &dependency_revision_ids {
                            insert_revision_dependency(
                                transaction,
                                revision_id,
                                *dependency_revision_id,
                                "chapter_body_input",
                            )
                            .await?;
                        }
                        let next_workflow = record_chapter_body_edit(&workflow, chapter_id)?;
                        // 执行轴未变时说明模型回合仍在进行（排队中或在途），Run 的状态
                        // 必须跟着那个回合走，不能改成 waiting 而把回调路径废掉。
                        if next_workflow.execution != workflow.execution {
                            update_chapter_body_run_status(
                                transaction,
                                run_id,
                                chapter_id,
                                "waiting",
                                None,
                                None,
                                now,
                            )
                            .await?;
                        }
                        persist_workflow(transaction, &next_workflow).await?;
                        insert_event(
                            transaction,
                            version_id,
                            "document.committed",
                            revision_id,
                            revision_no,
                            json!({
                                "chapter_id": chapter_id,
                                "role": CHAPTER_BODY_ROLE,
                                "word_count": non_whitespace_char_count(&next_content),
                            }),
                            now,
                        )
                        .await?;
                        insert_event(
                            transaction,
                            version_id,
                            "run.changed",
                            run_id,
                            next_workflow.state_revision,
                            json!({
                                "chapter_id": chapter_id,
                                "kind": "chapter_body",
                                "status": "waiting",
                            }),
                            now,
                        )
                        .await?;
                        insert_event(
                            transaction,
                            version_id,
                            "workflow.changed",
                            version_id,
                            next_workflow.state_revision,
                            json!({
                                "stage": next_workflow.stage,
                                "chapter_id": chapter_id,
                                "run_id": run_id,
                            }),
                            now,
                        )
                        .await?;
                    }
                    let response = chapter_body_snapshot(transaction, chapter_id).await?;
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        method,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    /// 提交当前章节正文至整章终稿审核。
    pub async fn submit_chapter_final_review(
        &self,
        chapter_id: ChapterId,
        command_id: Uuid,
        expected_state_revision: i64,
    ) -> Result<ChapterBodySnapshot, StoreError> {
        let target = format!("/api/chapters/{chapter_id}/body/review");
        let payload_hash = payload_hash(&SubmitChapterFinalReviewPayload {
            expected_state_revision,
        })?;
        let command = Command::new(
            command_id,
            SUBMIT_CHAPTER_FINAL_REVIEW_METHOD,
            &target,
            &payload_hash,
        );

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let (chapter_model, _) = chapter_and_document(
                        transaction,
                        chapter_id,
                        CHAPTER_BODY_ROLE,
                        "章节正文",
                    )
                    .await?;
                    if chapter_model.status != "in_progress" {
                        return Err(StoreError::InvalidChapterState(
                            "章节尚未处于可提交终稿状态".to_owned(),
                        ));
                    }
                    let version_id: VersionId = chapter_model.version_id.into();
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    if workflow.stage == WorkflowStage::WritingDrafting
                        && workflow.execution == ExecutionStatus::WaitingReview
                    {
                        return Err(StoreError::InvalidChapterState(
                            "当前正文仍有待决定的编辑提案，不能提交整章审核".to_owned(),
                        ));
                    }
                    if workflow.pause_requested || workflow.execution == ExecutionStatus::Paused {
                        return Err(StoreError::InvalidChapterState(
                            "暂停请求尚未在工具边界完成，不能提交整章审核".to_owned(),
                        ));
                    }
                    let next_workflow = submit_chapter_for_review(&workflow, chapter_id)?;
                    let run_id = workflow.active_run_id.ok_or_else(|| {
                        StoreError::CorruptRecord("整章审核缺少对应的活跃 Run".to_owned())
                    })?;
                    chapter_body_run(transaction, run_id, chapter_id).await?;
                    let body = current_chapter_body(transaction, chapter_id).await?;
                    let body_revision_id = body.revision_id.ok_or_else(|| {
                        StoreError::InvalidChapterBodyEdit("章节正文不能为空".to_owned())
                    })?;
                    let body_content = body.content.ok_or_else(|| {
                        StoreError::CorruptRecord("章节正文当前修订缺少内容".to_owned())
                    })?;
                    nekowriter_core::validate_document_content(
                        "章节正文",
                        &body_content,
                        nekowriter_core::MAX_ARTIFACT_CONTENT_LEN,
                    )
                    .map_err(|error| StoreError::InvalidChapterBodyEdit(error.to_string()))?;
                    let outline_revision_id = chapter_body_outline_dependency_revision_id(
                        transaction,
                        version_id,
                        chapter_id,
                    )
                    .await?;
                    let dependency_revision_ids =
                        revision_dependency_ids(transaction, body_revision_id).await?;
                    if !dependency_revision_ids.contains(&outline_revision_id) {
                        return Err(StoreError::InvalidChapterState(
                            "当前正文未依赖已通过的章节细纲".to_owned(),
                        ));
                    }
                    let now = utc_now_millis();
                    let review_id = insert_chapter_final_review(
                        transaction,
                        version_id,
                        body_revision_id,
                        &dependency_revision_ids,
                        now,
                    )
                    .await?;
                    update_chapter_body_run_status(
                        transaction,
                        run_id,
                        chapter_id,
                        "waiting",
                        None,
                        None,
                        now,
                    )
                    .await?;
                    persist_workflow(transaction, &next_workflow).await?;
                    insert_event(
                        transaction,
                        version_id,
                        "review.changed",
                        review_id,
                        next_workflow.state_revision,
                        json!({
                            "chapter_id": chapter_id,
                            "role": CHAPTER_BODY_ROLE,
                            "status": "open",
                        }),
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        version_id,
                        "run.changed",
                        run_id,
                        next_workflow.state_revision,
                        json!({
                            "chapter_id": chapter_id,
                            "kind": "chapter_body",
                            "status": "waiting",
                        }),
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        version_id,
                        "workflow.changed",
                        version_id,
                        next_workflow.state_revision,
                        json!({
                            "stage": next_workflow.stage,
                            "chapter_id": chapter_id,
                            "run_id": run_id,
                        }),
                        now,
                    )
                    .await?;
                    let response = chapter_body_snapshot(transaction, chapter_id).await?;
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        SUBMIT_CHAPTER_FINAL_REVIEW_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    /// 对当前章节整章终稿审核作出人工决定。
    pub async fn decide_chapter_final_review(
        &self,
        review_id: ReviewId,
        command_id: Uuid,
        expected_state_revision: i64,
        decision: HumanReviewDecision,
        citations: Vec<FeedbackCitation>,
    ) -> Result<PreparationSnapshot, StoreError> {
        let target = format!("/api/reviews/{review_id}/decision");
        let payload_hash = payload_hash(&DecidePreparationReviewPayload {
            expected_state_revision,
            decision: &decision,
            citations: &citations,
        })?;
        let command = Command::new(
            command_id,
            DECIDE_CHAPTER_FINAL_REVIEW_METHOD,
            &target,
            &payload_hash,
        );

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let review_target = chapter_final_review_target(transaction, review_id).await?;
                    if review_target.status != ReviewStatus::Open {
                        return Err(StoreError::ReviewAlreadyResolved);
                    }
                    let workflow =
                        workflow_for_version(transaction, review_target.version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    if workflow.stage != WorkflowStage::WritingChapterReviewing {
                        return Err(StoreError::InvalidChapterState(
                            "只有整章审核中状态可以作出终稿审核决定".to_owned(),
                        ));
                    }
                    if workflow.active_chapter_id != Some(review_target.chapter_id) {
                        return Err(StoreError::InvalidChapterState(
                            "整章审核目标不是当前章节".to_owned(),
                        ));
                    }
                    if review_target.current_revision_id != review_target.target_revision_id {
                        return Err(StoreError::InvalidChapterState(
                            "审核目标已不是当前正文修订，不能对旧修订作出决定".to_owned(),
                        ));
                    }
                    let body = current_chapter_body(transaction, review_target.chapter_id).await?;
                    let body_content = body.content.ok_or_else(|| {
                        StoreError::CorruptRecord("当前章节正文缺少内容".to_owned())
                    })?;
                    nekowriter_core::validate_document_content(
                        "章节正文",
                        &body_content,
                        nekowriter_core::MAX_ARTIFACT_CONTENT_LEN,
                    )
                    .map_err(|error| StoreError::InvalidChapterBodyEdit(error.to_string()))?;
                    let outline_revision_id = chapter_body_outline_dependency_revision_id(
                        transaction,
                        review_target.version_id,
                        review_target.chapter_id,
                    )
                    .await?;
                    let actual_dependencies =
                        revision_dependency_ids(transaction, review_target.target_revision_id)
                            .await?;
                    if review_target.dependency_revision_ids != actual_dependencies
                        || !actual_dependencies.contains(&outline_revision_id)
                    {
                        return Err(StoreError::InvalidChapterState(
                            "整章审核依赖已变化，不能对旧输入作出决定".to_owned(),
                        ));
                    }
                    let run_id = workflow.active_run_id.ok_or_else(|| {
                        StoreError::CorruptRecord("整章审核缺少对应的活跃 Run".to_owned())
                    })?;
                    let body_run =
                        chapter_body_run(transaction, run_id, review_target.chapter_id).await?;
                    // 整章终稿审核只能引用本章文档中的已持久化修订。
                    validate_review_decision_citations(
                        transaction,
                        &citations,
                        review_target.version_id,
                        Some(review_target.chapter_id),
                    )
                    .await?;
                    validate_review_decision_feedback(&decision, &citations)?;
                    let now = utc_now_millis();
                    persist_human_review_decision(
                        transaction,
                        review_id,
                        &decision,
                        &citations,
                        now,
                    )
                    .await?;
                    // 「退回修订」的审核意见就是这份稿子的修订要求，必须和面板里的自由意见
                    // 走同一条通道到达正文 Agent，而不是只落在审核行里：先排进未封存批次，
                    // 等模型产生下一条正文编辑的结果时随它投递；未投递期间直接终稿提交会被
                    // 守卫拦住，所以模型必须先真的改一次正文。
                    let rejection_feedback = if decision.passed {
                        None
                    } else {
                        Some(
                            queue_chapter_body_feedback(
                                transaction,
                                &body_run,
                                decision.issues.clone(),
                                &citations,
                                now,
                            )
                            .await?,
                        )
                    };
                    let next_workflow = if decision.passed {
                        let context_fold = build_chapter_body_context_fold(
                            transaction,
                            run_id,
                            review_target.target_revision_id,
                            &body_content,
                        )
                        .await?;
                        let checkpoint = RunCheckpoint {
                            chapter_id: Some(review_target.chapter_id),
                            context_fold: Some(context_fold),
                            held_chapter_body_submission: None,
                        };
                        update_chapter_body_run_status(
                            transaction,
                            run_id,
                            review_target.chapter_id,
                            "succeeded",
                            Some(&checkpoint),
                            Some(now),
                            now,
                        )
                        .await?;
                        let chapter_model =
                            chapter::Entity::find_by_id(db_id(review_target.chapter_id))
                                .filter(chapter::Entity::COLUMN.deleted_at.is_null())
                                .one(transaction)
                                .await?
                                .ok_or(StoreError::ChapterNotFound)?;
                        let chapter_kind = chapter_model.kind.clone();
                        let chapter_next_revision = chapter_model.revision + 1;
                        let mut active_chapter = chapter_model.into_active_model();
                        active_chapter.status = Set("approved".to_owned());
                        active_chapter.approved_revision_id =
                            Set(Some(review_target.target_revision_id.to_string()));
                        active_chapter.revision = Set(chapter_next_revision);
                        active_chapter.updated_at = Set(now);
                        active_chapter.update(transaction).await?;
                        match chapter_kind.as_str() {
                            "main" => finish_chapter_final_review(
                                &workflow,
                                review_target.chapter_id,
                                ChapterFinalReviewCompletion::Approved,
                            )?,
                            "extra" => finish_extra_chapter_final_review(
                                &workflow,
                                review_target.chapter_id,
                            )?,
                            kind => {
                                return Err(StoreError::CorruptRecord(format!(
                                    "终稿审核目标使用了未知章节类型：{kind}"
                                )));
                            }
                        }
                    } else {
                        update_chapter_body_run_status(
                            transaction,
                            run_id,
                            review_target.chapter_id,
                            "queued",
                            None,
                            None,
                            now,
                        )
                        .await?;
                        finish_chapter_final_review(
                            &workflow,
                            review_target.chapter_id,
                            ChapterFinalReviewCompletion::RevisionsRequired,
                        )?
                    };
                    persist_workflow(transaction, &next_workflow).await?;
                    insert_event(
                        transaction,
                        review_target.version_id,
                        "review.changed",
                        review_id,
                        next_workflow.state_revision,
                        json!({
                            "chapter_id": review_target.chapter_id,
                            "role": CHAPTER_BODY_ROLE,
                            "passed": decision.passed,
                            "status": if decision.passed { "approved" } else { "rejected" },
                        }),
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        review_target.version_id,
                        "run.changed",
                        run_id,
                        next_workflow.state_revision,
                        json!({
                            "chapter_id": review_target.chapter_id,
                            "kind": "chapter_body",
                            "status": if decision.passed { "succeeded" } else { "queued" },
                        }),
                        now,
                    )
                    .await?;
                    if let Some(feedback) = &rejection_feedback {
                        insert_event(
                            transaction,
                            review_target.version_id,
                            "feedback.changed",
                            feedback.id,
                            next_workflow.state_revision,
                            json!({
                                "chapter_id": review_target.chapter_id,
                                "run_id": run_id,
                                "status": feedback_status_name(feedback.status),
                                "sequence": feedback.sequence.to_string(),
                            }),
                            now,
                        )
                        .await?;
                    }
                    if decision.passed {
                        insert_event(
                            transaction,
                            review_target.version_id,
                            "chapter.changed",
                            review_target.chapter_id,
                            next_workflow.state_revision,
                            json!({
                                "status": "approved",
                                "approved_revision_id": review_target.target_revision_id,
                            }),
                            now,
                        )
                        .await?;
                    }
                    insert_event(
                        transaction,
                        review_target.version_id,
                        "workflow.changed",
                        review_target.version_id,
                        next_workflow.state_revision,
                        json!({
                            "stage": next_workflow.stage,
                            "chapter_id": review_target.chapter_id,
                            "run_id": next_workflow.active_run_id,
                        }),
                        now,
                    )
                    .await?;
                    let response =
                        preparation_snapshot(transaction, review_target.version_id).await?;
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        DECIDE_CHAPTER_FINAL_REVIEW_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    /// 在正文已确认完本后，新增一篇番外并进入其章节指导界面。
    ///
    /// 番外不修改已冻结的正文章节计划。它拥有独立的章节记录与三份文档槽位，
    /// 但继续使用已通过的准备、正文章节和既有番外纯文本折叠作为后续模型上下文。
    pub async fn add_extra_chapter(
        &self,
        version_id: VersionId,
        command_id: Uuid,
        expected_state_revision: i64,
        input: NewExtraChapter,
    ) -> Result<PreparationSnapshot, StoreError> {
        let target = format!("/api/versions/{version_id}/extras");
        let payload_hash = payload_hash(&AddExtraChapterPayload {
            expected_state_revision,
            title: &input.title,
        })?;
        let command = Command::new(command_id, ADD_EXTRA_CHAPTER_METHOD, &target, &payload_hash);

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    if workflow.stage != WorkflowStage::Completed
                        || workflow.execution != ExecutionStatus::Idle
                        || workflow.active_run_id.is_some()
                    {
                        return Err(StoreError::InvalidChapterState(
                            "只有已完本且没有活跃任务的稿本可以新增番外".to_owned(),
                        ));
                    }
                    if !all_main_chapters_are_approved(transaction, version_id).await? {
                        return Err(StoreError::InvalidChapterState(
                            "仍有计划正文章节未通过，不能新增番外".to_owned(),
                        ));
                    }
                    if open_review_exists(transaction, version_id).await?
                        || active_top_level_run_exists(transaction, version_id).await?
                    {
                        return Err(StoreError::InvalidChapterState(
                            "仍有审核或写作任务，不能新增番外".to_owned(),
                        ));
                    }

                    let version_model = novel_version::Entity::find_by_id(db_id(version_id))
                        .filter(novel_version::Entity::COLUMN.deleted_at.is_null())
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::VersionNotFound)?;
                    if version_model.body_completed_at.is_none() {
                        return Err(StoreError::CorruptRecord(
                            "完本工作流缺少正文完本时间".to_owned(),
                        ));
                    }

                    let chapters = chapter::Entity::find()
                        .filter(chapter::Entity::COLUMN.version_id.eq(db_id(version_id)))
                        .filter(chapter::Entity::COLUMN.deleted_at.is_null())
                        .all(transaction)
                        .await?;
                    let max_order_index = chapters
                        .iter()
                        .map(|chapter| chapter.order_index)
                        .max()
                        .ok_or_else(|| {
                            StoreError::CorruptRecord("完本稿本缺少正文章节".to_owned())
                        })?;
                    let order_index = max_order_index.checked_add(1).ok_or_else(|| {
                        StoreError::CorruptRecord("章节顺序已达到最大值".to_owned())
                    })?;
                    let chapter_id = Uuid::new_v4();
                    let now = utc_now_millis();
                    chapter::ActiveModel {
                        id: Set(db_id(chapter_id)),
                        version_id: Set(db_id(version_id)),
                        kind: Set("extra".to_owned()),
                        plan_item_id: Set(None),
                        order_index: Set(order_index),
                        title: Set(input.title.clone()),
                        status: Set("planned".to_owned()),
                        approved_revision_id: Set(None),
                        revision: Set(0),
                        created_at: Set(now),
                        updated_at: Set(now),
                        deleted_at: Set(None),
                    }
                    .insert(transaction)
                    .await?;
                    insert_chapter_document_slots(transaction, version_id, chapter_id, now).await?;

                    let next_workflow = enter_extra_chapter(&workflow, chapter_id)?;
                    let next_version_revision = version_model.revision + 1;
                    let mut active_version = version_model.into_active_model();
                    active_version.revision = Set(next_version_revision);
                    active_version.updated_at = Set(now);
                    active_version.update(transaction).await?;
                    persist_workflow(transaction, &next_workflow).await?;
                    insert_event(
                        transaction,
                        version_id,
                        "workflow.changed",
                        version_id,
                        next_workflow.state_revision,
                        json!({
                            "stage": next_workflow.stage,
                            "chapter_id": chapter_id,
                            "kind": "extra",
                            "run_id": serde_json::Value::Null,
                        }),
                        now,
                    )
                    .await?;
                    let response = preparation_snapshot(transaction, version_id).await?;
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        ADD_EXTRA_CHAPTER_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    /// 在已定稿章节之后进入下一份计划章节的指导界面。
    ///
    /// 这是一个独立于终稿审核决定的显式命令：先前正文 Run 的完成与折叠保持
    /// 不变，新的章节也不会因此自动调用模型。若已无后续计划章，调用方应保留
    /// 在 `writing_chapter_done` 并走显式完本确认流程。
    pub async fn advance_to_next_chapter(
        &self,
        version_id: VersionId,
        command_id: Uuid,
        expected_state_revision: i64,
    ) -> Result<PreparationSnapshot, StoreError> {
        let target = format!("/api/versions/{version_id}/chapters/next");
        let payload_hash = payload_hash(&AdvanceToNextChapterPayload {
            expected_state_revision,
        })?;
        let command = Command::new(
            command_id,
            ADVANCE_TO_NEXT_CHAPTER_METHOD,
            &target,
            &payload_hash,
        );

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    if workflow.stage != WorkflowStage::WritingChapterDone {
                        return Err(StoreError::InvalidChapterState(
                            "只有当前章节终稿通过后才能进入下一章".to_owned(),
                        ));
                    }
                    if workflow.active_run_id.is_some() {
                        return Err(StoreError::CorruptRecord(
                            "本章完成状态不应保留活跃 Run".to_owned(),
                        ));
                    }
                    let completed_chapter_id = workflow.active_chapter_id.ok_or_else(|| {
                        StoreError::CorruptRecord("本章完成状态缺少当前章节".to_owned())
                    })?;
                    let completed_chapter =
                        chapter::Entity::find_by_id(db_id(completed_chapter_id))
                            .filter(chapter::Entity::COLUMN.version_id.eq(db_id(version_id)))
                            .filter(chapter::Entity::COLUMN.deleted_at.is_null())
                            .one(transaction)
                            .await?
                            .ok_or(StoreError::ChapterNotFound)?;
                    if completed_chapter.status != "approved" {
                        return Err(StoreError::InvalidChapterState(
                            "当前章节尚未锁定为通过终稿，不能进入下一章".to_owned(),
                        ));
                    }
                    let next_chapter =
                        next_chapter_after(transaction, version_id, &completed_chapter)
                            .await?
                            .ok_or_else(|| {
                                StoreError::InvalidChapterState(
                                    "没有后续计划章节；请在完本确认功能中结束正文".to_owned(),
                                )
                            })?;
                    if next_chapter.status != "planned" {
                        return Err(StoreError::InvalidChapterState(
                            "下一计划章节不是待开始状态，不能跳过章节顺序".to_owned(),
                        ));
                    }
                    let next_chapter_id: ChapterId = next_chapter.id.into();
                    let next_workflow = enter_next_chapter(&workflow, next_chapter_id)?;
                    let now = utc_now_millis();
                    persist_workflow(transaction, &next_workflow).await?;
                    insert_event(
                        transaction,
                        version_id,
                        "workflow.changed",
                        version_id,
                        next_workflow.state_revision,
                        json!({
                            "stage": next_workflow.stage,
                            "chapter_id": next_chapter_id,
                            "previous_chapter_id": completed_chapter_id,
                            "run_id": serde_json::Value::Null,
                        }),
                        now,
                    )
                    .await?;
                    let response = preparation_snapshot(transaction, version_id).await?;
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        ADVANCE_TO_NEXT_CHAPTER_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    /// 显式确认所有计划正文章节已经完成。
    ///
    /// 该命令不自动从“本章完成”跳转：用户必须主动确认。确认时重新读取章节、
    /// 审核和 Run，避免陈旧页面或并发命令把仍有待处理内容的稿本错误标为完本。
    pub async fn confirm_novel_completion(
        &self,
        version_id: VersionId,
        command_id: Uuid,
        expected_state_revision: i64,
    ) -> Result<PreparationSnapshot, StoreError> {
        let target = format!("/api/versions/{version_id}/completion");
        let payload_hash = payload_hash(&ConfirmNovelCompletionPayload {
            expected_state_revision,
        })?;
        let command = Command::new(
            command_id,
            CONFIRM_NOVEL_COMPLETION_METHOD,
            &target,
            &payload_hash,
        );

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    if workflow.stage != WorkflowStage::WritingChapterDone {
                        return Err(StoreError::InvalidChapterState(
                            "只有最后一章终稿通过后才能确认完本".to_owned(),
                        ));
                    }
                    if workflow.active_run_id.is_some() {
                        return Err(StoreError::CorruptRecord(
                            "本章完成状态不应保留活跃 Run".to_owned(),
                        ));
                    }
                    let completed_chapter_id = workflow.active_chapter_id.ok_or_else(|| {
                        StoreError::CorruptRecord("本章完成状态缺少当前章节".to_owned())
                    })?;
                    let completed_chapter =
                        chapter::Entity::find_by_id(db_id(completed_chapter_id))
                            .filter(chapter::Entity::COLUMN.version_id.eq(db_id(version_id)))
                            .filter(chapter::Entity::COLUMN.deleted_at.is_null())
                            .one(transaction)
                            .await?
                            .ok_or(StoreError::ChapterNotFound)?;
                    if completed_chapter.status != "approved" {
                        return Err(StoreError::InvalidChapterState(
                            "当前章节尚未锁定为通过终稿，不能确认完本".to_owned(),
                        ));
                    }
                    if !all_main_chapters_are_approved(transaction, version_id).await? {
                        return Err(StoreError::InvalidChapterState(
                            "仍有计划正文章节未通过，不能确认完本".to_owned(),
                        ));
                    }
                    if open_review_exists(transaction, version_id).await? {
                        return Err(StoreError::InvalidChapterState(
                            "仍有开放审核，不能确认完本".to_owned(),
                        ));
                    }
                    if active_top_level_run_exists(transaction, version_id).await? {
                        return Err(StoreError::InvalidChapterState(
                            "仍有活跃写作任务，不能确认完本".to_owned(),
                        ));
                    }
                    let version_model = novel_version::Entity::find_by_id(db_id(version_id))
                        .filter(novel_version::Entity::COLUMN.deleted_at.is_null())
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::VersionNotFound)?;
                    if version_model.body_completed_at.is_some() {
                        return Err(StoreError::InvalidChapterState(
                            "该稿本已经确认完本".to_owned(),
                        ));
                    }
                    let now = utc_now_millis();
                    let next_workflow = confirm_novel_completion(&workflow)?;
                    let next_version_revision = version_model.revision + 1;
                    let mut active_version = version_model.into_active_model();
                    active_version.body_completed_at = Set(Some(now));
                    active_version.revision = Set(next_version_revision);
                    active_version.updated_at = Set(now);
                    active_version.update(transaction).await?;
                    persist_workflow(transaction, &next_workflow).await?;
                    insert_event(
                        transaction,
                        version_id,
                        "version.completed",
                        version_id,
                        next_workflow.state_revision,
                        json!({ "body_completed_at_ms": now }),
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        version_id,
                        "workflow.changed",
                        version_id,
                        next_workflow.state_revision,
                        json!({
                            "stage": next_workflow.stage,
                            "chapter_id": completed_chapter_id,
                            "run_id": serde_json::Value::Null,
                        }),
                        now,
                    )
                    .await?;
                    let response = preparation_snapshot(transaction, version_id).await?;
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        CONFIRM_NOVEL_COMPLETION_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    /// 提交当前章节的一份完整细纲以供人工审核。
    ///
    /// 该入口与模型的 `OutlineSubmission` 共用正式的文档修订、依赖、审核和折叠
    /// 模型。模型请求无法启动或用户选择手工创作时，它仍让本地工具完整走通可审核
    /// 写作流程，而不会把手工文本写进临时旁路。
    pub async fn submit_chapter_outline(
        &self,
        chapter_id: ChapterId,
        command_id: Uuid,
        expected_state_revision: i64,
        input: NewChapterOutline,
    ) -> Result<ChapterOutlineSnapshot, StoreError> {
        let target = format!("/api/chapters/{chapter_id}/outline/submission");
        let payload_hash = payload_hash(&SubmitChapterOutlinePayload {
            expected_state_revision,
            content: &input.content,
        })?;
        let command = Command::new(
            command_id,
            SUBMIT_CHAPTER_OUTLINE_METHOD,
            &target,
            &payload_hash,
        );

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let (chapter_model, _) = chapter_and_document(
                        transaction,
                        chapter_id,
                        CHAPTER_OUTLINE_ROLE,
                        "章节细纲",
                    )
                    .await?;
                    let version_id: VersionId = chapter_model.version_id.into();
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    if workflow.execution == ExecutionStatus::RequestInFlight {
                        return Err(StoreError::ResourceBusy(
                            "模型正在生成章节细纲，不能同时手动提交".to_owned(),
                        ));
                    }
                    let now = utc_now_millis();
                    persist_chapter_outline_submission(
                        transaction,
                        chapter_id,
                        &workflow,
                        &input,
                        "user",
                        now,
                    )
                    .await?;
                    let response = chapter_outline_snapshot(transaction, chapter_id).await?;
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        SUBMIT_CHAPTER_OUTLINE_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    /// 对当前章节细纲审核作出人工决定。
    pub async fn decide_chapter_outline_review(
        &self,
        review_id: ReviewId,
        command_id: Uuid,
        expected_state_revision: i64,
        decision: HumanReviewDecision,
        citations: Vec<FeedbackCitation>,
    ) -> Result<PreparationSnapshot, StoreError> {
        let target = format!("/api/reviews/{review_id}/decision");
        let payload_hash = payload_hash(&DecidePreparationReviewPayload {
            expected_state_revision,
            decision: &decision,
            citations: &citations,
        })?;
        let command = Command::new(
            command_id,
            DECIDE_CHAPTER_OUTLINE_REVIEW_METHOD,
            &target,
            &payload_hash,
        );

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let review_target =
                        chapter_outline_review_target(transaction, review_id).await?;
                    if review_target.status != ReviewStatus::Open {
                        return Err(StoreError::ReviewAlreadyResolved);
                    }
                    let workflow =
                        workflow_for_version(transaction, review_target.version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    if workflow.stage != WorkflowStage::WritingOutlineReviewing {
                        return Err(StoreError::InvalidChapterState(
                            "只有细纲审核中状态可以作出细纲审核决定".to_owned(),
                        ));
                    }
                    if workflow.active_chapter_id != Some(review_target.chapter_id) {
                        return Err(StoreError::InvalidChapterState(
                            "细纲审核目标不是当前章节".to_owned(),
                        ));
                    }
                    if review_target.current_revision_id != review_target.target_revision_id {
                        return Err(StoreError::InvalidChapterState(
                            "审核目标已不是当前细纲修订，不能对旧修订作出决定".to_owned(),
                        ));
                    }
                    let expected_dependencies = chapter_outline_dependency_revision_ids(
                        transaction,
                        review_target.version_id,
                        review_target.chapter_id,
                    )
                    .await?;
                    let actual_dependencies =
                        revision_dependency_ids(transaction, review_target.target_revision_id)
                            .await?;
                    if review_target.dependency_revision_ids != expected_dependencies
                        || actual_dependencies != expected_dependencies
                    {
                        return Err(StoreError::InvalidChapterState(
                            "细纲审核依赖已变化，不能对旧输入作出决定".to_owned(),
                        ));
                    }

                    let run_id = workflow.active_run_id.ok_or_else(|| {
                        StoreError::CorruptRecord("细纲审核缺少对应的活跃 Run".to_owned())
                    })?;
                    chapter_outline_run(transaction, run_id, review_target.chapter_id).await?;
                    // 细纲审核只能引用本章文档中的已持久化修订。
                    validate_review_decision_citations(
                        transaction,
                        &citations,
                        review_target.version_id,
                        Some(review_target.chapter_id),
                    )
                    .await?;
                    validate_review_decision_feedback(&decision, &citations)?;
                    let now = utc_now_millis();
                    persist_human_review_decision(
                        transaction,
                        review_id,
                        &decision,
                        &citations,
                        now,
                    )
                    .await?;
                    let (next_workflow, body_run_id) = if decision.passed {
                        let outline =
                            current_chapter_outline(transaction, review_target.chapter_id).await?;
                        if outline.revision_id != Some(review_target.target_revision_id) {
                            return Err(StoreError::CorruptRecord(
                                "当前章节细纲在审核决定期间发生变化".to_owned(),
                            ));
                        }
                        let content = outline.content.ok_or_else(|| {
                            StoreError::CorruptRecord("当前章节细纲缺少正文".to_owned())
                        })?;
                        let context_fold = build_chapter_outline_context_fold(
                            transaction,
                            run_id,
                            review_target.target_revision_id,
                            &content,
                        )
                        .await?;
                        let checkpoint = RunCheckpoint {
                            chapter_id: Some(review_target.chapter_id),
                            context_fold: Some(context_fold),
                            held_chapter_body_submission: None,
                        };
                        update_chapter_outline_run_status(
                            transaction,
                            run_id,
                            review_target.chapter_id,
                            "succeeded",
                            Some(&checkpoint),
                            Some(now),
                            now,
                        )
                        .await?;
                        let completed_outline = finish_chapter_outline_review(
                            &workflow,
                            review_target.chapter_id,
                            ChapterOutlineReviewCompletion::Approved,
                        )?;
                        let body_run_id = Uuid::new_v4();
                        let project_id =
                            project_id_for_version(transaction, review_target.version_id).await?;
                        ensure_no_active_top_level_write_run(transaction, project_id).await?;
                        insert_chapter_body_run(
                            transaction,
                            body_run_id,
                            project_id,
                            review_target.version_id,
                            review_target.chapter_id,
                            now,
                        )
                        .await?;
                        (
                            schedule_chapter_body_run(
                                &completed_outline,
                                review_target.chapter_id,
                                body_run_id,
                            )?,
                            Some(body_run_id),
                        )
                    } else {
                        update_chapter_outline_run_status(
                            transaction,
                            run_id,
                            review_target.chapter_id,
                            "queued",
                            None,
                            None,
                            now,
                        )
                        .await?;
                        (
                            finish_chapter_outline_review(
                                &workflow,
                                review_target.chapter_id,
                                ChapterOutlineReviewCompletion::RevisionsRequired,
                            )?,
                            None,
                        )
                    };
                    persist_workflow(transaction, &next_workflow).await?;
                    insert_event(
                        transaction,
                        review_target.version_id,
                        "review.changed",
                        review_id,
                        next_workflow.state_revision,
                        json!({
                            "chapter_id": review_target.chapter_id,
                            "role": CHAPTER_OUTLINE_ROLE,
                            "passed": decision.passed,
                            "status": if decision.passed { "approved" } else { "rejected" },
                        }),
                        now,
                    )
                    .await?;
                    if let Some(body_run_id) = body_run_id {
                        insert_event(
                            transaction,
                            review_target.version_id,
                            "run.changed",
                            body_run_id,
                            next_workflow.state_revision,
                            json!({
                                "chapter_id": review_target.chapter_id,
                                "kind": "chapter_body",
                                "status": "queued",
                            }),
                            now,
                        )
                        .await?;
                    }
                    insert_event(
                        transaction,
                        review_target.version_id,
                        "run.changed",
                        run_id,
                        next_workflow.state_revision,
                        json!({
                            "chapter_id": review_target.chapter_id,
                            "kind": "chapter_outline",
                            "status": if decision.passed { "succeeded" } else { "queued" },
                        }),
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        review_target.version_id,
                        "workflow.changed",
                        review_target.version_id,
                        next_workflow.state_revision,
                        json!({
                            "stage": next_workflow.stage,
                            "chapter_id": review_target.chapter_id,
                            "run_id": next_workflow.active_run_id,
                        }),
                        now,
                    )
                    .await?;
                    let response =
                        preparation_snapshot(transaction, review_target.version_id).await?;
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        DECIDE_CHAPTER_OUTLINE_REVIEW_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    /// 按审核类型分派统一的 `/reviews/{id}/decision` 命令。
    ///
    /// 路由不需要把“准备审核”、细纲审核和终稿审核编码在 URL 中；审核类型由后端读取，
    /// 因而客户端不能借由换路径越过阶段守卫。
    pub async fn decide_review(
        &self,
        review_id: ReviewId,
        command_id: Uuid,
        expected_state_revision: i64,
        decision: HumanReviewDecision,
        citations: Vec<FeedbackCitation>,
    ) -> Result<PreparationSnapshot, StoreError> {
        let target = format!("/api/reviews/{review_id}/decision");
        let payload_hash = payload_hash(&DecidePreparationReviewPayload {
            expected_state_revision,
            decision: &decision,
            citations: &citations,
        })?;
        // 派发前先查回执：命中即返回，省掉 `review_type_for_id` 的读库；而它下面的
        // 各条分支自己会预查一次、`commit` 在事务前再确认一次。这里必须保留，因为
        // `review_type_for_id` 在审核行不存在时返回 `ReviewNotFound`——重放一条已记账的
        // 决定不应该因为审核行后来消失而从「返回既有结果」变成报错。
        if let Some(receipt) = self.find_receipt(command_id).await? {
            return replay_or_conflict(
                receipt,
                DECIDE_PREPARATION_REVIEW_METHOD,
                &target,
                &payload_hash,
            );
        }
        match review_type_for_id(&self.database, review_id)
            .await?
            .as_str()
        {
            "preparation" => {
                self.decide_preparation_review(
                    review_id,
                    command_id,
                    expected_state_revision,
                    decision,
                    citations,
                )
                .await
            }
            "chapter_outline" => {
                self.decide_chapter_outline_review(
                    review_id,
                    command_id,
                    expected_state_revision,
                    decision,
                    citations,
                )
                .await
            }
            "chapter_final" => {
                self.decide_chapter_final_review(
                    review_id,
                    command_id,
                    expected_state_revision,
                    decision,
                    citations,
                )
                .await
            }
            review_type => Err(StoreError::CorruptRecord(format!(
                "未知审核类型：{review_type}"
            ))),
        }
    }

    /// 为一项当前开放的审核创建一次独立的审核 Agent 子 Run。
    ///
    /// 父写作 Run 继续停在 `waiting_review`；子 Run 只拥有本次审核请求，不会获得
    /// 正文编辑工具，也不会改变父 Run 的消息或额度。委托命令、Review 状态和子 Run
    /// 在同一事务中创建，因此同一 `command_id` 重试不会重复发送审核请求。
    pub async fn delegate_review(
        &self,
        review_id: ReviewId,
        command_id: Uuid,
        expected_state_revision: i64,
        notes: String,
        citations: Vec<FeedbackCitation>,
    ) -> Result<ReviewDelegationResult, StoreError> {
        validate_delegation_notes(&notes)?;
        let target = format!("/api/reviews/{review_id}/delegate");
        let payload_hash = payload_hash(&DelegateReviewPayload {
            expected_state_revision,
            notes: &notes,
            citations: &citations,
        })?;
        let command = Command::new(command_id, DELEGATE_REVIEW_METHOD, &target, &payload_hash);

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let target =
                        delegatable_review_target(transaction, review_id, expected_state_revision)
                            .await?;
                    // 引用必须在委托时就能被逐字复核；之后目标修订变化时，审核回调会
                    // 因 attempt 失效而不被应用，不靠引用在读取时再次回查。
                    validate_delegation_citations(
                        transaction,
                        &citations,
                        target.version_id,
                        review_id,
                        target.kind,
                    )
                    .await?;
                    let now = utc_now_millis();
                    let run_id = Uuid::new_v4();
                    insert_delegated_review_run(
                        transaction,
                        run_id,
                        target.parent_run_id,
                        target.project_id,
                        target.version_id,
                        review_id,
                        target.target_revision_id,
                        &target.dependency_revision_ids,
                        target.kind,
                        now,
                    )
                    .await?;
                    let review_model = review::Entity::find_by_id(db_id(review_id))
                        .filter(review::Entity::COLUMN.status.eq("open"))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::ReviewAlreadyResolved)?;
                    let mut active_review = review_model.into_active_model();
                    active_review.status = Set("delegated".to_owned());
                    active_review.attempt_id = Set(Some(run_id.to_string()));
                    active_review.decision_source = Set(None);
                    active_review.passed = Set(None);
                    active_review.confidence = Set(None);
                    active_review.issues_json = Set(None);
                    active_review.user_input_json = Set(Some(serde_json::to_string(&json!({
                        "source": "user",
                        "notes": notes,
                        "citations": citations,
                    }))?));
                    active_review.updated_at = Set(now);
                    active_review.update(transaction).await?;
                    insert_event(
                        transaction,
                        target.version_id,
                        "review.changed",
                        review_id,
                        target.workflow_revision,
                        json!({
                            "status": "delegated",
                            "run_id": run_id,
                            "kind": target.kind.stable_name(),
                        }),
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        target.version_id,
                        "run.changed",
                        run_id,
                        target.workflow_revision,
                        json!({
                            "kind": "review",
                            "status": "queued",
                            "review_id": review_id,
                        }),
                        now,
                    )
                    .await?;
                    let response = ReviewDelegationResult {
                        review_id,
                        run_id,
                        snapshot: preparation_snapshot(transaction, target.version_id).await?,
                    };
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        DELEGATE_REVIEW_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        202,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    /// 取消正在等待或运行的审核 Agent，并重新开放同一项 Review 供人工决定或重试。
    ///
    /// 旧 `attempt_id` 保留为诊断事实，但 Review 已不再接受它的迟到回调；下次委托
    /// 会生成新的子 Run 与 attempt 标识。
    pub async fn cancel_review_delegation(
        &self,
        review_id: ReviewId,
        command_id: Uuid,
        expected_state_revision: i64,
    ) -> Result<CancelReviewDelegationResult, StoreError> {
        let target = format!("/api/reviews/{review_id}/cancel-delegation");
        let payload_hash = payload_hash(&CancelReviewDelegationPayload {
            expected_state_revision,
        })?;
        let command = Command::new(
            command_id,
            CANCEL_REVIEW_DELEGATION_METHOD,
            &target,
            &payload_hash,
        );

        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let delegated = delegated_review_attempt(transaction, review_id).await?;
                    let workflow = workflow_for_version(transaction, delegated.version_id).await?;
                    ensure_expected_state_revision(&workflow, expected_state_revision)?;
                    if workflow.execution != ExecutionStatus::WaitingReview
                        || !review_stage_matches(&workflow, delegated.kind)
                    {
                        return Err(StoreError::InvalidChapterState(
                            "当前工作流不再等待该审核，不能取消委托".to_owned(),
                        ));
                    }
                    let now = utc_now_millis();
                    let outstanding = model_request::Entity::find()
                        .filter(
                            model_request::Entity::COLUMN
                                .run_id
                                .eq(db_id(delegated.run_id)),
                        )
                        .filter(
                            model_request::Entity::COLUMN
                                .status
                                .is_in(["prepared", "dispatched"]),
                        )
                        .all(transaction)
                        .await?;
                    let mut cancelled_request_ids = Vec::with_capacity(outstanding.len());
                    for request in outstanding {
                        let request_id: Uuid = request.id.into();
                        let mut active_request = request.into_active_model();
                        active_request.status = Set("cancelled".to_owned());
                        active_request.error_json = Set(Some(serde_json::to_string(&json!({
                            "kind": "delegation_cancelled",
                            "message": "用户取消了审核委托",
                        }))?));
                        active_request.updated_at = Set(now);
                        active_request.completed_at = Set(Some(now));
                        active_request.update(transaction).await?;
                        cancelled_request_ids.push(request_id);
                        insert_event(
                            transaction,
                            delegated.version_id,
                            "model_request.cancelled",
                            request_id,
                            workflow.state_revision,
                            json!({
                                "run_id": delegated.run_id,
                                "reason": "delegation_cancelled",
                            }),
                            now,
                        )
                        .await?;
                    }
                    let run_model = run::Entity::find_by_id(db_id(delegated.run_id))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    let mut active_run = run_model.into_active_model();
                    active_run.status = Set("cancelled".to_owned());
                    active_run.ended_at = Set(Some(now));
                    active_run.updated_at = Set(now);
                    active_run.update(transaction).await?;
                    let review_model = review::Entity::find_by_id(db_id(review_id))
                        .filter(review::Entity::COLUMN.status.eq("delegated"))
                        .filter(
                            review::Entity::COLUMN
                                .attempt_id
                                .eq(delegated.run_id.to_string()),
                        )
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::ReviewAlreadyResolved)?;
                    let mut active_review = review_model.into_active_model();
                    active_review.status = Set("open".to_owned());
                    active_review.decision_source = Set(None);
                    active_review.passed = Set(None);
                    active_review.confidence = Set(None);
                    active_review.issues_json = Set(None);
                    active_review.updated_at = Set(now);
                    active_review.update(transaction).await?;
                    insert_event(
                        transaction,
                        delegated.version_id,
                        "review.changed",
                        review_id,
                        workflow.state_revision,
                        json!({
                            "status": "open",
                            "cancelled_attempt_id": delegated.run_id,
                        }),
                        now,
                    )
                    .await?;
                    insert_event(
                        transaction,
                        delegated.version_id,
                        "run.changed",
                        delegated.run_id,
                        workflow.state_revision,
                        json!({
                            "kind": "review",
                            "status": "cancelled",
                            "review_id": review_id,
                        }),
                        now,
                    )
                    .await?;
                    let response = CancelReviewDelegationResult {
                        review_id,
                        run_id: delegated.run_id,
                        cancelled_request_ids,
                        snapshot: preparation_snapshot(transaction, delegated.version_id).await?,
                    };
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        CANCEL_REVIEW_DELEGATION_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    /// 读取一个仍然有效的审核 Agent 输入快照。
    pub async fn delegated_review_task(
        &self,
        review_id: ReviewId,
        run_id: RunId,
    ) -> Result<DelegatedReviewTask, StoreError> {
        self.database
            .transaction::<_, _, StoreError>(|transaction| {
                Box::pin(async move {
                    build_delegated_review_task(transaction, review_id, run_id).await
                })
            })
            .await
            .map_err(transaction_error)
    }

    /// 将审核 Agent 的已验证结构化结论、子 Run 请求终态和父写作工作流一次提交。
    ///
    /// 子 Run 的模型响应只能应用到仍由同一个 `attempt_id` 占有的 `delegated`
    /// Review。取消、重新委托或目标修订变化后到达的回调因此无法推进父工作流。
    #[allow(clippy::too_many_arguments)]
    pub async fn commit_delegated_review_result(
        &self,
        review_id: ReviewId,
        run_id: RunId,
        request_id: Uuid,
        provider_request_id: Option<String>,
        usage: Value,
        assistant_message: NewAgentMessage,
        passed: bool,
        confidence: f64,
        issues: Vec<String>,
    ) -> Result<PreparationSnapshot, StoreError> {
        let decision = DelegatedReviewDecision {
            passed,
            confidence,
            issues,
        };
        validate_delegated_review_decision(&decision)?;
        let submission_messages = phase_submission_messages(request_id, assistant_message)?;
        if !usage.is_object() {
            return Err(StoreError::InvalidModelRequest(
                "审核 Agent 用量必须是 JSON object".to_owned(),
            ));
        }
        let result = self
            .database
            .transaction::<_, _, StoreError>(move |transaction| {
                Box::pin(async move {
                    if let Some(snapshot) =
                        replayed_delegated_review_result(transaction, review_id, run_id, request_id)
                            .await?
                    {
                        return Ok(snapshot);
                    }
                    let attempt = delegated_review_attempt(transaction, review_id).await?;
                    if attempt.run_id != run_id {
                        return Err(StoreError::InvalidModelRequestState(
                            "审核 Agent 回调不属于当前委托尝试".to_owned(),
                        ));
                    }
                    let request = model_request::Entity::find_by_id(db_id(request_id))
                        .filter(model_request::Entity::COLUMN.run_id.eq(db_id(run_id)))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::ModelRequestNotFound)?;
                    if request.status != "dispatched" {
                        return Err(StoreError::InvalidModelRequestState(format!(
                            "只有 dispatched 审核请求可以提交结论，当前为 {}",
                            request.status
                        )));
                    }
                    let child_run = run::Entity::find_by_id(db_id(run_id))
                        .filter(run::Entity::COLUMN.kind.eq("review"))
                        .filter(run::Entity::COLUMN.version_id.eq(db_id(attempt.version_id)))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    if child_run.status != "running" {
                        return Err(StoreError::InvalidModelRequestState(
                            "审核 Agent 子 Run 不在运行状态".to_owned(),
                        ));
                    }
                    let now = utc_now_millis();
                    append_agent_messages_for_run(
                        transaction,
                        &child_run,
                        submission_messages.clone(),
                        now,
                    )
                    .await?;
                    let version_id = match attempt.kind {
                        DelegatedReviewKind::Preparation(_) => {
                            apply_delegated_preparation_review(
                                transaction,
                                review_id,
                                run_id,
                                &decision,
                                now,
                            )
                            .await?
                        }
                        DelegatedReviewKind::ChapterOutline => {
                            apply_delegated_chapter_outline_review(
                                transaction,
                                review_id,
                                run_id,
                                &decision,
                                now,
                            )
                            .await?
                        }
                        DelegatedReviewKind::ChapterEdit => {
                            let review_decision = delegated_review_decision_for_workflow(&decision);
                            apply_chapter_edit_review_decision(
                                transaction,
                                review_id,
                                ChapterEditReviewDecision {
                                    passed: review_decision.passed,
                                    issues: review_decision.issues,
                                    review_run_id: run_id,
                                    confidence: decision.confidence,
                                },
                                now,
                            )
                            .await?
                        }
                        DelegatedReviewKind::ChapterFinal => {
                            apply_delegated_chapter_final_review(
                                transaction,
                                review_id,
                                run_id,
                                &decision,
                                now,
                            )
                            .await?
                        }
                    };
                    let mut active_request = request.into_active_model();
                    active_request.status = Set("succeeded".to_owned());
                    active_request.provider_request_id = Set(provider_request_id);
                    active_request.usage_json = Set(Some(serde_json::to_string(&usage)?));
                    active_request.error_json = Set(None);
                    active_request.updated_at = Set(now);
                    active_request.completed_at = Set(Some(now));
                    active_request.update(transaction).await?;
                    let mut active_child_run = child_run.into_active_model();
                    active_child_run.status = Set("succeeded".to_owned());
                    active_child_run.ended_at = Set(Some(now));
                    active_child_run.updated_at = Set(now);
                    active_child_run.update(transaction).await?;
                    let workflow = workflow_for_version(transaction, version_id).await?;
                    insert_event(
                        transaction,
                        version_id,
                        "run.changed",
                        run_id,
                        workflow.state_revision,
                        json!({
                            "kind": "review",
                            "status": "succeeded",
                            "review_id": review_id,
                            "request_id": request_id,
                        }),
                        now,
                    )
                    .await?;
                    preparation_snapshot(transaction, version_id).await
                })
            })
            .await
            .map_err(transaction_error);
        match result {
            Ok(snapshot) => {
                self.notify_event_listeners();
                Ok(snapshot)
            }
            Err(error) => Err(error),
        }
    }

    /// 将无法应用的审核 Agent 尝试收束为失败，并重新开放 Review。
    ///
    /// 此路径覆盖模型传输失败、结构化响应无效和发送前上下文校验失败。它不会将
    /// 失败伪装成通过，也不会自动换用其他审核方；用户可以再次委托或直接审核。
    pub async fn fail_delegated_review_attempt(
        &self,
        review_id: ReviewId,
        run_id: RunId,
        request_id: Option<Uuid>,
        provider_request_id: Option<String>,
        failure: Value,
        assistant_message: Option<NewAgentMessage>,
        unknown: bool,
    ) -> Result<bool, StoreError> {
        if !failure.is_object() {
            return Err(StoreError::InvalidModelRequest(
                "审核 Agent 失败信息必须是 JSON object".to_owned(),
            ));
        }
        let result = self
            .database
            .transaction::<_, _, StoreError>(move |transaction| {
                let assistant_message = assistant_message.clone();
                Box::pin(async move {
                    let attempt = match delegated_review_attempt(transaction, review_id).await {
                        Ok(attempt) if attempt.run_id == run_id => attempt,
                        Ok(_) | Err(StoreError::ReviewAlreadyResolved) => return Ok(false),
                        Err(error) => return Err(error),
                    };
                    let child_run = run::Entity::find_by_id(db_id(run_id))
                        .filter(run::Entity::COLUMN.kind.eq("review"))
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::RunNotFound)?;
                    if !matches!(child_run.status.as_str(), "queued" | "running") {
                        return Ok(false);
                    }
                    let now = utc_now_millis();
                    if let Some(request_id) = request_id {
                        let request = model_request::Entity::find_by_id(db_id(request_id))
                            .filter(model_request::Entity::COLUMN.run_id.eq(db_id(run_id)))
                            .one(transaction)
                            .await?
                            .ok_or(StoreError::ModelRequestNotFound)?;
                        if request.status != "dispatched" {
                            return Ok(false);
                        }
                        if let Some(message) = assistant_message {
                            if message.role != AgentMessageRole::Assistant
                                || message.request_id != Some(request_id)
                            {
                                return Err(StoreError::InvalidAgentMessage(
                                    "审核 Agent 失败响应必须关联当前请求".to_owned(),
                                ));
                            }
                            append_agent_messages_for_run(
                                transaction,
                                &child_run,
                                vec![message],
                                now,
                            )
                            .await?;
                        }
                        let mut active_request = request.into_active_model();
                        active_request.status = Set(if unknown {
                            "unknown".to_owned()
                        } else {
                            "failed".to_owned()
                        });
                        active_request.provider_request_id = Set(provider_request_id);
                        active_request.error_json = Set(Some(serde_json::to_string(&failure)?));
                        active_request.updated_at = Set(now);
                        active_request.completed_at = Set(Some(now));
                        active_request.update(transaction).await?;
                    }
                    fail_delegated_review_child_run(child_run.into_active_model(), now)
                        .update(transaction)
                        .await?;
                    reopen_delegated_review(
                        transaction,
                        review_id,
                        run_id,
                        attempt.version_id,
                        &failure,
                        now,
                    )
                    .await?;
                    Ok(true)
                })
            })
            .await
            .map_err(transaction_error);
        match result {
            Ok(changed) => {
                if changed {
                    self.notify_event_listeners();
                }
                Ok(changed)
            }
            Err(error) => Err(error),
        }
    }

    /// 将自动执行中的 Run 收束到「已中断」并返回收束数量。
    ///
    /// 启动恢复（进程上次以非正常方式结束）与退出服务（本次主动收尾在途请求）
    /// 是同一种收束，只有事件原因与用户可见文案不同，因此共用同一个入口。
    ///
    /// # Errors
    ///
    /// 收束事务失败，或数据库里 Run 与工作流执行状态已经不一致，都会返回错误；
    /// 此时不会部分写入：整个收束在一笔事务内完成。
    pub async fn interrupt_active_runs(
        &self,
        source: InterruptionSource,
    ) -> Result<InterruptionOutcome, StoreError> {
        let writing_runs = self.interrupt_active_writing_runs(source).await?;
        let review_delegations = self.interrupt_active_review_delegations(source).await?;
        Ok(InterruptionOutcome {
            writing_runs,
            review_delegations,
        })
    }

    /// 将没有稳定交互边界的自动写作 Run 收束为 `interrupted`。
    ///
    /// `waiting_answer`、`waiting_review` 与 `paused` 分别拥有持久 Question、Review
    /// 或暂停边界，因此保持原状以便 UI 直接恢复。只有 queued/running 的顶层写作
    /// Run 会被中断；尚未发送的 prepared request 作废，已 dispatch 的 request 记为
    /// unknown，避免把外部服务可能已经接收的调用伪装成未发生。
    pub async fn interrupt_active_writing_runs(
        &self,
        source: InterruptionSource,
    ) -> Result<usize, StoreError> {
        let reason = source.reason();
        let now = utc_now_millis();
        let result = self
            .database
            .transaction::<_, _, StoreError>(|transaction| {
                Box::pin(async move {
                    let candidates = run::Entity::find()
                        .filter(run::Column::ParentRunId.is_null())
                        .filter(run::Column::Kind.is_in([
                            "preparation",
                            "chapter_outline",
                            "chapter_body",
                        ]))
                        .filter(run::Column::Status.is_in(["queued", "running"]))
                        .all(transaction)
                        .await?;
                    let mut recovered = 0_usize;
                    for run_model in candidates {
                        let run_id: RunId = run_model.id.into();
                        let version_id: VersionId = run_model.version_id.into();
                        let workflow = workflow_for_version(transaction, version_id).await?;
                        if workflow.active_run_id != Some(run_id) {
                            return Err(StoreError::CorruptRecord(
                                "自动恢复发现非活跃顶层 Run 仍处于 queued/running".to_owned(),
                            ));
                        }
                        let next_workflow =
                            interrupt_writing_run(&workflow, run_id).map_err(|error| {
                                StoreError::CorruptRecord(format!(
                                    "自动恢复发现 Run 与工作流执行状态不一致：{error}"
                                ))
                            })?;
                        let outstanding_requests = model_request::Entity::find()
                            .filter(model_request::Column::RunId.eq(run_model.id))
                            .filter(model_request::Column::Status.is_in(["prepared", "dispatched"]))
                            .all(transaction)
                            .await?;
                        for request in outstanding_requests {
                            let (status, message) = match request.status.as_str() {
                                "prepared" => (
                                    "cancelled",
                                    format!("{}前请求尚未发送，已作废", source.prefix()),
                                ),
                                "dispatched" => (
                                    "unknown",
                                    format!("{}时请求是否被模型服务接受未知", source.prefix()),
                                ),
                                _ => {
                                    return Err(StoreError::CorruptRecord(
                                        "中断收尾查询返回了非在途模型请求".to_owned(),
                                    ));
                                }
                            };
                            let mut active_request = request.into_active_model();
                            active_request.status = Set(status.to_owned());
                            active_request.error_json = Set(Some(serde_json::to_string(&json!({
                                "kind": reason,
                                "message": message,
                            }))?));
                            active_request.updated_at = Set(now);
                            active_request.completed_at = Set(Some(now));
                            active_request.update(transaction).await?;
                        }
                        let run_kind = run_model.kind.clone();
                        let request_count = run_model.request_count;
                        let mut active_run = run_model.into_active_model();
                        active_run.status = Set("interrupted".to_owned());
                        active_run.ended_at = Set(Some(now));
                        active_run.updated_at = Set(now);
                        active_run.update(transaction).await?;
                        persist_workflow(transaction, &next_workflow).await?;
                        insert_event(
                            transaction,
                            version_id,
                            "run.changed",
                            run_id,
                            next_workflow.state_revision,
                            json!({
                                "kind": run_kind,
                                "status": "interrupted",
                                "request_count": request_count.to_string(),
                                "reason": reason,
                            }),
                            now,
                        )
                        .await?;
                        insert_event(
                            transaction,
                            version_id,
                            "workflow.changed",
                            version_id,
                            next_workflow.state_revision,
                            json!({
                                "stage": next_workflow.stage,
                                "execution": next_workflow.execution,
                                "run_id": run_id,
                                "chapter_id": next_workflow.active_chapter_id,
                                "pause_requested": next_workflow.pause_requested,
                                "reason": reason,
                            }),
                            now,
                        )
                        .await?;
                        recovered += 1;
                    }
                    Ok(recovered)
                })
            })
            .await
            .map_err(transaction_error);
        match result {
            Ok(recovered) => {
                if recovered > 0 {
                    self.notify_event_listeners();
                }
                Ok(recovered)
            }
            Err(error) => Err(error),
        }
    }

    /// 审核 Agent 没有可恢复的网络会话时，把它的子 Run 收束为失败并把 Review 重新
    /// 打开；父写作 Run 继续停在 `waiting_review`，用户可以重新委托或直接给出人工
    /// 结论。启动恢复与退出收尾共用这条路径，只有原因文案不同。
    pub async fn interrupt_active_review_delegations(
        &self,
        source: InterruptionSource,
    ) -> Result<usize, StoreError> {
        let reason = source.reason();
        let now = utc_now_millis();
        let result = self
            .database
            .transaction::<_, _, StoreError>(|transaction| {
                Box::pin(async move {
                    let candidates = review::Entity::find()
                        .filter(review::Column::Status.eq("delegated"))
                        .all(transaction)
                        .await?;
                    let mut recovered = 0_usize;
                    for review_model in candidates {
                        let review_id: ReviewId = review_model.id.into();
                        let attempt = delegated_review_attempt(transaction, review_id).await?;
                        let workflow =
                            workflow_for_version(transaction, attempt.version_id).await?;
                        if workflow.execution != ExecutionStatus::WaitingReview
                            || !review_stage_matches(&workflow, attempt.kind)
                        {
                            return Err(StoreError::CorruptRecord(
                                "审核中断收尾发现委托审核与父工作流状态不一致".to_owned(),
                            ));
                        }
                        let child_run = run::Entity::find_by_id(db_id(attempt.run_id))
                            .filter(run::Column::Kind.eq("review"))
                            .one(transaction)
                            .await?
                            .ok_or(StoreError::RunNotFound)?;
                        if !matches!(child_run.status.as_str(), "queued" | "running") {
                            return Err(StoreError::CorruptRecord(
                                "审核中断收尾发现委托审核子 Run 不在可收束状态".to_owned(),
                            ));
                        }
                        let outstanding_requests = model_request::Entity::find()
                            .filter(model_request::Column::RunId.eq(child_run.id))
                            .filter(model_request::Column::Status.is_in(["prepared", "dispatched"]))
                            .all(transaction)
                            .await?;
                        for request in outstanding_requests {
                            let (status, message) = match request.status.as_str() {
                                "prepared" => (
                                    "cancelled",
                                    format!("{}前审核请求尚未发送，已作废", source.prefix()),
                                ),
                                "dispatched" => (
                                    "unknown",
                                    format!("{}时审核请求是否被模型服务接受未知", source.prefix()),
                                ),
                                _ => {
                                    return Err(StoreError::CorruptRecord(
                                        "审核中断收尾查询返回了非在途审核模型请求".to_owned(),
                                    ));
                                }
                            };
                            let mut active_request = request.into_active_model();
                            active_request.status = Set(status.to_owned());
                            active_request.error_json = Set(Some(serde_json::to_string(&json!({
                                "kind": reason,
                                "message": message,
                            }))?));
                            active_request.updated_at = Set(now);
                            active_request.completed_at = Set(Some(now));
                            active_request.update(transaction).await?;
                        }
                        let mut active_run = child_run.into_active_model();
                        active_run.status = Set("failed".to_owned());
                        active_run.ended_at = Set(Some(now));
                        active_run.updated_at = Set(now);
                        active_run.update(transaction).await?;
                        let mut active_review = review_model.into_active_model();
                        active_review.status = Set("open".to_owned());
                        active_review.decision_source = Set(None);
                        active_review.passed = Set(None);
                        active_review.confidence = Set(None);
                        active_review.issues_json = Set(None);
                        active_review.updated_at = Set(now);
                        active_review.update(transaction).await?;
                        insert_event(
                            transaction,
                            attempt.version_id,
                            "review.changed",
                            review_id,
                            workflow.state_revision,
                            json!({
                                "status": "open",
                                "failed_attempt_id": attempt.run_id,
                                "reason": reason,
                            }),
                            now,
                        )
                        .await?;
                        insert_event(
                            transaction,
                            attempt.version_id,
                            "run.changed",
                            attempt.run_id,
                            workflow.state_revision,
                            json!({
                                "kind": "review",
                                "status": "failed",
                                "review_id": review_id,
                                "reason": reason,
                            }),
                            now,
                        )
                        .await?;
                        recovered += 1;
                    }
                    Ok(recovered)
                })
            })
            .await
            .map_err(transaction_error);
        match result {
            Ok(recovered) => {
                if recovered > 0 {
                    self.notify_event_listeners();
                }
                Ok(recovered)
            }
            Err(error) => Err(error),
        }
    }
}

impl DelegatedReviewKind {
    const fn stable_name(self) -> &'static str {
        match self {
            Self::Preparation(kind) => match kind {
                PreparationArtifactKind::Background => "preparation.background",
                PreparationArtifactKind::Research => "preparation.research",
                PreparationArtifactKind::Characters => "preparation.characters",
                PreparationArtifactKind::Outline => "preparation.outline",
            },
            Self::ChapterOutline => "chapter.outline",
            Self::ChapterEdit => "chapter.edit",
            Self::ChapterFinal => "chapter.final",
        }
    }
}

struct DelegatableReviewTarget {
    parent_run_id: RunId,
    project_id: ProjectId,
    version_id: VersionId,
    target_revision_id: Option<RevisionId>,
    dependency_revision_ids: BTreeSet<RevisionId>,
    kind: DelegatedReviewKind,
    workflow_revision: i64,
}

/// 一次委托尝试：`run_id` 是审核子 Run，`version_id` 是它归属的稿本版本。
///
/// 协议纠正（`protocol_correction`）与取消/失败共用这一个读取入口，所以字段对
/// crate 内可见。
pub(crate) struct DelegatedReviewAttempt {
    pub(crate) run_id: RunId,
    pub(crate) version_id: VersionId,
    kind: DelegatedReviewKind,
}

fn validate_delegation_notes(notes: &str) -> Result<(), StoreError> {
    if notes.chars().count() > MAX_REVIEW_ISSUES_LEN {
        return Err(StoreError::InvalidChapterState(format!(
            "审核关注点不能超过 {MAX_REVIEW_ISSUES_LEN} 个字符"
        )));
    }
    Ok(())
}

/// 委托审核时固化用户附带的引用。
///
/// 引用在委托时就必须能逐字复核，之后目标修订变化会让整个 attempt 失效，所以审核
/// 子 Run 读取时不再回查：准备产物可以引用本稿本内的任意文档，章节审核只能引用本
/// 章文档；待审候选引用只对逐次编辑审核有意义。
///
/// # Errors
///
/// 引用不属于审核所在稿本、文档与修订不匹配、章节不一致或原文与 quote 不一致时
/// 返回 [`StoreError::InvalidCitation`]。
async fn validate_delegation_citations(
    transaction: &DatabaseTransaction,
    citations: &[FeedbackCitation],
    version_id: VersionId,
    review_id: ReviewId,
    kind: DelegatedReviewKind,
) -> Result<(), StoreError> {
    if citations.is_empty() {
        return Ok(());
    }
    let required_chapter_id = match kind {
        DelegatedReviewKind::Preparation(_) => None,
        DelegatedReviewKind::ChapterOutline => Some(
            chapter_outline_review_target(transaction, review_id)
                .await?
                .chapter_id,
        ),
        DelegatedReviewKind::ChapterFinal => Some(
            chapter_final_review_target(transaction, review_id)
                .await?
                .chapter_id,
        ),
        DelegatedReviewKind::ChapterEdit => Some(
            chapter_edit_review_target(transaction, review_id)
                .await?
                .chapter_id,
        ),
    };
    for citation in citations {
        match citation {
            FeedbackCitation::Document {
                document_id,
                revision_id,
                range,
                chapter_id,
            } => {
                validate_document_citation(
                    transaction,
                    *document_id,
                    *revision_id,
                    range,
                    *chapter_id,
                    version_id,
                    required_chapter_id,
                )
                .await?;
            }
            FeedbackCitation::Candidate {
                tool_call_id,
                candidate_hash,
                candidate_range,
            } => {
                if kind != DelegatedReviewKind::ChapterEdit {
                    return Err(StoreError::InvalidCitation(
                        "只有正文编辑审核可以引用待审候选".to_owned(),
                    ));
                }
                let target = chapter_edit_review_target(transaction, review_id).await?;
                if *tool_call_id != target.proposal.tool_call_id {
                    return Err(StoreError::InvalidCitation(
                        "候选引用必须指向本次待审编辑".to_owned(),
                    ));
                }
                if candidate_hash != &target.proposal.candidate_hash {
                    return Err(StoreError::InvalidCitation(
                        "候选正文已变化，请重新选择要引用的文本".to_owned(),
                    ));
                }
                validate_text_citation_range(&target.proposal.candidate_content, candidate_range)?;
            }
        }
    }
    Ok(())
}

async fn delegatable_review_target(
    transaction: &DatabaseTransaction,
    review_id: ReviewId,
    expected_state_revision: i64,
) -> Result<DelegatableReviewTarget, StoreError> {
    let review_model = review::Entity::find_by_id(db_id(review_id))
        .one(transaction)
        .await?
        .ok_or(StoreError::ReviewNotFound)?;
    if review_status_from_db(review_model.status.clone())? != ReviewStatus::Open {
        return Err(StoreError::ReviewAlreadyResolved);
    }

    let (
        version_id,
        target_revision_id,
        dependency_revision_ids,
        kind,
        expected_stage,
        chapter_id,
        review_parent_run_id,
    ) = match review_model.review_type.as_str() {
        "preparation" => {
            let target = preparation_review_target(transaction, review_id).await?;
            (
                target.version_id,
                Some(target.target_revision_id),
                serde_json::from_str::<Vec<String>>(&review_model.dependencies_json)?
                    .into_iter()
                    .map(|id| parse_id(&id, "preparation.dependencies_json"))
                    .collect::<Result<BTreeSet<_>, _>>()?,
                DelegatedReviewKind::Preparation(target.kind),
                WorkflowStage::PreparingReviewing,
                None,
                None,
            )
        }
        "chapter_outline" => {
            let target = chapter_outline_review_target(transaction, review_id).await?;
            (
                target.version_id,
                Some(target.target_revision_id),
                target.dependency_revision_ids,
                DelegatedReviewKind::ChapterOutline,
                WorkflowStage::WritingOutlineReviewing,
                Some(target.chapter_id),
                None,
            )
        }
        "chapter_final" => {
            let target = chapter_final_review_target(transaction, review_id).await?;
            (
                target.version_id,
                Some(target.target_revision_id),
                target.dependency_revision_ids,
                DelegatedReviewKind::ChapterFinal,
                WorkflowStage::WritingChapterReviewing,
                Some(target.chapter_id),
                None,
            )
        }
        "chapter_edit" => {
            let target = chapter_edit_review_target(transaction, review_id).await?;
            (
                target.version_id,
                target.base_revision_id,
                target.dependency_revision_ids,
                DelegatedReviewKind::ChapterEdit,
                WorkflowStage::WritingDrafting,
                Some(target.chapter_id),
                Some(target.parent_run_id),
            )
        }
        review_type => {
            return Err(StoreError::CorruptRecord(format!(
                "未知审核类型：{review_type}"
            )));
        }
    };
    let workflow = workflow_for_version(transaction, version_id).await?;
    ensure_expected_state_revision(&workflow, expected_state_revision)?;
    if workflow.stage != expected_stage || workflow.execution != ExecutionStatus::WaitingReview {
        return Err(StoreError::InvalidChapterState(
            "当前工作流不等待该审核，不能委托".to_owned(),
        ));
    }
    if workflow.active_chapter_id != chapter_id {
        return Err(StoreError::InvalidChapterState(
            "审核目标不是当前活动章节，不能委托".to_owned(),
        ));
    }
    validate_review_target_is_current(
        transaction,
        review_id,
        kind,
        target_revision_id,
        &dependency_revision_ids,
    )
    .await?;
    let parent_run_id = workflow
        .active_run_id
        .ok_or_else(|| StoreError::CorruptRecord("等待审核的工作流缺少活跃写作 Run".to_owned()))?;
    let parent_run = run::Entity::find_by_id(db_id(parent_run_id))
        .one(transaction)
        .await?
        .ok_or(StoreError::RunNotFound)?;
    if parent_run.parent_run_id.is_some()
        || parent_run.version_id != db_id(version_id)
        || parent_run.status != "waiting"
        || !parent_run_matches_review_kind(&parent_run.kind, kind)
    {
        return Err(StoreError::InvalidChapterState(
            "当前审核没有可委托的等待中写作 Run".to_owned(),
        ));
    }
    if review_parent_run_id
        .is_some_and(|review_parent_run_id| review_parent_run_id != parent_run_id)
    {
        return Err(StoreError::CorruptRecord(
            "正文编辑审核的候选不属于当前等待中的正文 Run".to_owned(),
        ));
    }
    Ok(DelegatableReviewTarget {
        parent_run_id,
        project_id: parent_run.project_id.into(),
        version_id,
        target_revision_id,
        dependency_revision_ids,
        kind,
        workflow_revision: workflow.state_revision,
    })
}

pub(crate) async fn delegated_review_attempt(
    transaction: &DatabaseTransaction,
    review_id: ReviewId,
) -> Result<DelegatedReviewAttempt, StoreError> {
    let review_model = review::Entity::find_by_id(db_id(review_id))
        .filter(review::Entity::COLUMN.status.eq("delegated"))
        .one(transaction)
        .await?
        .ok_or(StoreError::ReviewAlreadyResolved)?;
    let run_id = review_model
        .attempt_id
        .as_deref()
        .map(|id| parse_id(id, "review.attempt_id"))
        .transpose()?
        .ok_or_else(|| StoreError::CorruptRecord("委托审核缺少子 Run 标识".to_owned()))?;
    let kind = match review_model.review_type.as_str() {
        "preparation" => DelegatedReviewKind::Preparation(
            preparation_review_target(transaction, review_id)
                .await?
                .kind,
        ),
        "chapter_outline" => DelegatedReviewKind::ChapterOutline,
        "chapter_edit" => DelegatedReviewKind::ChapterEdit,
        "chapter_final" => DelegatedReviewKind::ChapterFinal,
        review_type => {
            return Err(StoreError::CorruptRecord(format!(
                "未知审核类型：{review_type}"
            )));
        }
    };
    let child_run = run::Entity::find_by_id(db_id(run_id))
        .one(transaction)
        .await?
        .ok_or(StoreError::RunNotFound)?;
    if child_run.parent_run_id.is_none()
        || child_run.kind != "review"
        || child_run.version_id != review_model.version_id
    {
        return Err(StoreError::CorruptRecord(
            "审核 attempt 没有对应的审核 Agent 子 Run".to_owned(),
        ));
    }
    Ok(DelegatedReviewAttempt {
        run_id,
        version_id: review_model.version_id.into(),
        kind,
    })
}

/// 相同审核子 Run 的成功回调可能因网络重试再次到达。审核结论已经在首次事务中
/// 伴随父工作流和折叠一起提交时，返回同一快照即可；绝不能重新应用一次决定。
async fn replayed_delegated_review_result(
    transaction: &DatabaseTransaction,
    review_id: ReviewId,
    run_id: RunId,
    request_id: Uuid,
) -> Result<Option<PreparationSnapshot>, StoreError> {
    let review_model = review::Entity::find_by_id(db_id(review_id))
        .one(transaction)
        .await?
        .ok_or(StoreError::ReviewNotFound)?;
    let run_id_string = run_id.to_string();
    if !matches!(review_model.status.as_str(), "approved" | "rejected")
        || review_model.attempt_id.as_deref() != Some(run_id_string.as_str())
    {
        return Ok(None);
    }
    let request = model_request::Entity::find_by_id(db_id(request_id))
        .filter(model_request::Entity::COLUMN.run_id.eq(db_id(run_id)))
        .one(transaction)
        .await?;
    if request.as_ref().map(|request| request.status.as_str()) != Some("succeeded") {
        return Ok(None);
    }
    preparation_snapshot(transaction, review_model.version_id.into())
        .await
        .map(Some)
}

fn review_stage_matches(workflow: &WorkflowSnapshot, kind: DelegatedReviewKind) -> bool {
    match kind {
        DelegatedReviewKind::Preparation(_) => workflow.stage == WorkflowStage::PreparingReviewing,
        DelegatedReviewKind::ChapterOutline => {
            workflow.stage == WorkflowStage::WritingOutlineReviewing
        }
        DelegatedReviewKind::ChapterEdit => workflow.stage == WorkflowStage::WritingDrafting,
        DelegatedReviewKind::ChapterFinal => {
            workflow.stage == WorkflowStage::WritingChapterReviewing
        }
    }
}

fn parent_run_matches_review_kind(parent_run_kind: &str, kind: DelegatedReviewKind) -> bool {
    matches!(
        (parent_run_kind, kind),
        ("preparation", DelegatedReviewKind::Preparation(_))
            | ("chapter_outline", DelegatedReviewKind::ChapterOutline)
            | (
                "chapter_body",
                DelegatedReviewKind::ChapterEdit | DelegatedReviewKind::ChapterFinal
            )
    )
}

async fn validate_review_target_is_current(
    transaction: &DatabaseTransaction,
    review_id: ReviewId,
    kind: DelegatedReviewKind,
    target_revision_id: Option<RevisionId>,
    dependency_revision_ids: &BTreeSet<RevisionId>,
) -> Result<(), StoreError> {
    match kind {
        DelegatedReviewKind::Preparation(_) => {
            let target = preparation_review_target(transaction, review_id).await?;
            if Some(target.current_revision_id) != target_revision_id {
                return Err(StoreError::InvalidPreparationState(
                    "审核目标已不是当前修订，不能委托".to_owned(),
                ));
            }
        }
        DelegatedReviewKind::ChapterOutline => {
            let target = chapter_outline_review_target(transaction, review_id).await?;
            let target_revision_id = target_revision_id.ok_or_else(|| {
                StoreError::CorruptRecord("章节细纲委托审核缺少目标修订".to_owned())
            })?;
            if target.current_revision_id != target_revision_id {
                return Err(StoreError::InvalidChapterState(
                    "审核目标已不是当前细纲修订，不能委托".to_owned(),
                ));
            }
            let expected = chapter_outline_dependency_revision_ids(
                transaction,
                target.version_id,
                target.chapter_id,
            )
            .await?;
            let actual = revision_dependency_ids(transaction, target_revision_id).await?;
            if &expected != dependency_revision_ids || actual != expected {
                return Err(StoreError::InvalidChapterState(
                    "细纲审核依赖已变化，不能委托".to_owned(),
                ));
            }
        }
        DelegatedReviewKind::ChapterFinal => {
            let target = chapter_final_review_target(transaction, review_id).await?;
            let target_revision_id = target_revision_id
                .ok_or_else(|| StoreError::CorruptRecord("整章委托审核缺少目标修订".to_owned()))?;
            if target.current_revision_id != target_revision_id {
                return Err(StoreError::InvalidChapterState(
                    "审核目标已不是当前正文修订，不能委托".to_owned(),
                ));
            }
            let outline_revision_id = chapter_body_outline_dependency_revision_id(
                transaction,
                target.version_id,
                target.chapter_id,
            )
            .await?;
            let actual = revision_dependency_ids(transaction, target_revision_id).await?;
            if dependency_revision_ids != &actual || !actual.contains(&outline_revision_id) {
                return Err(StoreError::InvalidChapterState(
                    "整章审核依赖已变化，不能委托".to_owned(),
                ));
            }
        }
        DelegatedReviewKind::ChapterEdit => {
            let target = chapter_edit_review_target(transaction, review_id).await?;
            if target.base_revision_id != target_revision_id
                || target.dependency_revision_ids != *dependency_revision_ids
            {
                return Err(StoreError::InvalidChapterState(
                    "正文编辑审核目标或依赖已变化，不能委托".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

async fn insert_delegated_review_run(
    transaction: &DatabaseTransaction,
    run_id: RunId,
    parent_run_id: RunId,
    project_id: ProjectId,
    version_id: VersionId,
    review_id: ReviewId,
    target_revision_id: Option<RevisionId>,
    dependency_revision_ids: &BTreeSet<RevisionId>,
    kind: DelegatedReviewKind,
    now: i64,
) -> Result<(), StoreError> {
    let checkpoint = json!({
        "review_id": review_id,
        "target_revision_id": target_revision_id,
        "dependency_revision_ids": dependency_revision_ids.iter().map(ToString::to_string).collect::<Vec<_>>(),
    });
    run::ActiveModel {
        id: Set(db_id(run_id)),
        project_id: Set(db_id(project_id)),
        version_id: Set(db_id(version_id)),
        kind: Set("review".to_owned()),
        parent_run_id: Set(Some(parent_run_id.to_string())),
        stage: Set(format!("review.{}", kind.stable_name())),
        status: Set("queued".to_owned()),
        resume_count: Set(0),
        request_count: Set(0),
        // 一次审核只该提交一次结论，但协议允许模型在被拒之后重提：连续协议错误达到
        // 上限才停止，因此子 Run 最多发出这么多轮请求。给少于这个数的额度会让
        // 「可纠正的提交」在重排时撞上额度耗尽，反而变成不可纠正的失败。
        request_allowance_end: Set(MAX_CONSECUTIVE_PROTOCOL_ERRORS),
        consecutive_protocol_errors: Set(0),
        checkpoint_json: Set(serde_json::to_string(&checkpoint)?),
        quota_scope_id: Set(None),
        started_at: Set(now),
        ended_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(transaction)
    .await?;
    Ok(())
}

/// 把审核子 Run 标成失败：它不再接受任何回调，也不再发请求。
///
/// 只改字段、不落库：调用方在同一次写入里补上本轮自己的字段（例如被拒回合的连续
/// 协议错误计数）。计数与失败状态必须落在同一次写入里，否则行里会留下上一轮的数字，
/// 与同一事务里刚发出的事件互相矛盾。取消/失败尝试与被拒回合达到上限共用这一处。
pub(crate) fn fail_delegated_review_child_run(
    mut child_run: run::ActiveModel,
    now: i64,
) -> run::ActiveModel {
    child_run.status = Set("failed".to_owned());
    child_run.ended_at = Set(Some(now));
    child_run.updated_at = Set(now);
    child_run
}

/// 重新开放被失败的委托占住的 Review，并广播两个状态事件。
///
/// 只有仍由同一个 `attempt_id` 占有的 Review 可以被重开；已经被取消或被新委托
/// 替换时返回 [`StoreError::ReviewAlreadyResolved`]。
pub(crate) async fn reopen_delegated_review(
    transaction: &DatabaseTransaction,
    review_id: ReviewId,
    run_id: RunId,
    version_id: VersionId,
    failure: &Value,
    now: i64,
) -> Result<(), StoreError> {
    let review_model = review::Entity::find_by_id(db_id(review_id))
        .filter(review::Entity::COLUMN.status.eq("delegated"))
        .filter(review::Entity::COLUMN.attempt_id.eq(run_id.to_string()))
        .one(transaction)
        .await?
        .ok_or(StoreError::ReviewAlreadyResolved)?;
    let mut active_review = review_model.into_active_model();
    active_review.status = Set("open".to_owned());
    active_review.decision_source = Set(None);
    active_review.passed = Set(None);
    active_review.confidence = Set(None);
    active_review.issues_json = Set(None);
    active_review.updated_at = Set(now);
    active_review.update(transaction).await?;
    let workflow = workflow_for_version(transaction, version_id).await?;
    insert_event(
        transaction,
        version_id,
        "review.changed",
        review_id,
        workflow.state_revision,
        json!({
            "status": "open",
            "failed_attempt_id": run_id,
            "error": failure,
        }),
        now,
    )
    .await?;
    insert_event(
        transaction,
        version_id,
        "run.changed",
        run_id,
        workflow.state_revision,
        json!({
            "kind": "review",
            "status": "failed",
            "review_id": review_id,
        }),
        now,
    )
    .await?;
    Ok(())
}

async fn build_delegated_review_task(
    transaction: &DatabaseTransaction,
    review_id: ReviewId,
    run_id: RunId,
) -> Result<DelegatedReviewTask, StoreError> {
    let attempt = delegated_review_attempt(transaction, review_id).await?;
    if attempt.run_id != run_id {
        return Err(StoreError::InvalidModelRequestState(
            "审核 Agent 子 Run 已被新的委托尝试替换".to_owned(),
        ));
    }
    let workflow = workflow_for_version(transaction, attempt.version_id).await?;
    if workflow.execution != ExecutionStatus::WaitingReview
        || !review_stage_matches(&workflow, attempt.kind)
    {
        return Err(StoreError::InvalidModelRequestState(
            "当前工作流不再接受该审核 Agent 请求".to_owned(),
        ));
    }
    let review_model = review::Entity::find_by_id(db_id(review_id))
        .filter(review::Entity::COLUMN.status.eq("delegated"))
        .filter(review::Entity::COLUMN.attempt_id.eq(run_id.to_string()))
        .one(transaction)
        .await?
        .ok_or(StoreError::ReviewAlreadyResolved)?;
    let user_notes = review_model
        .user_input_json
        .as_deref()
        .map(delegated_review_notes)
        .transpose()?
        .unwrap_or_default();
    let citations = review_model
        .user_input_json
        .as_deref()
        .map(delegated_review_citations)
        .transpose()?
        .unwrap_or_default();
    let target_revision_id = review_model
        .target_revision_id
        .as_deref()
        .map(|id| parse_id(id, "review.target_revision_id"))
        .transpose()?;
    let dependencies = serde_json::from_str::<Vec<String>>(&review_model.dependencies_json)?
        .into_iter()
        .map(|id| parse_id(&id, "review.dependencies_json"))
        .collect::<Result<BTreeSet<_>, _>>()?;
    validate_review_target_is_current(
        transaction,
        review_id,
        attempt.kind,
        target_revision_id,
        &dependencies,
    )
    .await?;
    let target_content = match attempt.kind {
        DelegatedReviewKind::ChapterEdit => {
            chapter_edit_review_target(transaction, review_id)
                .await?
                .proposal
                .candidate_content
        }
        _ => {
            revision_content(
                transaction,
                target_revision_id
                    .ok_or_else(|| StoreError::CorruptRecord("委托审核缺少目标修订".to_owned()))?,
                "审核目标修订",
            )
            .await?
        }
    };
    let mut task = DelegatedReviewTask {
        review_id,
        run_id,
        version_id: attempt.version_id,
        kind: attempt.kind,
        target_content,
        novel_brief: String::new(),
        approved_context: String::new(),
        chapter_context: String::new(),
        base_content: String::new(),
        chapter_outline: String::new(),
        user_notes,
        citations,
    };
    match task.kind {
        DelegatedReviewKind::Preparation(kind) => {
            if matches!(
                kind,
                PreparationArtifactKind::Background | PreparationArtifactKind::Research
            ) {
                task.novel_brief = current_novel_brief(transaction, task.version_id)
                    .await?
                    .content
                    .ok_or(StoreError::NovelBriefRequired)?;
            } else {
                task.approved_context =
                    approved_preparation_context(transaction, task.version_id, kind).await?;
            }
        }
        DelegatedReviewKind::ChapterOutline => {
            let target = chapter_outline_review_target(transaction, review_id).await?;
            task.chapter_context =
                delegated_chapter_context(transaction, task.version_id, target.chapter_id).await?;
        }
        DelegatedReviewKind::ChapterEdit => {
            let target = chapter_edit_review_target(transaction, review_id).await?;
            task.base_content = match target.base_revision_id {
                Some(revision_id) => {
                    revision_content(transaction, revision_id, "正文编辑基础修订").await?
                }
                None => String::new(),
            };
        }
        DelegatedReviewKind::ChapterFinal => {
            let target = chapter_final_review_target(transaction, review_id).await?;
            let outline = current_chapter_outline(transaction, target.chapter_id).await?;
            if outline.review.as_ref().map(|review| &review.status) != Some(&ReviewStatus::Approved)
            {
                return Err(StoreError::InvalidChapterState(
                    "整章审核前缺少已通过的章节细纲".to_owned(),
                ));
            }
            task.chapter_outline = outline
                .content
                .ok_or_else(|| StoreError::CorruptRecord("已通过章节细纲缺少内容".to_owned()))?;
        }
    }
    Ok(task)
}

fn delegated_review_notes(value: &str) -> Result<String, StoreError> {
    let value: Value = serde_json::from_str(value)?;
    Ok(value
        .get("notes")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned())
}

fn delegated_review_citations(value: &str) -> Result<Vec<FeedbackCitation>, StoreError> {
    let value: Value = serde_json::from_str(value)?;
    match value.get("citations") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(citations) => Ok(serde_json::from_value(citations.clone())?),
    }
}

async fn revision_content<C: ConnectionTrait>(
    connection: &C,
    revision_id: RevisionId,
    label: &str,
) -> Result<String, StoreError> {
    document_revision::Entity::find_by_id(db_id(revision_id))
        .one(connection)
        .await?
        .map(|revision| revision.content)
        .ok_or_else(|| StoreError::CorruptRecord(format!("{label}不存在")))
}

async fn approved_preparation_context(
    transaction: &DatabaseTransaction,
    version_id: VersionId,
    target_kind: PreparationArtifactKind,
) -> Result<String, StoreError> {
    let artifacts = current_preparation_artifacts(transaction, version_id).await?;
    Ok(artifacts
        .into_iter()
        .filter(|artifact| artifact.kind != target_kind)
        .filter(|artifact| {
            artifact.review.as_ref().map(|review| review.status.clone())
                == Some(ReviewStatus::Approved)
        })
        .filter_map(|artifact| {
            artifact
                .content
                .map(|content| format!("{}：{content}", artifact.kind.label()))
        })
        .collect::<Vec<_>>()
        .join("\n\n"))
}

async fn delegated_chapter_context(
    transaction: &DatabaseTransaction,
    version_id: VersionId,
    chapter_id: ChapterId,
) -> Result<String, StoreError> {
    let chapter_model = chapter::Entity::find_by_id(db_id(chapter_id))
        .filter(chapter::Entity::COLUMN.version_id.eq(db_id(version_id)))
        .filter(chapter::Entity::COLUMN.deleted_at.is_null())
        .one(transaction)
        .await?
        .ok_or(StoreError::ChapterNotFound)?;
    let guidance = current_chapter_guidance(transaction, chapter_id)
        .await?
        .content
        .unwrap_or_default();
    let plan_or_extra = match chapter_model.kind.as_str() {
        "main" => {
            let plan_item_id =
                parse_optional_id(chapter_model.plan_item_id, "chapter.plan_item_id")?.ok_or_else(
                    || StoreError::CorruptRecord("正文章节缺少计划项标识".to_owned()),
                )?;
            let plan = current_preparation_artifacts(transaction, version_id)
                .await?
                .into_iter()
                .find(|artifact| artifact.kind == PreparationArtifactKind::Outline)
                .and_then(|artifact| artifact.chapter_plan)
                .ok_or_else(|| StoreError::CorruptRecord("已通过粗纲缺少章节计划".to_owned()))?;
            let item = plan
                .items
                .into_iter()
                .find(|item| item.plan_item_id == plan_item_id)
                .ok_or_else(|| StoreError::CorruptRecord("章节计划缺少当前章节".to_owned()))?;
            format!("第{}章：{}\n{}", item.order_index, item.title, item.goal)
        }
        "extra" => format!("番外：{}", chapter_model.title),
        kind => {
            return Err(StoreError::CorruptRecord(format!(
                "章节使用了未知类型：{kind}"
            )));
        }
    };
    Ok(format!("{plan_or_extra}\n\n用户指导：{guidance}"))
}

async fn chapter_and_guidance_document<C: ConnectionTrait>(
    connection: &C,
    chapter_id: ChapterId,
) -> Result<(chapter::Model, document::Model), StoreError> {
    chapter_and_document(connection, chapter_id, CHAPTER_GUIDANCE_ROLE, "章节指导").await
}

async fn chapter_and_document<C: ConnectionTrait>(
    connection: &C,
    chapter_id: ChapterId,
    role: &str,
    label: &str,
) -> Result<(chapter::Model, document::Model), StoreError> {
    let chapter_model = chapter::Entity::find_by_id(db_id(chapter_id))
        .filter(chapter::Entity::COLUMN.deleted_at.is_null())
        .one(connection)
        .await?
        .ok_or(StoreError::ChapterNotFound)?;
    let chapter_document = document::Entity::find()
        .filter(
            document::Entity::COLUMN
                .version_id
                .eq(chapter_model.version_id),
        )
        .filter(
            document::Entity::COLUMN
                .chapter_id
                .eq(chapter_id.to_string()),
        )
        .filter(document::Entity::COLUMN.role.eq(role))
        .filter(document::Entity::COLUMN.deleted_at.is_null())
        .one(connection)
        .await?
        .ok_or_else(|| StoreError::CorruptRecord(format!("章节缺少{label}文档槽")))?;
    Ok((chapter_model, chapter_document))
}

/// 按已完成章节的同类逻辑顺序读取下一章。
///
/// 不在这里筛选 `planned`：若目录里最近的下一章已经被异常地提前开始或通过，
/// 调用方需要看见该不变量被破坏，而不是悄悄跳过它继续写更后面的章节。
async fn next_chapter_after<C: ConnectionTrait>(
    connection: &C,
    version_id: VersionId,
    completed_chapter: &chapter::Model,
) -> Result<Option<chapter::Model>, StoreError> {
    chapter::Entity::find()
        .filter(chapter::Entity::COLUMN.version_id.eq(db_id(version_id)))
        .filter(
            chapter::Entity::COLUMN
                .kind
                .eq(completed_chapter.kind.clone()),
        )
        .filter(
            chapter::Entity::COLUMN
                .order_index
                .gt(completed_chapter.order_index),
        )
        .filter(chapter::Entity::COLUMN.deleted_at.is_null())
        .order_by_asc(chapter::Entity::COLUMN.order_index)
        .order_by_asc(chapter::Entity::COLUMN.id)
        .one(connection)
        .await
        .map_err(StoreError::from)
}

async fn all_main_chapters_are_approved<C: ConnectionTrait>(
    connection: &C,
    version_id: VersionId,
) -> Result<bool, StoreError> {
    let chapters = chapter::Entity::find()
        .filter(chapter::Entity::COLUMN.version_id.eq(db_id(version_id)))
        .filter(chapter::Entity::COLUMN.kind.eq("main"))
        .filter(chapter::Entity::COLUMN.deleted_at.is_null())
        .all(connection)
        .await?;
    Ok(!chapters.is_empty() && chapters.iter().all(|chapter| chapter.status == "approved"))
}

pub(crate) async fn open_review_exists<C: ConnectionTrait>(
    connection: &C,
    version_id: VersionId,
) -> Result<bool, StoreError> {
    Ok(review::Entity::find()
        .filter(review::Entity::COLUMN.version_id.eq(db_id(version_id)))
        .filter(review::Entity::COLUMN.status.is_in(["open", "delegated"]))
        .one(connection)
        .await?
        .is_some())
}

pub(crate) async fn active_top_level_run_exists<C: ConnectionTrait>(
    connection: &C,
    version_id: VersionId,
) -> Result<bool, StoreError> {
    Ok(run::Entity::find()
        .filter(run::Entity::COLUMN.version_id.eq(db_id(version_id)))
        .filter(run::Entity::COLUMN.parent_run_id.is_null())
        .filter(
            run::Entity::COLUMN
                .kind
                .is_in(["preparation", "chapter_outline", "chapter_body"]),
        )
        .filter(run::Entity::COLUMN.status.is_in([
            "queued",
            "running",
            "waiting",
            "paused",
            "interrupted",
        ]))
        .one(connection)
        .await?
        .is_some())
}

/// 与 schema 中的两个 partial unique index 配套的业务错误。
///
/// 该预检给调用方稳定的 `RESOURCE_BUSY`，而数据库约束仍是并发竞争下最后的
/// 正确性保障。项目级检查使切换展示稿本不会绕过写作互斥。
async fn ensure_no_active_top_level_write_run<C: ConnectionTrait>(
    connection: &C,
    project_id: ProjectId,
) -> Result<(), StoreError> {
    let active_run = run::Entity::find()
        .filter(run::Entity::COLUMN.project_id.eq(db_id(project_id)))
        .filter(run::Entity::COLUMN.parent_run_id.is_null())
        .filter(
            run::Entity::COLUMN
                .kind
                .is_in(["preparation", "chapter_outline", "chapter_body"]),
        )
        .filter(run::Entity::COLUMN.status.is_in([
            "queued",
            "running",
            "waiting",
            "paused",
            "interrupted",
        ]))
        .one(connection)
        .await?;
    if let Some(active_run) = active_run {
        return Err(StoreError::ResourceBusy(format!(
            "Run {}（{}）尚未结束",
            active_run.id, active_run.kind
        )));
    }
    Ok(())
}

async fn current_chapter_guidance<C: ConnectionTrait>(
    connection: &C,
    chapter_id: ChapterId,
) -> Result<CurrentChapterGuidance, StoreError> {
    let (_, guidance_document) = chapter_and_guidance_document(connection, chapter_id).await?;
    let revision_id = parse_optional_id(
        guidance_document.current_revision_id.clone(),
        "chapter_guidance.current_revision_id",
    )?;
    let revision = match revision_id {
        Some(revision_id) => Some(
            document_revision::Entity::find_by_id(db_id(revision_id))
                .one(connection)
                .await?
                .ok_or_else(|| StoreError::CorruptRecord("章节指导当前修订不存在".to_owned()))?,
        ),
        None => None,
    };
    Ok(CurrentChapterGuidance {
        document_id: guidance_document.id.into(),
        revision_id,
        revision_no: revision.as_ref().map(|revision| revision.revision_no),
        content: revision.map(|revision| revision.content),
    })
}

async fn current_chapter_outline<C: ConnectionTrait>(
    connection: &C,
    chapter_id: ChapterId,
) -> Result<CurrentChapterOutline, StoreError> {
    let (_, outline_document) =
        chapter_and_document(connection, chapter_id, CHAPTER_OUTLINE_ROLE, "章节细纲").await?;
    let revision_id = parse_optional_id(
        outline_document.current_revision_id.clone(),
        "chapter_outline.current_revision_id",
    )?;
    let revision = match revision_id {
        Some(revision_id) => Some(
            document_revision::Entity::find_by_id(db_id(revision_id))
                .one(connection)
                .await?
                .ok_or_else(|| StoreError::CorruptRecord("章节细纲当前修订不存在".to_owned()))?,
        ),
        None => None,
    };
    let review = match revision_id {
        Some(revision_id) => review::Entity::find()
            .filter(
                review::Entity::COLUMN
                    .version_id
                    .eq(outline_document.version_id),
            )
            .filter(review::Entity::COLUMN.review_type.eq("chapter_outline"))
            .filter(
                review::Entity::COLUMN
                    .target_revision_id
                    .eq(revision_id.to_string()),
            )
            .order_by_desc(review::Entity::COLUMN.created_at)
            .order_by_desc(review::Entity::COLUMN.id)
            .one(connection)
            .await?
            .map(|model| chapter_outline_review_from_model(&model))
            .transpose()?,
        None => None,
    };
    Ok(CurrentChapterOutline {
        document_id: outline_document.id.into(),
        revision_id,
        revision_no: revision.as_ref().map(|revision| revision.revision_no),
        content: revision.map(|revision| revision.content),
        review,
    })
}

pub(crate) async fn chapter_outline_snapshot<C: ConnectionTrait>(
    connection: &C,
    chapter_id: ChapterId,
) -> Result<ChapterOutlineSnapshot, StoreError> {
    let (chapter_model, _) =
        chapter_and_document(connection, chapter_id, CHAPTER_OUTLINE_ROLE, "章节细纲").await?;
    let version_id: VersionId = chapter_model.version_id.into();
    let workflow = workflow_for_version(connection, version_id).await?;
    Ok(ChapterOutlineSnapshot {
        chapter_id,
        pending_question: pending_user_question(connection, &workflow).await?,
        workflow,
        guidance: current_chapter_guidance(connection, chapter_id).await?,
        outline: current_chapter_outline(connection, chapter_id).await?,
        context_fold: latest_chapter_outline_fold(connection, version_id, chapter_id).await?,
    })
}

async fn current_chapter_body<C: ConnectionTrait>(
    connection: &C,
    chapter_id: ChapterId,
) -> Result<CurrentChapterBody, StoreError> {
    let (_, body_document) =
        chapter_and_document(connection, chapter_id, CHAPTER_BODY_ROLE, "章节正文").await?;
    let revision_id = parse_optional_id(
        body_document.current_revision_id.clone(),
        "chapter_body.current_revision_id",
    )?;
    let revision = match revision_id {
        Some(revision_id) => Some(
            document_revision::Entity::find_by_id(db_id(revision_id))
                .one(connection)
                .await?
                .ok_or_else(|| StoreError::CorruptRecord("章节正文当前修订不存在".to_owned()))?,
        ),
        None => None,
    };
    let review = match revision_id {
        Some(revision_id) => review::Entity::find()
            .filter(
                review::Entity::COLUMN
                    .version_id
                    .eq(body_document.version_id),
            )
            .filter(review::Entity::COLUMN.review_type.eq("chapter_final"))
            .filter(
                review::Entity::COLUMN
                    .target_revision_id
                    .eq(revision_id.to_string()),
            )
            .order_by_desc(review::Entity::COLUMN.created_at)
            .order_by_desc(review::Entity::COLUMN.id)
            .one(connection)
            .await?
            .map(|model| chapter_final_review_from_model(&model))
            .transpose()?,
        None => None,
    };
    Ok(CurrentChapterBody {
        document_id: body_document.id.into(),
        revision_id,
        revision_no: revision.as_ref().map(|revision| revision.revision_no),
        content: revision.map(|revision| revision.content),
        review,
    })
}

async fn chapter_body_snapshot<C: ConnectionTrait>(
    connection: &C,
    chapter_id: ChapterId,
) -> Result<ChapterBodySnapshot, StoreError> {
    let (chapter_model, _) =
        chapter_and_document(connection, chapter_id, CHAPTER_BODY_ROLE, "章节正文").await?;
    let version_id: VersionId = chapter_model.version_id.into();
    let workflow = workflow_for_version(connection, version_id).await?;
    let held_tool = held_chapter_body_tool(connection, &workflow, chapter_id).await?;
    let held_submission = held_chapter_body_submission(connection, &workflow, chapter_id).await?;
    if workflow.active_chapter_id == Some(chapter_id)
        && workflow.stage == WorkflowStage::WritingDrafting
        && workflow.execution == ExecutionStatus::Paused
        && (held_tool.is_some() == held_submission.is_some())
    {
        return Err(StoreError::CorruptRecord(
            "正文暂停状态必须恰好保留一个工具调用或结构化提交".to_owned(),
        ));
    }
    let pending_proposal = pending_chapter_body_proposal(connection, &workflow, chapter_id).await?;
    let pending_proposal_review = match pending_proposal.as_ref() {
        Some(proposal) => {
            Some(pending_chapter_edit_review(connection, version_id, proposal.tool_call_id).await?)
        }
        None => None,
    };
    let last_proposal_review = if pending_proposal.is_none() {
        latest_chapter_edit_review(connection, version_id, workflow.active_run_id).await?
    } else {
        None
    };
    Ok(ChapterBodySnapshot {
        chapter_id,
        pending_proposal,
        pending_proposal_review,
        last_proposal_review,
        held_tool,
        held_submission,
        pending_question: pending_user_question(connection, &workflow).await?,
        pending_feedback: pending_chapter_body_feedback(connection, &workflow, chapter_id).await?,
        delivered_feedback: delivered_chapter_body_feedback(connection, &workflow, chapter_id)
            .await?,
        workflow,
        body: current_chapter_body(connection, chapter_id).await?,
        outline_context_fold: latest_chapter_outline_fold(connection, version_id, chapter_id)
            .await?,
        context_fold: latest_chapter_body_fold(connection, version_id, chapter_id).await?,
    })
}

async fn pending_user_question<C: ConnectionTrait>(
    connection: &C,
    workflow: &WorkflowSnapshot,
) -> Result<Option<PendingUserQuestion>, StoreError> {
    let Some(run_id) = workflow.active_run_id else {
        if workflow.execution == ExecutionStatus::WaitingAnswer {
            return Err(StoreError::CorruptRecord(
                "等待用户答案的工作流缺少活跃 Run".to_owned(),
            ));
        }
        return Ok(None);
    };
    let awaiting_tools = tool_call::Entity::find()
        .filter(tool_call::Entity::COLUMN.run_id.eq(db_id(run_id)))
        .filter(tool_call::Entity::COLUMN.status.eq("awaiting_answer"))
        .all(connection)
        .await?;
    let [awaiting_tool] = awaiting_tools.as_slice() else {
        if awaiting_tools.is_empty() && workflow.execution != ExecutionStatus::WaitingAnswer {
            return Ok(None);
        }
        return Err(StoreError::CorruptRecord(
            "等待用户答案状态与 ask_user 工具调用数量不一致".to_owned(),
        ));
    };
    if workflow.execution != ExecutionStatus::WaitingAnswer {
        return Err(StoreError::CorruptRecord(
            "待答 ask_user 工具调用与工作流执行状态不一致".to_owned(),
        ));
    }
    if awaiting_tool.name != "ask_user" || awaiting_tool.result_json.is_some() {
        return Err(StoreError::CorruptRecord(
            "待答工具调用不是完整的 ask_user 记录".to_owned(),
        ));
    }
    let question_model = question::Entity::find()
        .filter(question::Entity::COLUMN.tool_call_id.eq(awaiting_tool.id))
        .one(connection)
        .await?
        .ok_or_else(|| {
            StoreError::CorruptRecord("待答 ask_user 工具调用缺少问题记录".to_owned())
        })?;
    pending_user_question_from_model(&question_model).map(Some)
}

async fn pending_chapter_body_proposal<C: ConnectionTrait>(
    connection: &C,
    workflow: &WorkflowSnapshot,
    chapter_id: ChapterId,
) -> Result<Option<PendingChapterBodyProposal>, StoreError> {
    let Some(run_id) = workflow.active_run_id else {
        return Ok(None);
    };
    let proposal = tool_call::Entity::find()
        .filter(tool_call::Entity::COLUMN.run_id.eq(db_id(run_id)))
        .filter(tool_call::Entity::COLUMN.status.eq("awaiting_review"))
        .one(connection)
        .await?
        .map(|model| pending_chapter_body_proposal_from_tool_call(&model))
        .transpose()?;
    if proposal.is_some()
        && (workflow.active_chapter_id != Some(chapter_id)
            || workflow.stage != WorkflowStage::WritingDrafting
            || workflow.execution != ExecutionStatus::WaitingReview)
    {
        return Err(StoreError::CorruptRecord(
            "待审正文提案与工作流状态不一致".to_owned(),
        ));
    }
    if proposal.is_none()
        && workflow.active_chapter_id == Some(chapter_id)
        && workflow.stage == WorkflowStage::WritingDrafting
        && workflow.execution == ExecutionStatus::WaitingReview
    {
        return Err(StoreError::CorruptRecord(
            "正文工作流等待审核但缺少待审编辑提案".to_owned(),
        ));
    }
    Ok(proposal)
}

async fn pending_chapter_edit_review<C: ConnectionTrait>(
    connection: &C,
    version_id: VersionId,
    tool_call_id: Uuid,
) -> Result<ChapterEditReview, StoreError> {
    let review_model = review::Entity::find()
        .filter(review::Entity::COLUMN.version_id.eq(db_id(version_id)))
        .filter(review::Entity::COLUMN.review_type.eq("chapter_edit"))
        .filter(
            review::Entity::COLUMN
                .target_tool_call_id
                .eq(tool_call_id.to_string()),
        )
        .order_by_desc(review::Entity::COLUMN.created_at)
        .order_by_desc(review::Entity::COLUMN.id)
        .one(connection)
        .await?
        .ok_or_else(|| StoreError::CorruptRecord("待审正文编辑缺少统一审核记录".to_owned()))?;
    let review = chapter_edit_review_from_model(&review_model)?;
    if !matches!(review.status, ReviewStatus::Open | ReviewStatus::Delegated) {
        return Err(StoreError::CorruptRecord(
            "待审正文编辑关联的审核已有终态".to_owned(),
        ));
    }
    Ok(review)
}

async fn latest_chapter_edit_review<C: ConnectionTrait>(
    connection: &C,
    version_id: VersionId,
    parent_run_id: Option<RunId>,
) -> Result<Option<ChapterEditReview>, StoreError> {
    let Some(parent_run_id) = parent_run_id else {
        return Ok(None);
    };
    let target_tool_call_ids = tool_call::Entity::find()
        .filter(tool_call::Entity::COLUMN.run_id.eq(db_id(parent_run_id)))
        .all(connection)
        .await?
        .into_iter()
        .map(|tool_call| tool_call.id.to_string())
        .collect::<Vec<_>>();
    if target_tool_call_ids.is_empty() {
        return Ok(None);
    }
    review::Entity::find()
        .filter(review::Entity::COLUMN.version_id.eq(db_id(version_id)))
        .filter(review::Entity::COLUMN.review_type.eq("chapter_edit"))
        .filter(
            review::Entity::COLUMN
                .target_tool_call_id
                .is_in(target_tool_call_ids),
        )
        .filter(
            review::Entity::COLUMN
                .status
                .is_in(["approved", "rejected"]),
        )
        .order_by_desc(review::Entity::COLUMN.updated_at)
        .order_by_desc(review::Entity::COLUMN.id)
        .one(connection)
        .await?
        .map(|review| chapter_edit_review_from_model(&review))
        .transpose()
}

async fn held_chapter_body_tool<C: ConnectionTrait>(
    connection: &C,
    workflow: &WorkflowSnapshot,
    chapter_id: ChapterId,
) -> Result<Option<HeldChapterBodyTool>, StoreError> {
    let Some(run_id) = workflow.active_run_id else {
        return Ok(None);
    };
    let held = tool_call::Entity::find()
        .filter(tool_call::Entity::COLUMN.run_id.eq(db_id(run_id)))
        .filter(tool_call::Entity::COLUMN.status.eq("held"))
        .one(connection)
        .await?
        .map(|model| held_chapter_body_tool_payload_from_model(&model).map(|payload| payload.held))
        .transpose()?;
    if held.is_some()
        && (workflow.active_chapter_id != Some(chapter_id)
            || workflow.stage != WorkflowStage::WritingDrafting
            || workflow.execution != ExecutionStatus::Paused
            || !workflow.pause_requested)
    {
        return Err(StoreError::CorruptRecord(
            "已暂停正文工具调用与工作流状态不一致".to_owned(),
        ));
    }
    Ok(held)
}

async fn held_chapter_body_submission<C: ConnectionTrait>(
    connection: &C,
    workflow: &WorkflowSnapshot,
    chapter_id: ChapterId,
) -> Result<Option<HeldChapterBodySubmission>, StoreError> {
    let Some(run_id) = workflow.active_run_id else {
        return Ok(None);
    };
    let run_model = run::Entity::find_by_id(db_id(run_id))
        .one(connection)
        .await?
        .ok_or(StoreError::RunNotFound)?;
    if run_model.parent_run_id.is_some() || run_model.kind != "chapter_body" {
        return Ok(None);
    }
    let checkpoint: RunCheckpoint = serde_json::from_str(&run_model.checkpoint_json)?;
    if checkpoint.chapter_id != Some(chapter_id) {
        return Err(StoreError::CorruptRecord(
            "正文 Run 检查点与当前章节不匹配".to_owned(),
        ));
    }
    let held = checkpoint.held_chapter_body_submission;
    if let Some(held) = &held {
        if held.held.summary.trim().is_empty() {
            return Err(StoreError::CorruptRecord(
                "已暂停章节提交缺少摘要".to_owned(),
            ));
        }
        phase_submission_call_id(held.held.request_id, &held.assistant_message)?;
        if workflow.active_chapter_id != Some(chapter_id)
            || workflow.stage != WorkflowStage::WritingDrafting
            || workflow.execution != ExecutionStatus::Paused
            || !workflow.pause_requested
            || run_model.status != "paused"
        {
            return Err(StoreError::CorruptRecord(
                "已暂停章节提交与工作流状态不一致".to_owned(),
            ));
        }
    }
    Ok(held.map(|checkpoint| checkpoint.held))
}

async fn pending_chapter_body_feedback<C: ConnectionTrait>(
    connection: &C,
    workflow: &WorkflowSnapshot,
    chapter_id: ChapterId,
) -> Result<Vec<PendingChapterBodyFeedback>, StoreError> {
    if workflow.active_chapter_id != Some(chapter_id)
        || workflow.stage != WorkflowStage::WritingDrafting
    {
        return Ok(Vec::new());
    }
    let Some(run_id) = workflow.active_run_id else {
        return Ok(Vec::new());
    };
    feedback::Entity::find()
        .filter(feedback::Entity::COLUMN.run_id.eq(db_id(run_id)))
        .filter(feedback::Entity::COLUMN.status.is_in(["queued", "bound"]))
        .order_by_asc(feedback::Entity::COLUMN.sequence)
        .all(connection)
        .await?
        .iter()
        .map(pending_chapter_body_feedback_from_model)
        .collect()
}

fn pending_chapter_body_feedback_from_model(
    model: &feedback::Model,
) -> Result<PendingChapterBodyFeedback, StoreError> {
    let status = feedback_status_from_db(&model.status)?;
    if status == FeedbackStatus::Included {
        return Err(StoreError::CorruptRecord(
            "正文待投递意见查询返回了 included 记录".to_owned(),
        ));
    }
    let citations = feedback_citations_from_json(&model.citations_json)?;
    let bound_tool_call_id = parse_optional_id(
        model.bound_tool_call_id.clone(),
        "feedback.bound_tool_call_id",
    )?;
    if (status == FeedbackStatus::Queued && bound_tool_call_id.is_some())
        || (status == FeedbackStatus::Bound && bound_tool_call_id.is_none())
    {
        return Err(StoreError::CorruptRecord(
            "自由意见状态与 bound_tool_call_id 不一致".to_owned(),
        ));
    }
    Ok(PendingChapterBodyFeedback {
        id: model.id.into(),
        sequence: model.sequence,
        text: model.text.clone(),
        citations,
        status,
        bound_tool_call_id,
        created_at_ms: model.created_at,
    })
}

/// 返回当前正文 Run 最近一个尚未得到成功响应的请求所携带的意见。
///
/// 已封存的反馈不能重新回到队列：请求快照是不可变的。这里仅透出最近请求的
/// 网络结果，让界面区分“已随请求发送”与“请求失败，尚无模型回应”；一旦最近请求
/// 成功，模型已经给出响应，便不再把历史意见留在当前 Dock 中。
async fn delivered_chapter_body_feedback<C: ConnectionTrait>(
    connection: &C,
    workflow: &WorkflowSnapshot,
    chapter_id: ChapterId,
) -> Result<Vec<DeliveredChapterBodyFeedback>, StoreError> {
    if workflow.active_chapter_id != Some(chapter_id)
        || workflow.stage != WorkflowStage::WritingDrafting
    {
        return Ok(Vec::new());
    }
    let Some(run_id) = workflow.active_run_id else {
        return Ok(Vec::new());
    };
    let Some(request) = model_request::Entity::find()
        .filter(model_request::Entity::COLUMN.run_id.eq(db_id(run_id)))
        .order_by_desc(model_request::Entity::COLUMN.attempt_no)
        .one(connection)
        .await?
    else {
        return Ok(Vec::new());
    };
    let delivery_status = match model_request_status_from_db(&request.status)? {
        ModelRequestStatus::Dispatched => FeedbackDeliveryStatus::Sent,
        ModelRequestStatus::Failed => FeedbackDeliveryStatus::Failed,
        ModelRequestStatus::Unknown => FeedbackDeliveryStatus::Unknown,
        ModelRequestStatus::Cancelled => FeedbackDeliveryStatus::Cancelled,
        ModelRequestStatus::Prepared | ModelRequestStatus::Succeeded => return Ok(Vec::new()),
    };
    let request_id: Uuid = request.id.into();
    feedback::Entity::find()
        .filter(feedback::Entity::COLUMN.run_id.eq(db_id(run_id)))
        .filter(feedback::Entity::COLUMN.status.eq("included"))
        .filter(
            feedback::Entity::COLUMN
                .included_request_id
                .eq(request.id.to_string()),
        )
        .order_by_asc(feedback::Entity::COLUMN.sequence)
        .all(connection)
        .await?
        .iter()
        .map(|model| delivered_chapter_body_feedback_from_model(model, request_id, delivery_status))
        .collect()
}

fn delivered_chapter_body_feedback_from_model(
    model: &feedback::Model,
    request_id: Uuid,
    delivery_status: FeedbackDeliveryStatus,
) -> Result<DeliveredChapterBodyFeedback, StoreError> {
    if feedback_status_from_db(&model.status)? != FeedbackStatus::Included {
        return Err(StoreError::CorruptRecord(
            "正文已发送意见查询返回了非 included 记录".to_owned(),
        ));
    }
    let included_request_id = parse_optional_id(
        model.included_request_id.clone(),
        "feedback.included_request_id",
    )?
    .ok_or_else(|| StoreError::CorruptRecord("已发送自由意见缺少请求 ID".to_owned()))?;
    if included_request_id != request_id {
        return Err(StoreError::CorruptRecord(
            "已发送自由意见关联的请求与查询请求不一致".to_owned(),
        ));
    }
    if parse_optional_id(
        model.bound_tool_call_id.clone(),
        "feedback.bound_tool_call_id",
    )?
    .is_none()
    {
        return Err(StoreError::CorruptRecord(
            "已发送自由意见缺少工具锚点".to_owned(),
        ));
    }
    Ok(DeliveredChapterBodyFeedback {
        id: model.id.into(),
        sequence: model.sequence,
        text: model.text.clone(),
        citations: feedback_citations_from_json(&model.citations_json)?,
        request_id,
        delivery_status,
        created_at_ms: model.created_at,
    })
}

async fn latest_chapter_outline_fold<C: ConnectionTrait>(
    connection: &C,
    version_id: VersionId,
    chapter_id: ChapterId,
) -> Result<Option<ContextFold>, StoreError> {
    let runs = run::Entity::find()
        .filter(run::Entity::COLUMN.version_id.eq(db_id(version_id)))
        .filter(run::Entity::COLUMN.kind.eq("chapter_outline"))
        .filter(run::Entity::COLUMN.status.eq("succeeded"))
        .order_by_desc(run::Entity::COLUMN.ended_at)
        .order_by_desc(run::Entity::COLUMN.id)
        .all(connection)
        .await?;
    for run_model in runs {
        let checkpoint: RunCheckpoint = serde_json::from_str(&run_model.checkpoint_json)?;
        let checkpoint_chapter_id = checkpoint
            .chapter_id
            .ok_or_else(|| StoreError::CorruptRecord("细纲 Run 检查点缺少章节 ID".to_owned()))?;
        if checkpoint_chapter_id == chapter_id {
            return Ok(checkpoint.context_fold);
        }
    }
    copied_chapter_fold(
        connection,
        version_id,
        chapter_id,
        CopiedContextFoldStage::ChapterOutline,
    )
    .await
}

async fn latest_chapter_body_fold<C: ConnectionTrait>(
    connection: &C,
    version_id: VersionId,
    chapter_id: ChapterId,
) -> Result<Option<ContextFold>, StoreError> {
    let runs = run::Entity::find()
        .filter(run::Entity::COLUMN.version_id.eq(db_id(version_id)))
        .filter(run::Entity::COLUMN.kind.eq("chapter_body"))
        .filter(run::Entity::COLUMN.status.eq("succeeded"))
        .order_by_desc(run::Entity::COLUMN.ended_at)
        .order_by_desc(run::Entity::COLUMN.id)
        .all(connection)
        .await?;
    for run_model in runs {
        let checkpoint: RunCheckpoint = serde_json::from_str(&run_model.checkpoint_json)?;
        let checkpoint_chapter_id = checkpoint
            .chapter_id
            .ok_or_else(|| StoreError::CorruptRecord("正文 Run 检查点缺少章节 ID".to_owned()))?;
        if checkpoint_chapter_id == chapter_id {
            return Ok(checkpoint.context_fold);
        }
    }
    copied_chapter_fold(
        connection,
        version_id,
        chapter_id,
        CopiedContextFoldStage::ChapterBody,
    )
    .await
}

/// 委托审核子 Run 所检查的目标类型。
///
/// 它不包含文档修订或内部数据库 ID；调度器只能据此选择受限的内建审核提示词。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DelegatedReviewKind {
    Preparation(PreparationArtifactKind),
    ChapterOutline,
    ChapterEdit,
    ChapterFinal,
}

/// 一个已验证、可发送给审核 Agent 的输入快照。
///
/// 所有文本都来自委托时仍为当前修订的文档。调度器仍会在真正取得模型许可后读取
/// 此快照，避免把项目、章节或修订 ID 暴露给模型。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DelegatedReviewTask {
    pub review_id: ReviewId,
    pub run_id: RunId,
    pub version_id: VersionId,
    pub kind: DelegatedReviewKind,
    pub target_content: String,
    pub novel_brief: String,
    pub approved_context: String,
    pub chapter_context: String,
    pub base_content: String,
    pub chapter_outline: String,
    pub user_notes: String,
    /// 委托时固化的引用；审核子 Run 只看到引文文本，不接触内部 ID。
    pub citations: Vec<FeedbackCitation>,
}

/// 创建审核 Agent 子 Run 后返回给 HTTP 层的稳定结果。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewDelegationResult {
    pub review_id: ReviewId,
    pub run_id: RunId,
    pub snapshot: PreparationSnapshot,
}

/// 取消委托后需要由调度器中止的本地网络请求，以及刷新后的工作台快照。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelReviewDelegationResult {
    pub review_id: ReviewId,
    pub run_id: RunId,
    pub cancelled_request_ids: Vec<Uuid>,
    pub snapshot: PreparationSnapshot,
}

#[derive(Clone, Debug)]
struct DelegatedReviewDecision {
    passed: bool,
    confidence: f64,
    issues: Vec<String>,
}

struct ChapterEditReviewDecision {
    passed: bool,
    issues: String,
    review_run_id: RunId,
    confidence: f64,
}

/// 将逐次确认正文候选的 Review 决定与候选应用/拒绝合并为一个事务。
///
/// 人工与审核 Agent 仅在决定来源、置信度和引用输入上不同；候选的基础修订复核、
/// 工具结果、消息账本、工作流恢复和事件必须共用同一条路径。
async fn apply_chapter_edit_review_decision(
    transaction: &DatabaseTransaction,
    review_id: ReviewId,
    decision: ChapterEditReviewDecision,
    now: i64,
) -> Result<VersionId, StoreError> {
    let review_model = review::Entity::find_by_id(db_id(review_id))
        .filter(review::Entity::COLUMN.review_type.eq("chapter_edit"))
        .one(transaction)
        .await?
        .ok_or(StoreError::ReviewNotFound)?;
    let tool_call_id = review_model
        .target_tool_call_id
        .as_deref()
        .map(|id| parse_id(id, "chapter_edit.target_tool_call_id"))
        .transpose()?
        .ok_or_else(|| StoreError::CorruptRecord("正文编辑审核缺少目标工具调用".to_owned()))?;
    let proposal_call = tool_call::Entity::find_by_id(db_id(tool_call_id))
        .one(transaction)
        .await?
        .ok_or(StoreError::ProposalNotFound)?;
    if proposal_call.status != "awaiting_review" {
        return Err(StoreError::ProposalAlreadyResolved);
    }
    let run_id: RunId = proposal_call.run_id.into();
    let run_model = run::Entity::find_by_id(proposal_call.run_id)
        .filter(run::Entity::COLUMN.kind.eq("chapter_body"))
        .one(transaction)
        .await?
        .ok_or(StoreError::RunNotFound)?;
    if run_model.parent_run_id.is_some() {
        return Err(StoreError::CorruptRecord(
            "正文编辑审核目标不能属于子 Run".to_owned(),
        ));
    }
    let checkpoint: RunCheckpoint = serde_json::from_str(&run_model.checkpoint_json)?;
    let chapter_id = checkpoint
        .chapter_id
        .ok_or_else(|| StoreError::CorruptRecord("正文编辑审核 Run 缺少章节 ID".to_owned()))?;
    let (chapter_model, body_document) =
        chapter_and_document(transaction, chapter_id, CHAPTER_BODY_ROLE, "章节正文").await?;
    if chapter_model.status != "in_progress" {
        return Err(StoreError::InvalidChapterState(
            "章节尚未处于可编辑正文状态".to_owned(),
        ));
    }
    let version_id: VersionId = chapter_model.version_id.into();
    if run_model.version_id != db_id(version_id) || review_model.version_id != db_id(version_id) {
        return Err(StoreError::CorruptRecord(
            "正文编辑审核、候选工具调用和章节不属于同一稿本".to_owned(),
        ));
    }
    let workflow = workflow_for_version(transaction, version_id).await?;
    if workflow.active_run_id != Some(run_id)
        || workflow.active_chapter_id != Some(chapter_id)
        || workflow.stage != WorkflowStage::WritingDrafting
        || workflow.execution != ExecutionStatus::WaitingReview
        || run_model.status != "waiting"
    {
        return Err(StoreError::InvalidChapterState(
            "当前工作流不在正文编辑提案审核状态".to_owned(),
        ));
    }
    let proposal = pending_chapter_body_proposal_from_tool_call(&proposal_call)?;
    let review_base_revision_id = parse_optional_id(
        review_model.target_revision_id.clone(),
        "chapter_edit.target_revision_id",
    )?;
    if review_base_revision_id != proposal.base_revision_id {
        return Err(StoreError::CorruptRecord(
            "正文编辑审核的基础修订与候选不一致".to_owned(),
        ));
    }
    let review_dependency_revision_ids =
        serde_json::from_str::<Vec<String>>(&review_model.dependencies_json)?
            .into_iter()
            .map(|id| parse_id(&id, "chapter_edit.dependencies_json"))
            .collect::<Result<BTreeSet<_>, _>>()?;
    let dependency_revision_ids = chapter_body_revision_dependency_ids(
        transaction,
        version_id,
        chapter_id,
        proposal.base_revision_id,
    )
    .await?;
    if review_dependency_revision_ids != dependency_revision_ids {
        return Err(StoreError::InvalidChapterState(
            "正文编辑审核依赖已变化，不能应用决定".to_owned(),
        ));
    }
    let current_body = current_chapter_body(transaction, chapter_id).await?;
    if current_body.revision_id != proposal.base_revision_id {
        return Err(StoreError::ProposalBaseRevisionConflict {
            expected: proposal.base_revision_id,
            actual: current_body.revision_id,
        });
    }

    let expected_attempt_id = decision.review_run_id.to_string();
    if review_status_from_db(review_model.status.clone())? != ReviewStatus::Delegated
        || review_model.attempt_id.as_deref() != Some(expected_attempt_id.as_str())
    {
        return Err(StoreError::ReviewAlreadyResolved);
    }
    let notes = review_model
        .user_input_json
        .as_deref()
        .map(delegated_review_notes)
        .transpose()?
        .unwrap_or_default();
    let source = "agent";
    let confidence = Some(decision.confidence);
    let citations: Vec<FeedbackCitation> = Vec::new();
    let user_input = json!({
        "source": "agent",
        "notes": notes,
        "issues": decision.issues.clone(),
        "confidence": decision.confidence,
    });
    let (tool_result, committed_revision) = if decision.passed {
        let (revision_id, revision_no) = insert_document_revision_with_payload_and_source(
            transaction,
            body_document.id.into(),
            &proposal.candidate_content,
            None,
            "agent",
            proposal.base_revision_id,
            now,
        )
        .await?;
        for dependency_revision_id in &dependency_revision_ids {
            insert_revision_dependency(
                transaction,
                revision_id,
                *dependency_revision_id,
                "chapter_body_input",
            )
            .await?;
        }
        resolve_pending_candidate_feedback_citations(
            transaction,
            run_id,
            tool_call_id,
            &proposal.candidate_content,
            body_document.id.into(),
            revision_id,
            chapter_id,
            now,
        )
        .await?;
        (
            json!({
                "status": "applied",
                "tool_call_id": proposal.provider_call_id,
                "base_revision_id": proposal.base_revision_id.map(|revision_id| revision_id.to_string()),
                "result_revision_id": revision_id.to_string(),
                "data": {
                    "word_count": non_whitespace_char_count(&proposal.candidate_content).to_string(),
                },
                "feedback": [],
            }),
            Some((revision_id, revision_no)),
        )
    } else {
        (
            json!({
                "status": "rejected",
                "tool_call_id": proposal.provider_call_id,
                "base_revision_id": proposal.base_revision_id.map(|revision_id| revision_id.to_string()),
                "feedback": [{
                    "text": decision.issues.clone(),
                    "citations": citations,
                }],
            }),
            None,
        )
    };
    let request_id: Uuid = proposal_call.request_id.into();
    let proposal_sequence = proposal_call.sequence;
    let mut active_call = proposal_call.into_active_model();
    active_call.status = Set("resolved".to_owned());
    active_call.result_json = Set(Some(serde_json::to_string(&tool_result)?));
    active_call.resolved_at = Set(Some(now));
    active_call.update(transaction).await?;
    let bound_feedback_ids =
        bind_pending_feedback_to_tool(transaction, run_id, tool_call_id, proposal_sequence, now)
            .await?;
    append_agent_messages_for_run(
        transaction,
        &run_model,
        vec![NewAgentMessage {
            role: AgentMessageRole::Tool,
            content: json!({
                "kind": "tool_result",
                "content": serde_json::to_string(&tool_result)?,
            }),
            request_id: Some(request_id),
            tool_call_id: Some(proposal.provider_call_id.clone()),
        }],
        now,
    )
    .await?;
    let next_workflow = continue_chapter_body_after_proposal(&workflow, chapter_id, run_id)?;
    update_chapter_body_run_status(transaction, run_id, chapter_id, "queued", None, None, now)
        .await?;
    persist_workflow(transaction, &next_workflow).await?;
    let mut active_review = review_model.into_active_model();
    active_review.status = Set(if decision.passed {
        "approved"
    } else {
        "rejected"
    }
    .to_owned());
    active_review.decision_source = Set(Some(source.to_owned()));
    active_review.passed = Set(Some(decision.passed));
    active_review.confidence = Set(confidence);
    active_review.issues_json = Set((!decision.issues.is_empty())
        .then(|| serde_json::to_string(&decision.issues))
        .transpose()?);
    active_review.user_input_json = Set(Some(serde_json::to_string(&user_input)?));
    active_review.updated_at = Set(now);
    active_review.update(transaction).await?;
    if let Some((revision_id, revision_no)) = committed_revision {
        insert_event(
            transaction,
            version_id,
            "document.committed",
            revision_id,
            revision_no,
            json!({
                "chapter_id": chapter_id,
                "role": CHAPTER_BODY_ROLE,
                "word_count": non_whitespace_char_count(&proposal.candidate_content),
                "source": "agent_proposal",
            }),
            now,
        )
        .await?;
    }
    insert_event(
        transaction,
        version_id,
        "tool.resolved",
        tool_call_id,
        next_workflow.state_revision,
        json!({
            "chapter_id": chapter_id,
            "name": proposal.tool_name,
            "provider_call_id": proposal.provider_call_id,
            "status": tool_result["status"],
        }),
        now,
    )
    .await?;
    insert_event(
        transaction,
        version_id,
        "review.changed",
        review_id,
        next_workflow.state_revision,
        json!({
            "status": if decision.passed { "approved" } else { "rejected" },
            "source": source,
            "confidence": confidence,
            "tool_call_id": tool_call_id,
        }),
        now,
    )
    .await?;
    for feedback_id in bound_feedback_ids {
        insert_event(
            transaction,
            version_id,
            "feedback.changed",
            feedback_id,
            next_workflow.state_revision,
            json!({
                "chapter_id": chapter_id,
                "run_id": run_id,
                "status": "bound",
                "bound_tool_call_id": tool_call_id,
            }),
            now,
        )
        .await?;
    }
    insert_event(
        transaction,
        version_id,
        "run.changed",
        run_id,
        next_workflow.state_revision,
        json!({
            "chapter_id": chapter_id,
            "kind": "chapter_body",
            "status": "queued",
        }),
        now,
    )
    .await?;
    insert_event(
        transaction,
        version_id,
        "workflow.changed",
        version_id,
        next_workflow.state_revision,
        json!({
            "stage": next_workflow.stage,
            "execution": next_workflow.execution,
            "chapter_id": chapter_id,
            "run_id": run_id,
        }),
        now,
    )
    .await?;
    Ok(version_id)
}

async fn copied_chapter_fold<C: ConnectionTrait>(
    connection: &C,
    version_id: VersionId,
    chapter_id: ChapterId,
    stage: CopiedContextFoldStage,
) -> Result<Option<ContextFold>, StoreError> {
    let chapter_model = chapter::Entity::find_by_id(db_id(chapter_id))
        .filter(chapter::Entity::COLUMN.version_id.eq(db_id(version_id)))
        .filter(chapter::Entity::COLUMN.deleted_at.is_null())
        .one(connection)
        .await?
        .ok_or(StoreError::ChapterNotFound)?;
    let workflow = workflow_for_version(connection, version_id).await?;
    let matches = workflow
        .copied_context_folds
        .iter()
        .filter(|fold| fold.stage == stage && fold.chapter_order == Some(chapter_model.order_index))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Ok(None),
        [fold] => Ok(Some(fold.fold.clone())),
        _ => Err(StoreError::CorruptRecord(
            "复制稿本存在重复的章节上下文折叠".to_owned(),
        )),
    }
}

async fn review_type_for_id<C: ConnectionTrait>(
    connection: &C,
    review_id: ReviewId,
) -> Result<String, StoreError> {
    review::Entity::find_by_id(db_id(review_id))
        .one(connection)
        .await?
        .map(|model| model.review_type)
        .ok_or(StoreError::ReviewNotFound)
}

pub(crate) fn ensure_expected_state_revision(
    workflow: &WorkflowSnapshot,
    expected_state_revision: i64,
) -> Result<(), StoreError> {
    if workflow.state_revision != expected_state_revision {
        return Err(StoreError::StateRevisionConflict {
            expected: expected_state_revision,
            actual: workflow.state_revision,
        });
    }
    Ok(())
}

/// 在调用方已锁定同一工作流命令边界时，写入准备产物和审核记录。
///
/// 它故意不创建命令回执，也不单独变更 Run 状态：人工命令和模型回调分别在各自
/// 的外层事务中组合这些动作，避免模型响应已持久化但产物没有提交的中间状态。
pub(crate) async fn persist_preparation_submission(
    transaction: &DatabaseTransaction,
    version_id: VersionId,
    workflow: &WorkflowSnapshot,
    artifacts: &[PreparationArtifactInput],
    chapter_plan: Option<&[ChapterPlanDraft]>,
    author: &str,
    now: i64,
) -> Result<WorkflowSnapshot, StoreError> {
    let current_artifacts = current_preparation_artifacts(transaction, version_id).await?;
    validate_preparation_submission(workflow, &current_artifacts, artifacts, chapter_plan)?;
    let outline_plan = chapter_plan
        .map(|drafts| {
            ChapterPlan::from_drafts(drafts)
                .map_err(|error| StoreError::InvalidPreparationArtifacts(error.to_string()))
        })
        .transpose()?;

    let current_by_kind = current_artifacts
        .iter()
        .map(|artifact| (artifact.kind, artifact))
        .collect::<BTreeMap<_, _>>();
    let mut submitted_by_kind = BTreeMap::new();
    let mut new_revisions = BTreeMap::new();
    for artifact in artifacts {
        let current = current_by_kind.get(&artifact.kind).ok_or_else(|| {
            StoreError::CorruptRecord(format!("稿本缺少 {} 文档槽", artifact.kind.stable_key()))
        })?;
        let structured_payload = if artifact.kind == PreparationArtifactKind::Outline {
            Some(serde_json::to_string(outline_plan.as_ref().ok_or_else(
                || StoreError::InvalidPreparationArtifacts("粗纲提交缺少章节计划".to_owned()),
            )?)?)
        } else {
            None
        };
        let (revision_id, revision_no) = insert_document_revision_with_payload(
            transaction,
            current.document_id,
            &artifact.content,
            structured_payload.as_deref(),
            author,
            now,
        )
        .await?;
        submitted_by_kind.insert(artifact.kind, artifact);
        new_revisions.insert(artifact.kind, (revision_id, revision_no));
    }

    for kind in PreparationArtifactKind::ALL {
        let Some((revision_id, _)) = new_revisions.get(&kind) else {
            continue;
        };
        let dependencies = dependency_revision_ids(kind, &current_by_kind, &new_revisions)?;
        for dependency_revision_id in &dependencies {
            insert_revision_dependency(
                transaction,
                *revision_id,
                *dependency_revision_id,
                "preparation",
            )
            .await?;
        }
        let review_id =
            insert_preparation_review(transaction, version_id, *revision_id, &dependencies, now)
                .await?;
        let revision_no = new_revisions
            .get(&kind)
            .map(|(_, revision_no)| *revision_no)
            .ok_or_else(|| StoreError::CorruptRecord("准备修订缺失".to_owned()))?;
        insert_event(
            transaction,
            version_id,
            "document.committed",
            *revision_id,
            revision_no,
            json!({
                "role": kind.stable_key(),
                "review_id": review_id,
                "word_count": non_whitespace_char_count(&submitted_by_kind[&kind].content),
            }),
            now,
        )
        .await?;
    }

    let changed_kinds = new_revisions.keys().copied().collect::<BTreeSet<_>>();
    for kind in PreparationArtifactKind::ALL {
        if changed_kinds.contains(&kind) || !depends_on_any_changed_kind(kind, &changed_kinds) {
            continue;
        }
        let current = current_by_kind.get(&kind).ok_or_else(|| {
            StoreError::CorruptRecord(format!("稿本缺少 {} 文档槽", kind.stable_key()))
        })?;
        if let Some(revision_id) = current.revision_id {
            mark_review_stale(transaction, revision_id, now).await?;
        }
    }

    let next_workflow = submit_preparation_for_review(workflow)?;
    persist_workflow(transaction, &next_workflow).await?;
    Ok(next_workflow)
}

/// 在调用方已锁定同一工作流命令边界时，提交一份章节细纲及其审核记录。
///
/// 人工提交与模型最终响应共用这条事务路径；调用方仅负责各自的命令回执或模型
/// attempt 终态，因而不会形成“细纲已保存但 Run 仍在生成”的持久状态。
pub(crate) async fn persist_chapter_outline_submission(
    transaction: &DatabaseTransaction,
    chapter_id: ChapterId,
    workflow: &WorkflowSnapshot,
    input: &NewChapterOutline,
    author: &str,
    now: i64,
) -> Result<WorkflowSnapshot, StoreError> {
    let (chapter_model, outline_document) =
        chapter_and_document(transaction, chapter_id, CHAPTER_OUTLINE_ROLE, "章节细纲").await?;
    if chapter_model.status != "in_progress" {
        return Err(StoreError::InvalidChapterState(
            "章节尚未进入细纲流程".to_owned(),
        ));
    }
    let version_id: VersionId = chapter_model.version_id.into();
    if workflow.version_id != version_id {
        return Err(StoreError::CorruptRecord(
            "章节细纲工作流与章节稿本不一致".to_owned(),
        ));
    }
    let next_workflow = submit_chapter_outline_for_review(workflow, chapter_id)?;
    let run_id = workflow
        .active_run_id
        .ok_or_else(|| StoreError::CorruptRecord("细纲提交缺少对应的活跃 Run".to_owned()))?;
    chapter_outline_run(transaction, run_id, chapter_id).await?;
    let (revision_id, revision_no) = insert_document_revision_with_payload(
        transaction,
        outline_document.id.into(),
        &input.content,
        None,
        author,
        now,
    )
    .await?;
    let dependency_revision_ids =
        chapter_outline_dependency_revision_ids(transaction, version_id, chapter_id).await?;
    for dependency_revision_id in &dependency_revision_ids {
        insert_revision_dependency(
            transaction,
            revision_id,
            *dependency_revision_id,
            "chapter_outline_input",
        )
        .await?;
    }
    let review_id = insert_chapter_outline_review(
        transaction,
        version_id,
        revision_id,
        &dependency_revision_ids,
        now,
    )
    .await?;
    update_chapter_outline_run_status(transaction, run_id, chapter_id, "waiting", None, None, now)
        .await?;
    persist_workflow(transaction, &next_workflow).await?;
    insert_event(
        transaction,
        version_id,
        "document.committed",
        revision_id,
        revision_no,
        json!({
            "chapter_id": chapter_id,
            "role": CHAPTER_OUTLINE_ROLE,
            "word_count": non_whitespace_char_count(&input.content),
        }),
        now,
    )
    .await?;
    insert_event(
        transaction,
        version_id,
        "review.changed",
        review_id,
        next_workflow.state_revision,
        json!({
            "chapter_id": chapter_id,
            "role": CHAPTER_OUTLINE_ROLE,
            "status": "open",
        }),
        now,
    )
    .await?;
    insert_event(
        transaction,
        version_id,
        "run.changed",
        run_id,
        next_workflow.state_revision,
        json!({
            "chapter_id": chapter_id,
            "kind": "chapter_outline",
            "status": "waiting",
        }),
        now,
    )
    .await?;
    insert_event(
        transaction,
        version_id,
        "workflow.changed",
        version_id,
        next_workflow.state_revision,
        json!({
            "stage": next_workflow.stage,
            "execution": next_workflow.execution,
            "chapter_id": chapter_id,
            "run_id": run_id,
        }),
        now,
    )
    .await?;
    Ok(next_workflow)
}

fn validate_preparation_submission(
    workflow: &WorkflowSnapshot,
    current_artifacts: &[CurrentPreparationArtifact],
    submitted_artifacts: &[PreparationArtifactInput],
    chapter_plan: Option<&[ChapterPlanDraft]>,
) -> Result<(), StoreError> {
    if submitted_artifacts.is_empty() {
        return Err(StoreError::InvalidPreparationArtifacts(
            "至少需要提交一项产物".to_owned(),
        ));
    }
    let submitted_kinds = submitted_artifacts
        .iter()
        .map(|artifact| artifact.kind)
        .collect::<BTreeSet<_>>();
    if submitted_kinds.len() != submitted_artifacts.len() {
        return Err(StoreError::InvalidPreparationArtifacts(
            "同一次提交不能包含重复的产物类型".to_owned(),
        ));
    }
    let outline_is_submitted = submitted_kinds.contains(&PreparationArtifactKind::Outline);
    if outline_is_submitted != chapter_plan.is_some() {
        return Err(StoreError::InvalidPreparationArtifacts(
            if outline_is_submitted {
                "粗纲提交必须同时包含至少一个章节计划项".to_owned()
            } else {
                "未提交粗纲时不能修改章节计划".to_owned()
            },
        ));
    }

    match workflow.stage {
        WorkflowStage::PreparingGenerating => {
            if current_artifacts
                .iter()
                .any(|artifact| artifact.revision_id.is_some())
            {
                return Err(StoreError::InvalidPreparationState(
                    "初次准备已存在产物修订，不能再次按初次提交处理".to_owned(),
                ));
            }
            if submitted_kinds.len() != PreparationArtifactKind::ALL.len()
                || PreparationArtifactKind::ALL
                    .into_iter()
                    .any(|kind| !submitted_kinds.contains(&kind))
            {
                return Err(StoreError::InvalidPreparationArtifacts(
                    "初次准备必须一次提交背景、资料、人物和粗纲四项产物".to_owned(),
                ));
            }
        }
        WorkflowStage::PreparingRevising => {
            for artifact in submitted_artifacts {
                let current = current_artifacts
                    .iter()
                    .find(|current| current.kind == artifact.kind)
                    .ok_or_else(|| {
                        StoreError::CorruptRecord(format!(
                            "稿本缺少 {} 文档槽",
                            artifact.kind.stable_key()
                        ))
                    })?;
                let status = current.review.as_ref().map(|review| &review.status);
                if !matches!(status, Some(ReviewStatus::Rejected | ReviewStatus::Stale)) {
                    return Err(StoreError::InvalidPreparationArtifacts(format!(
                        "{} 当前不需要修订，不能覆盖已通过或待审修订",
                        artifact.kind.label()
                    )));
                }
            }
        }
        _ => {
            return Err(StoreError::InvalidPreparationState(
                "只有准备生成或准备修订状态可以提交产物".to_owned(),
            ));
        }
    }
    Ok(())
}

fn preparation_upstreams(kind: PreparationArtifactKind) -> &'static [PreparationArtifactKind] {
    match kind {
        PreparationArtifactKind::Research => &[],
        PreparationArtifactKind::Background => &[PreparationArtifactKind::Research],
        PreparationArtifactKind::Characters => &[
            PreparationArtifactKind::Research,
            PreparationArtifactKind::Background,
        ],
        PreparationArtifactKind::Outline => &[
            PreparationArtifactKind::Research,
            PreparationArtifactKind::Background,
            PreparationArtifactKind::Characters,
        ],
    }
}

fn dependency_revision_ids(
    kind: PreparationArtifactKind,
    current_by_kind: &BTreeMap<PreparationArtifactKind, &CurrentPreparationArtifact>,
    new_revisions: &BTreeMap<PreparationArtifactKind, (RevisionId, i64)>,
) -> Result<Vec<RevisionId>, StoreError> {
    preparation_upstreams(kind)
        .iter()
        .map(|upstream| {
            new_revisions
                .get(upstream)
                .map(|(revision_id, _)| *revision_id)
                .or_else(|| {
                    current_by_kind
                        .get(upstream)
                        .and_then(|artifact| artifact.revision_id)
                })
                .ok_or_else(|| {
                    StoreError::InvalidPreparationArtifacts(format!(
                        "{} 缺少所依赖的 {} 当前修订",
                        kind.label(),
                        upstream.label()
                    ))
                })
        })
        .collect()
}

fn depends_on_any_changed_kind(
    kind: PreparationArtifactKind,
    changed_kinds: &BTreeSet<PreparationArtifactKind>,
) -> bool {
    preparation_upstreams(kind)
        .iter()
        .any(|upstream| changed_kinds.contains(upstream))
}

async fn insert_revision_dependency(
    transaction: &DatabaseTransaction,
    revision_id: RevisionId,
    dependency_revision_id: RevisionId,
    purpose: &str,
) -> Result<(), StoreError> {
    revision_dependency::ActiveModel {
        revision_id: Set(db_id(revision_id)),
        dependency_revision_id: Set(db_id(dependency_revision_id)),
        purpose: Set(purpose.to_owned()),
        ..Default::default()
    }
    .insert(transaction)
    .await?;
    Ok(())
}

async fn insert_preparation_review(
    transaction: &DatabaseTransaction,
    version_id: VersionId,
    revision_id: RevisionId,
    dependency_revision_ids: &[RevisionId],
    now: i64,
) -> Result<ReviewId, StoreError> {
    let review_id = Uuid::new_v4();
    let dependencies = dependency_revision_ids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    review::ActiveModel {
        id: Set(db_id(review_id)),
        version_id: Set(db_id(version_id)),
        batch_id: Set(None),
        review_type: Set("preparation".to_owned()),
        target_revision_id: Set(Some(revision_id.to_string())),
        target_tool_call_id: Set(None),
        dependencies_json: Set(serde_json::to_string(&dependencies)?),
        status: Set("open".to_owned()),
        attempt_id: Set(None),
        decision_source: Set(None),
        source_review_id: Set(None),
        passed: Set(None),
        confidence: Set(None),
        issues_json: Set(None),
        user_input_json: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(transaction)
    .await?;
    Ok(review_id)
}

async fn insert_chapter_outline_review(
    transaction: &DatabaseTransaction,
    version_id: VersionId,
    revision_id: RevisionId,
    dependency_revision_ids: &BTreeSet<RevisionId>,
    now: i64,
) -> Result<ReviewId, StoreError> {
    let review_id = Uuid::new_v4();
    let dependencies = dependency_revision_ids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    review::ActiveModel {
        id: Set(db_id(review_id)),
        version_id: Set(db_id(version_id)),
        batch_id: Set(None),
        review_type: Set("chapter_outline".to_owned()),
        target_revision_id: Set(Some(revision_id.to_string())),
        target_tool_call_id: Set(None),
        dependencies_json: Set(serde_json::to_string(&dependencies)?),
        status: Set("open".to_owned()),
        attempt_id: Set(None),
        decision_source: Set(None),
        source_review_id: Set(None),
        passed: Set(None),
        confidence: Set(None),
        issues_json: Set(None),
        user_input_json: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(transaction)
    .await?;
    Ok(review_id)
}

async fn insert_chapter_final_review(
    transaction: &DatabaseTransaction,
    version_id: VersionId,
    revision_id: RevisionId,
    dependency_revision_ids: &BTreeSet<RevisionId>,
    now: i64,
) -> Result<ReviewId, StoreError> {
    let review_id = Uuid::new_v4();
    let dependencies = dependency_revision_ids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    review::ActiveModel {
        id: Set(db_id(review_id)),
        version_id: Set(db_id(version_id)),
        batch_id: Set(None),
        review_type: Set("chapter_final".to_owned()),
        target_revision_id: Set(Some(revision_id.to_string())),
        target_tool_call_id: Set(None),
        dependencies_json: Set(serde_json::to_string(&dependencies)?),
        status: Set("open".to_owned()),
        attempt_id: Set(None),
        decision_source: Set(None),
        source_review_id: Set(None),
        passed: Set(None),
        confidence: Set(None),
        issues_json: Set(None),
        user_input_json: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(transaction)
    .await?;
    Ok(review_id)
}

async fn insert_chapter_edit_review(
    transaction: &DatabaseTransaction,
    version_id: VersionId,
    tool_call_id: Uuid,
    base_revision_id: Option<RevisionId>,
    dependency_revision_ids: &BTreeSet<RevisionId>,
    now: i64,
) -> Result<ReviewId, StoreError> {
    let review_id = Uuid::new_v4();
    let dependencies = dependency_revision_ids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    review::ActiveModel {
        id: Set(db_id(review_id)),
        version_id: Set(db_id(version_id)),
        batch_id: Set(None),
        review_type: Set("chapter_edit".to_owned()),
        target_revision_id: Set(base_revision_id.map(|revision_id| revision_id.to_string())),
        target_tool_call_id: Set(Some(tool_call_id.to_string())),
        dependencies_json: Set(serde_json::to_string(&dependencies)?),
        status: Set("open".to_owned()),
        attempt_id: Set(None),
        decision_source: Set(None),
        source_review_id: Set(None),
        passed: Set(None),
        confidence: Set(None),
        issues_json: Set(None),
        user_input_json: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(transaction)
    .await?;
    Ok(review_id)
}

async fn mark_review_stale(
    transaction: &DatabaseTransaction,
    revision_id: RevisionId,
    now: i64,
) -> Result<(), StoreError> {
    let reviews = review::Entity::find()
        .filter(
            review::Entity::COLUMN
                .target_revision_id
                .eq(revision_id.to_string()),
        )
        .filter(review::Entity::COLUMN.review_type.eq("preparation"))
        .filter(
            review::Entity::COLUMN
                .status
                .is_in(["open", "delegated", "approved", "rejected"]),
        )
        .all(transaction)
        .await?;
    for review_model in reviews {
        let mut active_review = review_model.into_active_model();
        active_review.status = Set("stale".to_owned());
        active_review.updated_at = Set(now);
        active_review.update(transaction).await?;
    }
    Ok(())
}

struct PreparationReviewTarget {
    version_id: VersionId,
    kind: PreparationArtifactKind,
    target_revision_id: RevisionId,
    current_revision_id: RevisionId,
    status: ReviewStatus,
}

struct ChapterOutlineReviewTarget {
    version_id: VersionId,
    chapter_id: ChapterId,
    target_revision_id: RevisionId,
    current_revision_id: RevisionId,
    dependency_revision_ids: BTreeSet<RevisionId>,
    status: ReviewStatus,
}

struct ChapterFinalReviewTarget {
    version_id: VersionId,
    chapter_id: ChapterId,
    target_revision_id: RevisionId,
    current_revision_id: RevisionId,
    dependency_revision_ids: BTreeSet<RevisionId>,
    status: ReviewStatus,
}

struct ChapterEditReviewTarget {
    version_id: VersionId,
    chapter_id: ChapterId,
    parent_run_id: RunId,
    base_revision_id: Option<RevisionId>,
    dependency_revision_ids: BTreeSet<RevisionId>,
    proposal: PendingChapterBodyProposal,
}

async fn preparation_review_target(
    transaction: &DatabaseTransaction,
    review_id: ReviewId,
) -> Result<PreparationReviewTarget, StoreError> {
    let review_model = review::Entity::find_by_id(db_id(review_id))
        .filter(review::Entity::COLUMN.review_type.eq("preparation"))
        .one(transaction)
        .await?
        .ok_or(StoreError::ReviewNotFound)?;
    let target_revision_id = review_model
        .target_revision_id
        .ok_or_else(|| StoreError::CorruptRecord("准备审核缺少目标修订".to_owned()))?;
    let target_revision_uuid = parse_id(&target_revision_id, "target_revision_id")?;
    let revision_model = document_revision::Entity::find_by_id(db_id(target_revision_uuid))
        .one(transaction)
        .await?
        .ok_or(StoreError::ReviewNotFound)?;
    let document_model = document::Entity::find_by_id(revision_model.document_id)
        .filter(document::Entity::COLUMN.deleted_at.is_null())
        .one(transaction)
        .await?
        .ok_or(StoreError::ReviewNotFound)?;
    Ok(PreparationReviewTarget {
        version_id: review_model.version_id.into(),
        kind: preparation_kind_from_role(&document_model.role)?,
        target_revision_id: target_revision_uuid,
        current_revision_id: document_model
            .current_revision_id
            .map(|current_revision_id| parse_id(&current_revision_id, "current_revision_id"))
            .transpose()?
            .ok_or_else(|| StoreError::CorruptRecord("审核目标文档没有当前修订".to_owned()))?,
        status: review_status_from_db(review_model.status)?,
    })
}

async fn chapter_outline_review_target(
    transaction: &DatabaseTransaction,
    review_id: ReviewId,
) -> Result<ChapterOutlineReviewTarget, StoreError> {
    let review_model = review::Entity::find_by_id(db_id(review_id))
        .filter(review::Entity::COLUMN.review_type.eq("chapter_outline"))
        .one(transaction)
        .await?
        .ok_or(StoreError::ReviewNotFound)?;
    let target_revision_id = review_model
        .target_revision_id
        .as_deref()
        .ok_or_else(|| StoreError::CorruptRecord("细纲审核缺少目标修订".to_owned()))?;
    let target_revision_id = parse_id(target_revision_id, "target_revision_id")?;
    let revision_model = document_revision::Entity::find_by_id(db_id(target_revision_id))
        .one(transaction)
        .await?
        .ok_or(StoreError::ReviewNotFound)?;
    let document_model = document::Entity::find_by_id(revision_model.document_id)
        .filter(document::Entity::COLUMN.deleted_at.is_null())
        .filter(document::Entity::COLUMN.role.eq(CHAPTER_OUTLINE_ROLE))
        .one(transaction)
        .await?
        .ok_or(StoreError::ReviewNotFound)?;
    if document_model.version_id != review_model.version_id {
        return Err(StoreError::CorruptRecord(
            "章节细纲审核目标不属于审核稿本".to_owned(),
        ));
    }
    let chapter_id = parse_optional_id(document_model.chapter_id, "chapter_outline.chapter_id")?
        .ok_or_else(|| StoreError::CorruptRecord("章节细纲审核目标缺少章节 ID".to_owned()))?;
    let dependency_revision_ids =
        serde_json::from_str::<Vec<String>>(&review_model.dependencies_json)?
            .into_iter()
            .map(|revision_id| parse_id(&revision_id, "chapter_outline.dependencies_json"))
            .collect::<Result<BTreeSet<_>, _>>()?;
    Ok(ChapterOutlineReviewTarget {
        version_id: review_model.version_id.into(),
        chapter_id,
        target_revision_id,
        current_revision_id: document_model
            .current_revision_id
            .as_deref()
            .map(|current_revision_id| parse_id(current_revision_id, "current_revision_id"))
            .transpose()?
            .ok_or_else(|| StoreError::CorruptRecord("审核目标文档没有当前修订".to_owned()))?,
        dependency_revision_ids,
        status: review_status_from_db(review_model.status)?,
    })
}

async fn chapter_final_review_target(
    transaction: &DatabaseTransaction,
    review_id: ReviewId,
) -> Result<ChapterFinalReviewTarget, StoreError> {
    let review_model = review::Entity::find_by_id(db_id(review_id))
        .filter(review::Entity::COLUMN.review_type.eq("chapter_final"))
        .one(transaction)
        .await?
        .ok_or(StoreError::ReviewNotFound)?;
    let target_revision_id = review_model
        .target_revision_id
        .as_deref()
        .ok_or_else(|| StoreError::CorruptRecord("整章审核缺少目标修订".to_owned()))?;
    let target_revision_id = parse_id(target_revision_id, "target_revision_id")?;
    let revision_model = document_revision::Entity::find_by_id(db_id(target_revision_id))
        .one(transaction)
        .await?
        .ok_or(StoreError::ReviewNotFound)?;
    let document_model = document::Entity::find_by_id(revision_model.document_id)
        .filter(document::Entity::COLUMN.deleted_at.is_null())
        .filter(document::Entity::COLUMN.role.eq(CHAPTER_BODY_ROLE))
        .one(transaction)
        .await?
        .ok_or(StoreError::ReviewNotFound)?;
    if document_model.version_id != review_model.version_id {
        return Err(StoreError::CorruptRecord(
            "整章审核目标不属于审核稿本".to_owned(),
        ));
    }
    let chapter_id = parse_optional_id(document_model.chapter_id, "chapter_body.chapter_id")?
        .ok_or_else(|| StoreError::CorruptRecord("整章审核目标缺少章节 ID".to_owned()))?;
    let dependency_revision_ids =
        serde_json::from_str::<Vec<String>>(&review_model.dependencies_json)?
            .into_iter()
            .map(|revision_id| parse_id(&revision_id, "chapter_final.dependencies_json"))
            .collect::<Result<BTreeSet<_>, _>>()?;
    Ok(ChapterFinalReviewTarget {
        version_id: review_model.version_id.into(),
        chapter_id,
        target_revision_id,
        current_revision_id: document_model
            .current_revision_id
            .as_deref()
            .map(|current_revision_id| parse_id(current_revision_id, "current_revision_id"))
            .transpose()?
            .ok_or_else(|| StoreError::CorruptRecord("审核目标文档没有当前修订".to_owned()))?,
        dependency_revision_ids,
        status: review_status_from_db(review_model.status)?,
    })
}

async fn chapter_edit_review_target(
    transaction: &DatabaseTransaction,
    review_id: ReviewId,
) -> Result<ChapterEditReviewTarget, StoreError> {
    let review_model = review::Entity::find_by_id(db_id(review_id))
        .filter(review::Entity::COLUMN.review_type.eq("chapter_edit"))
        .one(transaction)
        .await?
        .ok_or(StoreError::ReviewNotFound)?;
    let tool_call_id = review_model
        .target_tool_call_id
        .as_deref()
        .map(|id| parse_id(id, "chapter_edit.target_tool_call_id"))
        .transpose()?
        .ok_or_else(|| StoreError::CorruptRecord("正文编辑审核缺少目标工具调用".to_owned()))?;
    let proposal_call = tool_call::Entity::find_by_id(db_id(tool_call_id))
        .one(transaction)
        .await?
        .ok_or(StoreError::ProposalNotFound)?;
    if proposal_call.status != "awaiting_review" {
        return Err(StoreError::ProposalAlreadyResolved);
    }
    let parent_run_id: RunId = proposal_call.run_id.into();
    let parent_run = run::Entity::find_by_id(proposal_call.run_id)
        .filter(run::Entity::COLUMN.kind.eq("chapter_body"))
        .one(transaction)
        .await?
        .ok_or(StoreError::RunNotFound)?;
    if parent_run.parent_run_id.is_some() {
        return Err(StoreError::CorruptRecord(
            "正文编辑审核目标不能属于子 Run".to_owned(),
        ));
    }
    let checkpoint: RunCheckpoint = serde_json::from_str(&parent_run.checkpoint_json)?;
    let chapter_id = checkpoint
        .chapter_id
        .ok_or_else(|| StoreError::CorruptRecord("正文编辑审核 Run 缺少章节 ID".to_owned()))?;
    let version_id: VersionId = review_model.version_id.into();
    if parent_run.version_id != db_id(version_id) {
        return Err(StoreError::CorruptRecord(
            "正文编辑审核与候选工具调用不属于同一稿本".to_owned(),
        ));
    }
    let chapter_model = chapter::Entity::find_by_id(db_id(chapter_id))
        .filter(chapter::Entity::COLUMN.version_id.eq(db_id(version_id)))
        .filter(chapter::Entity::COLUMN.deleted_at.is_null())
        .one(transaction)
        .await?
        .ok_or(StoreError::ChapterNotFound)?;
    if chapter_model.status != "in_progress" {
        return Err(StoreError::InvalidChapterState(
            "正文编辑审核目标章节不在写作中".to_owned(),
        ));
    }
    let proposal = pending_chapter_body_proposal_from_tool_call(&proposal_call)?;
    let base_revision_id = parse_optional_id(
        review_model.target_revision_id.clone(),
        "chapter_edit.target_revision_id",
    )?;
    if base_revision_id != proposal.base_revision_id {
        return Err(StoreError::CorruptRecord(
            "正文编辑审核的基础修订与候选不一致".to_owned(),
        ));
    }
    let dependency_revision_ids =
        serde_json::from_str::<Vec<String>>(&review_model.dependencies_json)?
            .into_iter()
            .map(|id| parse_id(&id, "chapter_edit.dependencies_json"))
            .collect::<Result<BTreeSet<_>, _>>()?;
    let expected_dependencies =
        chapter_body_revision_dependency_ids(transaction, version_id, chapter_id, base_revision_id)
            .await?;
    if dependency_revision_ids != expected_dependencies {
        return Err(StoreError::InvalidChapterState(
            "正文编辑审核依赖已变化，不能委托".to_owned(),
        ));
    }
    let current_body = current_chapter_body(transaction, chapter_id).await?;
    if current_body.revision_id != base_revision_id {
        return Err(StoreError::ProposalBaseRevisionConflict {
            expected: base_revision_id,
            actual: current_body.revision_id,
        });
    }
    Ok(ChapterEditReviewTarget {
        version_id,
        chapter_id,
        parent_run_id,
        base_revision_id,
        dependency_revision_ids,
        proposal,
    })
}

fn preparation_kind_from_role(role: &str) -> Result<PreparationArtifactKind, StoreError> {
    match role {
        "background" => Ok(PreparationArtifactKind::Background),
        "research" => Ok(PreparationArtifactKind::Research),
        "characters" => Ok(PreparationArtifactKind::Characters),
        "outline" => Ok(PreparationArtifactKind::Outline),
        _ => Err(StoreError::CorruptRecord(format!(
            "未知准备产物角色：{role}"
        ))),
    }
}

async fn persist_human_review_decision(
    transaction: &DatabaseTransaction,
    review_id: ReviewId,
    decision: &HumanReviewDecision,
    citations: &[FeedbackCitation],
    now: i64,
) -> Result<(), StoreError> {
    let status = if decision.passed {
        "approved"
    } else {
        "rejected"
    };
    let issues_json = (!decision.issues.is_empty())
        .then(|| serde_json::to_string(&decision.issues))
        .transpose()?;
    let review_model = review::Entity::find_by_id(db_id(review_id))
        .filter(review::Entity::COLUMN.status.eq("open"))
        .one(transaction)
        .await?
        .ok_or(StoreError::ReviewAlreadyResolved)?;
    let mut active_review = review_model.into_active_model();
    active_review.status = Set(status.to_owned());
    active_review.decision_source = Set(Some("human".to_owned()));
    active_review.passed = Set(Some(decision.passed));
    active_review.issues_json = Set(issues_json);
    active_review.user_input_json = Set(Some(serde_json::to_string(&json!({
        "source": "user",
        "citations": citations,
    }))?));
    active_review.updated_at = Set(now);
    active_review.update(transaction).await?;
    Ok(())
}

fn validate_delegated_review_decision(
    decision: &DelegatedReviewDecision,
) -> Result<(), StoreError> {
    if !decision.confidence.is_finite() || !(0.0..=1.0).contains(&decision.confidence) {
        return Err(StoreError::InvalidModelRequest(
            "审核 Agent confidence 必须是 [0, 1] 内的有限数值".to_owned(),
        ));
    }
    let mut total = 0_usize;
    for issue in &decision.issues {
        if issue.trim().is_empty() {
            return Err(StoreError::InvalidModelRequest(
                "审核 Agent issues 不能包含空字符串".to_owned(),
            ));
        }
        total = total.saturating_add(issue.chars().count());
        if total > MAX_REVIEW_ISSUES_LEN {
            return Err(StoreError::InvalidModelRequest(format!(
                "审核 Agent issues 不能超过 {MAX_REVIEW_ISSUES_LEN} 个字符"
            )));
        }
    }
    Ok(())
}

fn delegated_review_decision_for_workflow(
    decision: &DelegatedReviewDecision,
) -> HumanReviewDecision {
    let issues = decision.issues.join("\n");
    HumanReviewDecision {
        passed: decision.passed,
        issues: if !decision.passed && issues.is_empty() {
            "审核未通过，审核 Agent 未提供具体问题".to_owned()
        } else {
            issues
        },
    }
}

async fn persist_delegated_review_decision(
    transaction: &DatabaseTransaction,
    review_id: ReviewId,
    review_run_id: RunId,
    decision: &DelegatedReviewDecision,
    now: i64,
) -> Result<(), StoreError> {
    let review_model = review::Entity::find_by_id(db_id(review_id))
        .filter(review::Entity::COLUMN.status.eq("delegated"))
        .filter(
            review::Entity::COLUMN
                .attempt_id
                .eq(review_run_id.to_string()),
        )
        .one(transaction)
        .await?
        .ok_or(StoreError::ReviewAlreadyResolved)?;
    let status = if decision.passed {
        "approved"
    } else {
        "rejected"
    };
    let notes = review_model
        .user_input_json
        .as_deref()
        .map(delegated_review_notes)
        .transpose()?
        .unwrap_or_default();
    let issues = delegated_review_decision_for_workflow(decision).issues;
    let mut active_review = review_model.into_active_model();
    active_review.status = Set(status.to_owned());
    active_review.decision_source = Set(Some("agent".to_owned()));
    active_review.passed = Set(Some(decision.passed));
    active_review.confidence = Set(Some(decision.confidence));
    active_review.issues_json = Set((!issues.is_empty())
        .then(|| serde_json::to_string(&issues))
        .transpose()?);
    active_review.user_input_json = Set(Some(serde_json::to_string(&json!({
        "source": "agent",
        "notes": notes,
        "issues": decision.issues,
        "confidence": decision.confidence,
    }))?));
    active_review.updated_at = Set(now);
    active_review.update(transaction).await?;
    Ok(())
}

async fn apply_delegated_preparation_review(
    transaction: &DatabaseTransaction,
    review_id: ReviewId,
    review_run_id: RunId,
    delegated: &DelegatedReviewDecision,
    now: i64,
) -> Result<VersionId, StoreError> {
    let review_target = preparation_review_target(transaction, review_id).await?;
    if review_target.status != ReviewStatus::Delegated {
        return Err(StoreError::ReviewAlreadyResolved);
    }
    let workflow = workflow_for_version(transaction, review_target.version_id).await?;
    if workflow.stage != WorkflowStage::PreparingReviewing
        || workflow.execution != ExecutionStatus::WaitingReview
    {
        return Err(StoreError::InvalidPreparationState(
            "当前工作流不再等待准备产物审核".to_owned(),
        ));
    }
    if review_target.current_revision_id != review_target.target_revision_id {
        return Err(StoreError::InvalidPreparationState(
            "审核目标已不是当前修订，不能应用委托结果".to_owned(),
        ));
    }
    persist_delegated_review_decision(transaction, review_id, review_run_id, delegated, now)
        .await?;
    let decision = delegated_review_decision_for_workflow(delegated);
    let current_artifacts =
        current_preparation_artifacts(transaction, review_target.version_id).await?;
    let has_open_reviews = current_artifacts.iter().any(|artifact| {
        matches!(
            artifact.review.as_ref().map(|review| &review.status),
            Some(ReviewStatus::Open | ReviewStatus::Delegated)
        )
    });
    let (next_workflow, run_status) = if has_open_reviews {
        (record_preparation_review_decision(&workflow)?, None)
    } else if current_preparation_is_approved(transaction, &current_artifacts).await? {
        let active_run_id = workflow
            .active_run_id
            .ok_or_else(|| StoreError::CorruptRecord("准备审核缺少对应的活跃 Run".to_owned()))?;
        let context_fold =
            build_preparation_context_fold(transaction, active_run_id, &current_artifacts).await?;
        let first_chapter_id = materialize_chapter_plan(
            transaction,
            review_target.version_id,
            &current_artifacts,
            now,
        )
        .await?;
        let checkpoint = RunCheckpoint {
            chapter_id: None,
            context_fold: Some(context_fold),
            held_chapter_body_submission: None,
        };
        update_preparation_run_status(
            transaction,
            active_run_id,
            "succeeded",
            Some(&checkpoint),
            Some(now),
            now,
        )
        .await?;
        let mut next_workflow =
            finish_preparation_review(&workflow, PreparationReviewCompletion::AllApproved)?;
        next_workflow.active_chapter_id = Some(first_chapter_id);
        (next_workflow, Some((active_run_id, "succeeded")))
    } else {
        let active_run_id = workflow
            .active_run_id
            .ok_or_else(|| StoreError::CorruptRecord("准备审核缺少对应的活跃 Run".to_owned()))?;
        update_preparation_run_status(transaction, active_run_id, "queued", None, None, now)
            .await?;
        (
            finish_preparation_review(&workflow, PreparationReviewCompletion::RevisionsRequired)?,
            Some((active_run_id, "queued")),
        )
    };
    persist_workflow(transaction, &next_workflow).await?;
    insert_event(
        transaction,
        review_target.version_id,
        "review.changed",
        review_id,
        next_workflow.state_revision,
        json!({
            "role": review_target.kind.stable_key(),
            "passed": decision.passed,
            "status": if decision.passed { "approved" } else { "rejected" },
            "source": "agent",
            "confidence": delegated.confidence,
        }),
        now,
    )
    .await?;
    if let Some((run_id, status)) = run_status {
        insert_event(
            transaction,
            review_target.version_id,
            "run.changed",
            run_id,
            next_workflow.state_revision,
            json!({ "kind": "preparation", "status": status }),
            now,
        )
        .await?;
    }
    insert_event(
        transaction,
        review_target.version_id,
        "workflow.changed",
        review_target.version_id,
        next_workflow.state_revision,
        json!({ "stage": next_workflow.stage }),
        now,
    )
    .await?;
    Ok(review_target.version_id)
}

async fn apply_delegated_chapter_outline_review(
    transaction: &DatabaseTransaction,
    review_id: ReviewId,
    review_run_id: RunId,
    delegated: &DelegatedReviewDecision,
    now: i64,
) -> Result<VersionId, StoreError> {
    let review_target = chapter_outline_review_target(transaction, review_id).await?;
    if review_target.status != ReviewStatus::Delegated {
        return Err(StoreError::ReviewAlreadyResolved);
    }
    let workflow = workflow_for_version(transaction, review_target.version_id).await?;
    if workflow.stage != WorkflowStage::WritingOutlineReviewing
        || workflow.execution != ExecutionStatus::WaitingReview
        || workflow.active_chapter_id != Some(review_target.chapter_id)
    {
        return Err(StoreError::InvalidChapterState(
            "当前工作流不再等待章节细纲审核".to_owned(),
        ));
    }
    if review_target.current_revision_id != review_target.target_revision_id {
        return Err(StoreError::InvalidChapterState(
            "审核目标已不是当前细纲修订，不能应用委托结果".to_owned(),
        ));
    }
    let expected_dependencies = chapter_outline_dependency_revision_ids(
        transaction,
        review_target.version_id,
        review_target.chapter_id,
    )
    .await?;
    let actual_dependencies =
        revision_dependency_ids(transaction, review_target.target_revision_id).await?;
    if review_target.dependency_revision_ids != expected_dependencies
        || actual_dependencies != expected_dependencies
    {
        return Err(StoreError::InvalidChapterState(
            "细纲审核依赖已变化，不能应用委托结果".to_owned(),
        ));
    }
    let parent_run_id = workflow
        .active_run_id
        .ok_or_else(|| StoreError::CorruptRecord("细纲审核缺少对应的活跃 Run".to_owned()))?;
    chapter_outline_run(transaction, parent_run_id, review_target.chapter_id).await?;
    persist_delegated_review_decision(transaction, review_id, review_run_id, delegated, now)
        .await?;
    let decision = delegated_review_decision_for_workflow(delegated);
    let (next_workflow, body_run_id) = if decision.passed {
        let outline = current_chapter_outline(transaction, review_target.chapter_id).await?;
        if outline.revision_id != Some(review_target.target_revision_id) {
            return Err(StoreError::CorruptRecord(
                "当前章节细纲在委托审核决定期间发生变化".to_owned(),
            ));
        }
        let content = outline
            .content
            .ok_or_else(|| StoreError::CorruptRecord("当前章节细纲缺少正文".to_owned()))?;
        let context_fold = build_chapter_outline_context_fold(
            transaction,
            parent_run_id,
            review_target.target_revision_id,
            &content,
        )
        .await?;
        let checkpoint = RunCheckpoint {
            chapter_id: Some(review_target.chapter_id),
            context_fold: Some(context_fold),
            held_chapter_body_submission: None,
        };
        update_chapter_outline_run_status(
            transaction,
            parent_run_id,
            review_target.chapter_id,
            "succeeded",
            Some(&checkpoint),
            Some(now),
            now,
        )
        .await?;
        let completed_outline = finish_chapter_outline_review(
            &workflow,
            review_target.chapter_id,
            ChapterOutlineReviewCompletion::Approved,
        )?;
        let body_run_id = Uuid::new_v4();
        let project_id = project_id_for_version(transaction, review_target.version_id).await?;
        ensure_no_active_top_level_write_run(transaction, project_id).await?;
        insert_chapter_body_run(
            transaction,
            body_run_id,
            project_id,
            review_target.version_id,
            review_target.chapter_id,
            now,
        )
        .await?;
        (
            schedule_chapter_body_run(&completed_outline, review_target.chapter_id, body_run_id)?,
            Some(body_run_id),
        )
    } else {
        update_chapter_outline_run_status(
            transaction,
            parent_run_id,
            review_target.chapter_id,
            "queued",
            None,
            None,
            now,
        )
        .await?;
        (
            finish_chapter_outline_review(
                &workflow,
                review_target.chapter_id,
                ChapterOutlineReviewCompletion::RevisionsRequired,
            )?,
            None,
        )
    };
    persist_workflow(transaction, &next_workflow).await?;
    insert_event(
        transaction,
        review_target.version_id,
        "review.changed",
        review_id,
        next_workflow.state_revision,
        json!({
            "chapter_id": review_target.chapter_id,
            "role": CHAPTER_OUTLINE_ROLE,
            "passed": decision.passed,
            "status": if decision.passed { "approved" } else { "rejected" },
            "source": "agent",
            "confidence": delegated.confidence,
        }),
        now,
    )
    .await?;
    if let Some(body_run_id) = body_run_id {
        insert_event(
            transaction,
            review_target.version_id,
            "run.changed",
            body_run_id,
            next_workflow.state_revision,
            json!({
                "chapter_id": review_target.chapter_id,
                "kind": "chapter_body",
                "status": "queued",
            }),
            now,
        )
        .await?;
    }
    insert_event(
        transaction,
        review_target.version_id,
        "run.changed",
        parent_run_id,
        next_workflow.state_revision,
        json!({
            "chapter_id": review_target.chapter_id,
            "kind": "chapter_outline",
            "status": if decision.passed { "succeeded" } else { "queued" },
        }),
        now,
    )
    .await?;
    insert_event(
        transaction,
        review_target.version_id,
        "workflow.changed",
        review_target.version_id,
        next_workflow.state_revision,
        json!({
            "stage": next_workflow.stage,
            "chapter_id": review_target.chapter_id,
            "run_id": next_workflow.active_run_id,
        }),
        now,
    )
    .await?;
    Ok(review_target.version_id)
}

async fn apply_delegated_chapter_final_review(
    transaction: &DatabaseTransaction,
    review_id: ReviewId,
    review_run_id: RunId,
    delegated: &DelegatedReviewDecision,
    now: i64,
) -> Result<VersionId, StoreError> {
    let review_target = chapter_final_review_target(transaction, review_id).await?;
    if review_target.status != ReviewStatus::Delegated {
        return Err(StoreError::ReviewAlreadyResolved);
    }
    let workflow = workflow_for_version(transaction, review_target.version_id).await?;
    if workflow.stage != WorkflowStage::WritingChapterReviewing
        || workflow.execution != ExecutionStatus::WaitingReview
        || workflow.active_chapter_id != Some(review_target.chapter_id)
    {
        return Err(StoreError::InvalidChapterState(
            "当前工作流不再等待整章终稿审核".to_owned(),
        ));
    }
    if review_target.current_revision_id != review_target.target_revision_id {
        return Err(StoreError::InvalidChapterState(
            "审核目标已不是当前正文修订，不能应用委托结果".to_owned(),
        ));
    }
    let body = current_chapter_body(transaction, review_target.chapter_id).await?;
    let body_content = body
        .content
        .ok_or_else(|| StoreError::CorruptRecord("当前章节正文缺少内容".to_owned()))?;
    nekowriter_core::validate_document_content(
        "章节正文",
        &body_content,
        nekowriter_core::MAX_ARTIFACT_CONTENT_LEN,
    )
    .map_err(|error| StoreError::InvalidChapterBodyEdit(error.to_string()))?;
    let outline_revision_id = chapter_body_outline_dependency_revision_id(
        transaction,
        review_target.version_id,
        review_target.chapter_id,
    )
    .await?;
    let actual_dependencies =
        revision_dependency_ids(transaction, review_target.target_revision_id).await?;
    if review_target.dependency_revision_ids != actual_dependencies
        || !actual_dependencies.contains(&outline_revision_id)
    {
        return Err(StoreError::InvalidChapterState(
            "整章审核依赖已变化，不能应用委托结果".to_owned(),
        ));
    }
    let parent_run_id = workflow
        .active_run_id
        .ok_or_else(|| StoreError::CorruptRecord("整章审核缺少对应的活跃 Run".to_owned()))?;
    let body_run = chapter_body_run(transaction, parent_run_id, review_target.chapter_id).await?;
    persist_delegated_review_decision(transaction, review_id, review_run_id, delegated, now)
        .await?;
    let decision = delegated_review_decision_for_workflow(delegated);
    // 与人工「退回修订」同一条通道：委托审核的意见也要随下一条正文编辑的结果交给
    // 正文 Agent，而不是只落在审核行里。docs/02 §6.2 里那句「空 issues 时把系统说明
    // 交给写作 Agent」正是靠这里才真的成立。
    let rejection_feedback = if decision.passed {
        None
    } else {
        Some(
            queue_chapter_body_feedback(transaction, &body_run, decision.issues.clone(), &[], now)
                .await?,
        )
    };
    let next_workflow = if decision.passed {
        let context_fold = build_chapter_body_context_fold(
            transaction,
            parent_run_id,
            review_target.target_revision_id,
            &body_content,
        )
        .await?;
        let checkpoint = RunCheckpoint {
            chapter_id: Some(review_target.chapter_id),
            context_fold: Some(context_fold),
            held_chapter_body_submission: None,
        };
        update_chapter_body_run_status(
            transaction,
            parent_run_id,
            review_target.chapter_id,
            "succeeded",
            Some(&checkpoint),
            Some(now),
            now,
        )
        .await?;
        let chapter_model = chapter::Entity::find_by_id(db_id(review_target.chapter_id))
            .filter(chapter::Entity::COLUMN.deleted_at.is_null())
            .one(transaction)
            .await?
            .ok_or(StoreError::ChapterNotFound)?;
        let chapter_kind = chapter_model.kind.clone();
        let chapter_next_revision = chapter_model.revision + 1;
        let mut active_chapter = chapter_model.into_active_model();
        active_chapter.status = Set("approved".to_owned());
        active_chapter.approved_revision_id =
            Set(Some(review_target.target_revision_id.to_string()));
        active_chapter.revision = Set(chapter_next_revision);
        active_chapter.updated_at = Set(now);
        active_chapter.update(transaction).await?;
        match chapter_kind.as_str() {
            "main" => finish_chapter_final_review(
                &workflow,
                review_target.chapter_id,
                ChapterFinalReviewCompletion::Approved,
            )?,
            "extra" => finish_extra_chapter_final_review(&workflow, review_target.chapter_id)?,
            kind => {
                return Err(StoreError::CorruptRecord(format!(
                    "终稿审核目标使用了未知章节类型：{kind}"
                )));
            }
        }
    } else {
        update_chapter_body_run_status(
            transaction,
            parent_run_id,
            review_target.chapter_id,
            "queued",
            None,
            None,
            now,
        )
        .await?;
        finish_chapter_final_review(
            &workflow,
            review_target.chapter_id,
            ChapterFinalReviewCompletion::RevisionsRequired,
        )?
    };
    persist_workflow(transaction, &next_workflow).await?;
    insert_event(
        transaction,
        review_target.version_id,
        "review.changed",
        review_id,
        next_workflow.state_revision,
        json!({
            "chapter_id": review_target.chapter_id,
            "role": CHAPTER_BODY_ROLE,
            "passed": decision.passed,
            "status": if decision.passed { "approved" } else { "rejected" },
            "source": "agent",
            "confidence": delegated.confidence,
        }),
        now,
    )
    .await?;
    insert_event(
        transaction,
        review_target.version_id,
        "run.changed",
        parent_run_id,
        next_workflow.state_revision,
        json!({
            "chapter_id": review_target.chapter_id,
            "kind": "chapter_body",
            "status": if decision.passed { "succeeded" } else { "queued" },
        }),
        now,
    )
    .await?;
    if let Some(feedback) = &rejection_feedback {
        insert_event(
            transaction,
            review_target.version_id,
            "feedback.changed",
            feedback.id,
            next_workflow.state_revision,
            json!({
                "chapter_id": review_target.chapter_id,
                "run_id": parent_run_id,
                "status": feedback_status_name(feedback.status),
                "sequence": feedback.sequence.to_string(),
            }),
            now,
        )
        .await?;
    }
    if decision.passed {
        insert_event(
            transaction,
            review_target.version_id,
            "chapter.changed",
            review_target.chapter_id,
            next_workflow.state_revision,
            json!({
                "status": "approved",
                "approved_revision_id": review_target.target_revision_id,
            }),
            now,
        )
        .await?;
    }
    insert_event(
        transaction,
        review_target.version_id,
        "workflow.changed",
        review_target.version_id,
        next_workflow.state_revision,
        json!({
            "stage": next_workflow.stage,
            "chapter_id": review_target.chapter_id,
            "run_id": next_workflow.active_run_id,
        }),
        now,
    )
    .await?;
    Ok(review_target.version_id)
}

async fn current_preparation_is_approved(
    transaction: &DatabaseTransaction,
    current_artifacts: &[CurrentPreparationArtifact],
) -> Result<bool, StoreError> {
    if current_artifacts.len() != PreparationArtifactKind::ALL.len() {
        return Ok(false);
    }
    let current_by_kind = current_artifacts
        .iter()
        .map(|artifact| (artifact.kind, artifact))
        .collect::<BTreeMap<_, _>>();
    let no_new_revisions = BTreeMap::<PreparationArtifactKind, (RevisionId, i64)>::new();

    for kind in PreparationArtifactKind::ALL {
        let artifact = current_by_kind.get(&kind).ok_or_else(|| {
            StoreError::CorruptRecord(format!("稿本缺少 {} 文档槽", kind.stable_key()))
        })?;
        let Some(revision_id) = artifact.revision_id else {
            return Ok(false);
        };
        if artifact.review.as_ref().map(|review| &review.status) != Some(&ReviewStatus::Approved) {
            return Ok(false);
        }
        let expected = dependency_revision_ids(kind, &current_by_kind, &no_new_revisions)?
            .into_iter()
            .collect::<BTreeSet<_>>();
        let actual = revision_dependency_ids(transaction, revision_id).await?;
        if actual != expected {
            return Ok(false);
        }
    }
    Ok(true)
}

/// 细纲提交所冻结的全部输入修订。
///
/// 准备阶段通过的四项产物和当前章指导共同构成细纲 Run 的可追溯输入。准备阶段
/// 已经锁定，指导在细纲启动后也不可修改；此处仍在提交和审核时双重核验，避免
/// 任何未来入口悄悄打破这一不变量。
async fn chapter_outline_dependency_revision_ids(
    transaction: &DatabaseTransaction,
    version_id: VersionId,
    chapter_id: ChapterId,
) -> Result<BTreeSet<RevisionId>, StoreError> {
    let current_artifacts = current_preparation_artifacts(transaction, version_id).await?;
    if !current_preparation_is_approved(transaction, &current_artifacts).await? {
        return Err(StoreError::InvalidChapterState(
            "准备阶段产物尚未全部以当前修订审核通过".to_owned(),
        ));
    }
    if latest_preparation_fold(transaction, version_id)
        .await?
        .is_none()
    {
        return Err(StoreError::InvalidChapterState(
            "准备阶段尚未完成上下文折叠".to_owned(),
        ));
    }
    let guidance = current_chapter_guidance(transaction, chapter_id).await?;
    let guidance_revision_id = guidance
        .revision_id
        .ok_or_else(|| StoreError::CorruptRecord("细纲提交时章节指导缺少输入修订".to_owned()))?;
    let mut dependency_revision_ids = BTreeSet::new();
    dependency_revision_ids.insert(guidance_revision_id);
    for artifact in current_artifacts {
        dependency_revision_ids.insert(artifact.revision_id.ok_or_else(|| {
            StoreError::CorruptRecord(format!(
                "细纲提交时 {} 缺少当前修订",
                artifact.kind.stable_key()
            ))
        })?);
    }
    Ok(dependency_revision_ids)
}

/// 获取当前正文阶段唯一允许引用的、已通过章节细纲修订。
async fn chapter_body_outline_dependency_revision_id(
    transaction: &DatabaseTransaction,
    version_id: VersionId,
    chapter_id: ChapterId,
) -> Result<RevisionId, StoreError> {
    let outline = current_chapter_outline(transaction, chapter_id).await?;
    let outline_revision_id = outline
        .revision_id
        .ok_or_else(|| StoreError::InvalidChapterState("正文开始前缺少当前章节细纲".to_owned()))?;
    if outline.review.as_ref().map(|review| &review.status) != Some(&ReviewStatus::Approved) {
        return Err(StoreError::InvalidChapterState(
            "当前章节细纲尚未审核通过".to_owned(),
        ));
    }
    let context_fold = latest_chapter_outline_fold(transaction, version_id, chapter_id)
        .await?
        .ok_or_else(|| {
            StoreError::InvalidChapterState("当前章节细纲尚未完成上下文折叠".to_owned())
        })?;
    if context_fold.artifact_revision_ids.as_slice() != [outline_revision_id] {
        return Err(StoreError::CorruptRecord(
            "章节细纲折叠来源与当前修订不一致".to_owned(),
        ));
    }
    Ok(outline_revision_id)
}

async fn chapter_body_revision_dependency_ids(
    transaction: &DatabaseTransaction,
    version_id: VersionId,
    chapter_id: ChapterId,
    base_revision_id: Option<RevisionId>,
) -> Result<BTreeSet<RevisionId>, StoreError> {
    let mut dependency_revision_ids = BTreeSet::new();
    dependency_revision_ids.insert(
        chapter_body_outline_dependency_revision_id(transaction, version_id, chapter_id).await?,
    );
    if let Some(base_revision_id) = base_revision_id {
        dependency_revision_ids.insert(base_revision_id);
    }
    Ok(dependency_revision_ids)
}

async fn revision_dependency_ids(
    transaction: &DatabaseTransaction,
    revision_id: RevisionId,
) -> Result<BTreeSet<RevisionId>, StoreError> {
    Ok(revision_dependency::Entity::find()
        .filter(
            revision_dependency::Entity::COLUMN
                .revision_id
                .eq(db_id(revision_id)),
        )
        .all(transaction)
        .await?
        .into_iter()
        .map(|dependency| dependency.dependency_revision_id.into())
        .collect())
}

async fn build_preparation_context_fold(
    transaction: &DatabaseTransaction,
    run_id: Uuid,
    current_artifacts: &[CurrentPreparationArtifact],
) -> Result<ContextFold, StoreError> {
    let artifacts = PreparationArtifactKind::ALL
        .into_iter()
        .map(|kind| {
            let artifact = current_artifacts
                .iter()
                .find(|artifact| artifact.kind == kind)
                .ok_or_else(|| {
                    StoreError::CorruptRecord(format!("折叠时缺少 {} 产物", kind.stable_key()))
                })?;
            Ok(nekowriter_core::ApprovedPreparationArtifact {
                kind,
                revision_id: artifact.revision_id.ok_or_else(|| {
                    StoreError::CorruptRecord(format!("折叠时 {} 缺少当前修订", kind.stable_key()))
                })?,
                content: artifact.content.clone().ok_or_else(|| {
                    StoreError::CorruptRecord(format!("折叠时 {} 缺少当前内容", kind.stable_key()))
                })?,
                chapter_plan: artifact.chapter_plan.clone(),
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    let plain_text = render_preparation_artifact_plaintext(&artifacts)
        .map_err(|error| StoreError::CorruptRecord(error.to_string()))?;
    let through_message_sequence = latest_run_message_sequence(transaction, run_id).await?;
    let artifact_revision_ids = artifacts
        .iter()
        .map(|artifact| artifact.revision_id)
        .collect::<Vec<_>>();
    Ok(ContextFold {
        artifact_revision_ids,
        through_message_sequence,
        content_hash: format!("{:x}", Sha256::digest(plain_text.as_bytes())),
        plain_text,
    })
}

/// 将一份审核通过的章节细纲折叠为模型可见的唯一文本块。
///
/// 不拼接章节标题、审核结论或 Run 元数据；这些信息仍分别保存在类型化记录中，
/// 但不再作为下一阶段 Agent 的上下文内容。
async fn build_chapter_outline_context_fold(
    transaction: &DatabaseTransaction,
    run_id: Uuid,
    revision_id: RevisionId,
    content: &str,
) -> Result<ContextFold, StoreError> {
    let plain_text = nekowriter_core::validate_document_content(
        "章节细纲",
        content,
        nekowriter_core::MAX_ARTIFACT_CONTENT_LEN,
    )
    .map_err(|error| StoreError::CorruptRecord(error.to_string()))?;
    Ok(ContextFold {
        artifact_revision_ids: vec![revision_id],
        through_message_sequence: latest_run_message_sequence(transaction, run_id).await?,
        content_hash: format!("{:x}", Sha256::digest(plain_text.as_bytes())),
        plain_text,
    })
}

/// 将整章终稿审核通过的正文折叠为下一章可见的唯一文本块。
async fn build_chapter_body_context_fold(
    transaction: &DatabaseTransaction,
    run_id: Uuid,
    revision_id: RevisionId,
    content: &str,
) -> Result<ContextFold, StoreError> {
    let plain_text = nekowriter_core::validate_document_content(
        "章节正文",
        content,
        nekowriter_core::MAX_ARTIFACT_CONTENT_LEN,
    )
    .map_err(|error| StoreError::CorruptRecord(error.to_string()))?;
    Ok(ContextFold {
        artifact_revision_ids: vec![revision_id],
        through_message_sequence: latest_run_message_sequence(transaction, run_id).await?,
        content_hash: format!("{:x}", Sha256::digest(plain_text.as_bytes())),
        plain_text,
    })
}

pub(crate) async fn insert_chapter_document_slots<C: ConnectionTrait>(
    connection: &C,
    version_id: VersionId,
    chapter_id: ChapterId,
    now: i64,
) -> Result<(), StoreError> {
    for role in [
        CHAPTER_GUIDANCE_ROLE,
        CHAPTER_OUTLINE_ROLE,
        CHAPTER_BODY_ROLE,
    ] {
        document::ActiveModel {
            id: Set(db_id(Uuid::new_v4())),
            version_id: Set(db_id(version_id)),
            chapter_id: Set(Some(chapter_id.to_string())),
            role: Set(role.to_owned()),
            current_revision_id: Set(None),
            revision: Set(0),
            created_at: Set(now),
            updated_at: Set(now),
            deleted_at: Set(None),
            ..Default::default()
        }
        .insert(connection)
        .await?;
    }
    Ok(())
}

/// 将已通过粗纲中的计划项物化为正文阶段的章节行。
///
/// 该函数只在准备阶段最终通过的同一事务中调用；若计划或现有章节不满足预期，
/// 整个审核决定都会回滚，不会出现已经进入正文阶段却没有可写章节的状态。
async fn materialize_chapter_plan(
    transaction: &DatabaseTransaction,
    version_id: VersionId,
    current_artifacts: &[CurrentPreparationArtifact],
    now: i64,
) -> Result<Uuid, StoreError> {
    let outline = current_artifacts
        .iter()
        .find(|artifact| artifact.kind == PreparationArtifactKind::Outline)
        .ok_or_else(|| StoreError::CorruptRecord("折叠时缺少小说粗纲".to_owned()))?;
    let plan = outline
        .chapter_plan
        .as_ref()
        .ok_or_else(|| StoreError::CorruptRecord("审核通过的小说粗纲缺少章节计划".to_owned()))?;
    plan.validate()
        .map_err(|error| StoreError::CorruptRecord(format!("章节计划无效：{error}")))?;

    if chapter::Entity::find()
        .filter(chapter::Entity::COLUMN.version_id.eq(db_id(version_id)))
        .filter(chapter::Entity::COLUMN.deleted_at.is_null())
        .one(transaction)
        .await?
        .is_some()
    {
        return Err(StoreError::CorruptRecord(
            "准备阶段通过前稿本已经存在有效章节".to_owned(),
        ));
    }

    let mut first_chapter_id = None;
    for item in &plan.items {
        let chapter_id = Uuid::new_v4();
        chapter::ActiveModel {
            id: Set(db_id(chapter_id)),
            version_id: Set(db_id(version_id)),
            kind: Set("main".to_owned()),
            plan_item_id: Set(Some(item.plan_item_id.to_string())),
            order_index: Set(item.order_index),
            title: Set(item.title.clone()),
            status: Set("planned".to_owned()),
            approved_revision_id: Set(None),
            revision: Set(0),
            created_at: Set(now),
            updated_at: Set(now),
            deleted_at: Set(None),
            ..Default::default()
        }
        .insert(transaction)
        .await?;
        insert_chapter_document_slots(transaction, version_id, chapter_id, now).await?;
        first_chapter_id.get_or_insert(chapter_id);
    }
    first_chapter_id.ok_or_else(|| StoreError::CorruptRecord("章节计划为空".to_owned()))
}

async fn latest_run_message_sequence(
    transaction: &DatabaseTransaction,
    run_id: Uuid,
) -> Result<i64, StoreError> {
    Ok(agent_message::Entity::find()
        .filter(agent_message::Entity::COLUMN.run_id.eq(db_id(run_id)))
        .order_by_desc(agent_message::Entity::COLUMN.sequence)
        .one(transaction)
        .await?
        .map_or(0, |message| message.sequence))
}

pub(crate) async fn update_preparation_run_status(
    transaction: &DatabaseTransaction,
    run_id: Uuid,
    status: &str,
    checkpoint: Option<&RunCheckpoint>,
    ended_at: Option<i64>,
    now: i64,
) -> Result<(), StoreError> {
    let run_model = run::Entity::find_by_id(db_id(run_id))
        .filter(run::Entity::COLUMN.kind.eq("preparation"))
        .one(transaction)
        .await?
        .ok_or_else(|| StoreError::CorruptRecord("准备阶段 Run 不存在".to_owned()))?;
    let mut active_run = run_model.into_active_model();
    active_run.status = Set(status.to_owned());
    if let Some(checkpoint) = checkpoint {
        active_run.checkpoint_json = Set(serde_json::to_string(checkpoint)?);
    }
    if let Some(ended_at) = ended_at {
        active_run.ended_at = Set(Some(ended_at));
    }
    active_run.updated_at = Set(now);
    active_run.update(transaction).await?;
    Ok(())
}

/// 一次合规模型响应成功进入任一稳定业务边界后，清零本 Run 的连续协议错误计数。
///
/// 计数只由模型响应驱动；用户回答、审核决定或普通暂停恢复不会调用此函数。单独的
/// 小更新仍处于同一外层 SQLite 事务中，避免把错误次数清零而状态写入失败。
pub(crate) async fn reset_run_protocol_errors(
    transaction: &DatabaseTransaction,
    run_id: RunId,
    now: i64,
) -> Result<(), StoreError> {
    let run_model = run::Entity::find_by_id(db_id(run_id))
        .one(transaction)
        .await?
        .ok_or(StoreError::RunNotFound)?;
    if run_model.consecutive_protocol_errors < 0 {
        return Err(StoreError::CorruptRecord(
            "Run consecutive_protocol_errors 不能为负数".to_owned(),
        ));
    }
    if run_model.consecutive_protocol_errors == 0 {
        return Ok(());
    }
    let mut active_run = run_model.into_active_model();
    active_run.consecutive_protocol_errors = Set(0);
    active_run.updated_at = Set(now);
    active_run.update(transaction).await?;
    Ok(())
}

async fn chapter_outline_run(
    transaction: &DatabaseTransaction,
    run_id: Uuid,
    chapter_id: ChapterId,
) -> Result<run::Model, StoreError> {
    let run_model = run::Entity::find_by_id(db_id(run_id))
        .filter(run::Entity::COLUMN.kind.eq("chapter_outline"))
        .one(transaction)
        .await?
        .ok_or_else(|| StoreError::CorruptRecord("章节细纲 Run 不存在".to_owned()))?;
    let checkpoint: RunCheckpoint = serde_json::from_str(&run_model.checkpoint_json)?;
    if checkpoint.chapter_id != Some(chapter_id) {
        return Err(StoreError::CorruptRecord(
            "章节细纲 Run 与当前章节不匹配".to_owned(),
        ));
    }
    Ok(run_model)
}

async fn update_chapter_outline_run_status(
    transaction: &DatabaseTransaction,
    run_id: Uuid,
    chapter_id: ChapterId,
    status: &str,
    checkpoint: Option<&RunCheckpoint>,
    ended_at: Option<i64>,
    now: i64,
) -> Result<(), StoreError> {
    let run_model = chapter_outline_run(transaction, run_id, chapter_id).await?;
    if let Some(checkpoint) = checkpoint
        && checkpoint.chapter_id != Some(chapter_id)
    {
        return Err(StoreError::CorruptRecord(
            "章节细纲 Run 检查点与当前章节不匹配".to_owned(),
        ));
    }
    let mut active_run = run_model.into_active_model();
    active_run.status = Set(status.to_owned());
    if let Some(checkpoint) = checkpoint {
        active_run.checkpoint_json = Set(serde_json::to_string(checkpoint)?);
    }
    if let Some(ended_at) = ended_at {
        active_run.ended_at = Set(Some(ended_at));
    }
    active_run.updated_at = Set(now);
    active_run.update(transaction).await?;
    Ok(())
}

async fn chapter_body_run(
    transaction: &DatabaseTransaction,
    run_id: Uuid,
    chapter_id: ChapterId,
) -> Result<run::Model, StoreError> {
    let run_model = run::Entity::find_by_id(db_id(run_id))
        .filter(run::Entity::COLUMN.kind.eq("chapter_body"))
        .one(transaction)
        .await?
        .ok_or_else(|| StoreError::CorruptRecord("章节正文 Run 不存在".to_owned()))?;
    let checkpoint: RunCheckpoint = serde_json::from_str(&run_model.checkpoint_json)?;
    if checkpoint.chapter_id != Some(chapter_id) {
        return Err(StoreError::CorruptRecord(
            "章节正文 Run 与当前章节不匹配".to_owned(),
        ));
    }
    Ok(run_model)
}

async fn update_chapter_body_run_status(
    transaction: &DatabaseTransaction,
    run_id: Uuid,
    chapter_id: ChapterId,
    status: &str,
    checkpoint: Option<&RunCheckpoint>,
    ended_at: Option<i64>,
    now: i64,
) -> Result<(), StoreError> {
    let run_model = chapter_body_run(transaction, run_id, chapter_id).await?;
    if let Some(checkpoint) = checkpoint
        && checkpoint.chapter_id != Some(chapter_id)
    {
        return Err(StoreError::CorruptRecord(
            "章节正文 Run 检查点与当前章节不匹配".to_owned(),
        ));
    }
    let mut active_run = run_model.into_active_model();
    active_run.status = Set(status.to_owned());
    if let Some(checkpoint) = checkpoint {
        active_run.checkpoint_json = Set(serde_json::to_string(checkpoint)?);
    }
    if let Some(ended_at) = ended_at {
        active_run.ended_at = Set(Some(ended_at));
    }
    active_run.updated_at = Set(now);
    active_run.update(transaction).await?;
    Ok(())
}

pub(crate) async fn preparation_snapshot<C: ConnectionTrait>(
    connection: &C,
    version_id: VersionId,
) -> Result<PreparationSnapshot, StoreError> {
    let workflow = workflow_for_version(connection, version_id).await?;
    let novel_brief = current_novel_brief(connection, version_id).await?;
    let artifacts = current_preparation_artifacts(connection, version_id).await?;
    let context_fold = latest_preparation_fold(connection, version_id).await?;
    Ok(PreparationSnapshot {
        version_id,
        pending_question: pending_user_question(connection, &workflow).await?,
        workflow,
        novel_brief,
        artifacts,
        context_fold,
    })
}

pub(crate) async fn workflow_for_version<C: ConnectionTrait>(
    connection: &C,
    version_id: VersionId,
) -> Result<WorkflowSnapshot, StoreError> {
    let workflow_model = workflow_instance::Entity::find_by_id(db_id(version_id))
        .one(connection)
        .await?
        .ok_or(StoreError::VersionNotFound)?;
    workflow_from_model(&workflow_model)
}

async fn current_novel_brief<C: ConnectionTrait>(
    connection: &C,
    version_id: VersionId,
) -> Result<CurrentNovelBrief, StoreError> {
    let document_model = document::Entity::find()
        .filter(document::Entity::COLUMN.version_id.eq(db_id(version_id)))
        .filter(document::Entity::COLUMN.role.eq(NOVEL_BRIEF_ROLE))
        .filter(document::Entity::COLUMN.chapter_id.is_null())
        .filter(document::Entity::COLUMN.deleted_at.is_null())
        .one(connection)
        .await?
        .ok_or_else(|| StoreError::CorruptRecord("稿本缺少小说目标文档槽".to_owned()))?;
    let revision_id = parse_optional_id(
        document_model.current_revision_id.clone(),
        "current_revision_id",
    )?;
    let content = match revision_id {
        Some(revision_id) => Some(
            document_revision::Entity::find_by_id(db_id(revision_id))
                .one(connection)
                .await?
                .ok_or_else(|| StoreError::CorruptRecord("小说目标当前修订不存在".to_owned()))?
                .content,
        ),
        None => None,
    };
    Ok(CurrentNovelBrief {
        document_id: document_model.id.into(),
        revision_id,
        content,
    })
}

async fn current_preparation_artifacts<C: ConnectionTrait>(
    connection: &C,
    version_id: VersionId,
) -> Result<Vec<CurrentPreparationArtifact>, StoreError> {
    let roles = PreparationArtifactKind::ALL
        .into_iter()
        .map(|kind| kind.stable_key().to_owned())
        .collect::<Vec<_>>();
    let documents = document::Entity::find()
        .filter(document::Entity::COLUMN.version_id.eq(db_id(version_id)))
        .filter(document::Entity::COLUMN.chapter_id.is_null())
        .filter(document::Entity::COLUMN.deleted_at.is_null())
        .filter(document::Entity::COLUMN.role.is_in(roles))
        .all(connection)
        .await?;
    let mut documents_by_role = HashMap::with_capacity(documents.len());
    for document_model in documents {
        if documents_by_role
            .insert(document_model.role.clone(), document_model)
            .is_some()
        {
            return Err(StoreError::CorruptRecord(
                "同一稿本存在重复的准备产物文档槽".to_owned(),
            ));
        }
    }
    let mut current_revision_ids = HashSet::new();
    for document_model in documents_by_role.values() {
        if let Some(revision_id) = parse_optional_id(
            document_model.current_revision_id.clone(),
            "current_revision_id",
        )? {
            current_revision_ids.insert(db_id(revision_id));
        }
    }
    let revision_ids = current_revision_ids.iter().copied().collect::<Vec<_>>();
    let revisions = if revision_ids.is_empty() {
        Vec::new()
    } else {
        document_revision::Entity::find()
            .filter(
                document_revision::Entity::COLUMN
                    .id
                    .is_in(revision_ids.clone()),
            )
            .all(connection)
            .await?
    };
    let revisions_by_id = revisions
        .into_iter()
        .map(|revision| (revision.id, revision))
        .collect::<HashMap<_, _>>();
    let reviews = review::Entity::find()
        .filter(review::Entity::COLUMN.version_id.eq(db_id(version_id)))
        .filter(review::Entity::COLUMN.review_type.eq("preparation"))
        .order_by_desc(review::Entity::COLUMN.created_at)
        .order_by_desc(review::Entity::COLUMN.id)
        .all(connection)
        .await?;
    let mut latest_reviews_by_revision = HashMap::new();
    for review_model in reviews {
        let Some(target_revision_id) = review_model.target_revision_id.as_deref() else {
            continue;
        };
        let target_revision_id = db_id(parse_id(target_revision_id, "target_revision_id")?);
        if current_revision_ids.contains(&target_revision_id) {
            latest_reviews_by_revision
                .entry(target_revision_id)
                .or_insert(review_model);
        }
    }

    let mut artifacts = Vec::with_capacity(PreparationArtifactKind::ALL.len());
    for kind in PreparationArtifactKind::ALL {
        let document_model = documents_by_role.get(kind.stable_key()).ok_or_else(|| {
            StoreError::CorruptRecord(format!("稿本缺少 {} 文档槽", kind.stable_key()))
        })?;
        let revision_id = parse_optional_id(
            document_model.current_revision_id.clone(),
            "current_revision_id",
        )?;
        let revision = match revision_id {
            Some(revision_id) => {
                Some(revisions_by_id.get(&db_id(revision_id)).ok_or_else(|| {
                    StoreError::CorruptRecord(format!("{} 的当前修订不存在", kind.stable_key()))
                })?)
            }
            None => None,
        };
        let review = match revision_id {
            Some(revision_id) => latest_reviews_by_revision
                .get(&db_id(revision_id))
                .map(|review| preparation_review_from_model(review, kind))
                .transpose()?,
            None => None,
        };
        let chapter_plan = if kind == PreparationArtifactKind::Outline {
            revision
                .and_then(|revision| revision.structured_payload.as_deref())
                .map(|payload| {
                    let plan: ChapterPlan = serde_json::from_str(payload).map_err(|error| {
                        StoreError::CorruptRecord(format!(
                            "粗纲当前修订的章节计划无法解析：{error}"
                        ))
                    })?;
                    plan.validate().map_err(|error| {
                        StoreError::CorruptRecord(format!("粗纲当前修订的章节计划无效：{error}"))
                    })?;
                    Ok::<ChapterPlan, StoreError>(plan)
                })
                .transpose()?
        } else {
            None
        };
        artifacts.push(CurrentPreparationArtifact {
            document_id: document_model.id.into(),
            kind,
            revision_id,
            revision_no: revision.map(|revision| revision.revision_no),
            content: revision.map(|revision| revision.content.clone()),
            chapter_plan,
            review,
        });
    }
    Ok(artifacts)
}

async fn latest_preparation_fold<C: ConnectionTrait>(
    connection: &C,
    version_id: VersionId,
) -> Result<Option<ContextFold>, StoreError> {
    let preparation_run = run::Entity::find()
        .filter(run::Entity::COLUMN.version_id.eq(db_id(version_id)))
        .filter(run::Entity::COLUMN.kind.eq("preparation"))
        .filter(run::Entity::COLUMN.status.eq("succeeded"))
        .order_by_desc(run::Entity::COLUMN.ended_at)
        .order_by_desc(run::Entity::COLUMN.id)
        .one(connection)
        .await?;
    let run_fold = preparation_run
        .map(|run| -> Result<Option<ContextFold>, StoreError> {
            let checkpoint: RunCheckpoint = serde_json::from_str(&run.checkpoint_json)?;
            Ok(checkpoint.context_fold)
        })
        .transpose()?
        .flatten();
    if run_fold.is_some() {
        return Ok(run_fold);
    }
    let workflow = workflow_for_version(connection, version_id).await?;
    let copied_folds = workflow
        .copied_context_folds
        .iter()
        .filter(|fold| fold.stage == CopiedContextFoldStage::Preparation)
        .collect::<Vec<_>>();
    match copied_folds.as_slice() {
        [] => Ok(None),
        [fold] => Ok(Some(fold.fold.clone())),
        _ => Err(StoreError::CorruptRecord(
            "复制稿本存在重复的准备阶段上下文折叠".to_owned(),
        )),
    }
}

async fn completed_run_context_folds<C: ConnectionTrait>(
    connection: &C,
    version_id: VersionId,
) -> Result<Vec<CompletedRunContextFold>, StoreError> {
    let runs = run::Entity::find()
        .filter(run::Entity::COLUMN.version_id.eq(db_id(version_id)))
        .filter(run::Entity::COLUMN.status.eq("succeeded"))
        .filter(
            run::Entity::COLUMN
                .kind
                .is_in(["preparation", "chapter_outline", "chapter_body"]),
        )
        .all(connection)
        .await?;
    let workflow = workflow_for_version(connection, version_id).await?;
    let mut folds = Vec::with_capacity(runs.len() + workflow.copied_context_folds.len());
    for run_model in runs {
        let checkpoint: RunCheckpoint = serde_json::from_str(&run_model.checkpoint_json)?;
        let (stage, chapter_order) = match run_model.kind.as_str() {
            "preparation" => {
                if checkpoint.chapter_id.is_some() {
                    return Err(StoreError::CorruptRecord(
                        "准备阶段 Run 检查点不应包含章节 ID".to_owned(),
                    ));
                }
                (CompletedRunContextStage::Preparation, None)
            }
            "chapter_outline" | "chapter_body" => {
                let chapter_id = checkpoint.chapter_id.ok_or_else(|| {
                    StoreError::CorruptRecord("章节阶段 Run 检查点缺少章节 ID".to_owned())
                })?;
                let chapter_model = chapter::Entity::find_by_id(db_id(chapter_id))
                    .one(connection)
                    .await?
                    .ok_or(StoreError::ChapterNotFound)?;
                if chapter_model.version_id != run_model.version_id
                    || chapter_model.deleted_at.is_some()
                {
                    return Err(StoreError::CorruptRecord(
                        "章节阶段 Run 与有效章节不属于同一稿本".to_owned(),
                    ));
                }
                let stage = if run_model.kind == "chapter_outline" {
                    CompletedRunContextStage::ChapterOutline
                } else {
                    CompletedRunContextStage::ChapterBody
                };
                (stage, Some(chapter_model.order_index))
            }
            _ => unreachable!("查询已按 Run kind 过滤"),
        };
        let fold = checkpoint.context_fold.ok_or_else(|| {
            StoreError::CorruptRecord("已完成阶段 Run 缺少上下文折叠检查点".to_owned())
        })?;
        folds.push(CompletedRunContextFold {
            stage,
            chapter_order,
            source_run_id: run_model.id.into(),
            fold,
        });
    }
    for copied in workflow.copied_context_folds {
        let stage = match copied.stage {
            CopiedContextFoldStage::Preparation => CompletedRunContextStage::Preparation,
            CopiedContextFoldStage::ChapterOutline => CompletedRunContextStage::ChapterOutline,
            CopiedContextFoldStage::ChapterBody => CompletedRunContextStage::ChapterBody,
        };
        if (stage == CompletedRunContextStage::Preparation) != copied.chapter_order.is_none() {
            return Err(StoreError::CorruptRecord(
                "复制稿本上下文折叠的章节顺序不符合阶段类型".to_owned(),
            ));
        }
        folds.push(CompletedRunContextFold {
            stage,
            chapter_order: copied.chapter_order,
            source_run_id: copied.id,
            fold: copied.fold,
        });
    }
    Ok(folds)
}

/// 把审核行中的置信度还原成对外快照的数值。
///
/// SQLite 保存 `Option<f64>`，而快照合同使用 `serde_json::Number` 以便原样携带
/// 供应商返回的数值；不可表示的 NaN/无穷值属于损坏记录而不是可展示的置信度。
fn review_confidence_from_model(
    model: &review::Model,
) -> Result<Option<serde_json::Number>, StoreError> {
    model
        .confidence
        .map(|confidence| {
            serde_json::Number::from_f64(confidence).ok_or_else(|| {
                StoreError::CorruptRecord("审核置信度不是可表示的有限数值".to_owned())
            })
        })
        .transpose()
}

fn preparation_review_from_model(
    model: &review::Model,
    kind: PreparationArtifactKind,
) -> Result<PreparationReview, StoreError> {
    if model.review_type != "preparation" {
        return Err(StoreError::CorruptRecord(
            "审核类型不是准备阶段审核".to_owned(),
        ));
    }
    Ok(PreparationReview {
        id: model.id.into(),
        kind,
        target_revision_id: parse_id(
            model
                .target_revision_id
                .as_deref()
                .ok_or_else(|| StoreError::CorruptRecord("准备审核缺少目标修订".to_owned()))?,
            "target_revision_id",
        )?,
        status: review_status_from_db(model.status.clone())?,
        passed: model.passed,
        confidence: review_confidence_from_model(model)?,
        issues: model
            .issues_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?,
    })
}

fn chapter_outline_review_from_model(
    model: &review::Model,
) -> Result<ChapterOutlineReview, StoreError> {
    if model.review_type != "chapter_outline" {
        return Err(StoreError::CorruptRecord(
            "审核类型不是章节细纲审核".to_owned(),
        ));
    }
    Ok(ChapterOutlineReview {
        id: model.id.into(),
        target_revision_id: parse_id(
            model
                .target_revision_id
                .as_deref()
                .ok_or_else(|| StoreError::CorruptRecord("细纲审核缺少目标修订".to_owned()))?,
            "target_revision_id",
        )?,
        status: review_status_from_db(model.status.clone())?,
        passed: model.passed,
        confidence: review_confidence_from_model(model)?,
        issues: model
            .issues_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?,
    })
}

fn chapter_final_review_from_model(
    model: &review::Model,
) -> Result<ChapterFinalReview, StoreError> {
    if model.review_type != "chapter_final" {
        return Err(StoreError::CorruptRecord(
            "审核类型不是整章终稿审核".to_owned(),
        ));
    }
    Ok(ChapterFinalReview {
        id: model.id.into(),
        target_revision_id: parse_id(
            model
                .target_revision_id
                .as_deref()
                .ok_or_else(|| StoreError::CorruptRecord("整章审核缺少目标修订".to_owned()))?,
            "target_revision_id",
        )?,
        status: review_status_from_db(model.status.clone())?,
        passed: model.passed,
        confidence: review_confidence_from_model(model)?,
        issues: model
            .issues_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?,
    })
}

fn chapter_edit_review_from_model(model: &review::Model) -> Result<ChapterEditReview, StoreError> {
    if model.review_type != "chapter_edit" {
        return Err(StoreError::CorruptRecord(
            "审核类型不是正文编辑审核".to_owned(),
        ));
    }
    Ok(ChapterEditReview {
        id: model.id.into(),
        target_tool_call_id: parse_id(
            model.target_tool_call_id.as_deref().ok_or_else(|| {
                StoreError::CorruptRecord("正文编辑审核缺少目标工具调用".to_owned())
            })?,
            "target_tool_call_id",
        )?,
        status: review_status_from_db(model.status.clone())?,
        passed: model.passed,
        confidence: review_confidence_from_model(model)?,
        issues: model
            .issues_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?,
    })
}

fn review_status_from_db(value: String) -> Result<ReviewStatus, StoreError> {
    match value.as_str() {
        "open" => Ok(ReviewStatus::Open),
        "delegated" => Ok(ReviewStatus::Delegated),
        "approved" => Ok(ReviewStatus::Approved),
        "rejected" => Ok(ReviewStatus::Rejected),
        "stale" => Ok(ReviewStatus::Stale),
        _ => Err(StoreError::CorruptRecord(format!("未知审核状态：{value}"))),
    }
}

async fn insert_document_revision(
    transaction: &DatabaseTransaction,
    document_id: DocumentId,
    content: &str,
    created_by: &str,
    now: i64,
) -> Result<(RevisionId, i64), StoreError> {
    insert_document_revision_with_payload(transaction, document_id, content, None, created_by, now)
        .await
}

async fn insert_document_revision_with_payload(
    transaction: &DatabaseTransaction,
    document_id: DocumentId,
    content: &str,
    structured_payload: Option<&str>,
    created_by: &str,
    now: i64,
) -> Result<(RevisionId, i64), StoreError> {
    insert_document_revision_with_payload_and_source(
        transaction,
        document_id,
        content,
        structured_payload,
        created_by,
        None,
        now,
    )
    .await
}

async fn insert_document_revision_with_payload_and_source(
    transaction: &DatabaseTransaction,
    document_id: DocumentId,
    content: &str,
    structured_payload: Option<&str>,
    created_by: &str,
    source_revision_id: Option<RevisionId>,
    now: i64,
) -> Result<(RevisionId, i64), StoreError> {
    let revision_no = document_revision::Entity::find()
        .filter(
            document_revision::Entity::COLUMN
                .document_id
                .eq(db_id(document_id)),
        )
        .order_by_desc(document_revision::Entity::COLUMN.revision_no)
        .one(transaction)
        .await?
        .map_or(1, |revision| revision.revision_no + 1);
    let revision_id = Uuid::new_v4();
    document_revision::ActiveModel {
        id: Set(db_id(revision_id)),
        document_id: Set(db_id(document_id)),
        revision_no: Set(revision_no),
        content: Set(content.to_owned()),
        structured_payload: Set(structured_payload.map(ToOwned::to_owned)),
        word_count: Set(non_whitespace_char_count(content) as i64),
        created_by: Set(created_by.to_owned()),
        source_revision_id: Set(source_revision_id.map(|revision_id| revision_id.to_string())),
        created_at: Set(now),
        ..Default::default()
    }
    .insert(transaction)
    .await?;
    let document_model = document::Entity::find_by_id(db_id(document_id))
        .filter(document::Entity::COLUMN.deleted_at.is_null())
        .one(transaction)
        .await?
        .ok_or_else(|| StoreError::CorruptRecord("文档不存在或已经删除".to_owned()))?;
    let next_document_revision = document_model.revision + 1;
    let mut active_document = document_model.into_active_model();
    active_document.current_revision_id = Set(Some(revision_id.to_string()));
    active_document.revision = Set(next_document_revision);
    active_document.updated_at = Set(now);
    active_document.update(transaction).await?;
    Ok((revision_id, revision_no))
}

async fn project_id_for_version<C: ConnectionTrait>(
    connection: &C,
    version_id: VersionId,
) -> Result<ProjectId, StoreError> {
    Ok(novel_version::Entity::find_by_id(db_id(version_id))
        .filter(novel_version::Entity::COLUMN.deleted_at.is_null())
        .one(connection)
        .await?
        .ok_or(StoreError::VersionNotFound)?
        .project_id
        .into())
}

async fn insert_preparation_run(
    transaction: &DatabaseTransaction,
    run_id: Uuid,
    project_id: ProjectId,
    version_id: VersionId,
    now: i64,
) -> Result<(), StoreError> {
    let quota_scope_id = ensure_quota_scope(
        transaction,
        db_id(version_id),
        nekowriter_core::QUOTA_SCOPE_PREPARATION,
    )
    .await?;
    run::ActiveModel {
        id: Set(db_id(run_id)),
        project_id: Set(db_id(project_id)),
        version_id: Set(db_id(version_id)),
        kind: Set("preparation".to_owned()),
        parent_run_id: Set(None),
        stage: Set("preparing".to_owned()),
        status: Set("queued".to_owned()),
        resume_count: Set(0),
        request_count: Set(0),
        request_allowance_end: Set(REQUEST_ALLOWANCE_STEP),
        consecutive_protocol_errors: Set(0),
        checkpoint_json: Set("{}".to_owned()),
        quota_scope_id: Set(Some(quota_scope_id)),
        started_at: Set(now),
        ended_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(transaction)
    .await?;
    Ok(())
}

async fn insert_chapter_outline_run(
    transaction: &DatabaseTransaction,
    run_id: Uuid,
    project_id: ProjectId,
    version_id: VersionId,
    chapter_id: ChapterId,
    now: i64,
) -> Result<(), StoreError> {
    let quota_scope_id = ensure_quota_scope(
        transaction,
        db_id(version_id),
        &nekowriter_core::chapter_outline_quota_scope_key(chapter_id),
    )
    .await?;
    run::ActiveModel {
        id: Set(db_id(run_id)),
        project_id: Set(db_id(project_id)),
        version_id: Set(db_id(version_id)),
        kind: Set("chapter_outline".to_owned()),
        parent_run_id: Set(None),
        stage: Set("writing.outline".to_owned()),
        status: Set("queued".to_owned()),
        resume_count: Set(0),
        request_count: Set(0),
        request_allowance_end: Set(REQUEST_ALLOWANCE_STEP),
        consecutive_protocol_errors: Set(0),
        checkpoint_json: Set(serde_json::to_string(&RunCheckpoint {
            chapter_id: Some(chapter_id),
            context_fold: None,
            held_chapter_body_submission: None,
        })?),
        quota_scope_id: Set(Some(quota_scope_id)),
        started_at: Set(now),
        ended_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(transaction)
    .await?;
    Ok(())
}

async fn insert_chapter_body_run(
    transaction: &DatabaseTransaction,
    run_id: Uuid,
    project_id: ProjectId,
    version_id: VersionId,
    chapter_id: ChapterId,
    now: i64,
) -> Result<(), StoreError> {
    run::ActiveModel {
        id: Set(db_id(run_id)),
        project_id: Set(db_id(project_id)),
        version_id: Set(db_id(version_id)),
        kind: Set("chapter_body".to_owned()),
        parent_run_id: Set(None),
        stage: Set("writing.drafting".to_owned()),
        status: Set("queued".to_owned()),
        resume_count: Set(0),
        request_count: Set(0),
        request_allowance_end: Set(REQUEST_ALLOWANCE_STEP),
        consecutive_protocol_errors: Set(0),
        checkpoint_json: Set(serde_json::to_string(&RunCheckpoint {
            chapter_id: Some(chapter_id),
            context_fold: None,
            held_chapter_body_submission: None,
        })?),
        quota_scope_id: Set(None),
        started_at: Set(now),
        ended_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(transaction)
    .await?;
    Ok(())
}

pub(crate) async fn persist_workflow(
    transaction: &DatabaseTransaction,
    workflow: &WorkflowSnapshot,
) -> Result<(), StoreError> {
    let workflow_model = workflow_instance::Entity::find_by_id(db_id(workflow.version_id))
        .one(transaction)
        .await?
        .ok_or(StoreError::VersionNotFound)?;
    let mut active_workflow = workflow_model.into_active_model();
    active_workflow.state_json = Set(serde_json::to_string(workflow)?);
    active_workflow.state_revision = Set(workflow.state_revision);
    active_workflow.active_run_id = Set(workflow.active_run_id.map(|run_id| run_id.to_string()));
    active_workflow.approval_mode = Set(approval_mode_name(&workflow.approval_mode).to_owned());
    active_workflow.pause_requested = Set(workflow.pause_requested);
    active_workflow.update(transaction).await?;
    Ok(())
}

pub(crate) async fn insert_event(
    transaction: &DatabaseTransaction,
    version_id: VersionId,
    event_type: &str,
    entity_id: Uuid,
    revision: i64,
    payload: serde_json::Value,
    occurred_at: i64,
) -> Result<(), StoreError> {
    insert_event_row(
        transaction,
        Some(version_id),
        event_type,
        entity_id,
        revision,
        payload,
        occurred_at,
    )
    .await
}

/// 写入一条不属于任何稿本的持久事件（如知识库索引状态变化）。
pub(crate) async fn insert_app_event(
    transaction: &DatabaseTransaction,
    event_type: &str,
    entity_id: Uuid,
    revision: i64,
    payload: serde_json::Value,
    occurred_at: i64,
) -> Result<(), StoreError> {
    insert_event_row(
        transaction,
        None,
        event_type,
        entity_id,
        revision,
        payload,
        occurred_at,
    )
    .await
}

async fn insert_event_row(
    transaction: &DatabaseTransaction,
    version_id: Option<VersionId>,
    event_type: &str,
    entity_id: Uuid,
    revision: i64,
    payload: serde_json::Value,
    occurred_at: i64,
) -> Result<(), StoreError> {
    event::ActiveModel {
        version_id: Set(version_id.map(|id| id.to_string())),
        event_type: Set(event_type.to_owned()),
        entity_id: Set(db_id(entity_id)),
        revision: Set(revision.to_string()),
        payload_json: Set(serde_json::to_string(&payload)?),
        occurred_at: Set(occurred_at),
        ..Default::default()
    }
    .insert(transaction)
    .await?;
    Ok(())
}
