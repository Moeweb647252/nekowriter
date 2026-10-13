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
    pub contracts: Contracts,
    pub tools: Tools,
    pub results: Results,
    pub submission_problems: SubmissionProblems,
    pub protocol_problems: ProtocolProblems,
    pub arguments_problems: ArgumentsProblems,
    pub json_types: JsonTypes,
    pub corrections: Corrections,
    pub review: ReviewText,
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
pub struct Contracts {
    pub preparation: Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tools {
    pub submit_preparation: SubmitPreparationText,
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
