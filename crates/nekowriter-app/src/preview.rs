//! Streaming previews: what the model has sent so far for the request in flight
//! (`docs/product-spec.md` §4 invariant 1, §16.2).
//!
//! A preview is not content. It lives only in memory, grows as the answer streams, and is
//! cleared when the answer ends. Event streams send the latest state at a bounded rate, so
//! a slow subscriber skips intermediate states instead of falling behind.

use nekowriter_agent::client::Delta;
use nekowriter_core::ids::{ModelRequestId, RunId, VersionId};
use serde::Serialize;
use tokio::sync::watch;
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Preview {
    pub run_id: RunId,
    pub version_id: VersionId,
    pub request_id: ModelRequestId,
    /// The answer's text so far.
    pub text: String,
    /// The tool calls so far, in the order they started. Their arguments are incomplete
    /// JSON until the answer ends.
    pub tool_calls: Vec<PreviewToolCall>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PreviewToolCall {
    /// The position the model service gave the call.
    pub index: u32,
    pub name: String,
    pub arguments: String,
}

/// The preview of the request in flight, written by the dispatcher and read by the event
/// streams. Cloning is cheap; clones share the preview.
#[derive(Clone, Debug)]
pub(crate) struct Previews(watch::Sender<Option<Preview>>);

impl Previews {
    pub(crate) fn new() -> Self {
        Self(watch::Sender::new(None))
    }

    /// A receiver that sees the current preview, if any, as a change, so a new subscriber
    /// gets it right away.
    pub(crate) fn subscribe(&self) -> watch::Receiver<Option<Preview>> {
        let mut receiver = self.0.subscribe();
        if receiver.borrow().is_some() {
            receiver.mark_changed();
        }
        receiver
    }

    /// Starts the preview of a request.
    pub(crate) fn start(&self, run_id: RunId, version_id: VersionId, request_id: ModelRequestId) {
        self.0.send_replace(Some(Preview {
            run_id,
            version_id,
            request_id,
            text: String::new(),
            tool_calls: Vec::new(),
        }));
    }

    /// Adds a streamed piece of the answer.
    pub(crate) fn add(&self, delta: Delta<'_>) {
        self.0.send_if_modified(|preview| {
            let Some(preview) = preview else {
                return false;
            };
            match delta {
                Delta::Text(text) => preview.text.push_str(text),
                Delta::ToolCall {
                    index,
                    name,
                    arguments,
                } => {
                    let position = preview
                        .tool_calls
                        .iter()
                        .position(|call| call.index == index);
                    let position = position.unwrap_or_else(|| {
                        preview.tool_calls.push(PreviewToolCall {
                            index,
                            name: String::new(),
                            arguments: String::new(),
                        });
                        preview.tool_calls.len() - 1
                    });
                    let call = &mut preview.tool_calls[position];
                    if let Some(name) = name {
                        name.clone_into(&mut call.name);
                    }
                    call.arguments.push_str(arguments);
                }
            }
            true
        });
    }

    /// Ends the preview once the answer is complete, failed or abandoned.
    pub(crate) fn clear(&self) {
        self.0.send_replace(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deltas_accumulate_per_tool_call_until_cleared() {
        let previews = Previews::new();
        let mut receiver = previews.subscribe();
        previews.add(Delta::Text("ignored before the start"));
        assert!(receiver.borrow_and_update().is_none());

        previews.start(
            RunId::generate(),
            VersionId::generate(),
            ModelRequestId::generate(),
        );
        previews.add(Delta::Text("Plan"));
        previews.add(Delta::ToolCall {
            index: 0,
            name: Some("submit_preparation"),
            arguments: "{\"arti",
        });
        previews.add(Delta::Text("ning."));
        previews.add(Delta::ToolCall {
            index: 0,
            name: None,
            arguments: "facts\"",
        });

        let preview = receiver.borrow_and_update().clone().unwrap();
        assert_eq!(preview.text, "Planning.");
        assert_eq!(
            preview.tool_calls,
            vec![PreviewToolCall {
                index: 0,
                name: "submit_preparation".to_owned(),
                arguments: "{\"artifacts\"".to_owned(),
            }]
        );
        assert!(previews.subscribe().has_changed().unwrap());

        previews.clear();
        assert!(receiver.has_changed().unwrap());
        assert!(receiver.borrow_and_update().is_none());
        assert!(!previews.subscribe().has_changed().unwrap());
    }
}
