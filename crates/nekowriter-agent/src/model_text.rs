//! The model-facing text in `resources/model.zh-CN.toml`.
//!
//! Every entry is a required field, so a missing or misspelled key fails when the file is
//! loaded (and in this module's test), not when a run needs it.

use std::sync::LazyLock;

use nekowriter_core::template::Template;
use serde::{Deserialize, Deserializer};

static MODEL_TEXT: LazyLock<ModelText> = LazyLock::new(|| {
    toml::from_str(include_str!("../resources/model.zh-CN.toml"))
        .expect("resources/model.zh-CN.toml matches ModelText (checked by the tests)")
});

/// The model-facing text.
#[must_use]
pub fn model_text() -> &'static ModelText {
    &MODEL_TEXT
}

/// A text with `{{placeholders}}`.
#[derive(Debug)]
pub struct Text(Template);

impl Text {
    /// Fills in the placeholders. Every placeholder needs a value; the tests render every
    /// text with the values its callers pass.
    #[must_use]
    pub fn render(&self, values: &[(&str, &str)]) -> String {
        self.0
            .render(|name| {
                values
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| *value)
            })
            .expect("model text placeholders match the values passed (checked by the tests)")
    }
}

impl<'de> Deserialize<'de> for Text {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let source = String::deserialize(deserializer)?;
        Template::parse(&source)
            .map(Self)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelText {
    pub languages: Languages,
    pub artifacts: Artifacts,
    pub list: ListText,
    pub chapter_kinds: ChapterKinds,
    pub contracts: Contracts,
    pub tools: Tools,
    pub results: Results,
    pub edit_problems: EditProblems,
    pub question_problems: QuestionProblems,
    pub submission_problems: SubmissionProblems,
    pub protocol_problems: ProtocolProblems,
    pub arguments_problems: ArgumentsProblems,
    pub json_types: JsonTypes,
    pub corrections: Corrections,
    pub review: ReviewText,
    pub review_material: ReviewMaterial,
    pub folds: FoldText,
    pub chapters: ChapterText,
    pub connection_test: ConnectionTest,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Languages {
    #[serde(rename = "zh-CN")]
    pub zh_cn: Text,
    pub en: Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifacts {
    pub background: Text,
    pub research: Text,
    pub characters: Text,
    pub outline: Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListText {
    pub separator: Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChapterKinds {
    pub main: Text,
    pub extra: Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contracts {
    pub ask_user: Text,
    pub review: Text,
    pub preparation: Text,
    pub chapter_outline: Text,
    pub chapter_body: Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tools {
    pub submit_preparation: SubmitPreparationText,
    pub submit_chapter_outline: SubmitChapterOutlineText,
    pub append_content: AppendContentText,
    pub replace_content: ReplaceContentText,
    pub submit_chapter: SubmitChapterText,
    pub ask_user: AskUserText,
    pub submit_review: SubmitReviewText,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitReviewText {
    pub description: Text,
    pub passed: Text,
    pub confidence: Text,
    pub issues: Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AskUserText {
    pub description: Text,
    pub questions: Text,
    pub question_id: Text,
    pub title: Text,
    pub kind: Text,
    pub options: Text,
    pub option_id: Text,
    pub option_label: Text,
    pub required: Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitChapterOutlineText {
    pub description: Text,
    pub chapter_id: Text,
    pub content: Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppendContentText {
    pub description: Text,
    pub content: Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplaceContentText {
    pub description: Text,
    pub old_str: Text,
    pub new_str: Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitChapterText {
    pub description: Text,
    pub chapter_id: Text,
    pub summary: Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitPreparationText {
    pub description: Text,
    pub artifacts: Text,
    pub artifact_kind: Text,
    pub artifact_content: Text,
    pub chapter_plan: Text,
    pub chapter_title: Text,
    pub chapter_goal: Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Results {
    pub submission_accepted: Text,
    pub submission_rejected: Text,
    pub protocol_error: Text,
    pub edit_applied: Text,
    pub edit_no_change: Text,
    pub edit_rejected: Text,
    pub edit_declined: Text,
    pub answered: Text,
    pub question_rejected: Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionProblems {
    pub no_questions: Text,
    pub too_many_questions: Text,
    pub duplicate_question: Text,
    pub empty_title: Text,
    pub no_options: Text,
    pub too_many_options: Text,
    pub duplicate_option: Text,
    pub empty_option: Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditProblems {
    pub empty_content: Text,
    pub empty_old_str: Text,
    pub no_match: Text,
    pub ambiguous_match: Text,
    pub body_too_long: Text,
    pub stale_base: Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionProblems {
    pub missing_artifacts: Text,
    pub unexpected_artifacts: Text,
    pub duplicate_artifact: Text,
    pub empty_artifact: Text,
    pub artifact_too_long: Text,
    pub chapter_plan_missing: Text,
    pub chapter_plan_not_expected: Text,
    pub chapter_plan_empty: Text,
    pub chapter_plan_too_long: Text,
    pub chapter_title_empty: Text,
    pub chapter_title_not_single_line: Text,
    pub chapter_title_too_long: Text,
    pub chapter_goal_empty: Text,
    pub chapter_goal_too_long: Text,
    pub outline_empty: Text,
    pub outline_too_long: Text,
    pub summary_empty: Text,
    pub summary_too_long: Text,
    pub body_empty: Text,
    pub wrong_chapter: Text,
    pub feedback_pending: Text,
    pub confidence_out_of_range: Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolProblems {
    pub unknown_tool: Text,
    pub parallel_tool_calls: Text,
    pub invalid_arguments: Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArgumentsProblems {
    pub not_json: Text,
    pub missing_field: Text,
    pub wrong_type: Text,
    pub not_allowed: Text,
    pub root: Text,
    pub field: Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JsonTypes {
    pub object: Text,
    pub array: Text,
    pub string: Text,
    pub integer: Text,
    pub number: Text,
    pub boolean: Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Corrections {
    pub no_tool_call: Text,
    pub protocol_error: Text,
    pub submission_rejected: Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewText {
    pub preparation_outcome: Text,
    pub rejected: Text,
    pub stale: Text,
    pub item_separator: Text,
    pub outline_rejected: Text,
    pub chapter_rejected: Text,
    pub no_issues: Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewMaterial {
    pub artifact: Text,
    pub brief: Text,
    pub outline: Text,
    pub guidance: Text,
    pub chapter: Text,
    pub summary: Text,
    pub body: Text,
    pub append: Text,
    pub replace: Text,
    pub notes: Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoldText {
    pub intro: Text,
    pub separator: Text,
    pub preparation: Text,
    pub document: Text,
    pub chapter_plan: Text,
    pub plan_item: Text,
    pub chapter_outline: Text,
    pub chapter_body: Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChapterText {
    pub name: Text,
    pub none: Text,
    pub current_body: Text,
    pub empty_body: Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionTest {
    pub system: Text,
    pub user: Text,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_resource_file_loads() {
        let text = model_text();
        assert_ne!(text.languages.zh_cn.render(&[]), "");
    }
}
