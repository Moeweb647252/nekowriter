//! Error classification and retry policy (spec/04 §4, §6).

use std::time::Duration;

use thiserror::Error;

#[derive(Debug, Error, Clone)]
pub enum LlmError {
    #[error("network error: {0}")]
    Network(String),
    #[error("timeout: {0}")]
    Timeout(&'static str),
    #[error("rate limited")]
    RateLimited { retry_after: Option<Duration> },
    #[error("provider overloaded")]
    Overloaded,
    #[error("server error {0}: {1}")]
    Server(u16, String),
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("context overflow: {0}")]
    ContextOverflow(String),
    #[error("provider error: {0}")]
    Provider(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("cancelled")]
    Cancelled,
}

impl LlmError {
    /// Only transport-ish failures are worth retrying (spec/04 §6).
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            LlmError::Network(_)
                | LlmError::Timeout(_)
                | LlmError::RateLimited { .. }
                | LlmError::Overloaded
                | LlmError::Server(_, _)
        )
    }

    /// Attaches the provider request id to the message-bearing variants, so error reports and
    /// logs carry it (spec/04 §4).
    pub fn with_request_id(self, request_id: Option<&str>) -> Self {
        let Some(request_id) = request_id else {
            return self;
        };
        let suffix = format!(" [request-id: {request_id}]");
        match self {
            LlmError::Network(message) => LlmError::Network(message + &suffix),
            LlmError::Server(status, message) => LlmError::Server(status, message + &suffix),
            LlmError::Auth(message) => LlmError::Auth(message + &suffix),
            LlmError::BadRequest(message) => LlmError::BadRequest(message + &suffix),
            LlmError::NotFound(message) => LlmError::NotFound(message + &suffix),
            LlmError::ContextOverflow(message) => LlmError::ContextOverflow(message + &suffix),
            LlmError::Provider(message) => LlmError::Provider(message + &suffix),
            LlmError::Protocol(message) => LlmError::Protocol(message + &suffix),
            other => other,
        }
    }

    /// Maps an HTTP status plus the response body to an `LlmError` (spec/04 §4).
    pub fn from_status(status: u16, body: &str, retry_after: Option<Duration>) -> Self {
        match status {
            400 | 413 => {
                if is_context_overflow(body) {
                    LlmError::ContextOverflow(body.to_owned())
                } else {
                    LlmError::BadRequest(body.to_owned())
                }
            }
            401 | 403 => LlmError::Auth(body.to_owned()),
            404 => LlmError::NotFound(body.to_owned()),
            408 => LlmError::Timeout("request timeout"),
            409 => LlmError::Server(409, body.to_owned()),
            429 => LlmError::RateLimited { retry_after },
            529 => LlmError::Overloaded,
            status if (500..600).contains(&status) => LlmError::Server(status, body.to_owned()),
            _ => LlmError::Provider(format!("HTTP {status}: {body}")),
        }
    }
}

/// True when a 400 body names a context-length problem (spec/04 §4).
pub fn is_context_overflow(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    [
        "context_length_exceeded",
        "maximum context length",
        "prompt is too long",
        "context length",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

/// Parses `Retry-After` as seconds or an HTTP date (spec/04 §4).
pub fn parse_retry_after(value: &str, now: chrono::DateTime<chrono::Utc>) -> Option<Duration> {
    let trimmed = value.trim();
    if let Ok(seconds) = trimmed.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let parsed = chrono::DateTime::parse_from_rfc2822(trimmed)
        .or_else(|_| chrono::DateTime::parse_from_rfc3339(trimmed))
        .ok()?;
    let delta = parsed.with_timezone(&chrono::Utc) - now;
    let millis = delta.num_milliseconds();
    if millis <= 0 {
        Some(Duration::ZERO)
    } else {
        Some(Duration::from_millis(millis as u64))
    }
}

/// Total attempts including the first try (spec/04 §6).
pub const MAX_ATTEMPTS: u32 = 4;

/// Documented backoff ladder: 2 s → 8 s → 30 s (spec/04 §6).
const BACKOFF_SECS: [u64; 3] = [2, 8, 30];

/// `Retry-After` values above this are capped (spec/04 §6).
const MAX_RETRY_AFTER_SECS: u64 = 120;

/// Jitter applied to a backoff: ±20 %.
pub fn jitter(seconds: f64, seed: u64) -> f64 {
    // Deterministic pseudo-random factor in [0.8, 1.2) so tests stay reproducible.
    let unit = (seed % 1000) as f64 / 1000.0;
    seconds * (0.8 + 0.4 * unit)
}

/// The wait before retry number `attempt` (1-based: the first retry is attempt 1).
///
/// A longer `retry_after` wins, capped at 120 s. Returns `None` when no retry may happen.
pub fn backoff_delay(attempt: u32, retry_after: Option<Duration>, seed: u64) -> Option<Duration> {
    if attempt >= MAX_ATTEMPTS {
        return None;
    }
    let idx = (attempt.saturating_sub(1)) as usize;
    let base = *BACKOFF_SECS.get(idx).unwrap_or(&30);
    let jittered = jitter(base as f64, seed);
    let mut delay = Duration::from_secs_f64(jittered);
    if let Some(after) = retry_after {
        let capped = after.min(Duration::from_secs(MAX_RETRY_AFTER_SECS));
        if capped > delay {
            delay = capped;
        }
    }
    Some(delay)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_classification() {
        assert!(matches!(
            LlmError::from_status(401, "unauthorized", None),
            LlmError::Auth(_)
        ));
        assert!(matches!(
            LlmError::from_status(403, "forbidden", None),
            LlmError::Auth(_)
        ));
        assert!(matches!(
            LlmError::from_status(404, "not found", None),
            LlmError::NotFound(_)
        ));
        assert!(matches!(
            LlmError::from_status(408, "timeout", None),
            LlmError::Timeout(_)
        ));
        assert!(matches!(
            LlmError::from_status(409, "conflict", None),
            LlmError::Server(409, _)
        ));
        assert!(matches!(
            LlmError::from_status(429, "slow down", Some(Duration::from_secs(7))),
            LlmError::RateLimited {
                retry_after: Some(d)
            } if d == Duration::from_secs(7)
        ));
        assert!(matches!(
            LlmError::from_status(529, "overloaded", None),
            LlmError::Overloaded
        ));
        assert!(matches!(
            LlmError::from_status(500, "boom", None),
            LlmError::Server(500, _)
        ));
        assert!(matches!(
            LlmError::from_status(400, "prompt is too long: 300000 tokens", None),
            LlmError::ContextOverflow(_)
        ));
        assert!(matches!(
            LlmError::from_status(400, "This model's maximum context length is 128000", None),
            LlmError::ContextOverflow(_)
        ));
        assert!(matches!(
            LlmError::from_status(400, "invalid tool schema", None),
            LlmError::BadRequest(_)
        ));
        assert_eq!(
            LlmError::from_status(418, "teapot", None).to_string(),
            "provider error: HTTP 418: teapot"
        );
    }

    #[test]
    fn retryable_classification() {
        assert!(LlmError::Network("reset".into()).retryable());
        assert!(LlmError::Timeout("idle").retryable());
        assert!(LlmError::RateLimited { retry_after: None }.retryable());
        assert!(LlmError::Overloaded.retryable());
        assert!(LlmError::Server(500, "boom".into()).retryable());
        assert!(!LlmError::Auth("bad key".into()).retryable());
        assert!(!LlmError::BadRequest("bad".into()).retryable());
        assert!(!LlmError::ContextOverflow("long".into()).retryable());
        assert!(!LlmError::Cancelled.retryable());
    }

    #[test]
    fn retry_after_seconds_and_dates() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .expect("date")
            .with_timezone(&chrono::Utc);
        assert_eq!(parse_retry_after("12", now), Some(Duration::from_secs(12)));
        assert_eq!(
            parse_retry_after("Thu, 01 Jan 2026 00:00:30 GMT", now),
            Some(Duration::from_secs(30))
        );
        // A past date means "retry now".
        assert_eq!(
            parse_retry_after("Wed, 31 Dec 2025 23:00:00 GMT", now),
            Some(Duration::ZERO)
        );
        assert_eq!(parse_retry_after("not a date", now), None);
    }

    #[test]
    fn backoff_ladder_and_cap() {
        for attempt in 1..MAX_ATTEMPTS {
            let delay = backoff_delay(attempt, None, 0).expect("retry allowed");
            assert!(delay >= Duration::from_millis(1600));
        }
        assert!(backoff_delay(MAX_ATTEMPTS, None, 0).is_none());
        let long = backoff_delay(1, Some(Duration::from_secs(600)), 0).expect("retry");
        assert_eq!(long, Duration::from_secs(MAX_RETRY_AFTER_SECS));
        // retry_after shorter than the backoff does not shrink it.
        let short = backoff_delay(3, Some(Duration::from_secs(1)), 0).expect("retry");
        assert!(short >= Duration::from_secs(24));
    }
}
