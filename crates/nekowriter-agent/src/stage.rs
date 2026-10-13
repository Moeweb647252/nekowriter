//! Stage contracts: what a run's request contains and what its answer may be
//! (`docs/product-spec.md` §8.3, §8.4).
//!
//! Every stage is a sequence of rounds with the same shape: a system prompt, a stage input,
//! the conversation so far, the stage's tools and exactly one submission tool. This module
//! builds that per stage from the stage input and the effective prompts, and parses the
//! submission. Adding a stage means adding its arm here.

use std::collections::BTreeSet;

use nekowriter_core::{
    preparation::{ArtifactKind, PreparationSubmission},
    project::WritingLanguage,
    prompt::{PromptKey, PromptVariable},
    run_message::{ArgumentsProblem, JsonType},
    stage::StageInput,
    template::{Template, TemplateError},
};
use serde_json::Value;

use crate::{
    model_text::model_text,
    render::{artifact_list, language_name},
    schema::{Property, Schema},
};

/// The name of the preparation submission tool.
pub const SUBMIT_PREPARATION: &str = "submit_preparation";

/// A tool declared to the model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolDefinition {
    pub name: &'static str,
    pub description: String,
    pub parameters: Schema,
}

/// What a round may end with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contract {
    pub stage: StageKind,
    /// The tool that ends the stage.
    pub submission: ToolDefinition,
}

impl Contract {
    /// Every tool the round declares, in a stable order.
    #[must_use]
    pub fn tools(&self) -> Vec<&ToolDefinition> {
        vec![&self.submission]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageKind {
    Preparation,
}

/// The effective texts of the stage's prompt pair: built-in, or overridden.
#[derive(Clone, Copy, Debug)]
pub struct PromptTexts<'a> {
    pub system: &'a str,
    pub user: &'a str,
}

/// Everything stage-specific in a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageRequest {
    pub system: String,
    pub user: String,
    pub contract: Contract,
}

/// A parsed stage submission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Submission {
    Preparation(PreparationSubmission),
}

/// The prompt pair a stage uses.
#[must_use]
pub fn prompt_key(input: &StageInput) -> PromptKey {
    match input {
        StageInput::Preparation { .. } => PromptKey::PreparationGenerate,
    }
}

/// Builds the stage's part of a request. The submission contract is appended to the system
/// prompt here, so a custom prompt cannot lose it.
pub fn build(
    input: &StageInput,
    language: WritingLanguage,
    prompts: PromptTexts<'_>,
) -> Result<StageRequest, TemplateError> {
    let language = language_name(language);
    match input {
        StageInput::Preparation { brief, required } => {
            let required_list = artifact_list(required.iter().copied());
            let value = |variable: PromptVariable| match variable {
                PromptVariable::WritingLanguage => language.as_str(),
                PromptVariable::NovelBrief => brief.as_str(),
                PromptVariable::RequiredArtifacts => required_list.as_str(),
            };
            let contract = Contract {
                stage: StageKind::Preparation,
                submission: preparation_tool(required),
            };
            let system = format!(
                "{}\n\n{}",
                render(prompts.system, value)?.trim_end(),
                model_text()
                    .contracts
                    .preparation
                    .render(&[("tool", SUBMIT_PREPARATION)])
            );
            Ok(StageRequest {
                system,
                user: render(prompts.user, value)?,
                contract,
            })
        }
    }
}

fn render<'v>(
    source: &str,
    value: impl Fn(PromptVariable) -> &'v str,
) -> Result<String, TemplateError> {
    let variables = [
        PromptVariable::WritingLanguage,
        PromptVariable::NovelBrief,
        PromptVariable::RequiredArtifacts,
    ];
    Template::parse(source)?.render(|name| {
        variables
            .iter()
            .find(|variable| variable.name() == name)
            .map(|variable| value(*variable))
    })
}

/// The preparation submission tool. Artifact kinds are limited to the ones required this
/// round, and the chapter plan is required exactly when the outline is.
fn preparation_tool(required: &BTreeSet<ArtifactKind>) -> ToolDefinition {
    let text = &model_text().tools.submit_preparation;
    let string = |allowed| Schema::String { allowed };
    let property = |name, description: String, required, schema| Property {
        name,
        description,
        required,
        schema,
    };
    let kinds = required
        .iter()
        .map(|kind| artifact_key(*kind).to_owned())
        .collect();
    let artifact = Schema::Object {
        properties: vec![
            property(
                "kind",
                text.artifact_kind.render(&[]),
                true,
                string(Some(kinds)),
            ),
            property(
                "content",
                text.artifact_content.render(&[]),
                true,
                string(None),
            ),
        ],
    };
    let chapter = Schema::Object {
        properties: vec![
            property("title", text.chapter_title.render(&[]), true, string(None)),
            property("goal", text.chapter_goal.render(&[]), true, string(None)),
        ],
    };
    ToolDefinition {
        name: SUBMIT_PREPARATION,
        description: text.description.render(&[]),
        parameters: Schema::Object {
            properties: vec![
                property(
                    "artifacts",
                    text.artifacts.render(&[]),
                    true,
                    Schema::Array {
                        items: Box::new(artifact),
                    },
                ),
                property(
                    "chapter_plan",
                    text.chapter_plan.render(&[]),
                    required.contains(&ArtifactKind::Outline),
                    Schema::Array {
                        items: Box::new(chapter),
                    },
                ),
            ],
        },
    }
}

/// The artifact's identifier in tool arguments; the same as its serde name.
fn artifact_key(kind: ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::Background => "background",
        ArtifactKind::Research => "research",
        ArtifactKind::Characters => "characters",
        ArtifactKind::Outline => "outline",
    }
}

/// Parses submission arguments that passed the contract's schema.
pub fn parse_submission(
    contract: &Contract,
    arguments: &Value,
) -> Result<Submission, ArgumentsProblem> {
    contract.submission.parameters.validate(arguments)?;
    match contract.stage {
        StageKind::Preparation => serde_json::from_value(arguments.clone())
            .map(Submission::Preparation)
            // The schema covers every field the submission reads, so this happens only if
            // the two disagree; it is reported like any other malformed arguments.
            .map_err(|_| ArgumentsProblem::WrongType {
                path: String::new(),
                expected: JsonType::Object,
            }),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::prompts::builtin;
    use nekowriter_core::prompt::PromptRole;

    fn request(required: &[ArtifactKind]) -> StageRequest {
        let key = PromptKey::PreparationGenerate;
        build(
            &StageInput::Preparation {
                brief: "A ghost story set in a lighthouse.".to_owned(),
                required: required.iter().copied().collect(),
            },
            WritingLanguage::En,
            PromptTexts {
                system: builtin(key, PromptRole::System),
                user: builtin(key, PromptRole::User),
            },
        )
        .unwrap()
    }

    #[test]
    fn the_preparation_request_carries_the_brief_the_language_and_the_contract() {
        let request = request(&ArtifactKind::ALL);
        assert!(request.user.contains("A ghost story set in a lighthouse."));
        assert!(request.system.contains(&language_name(WritingLanguage::En)));
        assert!(request.system.contains(SUBMIT_PREPARATION));
    }

    #[test]
    fn the_submission_schema_follows_the_required_artifacts() {
        let revision = request(&[ArtifactKind::Characters]);
        let schema = revision.contract.submission.parameters.to_json_schema();
        assert_eq!(
            schema["properties"]["artifacts"]["items"]["properties"]["kind"]["enum"],
            json!(["characters"])
        );
        assert_eq!(schema["required"], json!(["artifacts"]));

        let first = request(&ArtifactKind::ALL);
        let schema = first.contract.submission.parameters.to_json_schema();
        assert_eq!(schema["required"], json!(["artifacts", "chapter_plan"]));
    }

    #[test]
    fn a_valid_submission_parses_and_an_unrequired_kind_is_a_protocol_error() {
        let contract = request(&[ArtifactKind::Characters]).contract;
        let parsed = parse_submission(
            &contract,
            &json!({ "artifacts": [{ "kind": "characters", "content": "Mara" }] }),
        )
        .unwrap();
        let Submission::Preparation(submission) = parsed;
        assert_eq!(submission.artifacts[0].kind, ArtifactKind::Characters);

        assert!(matches!(
            parse_submission(
                &contract,
                &json!({ "artifacts": [{ "kind": "outline", "content": "x" }] }),
            ),
            Err(ArgumentsProblem::NotAllowed { .. })
        ));
    }
}
