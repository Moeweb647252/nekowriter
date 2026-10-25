//! The web search providers (spec/07 §6).
//!
//! Nothing else in the server may reach the network except the web tools (spec/01 §10.5), and a
//! failed request returns a [`WebError`] whose `reason` code the prompt templates word for the
//! model, so a network problem never surfaces as an internal error (D-058).

pub mod search;

use std::time::Duration;

pub use search::{SearchError, SearchFailure, SearchProviderKind, search};

/// The web request timeout of spec/07 §6.
pub const WEB_TIMEOUT: Duration = Duration::from_secs(20);

/// The `web_search` result cap (spec/05 §7: `max_results` max 10).
pub const MAX_RESULTS_CAP: usize = 10;

/// The default number of search results when the tool does not ask for a number
/// (`app_settings["search"].max_results`, spec/02 §4.1).
pub const DEFAULT_RESULTS: usize = 6;

/// The web tooling of one process.
///
/// The Tavily host is fixed by spec/07 §6; this handle carries the one seam a test needs, so no
/// test has to change a production constant.
#[derive(Debug, Default)]
pub struct WebService {
    /// Test-only: replaces the fixed Tavily host so a `wiremock` server can answer for it.
    #[cfg(any(test, feature = "testing"))]
    tavily_base_url: Option<String>,
}

impl WebService {
    pub fn new() -> Self {
        Self::default()
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
}

/// The `User-Agent` every web request carries (spec/07 §6).
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
}

impl WebError {
    /// The `reason` code of a failed search, when it has one.
    pub fn search_reason(&self) -> Option<SearchFailure> {
        match self {
            WebError::Search { reason, .. } => Some(*reason),
            _ => None,
        }
    }

    /// The HTTP status of a failed search, when the failure was a status.
    pub fn status(&self) -> Option<u16> {
        match self {
            WebError::Search { status, .. } => *status,
            WebError::NotConfigured => None,
        }
    }
}
