//! 单轮 Agent 协议编排。
//!
//! 模型适配器负责传输和流协议；本模块负责将完整响应限制为「一个允许的工具调用」
//! 或「当前阶段的类型化提交」。它不访问数据库，也不直接修改正文：App 服务在
//! 持久化请求快照后调用它，并把返回的工具调用交给 [`ToolHost`]。

use std::collections::BTreeSet;

use futures_util::future::BoxFuture;
use nekowriter_core::{
    ChapterId, ChapterPlan, ChapterPlanDraft, MAX_ARTIFACT_CONTENT_LEN, MAX_REVIEW_ISSUES_LEN,
    NewChapterOutline, PreparationArtifactInput, PreparationArtifactKind,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

use crate::AgentToolDefinition;
use crate::provider::{
    ModelCompletion, ModelEvent, ModelFinishReason, ModelProvider, ModelRequest, ModelToolCall,
    ProviderError,
};

/// 当前模型响应必须使用的阶段提交结构。
///
/// 准备阶段携带 `required_artifacts`：这是本轮必须提交的产物项。首轮是四项全齐，
/// 修订轮只是需要修订的那些项 —— Store 在修订轮只接受这些，因此合同与解析器都
/// 按同一份清单生成，模型不会因为「照合同提交」而被拒绝。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PhaseSubmissionKind {
    Preparation {
        required_artifacts: Vec<PreparationArtifactKind>,
    },
    ChapterOutline {
        chapter_id: ChapterId,
    },
    Chapter {
        chapter_id: ChapterId,
    },
    Review,
}

/// 经过 JSON 与领域输入校验后的终态提交。
#[derive(Clone, Debug, PartialEq)]
pub enum PhaseSubmission {
    Preparation(PreparationSubmission),
    ChapterOutline(ChapterOutlineSubmission),
    Chapter(ChapterSubmission),
    Review(ReviewSubmission),
}

/// 准备阶段的四项产物及其有序章节计划。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparationSubmission {
    pub artifacts: Vec<PreparationArtifactInput>,
    /// 只有本轮需要修订粗纲时才带章节计划；其余轮次为 `None`，与 Store 的
    /// 「未提交粗纲时不能修改章节计划」一致。
    pub chapter_plan: Option<Vec<ChapterPlanDraft>>,
}

/// 一章细纲的类型化提交。
///
/// 细纲只有 `content` 一个交付字段：本章目标与写作约定都属于细纲正文，Store 也只
/// 保存正文，因此提交结构里没有第二个去处。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChapterOutlineSubmission {
    pub chapter_id: ChapterId,
    pub content: NewChapterOutline,
}

/// 正文阶段的终态提交。
///
/// 正文不在这里携带；唯一合法来源是已经通过 `ToolHost` 的正文编辑工具。`summary`
/// 仅供后续审核或统计使用。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChapterSubmission {
    pub chapter_id: ChapterId,
    pub summary: String,
}

/// 审核 Agent 的结构化结论。
#[derive(Clone, Debug, PartialEq)]
pub struct ReviewSubmission {
    pub passed: bool,
    pub confidence: f64,
    pub issues: Vec<String>,
}

/// 单轮响应的下一步。工具调用尚未执行，必须由应用服务在正确的事务边界内交给
/// [`ToolHost`]；阶段提交同样还必须接受工作流和修订依赖守卫。
///
/// 交回的业务工具调用已经把参数规范化为解码后的 JSON object：参数不可解码的调用
/// 在这里就变成可纠正的协议错误，不会带着不可用的参数到达工具执行层。
#[derive(Clone, Debug, PartialEq)]
pub enum AgentRoundOutcome {
    ToolCall(ModelToolCall),
    Submission(PhaseSubmission),
}

/// App 服务提供的业务工具边界。
///
/// 它拥有当前 Run、稿本和章节目标，所以模型参数绝不包含项目或章节 ID。返回值
/// 必须作为 `tool` 消息与原 assistant tool call 配对加入下一次请求快照。
pub trait ToolHost: Send + Sync {
    fn execute_tool<'a>(
        &'a self,
        call: &'a ModelToolCall,
    ) -> BoxFuture<'a, Result<ToolResult, ToolHostError>>;
}

/// 一次工具执行的、可安全回送模型的结果。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResult {
    pub tool_call_id: String,
    pub content: String,
}

/// `ToolHost` 的业务错误。它必须以 `tool result` 的形式回送，而不是让模型调用悄悄
/// 消失；App 可以据此构造稳定的错误 JSON 文本。
#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("工具 {code} 执行失败：{message}")]
pub struct ToolHostError {
    pub code: String,
    pub message: String,
}

/// 模型响应或阶段 JSON 无法安全推进时的错误。
#[derive(Debug, Error)]
pub enum AgentRunnerError {
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error("阶段 {stage:?} 不允许声明工具 {tool}")]
    DisallowedDeclaredTool {
        stage: PhaseSubmissionKind,
        tool: String,
    },
    #[error("模型在阶段 {stage:?} 调用了不允许的工具 {tool}")]
    DisallowedToolCall {
        stage: PhaseSubmissionKind,
        tool: String,
    },
    #[error("模型调用的工具 {tool} 没有在本次请求中声明")]
    UndeclaredToolCall { tool: String },
    #[error("模型调用的工具 {tool} 的参数不是合法 JSON：{reason}")]
    UndecodableToolArguments { tool: String, reason: String },
    #[error("模型在阶段 {stage:?} 没有调用提交工具 {tool}；本阶段的终态结果只能通过该工具提交")]
    MissingSubmissionToolCall {
        stage: PhaseSubmissionKind,
        tool: &'static str,
    },
    #[error("模型返回工具调用时的终止原因无效")]
    InvalidToolFinishReason,
    #[error("模型一次响应给出 {count} 个工具调用；仅允许一个")]
    ParallelToolCalls { count: usize },
    #[error("模型不能在同一响应同时返回正文文本和工具调用")]
    TextAndToolCalls,
    #[error("模型对提交工具 {tool} 的调用无效：{error}")]
    InvalidPhaseSubmission {
        /// 本阶段的提交工具名：纠正与诊断都要点名模型该用哪个工具。
        tool: &'static str,
        /// 供 App 服务持久化的完整工具参数；不会进入错误显示文本。
        raw_response: String,
        #[source]
        error: PhaseSubmissionError,
    },
}

/// 完整 assistant 文本无法成为当前阶段提交的原因。
///
/// 调用方同时持有完整原文，必须将其与此错误一起持久化，而不能从散文中猜测出
/// 一个提交结果。
#[derive(Debug, Error)]
pub enum PhaseSubmissionError {
    #[error("提交工具的参数不是当前阶段要求的 JSON：{reason}")]
    InvalidJson { reason: String },
    #[error("阶段提交字段无效：{reason}")]
    Invalid { reason: String },
    /// 粗纲提交缺了章节计划。它是本轮真实踩过的失败模式，因此有独立变体：工具结果
    /// 与纠正提示都能点名这一项，而不是只给一句泛泛的字段错误。
    #[error(
        "提交粗纲时必须同时提交至少一项 chapter_plan：本轮章节清单只能放在 chapter_plan，写进粗纲正文不算提交"
    )]
    MissingChapterPlan,
    #[error("阶段提交的 chapter_id 与当前章节不一致")]
    ChapterIdMismatch,
}

/// 组合一个模型协议适配器和单轮阶段守卫。
pub struct AgentRunner<P> {
    provider: P,
}

impl<P> AgentRunner<P> {
    #[must_use]
    pub const fn new(provider: P) -> Self {
        Self { provider }
    }

    #[must_use]
    pub fn provider(&self) -> &P {
        &self.provider
    }
}

impl<P: ModelProvider> AgentRunner<P> {
    /// 运行一轮模型请求，并在返回前完成工具白名单和结构化终态校验。
    ///
    /// # Errors
    ///
    /// 传输失败、工具越权、工具声明不一致、终态不是完整 JSON，或 JSON 不满足
    /// 当前阶段的领域输入规则时返回错误。调用方仍可持久化原始模型响应及错误。
    pub async fn run_round(
        &self,
        stage: PhaseSubmissionKind,
        request: &ModelRequest,
        emit: &mut (dyn FnMut(ModelEvent) + Send),
    ) -> Result<AgentRoundOutcome, AgentRunnerError> {
        let completion = self.provider.execute(request, emit).await?;
        classify_model_completion(stage, request, completion)
    }
}

/// 从提交工具的参数解析阶段提交。
///
/// 输入是 [`submission_tool`] 声明的那次工具调用的参数。这项函数不从散文中寻找
/// JSON，也不容忍未知字段；校验失败的原因会作为配对工具结果回到模型，因此每一句
/// 都必须说清缺的是哪一项。
///
/// # Errors
///
/// 参数不是当前阶段要求的 JSON 对象，或其中字段未通过领域校验时返回错误。
pub fn parse_phase_submission(
    stage: &PhaseSubmissionKind,
    arguments: &Value,
) -> Result<PhaseSubmission, PhaseSubmissionError> {
    if !arguments.is_object() {
        return Err(PhaseSubmissionError::InvalidJson {
            reason: "提交工具的参数必须是 JSON 对象".to_owned(),
        });
    }
    match stage {
        PhaseSubmissionKind::Preparation { required_artifacts } => {
            parse_preparation_submission(required_artifacts, arguments)
        }
        PhaseSubmissionKind::ChapterOutline { chapter_id } => {
            parse_outline_submission(*chapter_id, arguments)
        }
        PhaseSubmissionKind::Chapter { chapter_id } => {
            parse_chapter_submission(*chapter_id, arguments)
        }
        PhaseSubmissionKind::Review => parse_review_submission(arguments),
    }
}

/// 校验一个已经由模型适配器完整接收的响应，并分类为工具调用或阶段提交。
///
/// 阶段提交就是一次调用本阶段提交工具的响应：参数直接交给解析器，因此同一份校验
/// 既能产生类型化提交，也能在失败时产生一条点名缺项的配对工具结果。没有工具调用的
/// 响应不是提交——它必须由调用方作为可纠正的协议错误回给模型。App 可先持久化请求
/// 检查点和完整传输响应，再调用本函数；提供商请求 ID 与用量不会因校验失败而丢失。
///
/// # Errors
///
/// 工具白名单、结束原因或提交工具参数不符合当前阶段合同时返回
/// [`AgentRunnerError`]，调用方应保留原始响应并将本次 attempt 标记为失败。
pub fn classify_model_completion(
    stage: PhaseSubmissionKind,
    request: &ModelRequest,
    completion: ModelCompletion,
) -> Result<AgentRoundOutcome, AgentRunnerError> {
    validate_declared_tools(&stage, request)?;
    if completion.tool_calls.is_empty() {
        return Err(AgentRunnerError::MissingSubmissionToolCall {
            tool: submission_tool_name(&stage),
            stage,
        });
    }
    if completion.tool_calls.len() != 1 {
        return Err(AgentRunnerError::ParallelToolCalls {
            count: completion.tool_calls.len(),
        });
    }
    if completion
        .content
        .as_deref()
        .is_some_and(|content| !content.trim().is_empty())
    {
        return Err(AgentRunnerError::TextAndToolCalls);
    }
    let Some(call) = completion.tool_calls.into_iter().next() else {
        return Err(AgentRunnerError::MissingSubmissionToolCall {
            tool: submission_tool_name(&stage),
            stage,
        });
    };
    if completion.finish_reason != ModelFinishReason::ToolCalls {
        return Err(AgentRunnerError::InvalidToolFinishReason);
    }
    if !allowed_tools(&stage).contains(&call.name.as_str()) {
        return Err(AgentRunnerError::DisallowedToolCall {
            stage,
            tool: call.name,
        });
    }
    if !request.tools.iter().any(|tool| tool.name == call.name) {
        return Err(AgentRunnerError::UndeclaredToolCall { tool: call.name });
    }
    // 参数不可解码不是传输失败：提交工具的这一轮按「被拒的提交」回给模型，业务
    // 工具同样拿到一条说明原因的配对结果，两者都能在剩余额度里重试。
    let arguments = match call.decode_arguments() {
        Ok(arguments) => arguments,
        Err(reason) if call.name == submission_tool_name(&stage) => {
            return Err(AgentRunnerError::InvalidPhaseSubmission {
                tool: submission_tool_name(&stage),
                raw_response: call.arguments_text(),
                error: PhaseSubmissionError::InvalidJson { reason },
            });
        }
        Err(reason) => {
            return Err(AgentRunnerError::UndecodableToolArguments {
                tool: call.name,
                reason,
            });
        }
    };
    if call.name != submission_tool_name(&stage) {
        return Ok(AgentRoundOutcome::ToolCall(ModelToolCall {
            arguments,
            ..call
        }));
    }
    parse_phase_submission(&stage, &arguments)
        .map(AgentRoundOutcome::Submission)
        .map_err(|error| AgentRunnerError::InvalidPhaseSubmission {
            tool: submission_tool_name(&stage),
            raw_response: call.arguments_text(),
            error,
        })
}

fn validate_declared_tools(
    stage: &PhaseSubmissionKind,
    request: &ModelRequest,
) -> Result<(), AgentRunnerError> {
    let allowed_tools = allowed_tools(stage);
    for tool in &request.tools {
        if !allowed_tools.contains(&tool.name.as_str()) {
            return Err(AgentRunnerError::DisallowedDeclaredTool {
                stage: stage.clone(),
                tool: tool.name.clone(),
            });
        }
    }
    Ok(())
}

fn allowed_tools(stage: &PhaseSubmissionKind) -> &'static [&'static str] {
    match stage {
        PhaseSubmissionKind::Preparation { .. } => &[
            "ask_user",
            "query_knowledge",
            "research_web",
            SUBMIT_PREPARATION_TOOL,
        ],
        PhaseSubmissionKind::ChapterOutline { .. } => &[
            "ask_user",
            "query_knowledge",
            "research_web",
            SUBMIT_CHAPTER_OUTLINE_TOOL,
        ],
        PhaseSubmissionKind::Chapter { .. } => &[
            "ask_user",
            "append_content",
            "replace_content",
            SUBMIT_CHAPTER_TOOL,
        ],
        PhaseSubmissionKind::Review => &[SUBMIT_REVIEW_TOOL],
    }
}

/// 准备阶段的提交工具名。
pub const SUBMIT_PREPARATION_TOOL: &str = "submit_preparation";
/// 细纲阶段的提交工具名。
pub const SUBMIT_CHAPTER_OUTLINE_TOOL: &str = "submit_chapter_outline";
/// 正文阶段的提交工具名。
pub const SUBMIT_CHAPTER_TOOL: &str = "submit_chapter";
/// 审核阶段的提交工具名。
pub const SUBMIT_REVIEW_TOOL: &str = "submit_review";

/// 本阶段的提交工具名。
///
/// 提交是一工具调用，不是一个隐藏在散文里的 JSON：缺字段或字段非法时，模型能收到
/// 自己那次调用被拒绝的工具结果，而不用从一段说明里猜测缺了什么。
#[must_use]
pub fn submission_tool_name(stage: &PhaseSubmissionKind) -> &'static str {
    match stage {
        PhaseSubmissionKind::Preparation { .. } => SUBMIT_PREPARATION_TOOL,
        PhaseSubmissionKind::ChapterOutline { .. } => SUBMIT_CHAPTER_OUTLINE_TOOL,
        PhaseSubmissionKind::Chapter { .. } => SUBMIT_CHAPTER_TOOL,
        PhaseSubmissionKind::Review => SUBMIT_REVIEW_TOOL,
    }
}

/// 本阶段的提交工具定义。
///
/// 参数 schema 与 [`parse_phase_submission`] 同源：必填字段、本轮产物清单、章节 ID
/// 都在这里声明一次，[`submission_contract`] 只是它的可读说明。因此「缺 `chapter_plan`
/// 会被拒绝」是模型能看到的必填项，章节级 `chapter_id` 更是被固定成唯一取值。
#[must_use]
pub fn submission_tool(stage: &PhaseSubmissionKind) -> AgentToolDefinition {
    match stage {
        PhaseSubmissionKind::Preparation { required_artifacts } => {
            preparation_submission_tool(required_artifacts)
        }
        PhaseSubmissionKind::ChapterOutline { chapter_id } => AgentToolDefinition {
            name: SUBMIT_CHAPTER_OUTLINE_TOOL.to_owned(),
            description: "提交本章细纲。本阶段的终态结果只能通过这个工具提交。".to_owned(),
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["chapter_id", "content"],
                "properties": {
                    "chapter_id": chapter_id_property(*chapter_id),
                    "content": { "type": "string", "minLength": 1 }
                }
            }),
        },
        PhaseSubmissionKind::Chapter { chapter_id } => AgentToolDefinition {
            name: SUBMIT_CHAPTER_TOOL.to_owned(),
            description: "提交本章完成的信号与一句话小结。正文本身只能用 append_content / replace_content 修改，本工具不接受正文。"
                .to_owned(),
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["chapter_id", "summary"],
                "properties": {
                    "chapter_id": chapter_id_property(*chapter_id),
                    "summary": { "type": "string", "minLength": 1 }
                }
            }),
        },
        PhaseSubmissionKind::Review => AgentToolDefinition {
            name: SUBMIT_REVIEW_TOOL.to_owned(),
            description: "提交本轮审核结论。本阶段的终态结果只能通过这个工具提交。"
                .to_owned(),
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["passed", "confidence", "issues"],
                "properties": {
                    "passed": { "type": "boolean" },
                    "confidence": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                    "issues": { "type": "array", "items": { "type": "string", "minLength": 1 } }
                }
            }),
        },
    }
}

/// 当前阶段必须出现在系统消息里的提交说明。
///
/// docs/02 §4.1 要求阶段角色包含本阶段的「提交结构」。结构本身由 [`submission_tool`]
/// 的参数 schema 声明，这里只写模型必须知道的行为约定：用什么工具提交、哪些字段
/// 缺一不可、缺了会怎样；改了字段就必须同时改解析器与工具定义，三者同源。
#[must_use]
pub fn submission_contract(stage: &PhaseSubmissionKind) -> String {
    let tool = submission_tool_name(stage);
    match stage {
        PhaseSubmissionKind::Preparation { required_artifacts } => {
            preparation_contract(tool, required_artifacts)
        }
        PhaseSubmissionKind::ChapterOutline { chapter_id } => format!(
            "阶段提交：本轮结果只能通过 `{tool}` 工具提交，不要在普通文本里输出 JSON，也不要在工具调用之外附加说明。\n\
工具参数的 `chapter_id` 只有 \"{chapter_id}\" 一个合法取值；`content` 是完整细纲正文，不能为空，本章目标与写作约定都要写进去。\n\
缺少必填字段或字段非法时整轮提交会被拒绝、不会保存任何内容：工具结果里会给出具体原因，请在同一次调用里补齐后重新提交。"
        ),
        PhaseSubmissionKind::Chapter { chapter_id } => format!(
            "阶段提交：本章完成时只能通过 `{tool}` 工具提交，不要在普通文本里输出 JSON，也不要在工具调用之外附加说明。\n\
正文只能通过 append_content / replace_content 工具修改，不要在没有工具调用时把正文写进普通文本；提交工具只接受 `chapter_id`（只有 \"{chapter_id}\" 一个合法取值）与一句话 `summary`。\n\
缺少必填字段或字段非法时整轮提交会被拒绝：工具结果里会给出具体原因，请在同一次调用里补齐后重新提交。"
        ),
        PhaseSubmissionKind::Review => format!(
            "阶段提交：本轮结论只能通过 `{tool}` 工具提交，不要在普通文本里输出 JSON，也不要附加解释。\n\
`passed` 为布尔值；`confidence` 为 0 到 1 之间的数值；`issues` 为字符串数组，没有具体问题时留空数组。\n\
缺少必填字段或字段非法时本轮结论会被拒绝：工具结果里会给出具体原因，请在同一次调用里补齐后重新提交。"
        ),
    }
}

fn chapter_id_property(chapter_id: ChapterId) -> Value {
    json!({ "type": "string", "enum": [chapter_id.to_string()] })
}

/// 准备阶段的提交工具：产物清单与必填规则都按本轮真正需要提交的产物项生成。
fn preparation_submission_tool(
    required_artifacts: &[PreparationArtifactKind],
) -> AgentToolDefinition {
    let kinds = required_artifacts
        .iter()
        .map(|kind| Value::String(kind.stable_key().to_owned()))
        .collect::<Vec<_>>();
    let count = required_artifacts.len();
    let mut parameters = json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["artifacts"],
        "properties": {
            "artifacts": {
                "type": "array",
                "minItems": count,
                "maxItems": count,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["kind", "content"],
                    "properties": {
                        "kind": { "type": "string", "enum": kinds },
                        "content": { "type": "string", "minLength": 1 }
                    }
                }
            }
        }
    });
    if required_artifacts.contains(&PreparationArtifactKind::Outline) {
        // 本轮要修订粗纲，章节清单只能放在这个顶层数组里：它是模型唯一能看到
        // 「必填」的可执行位置，也是解析器唯一接受的位置。
        parameters["required"] = json!(["artifacts", "chapter_plan"]);
        parameters["properties"]["chapter_plan"] = json!({
            "type": "array",
            "minItems": 1,
            "items": {
                "type": "object",
                "additionalProperties": false,
                "required": ["title", "goal"],
                "properties": {
                    "title": { "type": "string", "minLength": 1 },
                    "goal": { "type": "string", "minLength": 1 }
                }
            }
        });
    }
    AgentToolDefinition {
        name: SUBMIT_PREPARATION_TOOL.to_owned(),
        description: "提交本轮的准备产物。四项产物与本轮章节清单都只能通过这个工具提交；章节清单属于 chapter_plan，不属于任何产物正文。"
            .to_owned(),
        parameters,
    }
}

/// 准备阶段的提交说明。
fn preparation_contract(tool: &str, required_artifacts: &[PreparationArtifactKind]) -> String {
    let names = required_artifacts
        .iter()
        .map(|kind| kind.stable_key())
        .collect::<Vec<_>>()
        .join("、");
    let plan_rule = if required_artifacts.contains(&PreparationArtifactKind::Outline) {
        "`chapter_plan` 是本轮章节清单的唯一承载字段，至少一项；只把章节清单写进粗纲的 content 不算提交，它必须放在 chapter_plan 里，且标题、顺序与目标同粗纲一致。"
    } else {
        "本轮不要提交 `chapter_plan`：没有修订粗纲就不能改章节计划。"
    };
    format!(
        "阶段提交：本轮结果只能通过 `{tool}` 工具提交，不要在普通文本里输出 JSON，也不要在工具调用之外附加说明。\n\
工具参数的 `artifacts` 必须恰好包含 {names} 各一次（本轮只需提交这些产物），每项 content 不能为空。\n\
{plan_rule}\n\
缺少必填字段或字段非法时整轮提交会被拒绝、不会保存任何产物：工具结果里会给出具体原因，请在同一次调用里补齐后重新提交。"
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPreparationSubmission {
    artifacts: Vec<RawPreparationArtifact>,
    #[serde(default)]
    chapter_plan: Option<Vec<RawChapterPlanDraft>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPreparationArtifact {
    kind: PreparationArtifactKind,
    content: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawChapterPlanDraft {
    title: String,
    goal: String,
}

fn parse_preparation_submission(
    required_artifacts: &[PreparationArtifactKind],
    arguments: &Value,
) -> Result<PhaseSubmission, PhaseSubmissionError> {
    let raw = decode_submission::<RawPreparationSubmission>(arguments)?;
    let mut seen_kinds = BTreeSet::new();
    let mut artifacts = Vec::with_capacity(raw.artifacts.len());
    for artifact in raw.artifacts {
        if !seen_kinds.insert(artifact.kind) {
            return Err(PhaseSubmissionError::Invalid {
                reason: format!("准备产物 {} 重复", artifact.kind.stable_key()),
            });
        }
        artifacts.push(
            PreparationArtifactInput::new(artifact.kind, artifact.content).map_err(|error| {
                PhaseSubmissionError::Invalid {
                    reason: error.to_string(),
                }
            })?,
        );
    }
    let required = required_artifacts.iter().copied().collect::<BTreeSet<_>>();
    if seen_kinds != required {
        let names = required_artifacts
            .iter()
            .map(|kind| kind.stable_key())
            .collect::<Vec<_>>()
            .join("、");
        return Err(PhaseSubmissionError::Invalid {
            reason: format!("准备提交必须恰好包含 {names} 各一次"),
        });
    }
    artifacts.sort_by_key(|artifact| artifact.kind);

    // 与 Store 的规则一致：粗纲提交必须带章节计划，没改粗纲就不能改章节计划。
    let outline_required = required.contains(&PreparationArtifactKind::Outline);
    let chapter_plan = match (outline_required, raw.chapter_plan) {
        (true, Some(plan)) => {
            let drafts = plan
                .into_iter()
                .map(|item| {
                    ChapterPlanDraft::new(item.title, item.goal).map_err(|error| {
                        PhaseSubmissionError::Invalid {
                            reason: error.to_string(),
                        }
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            // `ChapterPlan` 是唯一负责章节数量及顺序不变量的领域构造器。丢弃此处生成的
            // 临时 ID；真正提交时 Store 会在同一事务中创建可追溯的稳定计划项 ID。
            ChapterPlan::from_drafts(&drafts).map_err(|error| PhaseSubmissionError::Invalid {
                reason: error.to_string(),
            })?;
            Some(drafts)
        }
        (false, None) => None,
        (true, None) => {
            return Err(PhaseSubmissionError::MissingChapterPlan);
        }
        (false, Some(_)) => {
            return Err(PhaseSubmissionError::Invalid {
                reason: "本轮不修改粗纲，不能提交 chapter_plan".to_owned(),
            });
        }
    };

    Ok(PhaseSubmission::Preparation(PreparationSubmission {
        artifacts,
        chapter_plan,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawChapterOutlineSubmission {
    chapter_id: ChapterId,
    content: String,
}

fn parse_outline_submission(
    expected_chapter_id: ChapterId,
    arguments: &Value,
) -> Result<PhaseSubmission, PhaseSubmissionError> {
    let raw = decode_submission::<RawChapterOutlineSubmission>(arguments)?;
    if raw.chapter_id != expected_chapter_id {
        return Err(PhaseSubmissionError::ChapterIdMismatch);
    }
    let content =
        NewChapterOutline::new(raw.content).map_err(|error| PhaseSubmissionError::Invalid {
            reason: error.to_string(),
        })?;
    Ok(PhaseSubmission::ChapterOutline(ChapterOutlineSubmission {
        chapter_id: raw.chapter_id,
        content,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawChapterSubmission {
    chapter_id: ChapterId,
    summary: String,
}

fn parse_chapter_submission(
    expected_chapter_id: ChapterId,
    arguments: &Value,
) -> Result<PhaseSubmission, PhaseSubmissionError> {
    let raw = decode_submission::<RawChapterSubmission>(arguments)?;
    if raw.chapter_id != expected_chapter_id {
        return Err(PhaseSubmissionError::ChapterIdMismatch);
    }
    validate_text("章节摘要", &raw.summary, MAX_ARTIFACT_CONTENT_LEN)?;
    Ok(PhaseSubmission::Chapter(ChapterSubmission {
        chapter_id: raw.chapter_id,
        summary: raw.summary,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawReviewSubmission {
    passed: bool,
    confidence: f64,
    #[serde(deserialize_with = "deserialize_review_issues")]
    issues: Vec<String>,
}

/// 审核 `issues` 只能是字符串数组。
///
/// 真实端点会给过「把数组序列化成 JSON 字符串再塞进 `issues`」的提交
/// （`"issues": "[\"【建议】…\"]"`）：内容对、形状错。这条原因会原样进入配对的
/// 错误工具结果与纠正消息，所以必须点名字段和正确形状，而不是只报一句类型不符。
fn deserialize_review_issues<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match Value::deserialize(deserializer)? {
        Value::Array(items) => items
            .into_iter()
            .map(|item| match item {
                Value::String(text) => Ok(text),
                other => Err(serde::de::Error::custom(format!(
                    "审核 issues 的每一项都必须是字符串，收到{}",
                    json_type_name(&other)
                ))),
            })
            .collect(),
        Value::String(raw) => Err(serde::de::Error::custom(
            if serde_json::from_str::<Value>(&raw).is_ok_and(|value| value.is_array()) {
                "审核 issues 必须是字符串数组，不能是装着数组的 JSON 字符串；请把数组直接放在 issues 字段上"
            } else {
                "审核 issues 必须是字符串数组，不能是字符串"
            },
        )),
        other => Err(serde::de::Error::custom(format!(
            "审核 issues 必须是字符串数组，收到{}",
            json_type_name(&other)
        ))),
    }
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "布尔值",
        Value::Number(_) => "数值",
        Value::String(_) => "字符串",
        Value::Array(_) => "数组",
        Value::Object(_) => "object",
    }
}

fn parse_review_submission(arguments: &Value) -> Result<PhaseSubmission, PhaseSubmissionError> {
    let raw = decode_submission::<RawReviewSubmission>(arguments)?;
    if !raw.confidence.is_finite() || !(0.0..=1.0).contains(&raw.confidence) {
        return Err(PhaseSubmissionError::Invalid {
            reason: "审核 confidence 必须是 [0, 1] 内的有限数值".to_owned(),
        });
    }
    let mut total_issue_chars = 0_usize;
    for issue in &raw.issues {
        validate_text("审核问题", issue, MAX_REVIEW_ISSUES_LEN)?;
        total_issue_chars = total_issue_chars.saturating_add(issue.chars().count());
        if total_issue_chars > MAX_REVIEW_ISSUES_LEN {
            return Err(PhaseSubmissionError::Invalid {
                reason: format!("审核问题总长度不能超过 {MAX_REVIEW_ISSUES_LEN} 个字符"),
            });
        }
    }
    Ok(PhaseSubmission::Review(ReviewSubmission {
        passed: raw.passed,
        confidence: raw.confidence,
        issues: raw.issues,
    }))
}

fn decode_submission<T: serde::de::DeserializeOwned>(
    arguments: &Value,
) -> Result<T, PhaseSubmissionError> {
    serde_json::from_value(arguments.clone()).map_err(|error| PhaseSubmissionError::InvalidJson {
        reason: error.to_string(),
    })
}

fn validate_text(
    field: &'static str,
    content: &str,
    max_chars: usize,
) -> Result<(), PhaseSubmissionError> {
    if content.trim().is_empty() {
        return Err(PhaseSubmissionError::Invalid {
            reason: format!("{field} 不能为空"),
        });
    }
    if content.chars().count() > max_chars {
        return Err(PhaseSubmissionError::Invalid {
            reason: format!("{field} 不能超过 {max_chars} 个字符"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use futures_util::future;
    use serde_json::json;
    use uuid::Uuid;

    use super::*;
    use crate::{ContextMessageRole, provider::ModelRequestMessage};
    use nekowriter_core::{ModelCapabilityProfile, ThinkingLevel};

    #[derive(Clone)]
    struct StubProvider {
        completion: Arc<ModelCompletion>,
    }

    impl ModelProvider for StubProvider {
        fn capabilities(&self) -> ModelCapabilityProfile {
            ModelCapabilityProfile {
                streaming: true,
                tools: true,
                stream_usage: true,
                thinking: false,
            }
        }

        fn execute<'a>(
            &'a self,
            _request: &'a ModelRequest,
            _emit: &'a mut (dyn FnMut(ModelEvent) + Send),
        ) -> BoxFuture<'a, Result<ModelCompletion, ProviderError>> {
            Box::pin(future::ready(Ok((*self.completion).clone())))
        }
    }

    fn request(tools: Vec<AgentToolDefinition>) -> ModelRequest {
        ModelRequest {
            messages: vec![ModelRequestMessage::Text {
                role: ContextMessageRole::User,
                content: "测试".to_owned(),
            }],
            tools,
            thinking_level: ThinkingLevel::Off,
        }
    }

    /// 阶段请求必须声明本阶段的提交工具，否则合法提交会被判为未声明调用。
    fn submission_request(stage: &PhaseSubmissionKind) -> ModelRequest {
        request(vec![submission_tool(stage)])
    }

    /// 一个只有普通文本、没有任何工具调用的响应：它不是阶段提交。
    fn stop_completion(content: impl Into<String>) -> ModelCompletion {
        ModelCompletion {
            provider_request_id: None,
            content: Some(content.into()),
            tool_calls: Vec::new(),
            usage: None,
            usage_raw: None,
            finish_reason: ModelFinishReason::Stop,
        }
    }

    /// 一次调用本阶段提交工具的响应。
    fn submission_completion(stage: &PhaseSubmissionKind, arguments: Value) -> ModelCompletion {
        ModelCompletion {
            provider_request_id: None,
            content: None,
            tool_calls: vec![ModelToolCall {
                id: "submit-call-1".to_owned(),
                name: submission_tool_name(stage).to_owned(),
                arguments,
            }],
            usage: None,
            usage_raw: None,
            finish_reason: ModelFinishReason::ToolCalls,
        }
    }

    fn preparation_stage() -> PhaseSubmissionKind {
        PhaseSubmissionKind::Preparation {
            required_artifacts: PreparationArtifactKind::ALL.to_vec(),
        }
    }

    #[test]
    fn preparation_submission_requires_exactly_four_distinct_artifacts() {
        let three = json!({
            "artifacts": [
                { "kind": "background", "content": "背景" },
                { "kind": "research", "content": "资料" },
                { "kind": "characters", "content": "人物" }
            ],
            "chapter_plan": [{ "title": "第一章", "goal": "开场" }]
        });
        let error = parse_phase_submission(&preparation_stage(), &three).unwrap_err();
        assert!(matches!(error, PhaseSubmissionError::Invalid { .. }));

        let complete = json!({
            "artifacts": [
                { "kind": "outline", "content": "粗纲" },
                { "kind": "characters", "content": "人物" },
                { "kind": "research", "content": "资料" },
                { "kind": "background", "content": "背景" }
            ],
            "chapter_plan": [{ "title": "第一章", "goal": "开场" }]
        });
        let PhaseSubmission::Preparation(submission) =
            parse_phase_submission(&preparation_stage(), &complete).unwrap()
        else {
            panic!("expected preparation submission");
        };
        assert_eq!(
            submission
                .artifacts
                .iter()
                .map(|artifact| artifact.kind)
                .collect::<Vec<_>>(),
            PreparationArtifactKind::ALL
        );
    }

    /// 真实端点三次失败的那一种提交：四项产物齐全，章节清单却只写在粗纲正文里。
    #[test]
    fn outline_submitted_without_a_chapter_plan_names_the_missing_field() {
        let without_plan = json!({
            "artifacts": [
                { "kind": "background", "content": "背景" },
                { "kind": "research", "content": "资料" },
                { "kind": "characters", "content": "人物" },
                { "kind": "outline", "content": "第一章……第十章：章节清单写在这里" }
            ]
        });
        let error = parse_phase_submission(&preparation_stage(), &without_plan).unwrap_err();
        assert!(matches!(error, PhaseSubmissionError::MissingChapterPlan));
        assert!(error.to_string().contains("chapter_plan"));
    }

    #[test]
    fn preparation_tool_requires_chapter_plan_exactly_when_the_outline_is_revised() {
        let outline_tool = submission_tool(&preparation_stage());
        assert_eq!(outline_tool.name, SUBMIT_PREPARATION_TOOL);
        assert_eq!(
            outline_tool.parameters["required"],
            json!(["artifacts", "chapter_plan"])
        );
        assert_eq!(
            outline_tool.parameters["properties"]["chapter_plan"]["minItems"],
            1
        );
        assert_eq!(
            outline_tool.parameters["properties"]["artifacts"]["minItems"],
            4
        );
        assert_eq!(
            outline_tool.parameters["properties"]["artifacts"]["maxItems"],
            4
        );

        let research_only = PhaseSubmissionKind::Preparation {
            required_artifacts: vec![PreparationArtifactKind::Research],
        };
        let research_tool = submission_tool(&research_only);
        assert_eq!(research_tool.parameters["required"], json!(["artifacts"]));
        assert!(
            research_tool.parameters["properties"]
                .get("chapter_plan")
                .is_none()
        );
        assert_eq!(
            research_tool.parameters["properties"]["artifacts"]["items"]["properties"]["kind"]["enum"],
            json!(["research"])
        );
    }

    #[test]
    fn chapter_level_submission_tools_pin_the_active_chapter_id() {
        let chapter_id = Uuid::new_v4();
        for stage in [
            PhaseSubmissionKind::ChapterOutline { chapter_id },
            PhaseSubmissionKind::Chapter { chapter_id },
        ] {
            assert_eq!(
                submission_tool(&stage).parameters["properties"]["chapter_id"]["enum"],
                json!([chapter_id.to_string()])
            );
        }
    }

    #[test]
    fn preparation_contract_only_requires_the_artifacts_being_revised() {
        let research_stage = PhaseSubmissionKind::Preparation {
            required_artifacts: vec![PreparationArtifactKind::Research],
        };
        let contract = submission_contract(&research_stage);
        assert!(contract.contains(SUBMIT_PREPARATION_TOOL));
        for unexpected in ["background", "characters", "outline 各一次"] {
            assert!(
                !contract.contains(unexpected),
                "修订合同不应要求 {unexpected}"
            );
        }
        assert!(contract.contains("本轮不要提交 `chapter_plan`"));

        let research_only = json!({
            "artifacts": [{ "kind": "research", "content": "补充资料" }]
        });
        let PhaseSubmission::Preparation(submission) =
            parse_phase_submission(&research_stage, &research_only).unwrap()
        else {
            panic!("expected preparation submission");
        };
        assert_eq!(submission.artifacts.len(), 1);
        assert_eq!(
            submission.artifacts[0].kind,
            PreparationArtifactKind::Research
        );
        assert!(submission.chapter_plan.is_none());

        let extra_artifact = json!({
            "artifacts": [
                { "kind": "research", "content": "补充资料" },
                { "kind": "background", "content": "不应覆盖的背景" }
            ]
        });
        assert!(matches!(
            parse_phase_submission(&research_stage, &extra_artifact),
            Err(PhaseSubmissionError::Invalid { .. })
        ));

        let unexpected_plan = json!({
            "artifacts": [{ "kind": "research", "content": "补充资料" }],
            "chapter_plan": [{ "title": "第一章", "goal": "不应修改章节计划" }]
        });
        assert!(matches!(
            parse_phase_submission(&research_stage, &unexpected_plan),
            Err(PhaseSubmissionError::Invalid { .. })
        ));

        let outline_stage = PhaseSubmissionKind::Preparation {
            required_artifacts: vec![PreparationArtifactKind::Outline],
        };
        let outline_contract = submission_contract(&outline_stage);
        assert!(outline_contract.contains("outline"));
        assert!(outline_contract.contains("章节清单的唯一承载字段"));
        let outline_submission = json!({
            "artifacts": [{ "kind": "outline", "content": "修订后的粗纲" }],
            "chapter_plan": [{ "title": "第一章", "goal": "从开场引入冲突" }]
        });
        assert!(matches!(
            parse_phase_submission(&outline_stage, &outline_submission),
            Ok(PhaseSubmission::Preparation(_))
        ));
    }

    #[test]
    fn outline_and_chapter_submissions_are_bound_to_the_active_chapter() {
        let chapter_id = Uuid::new_v4();
        let mismatch = json!({
            "chapter_id": Uuid::new_v4(),
            "content": "细纲"
        });
        assert!(matches!(
            parse_phase_submission(
                &PhaseSubmissionKind::ChapterOutline { chapter_id },
                &mismatch
            ),
            Err(PhaseSubmissionError::ChapterIdMismatch)
        ));

        let chapter = json!({
            "chapter_id": chapter_id,
            "summary": "本章完成一次转折。"
        });
        assert!(matches!(
            parse_phase_submission(&PhaseSubmissionKind::Chapter { chapter_id }, &chapter),
            Ok(PhaseSubmission::Chapter(_))
        ));
    }

    #[test]
    fn review_submission_accepts_empty_issues_but_rejects_invalid_confidence() {
        let accepted = json!({ "passed": false, "confidence": 0.4, "issues": [] });
        assert!(matches!(
            parse_phase_submission(&PhaseSubmissionKind::Review, &accepted),
            Ok(PhaseSubmission::Review(ReviewSubmission { issues, .. })) if issues.is_empty()
        ));

        let invalid = json!({ "passed": true, "confidence": 1.2, "issues": [] });
        assert!(matches!(
            parse_phase_submission(&PhaseSubmissionKind::Review, &invalid),
            Err(PhaseSubmissionError::Invalid { .. })
        ));
    }

    /// 真实端点上过的那一次：`issues` 是装着数组的 JSON 字符串。内容对、形状错，
    /// 这条原因会原样进入配对的错误工具结果与纠正消息，所以必须点名字段与正确
    /// 形状，而不是只报一句类型不符。
    #[test]
    fn review_issues_as_a_json_string_names_the_field_and_the_expected_shape() {
        let arguments = json!({
            "passed": true,
            "confidence": 0.82,
            "issues": "[\"【建议】第三章：承接稍显含糊。\"]",
        });
        let error = parse_phase_submission(&PhaseSubmissionKind::Review, &arguments).unwrap_err();
        assert!(matches!(error, PhaseSubmissionError::InvalidJson { .. }));
        let message = error.to_string();
        assert!(message.contains("issues"), "{message}");
        assert!(
            message.contains("不能是装着数组的 JSON 字符串"),
            "{message}"
        );

        // 非数组元素同样点名字段与收到的类型；真正的数组仍然通过。
        let mixed = json!({ "passed": true, "confidence": 0.5, "issues": [7] });
        let error = parse_phase_submission(&PhaseSubmissionKind::Review, &mixed).unwrap_err();
        assert!(
            error.to_string().contains("issues 的每一项都必须是字符串"),
            "{error}"
        );
        let array = json!({ "passed": true, "confidence": 0.5, "issues": ["建议"] });
        assert!(matches!(
            parse_phase_submission(&PhaseSubmissionKind::Review, &array),
            Ok(PhaseSubmission::Review(ReviewSubmission { issues, .. })) if issues == vec!["建议".to_owned()]
        ));
    }

    #[test]
    fn submissions_only_come_from_submission_tool_arguments() {
        // 散文、字符串参数、缺字段的数组都不是提交：解析器不接受任何非对象参数。
        for arguments in [json!("我认为本章通过。"), json!(["passed"]), json!(3)] {
            assert!(matches!(
                parse_phase_submission(&PhaseSubmissionKind::Review, &arguments),
                Err(PhaseSubmissionError::InvalidJson { .. })
            ));
        }
    }

    /// 被端点截断的提交参数不是传输失败：它必须变成一次被拒的提交，模型拿得到
    /// 配对错误结果与纠正，而不是让整轮以 `provider_error` 结束。
    #[test]
    fn undecodable_submission_arguments_are_a_rejected_submission() {
        let truncated = r#"{"artifacts":[{"kind":"background","content":"【创作约定】未写完的背"#;
        let stage = preparation_stage();
        let error = classify_model_completion(
            stage.clone(),
            &submission_request(&stage),
            submission_completion(&stage, json!(truncated)),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            AgentRunnerError::InvalidPhaseSubmission {
                tool: SUBMIT_PREPARATION_TOOL,
                error: PhaseSubmissionError::InvalidJson { .. },
                ..
            }
        ));
        assert!(error.to_string().contains("不是合法 JSON"), "{error}");
    }

    /// 业务工具的调用参数不可解码时同样给出一条可纠正的错误：调用方不会拿到无法
    /// 执行的参数，模型也能在图一轮里重写这次调用。
    #[test]
    fn undecodable_business_tool_arguments_are_rejected_with_a_reason() {
        let completion = ModelCompletion {
            provider_request_id: None,
            content: None,
            tool_calls: vec![ModelToolCall {
                id: "call-1".to_owned(),
                name: "append_content".to_owned(),
                arguments: json!("{\"content\":\"未写完"),
            }],
            usage: None,
            usage_raw: None,
            finish_reason: ModelFinishReason::ToolCalls,
        };
        let error = classify_model_completion(
            PhaseSubmissionKind::Chapter {
                chapter_id: Uuid::new_v4(),
            },
            &request(vec![AgentToolDefinition {
                name: "append_content".to_owned(),
                description: "追加正文".to_owned(),
                parameters: json!({ "type": "object" }),
            }]),
            completion,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            AgentRunnerError::UndecodableToolArguments { ref tool, .. } if tool == "append_content"
        ));
        assert!(error.to_string().contains("不是合法 JSON"), "{error}");
    }

    #[tokio::test]
    async fn runner_rejects_tool_outside_the_current_stage_whitelist() {
        let completion = ModelCompletion {
            provider_request_id: None,
            content: None,
            tool_calls: vec![ModelToolCall {
                id: "call-1".to_owned(),
                name: "append_content".to_owned(),
                arguments: json!({ "content": "正文" }),
            }],
            usage: None,
            usage_raw: None,
            finish_reason: ModelFinishReason::ToolCalls,
        };
        let runner = AgentRunner::new(StubProvider {
            completion: Arc::new(completion),
        });
        let mut events = |_| {};
        let error = runner
            .run_round(
                preparation_stage(),
                &request(vec![AgentToolDefinition {
                    name: "append_content".to_owned(),
                    description: "错误工具".to_owned(),
                    parameters: json!({ "type": "object" }),
                }]),
                &mut events,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            AgentRunnerError::DisallowedDeclaredTool { .. }
        ));
    }

    #[tokio::test]
    async fn runner_returns_an_allowlisted_tool_call_or_a_typed_submission() {
        let tool_completion = ModelCompletion {
            provider_request_id: None,
            content: None,
            tool_calls: vec![ModelToolCall {
                id: "call-1".to_owned(),
                name: "append_content".to_owned(),
                arguments: json!({ "content": "正文" }),
            }],
            usage: None,
            usage_raw: None,
            finish_reason: ModelFinishReason::ToolCalls,
        };
        let runner = AgentRunner::new(StubProvider {
            completion: Arc::new(tool_completion),
        });
        let tool = AgentToolDefinition {
            name: "append_content".to_owned(),
            description: "追加正文".to_owned(),
            parameters: json!({ "type": "object" }),
        };
        let mut events = |_| {};
        assert!(matches!(
            runner
                .run_round(
                    PhaseSubmissionKind::Chapter {
                        chapter_id: Uuid::new_v4()
                    },
                    &request(vec![tool]),
                    &mut events,
                )
                .await,
            Ok(AgentRoundOutcome::ToolCall(call))
                if call.name == "append_content"
                    && call.arguments == json!({ "content": "正文" })
        ));

        let chapter_id = Uuid::new_v4();
        let stage = PhaseSubmissionKind::Chapter { chapter_id };
        let runner = AgentRunner::new(StubProvider {
            completion: Arc::new(submission_completion(
                &stage,
                json!({ "chapter_id": chapter_id, "summary": "本章完成。" }),
            )),
        });
        assert!(matches!(
            runner
                .run_round(stage.clone(), &submission_request(&stage), &mut events)
                .await,
            Ok(AgentRoundOutcome::Submission(PhaseSubmission::Chapter(_)))
        ));
    }

    #[tokio::test]
    async fn runner_demands_the_submission_tool_and_keeps_invalid_arguments() {
        let chapter_id = Uuid::new_v4();
        let stage = PhaseSubmissionKind::Chapter { chapter_id };
        let mut events = |_| {};

        // 普通文本不是提交：本阶段的终态结果必须来自提交工具。
        let runner = AgentRunner::new(StubProvider {
            completion: Arc::new(stop_completion("本章完成，摘要：雨夜重逢。")),
        });
        let error = runner
            .run_round(stage.clone(), &submission_request(&stage), &mut events)
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            AgentRunnerError::MissingSubmissionToolCall {
                tool: SUBMIT_CHAPTER_TOOL,
                ..
            }
        ));

        // 参数缺字段：原始参数被保留，下一轮的配对工具结果能点名缺了什么。
        let runner = AgentRunner::new(StubProvider {
            completion: Arc::new(submission_completion(
                &stage,
                json!({ "chapter_id": chapter_id }),
            )),
        });
        let error = runner
            .run_round(stage.clone(), &submission_request(&stage), &mut events)
            .await
            .unwrap_err();
        let raw_response = match &error {
            AgentRunnerError::InvalidPhaseSubmission { raw_response, .. } => raw_response.clone(),
            other => panic!("expected invalid submission, got {other:?}"),
        };
        assert!(raw_response.contains("chapter_id"));
    }

    #[tokio::test]
    async fn runner_never_drops_parallel_calls() {
        let mut events = |_| {};
        let parallel = ModelCompletion {
            provider_request_id: None,
            content: None,
            tool_calls: vec![
                ModelToolCall {
                    id: "call-1".to_owned(),
                    name: "append_content".to_owned(),
                    arguments: json!({ "content": "正文" }),
                },
                ModelToolCall {
                    id: "call-2".to_owned(),
                    name: "replace_content".to_owned(),
                    arguments: json!({ "old_str": "旧", "new_str": "新" }),
                },
            ],
            usage: None,
            usage_raw: None,
            finish_reason: ModelFinishReason::ToolCalls,
        };
        let runner = AgentRunner::new(StubProvider {
            completion: Arc::new(parallel),
        });
        let error = runner
            .run_round(
                PhaseSubmissionKind::Chapter {
                    chapter_id: Uuid::new_v4(),
                },
                &request(vec![
                    AgentToolDefinition {
                        name: "append_content".to_owned(),
                        description: "追加正文".to_owned(),
                        parameters: json!({ "type": "object" }),
                    },
                    AgentToolDefinition {
                        name: "replace_content".to_owned(),
                        description: "替换正文".to_owned(),
                        parameters: json!({ "type": "object" }),
                    },
                ]),
                &mut events,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            AgentRunnerError::ParallelToolCalls { count: 2 }
        ));
    }

    #[test]
    fn submission_contract_and_tool_state_every_field_the_parser_requires() {
        let chapter_id = Uuid::new_v4();
        let chord = chapter_id.to_string();

        let preparation = submission_contract(&preparation_stage());
        for key in [
            SUBMIT_PREPARATION_TOOL,
            "artifacts",
            "chapter_plan",
            "background",
            "research",
            "characters",
            "outline",
        ] {
            assert!(preparation.contains(key), "准备合同缺少 {key}");
        }
        // 章节计划的字段在工具 schema 里声明，不在合同散文里。
        let plan_schema =
            &submission_tool(&preparation_stage()).parameters["properties"]["chapter_plan"];
        for key in ["title", "goal"] {
            let required = plan_schema["items"]["required"].as_array();
            assert!(
                required.is_some_and(|required| required.contains(&json!(key))),
                "chapter_plan schema 缺少 {key}"
            );
        }

        let outline = submission_contract(&PhaseSubmissionKind::ChapterOutline { chapter_id });
        for key in [SUBMIT_CHAPTER_OUTLINE_TOOL, "chapter_id", "content"] {
            assert!(outline.contains(key), "细纲合同缺少 {key}");
        }
        // 模型无法推断的章节 ID 必须作为具体值出现，否则提交永远无法通过。
        assert!(outline.contains(&chord));

        let chapter = submission_contract(&PhaseSubmissionKind::Chapter { chapter_id });
        assert!(chapter.contains(SUBMIT_CHAPTER_TOOL));
        assert!(chapter.contains("summary"));
        assert!(chapter.contains("append_content"));
        assert!(chapter.contains(&chord));

        let review = submission_contract(&PhaseSubmissionKind::Review);
        for key in [SUBMIT_REVIEW_TOOL, "passed", "confidence", "issues"] {
            assert!(review.contains(key), "审核合同缺少 {key}");
        }
        assert_eq!(
            submission_tool(&PhaseSubmissionKind::Review).parameters["required"],
            json!(["passed", "confidence", "issues"])
        );
    }
}
