//! Rendering stored data into model text.
//!
//! Tool results, review outcomes and corrections are stored as data and rendered here when
//! they are sealed into a request (`nekowriter_core::run_message`). A tool result is a JSON
//! object with a `status` and a model-facing `message`.

use nekowriter_core::{
    preparation::{ArtifactKind, SubmissionProblem},
    project::WritingLanguage,
    run_message::{
        ArgumentsProblem, Correction, JsonType, ProtocolProblem, ToolOutcome, TranscriptMessage,
        UnsealedMessage,
    },
};
use serde_json::json;

use crate::model_text::model_text;

/// The writing language, named in the model's language.
#[must_use]
pub fn language_name(language: WritingLanguage) -> String {
    let languages = &model_text().languages;
    match language {
        WritingLanguage::ZhCn => languages.zh_cn.render(&[]),
        WritingLanguage::En => languages.en.render(&[]),
    }
}

#[must_use]
pub fn artifact_name(kind: ArtifactKind) -> String {
    let artifacts = &model_text().artifacts;
    match kind {
        ArtifactKind::Background => artifacts.background.render(&[]),
        ArtifactKind::Research => artifacts.research.render(&[]),
        ArtifactKind::Characters => artifacts.characters.render(&[]),
        ArtifactKind::Outline => artifacts.outline.render(&[]),
    }
}

/// Artifact names joined into one phrase.
#[must_use]
pub fn artifact_list(kinds: impl IntoIterator<Item = ArtifactKind>) -> String {
    let names: Vec<_> = kinds.into_iter().map(artifact_name).collect();
    names.join(&model_text().list.separator.render(&[]))
}

/// Renders a stored message for the request that first carries it.
#[must_use]
pub fn unsealed(message: &UnsealedMessage, submission_tool: &str) -> TranscriptMessage {
    match message {
        UnsealedMessage::ToolResult {
            provider_call_id,
            outcome,
        } => TranscriptMessage::Tool {
            provider_call_id: provider_call_id.clone(),
            content: tool_outcome(outcome),
        },
        UnsealedMessage::PreparationReviewOutcome { rejected, stale } => {
            let review = &model_text().review;
            let mut items: Vec<String> = rejected
                .iter()
                .map(|item| {
                    review.rejected.render(&[
                        ("artifact", &artifact_name(item.artifact)),
                        ("feedback", &item.feedback),
                    ])
                })
                .collect();
            items.extend(
                stale
                    .iter()
                    .map(|kind| review.stale.render(&[("artifact", &artifact_name(*kind))])),
            );
            let items = items.join(&review.item_separator.render(&[]));
            TranscriptMessage::User {
                content: review.preparation_outcome.render(&[("items", &items)]),
            }
        }
        UnsealedMessage::Correction { correction } => {
            let corrections = &model_text().corrections;
            let content = match correction {
                Correction::NoToolCall => corrections
                    .no_tool_call
                    .render(&[("tool", submission_tool)]),
                Correction::ProtocolError => corrections.protocol_error.render(&[]),
                Correction::SubmissionRejected => corrections.submission_rejected.render(&[]),
            };
            TranscriptMessage::User { content }
        }
    }
}

/// A tool result: `{"status": ..., "message": ...}`.
#[must_use]
pub fn tool_outcome(outcome: &ToolOutcome) -> String {
    let results = &model_text().results;
    let (status, message) = match outcome {
        ToolOutcome::SubmissionAccepted => ("accepted", results.submission_accepted.render(&[])),
        ToolOutcome::SubmissionRejected { problem } => (
            "rejected",
            results
                .submission_rejected
                .render(&[("problem", &submission_problem(problem))]),
        ),
        ToolOutcome::ProtocolError { problem } => (
            "error",
            results
                .protocol_error
                .render(&[("problem", &protocol_problem(problem))]),
        ),
    };
    json!({ "status": status, "message": message }).to_string()
}

#[must_use]
pub fn submission_problem(problem: &SubmissionProblem) -> String {
    let text = &model_text().submission_problems;
    let number = |number: &u32| number.to_string();
    match problem {
        SubmissionProblem::MissingArtifacts { kinds } => text
            .missing_artifacts
            .render(&[("artifacts", &artifact_list(kinds.iter().copied()))]),
        SubmissionProblem::UnexpectedArtifacts { kinds } => text
            .unexpected_artifacts
            .render(&[("artifacts", &artifact_list(kinds.iter().copied()))]),
        SubmissionProblem::DuplicateArtifact { artifact } => text
            .duplicate_artifact
            .render(&[("artifact", &artifact_name(*artifact))]),
        SubmissionProblem::EmptyArtifact { artifact } => text
            .empty_artifact
            .render(&[("artifact", &artifact_name(*artifact))]),
        SubmissionProblem::ArtifactTooLong {
            artifact,
            max_chars,
        } => text.artifact_too_long.render(&[
            ("artifact", &artifact_name(*artifact)),
            ("max_chars", &number(max_chars)),
        ]),
        SubmissionProblem::ChapterPlanMissing => text.chapter_plan_missing.render(&[]),
        SubmissionProblem::ChapterPlanNotExpected => text.chapter_plan_not_expected.render(&[]),
        SubmissionProblem::ChapterPlanEmpty => text.chapter_plan_empty.render(&[]),
        SubmissionProblem::ChapterPlanTooLong { max_items } => text
            .chapter_plan_too_long
            .render(&[("max_items", &number(max_items))]),
        SubmissionProblem::ChapterTitleEmpty { number: chapter } => text
            .chapter_title_empty
            .render(&[("number", &number(chapter))]),
        SubmissionProblem::ChapterTitleNotSingleLine { number: chapter } => text
            .chapter_title_not_single_line
            .render(&[("number", &number(chapter))]),
        SubmissionProblem::ChapterTitleTooLong {
            number: chapter,
            max_chars,
        } => text.chapter_title_too_long.render(&[
            ("number", &number(chapter)),
            ("max_chars", &number(max_chars)),
        ]),
        SubmissionProblem::ChapterGoalEmpty { number: chapter } => text
            .chapter_goal_empty
            .render(&[("number", &number(chapter))]),
        SubmissionProblem::ChapterGoalTooLong {
            number: chapter,
            max_chars,
        } => text.chapter_goal_too_long.render(&[
            ("number", &number(chapter)),
            ("max_chars", &number(max_chars)),
        ]),
    }
}

#[must_use]
pub fn protocol_problem(problem: &ProtocolProblem) -> String {
    let text = &model_text().protocol_problems;
    match problem {
        ProtocolProblem::UnknownTool { name } => text.unknown_tool.render(&[("name", name)]),
        ProtocolProblem::ParallelToolCalls => text.parallel_tool_calls.render(&[]),
        ProtocolProblem::InvalidArguments { problem } => text
            .invalid_arguments
            .render(&[("problem", &arguments_problem(problem))]),
    }
}

#[must_use]
pub fn arguments_problem(problem: &ArgumentsProblem) -> String {
    let text = &model_text().arguments_problems;
    let location = |path: &str| {
        if path.is_empty() {
            text.root.render(&[])
        } else {
            text.field.render(&[("path", path)])
        }
    };
    match problem {
        ArgumentsProblem::NotJson => text.not_json.render(&[]),
        ArgumentsProblem::MissingField { path, field } => text
            .missing_field
            .render(&[("location", &location(path)), ("field", field)]),
        ArgumentsProblem::WrongType { path, expected } => text.wrong_type.render(&[
            ("location", &location(path)),
            ("expected", &json_type(*expected)),
        ]),
        ArgumentsProblem::NotAllowed {
            path,
            value,
            allowed,
        } => text.not_allowed.render(&[
            ("location", &location(path)),
            ("value", value),
            ("allowed", &allowed.join(", ")),
        ]),
    }
}

fn json_type(json_type: JsonType) -> String {
    let types = &model_text().json_types;
    match json_type {
        JsonType::Object => types.object.render(&[]),
        JsonType::Array => types.array.render(&[]),
        JsonType::String => types.string.render(&[]),
        JsonType::Integer => types.integer.render(&[]),
        JsonType::Number => types.number.render(&[]),
        JsonType::Boolean => types.boolean.render(&[]),
    }
}

#[cfg(test)]
mod tests {
    use nekowriter_core::run_message::RejectedArtifact;

    use super::*;

    // Together these tests render every message kind, so every placeholder in the resource
    // file is checked against the values the code passes.

    #[test]
    fn every_submission_problem_renders() {
        let artifacts = vec![ArtifactKind::Background, ArtifactKind::Outline];
        let problems = [
            SubmissionProblem::MissingArtifacts {
                kinds: artifacts.clone(),
            },
            SubmissionProblem::UnexpectedArtifacts { kinds: artifacts },
            SubmissionProblem::DuplicateArtifact {
                artifact: ArtifactKind::Research,
            },
            SubmissionProblem::EmptyArtifact {
                artifact: ArtifactKind::Research,
            },
            SubmissionProblem::ArtifactTooLong {
                artifact: ArtifactKind::Characters,
                max_chars: 10,
            },
            SubmissionProblem::ChapterPlanMissing,
            SubmissionProblem::ChapterPlanNotExpected,
            SubmissionProblem::ChapterPlanEmpty,
            SubmissionProblem::ChapterPlanTooLong { max_items: 5 },
            SubmissionProblem::ChapterTitleEmpty { number: 2 },
            SubmissionProblem::ChapterTitleNotSingleLine { number: 2 },
            SubmissionProblem::ChapterTitleTooLong {
                number: 2,
                max_chars: 3,
            },
            SubmissionProblem::ChapterGoalEmpty { number: 2 },
            SubmissionProblem::ChapterGoalTooLong {
                number: 2,
                max_chars: 3,
            },
        ];
        for problem in problems {
            let rendered = tool_outcome(&ToolOutcome::SubmissionRejected { problem });
            assert!(rendered.contains("\"rejected\""), "{rendered}");
        }
        let accepted = tool_outcome(&ToolOutcome::SubmissionAccepted);
        assert!(accepted.contains("\"accepted\""), "{accepted}");
    }

    #[test]
    fn every_protocol_problem_renders() {
        let arguments = [
            ArgumentsProblem::NotJson,
            ArgumentsProblem::MissingField {
                path: String::new(),
                field: "artifacts".to_owned(),
            },
            ArgumentsProblem::WrongType {
                path: "artifacts".to_owned(),
                expected: JsonType::Array,
            },
            ArgumentsProblem::NotAllowed {
                path: "artifacts[0].kind".to_owned(),
                value: "x".to_owned(),
                allowed: vec!["outline".to_owned()],
            },
        ];
        let problems = arguments
            .into_iter()
            .map(|problem| ProtocolProblem::InvalidArguments { problem })
            .chain([
                ProtocolProblem::UnknownTool {
                    name: "x".to_owned(),
                },
                ProtocolProblem::ParallelToolCalls,
            ]);
        for problem in problems {
            let rendered = tool_outcome(&ToolOutcome::ProtocolError { problem });
            assert!(rendered.contains("\"error\""), "{rendered}");
        }
        for json_type in [
            JsonType::Integer,
            JsonType::Number,
            JsonType::Boolean,
            JsonType::Object,
            JsonType::String,
        ] {
            assert_ne!(super::json_type(json_type), "");
        }
    }

    #[test]
    fn corrections_review_outcomes_and_fixed_texts_render() {
        for correction in [
            Correction::NoToolCall,
            Correction::ProtocolError,
            Correction::SubmissionRejected,
        ] {
            let message = unsealed(
                &UnsealedMessage::Correction { correction },
                "submit_preparation",
            );
            assert!(matches!(message, TranscriptMessage::User { .. }));
        }
        let outcome = unsealed(
            &UnsealedMessage::PreparationReviewOutcome {
                rejected: vec![RejectedArtifact {
                    artifact: ArtifactKind::Outline,
                    feedback: "More conflict.".to_owned(),
                }],
                stale: vec![ArtifactKind::Characters],
            },
            "submit_preparation",
        );
        let TranscriptMessage::User { content } = outcome else {
            panic!("a review outcome is a user message");
        };
        assert!(content.contains("More conflict."));
        assert_ne!(language_name(WritingLanguage::En), "");
        let test = &model_text().connection_test;
        assert_ne!(test.system.render(&[]), "");
        assert_ne!(test.user.render(&[]), "");
    }

    #[test]
    fn a_no_tool_call_correction_names_the_submission_tool() {
        let TranscriptMessage::User { content } = unsealed(
            &UnsealedMessage::Correction {
                correction: Correction::NoToolCall,
            },
            "submit_preparation",
        ) else {
            panic!("a correction is a user message");
        };
        assert!(content.contains("submit_preparation"));
    }
}
