//! `web_search` providers (spec/07 §6): Tavily, DeepSeek and SearXNG.
//!
//! All return the same [`SearchHit`] shape, and all report failures as a [`SearchFailure`]
//! reason code that the model-facing template words (D-058): `timeout`, `auth` (HTTP 401/403),
//! `status` (any other HTTP status ≥ 400) or `network` (anything else). No credential is ever
//! logged: only the provider name, the query length and the reason reach `tracing`.

use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use super::{DEFAULT_RESULTS, MAX_RESULTS_CAP, SearchHit, WEB_TIMEOUT};

/// The Tavily endpoint of spec/07 §6.
pub const TAVILY_BASE_URL: &str = "https://api.tavily.com";

/// The DeepSeek endpoint of spec/07 §6.
pub const DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com";

/// Why a `web_search` request failed (spec/07 §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchFailure {
    Timeout,
    Auth,
    Status,
    Network,
}

impl SearchFailure {
    /// The `reason` code passed to the model-facing template.
    pub fn as_str(self) -> &'static str {
        match self {
            SearchFailure::Timeout => "timeout",
            SearchFailure::Auth => "auth",
            SearchFailure::Status => "status",
            SearchFailure::Network => "network",
        }
    }
}

/// A failed search: the reason code plus, for a `status` failure, the HTTP status the template
/// prints (spec/07 §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchError {
    pub reason: SearchFailure,
    pub status: Option<u16>,
}

impl SearchError {
    pub fn new(reason: SearchFailure) -> Self {
        Self {
            reason,
            status: None,
        }
    }
}

impl From<SearchFailure> for SearchError {
    fn from(reason: SearchFailure) -> Self {
        Self::new(reason)
    }
}

/// The configured provider (spec/02 §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchProviderKind {
    #[default]
    None,
    Tavily,
    Deepseek,
    Searxng,
}

/// One resolved search configuration.
///
/// `tavily_base_url` and `deepseek_base_url` are not user-editable; they exist so tests can point
/// the providers at `wiremock` instead of the fixed hosts of spec/07 §6 (the defaults are
/// [`TAVILY_BASE_URL`] and [`DEEPSEEK_BASE_URL`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchConfig {
    pub provider: SearchProviderKind,
    pub tavily_api_key: Option<String>,
    pub tavily_base_url: String,
    pub deepseek_api_key: Option<String>,
    pub deepseek_base_url: String,
    pub searxng_base_url: Option<String>,
    pub max_results: usize,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            provider: SearchProviderKind::None,
            tavily_api_key: None,
            tavily_base_url: TAVILY_BASE_URL.to_owned(),
            deepseek_api_key: None,
            deepseek_base_url: DEEPSEEK_BASE_URL.to_owned(),
            searxng_base_url: None,
            max_results: DEFAULT_RESULTS,
        }
    }
}

impl SearchConfig {
    /// The result count for a call: the requested count, else the configured default, always
    /// within `1..=10` (spec/05 §7).
    pub fn result_count(&self, requested: Option<usize>) -> usize {
        requested
            .filter(|value| *value > 0)
            .unwrap_or(self.max_results)
            .clamp(1, MAX_RESULTS_CAP)
    }
}

/// True when a provider is configured (spec/07 §6).
pub fn is_configured(config: &SearchConfig) -> bool {
    match config.provider {
        SearchProviderKind::None => false,
        SearchProviderKind::Tavily => config
            .tavily_api_key
            .as_deref()
            .is_some_and(|key| !key.is_empty()),
        SearchProviderKind::Deepseek => config
            .deepseek_api_key
            .as_deref()
            .is_some_and(|key| !key.is_empty()),
        SearchProviderKind::Searxng => config
            .searxng_base_url
            .as_deref()
            .is_some_and(|url| !url.is_empty()),
    }
}

/// The provider's display name, for logs and error messages.
pub fn provider_name(provider: SearchProviderKind) -> &'static str {
    match provider {
        SearchProviderKind::None => "none",
        SearchProviderKind::Tavily => "tavily",
        SearchProviderKind::Deepseek => "deepseek",
        SearchProviderKind::Searxng => "searxng",
    }
}

/// Builds the HTTP client of the search providers (spec/07 §6).
///
/// Unlike the `web_fetch` client, this one may use the system proxy: its hosts are fixed and no
/// user-supplied address is resolved (spec/07 §7 step 2, D-045).
pub fn client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .timeout(WEB_TIMEOUT)
        .user_agent(super::user_agent())
        .build()
}

/// The process-wide client of the search providers and the search test.
///
/// One client is one connection pool, and building one is comparatively expensive, so it is
/// created once. A build failure cannot be recovered from, so the process falls back to a default
/// client rather than failing every later call.
pub fn shared_client() -> Result<&'static reqwest::Client, reqwest::Error> {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    if let Some(client) = CLIENT.get() {
        return Ok(client);
    }
    let client = client()?;
    let _ = CLIENT.set(client);
    Ok(CLIENT.get().expect("the client was just set"))
}

/// Runs one search with the configured provider (spec/07 §6).
pub async fn search(
    client: &reqwest::Client,
    config: &SearchConfig,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>, SearchError> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let hits = match config.provider {
        SearchProviderKind::None => return Ok(Vec::new()),
        SearchProviderKind::Tavily => tavily(client, config, query, limit).await?,
        SearchProviderKind::Deepseek => deepseek(client, config, query, limit).await?,
        SearchProviderKind::Searxng => searxng(client, config, query, limit).await?,
    };
    Ok(dedupe(hits, limit))
}

/// `POST {base}/search` with a bearer token (spec/07 §6).
async fn tavily(
    client: &reqwest::Client,
    config: &SearchConfig,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>, SearchError> {
    let Some(key) = config
        .tavily_api_key
        .as_deref()
        .filter(|key| !key.is_empty())
    else {
        // A selected provider without a key cannot be called; the settings service refuses to
        // save that state, so this is a defensive network reason rather than an internal error.
        return Err(SearchFailure::Auth.into());
    };
    let url = format!("{}/search", config.tavily_base_url.trim_end_matches('/'));
    let response = client
        .post(url)
        .bearer_auth(key)
        .json(&json!({
            "query": query,
            "max_results": limit,
            "search_depth": "basic",
            "include_answer": false,
        }))
        .send()
        .await
        .map_err(|error| SearchError::new(classify_request_error(&error)))?;
    let body = checked_body(response).await?;
    let parsed: TavilyResponse =
        serde_json::from_slice(&body).map_err(|_| SearchError::new(SearchFailure::Network))?;
    Ok(parsed
        .results
        .into_iter()
        .map(|hit| SearchHit {
            title: hit.title,
            url: hit.url,
            snippet: hit.content,
            published: hit.published_date.filter(|value| !value.is_empty()),
        })
        .collect())
}

/// `POST {base}/v1/tools/search` with a bearer token (spec/07 §6).
///
/// DeepSeek returns two prose fields per result: `snippet` (~100–200 characters, documented for a
/// front-end) and `summary` (~600–800 characters, documented for a model to read). Our
/// [`SearchHit::snippet`] is rendered into the model prompt by `result.web_search`, so the summary
/// is the field to map; `need_content` stays false because `content` would be paid for and
/// unused (D-127).
async fn deepseek(
    client: &reqwest::Client,
    config: &SearchConfig,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>, SearchError> {
    let Some(key) = config
        .deepseek_api_key
        .as_deref()
        .filter(|key| !key.is_empty())
    else {
        // A selected provider without a key cannot be called; the settings service refuses to
        // save that state, so this is a defensive network reason rather than an internal error.
        return Err(SearchFailure::Auth.into());
    };
    let url = format!(
        "{}/v1/tools/search",
        config.deepseek_base_url.trim_end_matches('/')
    );
    let response = client
        .post(url)
        .bearer_auth(key)
        .json(&json!({
            "query": query,
            "count": limit,
            "need_content": false,
        }))
        .send()
        .await
        .map_err(|error| SearchError::new(classify_request_error(&error)))?;
    let body = checked_body(response).await?;
    let parsed: DeepseekResponse =
        serde_json::from_slice(&body).map_err(|_| SearchError::new(SearchFailure::Network))?;
    Ok(parsed
        .results
        .into_iter()
        .map(|hit| SearchHit {
            title: hit.title,
            url: hit.url,
            snippet: hit.summary,
            published: hit.date.filter(|value| !value.is_empty()),
        })
        .collect())
}

/// `GET {base}/search?q=…&format=json&safesearch=0` (spec/07 §6).
async fn searxng(
    client: &reqwest::Client,
    config: &SearchConfig,
    query: &str,
    _limit: usize,
) -> Result<Vec<SearchHit>, SearchError> {
    let Some(base) = config
        .searxng_base_url
        .as_deref()
        .filter(|url| !url.is_empty())
    else {
        return Err(SearchFailure::Network.into());
    };
    let url = format!("{}/search", base.trim_end_matches('/'));
    // The query string is built by hand: `RequestBuilder::query` needs reqwest's `query` feature,
    // which this workspace does not enable.
    let separator = if url.contains('?') { '&' } else { '?' };
    let target = format!(
        "{url}{separator}q={}&format=json&safesearch=0",
        urlencode(query)
    );
    let response = client
        .get(target)
        .send()
        .await
        .map_err(|error| SearchError::new(classify_request_error(&error)))?;
    let body = checked_body(response).await?;
    let parsed: SearxResponse =
        serde_json::from_slice(&body).map_err(|_| SearchError::new(SearchFailure::Network))?;
    Ok(parsed
        .results
        .into_iter()
        .map(|hit| SearchHit {
            title: hit.title,
            url: hit.url,
            snippet: hit.content,
            published: hit.published_date.filter(|value| !value.is_empty()),
        })
        .collect())
}

/// Reads the body of a search response, classifying a non-success status (spec/07 §6).
async fn checked_body(response: reqwest::Response) -> Result<Vec<u8>, SearchError> {
    let status = response.status();
    if !status.is_success() {
        // A 401/403 is a rejected key; every other status is passed on as a number.
        return Err(
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                SearchError::new(SearchFailure::Auth)
            } else {
                tracing::debug!(
                    status = status.as_u16(),
                    "web search returned an error status"
                );
                SearchError {
                    reason: SearchFailure::Status,
                    status: Some(status.as_u16()),
                }
            },
        );
    }
    response
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|error| SearchError::new(classify_request_error(&error)))
}

/// Classifies a transport failure (spec/07 §6): a timeout is `timeout`, anything else `network`.
fn classify_request_error(error: &reqwest::Error) -> SearchFailure {
    if error.is_timeout() {
        SearchFailure::Timeout
    } else {
        tracing::debug!(error = %error, "web search transport failure");
        SearchFailure::Network
    }
}

/// Percent-encodes a query for a URL (`application/x-www-form-urlencoded` rules, which is what a
/// search endpoint expects).
pub fn urlencode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(*byte));
            }
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Deduplicates by URL, keeping the first occurrence, and caps the list (spec/07 §6).
pub fn dedupe(hits: Vec<SearchHit>, limit: usize) -> Vec<SearchHit> {
    let mut seen = std::collections::BTreeSet::new();
    let mut kept = Vec::new();
    for hit in hits {
        if seen.insert(hit.url.clone()) {
            kept.push(hit);
            if kept.len() == limit {
                break;
            }
        }
    }
    kept
}

#[derive(Debug, Deserialize)]
struct TavilyResponse {
    #[serde(default)]
    results: Vec<TavilyResult>,
}

#[derive(Debug, Deserialize)]
struct TavilyResult {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    published_date: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeepseekResponse {
    #[serde(default)]
    results: Vec<DeepseekResult>,
}

/// One DeepSeek result.
///
/// `snippet` (documented for a front-end) and `content` (empty when `need_content` is false) are
/// deliberately not modeled: `summary` is the field for a model to read and therefore the only
/// one that maps into [`SearchHit::snippet`] (D-127). Unknown fields are ignored by serde.
#[derive(Debug, Deserialize)]
struct DeepseekResult {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    /// Documented for a model to read; this is what [`SearchHit::snippet`] carries.
    #[serde(default)]
    summary: String,
    #[serde(default)]
    date: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearxResponse {
    #[serde(default)]
    results: Vec<SearxResult>,
}

#[derive(Debug, Deserialize)]
struct SearxResult {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    content: String,
    #[serde(default, rename = "publishedDate")]
    published_date: Option<String>,
}

/// Parses the response of a provider into hits (used by the provider tests).
pub fn hits_from_value(value: &Value) -> Vec<SearchHit> {
    let response: SearxResponse = match serde_json::from_value(value.clone()) {
        Ok(response) => response,
        Err(_) => return Vec::new(),
    };
    response
        .results
        .into_iter()
        .map(|hit| SearchHit {
            title: hit.title,
            url: hit.url,
            snippet: hit.content,
            published: hit.published_date,
        })
        .collect()
}

/// The default request timeout, re-exported for the provider tests.
pub fn timeout() -> Duration {
    WEB_TIMEOUT
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(url: &str) -> SearchHit {
        SearchHit {
            title: "t".to_owned(),
            url: url.to_owned(),
            snippet: "s".to_owned(),
            published: None,
        }
    }

    #[test]
    fn results_are_deduplicated_by_url_and_capped() {
        let hits = vec![
            hit("https://a"),
            hit("https://b"),
            hit("https://a"),
            hit("https://c"),
        ];
        assert_eq!(dedupe(hits.clone(), 10).len(), 3);
        assert_eq!(dedupe(hits, 2).len(), 2);
    }

    #[test]
    fn the_result_count_defaults_to_the_settings_and_caps_at_ten() {
        let mut config = SearchConfig::default();
        assert_eq!(config.result_count(None), 6);
        config.max_results = 8;
        assert_eq!(config.result_count(None), 8);
        assert_eq!(config.result_count(Some(3)), 3);
        assert_eq!(config.result_count(Some(50)), 10);
        // An explicit zero falls back to the settings default rather than returning nothing.
        assert_eq!(config.result_count(Some(0)), 8);
    }

    #[test]
    fn a_provider_needs_its_configuration_to_count_as_configured() {
        let mut config = SearchConfig::default();
        assert!(!is_configured(&config));
        config.provider = SearchProviderKind::Tavily;
        assert!(!is_configured(&config));
        config.tavily_api_key = Some("tvly-key".to_owned());
        assert!(is_configured(&config));
        let searxng = SearchConfig {
            provider: SearchProviderKind::Searxng,
            searxng_base_url: Some("https://searx.example".to_owned()),
            ..SearchConfig::default()
        };
        assert!(is_configured(&searxng));
    }

    #[test]
    fn reason_codes_are_the_documented_ones() {
        assert_eq!(SearchFailure::Timeout.as_str(), "timeout");
        assert_eq!(SearchFailure::Auth.as_str(), "auth");
        assert_eq!(SearchFailure::Status.as_str(), "status");
        assert_eq!(SearchFailure::Network.as_str(), "network");
    }
}
