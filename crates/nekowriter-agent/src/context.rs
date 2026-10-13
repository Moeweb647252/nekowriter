//! Assembling a request's messages and checking that they fit the model
//! (`docs/product-spec.md` §11).
//!
//! Order: the system prompt (role, task and contract), the folds of the completed stages as
//! one assistant message, the stage input, then the run's conversation so far. Tool
//! definitions travel next to the messages.

use nekowriter_core::run_message::TranscriptMessage;
use serde::Serialize;
use serde_json::{Value, json};

use crate::{client::ChatMessage, stage::StageRequest};

/// A request ready to send, apart from the model options.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlannedRequest {
    pub messages: Vec<ChatMessage>,
    /// The tool declarations, in the Chat Completions format.
    pub tools: Vec<Value>,
    /// See [`estimate_tokens`].
    pub estimated_input_tokens: u32,
}

/// The request does not fit the model's input budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextTooLarge {
    pub estimated_tokens: u32,
    pub limit_tokens: u32,
}

/// Plans a request and checks it against `input_budget` tokens. `folds` is the rendered
/// text of the completed stages, if there are any.
pub fn plan(
    stage: &StageRequest,
    folds: Option<String>,
    transcript: &[TranscriptMessage],
    input_budget: u32,
) -> Result<PlannedRequest, ContextTooLarge> {
    let mut messages = vec![ChatMessage::System {
        content: stage.system.clone(),
    }];
    if let Some(folds) = folds {
        // A fold is plain approved text from the model's side of the conversation, never
        // an instruction (§11).
        messages.push(ChatMessage::Assistant {
            content: Some(folds),
            tool_calls: Vec::new(),
        });
    }
    messages.push(ChatMessage::User {
        content: stage.user.clone(),
    });
    messages.extend(transcript.iter().map(ChatMessage::from));
    let tools: Vec<Value> = stage
        .contract
        .all_tools()
        .into_iter()
        .map(|tool| {
            json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters.to_json_schema(),
                }
            })
        })
        .collect();
    let serialized =
        serde_json::to_string(&(&messages, &tools)).expect("messages and tools always serialize");
    let estimated_input_tokens = estimate_tokens(&serialized);
    if estimated_input_tokens > input_budget {
        return Err(ContextTooLarge {
            estimated_tokens: estimated_input_tokens,
            limit_tokens: input_budget,
        });
    }
    Ok(PlannedRequest {
        messages,
        tools,
        estimated_input_tokens,
    })
}

/// A conservative token estimate without a tokenizer: one token per three bytes of UTF-8.
/// A Chinese character is three bytes and usually about one token; English averages about
/// four characters per token, so English text is overestimated, never underestimated.
#[must_use]
pub fn estimate_tokens(text: &str) -> u32 {
    u32::try_from(text.len().div_ceil(3)).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use nekowriter_core::{
        preparation::ArtifactKind,
        project::WritingLanguage,
        prompt::{PromptKey, PromptRole},
        stage::StageInput,
    };

    use super::*;
    use crate::{
        prompts::builtin,
        stage::{PromptTexts, build},
    };

    fn stage() -> StageRequest {
        let key = PromptKey::PreparationGenerate;
        build(
            &StageInput::Preparation {
                brief: "Brief".to_owned(),
                required: ArtifactKind::ALL.into_iter().collect(),
            },
            WritingLanguage::ZhCn,
            PromptTexts {
                system: builtin(key, PromptRole::System),
                user: builtin(key, PromptRole::User),
            },
        )
        .unwrap()
    }

    #[test]
    fn the_request_starts_with_the_system_prompt_and_stage_input_then_the_conversation() {
        let transcript = [TranscriptMessage::User {
            content: "Correction".to_owned(),
        }];
        let planned = plan(&stage(), None, &transcript, 1_000_000).unwrap();
        assert!(matches!(planned.messages[0], ChatMessage::System { .. }));
        assert!(matches!(planned.messages[1], ChatMessage::User { .. }));
        assert_eq!(
            planned.messages[2],
            ChatMessage::User {
                content: "Correction".to_owned()
            }
        );
        assert_eq!(planned.tools.len(), 2);
    }

    #[test]
    fn folds_follow_the_system_prompt_as_one_assistant_message() {
        let planned = plan(&stage(), Some("Folded".to_owned()), &[], 1_000_000).unwrap();
        assert_eq!(
            planned.messages[1],
            ChatMessage::Assistant {
                content: Some("Folded".to_owned()),
                tool_calls: Vec::new()
            }
        );
        assert!(matches!(planned.messages[2], ChatMessage::User { .. }));
    }

    #[test]
    fn a_request_over_the_budget_is_refused() {
        let error = plan(&stage(), None, &[], 10).unwrap_err();
        assert_eq!(error.limit_tokens, 10);
        assert!(error.estimated_tokens > 10);
    }

    #[test]
    fn the_estimate_rounds_up_bytes_divided_by_three() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 2);
        assert_eq!(estimate_tokens("\u{4e2d}"), 1);
    }
}
