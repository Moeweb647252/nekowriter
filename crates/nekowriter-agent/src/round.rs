//! Classifying the model's answer to a round (`docs/product-spec.md` §8.2): a stage
//! submission, or a protocol error with the correction the model receives.

use nekowriter_core::run_message::{ArgumentsProblem, Correction, ModelToolCall, ProtocolProblem};
use serde_json::Value;

use crate::stage::{Contract, Submission, parse_submission};

/// The model's answer: its text and its tool calls, as sent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AssistantTurn {
    pub content: Option<String>,
    pub tool_calls: Vec<ModelToolCall>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoundOutcome {
    /// The model called the submission tool with valid arguments.
    Submission {
        call: ModelToolCall,
        submission: Submission,
    },
    /// The answer broke the protocol. Every call gets an error result; the correction
    /// follows them.
    ProtocolError {
        calls: Vec<(ModelToolCall, ProtocolProblem)>,
        correction: Correction,
    },
}

#[must_use]
pub fn classify(contract: &Contract, turn: &AssistantTurn) -> RoundOutcome {
    match turn.tool_calls.as_slice() {
        [] => RoundOutcome::ProtocolError {
            calls: Vec::new(),
            correction: Correction::NoToolCall,
        },
        [call] => match check_call(contract, call) {
            Ok(submission) => RoundOutcome::Submission {
                call: call.clone(),
                submission,
            },
            Err(problem) => RoundOutcome::ProtocolError {
                calls: vec![(call.clone(), problem)],
                correction: Correction::ProtocolError,
            },
        },
        calls => RoundOutcome::ProtocolError {
            calls: calls
                .iter()
                .map(|call| (call.clone(), ProtocolProblem::ParallelToolCalls))
                .collect(),
            correction: Correction::ProtocolError,
        },
    }
}

fn check_call(contract: &Contract, call: &ModelToolCall) -> Result<Submission, ProtocolProblem> {
    if call.name != contract.submission.name {
        return Err(ProtocolProblem::UnknownTool {
            name: call.name.clone(),
        });
    }
    let invalid = |problem| ProtocolProblem::InvalidArguments { problem };
    let arguments: Value =
        serde_json::from_str(&call.arguments).map_err(|_| invalid(ArgumentsProblem::NotJson))?;
    parse_submission(contract, &arguments).map_err(invalid)
}

#[cfg(test)]
mod tests {
    use nekowriter_core::{
        preparation::ArtifactKind, project::WritingLanguage, prompt::PromptRole, stage::StageInput,
    };

    use super::*;
    use crate::{
        prompts::builtin,
        stage::{PromptTexts, SUBMIT_PREPARATION, build},
    };

    fn contract() -> Contract {
        let key = nekowriter_core::prompt::PromptKey::PreparationGenerate;
        build(
            &StageInput::Preparation {
                brief: "Brief".to_owned(),
                required: [ArtifactKind::Research].into_iter().collect(),
            },
            WritingLanguage::En,
            PromptTexts {
                system: builtin(key, PromptRole::System),
                user: builtin(key, PromptRole::User),
            },
        )
        .unwrap()
        .contract
    }

    fn call(name: &str, arguments: &str) -> ModelToolCall {
        ModelToolCall {
            provider_id: format!("call_{name}"),
            name: name.to_owned(),
            arguments: arguments.to_owned(),
        }
    }

    fn turn(calls: Vec<ModelToolCall>) -> AssistantTurn {
        AssistantTurn {
            content: None,
            tool_calls: calls,
        }
    }

    #[test]
    fn a_valid_submission_call_is_a_submission() {
        let outcome = classify(
            &contract(),
            &turn(vec![call(
                SUBMIT_PREPARATION,
                r#"{"artifacts":[{"kind":"research","content":"Notes"}]}"#,
            )]),
        );
        assert!(matches!(outcome, RoundOutcome::Submission { .. }));
    }

    #[test]
    fn an_answer_without_tool_calls_gets_a_correction_only() {
        assert_eq!(
            classify(&contract(), &AssistantTurn::default()),
            RoundOutcome::ProtocolError {
                calls: Vec::new(),
                correction: Correction::NoToolCall,
            }
        );
    }

    #[test]
    fn several_calls_are_all_refused() {
        let RoundOutcome::ProtocolError { calls, .. } = classify(
            &contract(),
            &turn(vec![call(SUBMIT_PREPARATION, "{}"), call("other", "{}")]),
        ) else {
            panic!("parallel calls are a protocol error");
        };
        assert_eq!(calls.len(), 2);
        assert!(
            calls
                .iter()
                .all(|(_, problem)| *problem == ProtocolProblem::ParallelToolCalls)
        );
    }

    #[test]
    fn unknown_tools_and_broken_arguments_are_protocol_errors() {
        let unknown = classify(&contract(), &turn(vec![call("search_web", "{}")]));
        assert!(matches!(
            unknown,
            RoundOutcome::ProtocolError { ref calls, .. }
                if matches!(calls[0].1, ProtocolProblem::UnknownTool { .. })
        ));
        let truncated = classify(
            &contract(),
            &turn(vec![call(SUBMIT_PREPARATION, "{\"artif")]),
        );
        assert!(matches!(
            truncated,
            RoundOutcome::ProtocolError { ref calls, .. }
                if calls[0].1 == ProtocolProblem::InvalidArguments { problem: ArgumentsProblem::NotJson }
        ));
    }
}
