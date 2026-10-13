//! The restricted template language of prompts and prompt templates
//! (`docs/product-spec.md` §13.2).
//!
//! `{{name}}` is replaced by the value of variable `name`, whose name consists of
//! lowercase ASCII letters, digits and underscores and starts with a letter. Spaces inside
//! the braces are ignored. `\{{` produces a literal `{{`. Values are inserted as they are;
//! nothing inside a value is expanded. There are no expressions, loops or includes.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

/// A parsed template.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Template {
    parts: Vec<Part>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Part {
    Text(String),
    Variable(String),
}

/// A variable a template may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VariableSpec<'a> {
    pub name: &'a str,
    /// A required variable must appear in the template.
    pub required: bool,
}

/// Why a template is invalid or cannot be rendered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, Error)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum TemplateError {
    /// A `{{` at this character offset has no closing `}}`.
    #[error("the placeholder at character {offset} is not closed")]
    UnclosedPlaceholder { offset: u32 },
    #[error("`{name}` is not a valid variable name")]
    InvalidVariableName { name: String },
    #[error("the variable `{name}` is not available here")]
    UnknownVariable { name: String },
    #[error("the required variable `{name}` is not used")]
    MissingRequiredVariable { name: String },
    #[error("no value was given for the variable `{name}`")]
    MissingValue { name: String },
}

impl Template {
    pub fn parse(source: &str) -> Result<Self, TemplateError> {
        let mut parts = Vec::new();
        let mut text = String::new();
        let mut rest = source;
        let mut offset = 0_usize;
        while !rest.is_empty() {
            if let Some(after) = rest.strip_prefix("\\{{") {
                text.push_str("{{");
                rest = after;
                offset += 3;
            } else if let Some(after) = rest.strip_prefix("{{") {
                let Some(end) = after.find("}}") else {
                    return Err(TemplateError::UnclosedPlaceholder {
                        offset: u32::try_from(offset).unwrap_or(u32::MAX),
                    });
                };
                let name = after[..end].trim();
                if !is_variable_name(name) {
                    return Err(TemplateError::InvalidVariableName {
                        name: name.to_owned(),
                    });
                }
                if !text.is_empty() {
                    parts.push(Part::Text(std::mem::take(&mut text)));
                }
                parts.push(Part::Variable(name.to_owned()));
                offset += 2 + after[..end].chars().count() + 2;
                rest = &after[end + 2..];
            } else {
                let character = rest.chars().next().expect("rest is not empty");
                text.push(character);
                rest = &rest[character.len_utf8()..];
                offset += 1;
            }
        }
        if !text.is_empty() {
            parts.push(Part::Text(text));
        }
        Ok(Self { parts })
    }

    /// The variables the template uses, in order of first appearance.
    pub fn variables(&self) -> impl Iterator<Item = &str> {
        let mut seen = Vec::new();
        self.parts.iter().filter_map(move |part| match part {
            Part::Variable(name) if !seen.contains(name) => {
                seen.push(name.clone());
                Some(name.as_str())
            }
            _ => None,
        })
    }

    /// Checks that the template uses only `available` variables and all required ones.
    pub fn check(&self, available: &[VariableSpec<'_>]) -> Result<(), TemplateError> {
        for name in self.variables() {
            if !available.iter().any(|spec| spec.name == name) {
                return Err(TemplateError::UnknownVariable {
                    name: name.to_owned(),
                });
            }
        }
        for spec in available.iter().filter(|spec| spec.required) {
            if !self.variables().any(|name| name == spec.name) {
                return Err(TemplateError::MissingRequiredVariable {
                    name: spec.name.to_owned(),
                });
            }
        }
        Ok(())
    }

    /// Replaces every variable with its value.
    pub fn render<'v>(
        &self,
        value_of: impl Fn(&str) -> Option<&'v str>,
    ) -> Result<String, TemplateError> {
        let mut rendered = String::new();
        for part in &self.parts {
            match part {
                Part::Text(text) => rendered.push_str(text),
                Part::Variable(name) => rendered.push_str(
                    value_of(name)
                        .ok_or_else(|| TemplateError::MissingValue { name: name.clone() })?,
                ),
            }
        }
        Ok(rendered)
    }
}

fn is_variable_name(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(source: &str, values: &[(&str, &'static str)]) -> Result<String, TemplateError> {
        Template::parse(source)?.render(|name| {
            values
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| *value)
        })
    }

    #[test]
    fn variables_are_replaced_once_and_values_are_not_expanded() {
        assert_eq!(
            render(
                "Write in {{ language }}: {{brief}}",
                &[("language", "English"), ("brief", "a {{language}} story")]
            )
            .unwrap(),
            "Write in English: a {{language}} story"
        );
    }

    #[test]
    fn an_escaped_placeholder_is_literal_text() {
        assert_eq!(render("\\{{brief}} stays", &[]).unwrap(), "{{brief}} stays");
    }

    #[test]
    fn malformed_placeholders_are_rejected() {
        assert_eq!(
            Template::parse("a {{brief").unwrap_err(),
            TemplateError::UnclosedPlaceholder { offset: 2 }
        );
        assert_eq!(
            Template::parse("{{Brief}}").unwrap_err(),
            TemplateError::InvalidVariableName {
                name: "Brief".to_owned()
            }
        );
    }

    #[test]
    fn check_reports_unknown_and_missing_required_variables() {
        let available = [
            VariableSpec {
                name: "brief",
                required: true,
            },
            VariableSpec {
                name: "language",
                required: false,
            },
        ];
        assert!(
            Template::parse("{{brief}}")
                .unwrap()
                .check(&available)
                .is_ok()
        );
        assert_eq!(
            Template::parse("{{brief}} {{mood}}")
                .unwrap()
                .check(&available)
                .unwrap_err(),
            TemplateError::UnknownVariable {
                name: "mood".to_owned()
            }
        );
        assert_eq!(
            Template::parse("{{language}}")
                .unwrap()
                .check(&available)
                .unwrap_err(),
            TemplateError::MissingRequiredVariable {
                name: "brief".to_owned()
            }
        );
    }

    #[test]
    fn rendering_without_a_value_fails() {
        assert_eq!(
            render("{{brief}}", &[]).unwrap_err(),
            TemplateError::MissingValue {
                name: "brief".to_owned()
            }
        );
    }
}
