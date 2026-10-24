//! Sample contexts for every prompt key (spec/09 §2).
//!
//! One sample per branch of each shipped template, used to validate the shipped pack at startup
//! and every override when it is saved (spec/09 §4). The data lives in `samples.json` so the
//! non-English sample strings stay data (AGENTS.md §2, DECISIONS D-041); it mirrors the variant
//! table of `scripts/validate_prompts.py`.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::{Map, Value};

use super::pack::PromptEntry;

const SAMPLES_JSON: &str = include_str!("samples.json");

#[derive(Debug, Deserialize)]
struct SampleData {
    /// The base sample value of every template variable.
    base: Map<String, Value>,
    /// Per key, one override map per branch of the shipped template.
    #[serde(default)]
    variants: BTreeMap<String, Vec<Value>>,
}

fn data() -> &'static SampleData {
    static DATA: OnceLock<SampleData> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(SAMPLES_JSON).unwrap_or_else(|err| {
            tracing::error!(error = %err, "the prompt sample contexts are not valid JSON");
            SampleData {
                base: Map::new(),
                variants: BTreeMap::new(),
            }
        })
    })
}

/// One sample context per branch of the key's shipped template.
///
/// The context holds the base value of every declared variable, overridden by the branch's
/// variant values. Keys without a variant table render once with the base values.
pub fn contexts(entry: &PromptEntry) -> Vec<Value> {
    let data = data();
    let empty = vec![Value::Object(Map::new())];
    let cases = data.variants.get(&entry.key).unwrap_or(&empty);
    cases
        .iter()
        .map(|case| {
            let mut context = Map::new();
            for var in &entry.vars {
                if let Some(value) = data.base.get(var) {
                    context.insert(var.clone(), value.clone());
                }
            }
            if let Value::Object(overrides) = case {
                for (name, value) in overrides {
                    context.insert(name.clone(), value.clone());
                }
            }
            Value::Object(context)
        })
        .collect()
}

/// The keys that have a variant table (used by tests to prove per-branch coverage).
pub fn variant_keys() -> Vec<&'static str> {
    data().variants.keys().map(String::as_str).collect()
}

/// The base sample value of one documented template variable.
pub fn base_value(name: &str) -> Option<&'static Value> {
    data().base.get(name)
}
