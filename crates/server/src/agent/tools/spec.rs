//! Tool definitions and input validation (spec/05 §1.3–§1.4).
//!
//! The model-facing definition (name, Chinese description, JSON Schema) comes from
//! `prompts/tools.toml`. Parameter schemas are a fixed code contract and are **not** overridable;
//! only the top-level `description` is (it is the `tool.<name>` prompt key, spec/09 §1).
//!
//! Input handling is strictly ordered (spec/05 §1.4): strict JSON parse → JSON Schema validation →
//! typed deserialization (in the handler) → semantic validation.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;

use super::{ToolError, ToolErrorKind};
use crate::prompts::PromptPack;

/// One tool's contract: the schema is validated once per process.
#[derive(Debug)]
pub struct ToolSpec {
    pub name: String,
    pub input_schema: Value,
    /// Compiled schema validator, built lazily on first use.
    validator: std::sync::OnceLock<Option<Arc<jsonschema::Validator>>>,
}

impl ToolSpec {
    pub fn new(name: impl Into<String>, input_schema: Value) -> Self {
        Self {
            name: name.into(),
            input_schema,
            validator: std::sync::OnceLock::new(),
        }
    }

    fn validator(&self) -> Option<&Arc<jsonschema::Validator>> {
        self.validator
            .get_or_init(|| match jsonschema::validator_for(&self.input_schema) {
                Ok(validator) => Some(Arc::new(validator)),
                Err(err) => {
                    // A shipped schema that does not compile is a build defect: the pack is
                    // validated at startup (spec/01 §5 step 6), so this cannot happen at runtime.
                    tracing::error!(tool = %self.name, error = %err, "invalid tool input schema");
                    None
                }
            })
            .as_ref()
    }
}

/// Every tool's contract, keyed by name, in pack order.
#[derive(Debug, Default)]
pub struct ToolSpecs {
    specs: BTreeMap<String, Arc<ToolSpec>>,
}

impl ToolSpecs {
    /// Builds the contracts from the loaded pack (`prompts/tools.toml`).
    pub fn from_pack(pack: &PromptPack) -> Self {
        let mut specs = BTreeMap::new();
        for tool in pack.tools() {
            specs.insert(
                tool.name.clone(),
                Arc::new(ToolSpec::new(tool.name.clone(), tool.input_schema.clone())),
            );
        }
        Self { specs }
    }

    /// Adds the test-only tool contracts to a build used by the debug session (spec/10 §2.7).
    pub fn with_debug_tools(mut self) -> Self {
        for (name, schema) in super::test_tools::schemas() {
            self.specs
                .insert(name.to_owned(), Arc::new(ToolSpec::new(name, schema)));
        }
        self
    }

    pub fn get(&self, name: &str) -> Option<&Arc<ToolSpec>> {
        self.specs.get(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.specs.keys().map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.specs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    /// The JSON Schema of one tool (spec/05 §1.3).
    pub fn schema(&self, name: &str) -> Option<&Value> {
        self.specs.get(name).map(|spec| &spec.input_schema)
    }
}

/// Strictly parses the raw argument text and validates it against the tool's schema
/// (spec/05 §1.4 steps 1–2).
///
/// The returned value can then be handed to the handler, which deserializes it into its own
/// `#[serde(deny_unknown_fields)]` struct.
pub fn validate_input(specs: &ToolSpecs, name: &str, raw: &str) -> Result<Value, ToolError> {
    let value: Value = serde_json::from_str(raw).map_err(|_| {
        // `error.invalid_json` includes the first 500 characters of the raw text.
        let prefix: String = raw.chars().take(500).collect();
        ToolError::new(ToolErrorKind::InvalidJson).with("raw", prefix)
    })?;
    if !value.is_object() {
        return Err(ToolError::new(ToolErrorKind::Validation).with(
            "errors",
            vec![format!("$: expected an object, got {}", type_name(&value))],
        ));
    }
    if let Some(spec) = specs.get(name)
        && let Some(validator) = spec.validator()
    {
        let mut errors = Vec::new();
        for error in validator.iter_errors(&value) {
            let path = error.instance_path().to_string();
            let path = if path.is_empty() {
                "$".to_owned()
            } else {
                format!("$.{path}")
            };
            errors.push(format!("{path}: {error}"));
            if errors.len() == 3 {
                // `error.validation` carries the first three errors (spec/05 §1.4).
                break;
            }
        }
        if !errors.is_empty() {
            return Err(ToolError::new(ToolErrorKind::Validation).with("errors", errors));
        }
    }
    Ok(value)
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_json_carries_the_raw_prefix() {
        let specs = ToolSpecs::default();
        let error = validate_input(&specs, "anything", "{not json").expect_err("invalid");
        assert_eq!(error.kind, ToolErrorKind::InvalidJson);
        assert_eq!(error.fields.get("raw"), Some(&Value::from("{not json")));
    }

    #[test]
    fn a_non_object_instance_is_rejected() {
        let specs = ToolSpecs::default();
        let error = validate_input(&specs, "anything", "[1,2]").expect_err("rejected");
        assert_eq!(error.kind, ToolErrorKind::Validation);
    }
}
