//! Built-in prompts and the variables they may use (`docs/product-spec.md` §13.1).
//!
//! The prompt texts themselves are model-facing and live in the agent crate's resources;
//! this module defines what exists, so the store, the agent and the UI agree on it.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::template::VariableSpec;

/// A built-in prompt pair (a system prompt and a user prompt).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum PromptKey {
    #[serde(rename = "preparation.generate")]
    PreparationGenerate,
}

/// The two halves of a prompt pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PromptRole {
    System,
    User,
}

/// A value a prompt can include. The UI explains each one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PromptVariable {
    /// The language the novel is written in, named in the prompt's language.
    WritingLanguage,
    /// The novel brief.
    NovelBrief,
    /// The artifacts to submit in this round, in order.
    RequiredArtifacts,
}

impl PromptVariable {
    /// The name used in templates, as in `{{novel_brief}}`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::WritingLanguage => "writing_language",
            Self::NovelBrief => "novel_brief",
            Self::RequiredArtifacts => "required_artifacts",
        }
    }
}

/// A variable of a prompt, and whether the prompt must use it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PromptVariableSpec {
    pub variable: PromptVariable,
    pub required: bool,
}

const fn spec(variable: PromptVariable, required: bool) -> PromptVariableSpec {
    PromptVariableSpec { variable, required }
}

const PREPARATION_GENERATE_SYSTEM: &[PromptVariableSpec] =
    &[spec(PromptVariable::WritingLanguage, true)];
const PREPARATION_GENERATE_USER: &[PromptVariableSpec] = &[
    spec(PromptVariable::NovelBrief, true),
    spec(PromptVariable::RequiredArtifacts, true),
    spec(PromptVariable::WritingLanguage, false),
];

impl PromptKey {
    pub const ALL: [Self; 1] = [Self::PreparationGenerate];

    /// The variables the system or user prompt of this pair may use. The writing language
    /// is required in the system prompts of the writing stages, so a custom prompt cannot
    /// lose it (§17.3).
    #[must_use]
    pub fn variables(self, role: PromptRole) -> &'static [PromptVariableSpec] {
        match (self, role) {
            (Self::PreparationGenerate, PromptRole::System) => PREPARATION_GENERATE_SYSTEM,
            (Self::PreparationGenerate, PromptRole::User) => PREPARATION_GENERATE_USER,
        }
    }

    /// The variables as the template checker takes them.
    #[must_use]
    pub fn template_variables(self, role: PromptRole) -> Vec<VariableSpec<'static>> {
        self.variables(role)
            .iter()
            .map(|spec| VariableSpec {
                name: spec.variable.name(),
                required: spec.required,
            })
            .collect()
    }
}
