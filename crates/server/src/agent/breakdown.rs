//! The context breakdown of one session (D-171, M23).
//!
//! The function is deliberately pure: it takes already-loaded rows and returns the DTO the
//! Context view renders. The loader in `services::sessions` owns the three pool reads; no row
//! is read here and no transaction is held.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::agent::seal;
use crate::db::entities::{agent_messages, agent_sessions, agent_tool_calls};
use crate::db::json::{
    Appendix, AppendixKind, InboxKind, MessageKind, MessageRole, ModelOptions, Protocol,
    StoredBlock,
};

/// Fixed group order (plan §3.2 C4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ContextGroupKey {
    System,
    Tools,
    Kickoff,
    Author,
    Runtime,
    Model,
    Results,
}

/// The part keys of plan §3.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ContextPartKey {
    SystemPrompt,
    Tool,
    Block,
    Message,
    Feedback,
    EditReport,
    Notice,
    EffectNotice,
    Nudge,
    Text,
    Reasoning,
    Call,
    Result,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ContextBreakdownDto {
    pub session_id: i32,
    /// The request that produced an assistant message; `null` is the current context.
    pub seq: Option<i32>,
    pub context_window: i32,
    pub max_output_tokens: i32,
    #[ts(type = "number | null")]
    pub rollover_at: Option<i64>,
    #[ts(type = "number")]
    pub estimate: i64,
    #[ts(type = "number | null")]
    pub measured: Option<i64>,
    #[ts(type = "number | null")]
    pub cached: Option<i64>,
    pub scale: f64,
    pub calibrated: bool,
    pub pending_calls: i32,
    pub groups: Vec<ContextGroupDto>,
    pub requests: Vec<ContextRequestDto>,
    /// Display/exposition only: the pre-estimation leaf texts used by the rebuild test and the
    /// future treemap list. Never serialized in the API.
    #[serde(skip)]
    #[ts(skip)]
    pub leaves: Vec<ContextLeaf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ContextGroupDto {
    pub key: ContextGroupKey,
    #[ts(type = "number")]
    pub estimate: i64,
    pub parts: Vec<ContextPartDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ContextPartDto {
    pub key: ContextPartKey,
    pub name: Option<String>,
    #[ts(type = "number")]
    pub estimate: i64,
    pub items: Vec<ContextItemDto>,
    pub rest: Option<ContextRestDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ContextRestDto {
    #[ts(type = "number")]
    pub count: i64,
    #[ts(type = "number")]
    pub estimate: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ContextItemDto {
    pub seq: i32,
    pub call_index: Option<i32>,
    #[ts(type = "number")]
    pub estimate: i64,
    pub excerpt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ContextRequestDto {
    pub seq: i32,
    #[ts(type = "number")]
    pub prompt_tokens: i64,
    #[ts(type = "number")]
    pub cached_tokens: i64,
    pub estimated: bool,
    #[ts(type = "number")]
    pub estimate: i64,
    pub created_at: String,
}

/// One pre-estimation leaf of the breakdown, used to compare a view with the real request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextLeaf {
    pub group: ContextGroupKey,
    pub part: ContextPartKey,
    pub name: Option<String>,
    pub text: String,
}

/// The pure breakdown of one session at `seq`.
///
/// `seq = None` is the current context; `seq = Some(k)` is the request that produced assistant
/// message `k`. The caller has already validated that `k` names an assistant message.
pub fn breakdown(
    session: &agent_sessions::Model,
    messages: &[agent_messages::Model],
    calls: &[agent_tool_calls::Model],
    seq: Option<i32>,
) -> ContextBreakdownDto {
    build(session, messages, calls, seq, true)
}

fn build(
    session: &agent_sessions::Model,
    messages: &[agent_messages::Model],
    calls: &[agent_tool_calls::Model],
    seq: Option<i32>,
    with_requests: bool,
) -> ContextBreakdownDto {
    let snapshot = session.model_snapshot.clone().into_inner();
    let evidence = Evidence::new(session);
    let mut acc = Accumulator::default();

    // The system prompt and tools are part of every request. They have no message item.
    acc.add_scalar(
        ContextGroupKey::System,
        ContextPartKey::SystemPrompt,
        None,
        evidence.estimate(&session.system_prompt),
        &session.system_prompt,
    );
    for tool in session.tools.clone().into_inner() {
        let text = format!(
            "{}\n{}\n{}",
            tool.name,
            tool.description,
            serde_json::to_string(&tool.input_schema).unwrap_or_default()
        );
        acc.add_scalar(
            ContextGroupKey::Tools,
            ContextPartKey::Tool,
            Some(tool.name.clone()),
            evidence.estimate(&text),
            &text,
        );
    }

    let mut included_messages: Vec<&agent_messages::Model> = messages
        .iter()
        .filter(|message| seq.is_none_or(|limit| message.seq < limit))
        .collect();
    included_messages.sort_by_key(|message| message.seq);
    let included_ids: std::collections::HashSet<i32> =
        included_messages.iter().map(|message| message.id).collect();

    for message in &included_messages {
        match message.kind {
            MessageKind::Kickoff => {
                for block in message.content.clone().into_inner() {
                    if let StoredBlock::Text { text, source, .. } = block {
                        if text.trim().is_empty() {
                            continue;
                        }
                        acc.add_kickoff(source, evidence.estimate(&text), text);
                    }
                }
            }
            MessageKind::Assistant => {
                for block in message.content.clone().into_inner() {
                    match block {
                        StoredBlock::Text { text, .. } => {
                            if !text.trim().is_empty() {
                                acc.add_item(
                                    ContextGroupKey::Model,
                                    ContextPartKey::Text,
                                    None,
                                    message.seq,
                                    None,
                                    evidence.estimate(&text),
                                    excerpt(&text),
                                    &text,
                                );
                            }
                        }
                        StoredBlock::Reasoning { text } => {
                            if evidence.reasoning_replayed() && !text.trim().is_empty() {
                                acc.add_item(
                                    ContextGroupKey::Model,
                                    ContextPartKey::Reasoning,
                                    None,
                                    message.seq,
                                    None,
                                    evidence.estimate(&text),
                                    excerpt(&text),
                                    &text,
                                );
                            }
                        }
                        StoredBlock::ToolCall {
                            name, input_text, ..
                        } => {
                            let text = format!("{name}\n{input_text}");
                            acc.add_item(
                                ContextGroupKey::Model,
                                ContextPartKey::Call,
                                Some(name),
                                message.seq,
                                None,
                                evidence.estimate(&text),
                                excerpt(&text),
                                &text,
                            );
                        }
                    }
                }
            }
            MessageKind::User | MessageKind::Notice | MessageKind::Nudge => {
                for block in message.content.clone().into_inner() {
                    if let StoredBlock::Text { text, source, .. } = block {
                        if text.trim().is_empty() {
                            continue;
                        }
                        let (group, part) = classify_text(source.as_deref(), message.kind);
                        acc.add_item(
                            group,
                            part,
                            None,
                            message.seq,
                            None,
                            evidence.estimate(&text),
                            excerpt(&text),
                            &text,
                        );
                    }
                }
            }
        }
    }

    // Calls belong to the assistant message that emitted them. The loader drops calls whose
    // message was not loaded; this filter keeps the pure function safe for direct callers too.
    let mut pending_calls = 0;
    for call in calls {
        if !included_ids.contains(&call.message_id) {
            continue;
        }
        let message = included_messages
            .iter()
            .find(|message| message.id == call.message_id);
        let message_seq = message.map(|message| message.seq).unwrap_or(0);
        let appendices: Vec<Appendix> = call.appendices.clone().into_inner();
        let delivered = if call.sealed {
            call.result_text.clone().unwrap_or_default()
        } else if seq.is_none() && (call.raw_result_text.is_some() || !appendices.is_empty()) {
            seal::compose(call.raw_result_text.as_deref(), &appendices)
        } else {
            if seq.is_none() {
                pending_calls += 1;
            }
            continue;
        };

        let mut appendix_estimate = 0i64;
        for appendix in &appendices {
            if appendix.text.trim().is_empty() {
                continue;
            }
            let (group, part) = classify_appendix(appendix);
            let estimate = evidence.estimate(&appendix.text);
            appendix_estimate += estimate;
            acc.add_item(
                group,
                part,
                None,
                message_seq,
                Some(call.call_index),
                estimate,
                excerpt(&appendix.text),
                &appendix.text,
            );
        }

        let result_estimate = (evidence.estimate(&delivered) - appendix_estimate).max(0);
        if result_estimate > 0 {
            acc.add_item(
                ContextGroupKey::Results,
                ContextPartKey::Result,
                Some(call.name.clone()),
                message_seq,
                Some(call.call_index),
                result_estimate,
                excerpt(&delivered),
                &delivered,
            );
        }
    }

    let leaves = acc.take_leaves();
    let mut groups: Vec<ContextGroupDto> = Vec::new();
    for key in ContextGroupKey::all() {
        if let Some(group) = acc.take_group(key) {
            groups.push(group);
        }
    }
    // `Accumulator` stores the fixed order; sort parts only outside kickoff.
    for group in &mut groups {
        if group.key != ContextGroupKey::Kickoff {
            group
                .parts
                .sort_by_key(|part| std::cmp::Reverse(part.estimate));
        }
    }

    let estimate = groups.iter().map(|group| group.estimate).sum();
    let (measured, cached, scale, calibrated) =
        measurement(session, messages, calls, seq, estimate);

    let mut requests = Vec::new();
    if with_requests {
        for message in messages
            .iter()
            .filter(|message| message.kind == MessageKind::Assistant)
        {
            let usage = message.usage.clone().map(|usage| usage.into_inner());
            let prompt_tokens = usage
                .as_ref()
                .map(|usage| usage.last_prompt_tokens)
                .unwrap_or(0);
            let cached_tokens = usage
                .as_ref()
                .map(|usage| usage.cached_input_tokens)
                .unwrap_or(0);
            let estimated = usage.as_ref().is_none_or(|usage| usage.estimated);
            let request_estimate =
                build(session, messages, calls, Some(message.seq), false).estimate;
            requests.push(ContextRequestDto {
                seq: message.seq,
                prompt_tokens,
                cached_tokens,
                estimated,
                estimate: request_estimate,
                created_at: message.created_at.to_rfc3339(),
            });
        }
    }

    ContextBreakdownDto {
        session_id: session.id,
        seq,
        context_window: snapshot.context_window,
        max_output_tokens: snapshot.max_output_tokens,
        rollover_at: crate::agent::runner::rollover_eligible(session).then(|| {
            (f64::from(snapshot.context_window) * 0.85).floor() as i64
                - i64::from(snapshot.max_output_tokens)
        }),
        estimate,
        measured,
        cached,
        scale,
        calibrated,
        pending_calls,
        groups,
        requests,
        leaves,
    }
}

// ── evidence / helpers ───────────────────────────────────────────────────────

struct Evidence {
    protocol: Protocol,
    reasoning_replay: Option<String>,
}

impl Evidence {
    fn new(session: &agent_sessions::Model) -> Self {
        let snapshot = session.model_snapshot.clone().into_inner();
        let options: ModelOptions = serde_json::from_value(snapshot.options).unwrap_or_default();
        Self {
            protocol: snapshot.protocol,
            reasoning_replay: options.reasoning_replay,
        }
    }

    fn estimate(&self, text: &str) -> i64 {
        nekowriter_llm::estimate_tokens(text)
    }

    fn reasoning_replayed(&self) -> bool {
        match self.protocol {
            Protocol::Anthropic => true,
            Protocol::Openai | Protocol::Mock => self.reasoning_replay.as_deref() == Some("always"),
        }
    }
}

fn classify_text(source: Option<&str>, kind: MessageKind) -> (ContextGroupKey, ContextPartKey) {
    match source {
        Some("edit_report") => (ContextGroupKey::Author, ContextPartKey::EditReport),
        Some("inbox.user_message") => (ContextGroupKey::Author, ContextPartKey::Message),
        Some("inbox.notice") => (ContextGroupKey::Runtime, ContextPartKey::Notice),
        _ => match kind {
            MessageKind::User => (ContextGroupKey::Author, ContextPartKey::Message),
            MessageKind::Notice => (ContextGroupKey::Runtime, ContextPartKey::Notice),
            MessageKind::Nudge => (ContextGroupKey::Runtime, ContextPartKey::Nudge),
            _ => (ContextGroupKey::Runtime, ContextPartKey::Notice),
        },
    }
}

fn classify_appendix(appendix: &Appendix) -> (ContextGroupKey, ContextPartKey) {
    match appendix.kind {
        AppendixKind::UserFeedback => (ContextGroupKey::Author, ContextPartKey::Feedback),
        AppendixKind::EditReport => (ContextGroupKey::Author, ContextPartKey::EditReport),
        AppendixKind::EffectNotice => (ContextGroupKey::Runtime, ContextPartKey::EffectNotice),
        AppendixKind::Inbox => match appendix.inbox_kind {
            Some(InboxKind::Notice) => (ContextGroupKey::Runtime, ContextPartKey::Notice),
            _ => (ContextGroupKey::Author, ContextPartKey::Message),
        },
    }
}

fn measurement(
    session: &agent_sessions::Model,
    messages: &[agent_messages::Model],
    calls: &[agent_tool_calls::Model],
    seq: Option<i32>,
    estimate: i64,
) -> (Option<i64>, Option<i64>, f64, bool) {
    let snapshot = session.model_snapshot.clone().into_inner();
    let _ = snapshot;
    if let Some(target) = seq {
        if let Some(message) = messages.iter().find(|message| message.seq == target)
            && let Some(usage) = message.usage.clone().map(|usage| usage.into_inner())
            && !usage.estimated
            && usage.last_prompt_tokens > 0
        {
            let measured = usage.last_prompt_tokens;
            return (
                Some(measured),
                Some(usage.cached_input_tokens),
                fraction(measured, estimate),
                true,
            );
        }
        return (None, None, 1.0, false);
    }
    for message in messages.iter().rev() {
        if message.kind != MessageKind::Assistant {
            continue;
        }
        if let Some(usage) = message.usage.clone().map(|usage| usage.into_inner())
            && !usage.estimated
            && usage.last_prompt_tokens > 0
        {
            let request_estimate =
                build(session, messages, calls, Some(message.seq), false).estimate;
            return (
                None,
                None,
                fraction(usage.last_prompt_tokens, request_estimate),
                true,
            );
        }
    }
    (None, None, 1.0, false)
}

fn fraction(measured: i64, estimate: i64) -> f64 {
    if estimate > 0 {
        measured as f64 / estimate as f64
    } else {
        1.0
    }
}

fn excerpt(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(80).collect()
}

#[derive(Default)]
struct Accumulator {
    groups: Vec<(ContextGroupKey, Vec<PartBuilder>)>,
    leaves: Vec<ContextLeaf>,
}

struct PartBuilder {
    key: ContextPartKey,
    name: Option<String>,
    estimate: i64,
    items: Vec<ContextItemDto>,
}

impl Accumulator {
    fn part(
        &mut self,
        group: ContextGroupKey,
        key: ContextPartKey,
        name: Option<&str>,
    ) -> &mut PartBuilder {
        let group_index = match self
            .groups
            .iter()
            .position(|(candidate, _)| *candidate == group)
        {
            Some(index) => index,
            None => {
                self.groups.push((group, Vec::new()));
                self.groups.len() - 1
            }
        };
        let parts = &mut self.groups[group_index].1;
        let part_index = match parts
            .iter()
            .position(|part| part.key == key && part.name == name.map(str::to_owned))
        {
            Some(index) => index,
            None => {
                parts.push(PartBuilder {
                    key,
                    name: name.map(str::to_owned),
                    estimate: 0,
                    items: Vec::new(),
                });
                parts.len() - 1
            }
        };
        &mut self.groups[group_index].1[part_index]
    }

    fn add_scalar(
        &mut self,
        group: ContextGroupKey,
        key: ContextPartKey,
        name: Option<String>,
        estimate: i64,
        text: &str,
    ) {
        self.leaves.push(ContextLeaf {
            group,
            part: key,
            name: name.clone(),
            text: text.to_owned(),
        });
        let part = self.part(group, key, name.as_deref());
        part.estimate += estimate;
    }

    fn add_kickoff(&mut self, source: Option<String>, estimate: i64, text: String) {
        self.leaves.push(ContextLeaf {
            group: ContextGroupKey::Kickoff,
            part: ContextPartKey::Block,
            name: source.clone(),
            text,
        });
        if let Some((_, parts)) = self
            .groups
            .iter_mut()
            .find(|(candidate, _)| *candidate == ContextGroupKey::Kickoff)
        {
            parts.push(PartBuilder {
                key: ContextPartKey::Block,
                name: source,
                estimate,
                items: Vec::new(),
            });
        } else {
            self.groups.push((
                ContextGroupKey::Kickoff,
                vec![PartBuilder {
                    key: ContextPartKey::Block,
                    name: source,
                    estimate,
                    items: Vec::new(),
                }],
            ));
        }
    }

    // The builder API is clearer with the item fields split than with an opaque tuple.
    #[allow(clippy::too_many_arguments)]
    fn add_item(
        &mut self,
        group: ContextGroupKey,
        key: ContextPartKey,
        name: Option<String>,
        seq: i32,
        call_index: Option<i32>,
        estimate: i64,
        excerpt: String,
        text: &str,
    ) {
        self.leaves.push(ContextLeaf {
            group,
            part: key,
            name: name.clone(),
            text: text.to_owned(),
        });
        let part = self.part(group, key, name.as_deref());
        part.estimate += estimate;
        part.items.push(ContextItemDto {
            seq,
            call_index,
            estimate,
            excerpt,
        });
    }

    fn take_leaves(&mut self) -> Vec<ContextLeaf> {
        std::mem::take(&mut self.leaves)
    }

    fn take_group(&mut self, key: ContextGroupKey) -> Option<ContextGroupDto> {
        let index = self
            .groups
            .iter()
            .position(|(candidate, _)| *candidate == key)?;
        let (_, parts) = std::mem::replace(
            &mut self.groups[index],
            (ContextGroupKey::System, Vec::new()),
        );
        let mut result = Vec::new();
        for part in parts {
            let mut items = part.items;
            items.sort_by_key(|item| std::cmp::Reverse(item.estimate));
            let rest = if items.len() > 40 {
                let omitted = items.split_off(40);
                Some(ContextRestDto {
                    count: omitted.len() as i64,
                    estimate: omitted.iter().map(|item| item.estimate).sum(),
                })
            } else {
                None
            };
            if part.estimate == 0 && items.is_empty() {
                continue;
            }
            result.push(ContextPartDto {
                key: part.key,
                name: part.name,
                estimate: part.estimate,
                items,
                rest,
            });
        }
        if result.is_empty() {
            return None;
        }
        let estimate = result.iter().map(|part| part.estimate).sum();
        Some(ContextGroupDto {
            key,
            estimate,
            parts: result,
        })
    }
}

impl ContextGroupKey {
    fn all() -> [Self; 7] {
        [
            Self::System,
            Self::Tools,
            Self::Kickoff,
            Self::Author,
            Self::Runtime,
            Self::Model,
            Self::Results,
        ]
    }
}

#[allow(dead_code)]
fn _keep_role(_: MessageRole) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence(protocol: Protocol, replay: Option<&str>) -> Evidence {
        Evidence {
            protocol,
            reasoning_replay: replay.map(str::to_owned),
        }
    }

    #[test]
    fn reasoning_replay_follows_protocol_and_option() {
        for replay in [None, Some("always"), Some("never")] {
            assert!(evidence(Protocol::Anthropic, replay).reasoning_replayed());
            assert_eq!(
                evidence(Protocol::Openai, replay).reasoning_replayed(),
                replay == Some("always")
            );
            assert_eq!(
                evidence(Protocol::Mock, replay).reasoning_replayed(),
                replay == Some("always")
            );
        }
    }
}
