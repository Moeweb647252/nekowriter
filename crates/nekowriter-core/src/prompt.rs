//! Built-in prompts and the variables they may use (`docs/product-spec.md` §13.1).
//!
//! The prompt texts themselves are model-facing and live in the agent crate's resources;
//! this module defines what exists, so the store, the agent and the UI agree on it.

use sea_orm::{DeriveActiveEnum, EnumIter, sea_query::StringLen};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    field::{Field, FieldError, FieldProblem, multi_line},
    review::ReviewKind,
    template::{Template, TemplateTextError, VariableSpec},
};

/// A built-in prompt pair (a system prompt and a user prompt).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS, EnumIter, DeriveActiveEnum,
)]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::None)")]
#[ts(export)]
pub enum PromptKey {
    #[serde(rename = "preparation.generate")]
    #[sea_orm(string_value = "preparation.generate")]
    PreparationGenerate,
    #[serde(rename = "chapter.outline")]
    #[sea_orm(string_value = "chapter.outline")]
    ChapterOutline,
    #[serde(rename = "chapter.write")]
    #[sea_orm(string_value = "chapter.write")]
    ChapterWrite,
    #[serde(rename = "review.preparation.background")]
    #[sea_orm(string_value = "review.preparation.background")]
    ReviewPreparationBackground,
    #[serde(rename = "review.preparation.research")]
    #[sea_orm(string_value = "review.preparation.research")]
    ReviewPreparationResearch,
    #[serde(rename = "review.preparation.characters")]
    #[sea_orm(string_value = "review.preparation.characters")]
    ReviewPreparationCharacters,
    #[serde(rename = "review.preparation.outline")]
    #[sea_orm(string_value = "review.preparation.outline")]
    ReviewPreparationOutline,
    #[serde(rename = "review.chapter.outline")]
    #[sea_orm(string_value = "review.chapter.outline")]
    ReviewChapterOutline,
    #[serde(rename = "review.chapter.edit")]
    #[sea_orm(string_value = "review.chapter.edit")]
    ReviewChapterEdit,
    #[serde(rename = "review.chapter.final")]
    #[sea_orm(string_value = "review.chapter.final")]
    ReviewChapterFinal,
}

/// The two halves of a prompt pair.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS, EnumIter, DeriveActiveEnum,
)]
#[sea_orm(
    rs_type = "String",
    db_type = "String(StringLen::None)",
    rename_all = "snake_case"
)]
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
    /// Whether the chapter belongs to the plan or is an extra chapter.
    ChapterKind,
    /// The chapter's number among the chapters of its kind.
    ChapterNumber,
    ChapterTitle,
    /// The goal of the chapter's plan item; empty for extra chapters.
    ChapterGoal,
    /// What the user asked of the chapter; may be empty.
    ChapterGuidance,
}

impl PromptVariable {
    /// The name used in templates, as in `{{novel_brief}}`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::WritingLanguage => "writing_language",
            Self::NovelBrief => "novel_brief",
            Self::RequiredArtifacts => "required_artifacts",
            Self::ChapterKind => "chapter_kind",
            Self::ChapterNumber => "chapter_number",
            Self::ChapterTitle => "chapter_title",
            Self::ChapterGoal => "chapter_goal",
            Self::ChapterGuidance => "chapter_guidance",
        }
    }
}

/// A variable of a prompt, and whether the prompt must use it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PromptVariableSpec {
    pub variable: PromptVariable,
    pub required: bool,
}

const fn spec(variable: PromptVariable, required: bool) -> PromptVariableSpec {
    PromptVariableSpec { variable, required }
}

const WRITING_SYSTEM: &[PromptVariableSpec] = &[spec(PromptVariable::WritingLanguage, true)];
const PREPARATION_GENERATE_USER: &[PromptVariableSpec] = &[
    spec(PromptVariable::NovelBrief, true),
    spec(PromptVariable::RequiredArtifacts, true),
    spec(PromptVariable::WritingLanguage, false),
];
const CHAPTER_USER: &[PromptVariableSpec] = &[
    spec(PromptVariable::ChapterTitle, true),
    spec(PromptVariable::ChapterGoal, true),
    spec(PromptVariable::ChapterGuidance, true),
    spec(PromptVariable::ChapterNumber, false),
    spec(PromptVariable::ChapterKind, false),
    spec(PromptVariable::WritingLanguage, false),
];

/// Review prompts take no required variables: the review material is appended to the user
/// prompt, so a custom prompt cannot lose it.
const REVIEW_USER: &[PromptVariableSpec] = &[spec(PromptVariable::WritingLanguage, false)];

impl PromptKey {
    pub const ALL: [Self; 10] = [
        Self::PreparationGenerate,
        Self::ChapterOutline,
        Self::ChapterWrite,
        Self::ReviewPreparationBackground,
        Self::ReviewPreparationResearch,
        Self::ReviewPreparationCharacters,
        Self::ReviewPreparationOutline,
        Self::ReviewChapterOutline,
        Self::ReviewChapterEdit,
        Self::ReviewChapterFinal,
    ];

    /// The review prompt of a review kind.
    #[must_use]
    pub fn review(kind: ReviewKind) -> Self {
        match kind {
            ReviewKind::PreparationBackground => Self::ReviewPreparationBackground,
            ReviewKind::PreparationResearch => Self::ReviewPreparationResearch,
            ReviewKind::PreparationCharacters => Self::ReviewPreparationCharacters,
            ReviewKind::PreparationOutline => Self::ReviewPreparationOutline,
            ReviewKind::ChapterOutline => Self::ReviewChapterOutline,
            ReviewKind::ChapterEdit => Self::ReviewChapterEdit,
            ReviewKind::ChapterFinal => Self::ReviewChapterFinal,
        }
    }

    /// The variables the system or user prompt of this pair may use. The writing language
    /// is required in the system prompts of the writing stages, so a custom prompt cannot
    /// lose it (§17.3).
    #[must_use]
    pub fn variables(self, role: PromptRole) -> &'static [PromptVariableSpec] {
        match (self, role) {
            (_, PromptRole::System) => WRITING_SYSTEM,
            (Self::PreparationGenerate, PromptRole::User) => PREPARATION_GENERATE_USER,
            (Self::ChapterOutline | Self::ChapterWrite, PromptRole::User) => CHAPTER_USER,
            (
                Self::ReviewPreparationBackground
                | Self::ReviewPreparationResearch
                | Self::ReviewPreparationCharacters
                | Self::ReviewPreparationOutline
                | Self::ReviewChapterOutline
                | Self::ReviewChapterEdit
                | Self::ReviewChapterFinal,
                PromptRole::User,
            ) => REVIEW_USER,
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

/// The content of a prompt override (§13.1, §13.2): text up to [`Self::MAX_CHARS`], which
/// may be empty, using only its prompt's variables and every required one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PromptContent(String);

impl PromptContent {
    pub const MAX_CHARS: u32 = 100_000;

    pub fn parse(key: PromptKey, role: PromptRole, value: &str) -> Result<Self, TemplateTextError> {
        let text = match multi_line(Field::PromptContent, value, Self::MAX_CHARS) {
            Ok(text) => text,
            Err(FieldError {
                problem: FieldProblem::Empty,
                ..
            }) => String::new(),
            Err(error) => return Err(error.into()),
        };
        Template::parse(&text)?.check(&key.template_variables(role))?;
        Ok(Self(text))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::TemplateError;

    #[test]
    fn overrides_use_only_their_prompts_variables() {
        let system = |text| PromptContent::parse(PromptKey::ChapterWrite, PromptRole::System, text);
        assert_eq!(
            system("  Write in {{writing_language}}.\r\n").map(|content| content.0),
            Ok("Write in {{writing_language}}.".to_owned())
        );
        assert_eq!(
            system(""),
            Err(TemplateTextError::Template(
                TemplateError::MissingRequiredVariable {
                    name: "writing_language".to_owned()
                }
            ))
        );
        assert_eq!(
            system("{{writing_language}} {{novel_brief}}"),
            Err(TemplateTextError::Template(
                TemplateError::UnknownVariable {
                    name: "novel_brief".to_owned()
                }
            ))
        );
        // A review's user prompt needs no variables, so it may be empty.
        assert!(PromptContent::parse(PromptKey::ReviewChapterFinal, PromptRole::User, "").is_ok());
    }
}
