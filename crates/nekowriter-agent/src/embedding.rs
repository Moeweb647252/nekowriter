//! The embeddings adapter (`docs/product-spec.md` §12.2, D10): one OpenAI-compatible
//! `embeddings` endpoint, whose answers are checked against the configured model.

use std::time::Duration;

use futures_util::StreamExt;
use nekowriter_core::{
    embedding::{EmbeddingFailure, EmbeddingModel},
    model::ModelService,
    run::TimeoutPhase,
};
use serde::Deserialize;
use serde_json::json;
use thiserror::Error;
use tokio::time::timeout;

use crate::client::{ChatClient, ProviderError};

/// The most texts sent in one request, which bounds the size of the answer.
const MAX_BATCH: usize = 32;
/// The largest answer accepted: a full batch of the largest vectors as JSON text.
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// The vectors of some texts, in the order of the texts.
#[derive(Clone, Debug, PartialEq)]
pub struct Embeddings {
    pub vectors: Vec<Vec<f32>>,
    /// The input tokens the service reported, if it reported them for every request.
    pub input_tokens: Option<i64>,
}

#[derive(Debug, Error)]
pub enum EmbeddingError {
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error("the vectors have {actual} dimensions instead of {expected}")]
    WrongDimensions { expected: u32, actual: u32 },
    #[error("a vector holds a value that is not finite")]
    NotFinite,
    #[error("{actual} vectors came back for {expected} texts")]
    WrongCount { expected: u32, actual: u32 },
}

impl EmbeddingError {
    /// The failure as it is stored and shown.
    #[must_use]
    pub fn failure(&self) -> EmbeddingFailure {
        match self {
            Self::Provider(error) => EmbeddingFailure::Provider {
                failure: error.failure(),
            },
            Self::WrongDimensions { expected, actual } => EmbeddingFailure::WrongDimensions {
                expected: *expected,
                actual: *actual,
            },
            Self::NotFinite => EmbeddingFailure::NotFinite,
            Self::WrongCount { expected, actual } => EmbeddingFailure::WrongCount {
                expected: *expected,
                actual: *actual,
            },
        }
    }
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingItem>,
    #[serde(default)]
    usage: Option<EmbeddingUsage>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingItem {
    index: usize,
    embedding: Vec<f64>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingUsage {
    #[serde(default)]
    prompt_tokens: Option<i64>,
}

fn count(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

impl ChatClient {
    /// Embeds texts, in requests of at most [`MAX_BATCH`], and checks that every vector
    /// has the configured size and only finite values.
    pub async fn embed(
        &self,
        service: &ModelService,
        model: &EmbeddingModel,
        texts: &[String],
    ) -> Result<Embeddings, EmbeddingError> {
        let mut embeddings = Embeddings {
            vectors: Vec::with_capacity(texts.len()),
            input_tokens: Some(0),
        };
        for batch in texts.chunks(MAX_BATCH) {
            let part = self.embed_batch(service, model, batch).await?;
            embeddings.vectors.extend(part.vectors);
            embeddings.input_tokens = embeddings
                .input_tokens
                .zip(part.input_tokens)
                .map(|(sum, tokens)| sum + tokens);
        }
        Ok(embeddings)
    }

    /// Embeds one text, such as a search query.
    pub async fn embed_one(
        &self,
        service: &ModelService,
        model: &EmbeddingModel,
        text: String,
    ) -> Result<(Vec<f32>, Option<i64>), EmbeddingError> {
        let embeddings = self.embed(service, model, &[text]).await?;
        let count = embeddings.vectors.len();
        match <[Vec<f32>; 1]>::try_from(embeddings.vectors) {
            Ok([vector]) => Ok((vector, embeddings.input_tokens)),
            Err(_) => Err(EmbeddingError::WrongCount {
                expected: 1,
                actual: u32::try_from(count).unwrap_or(u32::MAX),
            }),
        }
    }

    async fn embed_batch(
        &self,
        service: &ModelService,
        model: &EmbeddingModel,
        texts: &[String],
    ) -> Result<Embeddings, EmbeddingError> {
        let body = json!({
            "model": model.model_id.as_str(),
            "input": texts,
            "dimensions": model.dimensions,
            "encoding_format": "float",
        });
        let answer = timeout(
            REQUEST_TIMEOUT,
            self.post_json(service, "embeddings", &body),
        )
        .await
        .map_err(|_| ProviderError::Timeout(TimeoutPhase::FirstByte))??;
        let response: EmbeddingResponse = serde_json::from_slice(&answer)
            .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
        if response.data.len() != texts.len() {
            return Err(EmbeddingError::WrongCount {
                expected: count(texts.len()),
                actual: count(response.data.len()),
            });
        }
        let mut items = response.data;
        items.sort_by_key(|item| item.index);
        let vectors = items
            .into_iter()
            .map(|item| {
                if item.embedding.len() != model.dimensions as usize {
                    return Err(EmbeddingError::WrongDimensions {
                        expected: model.dimensions,
                        actual: count(item.embedding.len()),
                    });
                }
                #[allow(clippy::cast_possible_truncation)] // Embeddings are stored as f32.
                let vector: Vec<f32> = item.embedding.iter().map(|&value| value as f32).collect();
                if vector.iter().all(|value| value.is_finite()) {
                    Ok(vector)
                } else {
                    Err(EmbeddingError::NotFinite)
                }
            })
            .collect::<Result<_, _>>()?;
        Ok(Embeddings {
            vectors,
            input_tokens: response.usage.and_then(|usage| usage.prompt_tokens),
        })
    }

    /// Posts a JSON body and reads the whole answer, up to [`MAX_RESPONSE_BYTES`].
    async fn post_json(
        &self,
        service: &ModelService,
        endpoint: &str,
        body: &serde_json::Value,
    ) -> Result<Vec<u8>, ProviderError> {
        let response = self.post(service, endpoint, body).await?;
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
            if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(ProviderError::ResponseTooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use axum::{Router, routing::post};
    use nekowriter_core::model::{ApiBaseUrl, ModelId};

    use super::*;

    async fn serve(body: &'static str) -> SocketAddr {
        let app = Router::new().route("/v1/embeddings", post(move || async move { body }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        address
    }

    async fn embed(body: &'static str, texts: &[&str]) -> Result<Embeddings, EmbeddingError> {
        let address = serve(body).await;
        let service = ModelService {
            base_url: ApiBaseUrl::parse(&format!("http://{address}/v1")).unwrap(),
            api_key: None,
        };
        let model = EmbeddingModel {
            model_id: ModelId::parse("embedder").unwrap(),
            dimensions: 2,
        };
        let texts: Vec<String> = texts.iter().map(|&text| text.to_owned()).collect();
        ChatClient::new().embed(&service, &model, &texts).await
    }

    #[tokio::test]
    async fn vectors_come_back_in_the_order_of_the_texts() {
        let embeddings = embed(
            r#"{"data":[{"index":1,"embedding":[0,1]},{"index":0,"embedding":[1,0]}],"usage":{"prompt_tokens":7}}"#,
            &["a", "b"],
        )
        .await
        .unwrap();
        assert_eq!(embeddings.vectors, [vec![1.0, 0.0], vec![0.0, 1.0]]);
        assert_eq!(embeddings.input_tokens, Some(7));
    }

    #[tokio::test]
    async fn answers_that_do_not_fit_the_model_name_the_problem() {
        assert!(matches!(
            embed(r#"{"data":[{"index":0,"embedding":[1,0,0]}]}"#, &["a"]).await,
            Err(EmbeddingError::WrongDimensions {
                expected: 2,
                actual: 3
            })
        ));
        assert!(matches!(
            embed(r#"{"data":[{"index":0,"embedding":[1e300,0]}]}"#, &["a"]).await,
            Err(EmbeddingError::NotFinite)
        ));
        assert!(matches!(
            embed(r#"{"data":[]}"#, &["a"]).await,
            Err(EmbeddingError::WrongCount {
                expected: 1,
                actual: 0
            })
        ));
    }
}
