//! `ScriptedProvider`: a queue-driven mock used by the runtime tests (spec/04 §7.1).
//!
//! It is compiled in normal builds (the server's test helpers live outside `cfg(test)`) but is
//! only used by tests and by the Demo provider's plumbing.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::retry::LlmError;
use crate::types::{
    AssistantTurn, ChatRequest, ProviderPayload, RemoteModel, StopReason, StreamEvent, ToolCallOut,
    Usage,
};

/// One scripted response.
#[derive(Debug, Clone, Default)]
pub struct ScriptStep {
    pub text: String,
    pub reasoning: String,
    pub tool_calls: Vec<(String, String, Value)>,
    pub stop: Option<StopReason>,
    pub usage: Usage,
    /// Delay before the step starts, and between events.
    pub delay: Duration,
    pub delay_per_event: Duration,
}

impl ScriptStep {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self
    }

    pub fn reasoning(mut self, reasoning: impl Into<String>) -> Self {
        self.reasoning = reasoning.into();
        self
    }

    /// Adds a tool call with already-serialized arguments.
    pub fn tool(mut self, name: &str, arguments: Value) -> Self {
        let index = self.tool_calls.len();
        self.tool_calls
            .push((format!("call_{index}"), name.to_owned(), arguments));
        self
    }

    pub fn stop(mut self, stop: StopReason) -> Self {
        self.stop = Some(stop);
        self
    }

    pub fn usage(mut self, usage: Usage) -> Self {
        self.usage = usage;
        self
    }

    pub fn delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    pub fn delay_per_event(mut self, delay: Duration) -> Self {
        self.delay_per_event = delay;
        self
    }
}

/// What the scripted provider should do when the queue is exhausted.
#[derive(Clone)]
enum Exhausted {
    /// Repeat the last step forever (useful for "keeps writing" scenarios).
    RepeatLast,
    /// Return this error.
    Fail(LlmError),
}

pub struct ScriptedState {
    steps: VecDeque<ScriptStep>,
    requests: Vec<ChatRequest>,
    /// Bodies produced by the OpenAI builder, for byte-identical request assertions.
    bodies: Vec<Value>,
    failures: VecDeque<LlmError>,
    exhausted: Exhausted,
}

/// A deterministic, queue-driven mock provider.
#[derive(Clone)]
pub struct ScriptedProvider {
    state: Arc<Mutex<ScriptedState>>,
}

impl ScriptedProvider {
    pub fn new(steps: Vec<ScriptStep>) -> Self {
        Self {
            state: Arc::new(Mutex::new(ScriptedState {
                steps: steps.into(),
                requests: Vec::new(),
                bodies: Vec::new(),
                failures: VecDeque::new(),
                exhausted: Exhausted::RepeatLast,
            })),
        }
    }

    /// Fails the next `count` calls with the given error before scripted steps resume.
    pub fn failing(self, error: LlmError, count: usize) -> Self {
        {
            let mut state = self.state.lock().expect("scripted state lock");
            for _ in 0..count {
                state.failures.push_back(error.clone());
            }
        }
        self
    }

    /// Makes the provider fail permanently once the queue is empty.
    pub fn fail_when_exhausted(self, error: LlmError) -> Self {
        self.state.lock().expect("scripted state lock").exhausted = Exhausted::Fail(error);
        self
    }

    /// Every request the provider received, in order.
    pub fn requests(&self) -> Vec<ChatRequest> {
        self.state
            .lock()
            .expect("scripted state lock")
            .requests
            .clone()
    }

    /// The JSON bodies the OpenAI builder produced, in order.
    pub fn bodies(&self) -> Vec<Value> {
        self.state
            .lock()
            .expect("scripted state lock")
            .bodies
            .clone()
    }

    /// Streams one scripted response.
    pub async fn run(
        &self,
        request: &ChatRequest,
        on_event: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<AssistantTurn, LlmError> {
        let body = crate::openai::build_body(request, &request.options);

        let failure = {
            let mut state = self.state.lock().expect("scripted state lock");
            state.requests.push(request.clone());
            state.bodies.push(body);
            state.failures.pop_front()
        };

        if let Some(error) = failure {
            return Err(error);
        }

        // The guard is scoped out before the first await point.
        let step = {
            let mut state = self.state.lock().expect("scripted state lock");
            match state.steps.pop_front() {
                Some(step) => step,
                None => match state.exhausted.clone() {
                    Exhausted::RepeatLast => ScriptStep::new(),
                    Exhausted::Fail(error) => return Err(error),
                },
            }
        };

        if !step.delay.is_zero() {
            tokio::time::sleep(step.delay).await;
        }

        // Emit reasoning, then text in chunks, then tool calls.
        if !step.reasoning.is_empty() {
            for chunk in chunk_text(&step.reasoning, 24) {
                on_event(StreamEvent::ReasoningDelta {
                    index: 0,
                    text: chunk,
                });
                sleep_per_event(&step).await;
            }
        }
        if !step.text.is_empty() {
            for chunk in chunk_text(&step.text, 24) {
                on_event(StreamEvent::TextDelta {
                    index: 0,
                    text: chunk,
                });
                sleep_per_event(&step).await;
            }
        }

        let mut calls = Vec::new();
        let mut replay_calls = Vec::new();
        for (ordinal, (id, name, arguments)) in step.tool_calls.iter().enumerate() {
            let input_text = serde_json::to_string(arguments).unwrap_or_else(|_| "{}".to_owned());
            on_event(StreamEvent::ToolCallStart {
                call_index: ordinal as u32,
                id: id.clone(),
                name: name.clone(),
            });
            // Emit the argument text in fragments so preview extraction is exercised.
            for chunk in chunk_text(&input_text, 16) {
                on_event(StreamEvent::ToolCallArgsDelta {
                    call_index: ordinal as u32,
                    fragment: chunk,
                });
                sleep_per_event(&step).await;
            }
            calls.push(ToolCallOut {
                id: id.clone(),
                name: name.clone(),
                input_text: input_text.clone(),
            });
            replay_calls.push(json!({
                "index": ordinal,
                "id": id,
                "type": "function",
                "function": { "name": name, "arguments": input_text },
            }));
        }

        on_event(StreamEvent::Usage(step.usage));

        let stop = step.stop.clone().unwrap_or(if calls.is_empty() {
            StopReason::EndTurn
        } else {
            StopReason::ToolUse
        });

        Ok(AssistantTurn {
            text: step.text.clone(),
            reasoning: step.reasoning.clone(),
            tool_calls: calls,
            stop_reason: stop,
            usage: step.usage,
            payload: ProviderPayload::Openai {
                content: if step.text.is_empty() {
                    None
                } else {
                    Some(step.text)
                },
                reasoning_content: if step.reasoning.is_empty() {
                    None
                } else {
                    Some(step.reasoning)
                },
                tool_calls: replay_calls,
            },
            request_id: Some("scripted".to_owned()),
        })
    }

    pub async fn list_models(&self) -> Result<Vec<RemoteModel>, LlmError> {
        Ok(vec![RemoteModel {
            id: "scripted-model".to_owned(),
            display_name: Some("Scripted".to_owned()),
            context_window: Some(200_000),
            max_output_tokens: Some(16_384),
        }])
    }
}

async fn sleep_per_event(step: &ScriptStep) {
    if !step.delay_per_event.is_zero() {
        tokio::time::sleep(step.delay_per_event).await;
    }
}

/// Splits text into chunks of at most `size` characters.
pub fn chunk_text(text: &str, size: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    chars
        .chunks(size)
        .map(|chunk| chunk.iter().collect())
        .collect()
}
