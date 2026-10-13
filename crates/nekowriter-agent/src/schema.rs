//! Tool argument schemas.
//!
//! A tool's arguments are defined once, as a [`Schema`]. The same definition produces the
//! JSON Schema declared to the model and validates the arguments the model sends, with
//! structured problems the model can act on (`docs/product-spec.md` §8.4). Only the subset
//! of JSON Schema the tools need is supported.

use nekowriter_core::run_message::{ArgumentsProblem, JsonType};
use serde_json::{Map, Value, json};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Schema {
    Object {
        properties: Vec<Property>,
    },
    Array {
        items: Box<Schema>,
    },
    /// A string, optionally restricted to the given values.
    String {
        allowed: Option<Vec<String>>,
    },
    Boolean,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Property {
    pub name: &'static str,
    pub description: String,
    pub required: bool,
    pub schema: Schema,
}

impl Schema {
    /// The schema as JSON Schema.
    #[must_use]
    pub fn to_json_schema(&self) -> Value {
        self.json_with_description(None)
    }

    fn json_with_description(&self, description: Option<&str>) -> Value {
        let mut schema = match self {
            Self::Object { properties } => {
                let mut fields = Map::new();
                for property in properties {
                    fields.insert(
                        property.name.to_owned(),
                        property
                            .schema
                            .json_with_description(Some(&property.description)),
                    );
                }
                let required: Vec<_> = properties
                    .iter()
                    .filter(|property| property.required)
                    .map(|property| property.name)
                    .collect();
                json!({ "type": "object", "properties": fields, "required": required })
            }
            Self::Array { items } => json!({ "type": "array", "items": items.to_json_schema() }),
            Self::String { allowed: None } => json!({ "type": "string" }),
            Self::String {
                allowed: Some(values),
            } => json!({ "type": "string", "enum": values }),
            Self::Boolean => json!({ "type": "boolean" }),
        };
        if let (Some(description), Value::Object(object)) = (description, &mut schema) {
            object.insert("description".to_owned(), description.into());
        }
        schema
    }

    /// Checks `value` against the schema. Properties the schema does not mention are
    /// ignored.
    pub fn validate(&self, value: &Value) -> Result<(), ArgumentsProblem> {
        self.validate_at(value, "")
    }

    fn validate_at(&self, value: &Value, path: &str) -> Result<(), ArgumentsProblem> {
        match self {
            Self::Object { properties } => {
                let Value::Object(object) = value else {
                    return Err(wrong_type(path, JsonType::Object));
                };
                for property in properties {
                    match object.get(property.name) {
                        None | Some(Value::Null) if property.required => {
                            return Err(ArgumentsProblem::MissingField {
                                path: path.to_owned(),
                                field: property.name.to_owned(),
                            });
                        }
                        None | Some(Value::Null) => {}
                        Some(field) => property
                            .schema
                            .validate_at(field, &join(path, property.name))?,
                    }
                }
                Ok(())
            }
            Self::Array { items } => {
                let Value::Array(elements) = value else {
                    return Err(wrong_type(path, JsonType::Array));
                };
                for (index, element) in elements.iter().enumerate() {
                    items.validate_at(element, &format!("{path}[{index}]"))?;
                }
                Ok(())
            }
            Self::String { allowed } => {
                let Value::String(text) = value else {
                    return Err(wrong_type(path, JsonType::String));
                };
                if let Some(allowed) = allowed
                    && !allowed.contains(text)
                {
                    return Err(ArgumentsProblem::NotAllowed {
                        path: path.to_owned(),
                        value: text.clone(),
                        allowed: allowed.clone(),
                    });
                }
                Ok(())
            }
            Self::Boolean => match value {
                Value::Bool(_) => Ok(()),
                _ => Err(wrong_type(path, JsonType::Boolean)),
            },
        }
    }
}

fn wrong_type(path: &str, expected: JsonType) -> ArgumentsProblem {
    ArgumentsProblem::WrongType {
        path: path.to_owned(),
        expected,
    }
}

fn join(path: &str, field: &str) -> String {
    if path.is_empty() {
        field.to_owned()
    } else {
        format!("{path}.{field}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Schema {
        Schema::Object {
            properties: vec![Property {
                name: "items",
                description: "Items.".to_owned(),
                required: true,
                schema: Schema::Array {
                    items: Box::new(Schema::Object {
                        properties: vec![Property {
                            name: "kind",
                            description: "Kind.".to_owned(),
                            required: true,
                            schema: Schema::String {
                                allowed: Some(vec!["a".to_owned(), "b".to_owned()]),
                            },
                        }],
                    }),
                },
            }],
        }
    }

    #[test]
    fn the_json_schema_carries_types_descriptions_and_required_fields() {
        assert_eq!(
            schema().to_json_schema(),
            json!({
                "type": "object",
                "properties": {
                    "items": {
                        "type": "array",
                        "description": "Items.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "kind": { "type": "string", "enum": ["a", "b"], "description": "Kind." }
                            },
                            "required": ["kind"]
                        }
                    }
                },
                "required": ["items"]
            })
        );
    }

    #[test]
    fn validation_locates_the_first_problem() {
        assert_eq!(
            schema().validate(&json!({ "items": [{ "kind": "a" }] })),
            Ok(())
        );
        assert_eq!(
            schema().validate(&json!({})),
            Err(ArgumentsProblem::MissingField {
                path: String::new(),
                field: "items".to_owned()
            })
        );
        assert_eq!(
            schema().validate(&json!({ "items": "a" })),
            Err(ArgumentsProblem::WrongType {
                path: "items".to_owned(),
                expected: JsonType::Array
            })
        );
        assert_eq!(
            schema().validate(&json!({ "items": [{ "kind": "a" }, { "kind": "c" }] })),
            Err(ArgumentsProblem::NotAllowed {
                path: "items[1].kind".to_owned(),
                value: "c".to_owned(),
                allowed: vec!["a".to_owned(), "b".to_owned()]
            })
        );
    }
}
