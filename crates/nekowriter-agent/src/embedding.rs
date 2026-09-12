//! Embedding 适配器：把文本批量转换成向量。
//!
//! 只实现 `OpenAI` 兼容的嵌入端点（`POST {api_base}/embeddings`）合同。调用方负责凭据解析、
//! 计量与失败重试；适配器在返回前校验响应条数、输出维度与有限值，绝不把 NaN、
//! 无穷大或长度不符的向量交给索引写入。

use std::time::Duration;

use reqwest::{Client, header};
use serde::Deserialize;
use serde_json::json;
use thiserror::Error;

use nekowriter_core::{PersistedModelUsage, UsageUnknownReason};

use crate::provider::normalize_api_base_url;

/// 单次 Embedding 请求超时。向量请求不是流式的，超时后整批失败重试。
const EMBEDDING_TIMEOUT: Duration = Duration::from_secs(60);
/// 单次请求允许的最大输入条数，避免一次索引任务构造过长请求体。
pub const MAX_EMBEDDING_BATCH: usize = 64;

#[derive(Debug, Error)]
pub enum EmbeddingError {
    #[error("Embedding 地址无效：{reason}")]
    InvalidApiBaseUrl { reason: String },
    #[error("Embedding API 根地址必须是根地址，不能包含 embeddings 端点")]
    EndpointInsteadOfApiBase,
    #[error("Embedding 模型 ID 不能为空")]
    EmptyModelId,
    #[error("Embedding 输出维度必须大于零")]
    EmptyDimension,
    #[error("无法创建 Embedding HTTP 客户端：{reason}")]
    ClientBuild { reason: String },
    #[error("Embedding 请求至少需要一条输入")]
    EmptyInput,
    #[error("Embedding 请求最多 {MAX_EMBEDDING_BATCH} 条输入")]
    TooManyInputs,
    #[error("Embedding 服务请求失败：{reason}")]
    Transport { reason: String },
    #[error("Embedding 服务返回 HTTP {status}")]
    HttpStatus { status: u16 },
    #[error("Embedding 服务响应无法解析：{reason}")]
    InvalidResponse { reason: String },
    #[error("Embedding 服务返回 {actual} 条向量，期望 {expected} 条")]
    UnexpectedCount { expected: usize, actual: usize },
    #[error("Embedding 维度不符：期望 {expected}，实际 {actual}")]
    DimensionMismatch { expected: i64, actual: i64 },
    #[error("Embedding 第 {index} 条向量包含 NaN 或无穷大")]
    NonFinite { index: usize },
}

/// 已从本地凭据存储解析出的 Embedding 调用参数。
///
/// `api_key` 只存在于进程内，不参与 `Debug` 输出，也不会写回 SQLite。
#[derive(Clone)]
pub struct EmbeddingConfig {
    api_base_url: String,
    model: String,
    dimension: i64,
    api_key: Option<String>,
}

impl std::fmt::Debug for EmbeddingConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EmbeddingConfig")
            .field("api_base_url", &self.api_base_url)
            .field("model", &self.model)
            .field("dimension", &self.dimension)
            .field("api_key_configured", &self.api_key.is_some())
            .finish_non_exhaustive()
    }
}

impl EmbeddingConfig {
    /// 创建并验证显式 API 根地址、模型与输出维度。
    ///
    /// # Errors
    ///
    /// 地址不是带 `/v1` 之类前缀的 http(s) 根地址、模型为空、维度非正时返回错误。
    pub fn new(
        api_base_url: impl AsRef<str>,
        model: impl Into<String>,
        dimension: i64,
        api_key: Option<String>,
    ) -> Result<Self, EmbeddingError> {
        let normalized = normalize_api_base_url(api_base_url.as_ref()).map_err(|error| {
            EmbeddingError::InvalidApiBaseUrl {
                reason: error.to_string(),
            }
        })?;
        if normalized
            .path()
            .trim_end_matches('/')
            .ends_with("/embeddings")
        {
            return Err(EmbeddingError::EndpointInsteadOfApiBase);
        }
        let model = model.into();
        if model.trim().is_empty() {
            return Err(EmbeddingError::EmptyModelId);
        }
        if dimension <= 0 {
            return Err(EmbeddingError::EmptyDimension);
        }
        Ok(Self {
            api_base_url: normalized.to_string(),
            model,
            dimension,
            api_key,
        })
    }

    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    #[must_use]
    pub fn dimension(&self) -> i64 {
        self.dimension
    }
}

/// 一次 Embedding 请求的结果：与输入顺序一致的向量，以及供应商报告的用量。
///
/// 调用方负责把 `usage` 落库：适配器只把“供应商说了什么”翻译成统一口径，没有报告
/// 时给出带原因的未知记录，绝不自己估算一个看起来完整的数字。
#[derive(Clone, Debug, PartialEq)]
pub struct EmbeddingBatch {
    pub vectors: Vec<Vec<f32>>,
    pub usage: PersistedModelUsage,
}

/// 一次 Embedding 请求的固定配置所对应的客户端。
#[derive(Clone)]
pub struct EmbeddingClient {
    client: Client,
    config: EmbeddingConfig,
}

impl std::fmt::Debug for EmbeddingClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EmbeddingClient")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl EmbeddingClient {
    /// 用已验证的配置创建客户端。
    ///
    /// # Errors
    ///
    /// 无法创建带超时的 HTTP 客户端时返回错误。
    pub fn new(config: EmbeddingConfig) -> Result<Self, EmbeddingError> {
        let client = Client::builder()
            .timeout(EMBEDDING_TIMEOUT)
            .build()
            .map_err(|error| EmbeddingError::ClientBuild {
                reason: error.to_string(),
            })?;
        Ok(Self { client, config })
    }

    #[must_use]
    pub fn dimension(&self) -> i64 {
        self.config.dimension
    }

    /// 批量计算向量，返回顺序与输入一致，并带上供应商报告的用量。
    ///
    /// # Errors
    ///
    /// 输入为空或超批、网络失败、非 2xx 响应、响应条数或维度不符、出现非有限值时
    /// 返回错误；调用方据此把索引任务标记为失败。
    pub async fn embed(&self, inputs: &[String]) -> Result<EmbeddingBatch, EmbeddingError> {
        if inputs.is_empty() {
            return Err(EmbeddingError::EmptyInput);
        }
        if inputs.len() > MAX_EMBEDDING_BATCH {
            return Err(EmbeddingError::TooManyInputs);
        }
        let url = format!("{}embeddings", self.config.api_base_url);
        let mut request = self.client.post(&url).json(&json!({
            "model": self.config.model,
            "input": inputs,
            "encoding_format": "float",
        }));
        if let Some(api_key) = &self.config.api_key {
            request = request.header(header::AUTHORIZATION, format!("Bearer {api_key}"));
        }
        let response = request
            .send()
            .await
            .map_err(|error| EmbeddingError::Transport {
                reason: error.to_string(),
            })?;
        let status = response.status();
        if !status.is_success() {
            return Err(EmbeddingError::HttpStatus {
                status: status.as_u16(),
            });
        }
        let body: EmbeddingResponse =
            response
                .json()
                .await
                .map_err(|error| EmbeddingError::InvalidResponse {
                    reason: error.to_string(),
                })?;
        if body.data.len() != inputs.len() {
            return Err(EmbeddingError::UnexpectedCount {
                expected: inputs.len(),
                actual: body.data.len(),
            });
        }
        let mut ordered: Vec<Option<Vec<f32>>> = vec![None; inputs.len()];
        for item in body.data {
            let index = item.index;
            if index >= ordered.len() {
                return Err(EmbeddingError::InvalidResponse {
                    reason: format!("响应 index {index} 超出输入范围"),
                });
            }
            ordered[index] = Some(item.embedding);
        }
        let mut vectors = Vec::with_capacity(inputs.len());
        for (index, vector) in ordered.into_iter().enumerate() {
            let Some(vector) = vector else {
                return Err(EmbeddingError::InvalidResponse {
                    reason: format!("响应缺少 index {index}"),
                });
            };
            let actual = usize::try_from(self.config.dimension).unwrap_or(usize::MAX);
            if vector.len() != actual {
                return Err(EmbeddingError::DimensionMismatch {
                    expected: self.config.dimension,
                    actual: i64::try_from(vector.len()).unwrap_or(i64::MAX),
                });
            }
            if vector.iter().any(|value| !value.is_finite()) {
                return Err(EmbeddingError::NonFinite { index });
            }
            vectors.push(vector);
        }
        Ok(EmbeddingBatch {
            vectors,
            usage: body.usage.map_or_else(
                || PersistedModelUsage::unknown(UsageUnknownReason::ProviderSilent),
                |usage| embed_usage(&usage),
            ),
        })
    }
}

/// 把供应商报告的用量映射成统一口径。
///
/// 用量只是一个统计字段，因此这里永不因为它的形状而让向量请求失败：不管供应商把
/// `usage` 写成对象、字符串形式的数字、空数组还是别的什么，解析结果只影响用量口径
/// 本身。嵌入请求没有输出侧，所以供应商报告的 `total_tokens` 就等于输入用量，只有
/// `prompt_tokens` 缺失时才用它；报告在但读不出任何计数，才是「无法映射」。
fn embed_usage(usage: &serde_json::Value) -> PersistedModelUsage {
    let fields = usage.as_object();
    let count = |name: &str| {
        fields
            .and_then(|fields| fields.get(name))
            .and_then(token_count)
    };
    let prompt_tokens = count("prompt_tokens");
    let total_tokens = count("total_tokens");
    let Some(input_tokens) = prompt_tokens.or(total_tokens) else {
        return PersistedModelUsage::unknown(UsageUnknownReason::InvalidReport);
    };
    let total_tokens = total_tokens.unwrap_or(input_tokens);
    PersistedModelUsage::from_embeddings_response(input_tokens, total_tokens, Some(usage.clone()))
}

/// 宽容地读一个 token 计数：供应商可能报整数，也可能报字符串形式的数字。
fn token_count(value: &serde_json::Value) -> Option<i64> {
    match value {
        serde_json::Value::Number(number) => number.as_i64(),
        serde_json::Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingItem>,
    /// 供应商报告的用量原文。是否报告、以什么形状报告都由供应商决定，所以这里不做
    /// 结构约束：形状不合预期只让用量变成「无法解析」，绝不让向量请求失败。
    #[serde(default)]
    usage: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingItem {
    index: usize,
    embedding: Vec<f32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use nekowriter_core::{UsageSource, UsageStatus};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn rejects_endpoint_instead_of_api_base() {
        let error =
            EmbeddingConfig::new("https://host.example/v1/embeddings", "m", 3, None).unwrap_err();
        assert!(matches!(error, EmbeddingError::EndpointInsteadOfApiBase));
    }

    #[test]
    fn rejects_non_positive_dimension_and_empty_model() {
        assert!(matches!(
            EmbeddingConfig::new("https://host.example/v1", "", 3, None).unwrap_err(),
            EmbeddingError::EmptyModelId
        ));
        assert!(matches!(
            EmbeddingConfig::new("https://host.example/v1", "m", 0, None).unwrap_err(),
            EmbeddingError::EmptyDimension
        ));
    }

    /// 一次性本地服务，返回给定的 JSON 响应体。
    fn serve_once(body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut buffer = [0_u8; 8192];
            let _ = socket.read(&mut buffer);
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes());
        });
        format!("http://{address}/v1")
    }

    #[tokio::test]
    async fn never_fails_a_vector_request_over_the_shape_of_usage() {
        // 用量只是统计字段：不管供应商把它写成什么形状，向量都必须拿到，而用量口径
        // 只如实反映供应商到底说了什么。
        let cases = [
            (
                r#"{"data":[{"index":0,"embedding":[1.0,2.0]}],"usage":{"prompt_tokens":"7"}}"#,
                Some(7),
                UsageStatus::Reported,
            ),
            (
                r#"{"data":[{"index":0,"embedding":[1.0,2.0]}],"usage":7}"#,
                None,
                UsageStatus::Invalid,
            ),
            (
                r#"{"data":[{"index":0,"embedding":[1.0,2.0]}],"usage":null}"#,
                None,
                UsageStatus::Unknown,
            ),
            (
                r#"{"data":[{"index":0,"embedding":[1.0,2.0]}],"usage":[]}"#,
                None,
                UsageStatus::Invalid,
            ),
            (
                r#"{"data":[{"index":0,"embedding":[1.0,2.0]}],"usage":{}}"#,
                None,
                UsageStatus::Invalid,
            ),
            (
                r#"{"data":[{"index":0,"embedding":[1.0,2.0]}],"usage":{"total_tokens":9}}"#,
                Some(9),
                UsageStatus::Reported,
            ),
            (
                r#"{"data":[{"index":0,"embedding":[1.0,2.0]}]}"#,
                None,
                UsageStatus::Unknown,
            ),
        ];
        for (body, input_tokens, status) in cases {
            let base = serve_once(body);
            let client =
                EmbeddingClient::new(EmbeddingConfig::new(base, "m", 2, None).unwrap()).unwrap();
            let batch = client.embed(&["a".to_owned()]).await.unwrap();
            assert_eq!(batch.vectors, vec![vec![1.0, 2.0]], "{body}");
            assert_eq!(batch.usage.usage_status, status, "{body}");
            assert_eq!(batch.usage.input_tokens, input_tokens, "{body}");
        }
    }

    #[tokio::test]
    async fn parses_vectors_in_index_order() {
        let base = serve_once(
            r#"{"object":"list","data":[{"object":"embedding","index":1,"embedding":[3.0,4.0]},{"object":"embedding","index":0,"embedding":[1.0,2.0]}]}"#,
        );
        let client =
            EmbeddingClient::new(EmbeddingConfig::new(base, "m", 2, None).unwrap()).unwrap();
        let batch = client
            .embed(&["a".to_owned(), "b".to_owned()])
            .await
            .unwrap();
        assert_eq!(batch.vectors, vec![vec![1.0, 2.0], vec![3.0, 4.0]]);
        // 供应商没有报告用量时留下带原因的未知记录，不估算一个看起来完整的数字。
        assert_eq!(
            batch.usage,
            PersistedModelUsage::unknown(UsageUnknownReason::ProviderSilent)
        );
    }

    #[tokio::test]
    async fn reports_the_usage_the_provider_actually_returned() {
        let base = serve_once(
            r#"{"data":[{"index":0,"embedding":[1.0,2.0]}],"usage":{"prompt_tokens":128,"total_tokens":128}}"#,
        );
        let client =
            EmbeddingClient::new(EmbeddingConfig::new(base, "m", 2, None).unwrap()).unwrap();
        let batch = client.embed(&["a".to_owned()]).await.unwrap();
        assert_eq!(batch.usage.usage_source, UsageSource::EmbeddingsHttp);
        assert_eq!(batch.usage.usage_status, UsageStatus::Reported);
        assert_eq!(batch.usage.input_tokens, Some(128));
        assert_eq!(batch.usage.output_tokens, Some(0));
        assert!(batch.usage.raw.is_some());

        // 报告存在但没有任何可用计数：标为无法解析，而不是 0。
        let base = serve_once(r#"{"data":[{"index":0,"embedding":[1.0,2.0]}],"usage":{}}"#);
        let client =
            EmbeddingClient::new(EmbeddingConfig::new(base, "m", 2, None).unwrap()).unwrap();
        let batch = client.embed(&["a".to_owned()]).await.unwrap();
        assert_eq!(
            batch.usage,
            PersistedModelUsage::unknown(UsageUnknownReason::InvalidReport)
        );
    }

    #[tokio::test]
    async fn rejects_wrong_dimension_and_non_finite_values() {
        let base = serve_once(r#"{"data":[{"index":0,"embedding":[1.0,2.0,3.0]}]}"#);
        let client =
            EmbeddingClient::new(EmbeddingConfig::new(base, "m", 2, None).unwrap()).unwrap();
        assert!(matches!(
            client.embed(&["a".to_owned()]).await.unwrap_err(),
            EmbeddingError::DimensionMismatch {
                expected: 2,
                actual: 3
            }
        ));

        let base = serve_once(r#"{"data":[{"index":0,"embedding":[1.0,"x"]}]}"#);
        let client =
            EmbeddingClient::new(EmbeddingConfig::new(base, "m", 2, None).unwrap()).unwrap();
        assert!(matches!(
            client.embed(&["a".to_owned()]).await.unwrap_err(),
            EmbeddingError::InvalidResponse { .. }
        ));
    }

    #[tokio::test]
    async fn rejects_count_mismatch_and_empty_input() {
        let base = serve_once(r#"{"data":[{"index":0,"embedding":[1.0,2.0]}]}"#);
        let client =
            EmbeddingClient::new(EmbeddingConfig::new(base, "m", 2, None).unwrap()).unwrap();
        assert!(matches!(
            client.embed(&[]).await.unwrap_err(),
            EmbeddingError::EmptyInput
        ));
        assert!(matches!(
            client
                .embed(&["a".to_owned(), "b".to_owned()])
                .await
                .unwrap_err(),
            EmbeddingError::UnexpectedCount {
                expected: 2,
                actual: 1
            }
        ));
    }
}
