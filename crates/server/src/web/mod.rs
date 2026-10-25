//! The network-facing tools: web search providers (spec/07 §6) and `web_fetch` (spec/07 §7).
//!
//! Nothing else in the server may reach the network (spec/01 §10.5): `web_search` and `web_fetch`
//! are the only doors, and both return [`WebError`]s whose `reason` codes the prompt templates
//! word for the model (D-058).

pub mod fetch;
pub mod search;

use std::time::Duration;

pub use fetch::{FetchFailure, FetchedDocument, Fetcher, PageCache, fetch};
pub use search::{SearchFailure, SearchProviderKind, search};

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
        }
    }

    /// Replaces the fetcher (tests point one host name at a local server).
    #[cfg(any(test, feature = "testing"))]
    pub fn with_fetcher(fetcher: Fetcher) -> Self {
        Self {
            fetcher: Some(fetcher),
            cache: PageCache::new(),
            tavily_base_url: None,
        }
    }

    /// Points the Tavily provider at another host (tests only).
    #[cfg(any(test, feature = "testing"))]
    pub fn with_tavily_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.tavily_base_url = Some(base_url.into());
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

/// True when the URL uses http or https and carries no credentials (spec/07 §7 step 1).
pub fn check_url(raw: &str) -> Result<url::Url, FetchFailure> {
    const MAX_URL_CHARS: usize = 2_048;
    if raw.chars().count() > MAX_URL_CHARS {
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
