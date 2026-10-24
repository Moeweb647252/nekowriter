//! The shipped prompt pack (spec/09 §1): parsing, embedded loading and variable documentation.
//!
//! The pack is embedded at compile time, so a missing or malformed file is a build defect that
//! aborts startup (spec/01 §5 step 6). Code only ever loads and renders these templates; the
//! model-facing text lives exclusively in `prompts/`.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use super::PromptError;

/// The prompt files of the pack, embedded at compile time (spec/09 §1).
const PACK_FILES: &[(&str, &str)] = &[
    (
        "system.toml",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../prompts/system.toml"
        )),
    ),
    (
        "tasks.toml",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../prompts/tasks.toml"
        )),
    ),
    (
        "blocks.toml",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../prompts/blocks.toml"
        )),
    ),
    (
        "shared.toml",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../prompts/shared.toml"
        )),
    ),
    (
        "notices.toml",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../prompts/notices.toml"
        )),
    ),
    (
        "results.toml",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../prompts/results.toml"
        )),
    ),
];

const TOOLS_FILE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../prompts/tools.toml"
));
const VARIABLES_FILE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../prompts/variables.toml"
));

/// The kinds of entries in the pack (spec/09 §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PromptKind {
    System,
    Task,
    Block,
    Notice,
    Result,
    Shared,
    Tool,
}

impl PromptKind {
    pub fn as_str(self) -> &'static str {
        match self {
            PromptKind::System => "system",
            PromptKind::Task => "task",
            PromptKind::Block => "block",
            PromptKind::Notice => "notice",
            PromptKind::Result => "result",
            PromptKind::Shared => "shared",
            PromptKind::Tool => "tool",
        }
    }
}

/// One template of the pack.
#[derive(Debug, Clone)]
pub struct PromptEntry {
    pub key: String,
    pub kind: PromptKind,
    /// The role the prompt belongs to (`conception`, `researcher`, `writer`, `reviewer`,
    /// `archivist`, `shared`; tool entries are shared).
    pub agent: String,
    /// Top-level variables the template may use (spec/09 §2).
    pub vars: Vec<String>,
    pub title_en: String,
    pub title_zh: String,
    /// The shipped default template text.
    pub text: String,
}

/// A tool definition from `tools.toml`: the description is the overridable `tool.<name>` key;
/// the input schema is a fixed contract (spec/09 §1).
#[derive(Debug, Clone)]
pub struct ToolSchema {
    pub name: String,
    pub input_schema: Value,
    pub examples: Vec<Value>,
}

/// One documented template variable, shown in the prompt editor (spec/09 §1).
#[derive(Debug, Clone)]
pub struct VariableDoc {
    pub name: String,
    pub en: String,
    pub zh: String,
    pub fields: Vec<VariableFieldDoc>,
}

/// One documented field of a template variable.
#[derive(Debug, Clone)]
pub struct VariableFieldDoc {
    pub name: String,
    pub en: String,
    pub zh: String,
}

/// The parsed shipped pack.
#[derive(Debug)]
pub struct PromptPack {
    entries: BTreeMap<String, PromptEntry>,
    /// The pack order: the prompt files in their documented order, tools last.
    order: Vec<String>,
    variables: BTreeMap<String, VariableDoc>,
    tools: BTreeMap<String, ToolSchema>,
}

impl PromptPack {
    /// Parses and structurally validates the embedded pack.
    pub fn embedded() -> Result<Self, PromptError> {
        let mut entries = BTreeMap::new();
        let mut order = Vec::new();
        let mut seen = BTreeSet::new();

        for (file, content) in PACK_FILES {
            let parsed: RawFile = toml::from_str(content)
                .map_err(|err| PromptError::Pack(format!("{file}: {err}")))?;
            for raw in parsed.prompt {
                if !seen.insert(raw.key.clone()) {
                    return Err(PromptError::Pack(format!(
                        "duplicate prompt key {}",
                        raw.key
                    )));
                }
                let kind = match raw.kind {
                    RawKind::System => PromptKind::System,
                    RawKind::Task => PromptKind::Task,
                    RawKind::Block => PromptKind::Block,
                    RawKind::Notice => PromptKind::Notice,
                    RawKind::Result => PromptKind::Result,
                    RawKind::Shared => PromptKind::Shared,
                };
                if raw.text.trim().is_empty() {
                    return Err(PromptError::Pack(format!(
                        "{} has an empty template",
                        raw.key
                    )));
                }
                order.push(raw.key.clone());
                entries.insert(
                    raw.key.clone(),
                    PromptEntry {
                        key: raw.key,
                        kind,
                        agent: raw.agent,
                        vars: raw.vars,
                        title_en: raw.title_en,
                        title_zh: raw.title_zh,
                        text: raw.text,
                    },
                );
            }
        }

        let tools_file: RawTools = toml::from_str(TOOLS_FILE)
            .map_err(|err| PromptError::Pack(format!("tools.toml: {err}")))?;
        let mut tools = BTreeMap::new();
        for (name, raw) in tools_file.tools {
            if raw.description.trim().is_empty() {
                return Err(PromptError::Pack(format!(
                    "tool {name} has an empty description"
                )));
            }
            let key = format!("tool.{name}");
            if !seen.insert(key.clone()) {
                return Err(PromptError::Pack(format!("duplicate prompt key {key}")));
            }
            let input_schema = serde_json::to_value(&raw.input_schema)
                .map_err(|err| PromptError::Pack(format!("tool {name}: {err}")))?;
            let examples = raw
                .examples
                .iter()
                .map(|example| {
                    serde_json::to_value(example)
                        .map_err(|err| PromptError::Pack(format!("tool {name}: {err}")))
                })
                .collect::<Result<Vec<_>, _>>()?;
            order.push(key.clone());
            entries.insert(
                key.clone(),
                PromptEntry {
                    key: key.clone(),
                    kind: PromptKind::Tool,
                    agent: "shared".to_owned(),
                    vars: Vec::new(),
                    title_en: name.clone(),
                    title_zh: name.clone(),
                    text: raw.description,
                },
            );
            tools.insert(
                name.clone(),
                ToolSchema {
                    name,
                    input_schema,
                    examples,
                },
            );
        }

        let variables_file: RawVariables = toml::from_str(VARIABLES_FILE)
            .map_err(|err| PromptError::Pack(format!("variables.toml: {err}")))?;
        let variables = variables_file
            .vars
            .into_iter()
            .map(|(name, raw)| {
                let mut fields: Vec<VariableFieldDoc> = raw
                    .fields
                    .into_iter()
                    .map(|(field, localized)| VariableFieldDoc {
                        name: field,
                        en: localized.en,
                        zh: localized.zh,
                    })
                    .collect();
                fields.sort_by(|a, b| a.name.cmp(&b.name));
                (
                    name.clone(),
                    VariableDoc {
                        name,
                        en: raw.en,
                        zh: raw.zh,
                        fields,
                    },
                )
            })
            .collect();

        Ok(Self {
            entries,
            order,
            variables,
            tools,
        })
    }

    /// The entry for `key`, when it exists.
    pub fn entry(&self, key: &str) -> Option<&PromptEntry> {
        self.entries.get(key)
    }

    /// Every key in pack order (prompt files, then tools).
    pub fn keys(&self) -> &[String] {
        &self.order
    }

    /// Every entry in pack order.
    pub fn entries(&self) -> impl Iterator<Item = &PromptEntry> {
        self.order.iter().filter_map(|key| self.entries.get(key))
    }

    /// The variable docs of the key's declared variables (spec/09 §1).
    pub fn variable_docs(&self, key: &str) -> Vec<&VariableDoc> {
        let Some(entry) = self.entry(key) else {
            return Vec::new();
        };
        entry
            .vars
            .iter()
            .filter_map(|name| self.variables.get(name))
            .collect()
    }

    /// The tool contract and examples, when `name` is a known tool.
    pub fn tool(&self, name: &str) -> Option<&ToolSchema> {
        self.tools.get(name)
    }

    /// Every tool in `tools.toml` order.
    pub fn tools(&self) -> impl Iterator<Item = &ToolSchema> {
        self.tools.values()
    }
}

// ───────────────────────────── raw TOML shapes ─────────────────────────────

#[derive(Debug, Deserialize)]
struct RawFile {
    #[serde(default)]
    prompt: Vec<RawEntry>,
}

#[derive(Debug, Deserialize)]
struct RawEntry {
    key: String,
    kind: RawKind,
    agent: String,
    #[serde(default)]
    vars: Vec<String>,
    title_en: String,
    title_zh: String,
    text: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawKind {
    System,
    Task,
    Block,
    Notice,
    Result,
    Shared,
}

#[derive(Debug, Deserialize)]
struct RawTools {
    #[serde(default)]
    tools: BTreeMap<String, RawTool>,
}

#[derive(Debug, Deserialize)]
struct RawTool {
    description: String,
    input_schema: toml::Value,
    #[serde(default)]
    examples: Vec<toml::Value>,
}

#[derive(Debug, Deserialize)]
struct RawVariables {
    #[serde(default)]
    vars: BTreeMap<String, RawVariable>,
}

#[derive(Debug, Deserialize)]
struct RawVariable {
    en: String,
    zh: String,
    #[serde(default)]
    fields: BTreeMap<String, RawLocalized>,
}

#[derive(Debug, Deserialize)]
struct RawLocalized {
    en: String,
    zh: String,
}
