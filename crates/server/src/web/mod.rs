//! The network-facing tools: web search providers (spec/07 §6) and `web_fetch` (spec/07 §7).
//!
//! Nothing else in the server may reach the network (spec/01 §10.5): `web_search` and `web_fetch`
//! are the only doors, and both return [`WebError`]s whose `reason` codes the prompt templates
//! word for the model (D-058).

pub mod deepseek_fetch;
pub mod fetch;
pub mod search;

use std::time::Duration;

use fetch::FetchError;

pub use fetch::{FetchFailure, FetchedDocument, Fetcher, PageCache, fetch};
pub use search::{SearchFailure, SearchProviderKind, search};

/// Which `web_fetch` backend a call runs on (spec/07 §7, spec/02 §4.1, D-128).
///
/// `Direct` is the default: it is free, local, and produces our own Readability output. `Deepseek`
/// sends the URL as data to the fixed DeepSeek host, which fetches and extracts the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FetchBackend {
    #[default]
    Direct,
    Deepseek,
}

/// One resolved `web_fetch` configuration (spec/07 §7, spec/02 §4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchConfig {
    pub backend: FetchBackend,
    /// Resolved only for the DeepSeek backend, so an unset `env:` key of an unused provider
    /// cannot break the direct fetcher.
    pub deepseek_api_key: Option<String>,
    pub deepseek_base_url: String,
}

impl Default for FetchConfig {
    fn default() -> Self {
        Self {
            backend: FetchBackend::Direct,
            deepseek_api_key: None,
            deepseek_base_url: search::DEEPSEEK_BASE_URL.to_owned(),
        }
    }
}

/// The web tooling of one process: the guarded fetch client and the converted-page cache
/// (spec/07 §7).
///
/// Both live here rather than as globals so a test can build a state whose fetcher resolves a
/// stand-in host, while every production instance is identical.
pub struct WebService {
    /// `None` when the guarded client cannot be built at all (a broken TLS backend): `web_fetch`
    /// then reports a network failure instead of the process refusing to start.
    fetcher: Option<Fetcher>,
    cache: PageCache,
    /// Test-only: replaces the fixed Tavily host so a `wiremock` server can answer for it. The
    /// production address is `https://api.tavily.com` (spec/07 §6) and cannot be changed by the
    /// author; only the tests of this crate override it.
    #[cfg(any(test, feature = "testing"))]
    tavily_base_url: Option<String>,
    /// Test-only: replaces the fixed DeepSeek host, for the same reason as `tavily_base_url`. The
    /// production address is `https://api.deepseek.com` (spec/07 §6).
    #[cfg(any(test, feature = "testing"))]
    deepseek_base_url: Option<String>,
}

impl WebService {
    /// The production service: a real DNS resolver and an empty cache.
    pub fn new() -> Self {
        let fetcher = Fetcher::new()
            .map_err(|error| {
                tracing::error!(error = %error, "cannot build the web_fetch client");
                error
            })
            .ok();
        Self {
            fetcher,
            cache: PageCache::new(),
            #[cfg(any(test, feature = "testing"))]
            tavily_base_url: None,
            #[cfg(any(test, feature = "testing"))]
            deepseek_base_url: None,
        }
    }

    /// Replaces the fetcher (tests point one host name at a local server).
    #[cfg(any(test, feature = "testing"))]
    pub fn with_fetcher(fetcher: Fetcher) -> Self {
        Self {
            fetcher: Some(fetcher),
            cache: PageCache::new(),
            tavily_base_url: None,
            deepseek_base_url: None,
        }
    }

    /// Points the Tavily provider at another host (tests only).
    #[cfg(any(test, feature = "testing"))]
    pub fn with_tavily_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.tavily_base_url = Some(base_url.into());
        self
    }

    /// Points the DeepSeek provider at another host (tests only).
    #[cfg(any(test, feature = "testing"))]
    pub fn with_deepseek_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.deepseek_base_url = Some(base_url.into());
        self
    }

    /// The Tavily host this process actually calls.
    pub fn tavily_base_url(&self) -> &str {
        #[cfg(any(test, feature = "testing"))]
        if let Some(base) = &self.tavily_base_url {
            return base;
        }
        search::TAVILY_BASE_URL
    }

    /// The DeepSeek host this process actually calls.
    pub fn deepseek_base_url(&self) -> &str {
        #[cfg(any(test, feature = "testing"))]
        if let Some(base) = &self.deepseek_base_url {
            return base;
        }
        search::DEEPSEEK_BASE_URL
    }

    /// Fetches and extracts one page through the configured backend (spec/07 §7).
    ///
    /// Both backends go through the same [`PageCache`], so `offset` reads and LRU behaviour are
    /// identical whichever backend ran (D-128). The tool calls this one entry point and does not
    /// know which backend served the page.
    pub async fn fetch_document(
        &self,
        config: &FetchConfig,
        url: &str,
    ) -> Result<FetchedDocument, FetchError> {
        if let Some(hit) = self.cache.get(url) {
            return Ok(hit);
        }
        let document = match config.backend {
            FetchBackend::Direct => self.fetch_direct(url).await?,
            FetchBackend::Deepseek => self.fetch_deepseek(config, url).await?,
        };
        // Keyed by the requested URL, exactly like the free function (spec/07 §7 step 7).
        self.cache.put(url, document.clone());
        Ok(document)
    }

    /// The direct backend: the guarded client of spec/07 §7 steps 1–6.
    async fn fetch_direct(&self, url: &str) -> Result<FetchedDocument, FetchError> {
        let Some(fetcher) = &self.fetcher else {
            return Err(FetchError::failed(FetchFailure::Network, url));
        };
        fetcher.fetch(url).await
    }

    /// The DeepSeek backend: `POST {base}/v1/tools/fetch` on the fixed host.
    async fn fetch_deepseek(
        &self,
        config: &FetchConfig,
        url: &str,
    ) -> Result<FetchedDocument, FetchError> {
        let Some(key) = config
            .deepseek_api_key
            .as_deref()
            .filter(|key| !key.is_empty())
        else {
            // Validation refuses to save `fetch_via: deepseek` without a key, so this is a
            // defensive network reason rather than an internal error.
            return Err(FetchError::failed(FetchFailure::Network, url));
        };
        let client =
            search::shared_client().map_err(|_| FetchError::failed(FetchFailure::Network, url))?;
        deepseek_fetch::DeepseekFetcher::new(client, &config.deepseek_base_url, key)
            .fetch(url)
            .await
    }

    /// The guarded client, when it could be built.
    pub fn fetcher(&self) -> Option<&Fetcher> {
        self.fetcher.as_ref()
    }

    pub fn cache(&self) -> &PageCache {
        &self.cache
    }
}

impl Default for WebService {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for WebService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebService").finish_non_exhaustive()
    }
}

/// The web request timeout of spec/07 §6 and §7.
pub const WEB_TIMEOUT: Duration = Duration::from_secs(20);

/// The `web_search` result cap (spec/05 §7: `max_results` max 10).
pub const MAX_RESULTS_CAP: usize = 10;

/// The default number of search results when the tool does not ask for a number
/// (`app_settings["search"].max_results`, spec/02 §4.1).
pub const DEFAULT_RESULTS: usize = 6;

/// The `User-Agent` every web request carries (spec/07 §7).
pub fn user_agent() -> String {
    format!(
        "NekoWriter/{} (+local research assistant)",
        env!("CARGO_PKG_VERSION")
    )
}

/// One web search hit (spec/07 §6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub published: Option<String>,
}

/// A failed web call (spec/07 §6–§7). Every variant carries the `reason` code the model-facing
/// template switches on, so a network problem never surfaces as an internal error (D-058).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WebError {
    /// No provider is configured (`error.search_not_configured`).
    #[error("no web search provider is configured")]
    NotConfigured,
    /// A `web_search` request failed; `status` is set for the `status` reason.
    #[error("web search failed: {}", .reason.as_str())]
    Search {
        reason: SearchFailure,
        status: Option<u16>,
    },
    /// A `web_fetch` call failed (spec/07 §7 step 9).
    #[error("web fetch failed: {}", .reason.as_str())]
    Fetch {
        reason: FetchFailure,
        status: Option<u16>,
        url: String,
    },
    /// `web_fetch` got a content type the MVP cannot convert (spec/07 §7 step 4).
    #[error("unsupported content type {content_type}")]
    UnsupportedContentType { content_type: String, url: String },
}

impl WebError {
    /// The `reason` code of a failed search, when it has one.
    pub fn search_reason(&self) -> Option<SearchFailure> {
        match self {
            WebError::Search { reason, .. } => Some(*reason),
            _ => None,
        }
    }

    /// The HTTP status of a failed call, when the failure was a status.
    pub fn status(&self) -> Option<u16> {
        match self {
            WebError::Search { status, .. } | WebError::Fetch { status, .. } => *status,
            _ => None,
        }
    }
}

/// The URL cap of the direct fetcher (spec/07 §7 step 1).
const MAX_URL_CHARS: usize = 2_048;

/// The URL cap of the DeepSeek Fetch endpoint (spec/07 §7, D-128).
const DEEPSEEK_MAX_URL_CHARS: usize = 8_192;

/// True when the URL uses http or https and carries no credentials (spec/07 §7 step 1).
pub fn check_url(raw: &str) -> Result<url::Url, FetchFailure> {
    check_url_capped(raw, MAX_URL_CHARS)
}

/// The shapes check of the DeepSeek backend: the same rules as [`check_url`] with the endpoint's
/// larger cap. It deliberately does **not** check the address class: our process never resolves or
/// connects to this URL, so a loopback target is not reachable from here (D-128).
pub fn check_deepseek_url(raw: &str) -> Result<url::Url, FetchFailure> {
    check_url_capped(raw, DEEPSEEK_MAX_URL_CHARS)
}

fn check_url_capped(raw: &str, max_chars: usize) -> Result<url::Url, FetchFailure> {
    if raw.chars().count() > max_chars {
        return Err(FetchFailure::InvalidUrl);
    }
    let parsed = url::Url::parse(raw).map_err(|_| FetchFailure::InvalidUrl)?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(FetchFailure::InvalidUrl);
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(FetchFailure::InvalidUrl);
    }
    if parsed.host_str().is_none() {
        return Err(FetchFailure::InvalidUrl);
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_http_urls_without_credentials_pass() {
        assert!(check_url("https://example.com/a").is_ok());
        assert!(check_url("http://example.com").is_ok());
        for rejected in [
            "ftp://example.com",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,x",
            "not a url",
            "https://user:secret@example.com/",
            "https://user@example.com/",
        ] {
            assert_eq!(
                check_url(rejected),
                Err(FetchFailure::InvalidUrl),
                "{rejected}"
            );
        }
        let long = format!("https://example.com/{}", "a".repeat(2_100));
        assert_eq!(check_url(&long), Err(FetchFailure::InvalidUrl));
    }
}
