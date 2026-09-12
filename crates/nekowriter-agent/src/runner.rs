//! 单轮 Agent 协议编排。
//!
//! 模型适配器负责传输和流协议；本模块负责将完整响应限制为「一个允许的工具调用」
//! 或「当前阶段的类型化提交」。它不访问数据库，也不直接修改正文：App 服务在
//! 持久化请求快照后调用它，并把返回的工具调用交给 [`ToolHost`]。

use std::collections::BTreeSet;

use futures_util::future::BoxFuture;
use nekowriter_core::{
    ChapterId, ChapterPlan, ChapterPlanDraft, MAX_ARTIFACT_CONTENT_LEN, MAX_CHAPTER_GOAL_LEN,
    MAX_REVIEW_ISSUES_LEN, NewChapterOutline, PreparationArtifactInput, PreparationArtifactKind,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

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
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChapterOutlineSubmission {
    pub chapter_id: ChapterId,
    pub content: NewChapterOutline,
    pub goals: Vec<String>,
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
    #[error("模型返回工具调用时的终止原因无效")]
    InvalidToolFinishReason,
    #[error("模型一次响应给出 {count} 个工具调用；仅允许一个")]
    ParallelToolCalls { count: usize },
    #[error("模型不能在同一响应同时返回正文文本和工具调用")]
    TextAndToolCalls,
    #[error("模型终态提交无效：{error}")]
    InvalidPhaseSubmission {
        /// 供 App 服务持久化的完整 assistant 响应；不会进入错误显示文本。
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
    #[error("模型的终态响应为空")]
    EmptyFinalResponse,
    #[error("模型终态响应不是当前阶段要求的 JSON：{reason}")]
    InvalidJson { reason: String },
    #[error("阶段提交字段无效：{reason}")]
    Invalid { reason: String },
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

/// 从已经完整接收的模型文本解析阶段提交。
///
/// 这项函数不从散文中寻找 JSON，也不容忍未知字段。原始响应由调用方保留后，模型
/// 可以在下一轮收到明确的协议纠正结果。
///
/// # Errors
///
/// 响应不是当前阶段要求的完整 JSON，或其中字段未通过领域校验时返回错误。
pub fn parse_phase_submission(
    stage: &PhaseSubmissionKind,
    content: &str,
) -> Result<PhaseSubmission, PhaseSubmissionError> {
    if content.trim().is_empty() {
        return Err(PhaseSubmissionError::EmptyFinalResponse);
    }
    match stage {
        PhaseSubmissionKind::Preparation { required_artifacts } => {
            parse_preparation_submission(required_artifacts, content)
        }
        PhaseSubmissionKind::ChapterOutline { chapter_id } => {
            parse_outline_submission(*chapter_id, content)
        }
        PhaseSubmissionKind::Chapter { chapter_id } => {
            parse_chapter_submission(*chapter_id, content)
        }
        PhaseSubmissionKind::Review => parse_review_submission(content),
    }
}

/// 校验一个已经由模型适配器完整接收的响应，并分类为工具调用或阶段提交。
///
/// App 可先持久化请求检查点和完整传输响应，再调用本函数；这样提供商请求 ID 和
/// 用量不会因严格阶段校验失败而丢失。
///
/// # Errors
///
/// 工具白名单、结束原因或最终结构化提交不符合当前阶段合同时返回
/// [`AgentRunnerError`]，调用方应保留原始响应并将本次 attempt 标记为失败。
pub fn classify_model_completion(
    stage: PhaseSubmissionKind,
    request: &ModelRequest,
    completion: ModelCompletion,
) -> Result<AgentRoundOutcome, AgentRunnerError> {
    validate_declared_tools(&stage, request)?;
    if !completion.tool_calls.is_empty() {
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
        let call = completion
            .tool_calls
            .into_iter()
            .next()
            .expect("length checked above");
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
        return Ok(AgentRoundOutcome::ToolCall(call));
    }
    if completion.finish_reason != ModelFinishReason::Stop {
        return Err(AgentRunnerError::InvalidToolFinishReason);
    }
    let content = completion.content.unwrap_or_default();
    parse_phase_submission(&stage, &content)
        .map(AgentRoundOutcome::Submission)
        .map_err(|error| AgentRunnerError::InvalidPhaseSubmission {
            raw_response: content,
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
        PhaseSubmissionKind::Preparation { .. } | PhaseSubmissionKind::ChapterOutline { .. } => {
            &["ask_user", "query_knowledge", "research_web"]
        }
        PhaseSubmissionKind::Chapter { .. } => &["ask_user", "append_content", "replace_content"],
        PhaseSubmissionKind::Review => &[],
    }
}

/// 当前阶段必须出现在系统消息里的提交结构说明。
///
/// docs/02 §4.1 要求阶段角色包含本阶段的「提交结构」，而章节级提交还带有一个模型无法
/// 自行推断的 `chapter_id`。合同文本与下方的解析器同源放在这里，避免提示词和校验各自
/// 漂移：改了字段就必须同时改 [`parse_phase_submission`]。
#[must_use]
pub fn submission_contract(stage: &PhaseSubmissionKind) -> String {
    match stage {
        PhaseSubmissionKind::Preparation { required_artifacts } => {
            preparation_contract(required_artifacts)
        }
        PhaseSubmissionKind::ChapterOutline { chapter_id } => format!(
            "阶段提交：完成后只输出一个 JSON 对象，不要使用 Markdown 代码围栏或附加解释。\n\
{{\"chapter_id\":\"{chapter_id}\",\"content\":\"本细纲正文\",\"goals\":[\"细纲目标一\"]}}\n\
chapter_id 必须逐字为 \"{chapter_id}\"；content 与每个 goal 都不能为空。"
        ),
        PhaseSubmissionKind::Chapter { chapter_id } => format!(
            "正文只能通过 append_content / replace_content 工具修改，不要在没有工具调用时把正文写进普通文本。\n\
本章结束时只输出一个 JSON 对象，不要使用 Markdown 代码围栏或附加解释：\n\
{{\"chapter_id\":\"{chapter_id}\",\"summary\":\"一句话本章小结\"}}\n\
chapter_id 必须逐字为 \"{chapter_id}\"；summary 不能为空。"
        ),
        PhaseSubmissionKind::Review => {
            "阶段提交：完成后只输出一个 JSON 对象，不要使用 Markdown 代码围栏或附加解释。\n\
{\"passed\":true,\"confidence\":0.0,\"issues\":[]}\n\
passed 为布尔值；confidence 为 0 到 1 之间的数值；issues 为字符串数组，没有具体问题时留空数组。"
                .to_owned()
        }
    }
}

/// 准备阶段的合同文本：骨架与必填规则都按本轮真正需要提交的产物项生成。
fn preparation_contract(required_artifacts: &[PreparationArtifactKind]) -> String {
    let skeleton = required_artifacts
        .iter()
        .map(|kind| format!("{{\"kind\":\"{}\",\"content\":\"…\"}}", kind.stable_key()))
        .collect::<Vec<_>>()
        .join(",");
    let names = required_artifacts
        .iter()
        .map(|kind| kind.stable_key())
        .collect::<Vec<_>>()
        .join("、");
    let outline_required = required_artifacts.contains(&PreparationArtifactKind::Outline);
    let plan_skeleton = if outline_required {
        ",\"chapter_plan\":[{\"title\":\"…\",\"goal\":\"…\"}]"
    } else {
        ""
    };
    let plan_rule = if outline_required {
        "chapter_plan 至少一项。"
    } else {
        "本轮不要输出 chapter_plan：没有修订粗纲就不能改章节计划。"
    };
    format!(
        "阶段提交：完成后只输出一个 JSON 对象，不要使用 Markdown 代码围栏或附加解释。\n\
{{\"artifacts\":[{skeleton}]{plan_skeleton}}}\n\
artifacts 必须恰好包含 {names} 各一次（本轮只需提交这些产物）；{plan_rule}"
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
    content: &str,
) -> Result<PhaseSubmission, PhaseSubmissionError> {
    let raw = decode_submission::<RawPreparationSubmission>(content)?;
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
            return Err(PhaseSubmissionError::Invalid {
                reason: "提交粗纲时必须同时给出至少一项章节计划".to_owned(),
            });
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
    goals: Vec<String>,
}

fn parse_outline_submission(
    expected_chapter_id: ChapterId,
    content: &str,
) -> Result<PhaseSubmission, PhaseSubmissionError> {
    let raw = decode_submission::<RawChapterOutlineSubmission>(content)?;
    if raw.chapter_id != expected_chapter_id {
        return Err(PhaseSubmissionError::ChapterIdMismatch);
    }
    let content =
        NewChapterOutline::new(raw.content).map_err(|error| PhaseSubmissionError::Invalid {
            reason: error.to_string(),
        })?;
    validate_goals(&raw.goals)?;
    Ok(PhaseSubmission::ChapterOutline(ChapterOutlineSubmission {
        chapter_id: raw.chapter_id,
        content,
        goals: raw.goals,
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
    content: &str,
) -> Result<PhaseSubmission, PhaseSubmissionError> {
    let raw = decode_submission::<RawChapterSubmission>(content)?;
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
    issues: Vec<String>,
}

fn parse_review_submission(content: &str) -> Result<PhaseSubmission, PhaseSubmissionError> {
    let raw = decode_submission::<RawReviewSubmission>(content)?;
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
    content: &str,
) -> Result<T, PhaseSubmissionError> {
    serde_json::from_str(content).map_err(|error| PhaseSubmissionError::InvalidJson {
        reason: error.to_string(),
    })
}

fn validate_goals(goals: &[String]) -> Result<(), PhaseSubmissionError> {
    for goal in goals {
        validate_text("细纲目标", goal, MAX_CHAPTER_GOAL_LEN)?;
    }
    Ok(())
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
    use crate::{AgentToolDefinition, ContextMessageRole, provider::ModelRequestMessage};
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
                structured_output: true,
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
            response_format: None,
            thinking_level: ThinkingLevel::Off,
        }
    }

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

    fn preparation_stage() -> PhaseSubmissionKind {
        PhaseSubmissionKind::Preparation {
            required_artifacts: PreparationArtifactKind::ALL.to_vec(),
        }
    }

    #[test]
    fn preparation_submission_requires_exactly_four_distinct_artifacts() {
        let content = json!({
            "artifacts": [
                { "kind": "background", "content": "背景" },
                { "kind": "research", "content": "资料" },
                { "kind": "characters", "content": "人物" }
            ],
            "chapter_plan": [{ "title": "第一章", "goal": "开场" }]
        })
        .to_string();
        let error = parse_phase_submission(&preparation_stage(), &content).unwrap_err();
        assert!(matches!(error, PhaseSubmissionError::Invalid { .. }));

        let content = json!({
            "artifacts": [
                { "kind": "outline", "content": "粗纲" },
                { "kind": "characters", "content": "人物" },
                { "kind": "research", "content": "资料" },
                { "kind": "background", "content": "背景" }
            ],
            "chapter_plan": [{ "title": "第一章", "goal": "开场" }]
        })
        .to_string();
        let PhaseSubmission::Preparation(submission) =
            parse_phase_submission(&preparation_stage(), &content).unwrap()
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

    #[test]
    fn preparation_revision_contract_only_requires_the_artifacts_being_revised() {
        let research_stage = PhaseSubmissionKind::Preparation {
            required_artifacts: vec![PreparationArtifactKind::Research],
        };
        let contract = submission_contract(&research_stage);
        assert!(contract.contains("{\"kind\":\"research\""));
        for unexpected in [
            "{\"kind\":\"background\"",
            "{\"kind\":\"characters\"",
            "{\"kind\":\"outline\"",
        ] {
            assert!(
                !contract.contains(unexpected),
                "修订合同不应要求 {unexpected}"
            );
        }
        assert!(contract.contains("本轮不要输出 chapter_plan"));

        let research_only = json!({
            "artifacts": [{ "kind": "research", "content": "补充资料" }]
        })
        .to_string();
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
        })
        .to_string();
        assert!(matches!(
            parse_phase_submission(&research_stage, &extra_artifact),
            Err(PhaseSubmissionError::Invalid { .. })
        ));

        let unexpected_plan = json!({
            "artifacts": [{ "kind": "research", "content": "补充资料" }],
            "chapter_plan": [{ "title": "第一章", "goal": "不应修改章节计划" }]
        })
        .to_string();
        assert!(matches!(
            parse_phase_submission(&research_stage, &unexpected_plan),
            Err(PhaseSubmissionError::Invalid { .. })
        ));

        let outline_stage = PhaseSubmissionKind::Preparation {
            required_artifacts: vec![PreparationArtifactKind::Outline],
        };
        let outline_contract = submission_contract(&outline_stage);
        assert!(outline_contract.contains("{\"kind\":\"outline\""));
        assert!(outline_contract.contains("chapter_plan 至少一项"));
        let outline_submission = json!({
            "artifacts": [{ "kind": "outline", "content": "修订后的粗纲" }],
            "chapter_plan": [{ "title": "第一章", "goal": "从开场引入冲突" }]
        })
        .to_string();
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
            "content": "细纲",
            "goals": ["推进冲突"]
        })
        .to_string();
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
        })
        .to_string();
        assert!(matches!(
            parse_phase_submission(&PhaseSubmissionKind::Chapter { chapter_id }, &chapter),
            Ok(PhaseSubmission::Chapter(_))
        ));
    }

    #[test]
    fn review_submission_accepts_empty_issues_but_rejects_invalid_confidence() {
        let accepted = json!({ "passed": false, "confidence": 0.4, "issues": [] }).to_string();
        assert!(matches!(
            parse_phase_submission(&PhaseSubmissionKind::Review, &accepted),
            Ok(PhaseSubmission::Review(ReviewSubmission { issues, .. })) if issues.is_empty()
        ));

        let invalid = json!({ "passed": true, "confidence": 1.2, "issues": [] }).to_string();
        assert!(matches!(
            parse_phase_submission(&PhaseSubmissionKind::Review, &invalid),
            Err(PhaseSubmissionError::Invalid { .. })
        ));
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
            Ok(AgentRoundOutcome::ToolCall(ModelToolCall { name, .. })) if name == "append_content"
        ));

        let chapter_id = Uuid::new_v4();
        let runner = AgentRunner::new(StubProvider {
            completion: Arc::new(stop_completion(
                json!({ "chapter_id": chapter_id, "summary": "本章完成。" }).to_string(),
            )),
        });
        assert!(matches!(
            runner
                .run_round(
                    PhaseSubmissionKind::Chapter { chapter_id },
                    &request(Vec::new()),
                    &mut events,
                )
                .await,
            Ok(AgentRoundOutcome::Submission(PhaseSubmission::Chapter(_)))
        ));
    }

    #[tokio::test]
    async fn runner_preserves_invalid_final_text_and_never_drops_parallel_calls() {
        let raw_response = "这不是 JSON".to_owned();
        let runner = AgentRunner::new(StubProvider {
            completion: Arc::new(stop_completion(raw_response.clone())),
        });
        let mut events = |_| {};
        let error = runner
            .run_round(
                PhaseSubmissionKind::Review,
                &request(Vec::new()),
                &mut events,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            AgentRunnerError::InvalidPhaseSubmission {
                raw_response: saved,
                error: PhaseSubmissionError::InvalidJson { .. },
            } if saved == raw_response
        ));

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
    fn plain_prose_is_never_guessed_as_a_submission() {
        assert!(matches!(
            parse_phase_submission(&PhaseSubmissionKind::Review, "我认为本章通过。"),
            Err(PhaseSubmissionError::InvalidJson { .. })
        ));
    }

    #[test]
    fn submission_contract_states_every_field_the_parser_requires() {
        let chapter_id = Uuid::new_v4();
        let chord = chapter_id.to_string();

        let preparation = submission_contract(&preparation_stage());
        for key in [
            "artifacts",
            "chapter_plan",
            "background",
            "research",
            "characters",
            "outline",
            "title",
            "goal",
        ] {
            assert!(preparation.contains(key), "准备合同缺少 {key}");
        }

        let outline = submission_contract(&PhaseSubmissionKind::ChapterOutline { chapter_id });
        for key in ["chapter_id", "content", "goals"] {
            assert!(outline.contains(key), "细纲合同缺少 {key}");
        }
        // 模型无法推断的章节 ID 必须作为具体值出现，否则提交永远无法通过。
        assert!(outline.contains(&chord));

        let chapter = submission_contract(&PhaseSubmissionKind::Chapter { chapter_id });
        assert!(chapter.contains("summary"));
        assert!(chapter.contains(&chord));

        let review = submission_contract(&PhaseSubmissionKind::Review);
        for key in ["passed", "confidence", "issues"] {
            assert!(review.contains(key), "审核合同缺少 {key}");
        }
    }
}
