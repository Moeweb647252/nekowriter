//! The prompt registry (spec/09): the embedded pack, override resolution, rendering, validation
//! and the label helpers.
//!
//! Every model-facing text is a template of the shipped pack (`prompts/`). This module loads it,
//! resolves the effective template of a key (book override → global override → shipped default,
//! spec/09 §3), renders it with strict undefined variables, and validates overrides before they
//! are stored so a broken prompt can never stop an agent (DECISIONS D-044).

pub mod context;
mod pack;
mod samples;

pub use pack::{PromptEntry, PromptKind, PromptPack, ToolSchema, VariableDoc, VariableFieldDoc};
pub use samples::{base_value as sample_base_value, variant_keys as sampled_variant_keys};

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use minijinja::syntax::SyntaxConfig;
use minijinja::value::Serde;
use minijinja::{AutoEscape, Environment, UndefinedBehavior, Value};
use sea_orm::EntityTrait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;

use crate::db::Db;
use crate::db::entities::prompt_overrides;
use crate::error::{AppError, ErrorCode};

/// A tool description is limited to 4,000 characters (spec/09 §4).
pub const MAX_TOOL_DESCRIPTION_CHARS: usize = 4_000;

/// Where the effective template of a key comes from (spec/09 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PromptSource {
    /// The shipped default in `prompts/`.
    Default,
    /// A global override (`prompt_overrides` with `scope = 'global'`).
    Global,
    /// A book override (`prompt_overrides` with `scope = 'book'`).
    Book,
}

impl PromptSource {
    pub fn as_str(self) -> &'static str {
        match self {
            PromptSource::Default => "default",
            PromptSource::Global => "global",
            PromptSource::Book => "book",
        }
    }
}

/// A prompt failure: an unknown key, an invalid template or a malformed pack.
#[derive(Debug, Clone, thiserror::Error)]
pub enum PromptError {
    #[error("unknown prompt key: {0}")]
    UnknownKey(String),
    #[error("{message}")]
    Template {
        line: Option<usize>,
        column: Option<usize>,
        message: String,
    },
    #[error("the prompt pack cannot be loaded: {0}")]
    Pack(String),
}

impl PromptError {
    pub fn line(&self) -> Option<usize> {
        match self {
            PromptError::Template { line, .. } => *line,
            _ => None,
        }
    }

    pub fn column(&self) -> Option<usize> {
        match self {
            PromptError::Template { column, .. } => *column,
            _ => None,
        }
    }

    /// Renders as an HTTP error (spec/10 §4): unknown keys are 404, invalid templates carry the
    /// `prompt.template_invalid` code with `details.line`, `details.column` and `details.message`.
    pub fn into_app_error(self) -> AppError {
        match self {
            PromptError::UnknownKey(key) => AppError::not_found(format!("prompt {key}")),
            PromptError::Template {
                line,
                column,
                message,
            } => AppError::new(ErrorCode::PromptTemplateInvalid, message.clone()).with_details(
                json!({
                    "line": line,
                    "column": column,
                    "message": message,
                }),
            ),
            PromptError::Pack(message) => AppError::internal(message),
        }
    }
}

/// The result of rendering one key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedPrompt {
    pub text: String,
    /// The key whose stored override failed at render time and fell back to the default; the
    /// session reports the `prompt.override_failed` warning (spec/09 §3).
    pub override_failed: Option<String>,
}

impl RenderedPrompt {
    /// The warning codes this render adds to a session (spec/10 §1).
    pub fn warnings(&self) -> Vec<String> {
        match &self.override_failed {
            Some(key) => {
                tracing::warn!(
                    key,
                    "prompt override failed at render time; the default was used"
                );
                vec!["prompt.override_failed".to_owned()]
            }
            None => Vec::new(),
        }
    }
}

/// A MiniJinja environment with the registry's rules (spec/09 §3).
///
/// `trim_blocks`/`lstrip_blocks` keep templates readable; strict undefined turns a missing
/// variable into an error, and there is no autoescaping because the output is model-facing text,
/// not HTML. Spans are tracked so validation can report a line and column.
fn new_environment() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.set_auto_escape_callback(|_| AutoEscape::None);
    env.set_debug(true);
    let syntax = SyntaxConfig::builder()
        .trim_blocks(true)
        .lstrip_blocks(true)
        .build()
        .unwrap_or_default();
    env.set_syntax(syntax);
    env
}

/// Converts a serializable JSON value into a MiniJinja value.
///
/// Templates receive plain data: the context structs of `prompts/context.rs` serialize to JSON
/// and reach MiniJinja through this conversion (spec/09 §2).
pub fn template_value<T: Serialize>(value: &T) -> Value {
    Value::from(Serde(value))
}

/// Builds the effective environment of one override resolution.
fn build_environment(
    pack: &PromptPack,
    global: &BTreeMap<String, String>,
    book: &BTreeMap<String, String>,
) -> (Environment<'static>, BTreeMap<String, PromptSource>) {
    let mut env = new_environment();
    let mut sources = BTreeMap::new();
    for key in pack.keys() {
        let Some(entry) = pack.entry(key) else {
            continue;
        };
        let (text, source) = if let Some(text) = book.get(key) {
            (text.clone(), PromptSource::Book)
        } else if let Some(text) = global.get(key) {
            (text.clone(), PromptSource::Global)
        } else {
            (entry.text.clone(), PromptSource::Default)
        };
        match env.add_template_owned(key.clone(), text) {
            Ok(()) => {
                sources.insert(key.clone(), source);
            }
            Err(err) => {
                // A stored override that no longer compiles can only come from a pack change;
                // the shipped default keeps the agent usable (spec/09 §3, D-044).
                tracing::warn!(
                    key,
                    error = %err,
                    "a stored prompt override does not compile; using the shipped default"
                );
                let _ = env.add_template_owned(key.clone(), entry.text.clone());
                sources.insert(key.clone(), PromptSource::Default);
            }
        }
    }
    (env, sources)
}

/// The rendered text normalization of spec/09 §3: trimmed, and three or more consecutive
/// newlines collapsed to two.
fn normalize(text: &str) -> String {
    let trimmed = text.trim();
    let mut out = String::with_capacity(trimmed.len());
    let mut newlines = 0usize;
    for ch in trimmed.chars() {
        if ch == '\n' {
            newlines += 1;
            if newlines <= 2 {
                out.push(ch);
            }
        } else {
            newlines = 0;
            out.push(ch);
        }
    }
    out
}

/// The key of the chapter-label cache.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ChapterLabelKey {
    track_kind: String,
    track_title: String,
    number: i64,
    title: String,
    with_title: bool,
}

/// One resolved prompt environment: the effective templates of a book (or of the whole app when
/// there is no book scope) plus the label helpers.
pub struct PromptSet {
    pack: Arc<PromptPack>,
    env: Environment<'static>,
    default_env: Arc<Environment<'static>>,
    sources: BTreeMap<String, PromptSource>,
    labels: Mutex<HashMap<(String, String), String>>,
    chapter_labels: Mutex<HashMap<ChapterLabelKey, String>>,
}

impl PromptSet {
    fn new(
        pack: Arc<PromptPack>,
        default_env: Arc<Environment<'static>>,
        global: &BTreeMap<String, String>,
        book: &BTreeMap<String, String>,
    ) -> Self {
        let (env, sources) = build_environment(&pack, global, book);
        Self {
            pack,
            env,
            default_env,
            sources,
            labels: Mutex::new(HashMap::new()),
            chapter_labels: Mutex::new(HashMap::new()),
        }
    }

    /// The shipped pack behind this set.
    pub fn pack(&self) -> &PromptPack {
        &self.pack
    }

    /// The effective source of one key (spec/09 §3).
    pub fn source(&self, key: &str) -> PromptSource {
        self.sources
            .get(key)
            .copied()
            .unwrap_or(PromptSource::Default)
    }

    /// Renders one key with a serializable context (spec/09 §3).
    pub fn render<S: Serialize>(
        &self,
        key: &str,
        context: &S,
    ) -> Result<RenderedPrompt, PromptError> {
        self.render_value(key, template_value(context))
    }

    /// Renders one key with an already-built context value.
    pub fn render_value(&self, key: &str, context: Value) -> Result<RenderedPrompt, PromptError> {
        let template = self
            .env
            .get_template(key)
            .map_err(|_| PromptError::UnknownKey(key.to_owned()))?;
        match template.render(&context) {
            Ok(text) => Ok(RenderedPrompt {
                text: normalize(&text),
                override_failed: None,
            }),
            Err(err) => {
                if self.source(key) == PromptSource::Default {
                    return Err(template_failure(key, &err));
                }
                // An override can never stop an agent: fall back to the shipped default and report
                // the failure so the session carries `prompt.override_failed` (spec/09 §3).
                let default = self
                    .default_env
                    .get_template(key)
                    .map_err(|_| PromptError::UnknownKey(key.to_owned()))?;
                let text = default
                    .render(&context)
                    .map_err(|err| template_failure(key, &err))?;
                Ok(RenderedPrompt {
                    text: normalize(&text),
                    override_failed: Some(key.to_owned()),
                })
            }
        }
    }

    /// The system prompt of a role (static by design, spec/09 §5 rule 1).
    pub fn system_prompt(&self, role: &str) -> Result<String, PromptError> {
        Ok(self.render(&format!("{role}.system"), &json!({}))?.text)
    }

    /// Every tool definition with its (possibly overridden) description and fixed schema
    /// (spec/09 §1).
    pub fn tool_definitions(&self) -> Result<Vec<nekowriter_llm::ToolDef>, PromptError> {
        self.pack
            .tools()
            .map(|tool| {
                let key = format!("tool.{}", tool.name);
                let rendered = self.render(&key, &json!({}))?;
                Ok(nekowriter_llm::ToolDef {
                    name: tool.name.clone(),
                    description: rendered.text,
                    input_schema: tool.input_schema.clone(),
                })
            })
            .collect()
    }

    /// The display label of an enum value, rendered through `shared.label` and cached per set
    /// (spec/09 §1). An unknown enum or value renders the raw value.
    pub fn label(&self, enum_name: &str, value: &str) -> String {
        let cache_key = (enum_name.to_owned(), value.to_owned());
        if let Some(hit) = lock(&self.labels).get(&cache_key) {
            return hit.clone();
        }
        let rendered = self
            .render_value(
                "shared.label",
                template_value(&json!({ "enum": enum_name, "value": value })),
            )
            .map(|rendered| rendered.text)
            .unwrap_or_else(|err| {
                tracing::warn!(
                    enum_name,
                    value,
                    error = %err,
                    "shared.label failed to render; using the raw value"
                );
                value.to_owned()
            });
        let rendered = if rendered.is_empty() {
            value.to_owned()
        } else {
            rendered
        };
        lock(&self.labels).insert(cache_key, rendered.clone());
        rendered
    }

    /// A chapter label rendered through `shared.chapter_label` and cached per set (spec/09 §1).
    pub fn chapter_label(
        &self,
        track_kind: &str,
        track_title: &str,
        number: i64,
        title: &str,
        with_title: bool,
    ) -> String {
        let cache_key = ChapterLabelKey {
            track_kind: track_kind.to_owned(),
            track_title: track_title.to_owned(),
            number,
            title: title.to_owned(),
            with_title,
        };
        if let Some(hit) = lock(&self.chapter_labels).get(&cache_key) {
            return hit.clone();
        }
        let rendered = self
            .render_value(
                "shared.chapter_label",
                template_value(&json!({
                    "track_kind": track_kind,
                    "track_title": track_title,
                    "number": number,
                    "title": title,
                    "with_title": with_title,
                })),
            )
            .map(|rendered| rendered.text)
            .unwrap_or_else(|err| {
                tracing::warn!(error = %err, "shared.chapter_label failed to render");
                String::new()
            });
        lock(&self.chapter_labels).insert(cache_key, rendered.clone());
        rendered
    }
}

/// The prompt registry: the loaded pack, the default environment and the per-book cache
/// (spec/09 §3: "cached per book and invalidated on override changes").
///
/// One registry belongs to one [`crate::state::AppState`], so its cache is keyed by book only.
/// The embedded pack is parsed and validated once per process (it cannot change at runtime) and
/// shared between registries, because the resolved sets differ per database.
pub struct PromptRegistry {
    pack: Arc<PromptPack>,
    default_env: Arc<Environment<'static>>,
    cache: Mutex<HashMap<Option<i32>, Arc<PromptSet>>>,
}

impl PromptRegistry {
    /// The embedded pack, parsed and validated once (spec/01 §5 step 6).
    fn embedded_pack() -> Result<Arc<PromptPack>, PromptError> {
        static PACK: OnceLock<Result<Arc<PromptPack>, String>> = OnceLock::new();
        match PACK.get_or_init(|| {
            let pack = Arc::new(PromptPack::embedded().map_err(|err| err.to_string())?);
            // Validate every shipped default against every sample context: a failure is a build
            // defect that must abort startup (spec/09 §2).
            let registry = PromptRegistry {
                pack: pack.clone(),
                default_env: Arc::new(
                    build_environment(&pack, &BTreeMap::new(), &BTreeMap::new()).0,
                ),
                cache: Mutex::new(HashMap::new()),
            };
            registry.validate_pack().map_err(|err| err.to_string())?;
            Ok(pack)
        }) {
            Ok(pack) => Ok(pack.clone()),
            Err(message) => Err(PromptError::Pack(message.clone())),
        }
    }

    /// Builds a registry over the embedded pack. The pack itself is loaded once per process.
    pub fn embedded() -> Result<Self, PromptError> {
        let pack = Self::embedded_pack()?;
        let default_env = Arc::new(build_environment(&pack, &BTreeMap::new(), &BTreeMap::new()).0);
        Ok(Self {
            pack,
            default_env,
            cache: Mutex::new(HashMap::new()),
        })
    }

    /// The loaded pack.
    pub fn pack(&self) -> &Arc<PromptPack> {
        &self.pack
    }

    /// A set with no overrides (tests, default rendering).
    pub fn defaults_only(&self) -> PromptSet {
        PromptSet::new(
            self.pack.clone(),
            self.default_env.clone(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
    }

    /// Renders every shipped default with every sample context, so a build defect aborts startup
    /// (spec/01 §5 step 6, spec/09 §2).
    fn validate_pack(&self) -> Result<(), PromptError> {
        for key in self.pack.keys() {
            let samples = self.sample_contexts(key);
            if samples.is_empty() {
                return Err(PromptError::Pack(format!(
                    "{key} has no sample context; add one to samples.json"
                )));
            }
            for (index, sample) in samples.iter().enumerate() {
                let template = self
                    .default_env
                    .get_template(key)
                    .map_err(|_| PromptError::UnknownKey(key.clone()))?;
                template.render(template_value(sample)).map_err(|err| {
                    PromptError::Pack(format!("{key} sample {index} failed to render: {}", err))
                })?;
            }
        }
        Ok(())
    }

    /// One sample context per branch of the shipped template of `key` (spec/09 §2, §4).
    pub fn sample_contexts(&self, key: &str) -> Vec<serde_json::Value> {
        self.pack
            .entry(key)
            .map(samples::contexts)
            .unwrap_or_default()
    }

    /// Validates a stored override (spec/09 §4): template syntax (with line and column), then a
    /// render with every sample context under strict undefined.
    pub fn validate_override(&self, key: &str, content: &str) -> Result<(), PromptError> {
        let entry = self
            .pack
            .entry(key)
            .ok_or_else(|| PromptError::UnknownKey(key.to_owned()))?;
        if entry.kind == PromptKind::Tool {
            if content.trim().is_empty() {
                return Err(PromptError::Template {
                    line: None,
                    column: None,
                    message: "a tool description must not be empty".to_owned(),
                });
            }
            if content.chars().count() > MAX_TOOL_DESCRIPTION_CHARS {
                return Err(PromptError::Template {
                    line: None,
                    column: None,
                    message: format!(
                        "a tool description is at most {MAX_TOOL_DESCRIPTION_CHARS} characters"
                    ),
                });
            }
        }

        let mut env = new_environment();
        for pack_key in self.pack.keys() {
            let text = if pack_key == key {
                content.to_owned()
            } else {
                self.pack
                    .entry(pack_key)
                    .map(|entry| entry.text.clone())
                    .unwrap_or_default()
            };
            env.add_template_owned(pack_key.clone(), text)
                .map_err(|err| template_failure(key, &err))?;
        }
        let template = env
            .get_template(key)
            .map_err(|err| template_failure(key, &err))?;
        for (index, sample) in self.sample_contexts(key).iter().enumerate() {
            template.render(template_value(sample)).map_err(|err| {
                let mut failure = template_failure(key, &err);
                if let PromptError::Template { message, .. } = &mut failure {
                    *message = format!("sample {index}: {message}");
                }
                failure
            })?;
        }
        Ok(())
    }

    /// The resolved set of a book (or of the app when `book_id` is `None`), cached until an
    /// override changes (spec/09 §3).
    pub async fn for_book(
        &self,
        db: &Db,
        book_id: Option<i32>,
    ) -> Result<Arc<PromptSet>, AppError> {
        if let Some(set) = lock(&self.cache).get(&book_id) {
            return Ok(set.clone());
        }
        let rows = prompt_overrides::Entity::find()
            .all(db.conn())
            .await
            .map_err(AppError::from)?;
        let mut global = BTreeMap::new();
        let mut book = BTreeMap::new();
        for row in rows {
            match row.scope.as_str() {
                "global" => {
                    global.insert(row.key, row.content);
                }
                "book" if row.book_id == book_id => {
                    book.insert(row.key, row.content);
                }
                _ => {}
            }
        }
        let set = Arc::new(PromptSet::new(
            self.pack.clone(),
            self.default_env.clone(),
            &global,
            &book,
        ));
        lock(&self.cache).insert(book_id, set.clone());
        Ok(set)
    }

    /// Drops the cached sets after an override change. A global change invalidates every book
    /// (spec/09 §3).
    pub fn invalidate(&self, book_id: Option<i32>) {
        let mut cache = lock(&self.cache);
        match book_id {
            Some(book_id) => {
                cache.remove(&Some(book_id));
            }
            None => cache.clear(),
        }
    }

    /// Drops every cached set (used by tests).
    pub fn invalidate_all(&self) {
        lock(&self.cache).clear();
    }
}

/// Locks a mutex, recovering the guard from a poisoned lock: a panicking render must not break
/// the registry for the rest of the process.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Maps a MiniJinja error to a line/column-bearing [`PromptError`].
fn template_failure(key: &str, error: &minijinja::Error) -> PromptError {
    let line = error.line();
    let column = error.range().and_then(|range| {
        error
            .template_source()
            .map(|source| column_at(source, range.start))
    });
    PromptError::Template {
        line,
        column,
        message: format!("{key}: {error}"),
    }
}

/// The 1-based column of a byte offset inside a template source.
fn column_at(source: &str, offset: usize) -> usize {
    let offset = offset.min(source.len());
    let line_start = source[..offset]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    source[line_start..offset].chars().count() + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_trims_and_collapses_newlines() {
        assert_eq!(normalize("  a\n\n\n\nb  "), "a\n\nb");
        assert_eq!(normalize("\n\n a \n\n"), "a");
        assert_eq!(normalize("a\nb"), "a\nb");
    }

    #[test]
    fn columns_count_characters_not_bytes() {
        assert_eq!(column_at("abc", 0), 1);
        assert_eq!(column_at("abc", 2), 3);
        assert_eq!(column_at("ab\ncd", 3), 1);
        assert_eq!(column_at("ab\ncd", 4), 2);
        assert_eq!(column_at("中文\nx", 7), 1);
    }
}
