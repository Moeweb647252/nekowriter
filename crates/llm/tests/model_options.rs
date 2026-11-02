//! The option → protocol table (spec/04 §5) is verified against the adapters, not against the doc
//! comments in `types.rs`.
//!
//! The bug this file exists for: a model profile stored the **Anthropic** `effort` field on an
//! **OpenAI**-protocol provider. The OpenAI adapter sends only `reasoning_effort`, so the request
//! carried no reasoning parameter at all, the turn succeeded, and nothing reported the mismatch.
//! A doc comment saying "OpenAI-compatible" is not evidence that a field reaches the wire, so every
//! key below is set on a request and both bodies are built: a key classified as reachable by a
//! protocol **must appear** in that protocol's body, and a key classified as unreachable **must
//! not**.

use std::collections::BTreeMap;

use nekowriter_llm::anthropic::build_body as anthropic_body;
use nekowriter_llm::openai::build_body as openai_body;
use nekowriter_llm::types::{
    CacheHints, CacheTtl, CacheTtlOption, ChatRequest, Effort, EffortRule, MaxTokensField, Message,
    ModelOptionKey, ModelOptions, Protocol, ReasoningReplay, TextBlock, ThinkingDisplay, ToolDef,
};

fn request() -> ChatRequest {
    ChatRequest {
        model: "test-model".to_owned(),
        system: "You are a writer.".to_owned(),
        messages: vec![Message::User {
            blocks: vec![TextBlock {
                text: "Write.".to_owned(),
                cache: true,
            }],
        }],
        tools: vec![ToolDef {
            name: "insert_text".to_owned(),
            description: "Write text.".to_owned(),
            input_schema: serde_json::json!({ "type": "object" }),
        }],
        max_output_tokens: 1024,
        parallel_tool_calls: false,
        cache: CacheHints {
            ttl: CacheTtl::OneHour,
        },
        options: ModelOptions::default(),
        demo_hints: None,
    }
}

/// A request whose options set exactly one key, plus the options themselves.
fn options_with(key: ModelOptionKey) -> ModelOptions {
    let mut extra = BTreeMap::new();
    extra.insert("probe_knob".to_owned(), serde_json::json!("probe"));
    match key {
        ModelOptionKey::ReasoningReplay => ModelOptions {
            reasoning_replay: ReasoningReplay::Always,
            ..ModelOptions::default()
        },
        ModelOptionKey::CacheTtl => ModelOptions {
            cache_ttl: CacheTtlOption::OneHour,
            ..ModelOptions::default()
        },
        ModelOptionKey::ReasoningEffort => ModelOptions {
            reasoning_effort: Some("probe-effort".to_owned()),
            ..ModelOptions::default()
        },
        ModelOptionKey::MaxTokensField => ModelOptions {
            max_tokens_field: MaxTokensField::MaxCompletionTokens,
            ..ModelOptions::default()
        },
        ModelOptionKey::SendParallelToolCalls => ModelOptions {
            send_parallel_tool_calls: Some(false),
            ..ModelOptions::default()
        },
        ModelOptionKey::StreamUsage => ModelOptions {
            stream_usage: Some(false),
            ..ModelOptions::default()
        },
        ModelOptionKey::Temperature => ModelOptions {
            temperature: Some(0.25),
            ..ModelOptions::default()
        },
        ModelOptionKey::ExtraBody => ModelOptions {
            extra_body: Some(extra),
            ..ModelOptions::default()
        },
        ModelOptionKey::Effort => ModelOptions {
            effort: Some(Effort::Xhigh),
            ..ModelOptions::default()
        },
        ModelOptionKey::ThinkingDisplay => ModelOptions {
            thinking_display: Some(ThinkingDisplay::Omitted),
            ..ModelOptions::default()
        },
        ModelOptionKey::RefusalFallback => ModelOptions {
            refusal_fallback: Some(true),
            ..ModelOptions::default()
        },
        ModelOptionKey::EagerInputStreaming => ModelOptions {
            eager_input_streaming: Some(false),
            ..ModelOptions::default()
        },
    }
}

/// The markers an option leaves in a body: the paths to look at, and the probe value expected
/// there. `None` as the expected value means "the option leaves no marker at all".
fn markers(key: ModelOptionKey) -> Vec<(&'static str, serde_json::Value)> {
    match key {
        // Read during message mapping, so the probe is an assistant turn carrying reasoning.
        ModelOptionKey::ReasoningReplay => Vec::new(),
        ModelOptionKey::CacheTtl => Vec::new(),
        ModelOptionKey::ReasoningEffort => {
            vec![("reasoning_effort", serde_json::json!("probe-effort"))]
        }
        ModelOptionKey::MaxTokensField => {
            vec![("max_completion_tokens", serde_json::json!(1024))]
        }
        ModelOptionKey::SendParallelToolCalls => Vec::new(),
        ModelOptionKey::StreamUsage => Vec::new(),
        ModelOptionKey::Temperature => vec![("temperature", serde_json::json!(0.25))],
        ModelOptionKey::ExtraBody => vec![("probe_knob", serde_json::json!("probe"))],
        ModelOptionKey::Effort => vec![("output_config.effort", serde_json::json!("xhigh"))],
        ModelOptionKey::ThinkingDisplay => {
            vec![("thinking.display", serde_json::json!("omitted"))]
        }
        ModelOptionKey::RefusalFallback => vec![("fallbacks", serde_json::json!("default"))],
        ModelOptionKey::EagerInputStreaming => {
            vec![("tools.0.eager_input_streaming", serde_json::json!(false))]
        }
    }
}

/// Resolves a dotted path in a body, treating a numeric segment as an array index.
fn at<'a>(body: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = body;
    for segment in path.split('.') {
        current = match segment.parse::<usize>() {
            Ok(index) => current.get(index)?,
            Err(_) => current.get(segment)?,
        };
    }
    Some(current)
}

/// The keys whose effect is not a top-level body field. They are checked separately below, but
/// they still have to obey the same table.
const MESSAGE_MAPPING_KEYS: [ModelOptionKey; 4] = [
    ModelOptionKey::ReasoningReplay,
    ModelOptionKey::CacheTtl,
    ModelOptionKey::SendParallelToolCalls,
    ModelOptionKey::StreamUsage,
];

#[test]
fn the_body_builder_table_matches_the_adapters() {
    for key in ModelOptionKey::ALL {
        if MESSAGE_MAPPING_KEYS.contains(&key) {
            continue;
        }
        let options = options_with(key);
        let bodies = [
            (Protocol::Openai, openai_body(&request(), &options)),
            (Protocol::Anthropic, anthropic_body(&request(), &options)),
        ];
        for (protocol, body) in bodies {
            let reachable = key.reachable_by(protocol);
            let found = markers(key)
                .iter()
                .all(|(path, expected)| at(&body, path).is_some_and(|value| value == expected));
            if reachable {
                assert!(
                    found,
                    "{} is classified as reachable by {protocol:?} but the body has no marker: {body}",
                    key.as_str()
                );
            } else {
                let absent = markers(key)
                    .iter()
                    .all(|(path, _)| at(&body, path).is_none());
                assert!(
                    absent,
                    "{} is classified as unreachable by {protocol:?} but the body carries a marker: {body}",
                    key.as_str()
                );
            }
        }
    }
}

/// `reasoning_replay` is read by both adapters — the OpenAI one replays `reasoning_content`, the
/// Anthropic one replays the stored content blocks verbatim. This is why it is classified as
/// "both" rather than OpenAI-only, despite the doc comment that used to say so.
#[test]
fn reasoning_replay_is_read_by_both_adapters() {
    let payload_openai = nekowriter_llm::types::ProviderPayload::Openai {
        content: Some("hi".to_owned()),
        reasoning_content: Some("because".to_owned()),
        tool_calls: Vec::new(),
    };
    let payload_anthropic = nekowriter_llm::types::ProviderPayload::Anthropic {
        content: vec![
            serde_json::json!({ "type": "thinking", "thinking": "because", "signature": "s" }),
        ],
    };
    let request_with = |payload: nekowriter_llm::types::ProviderPayload| {
        let mut req = request();
        req.messages = vec![Message::Assistant { payload }];
        req
    };

    for replay in [ReasoningReplay::Never, ReasoningReplay::Always] {
        let options = ModelOptions {
            reasoning_replay: replay,
            ..ModelOptions::default()
        };
        let openai = openai_body(&request_with(payload_openai.clone()), &options);
        let anthropic = anthropic_body(&request_with(payload_anthropic.clone()), &options);
        let openai_replays = at(&openai, "messages.1.reasoning_content").is_some();
        let anthropic_replays = at(&anthropic, "messages.0.content.0.thinking").is_some();
        let expected_openai = replay == ReasoningReplay::Always;
        assert_eq!(
            openai_replays, expected_openai,
            "the OpenAI adapter replays reasoning_content only with always: {openai}"
        );
        // The Anthropic adapter always echoes the raw blocks; the option does not change that.
        assert!(
            anthropic_replays,
            "the Anthropic adapter replays stored blocks verbatim: {anthropic}"
        );
    }
    assert!(ModelOptionKey::ReasoningReplay.reachable_by(Protocol::Anthropic));
    assert!(ModelOptionKey::ReasoningReplay.reachable_by(Protocol::Openai));
}

/// `cache_ttl` reaches both adapters through `req.cache`, which the server resolves from the
/// option (spec/03 §4 step 1). The OpenAI body has no explicit breakpoint by design, so this pins
/// the neutral-path behaviour that the Anthropic body shows.
#[test]
fn cache_ttl_is_read_by_both_adapters() {
    assert!(ModelOptionKey::CacheTtl.reachable_by(Protocol::Anthropic));
    assert!(ModelOptionKey::CacheTtl.reachable_by(Protocol::Openai));
    let mut req = request();
    req.cache = CacheHints {
        ttl: CacheTtl::FiveMinutes,
    };
    let body = anthropic_body(&req, &ModelOptions::default());
    assert!(
        at(&body, "cache_control.ttl").is_none(),
        "a 5m TTL is expressed by omitting ttl: {body}"
    );
    req.cache = CacheHints {
        ttl: CacheTtl::OneHour,
    };
    let body = anthropic_body(&req, &ModelOptions::default());
    assert_eq!(
        at(&body, "cache_control.ttl"),
        Some(&serde_json::json!("1h")),
        "the resolved TTL reaches the body: {body}"
    );
}

/// `send_parallel_tool_calls` and `stream_usage` are OpenAI-only *sending* flags: the Anthropic
/// adapter expresses the same intent through `tool_choice.disable_parallel_tool_use` (always sent)
/// and has no usage option.
#[test]
fn the_openai_only_sending_flags_are_absent_from_the_anthropic_body() {
    for key in [
        ModelOptionKey::SendParallelToolCalls,
        ModelOptionKey::StreamUsage,
    ] {
        assert!(!key.reachable_by(Protocol::Anthropic), "{}", key.as_str());
        assert!(key.reachable_by(Protocol::Openai), "{}", key.as_str());
    }
    let options = options_with(ModelOptionKey::StreamUsage);
    let body = anthropic_body(&request(), &options);
    assert!(at(&body, "stream_options").is_none(), "{body}");
    let options = options_with(ModelOptionKey::SendParallelToolCalls);
    let body = anthropic_body(&request(), &options);
    assert!(at(&body, "parallel_tool_calls").is_none(), "{body}");
}

/// `extra_body` is the documented escape hatch for a gateway knob the protocol has no option for
/// (spec/04 §1), so it must merge **last** and win over every built-in field.
#[test]
fn extra_body_merges_last_and_wins() {
    let mut extra = BTreeMap::new();
    extra.insert("max_tokens".to_owned(), serde_json::json!(99));
    extra.insert("temperature".to_owned(), serde_json::json!(0.0));
    extra.insert("reasoning_effort".to_owned(), serde_json::json!("low"));
    extra.insert("model".to_owned(), serde_json::json!("overridden"));
    extra.insert("probe_knob".to_owned(), serde_json::json!("probe"));
    let options = ModelOptions {
        temperature: Some(0.7),
        reasoning_effort: Some("high".to_owned()),
        extra_body: Some(extra),
        ..ModelOptions::default()
    };
    let body = openai_body(&request(), &options);
    assert_eq!(
        at(&body, "max_tokens"),
        Some(&serde_json::json!(99)),
        "{body}"
    );
    assert_eq!(
        at(&body, "temperature"),
        Some(&serde_json::json!(0.0)),
        "{body}"
    );
    assert_eq!(
        at(&body, "reasoning_effort"),
        Some(&serde_json::json!("low")),
        "{body}"
    );
    assert_eq!(
        at(&body, "model"),
        Some(&serde_json::json!("overridden")),
        "{body}"
    );
    assert_eq!(
        at(&body, "probe_knob"),
        Some(&serde_json::json!("probe")),
        "a key the builder never writes is added: {body}"
    );
}

/// The mock protocol sends nothing: its client is the in-process Demo provider, which ignores the
/// request. Accepting an option there would be the same stored-but-inert defect on a dev provider,
/// so the whole table is unreachable.
#[test]
fn the_mock_protocol_sends_no_option() {
    for key in ModelOptionKey::ALL {
        assert!(
            !key.reachable_by(Protocol::Mock),
            "{} must not be accepted on the mock protocol",
            key.as_str()
        );
        assert!(
            !key.protocols().contains(&Protocol::Mock),
            "{}",
            key.as_str()
        );
    }
    assert!(ModelOptionKey::reachable(Protocol::Mock).is_empty());
}

/// `reachable` and `protocols` are the inverse views of `reachable_by`, and the UI reads both.
#[test]
fn the_table_views_agree() {
    for protocol in [Protocol::Openai, Protocol::Anthropic, Protocol::Mock] {
        let reachable = ModelOptionKey::reachable(protocol);
        for key in ModelOptionKey::ALL {
            assert_eq!(
                reachable.contains(&key),
                key.reachable_by(protocol),
                "{} on {protocol:?}",
                key.as_str()
            );
            assert_eq!(
                key.protocols().contains(&protocol),
                key.reachable_by(protocol),
                "{} on {protocol:?}",
                key.as_str()
            );
        }
    }
    // Both lists are in `ModelOptions` order and exhaustive.
    for protocol in [Protocol::Openai, Protocol::Anthropic, Protocol::Mock] {
        let reachable = ModelOptionKey::reachable(protocol);
        let positions: Vec<usize> = reachable
            .iter()
            .map(|key| {
                ModelOptionKey::ALL
                    .iter()
                    .position(|candidate| candidate == key)
                    .expect("a known key")
            })
            .collect();
        let mut sorted = positions.clone();
        sorted.sort_unstable();
        assert_eq!(positions, sorted, "the list keeps the ModelOptions order");
    }
    assert_eq!(ModelOptionKey::reachable(Protocol::Openai).len(), 8);
    assert_eq!(ModelOptionKey::reachable(Protocol::Anthropic).len(), 6);
}

/// The thinking-effort rule (spec/04 §5.1, D-162) is one source of truth for a protocol's wire
/// field and its levels: free-form tokens on `openai`, the canonical closed set on `anthropic`,
/// none on `mock`.
#[test]
fn effort_rule_follows_the_protocol() {
    assert_eq!(EffortRule::for_protocol(Protocol::Openai), EffortRule::Free);
    assert_eq!(
        EffortRule::for_protocol(Protocol::Anthropic),
        EffortRule::Closed(&["low", "medium", "high", "xhigh", "max"])
    );
    assert_eq!(EffortRule::for_protocol(Protocol::Mock), EffortRule::None);

    assert_eq!(
        EffortRule::for_protocol(Protocol::Openai).wire_key(),
        Some("reasoning_effort")
    );
    assert_eq!(
        EffortRule::for_protocol(Protocol::Anthropic).wire_key(),
        Some("output_config.effort")
    );
    assert_eq!(EffortRule::for_protocol(Protocol::Mock).wire_key(), None);
}

/// `Free` accepts exactly `^[A-Za-z0-9_-]{1,32}$`; `Closed` accepts exactly its canonical list;
/// `None` accepts nothing. The same rule decides the role-effort save validation and the
/// per-session resolution, so its boundaries are pinned here.
#[test]
fn effort_rule_enforces_the_level_pattern_and_canonical_order() {
    let openai = EffortRule::for_protocol(Protocol::Openai);
    for level in ["a", "low", "max", "x-high_1", "A9_-"] {
        assert!(EffortRule::is_valid_level(level), "{level}");
        assert!(openai.accepts(level), "{level}");
    }
    assert!(EffortRule::is_valid_level(&"a".repeat(32)));
    for level in ["", "high effort", "high!", "思考", "a.b"] {
        assert!(!EffortRule::is_valid_level(level), "{level}");
        assert!(!openai.accepts(level), "{level}");
    }
    assert!(!EffortRule::is_valid_level(&"a".repeat(33)));

    let anthropic = EffortRule::for_protocol(Protocol::Anthropic);
    assert_eq!(
        anthropic.levels(),
        &["low", "medium", "high", "xhigh", "max"],
        "the Anthropic set is stored lowest first, whatever order a caller uses"
    );
    for level in ["low", "medium", "high", "xhigh", "max"] {
        assert!(anthropic.accepts(level), "{level}");
    }
    for level in ["ultra", "none", "LOW", "", " high"] {
        assert!(!anthropic.accepts(level), "{level}");
    }

    let mock = EffortRule::for_protocol(Protocol::Mock);
    assert!(mock.levels().is_empty());
    assert!(!mock.accepts("high"), "the mock protocol takes no effort");
}

/// A model profile may hold every option except the two effort wire carriers, which belong to
/// the role's resolved effort (spec/04 §5, D-162). `ALL` stays complete for the adapter wire test.
#[test]
fn profile_options_exclude_the_effort_wire_fields() {
    let expected: Vec<ModelOptionKey> = ModelOptionKey::ALL
        .into_iter()
        .filter(|key| {
            !matches!(
                key,
                ModelOptionKey::ReasoningEffort | ModelOptionKey::Effort
            )
        })
        .collect();
    assert_eq!(ModelOptionKey::PROFILE.to_vec(), expected);
    assert_eq!(
        ModelOptionKey::PROFILE.len(),
        ModelOptionKey::ALL.len() - 2,
        "PROFILE is ALL minus exactly the two effort keys"
    );
    assert!(!ModelOptionKey::PROFILE.contains(&ModelOptionKey::ReasoningEffort));
    assert!(!ModelOptionKey::PROFILE.contains(&ModelOptionKey::Effort));
    assert!(ModelOptionKey::ALL.contains(&ModelOptionKey::ReasoningEffort));
    assert!(ModelOptionKey::ALL.contains(&ModelOptionKey::Effort));
}

/// The set of options a value actually sets drives both validation and the mismatch report, so a
/// default value must not be reported: profiles are stored with every field serialized.
#[test]
fn set_keys_ignores_defaults_and_reports_set_fields() {
    assert!(ModelOptions::default().set_keys().is_empty());
    // The stored shape of the author's profile #4: the Anthropic `effort` on an OpenAI provider.
    let stored: ModelOptions = serde_json::from_value(serde_json::json!({
        "reasoning_replay": "always",
        "max_tokens_field": "max_tokens",
        "effort": "high",
        "thinking_display": "omitted",
        "cache_ttl": "1h",
    }))
    .expect("the stored blob");
    let keys = stored.set_keys();
    for expected in [
        ModelOptionKey::ReasoningReplay,
        ModelOptionKey::Effort,
        ModelOptionKey::ThinkingDisplay,
        ModelOptionKey::CacheTtl,
    ] {
        assert!(keys.contains(&expected), "{}", expected.as_str());
    }
    // `max_tokens_field: max_tokens` is the default and is not reported as set.
    assert!(!keys.contains(&ModelOptionKey::MaxTokensField));

    assert_eq!(
        stored.unreachable_keys(Protocol::Openai),
        vec![ModelOptionKey::Effort, ModelOptionKey::ThinkingDisplay],
        "the OpenAI protocol cannot send either field"
    );
    assert!(
        stored.unreachable_keys(Protocol::Anthropic).is_empty(),
        "both are reachable on Anthropic"
    );
}

/// `clear_unreachable` removes exactly the stranded fields and keeps everything else, which is
/// what the settings page does when a profile's provider switches protocol.
#[test]
fn clear_unreachable_removes_only_the_stranded_options() {
    let mut options: ModelOptions = serde_json::from_value(serde_json::json!({
        "reasoning_replay": "always",
        "max_tokens_field": "max_completion_tokens",
        "reasoning_effort": "high",
        "effort": "xhigh",
        "thinking_display": "omitted",
        "cache_ttl": "1h",
    }))
    .expect("options");
    let removed = options.clear_unreachable(Protocol::Openai);
    assert_eq!(
        removed,
        vec![ModelOptionKey::Effort, ModelOptionKey::ThinkingDisplay]
    );
    assert_eq!(options.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(
        options.max_tokens_field,
        MaxTokensField::MaxCompletionTokens
    );
    assert_eq!(options.reasoning_replay, ReasoningReplay::Always);
    assert_eq!(options.cache_ttl, CacheTtlOption::OneHour);
    assert!(options.effort.is_none());
    assert!(options.thinking_display.is_none());

    let mut anthropic_only: ModelOptions = serde_json::from_value(serde_json::json!({
        "reasoning_effort": "high",
        "temperature": 0.5,
        "extra_body": { "top_k": 5 },
        "effort": "max",
    }))
    .expect("options");
    let removed = anthropic_only.clear_unreachable(Protocol::Anthropic);
    assert_eq!(
        removed,
        vec![
            ModelOptionKey::ReasoningEffort,
            ModelOptionKey::Temperature,
            ModelOptionKey::ExtraBody
        ]
    );
    assert_eq!(anthropic_only.effort, Some(Effort::Max));
}
