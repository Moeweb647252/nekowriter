//! User prompt templates (`docs/product-spec.md` §13.2): reusable text with placeholders of
//! its own, which the user fills in and inserts into guidance or feedback drafts. A
//! template is never a prompt: it only ever becomes part of what the user writes.

use sea_orm::FromJsonQueryResult;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    field::{Field, FieldError, FieldProblem, multi_line_text, single_line, single_line_text},
    template::{Template, TemplateError, TemplateTextError, VariableSpec, is_variable_name},
};

single_line_text!(
    /// The title the user picks a template by.
    TemplateTitle,
    Field::TemplateTitle,
    200
);

multi_line_text!(
    /// A template's text with its `{{placeholders}}`.
    TemplateBody,
    Field::TemplateBody,
    20_000
);

/// A placeholder of a template and what to fill in for it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TemplateVariable {
    pub name: String,
    /// May be empty.
    pub description: String,
}

/// A template's placeholders, in the order they were declared, as a stored column.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, FromJsonQueryResult)]
pub struct TemplateVariables(pub Vec<TemplateVariable>);

/// A template as saved: its body uses exactly the placeholders it declares.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TemplateContent {
    pub title: TemplateTitle,
    pub body: TemplateBody,
    pub variables: TemplateVariables,
}

impl TemplateContent {
    /// The longest variable description, in characters.
    pub const DESCRIPTION_MAX_CHARS: u32 = 500;

    pub fn parse(
        title: &str,
        body: &str,
        variables: Vec<TemplateVariable>,
    ) -> Result<Self, TemplateTextError> {
        let title = TemplateTitle::parse(title)?;
        let body = TemplateBody::parse(body)?;
        let mut declared: Vec<TemplateVariable> = Vec::with_capacity(variables.len());
        for variable in variables {
            let name = variable.name.trim().to_owned();
            if !is_variable_name(&name) {
                return Err(TemplateError::InvalidVariableName { name }.into());
            }
            if declared.iter().any(|existing| existing.name == name) {
                return Err(TemplateError::DuplicateVariable { name }.into());
            }
            let description = match single_line(
                Field::TemplateVariableDescription,
                &variable.description,
                Self::DESCRIPTION_MAX_CHARS,
            ) {
                Ok(description) => description,
                Err(FieldError {
                    problem: FieldProblem::Empty,
                    ..
                }) => String::new(),
                Err(error) => return Err(error.into()),
            };
            declared.push(TemplateVariable { name, description });
        }
        let specs: Vec<VariableSpec<'_>> = declared
            .iter()
            .map(|variable| VariableSpec {
                name: &variable.name,
                required: true,
            })
            .collect();
        Template::parse(body.as_str())?.check(&specs)?;
        Ok(Self {
            title,
            body,
            variables: TemplateVariables(declared),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variable(name: &str) -> TemplateVariable {
        TemplateVariable {
            name: name.to_owned(),
            description: String::new(),
        }
    }

    #[test]
    fn a_body_uses_exactly_its_declared_placeholders() {
        let parsed =
            TemplateContent::parse("Pace", "Slow {{scene}} down.", vec![variable(" scene ")])
                .unwrap();
        assert_eq!(parsed.variables.0, [variable("scene")]);

        let refused = |body: &str, variables: Vec<TemplateVariable>| {
            TemplateContent::parse("Pace", body, variables).unwrap_err()
        };
        assert_eq!(
            refused("Slow {{scene}} down.", Vec::new()),
            TemplateError::UnknownVariable {
                name: "scene".to_owned()
            }
            .into()
        );
        assert_eq!(
            refused("Slow down.", vec![variable("scene")]),
            TemplateError::MissingRequiredVariable {
                name: "scene".to_owned()
            }
            .into()
        );
        assert_eq!(
            refused("{{scene}}", vec![variable("scene"), variable("scene")]),
            TemplateError::DuplicateVariable {
                name: "scene".to_owned()
            }
            .into()
        );
        assert_eq!(
            refused("{{scene}}", vec![variable("Scene")]),
            TemplateError::InvalidVariableName {
                name: "Scene".to_owned()
            }
            .into()
        );
    }
}
