//! Chat Completions 兼容模型协议。
//!
//! 这个模块只实现一份经过验证的 HTTP/SSE 合同：它不猜测供应商路径、不重试
//! 已经发出的模型请求，也不把不完整流、并行工具调用或无效工具参数交给业务
//! 层。调用方在自己的 Tokio 任务中执行 [`ModelProvider::execute`]，便可在每个
//! 已完整解析的增量到达时更新临时预览，而正式持久化仍由 App 服务负责。

use std::{
    collections::{BTreeMap, HashSet},
    fmt,
    time::Duration,
};

use futures_util::{StreamExt, future::BoxFuture};
use nekowriter_core::{
    ModelCapabilityProfile, PersistedModelUsage, ThinkingLevel, UsageUnknownReason, decimal_i64,
};
use reqwest::{Client, header};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::time::timeout;
use url::Url;

use super::{ContextMessage, ContextMessageRole};

/// 单个工具调用的完整参数最大字节数。
pub const MAX_TOOL_ARGUMENT_BYTES: usize = 256 * 1024;
/// 单次模型流在解码前可接收的总字节数。
pub const MAX_MODEL_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

/// 模型网络调用的三个独立超时。
///
/// 没有总时限：正常长输出只要持续返回 SSE 数据即可继续。连接、首个正文分块和
/// 后续流空闲则分别受限。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModelTimeouts {
    pub connect: Duration,
    pub first_byte: Duration,
    pub idle: Duration,
}

impl Default for ModelTimeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(10),
            first_byte: Duration::from_secs(45),
            idle: Duration::from_secs(90),
        }
    }
}

/// 一个已经从本地凭据存储解析出的 Chat Completions 配置。
///
/// `api_key` 仅存在于进程内，既不实现 `Serialize`，也不会出现在 `Debug` 输出中。
/// 设置表只应保存其凭据引用。
#[derive(Clone)]
pub struct ChatCompletionsConfig {
    api_base_url: Url,
    model: String,
    api_key: Option<String>,
    capabilities: ModelCapabilityProfile,
    timeouts: ModelTimeouts,
}

impl fmt::Debug for ChatCompletionsConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChatCompletionsConfig")
            .field("api_base_url", &self.api_base_url)
            .field("model", &self.model)
            .field("api_key_configured", &self.api_key.is_some())
            .field("capabilities", &self.capabilities)
            .field("timeouts", &self.timeouts)
            .finish()
    }
}

impl ChatCompletionsConfig {
    /// 创建并验证显式的 API 根地址。
    ///
    /// 地址必须含有实际 API 前缀，例如 `https://host.example/v1`；不能直接填写
    /// `chat/completions` 完整端点，以免调用方误以为适配器会猜测不同服务的路径。
    ///
    /// # Errors
    ///
    /// API 根地址、模型 ID、凭据、能力组合或超时不满足协议合同会返回错误。
    pub fn new(
        api_base_url: impl AsRef<str>,
        model: impl Into<String>,
        api_key: Option<String>,
        capabilities: ModelCapabilityProfile,
        timeouts: ModelTimeouts,
    ) -> Result<Self, ProviderError> {
        let api_base_url = normalize_api_base_url(api_base_url.as_ref())?;
        let model = model.into();
        if model.trim().is_empty() {
            return Err(ProviderError::EmptyModelId);
        }
        if api_key.as_ref().is_some_and(|key| key.trim().is_empty()) {
            return Err(ProviderError::EmptyApiKey);
        }
        if !capabilities.streaming {
            return Err(ProviderError::StreamingCapabilityRequired);
        }
        validate_timeouts(timeouts)?;
        Ok(Self {
            api_base_url,
            model,
            api_key,
            capabilities,
            timeouts,
        })
    }

    #[must_use]
    pub fn api_base_url(&self) -> &Url {
        &self.api_base_url
    }

    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    #[must_use]
    pub const fn capabilities(&self) -> ModelCapabilityProfile {
        self.capabilities
    }
}

/// 给模型的一条消息。工具调用与工具结果使用单独变体，以保留 Chat Completions
/// 所要求的 assistant/tool 配对关系。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelRequestMessage {
    Text {
        role: ContextMessageRole,
        content: String,
    },
    AssistantToolCalls {
        content: Option<String>,
        tool_calls: Vec<ModelToolCall>,
    },
    ToolResult {
        tool_call_id: String,
        content: String,
    },
}

impl From<ContextMessage> for ModelRequestMessage {
    fn from(message: ContextMessage) -> Self {
        Self::Text {
            role: message.role,
            content: message.content,
        }
    }
}

/// 一次已完整解析的 function tool call。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// 希望模型将最终 assistant 文本限制为的结构化格式。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StructuredOutput {
    JsonObject,
    JsonSchema { name: String, schema: Value },
}

/// 即将提交给模型适配器的已封存请求。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRequest {
    pub messages: Vec<ModelRequestMessage>,
    pub tools: Vec<super::AgentToolDefinition>,
    pub response_format: Option<StructuredOutput>,
    /// 本次请求要求模型使用的思考等级。由项目设置解析而来，因此每次请求都会
    /// 按当时的项目设置重新构建。
    #[serde(default)]
    pub thinking_level: ThinkingLevel,
}

impl ModelRequest {
    #[must_use]
    pub fn from_context(
        messages: Vec<ContextMessage>,
        tools: Vec<super::AgentToolDefinition>,
    ) -> Self {
        Self {
            messages: messages.into_iter().map(Into::into).collect(),
            tools,
            response_format: None,
            thinking_level: ThinkingLevel::Off,
        }
    }
}

/// 已映射的模型用量。跨 crate/HTTP 边界时以十进制字符串编码，以免 JavaScript
/// 丢失长时间运行任务的累计值精度。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    #[serde(with = "decimal_i64")]
    pub input_tokens: i64,
    #[serde(with = "decimal_i64")]
    pub output_tokens: i64,
    #[serde(with = "decimal_i64")]
    pub total_tokens: i64,
    #[serde(with = "optional_decimal_i64")]
    pub cached_input_tokens: Option<i64>,
    /// 供应商报告为输出子集的思考用量；未报告时为 `None`，不参与总量相加。
    #[serde(default, with = "optional_decimal_i64")]
    pub reasoning_tokens: Option<i64>,
}

/// 供应商流中被消费的一个可见事件。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelEvent {
    TextDelta {
        content: String,
    },
    ToolCallDelta {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments_delta: String,
    },
    Usage {
        usage: ModelUsage,
    },
    Completed {
        finish_reason: ModelFinishReason,
    },
    Failed {
        code: ModelFailureCode,
        message: String,
    },
}

/// 完整响应被允许的终止类型。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelFinishReason {
    Stop,
    ToolCalls,
}

/// 发送给运行器的失败分类。细节仍保留在 [`ProviderError`]，该稳定分类用于 UI
/// 提示和 Run 检查点，不包含凭据或原始敏感请求内容。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelFailureCode {
    CapabilityMismatch,
    Connection,
    Timeout,
    ProviderResponse,
    Protocol,
}

/// 一个完整且经过协议校验的模型响应。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCompletion {
    pub provider_request_id: Option<String>,
    pub content: Option<String>,
    pub tool_calls: Vec<ModelToolCall>,
    pub usage: Option<ModelUsage>,
    /// 供应商报告的原始用量对象（未映射的字段名），随 `usage` 一起存在。
    pub usage_raw: Option<serde_json::Value>,
    pub finish_reason: ModelFinishReason,
}

impl ModelCompletion {
    /// 把一次完整响应的用量封存成持久口径。
    ///
    /// 供应商没有报告时写入「供应商未报告」而不是空对象，Dashboard 因此能区分
    /// 「报告了零用量」与「我们不知道」，并保留原始字段供事后核对。
    #[must_use]
    pub fn persisted_usage(&self) -> PersistedModelUsage {
        match &self.usage {
            Some(usage) => PersistedModelUsage::from_provider_stream(
                usage.input_tokens,
                usage.output_tokens,
                usage.total_tokens,
                usage.cached_input_tokens,
                usage.reasoning_tokens,
                self.usage_raw.clone(),
            ),
            None => PersistedModelUsage::unknown(UsageUnknownReason::ProviderSilent),
        }
    }
}

/// 可被应用服务替换的模型协议边界。
///
/// `emit` 在同一请求任务中按 SSE 顺序调用；它只能处理临时预览和内存状态，不能
/// 直接绕过 App 服务写入正文。返回的 [`ModelCompletion`] 才能进入工具或结构化
/// 阶段提交的服务端校验。
pub trait ModelProvider: Send + Sync {
    fn capabilities(&self) -> ModelCapabilityProfile;

    fn execute<'a>(
        &'a self,
        request: &'a ModelRequest,
        emit: &'a mut (dyn FnMut(ModelEvent) + Send),
    ) -> BoxFuture<'a, Result<ModelCompletion, ProviderError>>;
}

/// Chat Completions SSE 协议的实际 HTTP 适配器。
#[derive(Clone)]
pub struct ChatCompletionsAdapter {
    config: ChatCompletionsConfig,
    endpoint: Url,
    client: Client,
}

impl fmt::Debug for ChatCompletionsAdapter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChatCompletionsAdapter")
            .field("config", &self.config)
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl ChatCompletionsAdapter {
    /// # Errors
    ///
    /// 返回配置、URL 或 HTTP 客户端初始化错误；不会发出网络请求。
    pub fn new(config: ChatCompletionsConfig) -> Result<Self, ProviderError> {
        let endpoint = config
            .api_base_url
            .join("chat/completions")
            .map_err(|error| ProviderError::InvalidApiBaseUrl {
                reason: error.to_string(),
            })?;
        let client = Client::builder()
            .connect_timeout(config.timeouts.connect)
            .build()
            .map_err(|error| ProviderError::ClientBuild {
                reason: error.to_string(),
            })?;
        Ok(Self {
            config,
            endpoint,
            client,
        })
    }

    #[must_use]
    pub fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    /// 执行请求并丢弃临时事件，适合连接测试或非交互式调用。
    ///
    /// # Errors
    ///
    /// 网络、超时、能力或模型协议不满足合同都会返回错误。
    pub async fn complete(&self, request: &ModelRequest) -> Result<ModelCompletion, ProviderError> {
        let mut discard = |_| {};
        <Self as ModelProvider>::execute(self, request, &mut discard).await
    }

    /// 执行请求并同步报告解析完成的 SSE 事件。
    ///
    /// 这是一个真实网络调用；它刻意不在传输失败后自动重试，避免不确定状态的
    /// 模型请求被重复计费或重复执行工具。
    ///
    /// # Errors
    ///
    /// 网络、超时、能力或模型协议不满足合同都会返回错误。
    pub async fn execute_with<F>(
        &self,
        request: &ModelRequest,
        mut emit: F,
    ) -> Result<ModelCompletion, ProviderError>
    where
        F: FnMut(ModelEvent) + Send,
    {
        <Self as ModelProvider>::execute(self, request, &mut emit).await
    }

    async fn execute_inner(
        &self,
        request: &ModelRequest,
        emit: &mut (dyn FnMut(ModelEvent) + Send),
    ) -> Result<ModelCompletion, ProviderError> {
        let payload = self.request_payload(request)?;
        let mut builder = self
            .client
            .post(self.endpoint.clone())
            .header(header::ACCEPT, "text/event-stream")
            .header(header::CONTENT_TYPE, "application/json")
            .json(&payload);
        if let Some(api_key) = &self.config.api_key {
            builder = builder.bearer_auth(api_key);
        }

        let response = match timeout(self.config.timeouts.first_byte, builder.send()).await {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => return Err(provider_transport_error(&error)),
            Err(_) => {
                return Err(ProviderError::Timeout {
                    phase: TimeoutPhase::FirstByte,
                });
            }
        };
        if !response.status().is_success() {
            return Err(ProviderError::HttpStatus {
                status: response.status().as_u16(),
            });
        }
        validate_sse_content_type(&response)?;
        let provider_request_id = provider_request_id(&response);

        let mut response_stream = response.bytes_stream().boxed();
        let mut decoder = SseDecoder::default();
        let mut accumulator = StreamAccumulator::default();
        let mut received_bytes = 0_usize;

        let first_chunk =
            match timeout(self.config.timeouts.first_byte, response_stream.next()).await {
                Ok(Some(Ok(chunk))) => chunk,
                Ok(Some(Err(error))) => return Err(provider_transport_error(&error)),
                Ok(None) => return Err(ProviderError::StreamEndedBeforeFirstEvent),
                Err(_) => {
                    return Err(ProviderError::Timeout {
                        phase: TimeoutPhase::FirstByte,
                    });
                }
            };
        consume_stream_chunk(
            &first_chunk,
            &mut received_bytes,
            &mut decoder,
            &mut accumulator,
            emit,
        )?;

        loop {
            let next_chunk = match timeout(self.config.timeouts.idle, response_stream.next()).await
            {
                Ok(Some(Ok(chunk))) => chunk,
                Ok(Some(Err(error))) => return Err(provider_transport_error(&error)),
                Ok(None) => break,
                Err(_) => {
                    return Err(ProviderError::Timeout {
                        phase: TimeoutPhase::Idle,
                    });
                }
            };
            consume_stream_chunk(
                &next_chunk,
                &mut received_bytes,
                &mut decoder,
                &mut accumulator,
                emit,
            )?;
        }

        for data in decoder.finish()? {
            accumulator.consume_data(&data, emit)?;
        }
        let completion = accumulator.finish(provider_request_id)?;
        emit(ModelEvent::Completed {
            finish_reason: completion.finish_reason,
        });
        Ok(completion)
    }

    fn request_payload(&self, request: &ModelRequest) -> Result<Value, ProviderError> {
        if request.messages.is_empty() {
            return Err(ProviderError::EmptyRequestMessages);
        }
        let messages = serialize_request_messages(&request.messages)?;
        let tools = serialize_tools(&request.tools)?;
        if !tools.is_empty() && !self.config.capabilities.tools {
            return Err(ProviderError::CapabilityMismatch {
                capability: "tools",
            });
        }
        if request.response_format.is_some() && !self.config.capabilities.structured_output {
            return Err(ProviderError::CapabilityMismatch {
                capability: "structured_output",
            });
        }
        if request.thinking_level != ThinkingLevel::Off && !self.config.capabilities.thinking {
            return Err(ProviderError::CapabilityMismatch {
                capability: "thinking",
            });
        }

        let mut payload = json!({
            "model": self.config.model,
            "messages": messages,
            "stream": true,
        });
        let object = payload
            .as_object_mut()
            .expect("JSON object literal must be an object");
        if !tools.is_empty() {
            object.insert("tools".to_owned(), Value::Array(tools));
            object.insert("tool_choice".to_owned(), Value::String("auto".to_owned()));
            object.insert("parallel_tool_calls".to_owned(), Value::Bool(false));
        }
        if self.config.capabilities.stream_usage {
            object.insert(
                "stream_options".to_owned(),
                json!({ "include_usage": true }),
            );
        }
        if let Some(response_format) = &request.response_format {
            object.insert(
                "response_format".to_owned(),
                serialize_response_format(response_format)?,
            );
        }
        if request.thinking_level != ThinkingLevel::Off {
            object.insert(
                "reasoning_effort".to_owned(),
                Value::String(request.thinking_level.as_str().to_owned()),
            );
        }
        Ok(payload)
    }
}

impl ModelProvider for ChatCompletionsAdapter {
    fn capabilities(&self) -> ModelCapabilityProfile {
        self.config.capabilities
    }

    fn execute<'a>(
        &'a self,
        request: &'a ModelRequest,
        emit: &'a mut (dyn FnMut(ModelEvent) + Send),
    ) -> BoxFuture<'a, Result<ModelCompletion, ProviderError>> {
        Box::pin(async move {
            let result = self.execute_inner(request, emit).await;
            if let Err(error) = &result {
                emit(ModelEvent::Failed {
                    code: error.failure_code(),
                    message: error.to_string(),
                });
            }
            result
        })
    }
}

/// 模型协议、配置或网络合同错误。
#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("模型 API 根地址无效：{reason}")]
    InvalidApiBaseUrl { reason: String },
    #[error("模型 API 根地址必须包含实际 API 前缀，例如 /v1")]
    MissingApiPrefix,
    #[error("模型 API 根地址必须是根地址，不能包含 chat/completions")]
    EndpointInsteadOfApiBase,
    #[error("模型 ID 不能为空")]
    EmptyModelId,
    #[error("已配置的模型 API 密钥不能为空")]
    EmptyApiKey,
    #[error("流式模型能力是 Chat Completions 适配器的必要条件")]
    StreamingCapabilityRequired,
    #[error("模型超时必须全部大于零")]
    InvalidTimeouts,
    #[error("无法创建模型 HTTP 客户端：{reason}")]
    ClientBuild { reason: String },
    #[error("模型不支持当前请求所需能力：{capability}")]
    CapabilityMismatch { capability: &'static str },
    #[error("模型请求至少需要一条消息")]
    EmptyRequestMessages,
    #[error("普通文本消息不能使用 tool role；工具结果必须携带 tool_call_id")]
    TextToolRole,
    #[error("assistant 工具调用必须至少包含一个调用")]
    EmptyAssistantToolCalls,
    #[error("工具调用 ID 不能为空")]
    EmptyToolCallId,
    #[error("工具调用 {id} 的名称不能为空")]
    EmptyToolCallName { id: String },
    #[error("工具调用 {id} 的参数必须是 JSON object")]
    InvalidToolCallArguments { id: String },
    #[error("工具调用 {id} 的参数超过 {MAX_TOOL_ARGUMENT_BYTES} 字节上限")]
    ToolCallArgumentsTooLarge { id: String },
    #[error("工具调用 ID {id} 在消息序列中重复")]
    DuplicateToolCallId { id: String },
    #[error("工具结果引用了不存在或已完成的工具调用 {id}")]
    UnexpectedToolResult { id: String },
    #[error("工具调用 {id} 缺少紧随其后的工具结果")]
    MissingToolResult { id: String },
    #[error("工具名称不能为空")]
    EmptyToolName,
    #[error("工具名称 {name} 重复")]
    DuplicateToolName { name: String },
    #[error("工具 {name} 的参数 schema 必须是 JSON object")]
    InvalidToolSchema { name: String },
    #[error("JSON Schema 输出名称不能为空")]
    EmptyJsonSchemaName,
    #[error("JSON Schema 输出 schema 必须是 JSON object")]
    InvalidJsonSchema,
    #[error("模型服务连接失败：{reason}")]
    Connection { reason: String },
    #[error("模型服务请求失败：{reason}")]
    Transport { reason: String },
    #[error("模型服务在{phase}阶段超时")]
    Timeout { phase: TimeoutPhase },
    #[error("模型服务返回 HTTP {status}")]
    HttpStatus { status: u16 },
    #[error("模型服务没有返回 text/event-stream，实际为 {actual:?}")]
    UnexpectedContentType { actual: Option<String> },
    #[error("模型流在第一个事件前结束")]
    StreamEndedBeforeFirstEvent,
    #[error("模型响应超过 {MAX_MODEL_RESPONSE_BYTES} 字节上限")]
    ResponseTooLarge,
    #[error("模型 SSE 流不是有效 UTF-8")]
    InvalidSseEncoding,
    #[error("模型 SSE 数据不是有效 JSON：{reason}")]
    InvalidSseJson { reason: String },
    #[error("模型流在 [DONE] 后继续返回数据")]
    DataAfterDone,
    #[error("模型流结束时没有 [DONE] 标志")]
    MissingDoneMarker,
    #[error("模型流没有有效 finish_reason")]
    MissingFinishReason,
    #[error("模型返回了不受支持的 finish_reason {reason}")]
    UnsupportedFinishReason { reason: String },
    #[error("模型返回了非零 choice index {index}")]
    UnexpectedChoiceIndex { index: usize },
    #[error("模型返回了多个 choice")]
    MultipleChoices,
    #[error("模型拒绝了请求：{reason}")]
    ModelRefusal { reason: String },
    #[error("同一响应的 finish_reason 相互冲突")]
    ConflictingFinishReason,
    #[error("工具调用 {index} 的 type 不是 function")]
    UnsupportedToolCallType { index: usize },
    #[error("工具调用 {index} 的 ID 在流中相互冲突")]
    ConflictingToolCallId { index: usize },
    #[error("工具调用 {index} 的名称在流中相互冲突")]
    ConflictingToolCallName { index: usize },
    #[error("模型同时返回正文文本和工具调用")]
    TextAndToolCalls,
    #[error("tool_calls 终止响应没有包含任何工具调用")]
    ToolCallsWithoutCalls,
    #[error("stop 响应不能包含工具调用")]
    ToolCallsWithStop,
    #[error("工具调用 {index} 缺少 ID")]
    MissingStreamToolCallId { index: usize },
    #[error("工具调用 {index} 缺少名称")]
    MissingStreamToolCallName { index: usize },
    #[error("工具调用 {index} 缺少 function type")]
    MissingStreamToolCallType { index: usize },
    #[error("工具调用 {index} 的参数超过 {MAX_TOOL_ARGUMENT_BYTES} 字节上限")]
    ToolArgumentsTooLarge { index: usize },
    #[error("工具调用 {index} 的参数不是有效 JSON：{reason}")]
    InvalidStreamToolArguments { index: usize, reason: String },
    #[error("模型 usage 中的 {field} 不能为负数")]
    NegativeUsage { field: &'static str },
}

impl ProviderError {
    #[must_use]
    pub const fn failure_code(&self) -> ModelFailureCode {
        match self {
            Self::CapabilityMismatch { .. } | Self::StreamingCapabilityRequired => {
                ModelFailureCode::CapabilityMismatch
            }
            Self::Connection { .. } => ModelFailureCode::Connection,
            Self::Timeout { .. } => ModelFailureCode::Timeout,
            Self::HttpStatus { .. }
            | Self::Transport { .. }
            | Self::UnexpectedContentType { .. }
            | Self::ClientBuild { .. } => ModelFailureCode::ProviderResponse,
            _ => ModelFailureCode::Protocol,
        }
    }
}

/// 哪一个网络阶段超时。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeoutPhase {
    FirstByte,
    Idle,
}

impl fmt::Display for TimeoutPhase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FirstByte => formatter.write_str("首包"),
            Self::Idle => formatter.write_str("流空闲"),
        }
    }
}

pub(crate) fn normalize_api_base_url(raw: &str) -> Result<Url, ProviderError> {
    let mut url = Url::parse(raw).map_err(|error| ProviderError::InvalidApiBaseUrl {
        reason: error.to_string(),
    })?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(ProviderError::InvalidApiBaseUrl {
            reason: "仅支持包含主机的 http 或 https 地址".to_owned(),
        });
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(ProviderError::InvalidApiBaseUrl {
            reason: "地址中不能携带用户名或密码".to_owned(),
        });
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(ProviderError::InvalidApiBaseUrl {
            reason: "地址中不能携带 query 或 fragment".to_owned(),
        });
    }

    let path = url.path().trim_end_matches('/');
    if path.is_empty() {
        return Err(ProviderError::MissingApiPrefix);
    }
    if path.ends_with("/chat/completions") {
        return Err(ProviderError::EndpointInsteadOfApiBase);
    }
    url.set_path(&format!("{path}/"));
    Ok(url)
}

fn validate_timeouts(timeouts: ModelTimeouts) -> Result<(), ProviderError> {
    if timeouts.connect.is_zero() || timeouts.first_byte.is_zero() || timeouts.idle.is_zero() {
        return Err(ProviderError::InvalidTimeouts);
    }
    Ok(())
}

fn serialize_request_messages(
    messages: &[ModelRequestMessage],
) -> Result<Vec<Value>, ProviderError> {
    let mut serialized = Vec::with_capacity(messages.len());
    let mut pending_tool_call_ids = HashSet::<String>::new();
    let mut all_tool_call_ids = HashSet::<String>::new();

    for message in messages {
        if !pending_tool_call_ids.is_empty()
            && !matches!(message, ModelRequestMessage::ToolResult { .. })
        {
            let id = pending_tool_call_ids
                .iter()
                .next()
                .expect("checked nonempty")
                .clone();
            return Err(ProviderError::MissingToolResult { id });
        }
        match message {
            ModelRequestMessage::Text { role, content } => {
                if *role == ContextMessageRole::Tool {
                    return Err(ProviderError::TextToolRole);
                }
                serialized.push(json!({
                    "role": context_role_name(*role),
                    "content": content,
                }));
            }
            ModelRequestMessage::AssistantToolCalls {
                content,
                tool_calls,
            } => {
                if tool_calls.is_empty() {
                    return Err(ProviderError::EmptyAssistantToolCalls);
                }
                let mut wire_calls = Vec::with_capacity(tool_calls.len());
                for call in tool_calls {
                    validate_complete_tool_call(call)?;
                    if !all_tool_call_ids.insert(call.id.clone()) {
                        return Err(ProviderError::DuplicateToolCallId {
                            id: call.id.clone(),
                        });
                    }
                    pending_tool_call_ids.insert(call.id.clone());
                    wire_calls.push(json!({
                        "id": call.id,
                        "type": "function",
                        "function": {
                            "name": call.name,
                            "arguments": call.arguments.to_string(),
                        },
                    }));
                }
                serialized.push(json!({
                    "role": "assistant",
                    "content": content,
                    "tool_calls": wire_calls,
                }));
            }
            ModelRequestMessage::ToolResult {
                tool_call_id,
                content,
            } => {
                if !pending_tool_call_ids.remove(tool_call_id) {
                    return Err(ProviderError::UnexpectedToolResult {
                        id: tool_call_id.clone(),
                    });
                }
                serialized.push(json!({
                    "role": "tool",
                    "tool_call_id": tool_call_id,
                    "content": content,
                }));
            }
        }
    }
    if let Some(id) = pending_tool_call_ids.into_iter().next() {
        return Err(ProviderError::MissingToolResult { id });
    }
    Ok(serialized)
}

/// 验证一组已封存消息是否仍保留 Chat Completions 所需的工具调用配对。
///
/// 供同 crate 的上下文构建器在请求封存前使用。调用者不取得 wire JSON，因此不会
/// 通过这个辅助函数绕过真实适配器对消息、工具或能力的完整校验。
pub(crate) fn validate_request_message_sequence(
    messages: &[ModelRequestMessage],
) -> Result<(), ProviderError> {
    serialize_request_messages(messages).map(|_| ())
}

fn context_role_name(role: ContextMessageRole) -> &'static str {
    match role {
        ContextMessageRole::System => "system",
        ContextMessageRole::User => "user",
        ContextMessageRole::Assistant => "assistant",
        ContextMessageRole::Tool => unreachable!("tool text roles are rejected first"),
    }
}

fn validate_complete_tool_call(call: &ModelToolCall) -> Result<(), ProviderError> {
    if call.id.trim().is_empty() {
        return Err(ProviderError::EmptyToolCallId);
    }
    if call.name.trim().is_empty() {
        return Err(ProviderError::EmptyToolCallName {
            id: call.id.clone(),
        });
    }
    if !call.arguments.is_object() {
        return Err(ProviderError::InvalidToolCallArguments {
            id: call.id.clone(),
        });
    }
    if call.arguments.to_string().len() > MAX_TOOL_ARGUMENT_BYTES {
        return Err(ProviderError::ToolCallArgumentsTooLarge {
            id: call.id.clone(),
        });
    }
    Ok(())
}

fn serialize_tools(tools: &[super::AgentToolDefinition]) -> Result<Vec<Value>, ProviderError> {
    let mut sorted_tools = tools.to_vec();
    sorted_tools.sort_by(|left, right| left.name.cmp(&right.name));
    let mut previous_name: Option<&str> = None;
    let mut serialized = Vec::with_capacity(sorted_tools.len());
    for tool in &sorted_tools {
        if tool.name.trim().is_empty() {
            return Err(ProviderError::EmptyToolName);
        }
        if previous_name == Some(tool.name.as_str()) {
            return Err(ProviderError::DuplicateToolName {
                name: tool.name.clone(),
            });
        }
        if !tool.parameters.is_object() {
            return Err(ProviderError::InvalidToolSchema {
                name: tool.name.clone(),
            });
        }
        previous_name = Some(&tool.name);
        serialized.push(json!({
            "type": "function",
            "function": {
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
            },
        }));
    }
    Ok(serialized)
}

fn serialize_response_format(format: &StructuredOutput) -> Result<Value, ProviderError> {
    match format {
        StructuredOutput::JsonObject => Ok(json!({ "type": "json_object" })),
        StructuredOutput::JsonSchema { name, schema } => {
            if name.trim().is_empty() {
                return Err(ProviderError::EmptyJsonSchemaName);
            }
            if !schema.is_object() {
                return Err(ProviderError::InvalidJsonSchema);
            }
            Ok(json!({
                "type": "json_schema",
                "json_schema": {
                    "name": name,
                    "strict": true,
                    "schema": schema,
                },
            }))
        }
    }
}

fn validate_sse_content_type(response: &reqwest::Response) -> Result<(), ProviderError> {
    let actual = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let is_sse = actual
        .as_deref()
        .and_then(|value| value.split(';').next())
        .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/event-stream"));
    if is_sse {
        Ok(())
    } else {
        Err(ProviderError::UnexpectedContentType { actual })
    }
}

fn provider_request_id(response: &reqwest::Response) -> Option<String> {
    ["x-request-id", "request-id"]
        .into_iter()
        .find_map(|name| response.headers().get(name))
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
}

fn provider_transport_error(error: &reqwest::Error) -> ProviderError {
    let reason = error.to_string();
    if error.is_connect() {
        ProviderError::Connection { reason }
    } else {
        ProviderError::Transport { reason }
    }
}

fn consume_stream_chunk(
    chunk: &[u8],
    received_bytes: &mut usize,
    decoder: &mut SseDecoder,
    accumulator: &mut StreamAccumulator,
    emit: &mut (dyn FnMut(ModelEvent) + Send),
) -> Result<(), ProviderError> {
    *received_bytes = received_bytes
        .checked_add(chunk.len())
        .ok_or(ProviderError::ResponseTooLarge)?;
    if *received_bytes > MAX_MODEL_RESPONSE_BYTES {
        return Err(ProviderError::ResponseTooLarge);
    }
    for data in decoder.push(chunk)? {
        accumulator.consume_data(&data, emit)?;
    }
    Ok(())
}

#[derive(Default)]
struct SseDecoder {
    pending_bytes: Vec<u8>,
    data_lines: Vec<String>,
}

impl SseDecoder {
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>, ProviderError> {
        self.pending_bytes.extend_from_slice(bytes);
        let mut messages = Vec::new();
        while let Some(newline_index) = self.pending_bytes.iter().position(|byte| *byte == b'\n') {
            let mut line = self
                .pending_bytes
                .drain(..=newline_index)
                .collect::<Vec<_>>();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            self.consume_line(&line, &mut messages)?;
        }
        Ok(messages)
    }

    fn finish(mut self) -> Result<Vec<String>, ProviderError> {
        let mut messages = Vec::new();
        if !self.pending_bytes.is_empty() {
            let mut line = std::mem::take(&mut self.pending_bytes);
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            self.consume_line(&line, &mut messages)?;
        }
        if !self.data_lines.is_empty() {
            messages.push(self.data_lines.join("\n"));
        }
        Ok(messages)
    }

    fn consume_line(
        &mut self,
        line: &[u8],
        messages: &mut Vec<String>,
    ) -> Result<(), ProviderError> {
        let line = std::str::from_utf8(line).map_err(|_| ProviderError::InvalidSseEncoding)?;
        if line.is_empty() {
            if !self.data_lines.is_empty() {
                messages.push(self.data_lines.join("\n"));
                self.data_lines.clear();
            }
            return Ok(());
        }
        if line.starts_with(':') {
            return Ok(());
        }
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.strip_prefix(' ').unwrap_or(data);
            self.data_lines.push(data.to_owned());
        }
        Ok(())
    }
}

#[derive(Default)]
struct StreamAccumulator {
    content: String,
    tool_calls: BTreeMap<usize, StreamToolCall>,
    usage: Option<ModelUsage>,
    usage_raw: Option<serde_json::Value>,
    finish_reason: Option<ModelFinishReason>,
    saw_done: bool,
}

#[derive(Default)]
struct StreamToolCall {
    id: Option<String>,
    call_type: Option<String>,
    name: Option<String>,
    arguments: String,
}

impl StreamAccumulator {
    fn consume_data(
        &mut self,
        data: &str,
        emit: &mut (dyn FnMut(ModelEvent) + Send),
    ) -> Result<(), ProviderError> {
        if self.saw_done {
            return Err(ProviderError::DataAfterDone);
        }
        if data == "[DONE]" {
            self.saw_done = true;
            return Ok(());
        }
        let chunk = serde_json::from_str::<WireStreamChunk>(data).map_err(|error| {
            ProviderError::InvalidSseJson {
                reason: error.to_string(),
            }
        })?;
        if chunk.choices.len() > 1 {
            return Err(ProviderError::MultipleChoices);
        }
        if let Some(usage) = chunk.usage {
            let raw_usage = usage.clone();
            let usage = serde_json::from_value::<WireUsage>(usage).map_err(|error| {
                ProviderError::InvalidSseJson {
                    reason: format!("usage 字段无法映射：{error}"),
                }
            })?;
            let usage = ModelUsage::try_from(usage)?;
            match &self.usage {
                Some(current) if current == &usage => {}
                Some(_) => {
                    return Err(ProviderError::InvalidSseJson {
                        reason: "同一响应包含相互冲突的 usage".to_owned(),
                    });
                }
                None => {
                    self.usage = Some(usage.clone());
                    self.usage_raw = Some(raw_usage);
                    emit(ModelEvent::Usage { usage });
                }
            }
        }
        if let Some(choice) = chunk.choices.into_iter().next() {
            if choice.index != 0 {
                return Err(ProviderError::UnexpectedChoiceIndex {
                    index: choice.index,
                });
            }
            if let Some(refusal) = choice.delta.refusal {
                return Err(ProviderError::ModelRefusal { reason: refusal });
            }
            if let Some(content) = choice.delta.content {
                self.content.push_str(&content);
                emit(ModelEvent::TextDelta { content });
            }
            for tool_delta in choice.delta.tool_calls {
                self.consume_tool_delta(tool_delta, emit)?;
            }
            if let Some(reason) = choice.finish_reason {
                let reason = parse_finish_reason(&reason)?;
                if self.finish_reason.is_some_and(|current| current != reason) {
                    return Err(ProviderError::ConflictingFinishReason);
                }
                self.finish_reason = Some(reason);
            }
        }
        Ok(())
    }

    fn consume_tool_delta(
        &mut self,
        delta: WireToolCallDelta,
        emit: &mut (dyn FnMut(ModelEvent) + Send),
    ) -> Result<(), ProviderError> {
        let index = delta.index;
        let call = self.tool_calls.entry(index).or_default();
        if let Some(id) = &delta.id {
            if id.trim().is_empty() {
                return Err(ProviderError::MissingStreamToolCallId { index });
            }
            if call.id.as_ref().is_some_and(|current| current != id) {
                return Err(ProviderError::ConflictingToolCallId { index });
            }
            call.id = Some(id.clone());
        }
        if let Some(call_type) = &delta.call_type {
            if call_type != "function" {
                return Err(ProviderError::UnsupportedToolCallType { index });
            }
            call.call_type = Some(call_type.clone());
        }

        let mut name = None;
        let mut arguments_delta = String::new();
        if let Some(function) = delta.function {
            if let Some(function_name) = function.name {
                if function_name.trim().is_empty() {
                    return Err(ProviderError::MissingStreamToolCallName { index });
                }
                if call
                    .name
                    .as_ref()
                    .is_some_and(|current| current != &function_name)
                {
                    return Err(ProviderError::ConflictingToolCallName { index });
                }
                call.name = Some(function_name.clone());
                name = Some(function_name);
            }
            if let Some(arguments) = function.arguments {
                call.arguments.push_str(&arguments);
                arguments_delta = arguments;
            }
        }
        emit(ModelEvent::ToolCallDelta {
            index,
            id: delta.id,
            name,
            arguments_delta,
        });
        Ok(())
    }

    fn finish(self, provider_request_id: Option<String>) -> Result<ModelCompletion, ProviderError> {
        if !self.saw_done {
            return Err(ProviderError::MissingDoneMarker);
        }
        let finish_reason = self
            .finish_reason
            .ok_or(ProviderError::MissingFinishReason)?;
        let tool_calls = match finish_reason {
            ModelFinishReason::Stop => {
                if !self.tool_calls.is_empty() {
                    return Err(ProviderError::ToolCallsWithStop);
                }
                Vec::new()
            }
            ModelFinishReason::ToolCalls => {
                if self.tool_calls.is_empty() {
                    return Err(ProviderError::ToolCallsWithoutCalls);
                }
                // 适配器只负责把一次响应里出现的每个工具调用完整还原；「一轮只允许一个」
                // 的协议判定由阶段守卫（`AgentRunner`）统一负责，因此两者不会各判一次。
                self.tool_calls
                    .into_iter()
                    .map(|(index, call)| complete_stream_tool_call(index, call))
                    .collect::<Result<Vec<_>, _>>()?
            }
        };
        // Some reasoning-capable OpenAI-compatible endpoints stream a small amount of
        // assistant text (and may place the reasoning trace there) before the actual
        // function call.  A tool round is authoritative for the agent, so discard that
        // auxiliary text instead of rejecting an otherwise valid call as mixed output.
        Ok(ModelCompletion {
            provider_request_id,
            content: if tool_calls.is_empty() {
                Some(self.content)
            } else {
                None
            },
            tool_calls,
            usage: self.usage,
            usage_raw: self.usage_raw,
            finish_reason,
        })
    }
}

fn complete_stream_tool_call(
    index: usize,
    call: StreamToolCall,
) -> Result<ModelToolCall, ProviderError> {
    let id = call
        .id
        .ok_or(ProviderError::MissingStreamToolCallId { index })?;
    let name = call
        .name
        .ok_or(ProviderError::MissingStreamToolCallName { index })?;
    if call.call_type.as_deref() != Some("function") {
        return Err(ProviderError::MissingStreamToolCallType { index });
    }
    if call.arguments.len() > MAX_TOOL_ARGUMENT_BYTES {
        return Err(ProviderError::ToolArgumentsTooLarge { index });
    }
    let arguments = serde_json::from_str::<Value>(&call.arguments).map_err(|error| {
        ProviderError::InvalidStreamToolArguments {
            index,
            reason: error.to_string(),
        }
    })?;
    if !arguments.is_object() {
        return Err(ProviderError::InvalidStreamToolArguments {
            index,
            reason: "参数必须是 JSON object".to_owned(),
        });
    }
    Ok(ModelToolCall {
        id,
        name,
        arguments,
    })
}

fn parse_finish_reason(value: &str) -> Result<ModelFinishReason, ProviderError> {
    match value {
        "stop" => Ok(ModelFinishReason::Stop),
        "tool_calls" => Ok(ModelFinishReason::ToolCalls),
        other => Err(ProviderError::UnsupportedFinishReason {
            reason: other.to_owned(),
        }),
    }
}

#[derive(Deserialize)]
struct WireStreamChunk {
    #[serde(default)]
    choices: Vec<WireChoice>,
    #[serde(default)]
    usage: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct WireChoice {
    index: usize,
    #[serde(default)]
    delta: WireDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Default, Deserialize)]
struct WireDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    refusal: Option<String>,
    #[serde(default)]
    tool_calls: Vec<WireToolCallDelta>,
}

#[derive(Deserialize)]
struct WireToolCallDelta {
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(rename = "type", default)]
    call_type: Option<String>,
    #[serde(default)]
    function: Option<WireFunctionDelta>,
}

#[derive(Deserialize)]
struct WireFunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Deserialize)]
struct WireUsage {
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
    #[serde(default)]
    prompt_tokens_details: Option<WirePromptTokensDetails>,
    #[serde(default)]
    completion_tokens_details: Option<WireCompletionTokensDetails>,
}

#[derive(Deserialize)]
struct WirePromptTokensDetails {
    #[serde(default)]
    cached_tokens: Option<i64>,
}

#[derive(Deserialize)]
struct WireCompletionTokensDetails {
    #[serde(default)]
    reasoning_tokens: Option<i64>,
}

impl TryFrom<WireUsage> for ModelUsage {
    type Error = ProviderError;

    fn try_from(value: WireUsage) -> Result<Self, Self::Error> {
        for (field, count) in [
            ("prompt_tokens", value.prompt_tokens),
            ("completion_tokens", value.completion_tokens),
            ("total_tokens", value.total_tokens),
        ] {
            if count < 0 {
                return Err(ProviderError::NegativeUsage { field });
            }
        }
        if value
            .prompt_tokens_details
            .as_ref()
            .and_then(|details| details.cached_tokens)
            .is_some_and(|count| count < 0)
        {
            return Err(ProviderError::NegativeUsage {
                field: "cached_tokens",
            });
        }
        if value
            .completion_tokens_details
            .as_ref()
            .and_then(|details| details.reasoning_tokens)
            .is_some_and(|count| count < 0)
        {
            return Err(ProviderError::NegativeUsage {
                field: "reasoning_tokens",
            });
        }
        Ok(Self {
            input_tokens: value.prompt_tokens,
            output_tokens: value.completion_tokens,
            total_tokens: value.total_tokens,
            cached_input_tokens: value
                .prompt_tokens_details
                .and_then(|details| details.cached_tokens),
            reasoning_tokens: value
                .completion_tokens_details
                .and_then(|details| details.reasoning_tokens),
        })
    }
}

mod optional_decimal_i64 {
    use serde::{Deserialize, Deserializer, Serializer, de::Error as _};

    /// # Errors
    ///
    /// 当调用方的序列化器拒绝可空字符串值时返回其错误。
    #[allow(clippy::ref_option)] // Serde `with` 模块固定要求 `&Option<T>` 签名。
    pub fn serialize<S>(value: &Option<i64>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(value) => serializer.serialize_some(&value.to_string()),
            None => serializer.serialize_none(),
        }
    }

    /// # Errors
    ///
    /// 非空 JSON 值不是十进制字符串，或数值超出 `i64` 范围时返回反序列化错误。
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<String>::deserialize(deserializer)?
            .map(|value| {
                value.parse::<i64>().map_err(|error| {
                    D::Error::custom(format!("64 位整数必须是十进制字符串：{error}"))
                })
            })
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        sync::oneshot,
    };

    use super::*;

    fn capabilities() -> ModelCapabilityProfile {
        ModelCapabilityProfile {
            streaming: true,
            tools: true,
            structured_output: true,
            stream_usage: true,
            thinking: true,
        }
    }

    fn adapter(base_url: &str) -> ChatCompletionsAdapter {
        ChatCompletionsAdapter::new(
            ChatCompletionsConfig::new(
                base_url,
                "tested-model",
                Some("test-secret".to_owned()),
                capabilities(),
                ModelTimeouts::default(),
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn text_request() -> ModelRequest {
        ModelRequest::from_context(
            vec![ContextMessage {
                role: ContextMessageRole::User,
                content: "请写一句话".to_owned(),
            }],
            Vec::new(),
        )
    }

    #[test]
    fn api_base_must_be_an_explicit_prefix_and_never_a_full_endpoint() {
        assert!(matches!(
            ChatCompletionsConfig::new(
                "http://127.0.0.1:11434",
                "model",
                None,
                capabilities(),
                ModelTimeouts::default(),
            ),
            Err(ProviderError::MissingApiPrefix)
        ));
        assert!(matches!(
            ChatCompletionsConfig::new(
                "http://127.0.0.1:11434/v1/chat/completions",
                "model",
                None,
                capabilities(),
                ModelTimeouts::default(),
            ),
            Err(ProviderError::EndpointInsteadOfApiBase)
        ));

        let adapter = adapter("http://127.0.0.1:11434/v1");
        assert_eq!(
            adapter.endpoint().as_str(),
            "http://127.0.0.1:11434/v1/chat/completions"
        );
    }

    #[test]
    fn sse_decoder_handles_crlf_and_utf8_across_transport_chunks() {
        let mut decoder = SseDecoder::default();
        assert_eq!(
            decoder.push(b"data: {\"content\":\"").unwrap(),
            [] as [std::string::String; 0]
        );
        let messages = decoder.push("你好\"}\r\n\r\n".as_bytes()).unwrap();
        assert_eq!(messages, vec![r#"{"content":"你好"}"#]);
    }

    #[tokio::test]
    async fn adapter_streams_text_usage_and_a_stable_request_shape() {
        let stream = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"你好\"}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":3,\"total_tokens\":15,\"prompt_tokens_details\":{\"cached_tokens\":4},\"completion_tokens_details\":{\"reasoning_tokens\":2}}}\n\n",
            "data: [DONE]\n\n",
        );
        let split_at = stream.find("你好").unwrap() + 1;
        let (base_url, request) = spawn_sse_server(vec![
            stream.as_bytes()[..split_at].to_vec(),
            stream.as_bytes()[split_at..].to_vec(),
        ])
        .await;

        let mut events = Vec::new();
        let completion = adapter(&base_url)
            .execute_with(&text_request(), |event| events.push(event))
            .await
            .unwrap();

        assert_eq!(completion.content.as_deref(), Some("你好"));
        // 供应商的原始用量字段必须原样保留，供事后核对为什么某次用量是未知。
        assert_eq!(
            completion.usage_raw,
            Some(serde_json::json!({
                "prompt_tokens": 12,
                "completion_tokens": 3,
                "total_tokens": 15,
                "prompt_tokens_details": { "cached_tokens": 4 },
                "completion_tokens_details": { "reasoning_tokens": 2 }
            }))
        );
        assert_eq!(
            completion.provider_request_id.as_deref(),
            Some("provider-req-1")
        );
        assert_eq!(
            completion.usage,
            Some(ModelUsage {
                input_tokens: 12,
                output_tokens: 3,
                total_tokens: 15,
                cached_input_tokens: Some(4),
                reasoning_tokens: Some(2),
            })
        );
        assert!(
            matches!(events.first(), Some(ModelEvent::TextDelta { content }) if content == "你好")
        );
        assert!(matches!(
            events.last(),
            Some(ModelEvent::Completed {
                finish_reason: ModelFinishReason::Stop
            })
        ));

        let request = request.await.unwrap();
        let body = request_body(&request);
        assert_eq!(body["model"], "tested-model");
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(body["messages"][0]["role"], "user");
        assert!(request.contains("authorization: Bearer test-secret"));
    }

    #[tokio::test]
    async fn adapter_accumulates_exactly_one_tool_call_and_disables_parallel_calls() {
        let stream = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-1\",\"type\":\"function\",\"function\":{\"name\":\"append_content\",\"arguments\":\"{\\\"content\\\":\\\"\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"正文\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let (base_url, request) = spawn_sse_server(vec![stream.as_bytes().to_vec()]).await;
        let model_request = ModelRequest {
            messages: text_request().messages,
            tools: vec![super::super::AgentToolDefinition {
                name: "append_content".to_owned(),
                description: "追加正文".to_owned(),
                parameters: json!({
                    "type": "object",
                    "required": ["content"],
                    "properties": { "content": { "type": "string" } },
                }),
            }],
            response_format: None,
            thinking_level: ThinkingLevel::Off,
        };
        let completion = adapter(&base_url).complete(&model_request).await.unwrap();

        assert_eq!(completion.content, None);
        assert_eq!(completion.finish_reason, ModelFinishReason::ToolCalls);
        assert_eq!(completion.tool_calls.len(), 1);
        assert_eq!(completion.tool_calls[0].id, "call-1");
        assert_eq!(completion.tool_calls[0].name, "append_content");
        assert_eq!(
            completion.tool_calls[0].arguments,
            json!({ "content": "正文" })
        );

        let body = request_body(&request.await.unwrap());
        assert_eq!(body["parallel_tool_calls"], false);
        assert_eq!(body["tools"][0]["function"]["name"], "append_content");
    }

    #[tokio::test]
    async fn adapter_discards_auxiliary_text_before_a_tool_call() {
        // Reasoning-capable providers may stream a separator or reasoning trace as
        // `delta.content` before returning the function call. The tool arguments remain
        // the authoritative output for this round.
        let stream = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"\\n\\n\"}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-mixed\",\"type\":\"function\",\"function\":{\"name\":\"append_content\",\"arguments\":\"{\\\"content\\\":\\\"正文\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let (base_url, _request) = spawn_sse_server(vec![stream.as_bytes().to_vec()]).await;
        let model_request = ModelRequest {
            messages: text_request().messages,
            tools: vec![super::super::AgentToolDefinition {
                name: "append_content".to_owned(),
                description: "追加正文".to_owned(),
                parameters: json!({
                    "type": "object",
                    "required": ["content"],
                    "properties": { "content": { "type": "string" } },
                }),
            }],
            response_format: None,
            thinking_level: ThinkingLevel::Low,
        };

        let completion = adapter(&base_url).complete(&model_request).await.unwrap();

        assert_eq!(completion.content, None);
        assert_eq!(completion.tool_calls.len(), 1);
        assert_eq!(completion.tool_calls[0].name, "append_content");
        assert_eq!(
            completion.tool_calls[0].arguments,
            json!({ "content": "正文" })
        );
    }

    #[tokio::test]
    async fn adapter_surfaces_every_tool_call_in_a_parallel_batch() {
        let stream = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-1\",\"type\":\"function\",\"function\":{\"name\":\"append_content\",\"arguments\":\"{}\"}},{\"index\":1,\"id\":\"call-2\",\"type\":\"function\",\"function\":{\"name\":\"replace_content\",\"arguments\":\"{}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let (base_url, _request) = spawn_sse_server(vec![stream.as_bytes().to_vec()]).await;
        let completion = adapter(&base_url).complete(&text_request()).await.unwrap();
        assert_eq!(completion.tool_calls.len(), 2);
        assert_eq!(completion.tool_calls[0].id, "call-1");
        assert_eq!(completion.tool_calls[1].id, "call-2");
    }

    #[test]
    fn request_rejects_unpaired_tool_messages_and_missing_capability() {
        let request = ModelRequest {
            messages: vec![ModelRequestMessage::AssistantToolCalls {
                content: None,
                tool_calls: vec![ModelToolCall {
                    id: "call-1".to_owned(),
                    name: "append_content".to_owned(),
                    arguments: json!({ "content": "正文" }),
                }],
            }],
            tools: Vec::new(),
            response_format: None,
            thinking_level: ThinkingLevel::Off,
        };
        let error = adapter("http://127.0.0.1:11434/v1")
            .request_payload(&request)
            .unwrap_err();
        assert!(matches!(error, ProviderError::MissingToolResult { .. }));

        let no_tools = ModelCapabilityProfile {
            tools: false,
            ..capabilities()
        };
        let adapter = ChatCompletionsAdapter::new(
            ChatCompletionsConfig::new(
                "http://127.0.0.1:11434/v1",
                "model",
                None,
                no_tools,
                ModelTimeouts::default(),
            )
            .unwrap(),
        )
        .unwrap();
        let request = ModelRequest {
            messages: text_request().messages,
            tools: vec![super::super::AgentToolDefinition {
                name: "append_content".to_owned(),
                description: "追加".to_owned(),
                parameters: json!({ "type": "object" }),
            }],
            response_format: None,
            thinking_level: ThinkingLevel::Off,
        };
        assert!(matches!(
            adapter.request_payload(&request),
            Err(ProviderError::CapabilityMismatch {
                capability: "tools"
            })
        ));
    }

    #[test]
    fn thinking_level_controls_reasoning_effort_in_the_payload() {
        let request = ModelRequest {
            messages: text_request().messages,
            tools: Vec::new(),
            response_format: None,
            thinking_level: ThinkingLevel::Low,
        };
        let payload = adapter("http://127.0.0.1:11434/v1")
            .request_payload(&request)
            .unwrap();
        assert_eq!(payload["reasoning_effort"], json!("low"));

        // `Off` 时不发送该字段：能力声明可以宽松，但请求不能凭空多带参数。
        let off = ModelRequest {
            thinking_level: ThinkingLevel::Off,
            ..request
        };
        assert!(
            adapter("http://127.0.0.1:11434/v1")
                .request_payload(&off)
                .unwrap()
                .get("reasoning_effort")
                .is_none()
        );
    }

    async fn spawn_sse_server(chunks: Vec<Vec<u8>>) -> (String, oneshot::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (request_sender, request_receiver) = oneshot::channel();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (mut stream, request) = read_http_request(stream).await;
            request_sender.send(request).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream; charset=utf-8\r\nx-request-id: provider-req-1\r\nconnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            for chunk in chunks {
                stream.write_all(&chunk).await.unwrap();
            }
            stream.shutdown().await.unwrap();
        });
        (format!("http://{address}/v1"), request_receiver)
    }

    async fn read_http_request(mut stream: TcpStream) -> (TcpStream, String) {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 4096];
        let header_end;
        loop {
            let read = stream.read(&mut buffer).await.unwrap();
            assert_ne!(read, 0, "client closed before sending a request");
            bytes.extend_from_slice(&buffer[..read]);
            if let Some(position) = find_bytes(&bytes, b"\r\n\r\n") {
                header_end = position + 4;
                break;
            }
        }
        let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
        let content_length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length: "))
            .unwrap()
            .parse::<usize>()
            .unwrap();
        while bytes.len() < header_end + content_length {
            let read = stream.read(&mut buffer).await.unwrap();
            assert_ne!(read, 0, "client closed before request body completed");
            bytes.extend_from_slice(&buffer[..read]);
        }
        (
            stream,
            String::from_utf8(bytes[..header_end + content_length].to_vec()).unwrap(),
        )
    }

    fn request_body(request: &str) -> Value {
        let (_, body) = request.split_once("\r\n\r\n").unwrap();
        serde_json::from_str(body).unwrap()
    }

    fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }
}
