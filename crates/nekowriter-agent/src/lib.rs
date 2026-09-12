//! Agent 的稳定边界：内建提示词、上下文构建与后续模型适配器共享的能力描述。
//!
//! M0 不假装存在已经接通的模型服务。真正的 Chat Completions 适配器将在
//! M1 建立在本目录之上。

use std::collections::HashSet;

use nekowriter_core::{ContextFold, RevisionId, RunId, ThinkingLevel};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::provider::{
    ModelRequest, ModelRequestMessage, StructuredOutput, validate_request_message_sequence,
};

pub mod embedding;
pub mod engine;
pub mod prompt;
pub mod provider;
pub mod runner;

pub use prompt::{
    PromptVariableDefinition, RenderedBuiltinPrompt, builtin_prompt_examples,
    builtin_prompt_variables, builtin_template_variables, render_builtin_prompt,
    render_builtin_prompt_content, render_prompt_content, validate_builtin_prompt_pair,
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptKey {
    PreparationGenerate,
    ChapterOutline,
    ChapterWrite,
    ReviewPreparationBackground,
    ReviewPreparationResearch,
    ReviewPreparationCharacters,
    ReviewPreparationOutline,
    ReviewChapterOutline,
    ReviewChapterEdit,
    ReviewChapterFinal,
}

impl PromptKey {
    pub const ALL: [Self; 10] = [
        Self::PreparationGenerate,
        Self::ChapterOutline,
        Self::ChapterWrite,
        Self::ReviewPreparationBackground,
        Self::ReviewPreparationResearch,
        Self::ReviewPreparationCharacters,
        Self::ReviewPreparationOutline,
        Self::ReviewChapterOutline,
        Self::ReviewChapterEdit,
        Self::ReviewChapterFinal,
    ];

    /// 解析 HTTP 合同中使用的稳定键（例如 `chapter.write`）。
    ///
    /// 覆盖记录和界面都以稳定键为身份，不依赖 Rust 枚举名的 serde 形式。
    #[must_use]
    pub fn from_stable_key(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|key| key.as_str() == value)
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PreparationGenerate => "preparation.generate",
            Self::ChapterOutline => "chapter.outline",
            Self::ChapterWrite => "chapter.write",
            Self::ReviewPreparationBackground => "review.preparation.background",
            Self::ReviewPreparationResearch => "review.preparation.research",
            Self::ReviewPreparationCharacters => "review.preparation.characters",
            Self::ReviewPreparationOutline => "review.preparation.outline",
            Self::ReviewChapterOutline => "review.chapter.outline",
            Self::ReviewChapterEdit => "review.chapter.edit",
            Self::ReviewChapterFinal => "review.chapter.final",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BuiltinPrompt {
    pub key: PromptKey,
    pub system: &'static str,
    pub user: &'static str,
}

#[must_use]
pub fn builtin_prompt(key: PromptKey) -> BuiltinPrompt {
    match key {
        PromptKey::PreparationGenerate => BuiltinPrompt {
            key,
            system: include_str!("../prompts/preparation.generate.system.md"),
            user: include_str!("../prompts/preparation.generate.user.md"),
        },
        PromptKey::ChapterOutline => BuiltinPrompt {
            key,
            system: include_str!("../prompts/chapter.outline.system.md"),
            user: include_str!("../prompts/chapter.outline.user.md"),
        },
        PromptKey::ChapterWrite => BuiltinPrompt {
            key,
            system: include_str!("../prompts/chapter.write.system.md"),
            user: include_str!("../prompts/chapter.write.user.md"),
        },
        PromptKey::ReviewPreparationBackground => BuiltinPrompt {
            key,
            system: include_str!("../prompts/review.preparation.background.system.md"),
            user: include_str!("../prompts/review.preparation.background.user.md"),
        },
        PromptKey::ReviewPreparationResearch => BuiltinPrompt {
            key,
            system: include_str!("../prompts/review.preparation.research.system.md"),
            user: include_str!("../prompts/review.preparation.research.user.md"),
        },
        PromptKey::ReviewPreparationCharacters => BuiltinPrompt {
            key,
            system: include_str!("../prompts/review.preparation.characters.system.md"),
            user: include_str!("../prompts/review.preparation.characters.user.md"),
        },
        PromptKey::ReviewPreparationOutline => BuiltinPrompt {
            key,
            system: include_str!("../prompts/review.preparation.outline.system.md"),
            user: include_str!("../prompts/review.preparation.outline.user.md"),
        },
        PromptKey::ReviewChapterOutline => BuiltinPrompt {
            key,
            system: include_str!("../prompts/review.chapter.outline.system.md"),
            user: include_str!("../prompts/review.chapter.outline.user.md"),
        },
        PromptKey::ReviewChapterEdit => BuiltinPrompt {
            key,
            system: include_str!("../prompts/review.chapter.edit.system.md"),
            user: include_str!("../prompts/review.chapter.edit.user.md"),
        },
        PromptKey::ReviewChapterFinal => BuiltinPrompt {
            key,
            system: include_str!("../prompts/review.chapter.final.system.md"),
            user: include_str!("../prompts/review.chapter.final.user.md"),
        },
    }
}

/// 模型传输层的文本消息角色。
///
/// 阶段折叠永远使用 [`Self::Assistant`]：它只是模型 API 所需的传输信封，不会被
/// 提升为 system 指令，也不会携带工具调用结构。
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextMessageRole {
    System,
    User,
    Assistant,
    Tool,
}

/// 即将发送给模型的一条纯文本上下文消息。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextMessage {
    pub role: ContextMessageRole,
    pub content: String,
}

/// 稳定工具定义。具体协议适配器将其转成供应商的 tools 字段，而不是把 schema
/// 拼进阶段折叠文本。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// 一个已完成写作阶段的折叠类别。
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextFoldStage {
    Preparation,
    ChapterOutline,
    ChapterBody,
}

impl ContextFoldStage {
    /// 同一章内细纲先于正文；不同章节则必须先完成前一章的全部阶段。
    ///
    /// 不能只按阶段全局排序，否则第二章细纲会被排在第一章正文之前，或同一章的
    /// 正文折叠会先于细纲折叠进入模型上下文。
    const fn within_chapter_order(self) -> u8 {
        match self {
            Self::Preparation => 0,
            Self::ChapterOutline => 0,
            Self::ChapterBody => 1,
        }
    }

    const fn requires_chapter_order(self) -> bool {
        !matches!(self, Self::Preparation)
    }
}

/// 一个通过审核、可替换其源 Run 历史的阶段上下文。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletedStageContext {
    pub stage: ContextFoldStage,
    pub chapter_order: Option<i64>,
    pub source_run_id: RunId,
    pub fold: ContextFold,
}

/// 当前阶段的一份类型化输入。
///
/// 若它来源于已经作为 fold 注入的修订，构建器会省略它，避免同一产物的全文在
/// 一次请求中出现两次。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentContextInput {
    pub role: ContextMessageRole,
    pub content: String,
    pub source_revision_id: Option<RevisionId>,
}

/// 当前未折叠阶段的一条原始消息。
///
/// `source_run_id + sequence` 是持久化消息的稳定顺序键；来自任何已折叠 Run 的
/// 消息会被拒绝，而不是静默地重新带回模型上下文。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnfoldedStageMessage {
    pub source_run_id: RunId,
    pub sequence: i64,
    pub message: ModelRequestMessage,
}

/// 当前模型资料给出的上下文预算。
///
/// 在没有供应商 tokenizer 的阶段，字符数按一 Unicode 字符至少一 Token 作保守
/// 估计；超过预算时返回错误，绝不摘要、截断或删除通过产物。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextBudget {
    pub max_context_tokens: usize,
    pub reserved_output_tokens: usize,
}

/// 构建一次模型上下文所需的全部输入。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextBuildRequest {
    pub system_instructions: String,
    pub tools: Vec<AgentToolDefinition>,
    pub completed_stages: Vec<CompletedStageContext>,
    pub current_inputs: Vec<CurrentContextInput>,
    pub current_messages: Vec<UnfoldedStageMessage>,
    pub budget: ContextBudget,
}

/// 已注入 fold 的本地来源元数据；不会写入模型可见 content。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextFoldSource {
    pub stage: ContextFoldStage,
    pub chapter_order: Option<i64>,
    pub source_run_id: RunId,
    pub artifact_revision_ids: Vec<RevisionId>,
    pub through_message_sequence: i64,
    pub content_hash: String,
}

/// 适配器可以直接使用的、顺序稳定且已完成去重的模型上下文。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltAgentContext {
    pub messages: Vec<ModelRequestMessage>,
    pub tools: Vec<AgentToolDefinition>,
    pub fold_sources: Vec<ContextFoldSource>,
    pub omitted_input_revision_ids: Vec<RevisionId>,
    pub estimated_input_tokens: usize,
    pub uses_estimated_token_count: bool,
}

impl BuiltAgentContext {
    /// 以当前已验证的消息顺序构造可交给模型适配器的请求。
    ///
    /// 该方法不重新渲染、截断或排序上下文；调用方仍必须在 dispatch 前封存它的
    /// 设置、提示词和来源元数据。思考等级由调用方从项目设置解析后传入，不在这里
    /// 读取全局状态。
    #[must_use]
    pub fn model_request(
        &self,
        response_format: Option<StructuredOutput>,
        thinking_level: ThinkingLevel,
    ) -> ModelRequest {
        ModelRequest {
            messages: self.messages.clone(),
            tools: self.tools.clone(),
            response_format,
            thinking_level,
        }
    }
}

/// 上下文输入不满足折叠或长度合同的原因。
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ContextBuildError {
    #[error("系统提示词不能为空")]
    EmptySystemInstructions,
    #[error("工具名称不能为空")]
    EmptyToolName,
    #[error("工具 {name} 重复")]
    DuplicateToolName { name: String },
    #[error("折叠阶段 {stage:?} 的章节顺序无效")]
    InvalidChapterOrder { stage: ContextFoldStage },
    #[error("折叠阶段 {stage:?} 的来源重复")]
    DuplicateFoldStage { stage: ContextFoldStage },
    #[error("折叠 Run {run_id} 重复")]
    DuplicateFoldRun { run_id: RunId },
    #[error("折叠 Run {run_id} 缺少产物修订")]
    MissingFoldArtifacts { run_id: RunId },
    #[error("折叠 Run {run_id} 的产物修订重复")]
    DuplicateFoldArtifact { run_id: RunId },
    #[error("折叠 Run {run_id} 的消息水位无效")]
    InvalidFoldWatermark { run_id: RunId },
    #[error("折叠 Run {run_id} 的纯文本为空")]
    EmptyFoldText { run_id: RunId },
    #[error("折叠 Run {run_id} 的内容散列不一致")]
    InvalidFoldHash { run_id: RunId },
    #[error("已折叠 Run {run_id} 的历史消息不能重新进入上下文")]
    FoldedRunMessage { run_id: RunId },
    #[error("当前消息序号不能为负")]
    InvalidMessageSequence,
    #[error("当前消息 ({run_id}, {sequence}) 重复")]
    DuplicateCurrentMessage { run_id: RunId, sequence: i64 },
    #[error("当前消息不符合模型协议：{reason}")]
    InvalidMessageProtocol { reason: String },
    #[error("上下文预算无效：最大值必须大于保留输出")]
    InvalidBudget,
    #[error(
        "上下文预计使用 {estimated_input_tokens} Token，加上 {reserved_output_tokens} Token 输出预留后超过上限 {max_context_tokens}"
    )]
    ContextLimitExceeded {
        estimated_input_tokens: usize,
        reserved_output_tokens: usize,
        max_context_tokens: usize,
    },
}

/// 按文档 §9.4 组装一次新的模型请求上下文。
pub struct ContextBuilder;

impl ContextBuilder {
    /// # Errors
    ///
    /// 输入包含不可信 fold、已折叠历史、重复工具/消息或超过上下文预算时返回
    /// [`ContextBuildError`]；函数从不自动摘要或裁剪文本。
    pub fn build(request: ContextBuildRequest) -> Result<BuiltAgentContext, ContextBuildError> {
        if request.system_instructions.trim().is_empty() {
            return Err(ContextBuildError::EmptySystemInstructions);
        }
        if request.budget.max_context_tokens <= request.budget.reserved_output_tokens {
            return Err(ContextBuildError::InvalidBudget);
        }

        let mut tools = request.tools;
        tools.sort_by(|left, right| left.name.cmp(&right.name));
        for pair in tools.windows(2) {
            if pair[0].name.trim().is_empty() {
                return Err(ContextBuildError::EmptyToolName);
            }
            if pair[0].name == pair[1].name {
                return Err(ContextBuildError::DuplicateToolName {
                    name: pair[0].name.clone(),
                });
            }
        }
        if tools.last().is_some_and(|tool| tool.name.trim().is_empty()) {
            return Err(ContextBuildError::EmptyToolName);
        }

        let mut completed_stages = request.completed_stages;
        let mut fold_stage_keys = HashSet::new();
        let mut folded_run_ids = HashSet::new();
        let mut folded_artifact_ids = HashSet::new();
        for completed in &completed_stages {
            let chapter_order_is_valid = match completed.stage.requires_chapter_order() {
                true => completed.chapter_order.is_some_and(|order| order > 0),
                false => completed.chapter_order.is_none(),
            };
            if !chapter_order_is_valid {
                return Err(ContextBuildError::InvalidChapterOrder {
                    stage: completed.stage,
                });
            }
            if !fold_stage_keys.insert((completed.stage, completed.chapter_order)) {
                return Err(ContextBuildError::DuplicateFoldStage {
                    stage: completed.stage,
                });
            }
            if !folded_run_ids.insert(completed.source_run_id) {
                return Err(ContextBuildError::DuplicateFoldRun {
                    run_id: completed.source_run_id,
                });
            }
            if completed.fold.artifact_revision_ids.is_empty() {
                return Err(ContextBuildError::MissingFoldArtifacts {
                    run_id: completed.source_run_id,
                });
            }
            let mut stage_artifact_ids = HashSet::new();
            for revision_id in &completed.fold.artifact_revision_ids {
                if !stage_artifact_ids.insert(*revision_id)
                    || !folded_artifact_ids.insert(*revision_id)
                {
                    return Err(ContextBuildError::DuplicateFoldArtifact {
                        run_id: completed.source_run_id,
                    });
                }
            }
            if completed.fold.through_message_sequence < 0 {
                return Err(ContextBuildError::InvalidFoldWatermark {
                    run_id: completed.source_run_id,
                });
            }
            if completed.fold.plain_text.trim().is_empty() {
                return Err(ContextBuildError::EmptyFoldText {
                    run_id: completed.source_run_id,
                });
            }
            if fold_content_hash(&completed.fold.plain_text) != completed.fold.content_hash {
                return Err(ContextBuildError::InvalidFoldHash {
                    run_id: completed.source_run_id,
                });
            }
        }
        completed_stages.sort_by_key(|completed| match completed.stage {
            ContextFoldStage::Preparation => (0, 0, 0, completed.source_run_id.as_u128()),
            ContextFoldStage::ChapterOutline | ContextFoldStage::ChapterBody => (
                1,
                completed.chapter_order.expect("validated above"),
                completed.stage.within_chapter_order(),
                completed.source_run_id.as_u128(),
            ),
        });

        let mut current_messages = request.current_messages;
        let mut current_message_keys = HashSet::new();
        for message in &current_messages {
            if folded_run_ids.contains(&message.source_run_id) {
                return Err(ContextBuildError::FoldedRunMessage {
                    run_id: message.source_run_id,
                });
            }
            if message.sequence < 0 {
                return Err(ContextBuildError::InvalidMessageSequence);
            }
            if !current_message_keys.insert((message.source_run_id, message.sequence)) {
                return Err(ContextBuildError::DuplicateCurrentMessage {
                    run_id: message.source_run_id,
                    sequence: message.sequence,
                });
            }
        }
        current_messages.sort_by_key(|message| (message.sequence, message.source_run_id.as_u128()));

        let mut messages = vec![ModelRequestMessage::Text {
            role: ContextMessageRole::System,
            content: request.system_instructions,
        }];
        let mut fold_sources = Vec::with_capacity(completed_stages.len());
        for completed in completed_stages {
            messages.push(ModelRequestMessage::Text {
                role: ContextMessageRole::Assistant,
                content: completed.fold.plain_text.clone(),
            });
            fold_sources.push(ContextFoldSource {
                stage: completed.stage,
                chapter_order: completed.chapter_order,
                source_run_id: completed.source_run_id,
                artifact_revision_ids: completed.fold.artifact_revision_ids,
                through_message_sequence: completed.fold.through_message_sequence,
                content_hash: completed.fold.content_hash,
            });
        }

        let mut omitted_input_revision_ids = Vec::new();
        for input in request.current_inputs {
            if input
                .source_revision_id
                .is_some_and(|revision_id| folded_artifact_ids.contains(&revision_id))
            {
                omitted_input_revision_ids.push(input.source_revision_id.expect("checked above"));
                continue;
            }
            messages.push(ModelRequestMessage::Text {
                role: input.role,
                content: input.content,
            });
        }
        for message in current_messages {
            messages.push(message.message);
        }

        validate_request_message_sequence(&messages).map_err(|error| {
            ContextBuildError::InvalidMessageProtocol {
                reason: error.to_string(),
            }
        })?;
        let estimated_input_tokens = estimated_context_tokens(&messages, &tools);
        if estimated_input_tokens.saturating_add(request.budget.reserved_output_tokens)
            > request.budget.max_context_tokens
        {
            return Err(ContextBuildError::ContextLimitExceeded {
                estimated_input_tokens,
                reserved_output_tokens: request.budget.reserved_output_tokens,
                max_context_tokens: request.budget.max_context_tokens,
            });
        }
        Ok(BuiltAgentContext {
            messages,
            tools,
            fold_sources,
            omitted_input_revision_ids,
            estimated_input_tokens,
            uses_estimated_token_count: true,
        })
    }
}

fn fold_content_hash(content: &str) -> String {
    format!("{:x}", Sha256::digest(content.as_bytes()))
}

fn estimated_context_tokens(
    messages: &[ModelRequestMessage],
    tools: &[AgentToolDefinition],
) -> usize {
    let message_tokens = messages.iter().map(estimated_message_tokens).sum::<usize>();
    let tool_tokens = tools
        .iter()
        .map(|tool| {
            tool.name.chars().count()
                + tool.description.chars().count()
                + serde_json::to_string(&tool.parameters)
                    .expect("serde_json::Value 总能序列化")
                    .chars()
                    .count()
        })
        .sum::<usize>();
    message_tokens.saturating_add(tool_tokens)
}

fn estimated_message_tokens(message: &ModelRequestMessage) -> usize {
    match message {
        ModelRequestMessage::Text { content, .. } => content.chars().count(),
        ModelRequestMessage::AssistantToolCalls {
            content,
            tool_calls,
        } => content
            .as_deref()
            .map_or(0, |content| content.chars().count())
            .saturating_add(
                tool_calls
                    .iter()
                    .map(|call| {
                        call.id.chars().count()
                            + call.name.chars().count()
                            + serde_json::to_string(&call.arguments)
                                .expect("serde_json::Value 总能序列化")
                                .chars()
                                .count()
                    })
                    .sum::<usize>(),
            ),
        ModelRequestMessage::ToolResult {
            tool_call_id,
            content,
        } => tool_call_id.chars().count() + content.chars().count(),
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    pub model_adapters: Vec<String>,
    /// 已注册的联网研究引擎；MVP 为空数组。
    pub research_engines: Vec<engine::EngineDescriptor>,
    /// 不能使用联网研究时的原因，有注册项时为 `None`。
    pub research_unavailable_reason: Option<String>,
    /// 已注册的知识积累引擎；MVP 为空数组。
    pub knowledge_accumulation_engines: Vec<engine::EngineDescriptor>,
    /// 不能使用知识积累时的原因，有注册项时为 `None`。
    pub knowledge_accumulation_unavailable_reason: Option<String>,
    pub builtin_prompt_count: usize,
}

/// 描述当前进程实际拥有的能力。
///
/// 知识积累引擎由调用方给出：应用把它作为应用状态的一部分持有，能力查询与积累
/// 入口因此读的是同一份注册表（细则 §7.4）。MVP 的生产注册表为空。
#[must_use]
pub fn capabilities(accumulation_engines: &[&dyn engine::AccumulationEngine]) -> AgentCapabilities {
    AgentCapabilities {
        model_adapters: vec!["chat_completions".to_owned()],
        research_engines: engine::RESEARCH_ENGINES
            .iter()
            .map(|engine| engine.descriptor())
            .collect(),
        research_unavailable_reason: engine::RESEARCH_ENGINES
            .is_empty()
            .then(|| engine::RESEARCH_UNAVAILABLE_REASON.to_owned()),
        knowledge_accumulation_engines: accumulation_engines
            .iter()
            .map(|engine| engine.descriptor())
            .collect(),
        knowledge_accumulation_unavailable_reason: accumulation_engines
            .is_empty()
            .then(|| engine::ACCUMULATION_UNAVAILABLE_REASON.to_owned()),
        builtin_prompt_count: PromptKey::ALL.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use uuid::Uuid;

    fn folded(content: &str, revision_ids: Vec<RevisionId>) -> ContextFold {
        ContextFold {
            artifact_revision_ids: revision_ids,
            through_message_sequence: 3,
            plain_text: content.to_owned(),
            content_hash: fold_content_hash(content),
        }
    }

    #[test]
    fn every_builtin_prompt_has_both_nonempty_parts() {
        for key in PromptKey::ALL {
            let prompt = builtin_prompt(key);
            assert_eq!(prompt.key, key);
            assert!(!prompt.system.trim().is_empty(), "{}", key.as_str());
            assert!(!prompt.user.trim().is_empty(), "{}", key.as_str());
        }
    }

    #[test]
    fn capabilities_advertise_the_built_in_chat_completions_protocol() {
        assert_eq!(
            capabilities(engine::ACCUMULATION_ENGINES).model_adapters,
            vec!["chat_completions"]
        );
    }

    #[test]
    fn context_builder_replaces_completed_run_history_with_stable_plaintext_folds() {
        let preparation_run_id = Uuid::new_v4();
        let first_body_run_id = Uuid::new_v4();
        let current_outline_run_id = Uuid::new_v4();
        let current_writing_run_id = Uuid::new_v4();
        let preparation_revision_id = Uuid::new_v4();
        let first_body_revision_id = Uuid::new_v4();
        let outline_revision_id = Uuid::new_v4();
        let built = ContextBuilder::build(ContextBuildRequest {
            system_instructions: "你是严格的小说写作助手。".to_owned(),
            tools: vec![
                AgentToolDefinition {
                    name: "replace_content".to_owned(),
                    description: "替换唯一正文片段".to_owned(),
                    parameters: json!({"type": "object"}),
                },
                AgentToolDefinition {
                    name: "append_content".to_owned(),
                    description: "追加正文".to_owned(),
                    parameters: json!({"type": "object"}),
                },
            ],
            completed_stages: vec![
                CompletedStageContext {
                    stage: ContextFoldStage::ChapterOutline,
                    chapter_order: Some(2),
                    source_run_id: current_outline_run_id,
                    fold: folded("第二章细纲全文", vec![outline_revision_id]),
                },
                CompletedStageContext {
                    stage: ContextFoldStage::ChapterBody,
                    chapter_order: Some(1),
                    source_run_id: first_body_run_id,
                    fold: folded("第一章终稿全文", vec![first_body_revision_id]),
                },
                CompletedStageContext {
                    stage: ContextFoldStage::Preparation,
                    chapter_order: None,
                    source_run_id: preparation_run_id,
                    fold: folded("准备阶段四项完整产物", vec![preparation_revision_id]),
                },
            ],
            current_inputs: vec![
                CurrentContextInput {
                    role: ContextMessageRole::Assistant,
                    content: "第二章细纲全文".to_owned(),
                    source_revision_id: Some(outline_revision_id),
                },
                CurrentContextInput {
                    role: ContextMessageRole::User,
                    content: "本章请加强林舟的犹豫。".to_owned(),
                    source_revision_id: None,
                },
            ],
            current_messages: vec![UnfoldedStageMessage {
                source_run_id: current_writing_run_id,
                sequence: 4,
                message: ModelRequestMessage::Text {
                    role: ContextMessageRole::User,
                    content: "继续从仓库门口写起。".to_owned(),
                },
            }],
            budget: ContextBudget {
                max_context_tokens: 10_000,
                reserved_output_tokens: 1_000,
            },
        })
        .unwrap();

        assert_eq!(
            built
                .messages
                .iter()
                .map(|message| match message {
                    ModelRequestMessage::Text { content, .. } => content.as_str(),
                    ModelRequestMessage::AssistantToolCalls { .. }
                    | ModelRequestMessage::ToolResult { .. } => {
                        panic!("this context only contains text messages")
                    }
                })
                .collect::<Vec<_>>(),
            vec![
                "你是严格的小说写作助手。",
                "准备阶段四项完整产物",
                "第一章终稿全文",
                "第二章细纲全文",
                "本章请加强林舟的犹豫。",
                "继续从仓库门口写起。",
            ]
        );
        assert_eq!(
            built
                .tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            vec!["append_content", "replace_content"]
        );
        assert_eq!(
            built
                .fold_sources
                .iter()
                .map(|source| source.stage)
                .collect::<Vec<_>>(),
            vec![
                ContextFoldStage::Preparation,
                ContextFoldStage::ChapterBody,
                ContextFoldStage::ChapterOutline,
            ]
        );
        assert_eq!(built.omitted_input_revision_ids, vec![outline_revision_id]);
        assert!(built.uses_estimated_token_count);
    }

    #[test]
    fn context_builder_orders_outline_before_body_within_one_chapter() {
        let built = ContextBuilder::build(ContextBuildRequest {
            system_instructions: "系统约束".to_owned(),
            tools: Vec::new(),
            completed_stages: vec![
                CompletedStageContext {
                    stage: ContextFoldStage::ChapterBody,
                    chapter_order: Some(3),
                    source_run_id: Uuid::new_v4(),
                    fold: folded("第三章终稿", vec![Uuid::new_v4()]),
                },
                CompletedStageContext {
                    stage: ContextFoldStage::ChapterOutline,
                    chapter_order: Some(3),
                    source_run_id: Uuid::new_v4(),
                    fold: folded("第三章细纲", vec![Uuid::new_v4()]),
                },
            ],
            current_inputs: Vec::new(),
            current_messages: Vec::new(),
            budget: ContextBudget {
                max_context_tokens: 10_000,
                reserved_output_tokens: 1_000,
            },
        })
        .unwrap();

        assert_eq!(
            built
                .messages
                .iter()
                .filter_map(|message| match message {
                    ModelRequestMessage::Text { content, .. } => Some(content.as_str()),
                    ModelRequestMessage::AssistantToolCalls { .. }
                    | ModelRequestMessage::ToolResult { .. } => None,
                })
                .collect::<Vec<_>>(),
            vec!["系统约束", "第三章细纲", "第三章终稿"]
        );
    }

    #[test]
    fn context_builder_rejects_folded_history_and_never_truncates_for_budget() {
        let folded_run_id = Uuid::new_v4();
        let folded_revision_id = Uuid::new_v4();
        let folded_stage = CompletedStageContext {
            stage: ContextFoldStage::Preparation,
            chapter_order: None,
            source_run_id: folded_run_id,
            fold: folded("审核通过的准备产物", vec![folded_revision_id]),
        };
        let folded_history_error = ContextBuilder::build(ContextBuildRequest {
            system_instructions: "系统约束".to_owned(),
            tools: Vec::new(),
            completed_stages: vec![folded_stage],
            current_inputs: Vec::new(),
            current_messages: vec![UnfoldedStageMessage {
                source_run_id: folded_run_id,
                sequence: 4,
                message: ModelRequestMessage::Text {
                    role: ContextMessageRole::Assistant,
                    content: "不应回流的工具和审核过程".to_owned(),
                },
            }],
            budget: ContextBudget {
                max_context_tokens: 10_000,
                reserved_output_tokens: 1,
            },
        })
        .unwrap_err();
        assert_eq!(
            folded_history_error,
            ContextBuildError::FoldedRunMessage {
                run_id: folded_run_id
            }
        );

        let budget_error = ContextBuilder::build(ContextBuildRequest {
            system_instructions: "系统".to_owned(),
            tools: Vec::new(),
            completed_stages: Vec::new(),
            current_inputs: vec![CurrentContextInput {
                role: ContextMessageRole::User,
                content: "正文输入".to_owned(),
                source_revision_id: None,
            }],
            current_messages: Vec::new(),
            budget: ContextBudget {
                max_context_tokens: 4,
                reserved_output_tokens: 1,
            },
        })
        .unwrap_err();
        assert!(matches!(
            budget_error,
            ContextBuildError::ContextLimitExceeded { .. }
        ));
    }

    #[test]
    fn context_builder_keeps_tool_call_pairs_and_rejects_unpaired_history() {
        let run_id = Uuid::new_v4();
        let tool_call = provider::ModelToolCall {
            id: "call-1".to_owned(),
            name: "append_content".to_owned(),
            arguments: json!({ "content": "正文" }),
        };
        let request = ContextBuildRequest {
            system_instructions: "系统约束".to_owned(),
            tools: Vec::new(),
            completed_stages: Vec::new(),
            current_inputs: Vec::new(),
            current_messages: vec![
                UnfoldedStageMessage {
                    source_run_id: run_id,
                    sequence: 1,
                    message: ModelRequestMessage::AssistantToolCalls {
                        content: None,
                        tool_calls: vec![tool_call.clone()],
                    },
                },
                UnfoldedStageMessage {
                    source_run_id: run_id,
                    sequence: 2,
                    message: ModelRequestMessage::ToolResult {
                        tool_call_id: tool_call.id.clone(),
                        content: "{\"status\":\"applied\"}".to_owned(),
                    },
                },
            ],
            budget: ContextBudget {
                max_context_tokens: 10_000,
                reserved_output_tokens: 1,
            },
        };
        let built = ContextBuilder::build(request.clone()).unwrap();
        assert!(matches!(
            &built.messages[1],
            ModelRequestMessage::AssistantToolCalls { tool_calls, .. }
                if tool_calls == &vec![tool_call.clone()]
        ));
        assert!(matches!(
            &built.messages[2],
            ModelRequestMessage::ToolResult { tool_call_id, .. } if tool_call_id == "call-1"
        ));

        let mut unpaired = request;
        unpaired.current_messages.pop();
        assert!(matches!(
            ContextBuilder::build(unpaired),
            Err(ContextBuildError::InvalidMessageProtocol { .. })
        ));
    }
}
