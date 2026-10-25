//! The DeepSeek `web_fetch` backend (spec/07 §7, D-128).
//!
//! Unlike the direct fetcher, this one never resolves or connects to the requested host: the URL
//! travels as JSON to the fixed `api.deepseek.com` host, which fetches and extracts the page. The
//! URL is still shape-checked locally (http/https, a host, no userinfo, ≤8,192 characters), so a
//! malformed URL costs no request; the private-address check is deliberately not applied because
//! this process never reaches that address.

use serde::Deserialize;
use serde_json::json;

use super::fetch::{FetchError, FetchFailure, FetchedDocument};

/// The DeepSeek Fetch endpoint's URL cap.
pub const MAX_URL_CHARS: usize = 8_192;

/// One DeepSeek fetch response.
#[derive(Debug, Deserialize)]
struct FetchResponse {
    #[serde(default)]
    title: String,
    #[serde(default)]
    content: String,
}

/// `POST {base}/v1/tools/fetch` (spec/07 §7, D-128).
pub struct DeepseekFetcher<'a> {
    client: &'a reqwest::Client,
    base_url: &'a str,
    api_key: &'a str,
}

impl<'a> DeepseekFetcher<'a> {
    pub fn new(client: &'a reqwest::Client, base_url: &'a str, api_key: &'a str) -> Self {
        Self {
            client,
            base_url,
            api_key,
        }
    }

    /// Fetches and extracts one page through DeepSeek.
    pub async fn fetch(&self, raw_url: &str) -> Result<FetchedDocument, FetchError> {
        // Shape only: the address class is not checked (the request goes to the fixed host).
        super::check_deepseek_url(raw_url).map_err(|reason| FetchError::failed(reason, raw_url))?;
        let endpoint = format!("{}/v1/tools/fetch", self.base_url.trim_end_matches('/'));
        let response = self
            .client
            .post(endpoint)
            .bearer_auth(self.api_key)
            .json(&json!({ "url": raw_url }))
            .send()
            .await
            .map_err(|error| FetchError::failed(classify_transport(&error), raw_url))?;

        let status = response.status();
        if !status.is_success() {
            let failure = classify_status(status);
            tracing::debug!(
                status = status.as_u16(),
                "DeepSeek fetch returned an error status"
            );
            return Err(match failure {
                // 408/504 and 413 are reported by reason, like the direct fetcher does for its
                // own timeouts and body cap; every other failure carries the number.
                FetchFailure::Timeout | FetchFailure::TooLarge => {
                    FetchError::failed(failure, raw_url)
                }
                _ => FetchError::Failed {
                    reason: FetchFailure::Status,
                    status: Some(status.as_u16()),
                    url: raw_url.to_owned(),
                },
            });
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| FetchError::failed(classify_transport(&error), raw_url))?;
        let parsed: FetchResponse = serde_json::from_slice(&bytes)
            .map_err(|_| FetchError::failed(FetchFailure::Network, raw_url))?;
        if parsed.content.trim().is_empty() {
            // A 200 without a body is a successful answer with nothing usable in it. There is no
            // HTTP status to carry, so it is `network` (the reason the template already words for
            // "try another source"); D-128 records the choice.
            return Err(FetchError::failed(FetchFailure::Network, raw_url));
        }
        Ok(FetchedDocument {
            // DeepSeek reports neither the final URL nor redirects; the requested URL is what the
            // tool names in its result (D-128).
            url: raw_url.to_owned(),
            title: parsed.title,
            markdown: parsed.content,
            cached: false,
        })
    }
}

/// Classifies a transport failure: a timeout is `timeout`, anything else `network`.
fn classify_transport(error: &reqwest::Error) -> FetchFailure {
    if error.is_timeout() {
        FetchFailure::Timeout
    } else {
        FetchFailure::Network
    }
}

/// Maps the endpoint's error statuses onto the `web_fetch` reason vocabulary (spec/07 §7 step 9).
fn classify_status(status: reqwest::StatusCode) -> FetchFailure {
    match status.as_u16() {
        408 | 504 => FetchFailure::Timeout,
        413 => FetchFailure::TooLarge,
        // 401/402/403/404/429 and every 5xx keep their number: 403 (policy refusal) and 402
        // (insufficient balance) are diagnostics the author needs to see.
        _ => FetchFailure::Status,
    }
}
