//! Error classification and retry policy tests (spec/04 §4, §6, §9).

use std::time::Duration;

use nekowriter_llm::retry::{
    LlmError, MAX_ATTEMPTS, backoff_delay, is_context_overflow, parse_retry_after,
};
use pretty_assertions::assert_eq;

#[test]
fn status_mapping_table() {
    // 401/403 → Auth
    assert!(matches!(
        LlmError::from_status(401, "unauthorized", None),
        LlmError::Auth(_)
    ));
    assert!(matches!(
        LlmError::from_status(403, "forbidden", None),
        LlmError::Auth(_)
    ));
    // 404 → NotFound
    assert!(matches!(
        LlmError::from_status(404, "model not found", None),
        LlmError::NotFound(_)
    ));
    // 408 → Timeout
    assert!(matches!(
        LlmError::from_status(408, "request timeout", None),
        LlmError::Timeout(_)
    ));
    // 409 → Server(409), a transient conflict
    assert!(matches!(
        LlmError::from_status(409, "conflict", None),
        LlmError::Server(409, _)
    ));
    // 429 → RateLimited with the parsed hint
    match LlmError::from_status(429, "slow down", Some(Duration::from_secs(7))) {
        LlmError::RateLimited { retry_after } => {
            assert_eq!(retry_after, Some(Duration::from_secs(7)));
        }
        other => panic!("unexpected {other:?}"),
    }
    // 529 → Overloaded
    assert!(matches!(
        LlmError::from_status(529, "overloaded", None),
        LlmError::Overloaded
    ));
    // 5xx → Server
    assert!(matches!(
        LlmError::from_status(500, "boom", None),
        LlmError::Server(500, _)
    ));
    assert!(matches!(
        LlmError::from_status(503, "unavailable", None),
        LlmError::Server(503, _)
    ));
    // 400/413 → ContextOverflow when the text names a length problem, else BadRequest
    assert!(matches!(
        LlmError::from_status(
            400,
            "This model's maximum context length is 128000 tokens",
            None
        ),
        LlmError::ContextOverflow(_)
    ));
    assert!(matches!(
        LlmError::from_status(413, "prompt is too long: 300000 tokens", None),
        LlmError::ContextOverflow(_)
    ));
    assert!(matches!(
        LlmError::from_status(400, "invalid tool schema", None),
        LlmError::BadRequest(_)
    ));
    // Anything else is a generic provider error.
    assert!(matches!(
        LlmError::from_status(418, "teapot", None),
        LlmError::Provider(_)
    ));
}

#[test]
fn context_overflow_detection() {
    for body in [
        "context_length_exceeded",
        "maximum context length",
        "prompt is too long",
        "CONTEXT LENGTH exceeded",
    ] {
        assert!(
            is_context_overflow(body),
            "{body} should be a context overflow"
        );
    }
    assert!(!is_context_overflow("invalid api key"));
}

#[test]
fn retryable_table() {
    assert!(LlmError::Network("reset".into()).retryable());
    assert!(LlmError::Timeout("idle").retryable());
    assert!(LlmError::RateLimited { retry_after: None }.retryable());
    assert!(LlmError::Overloaded.retryable());
    assert!(LlmError::Server(500, "boom".into()).retryable());
    assert!(LlmError::Server(409, "conflict".into()).retryable());
    for error in [
        LlmError::Auth("bad key".into()),
        LlmError::BadRequest("bad".into()),
        LlmError::NotFound("missing".into()),
        LlmError::ContextOverflow("long".into()),
        LlmError::Provider("nope".into()),
        LlmError::Protocol("bad body".into()),
        LlmError::Cancelled,
    ] {
        assert!(!error.retryable(), "{error:?} must not be retried");
    }
}

#[test]
fn retry_after_parses_seconds_and_http_dates() {
    let now = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
        .expect("date")
        .with_timezone(&chrono::Utc);
    assert_eq!(parse_retry_after("12", now), Some(Duration::from_secs(12)));
    assert_eq!(
        parse_retry_after("Thu, 01 Jan 2026 00:00:30 GMT", now),
        Some(Duration::from_secs(30))
    );
    // A date in the past means "retry now".
    assert_eq!(
        parse_retry_after("Wed, 31 Dec 2025 23:00:00 GMT", now),
        Some(Duration::ZERO)
    );
    // An unparseable value carries no hint.
    assert_eq!(parse_retry_after("soon", now), None);
}

#[test]
fn backoff_ladder_and_retry_after_cap() {
    // 2 s → 8 s → 30 s, with ±20 % jitter.
    let first = backoff_delay(1, None, 0).expect("first retry");
    assert!(first >= Duration::from_millis(1_600));
    assert!(first <= Duration::from_secs(3));
    let second = backoff_delay(2, None, 0).expect("second retry");
    assert!(second >= Duration::from_millis(6_400));
    assert!(second <= Duration::from_secs(10));
    let third = backoff_delay(3, None, 0).expect("third retry");
    assert!(third >= Duration::from_secs(24));
    assert!(third <= Duration::from_secs(30) + Duration::from_secs(6));
    // Four attempts in total means three retries; a fourth retry is refused.
    assert!(backoff_delay(MAX_ATTEMPTS, None, 0).is_none());

    // A longer retry-after wins, capped at 120 s.
    assert_eq!(
        backoff_delay(1, Some(Duration::from_secs(600)), 0),
        Some(Duration::from_secs(120))
    );
    // A shorter retry-after does not shrink the backoff.
    assert!(
        backoff_delay(3, Some(Duration::from_secs(1)), 0).expect("retry")
            >= Duration::from_secs(24)
    );
}

#[test]
fn request_id_is_attached_to_message_bearing_errors() {
    let error = LlmError::from_status(429, "slow down", None).with_request_id(Some("req-9"));
    assert!(matches!(error, LlmError::RateLimited { .. }));
    let error = LlmError::Server(500, "boom".into()).with_request_id(Some("req-9"));
    assert!(error.to_string().contains("req-9"));
    // No id available: unchanged.
    let error = LlmError::BadRequest("bad".into()).with_request_id(None);
    assert!(!error.to_string().contains("request-id"));
}
