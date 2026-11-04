//! Provider-neutral LLM client for NekoWriter.
//!
//! Two protocol adapters (OpenAI-compatible Chat Completions and Anthropic Messages) plus a
//! mock provider for tests and the Demo provider (see `docs/spec/04-llm.md`).

pub mod anthropic;
pub mod demo;
pub mod mock;
pub mod openai;
pub mod retry;
pub mod sse;
pub mod types;

pub use openai::estimate_tokens;
pub use retry::LlmError;
pub use types::*;

use std::sync::Arc;
use std::time::Duration;

/// Connect timeout for provider requests (spec/04 §4).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Builds the shared HTTP client: connect timeout, no overall timeout (streams are long), no
/// proxy for provider calls? Provider calls keep the system proxy; only `web_fetch` ignores
/// proxies (spec/01 §10.4, DECISIONS D-045).
pub fn http_client() -> Result<reqwest::Client, LlmError> {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .map_err(|err| LlmError::Network(err.to_string()))
}

/// Enum-dispatch provider client (spec/04 §1). Dropping the `run` future cancels the request.
#[derive(Clone)]
pub enum ProviderClient {
    OpenAi(openai::OpenAiClient),
    Anthropic(anthropic::AnthropicClient),
    Mock(Arc<demo::DemoProvider>),
    Scripted(Arc<mock::ScriptedProvider>),
}

impl std::fmt::Debug for ProviderClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderClient::OpenAi(_) => f.write_str("ProviderClient::OpenAi"),
            ProviderClient::Anthropic(_) => f.write_str("ProviderClient::Anthropic"),
            ProviderClient::Mock(_) => f.write_str("ProviderClient::Mock"),
            ProviderClient::Scripted(_) => f.write_str("ProviderClient::Scripted"),
        }
    }
}

impl ProviderClient {
    /// Builds the client for a provider row's configuration.
    pub fn from_config(config: ProviderConfig) -> Result<Self, LlmError> {
        match config.protocol {
            Protocol::Openai => Ok(ProviderClient::OpenAi(openai::OpenAiClient::new(config)?)),
            Protocol::Anthropic => Ok(ProviderClient::Anthropic(anthropic::AnthropicClient::new(
                config,
            )?)),
            Protocol::Mock => Ok(ProviderClient::Mock(Arc::new(demo::DemoProvider::new()))),
        }
    }

    /// Streams one assistant turn.
    pub async fn run(
        &self,
        request: &ChatRequest,
        on_event: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<AssistantTurn, LlmError> {
        match self {
            ProviderClient::OpenAi(client) => client.run(request, on_event).await,
            ProviderClient::Anthropic(client) => client.run(request, on_event).await,
            ProviderClient::Mock(provider) => provider.run(request, on_event).await,
            ProviderClient::Scripted(provider) => provider.run(request, on_event).await,
        }
    }

    /// Lists the provider's models (spec/04 §8).
    pub async fn list_models(&self) -> Result<Vec<RemoteModel>, LlmError> {
        match self {
            ProviderClient::OpenAi(client) => client.list_models().await,
            ProviderClient::Anthropic(client) => client.list_models().await,
            ProviderClient::Mock(provider) => provider.list_models().await,
            ProviderClient::Scripted(provider) => provider.list_models().await,
        }
    }
}

/// Re-exported so server code does not need the adapter modules.
pub use anthropic::{ANTHROPIC_VERSION, FALLBACK_BETA};
pub use openai::{build_body as openai_body, estimate_usage};
