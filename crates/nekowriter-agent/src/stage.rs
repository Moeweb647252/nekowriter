//! Stage contracts: what a run's request contains and what its answer may be
//! (`docs/product-spec.md` §8.3, §8.4).
//!
//! Every stage is a sequence of rounds with the same shape: a system prompt, the folds of
//! the stages before it, a stage input, the conversation so far, the stage's tools and
//! exactly one submission tool. This module builds that per stage from the stage input and
//! the effective prompts, and parses submissions and tool calls. Adding a stage means
//! adding its arm here.

use std::collections::BTreeSet;

use nekowriter_core::{
    body::BodyEdit,
    call::{CallRequest, StageSubmission},
    chapter::{ChapterInput, ChapterKind},
    ids::ChapterId,
    preparation::ArtifactKind,
    project::WritingLanguage,
    prompt::{PromptKey, PromptVariable},
    question::Question,
    run_message::{ArgumentsProblem, JsonType},
    stage::StageInput,
    template::{Template, TemplateError},
};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{
    model_text::model_text,
    render::{artifact_list, current_body, language_name},
    schema::{Property, Schema},
};

/// The name of the preparation submission tool.
pub const SUBMIT_PREPARATION: &str = "submit_preparation";
/// The name of the chapter outline submission tool.
pub const SUBMIT_CHAPTER_OUTLINE: &str = "submit_chapter_outline";
/// The name of the chapter submission tool.
pub const SUBMIT_CHAPTER: &str = "submit_chapter";
pub const APPEND_CONTENT: &str = "append_content";
pub const REPLACE_CONTENT: &str = "replace_content";
pub const ASK_USER: &str = "ask_user";

/// A tool declared to the model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolDefinition {
    pub name: &'static str,
    pub description: String,
    pub parameters: Schema,
}

/// What a round may do: call one of the stage's work tools, or end the stage by
/// submitting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contract {
    pub stage: StageKind,
    /// The tools that do work within the stage.
    pub tools: Vec<WorkTool>,
    /// The tool that ends the stage.
    pub submission: ToolDefinition,
}

impl Contract {
    /// Every tool the round declares, in a stable order.
    #[must_use]
    pub fn all_tools(&self) -> Vec<ToolDefinition> {
        self.tools
            .iter()
            .map(|tool| tool.definition())
            .chain([self.submission.clone()])
            .collect()
    }
}

/// A tool that does work within a stage without ending it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkTool {
    AppendContent,
    ReplaceContent,
    AskUser,
}

impl WorkTool {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::AppendContent => APPEND_CONTENT,
            Self::ReplaceContent => REPLACE_CONTENT,
            Self::AskUser => ASK_USER,
        }
    }

    #[must_use]
    pub fn definition(self) -> ToolDefinition {
        match self {
            Self::AppendContent => append_tool(),
            Self::ReplaceContent => replace_tool(),
            Self::AskUser => ask_tool(),
        }
    }

    /// Parses the arguments of a call to this tool.
    pub fn parse(self, arguments: &Value) -> Result<CallRequest, ArgumentsProblem> {
        #[derive(serde::Deserialize)]
        struct Append {
            content: String,
        }
        #[derive(serde::Deserialize)]
        struct Replace {
            old_str: String,
            new_str: String,
        }
        #[derive(serde::Deserialize)]
        struct Ask {
            questions: Vec<Question>,
        }
        let schema = self.definition().parameters;
        match self {
            Self::AppendContent => {
                read::<Append>(&schema, arguments).map(|append| CallRequest::Edit {
                    edit: BodyEdit::Append {
                        content: append.content,
                    },
                })
            }
            Self::ReplaceContent => {
                read::<Replace>(&schema, arguments).map(|replace| CallRequest::Edit {
                    edit: BodyEdit::Replace {
                        old_str: replace.old_str,
                        new_str: replace.new_str,
                    },
                })
            }
            Self::AskUser => read::<Ask>(&schema, arguments).map(|ask| CallRequest::Ask {
                questions: ask.questions,
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageKind {
    Preparation,
    ChapterOutline,
    ChapterBody,
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

/// The prompt pair a stage uses.
#[must_use]
pub fn prompt_key(input: &StageInput) -> PromptKey {
    match input {
        StageInput::Preparation { .. } => PromptKey::PreparationGenerate,
        StageInput::ChapterOutline { .. } => PromptKey::ChapterOutline,
        StageInput::ChapterBody { .. } => PromptKey::ChapterWrite,
    }
}

/// Builds the stage's part of a request. The contract is appended to the system prompt,
/// and the current body to the user prompt, so a custom prompt cannot lose them.
pub fn build(
    input: &StageInput,
    language: WritingLanguage,
    prompts: PromptTexts<'_>,
) -> Result<StageRequest, TemplateError> {
    let language = language_name(language);
    let contracts = &model_text().contracts;
    match input {
        StageInput::Preparation { brief, required } => {
            let required_list = artifact_list(required.iter().copied());
            let values = [
                (PromptVariable::WritingLanguage, language.as_str()),
                (PromptVariable::NovelBrief, brief.as_str()),
                (PromptVariable::RequiredArtifacts, required_list.as_str()),
            ];
            let contract = contracts
                .preparation
                .render(&[("tool", SUBMIT_PREPARATION)]);
            Ok(StageRequest {
                system: with_section(&render(prompts.system, &values)?, &with_asking(&contract)),
                user: render(prompts.user, &values)?,
                contract: Contract {
                    stage: StageKind::Preparation,
                    tools: vec![WorkTool::AskUser],
                    submission: preparation_tool(required),
                },
            })
        }
        StageInput::ChapterOutline { chapter, guidance } => {
            let chapter_values = ChapterValues::new(chapter, guidance);
            let values = chapter_values.with_language(&language);
            let id = chapter.id.to_string();
            let contract = contracts
                .chapter_outline
                .render(&[("tool", SUBMIT_CHAPTER_OUTLINE), ("chapter_id", &id)]);
            Ok(StageRequest {
                system: with_section(&render(prompts.system, &values)?, &with_asking(&contract)),
                user: render(prompts.user, &values)?,
                contract: Contract {
                    stage: StageKind::ChapterOutline,
                    tools: vec![WorkTool::AskUser],
                    submission: outline_tool(chapter.id),
                },
            })
        }
        StageInput::ChapterBody {
            chapter,
            guidance,
            body,
        } => {
            let chapter_values = ChapterValues::new(chapter, guidance);
            let values = chapter_values.with_language(&language);
            let id = chapter.id.to_string();
            let contract = contracts.chapter_body.render(&[
                ("append", APPEND_CONTENT),
                ("replace", REPLACE_CONTENT),
                ("submit", SUBMIT_CHAPTER),
                ("chapter_id", &id),
            ]);
            Ok(StageRequest {
                system: with_section(&render(prompts.system, &values)?, &with_asking(&contract)),
                user: with_section(&render(prompts.user, &values)?, &current_body(body)),
                contract: Contract {
                    stage: StageKind::ChapterBody,
                    tools: vec![
                        WorkTool::AppendContent,
                        WorkTool::ReplaceContent,
                        WorkTool::AskUser,
                    ],
                    submission: chapter_tool(chapter.id),
                },
            })
        }
    }
}

/// The values of a chapter stage's prompt variables. Empty goals and guidance are named as
/// such, so the model does not read a blank as something missing.
struct ChapterValues {
    kind: String,
    number: String,
    title: String,
    goal: String,
    guidance: String,
}

impl ChapterValues {
    fn new(chapter: &ChapterInput, guidance: &str) -> Self {
        let kinds = &model_text().chapter_kinds;
        let or_none = |text: &str| {
            if text.trim().is_empty() {
                model_text().chapters.none.render(&[])
            } else {
                text.to_owned()
            }
        };
        Self {
            kind: match chapter.kind {
                ChapterKind::Main => kinds.main.render(&[]),
                ChapterKind::Extra => kinds.extra.render(&[]),
            },
            number: chapter.number.to_string(),
            title: chapter.title.clone(),
            goal: or_none(&chapter.goal),
            guidance: or_none(guidance),
        }
    }

    fn with_language<'a>(&'a self, language: &'a str) -> [(PromptVariable, &'a str); 6] {
        [
            (PromptVariable::WritingLanguage, language),
            (PromptVariable::ChapterKind, self.kind.as_str()),
            (PromptVariable::ChapterNumber, self.number.as_str()),
            (PromptVariable::ChapterTitle, self.title.as_str()),
            (PromptVariable::ChapterGoal, self.goal.as_str()),
            (PromptVariable::ChapterGuidance, self.guidance.as_str()),
        ]
    }
}

/// The stage's contract followed by when and how to ask the user (every writer can).
fn with_asking(contract: &str) -> String {
    let asking = model_text()
        .contracts
        .ask_user
        .render(&[("tool", ASK_USER)]);
    with_section(contract, &asking)
}

fn with_section(text: &str, section: &str) -> String {
    format!("{}\n\n{}", text.trim_end(), section.trim())
}

fn render(source: &str, values: &[(PromptVariable, &str)]) -> Result<String, TemplateError> {
    Template::parse(source)?.render(|name| {
        values
            .iter()
            .find(|(variable, _)| variable.name() == name)
            .map(|(_, value)| *value)
    })
}

fn property(name: &'static str, description: String, required: bool, schema: Schema) -> Property {
    Property {
        name,
        description,
        required,
        schema,
    }
}

fn string() -> Schema {
    Schema::String { allowed: None }
}

/// A string that must be the chapter's ID, so the model cannot address another chapter.
fn chapter_id(chapter: ChapterId) -> Schema {
    Schema::String {
        allowed: Some(vec![chapter.to_string()]),
    }
}

/// The preparation submission tool. Artifact kinds are limited to the ones required this
/// round, and the chapter plan is required exactly when the outline is.
fn preparation_tool(required: &BTreeSet<ArtifactKind>) -> ToolDefinition {
    let text = &model_text().tools.submit_preparation;
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
                Schema::String {
                    allowed: Some(kinds),
                },
            ),
            property("content", text.artifact_content.render(&[]), true, string()),
        ],
    };
    let chapter = Schema::Object {
        properties: vec![
            property("title", text.chapter_title.render(&[]), true, string()),
            property("goal", text.chapter_goal.render(&[]), true, string()),
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

fn outline_tool(chapter: ChapterId) -> ToolDefinition {
    let text = &model_text().tools.submit_chapter_outline;
    ToolDefinition {
        name: SUBMIT_CHAPTER_OUTLINE,
        description: text.description.render(&[]),
        parameters: Schema::Object {
            properties: vec![
                property(
                    "chapter_id",
                    text.chapter_id.render(&[]),
                    true,
                    chapter_id(chapter),
                ),
                property("content", text.content.render(&[]), true, string()),
            ],
        },
    }
}

fn chapter_tool(chapter: ChapterId) -> ToolDefinition {
    let text = &model_text().tools.submit_chapter;
    ToolDefinition {
        name: SUBMIT_CHAPTER,
        description: text.description.render(&[]),
        parameters: Schema::Object {
            properties: vec![
                property(
                    "chapter_id",
                    text.chapter_id.render(&[]),
                    true,
                    chapter_id(chapter),
                ),
                property("summary", text.summary.render(&[]), true, string()),
            ],
        },
    }
}

fn append_tool() -> ToolDefinition {
    let text = &model_text().tools.append_content;
    ToolDefinition {
        name: APPEND_CONTENT,
        description: text.description.render(&[]),
        parameters: Schema::Object {
            properties: vec![property(
                "content",
                text.content.render(&[]),
                true,
                string(),
            )],
        },
    }
}

fn replace_tool() -> ToolDefinition {
    let text = &model_text().tools.replace_content;
    ToolDefinition {
        name: REPLACE_CONTENT,
        description: text.description.render(&[]),
        parameters: Schema::Object {
            properties: vec![
                property("old_str", text.old_str.render(&[]), true, string()),
                property("new_str", text.new_str.render(&[]), true, string()),
            ],
        },
    }
}

fn ask_tool() -> ToolDefinition {
    let text = &model_text().tools.ask_user;
    let option = Schema::Object {
        properties: vec![
            property("id", text.option_id.render(&[]), true, string()),
            property("label", text.option_label.render(&[]), true, string()),
        ],
    };
    let kinds = ["single", "multiple", "text"].map(str::to_owned).to_vec();
    let question = Schema::Object {
        properties: vec![
            property("id", text.question_id.render(&[]), true, string()),
            property("title", text.title.render(&[]), true, string()),
            property(
                "kind",
                text.kind.render(&[]),
                true,
                Schema::String {
                    allowed: Some(kinds),
                },
            ),
            property(
                "options",
                text.options.render(&[]),
                true,
                Schema::Array {
                    items: Box::new(option),
                },
            ),
            property("required", text.required.render(&[]), true, Schema::Boolean),
        ],
    };
    ToolDefinition {
        name: ASK_USER,
        description: text.description.render(&[]),
        parameters: Schema::Object {
            properties: vec![property(
                "questions",
                text.questions.render(&[]),
                true,
                Schema::Array {
                    items: Box::new(question),
                },
            )],
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

/// Reads arguments that passed `schema` into their type.
fn read<T: DeserializeOwned>(schema: &Schema, arguments: &Value) -> Result<T, ArgumentsProblem> {
    schema.validate(arguments)?;
    serde_json::from_value(arguments.clone())
        // The schema covers every field the type reads, so this happens only if the two
        // disagree; it is reported like any other malformed arguments.
        .map_err(|_| ArgumentsProblem::WrongType {
            path: String::new(),
            expected: JsonType::Object,
        })
}

/// Parses submission arguments.
pub fn parse_submission(
    contract: &Contract,
    arguments: &Value,
) -> Result<StageSubmission, ArgumentsProblem> {
    let schema = &contract.submission.parameters;
    match contract.stage {
        StageKind::Preparation => {
            read(schema, arguments).map(|submission| StageSubmission::Preparation { submission })
        }
        StageKind::ChapterOutline => {
            read(schema, arguments).map(|submission| StageSubmission::ChapterOutline { submission })
        }
        StageKind::ChapterBody => {
            read(schema, arguments).map(|submission| StageSubmission::Chapter { submission })
        }
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
        let StageSubmission::Preparation { submission } = parsed else {
            panic!("a preparation submission");
        };
        assert_eq!(submission.artifacts[0].kind, ArtifactKind::Characters);

        assert!(matches!(
            parse_submission(
                &contract,
                &json!({ "artifacts": [{ "kind": "outline", "content": "x" }] }),
            ),
            Err(ArgumentsProblem::NotAllowed { .. })
        ));
    }

    fn chapter() -> nekowriter_core::chapter::ChapterInput {
        nekowriter_core::chapter::ChapterInput {
            id: ChapterId::generate(),
            kind: ChapterKind::Main,
            number: 1,
            title: "Arrival".to_owned(),
            goal: "The letter arrives.".to_owned(),
        }
    }

    fn texts(key: PromptKey) -> PromptTexts<'static> {
        PromptTexts {
            system: builtin(key, PromptRole::System),
            user: builtin(key, PromptRole::User),
        }
    }

    #[test]
    fn a_body_stage_declares_the_edit_tools_and_pins_the_chapter() {
        let chapter = chapter();
        let request = build(
            &StageInput::ChapterBody {
                chapter: chapter.clone(),
                guidance: String::new(),
                body: "The keeper woke.".to_owned(),
            },
            WritingLanguage::En,
            texts(PromptKey::ChapterWrite),
        )
        .unwrap();
        assert!(request.user.contains("The keeper woke."));
        assert!(request.user.contains("Arrival"));
        assert!(request.system.contains(&chapter.id.to_string()));
        let names: Vec<_> = request
            .contract
            .all_tools()
            .iter()
            .map(|tool| tool.name)
            .collect();
        assert_eq!(
            names,
            [APPEND_CONTENT, REPLACE_CONTENT, ASK_USER, SUBMIT_CHAPTER]
        );

        assert!(matches!(
            parse_submission(
                &request.contract,
                &json!({ "chapter_id": "another", "summary": "x" })
            ),
            Err(ArgumentsProblem::NotAllowed { .. })
        ));
        let parsed = parse_submission(
            &request.contract,
            &json!({ "chapter_id": chapter.id.to_string(), "summary": "The keeper wakes." }),
        )
        .unwrap();
        assert!(
            matches!(parsed, StageSubmission::Chapter { submission } if submission.chapter_id == chapter.id)
        );
        assert_eq!(
            WorkTool::ReplaceContent
                .parse(&json!({ "old_str": "woke", "new_str": "" }))
                .unwrap(),
            CallRequest::Edit {
                edit: BodyEdit::Replace {
                    old_str: "woke".to_owned(),
                    new_str: String::new()
                }
            }
        );
        assert!(matches!(
            WorkTool::AppendContent.parse(&json!({})),
            Err(ArgumentsProblem::MissingField { .. })
        ));
    }

    #[test]
    fn an_outline_stage_names_empty_guidance_and_submits_the_outline() {
        let chapter = chapter();
        let request = build(
            &StageInput::ChapterOutline {
                chapter: chapter.clone(),
                guidance: " ".to_owned(),
            },
            WritingLanguage::ZhCn,
            texts(PromptKey::ChapterOutline),
        )
        .unwrap();
        assert!(
            request
                .user
                .contains(&model_text().chapters.none.render(&[]))
        );
        assert_eq!(request.contract.tools, [WorkTool::AskUser]);
        assert_eq!(request.contract.submission.name, SUBMIT_CHAPTER_OUTLINE);
        let parsed = parse_submission(
            &request.contract,
            &json!({ "chapter_id": chapter.id.to_string(), "content": "Scene one." }),
        )
        .unwrap();
        assert!(
            matches!(parsed, StageSubmission::ChapterOutline { submission } if submission.content == "Scene one.")
        );
    }
}
