//! The supported embedding model table (spec/07 §4).
//!
//! The table lives in `models.toml` because one query prefix is Chinese: model-facing and
//! non-English text must not appear in code (AGENTS.md §2, DECISIONS D-041).

use std::sync::LazyLock;

use serde::Deserialize;

const MODELS_TOML: &str = include_str!("../models.toml");

/// The default model key.
pub const DEFAULT_MODEL_KEY: &str = "multilingual-e5-small";

#[derive(Debug, Clone, Deserialize)]
pub struct ModelTable {
    pub models: Vec<ModelEntry>,
}

/// One supported model: its key, fastembed variant name, dimension and input prefixes.
#[derive(Debug, Clone, Deserialize)]
pub struct ModelEntry {
    /// The stable key stored in `app_settings["embedding"].model`.
    pub key: String,
    /// The `fastembed::EmbeddingModel` variant name.
    pub variant: String,
    pub dimension: usize,
    pub query_prefix: String,
    pub passage_prefix: String,
}

impl ModelEntry {
    /// Applies the query convention to a search query.
    pub fn query_text(&self, text: &str) -> String {
        format!("{}{}", self.query_prefix, text)
    }

    /// Applies the passage convention to stored text.
    pub fn passage_text(&self, text: &str) -> String {
        format!("{}{}", self.passage_prefix, text)
    }
}

/// The parsed table, read once.
pub static SUPPORTED_MODELS: LazyLock<Vec<ModelEntry>> = LazyLock::new(|| {
    let parsed: ModelTable = toml::from_str(MODELS_TOML).unwrap_or_else(|err| {
        // A malformed compile-time table is a build defect; fail loudly at first use.
        panic!("crates/embed/models.toml is not valid TOML: {err}")
    });
    parsed.models
});

/// Looks up a model by key.
pub fn model(key: &str) -> Option<&'static ModelEntry> {
    SUPPORTED_MODELS.iter().find(|entry| entry.key == key)
}

/// The default model entry.
pub fn default_model() -> &'static ModelEntry {
    model(DEFAULT_MODEL_KEY).unwrap_or_else(|| {
        SUPPORTED_MODELS
            .first()
            .expect("models.toml must define at least one model")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_parses_and_has_the_three_models() {
        assert_eq!(SUPPORTED_MODELS.len(), 3);
        let keys: Vec<&str> = SUPPORTED_MODELS.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "multilingual-e5-small",
                "bge-small-zh-v1.5",
                "embeddinggemma-300m-q"
            ]
        );
        assert_eq!(default_model().dimension, 384);
        assert_eq!(model("bge-small-zh-v1.5").expect("model").dimension, 512);
        assert_eq!(
            model("embeddinggemma-300m-q").expect("model").dimension,
            768
        );
    }

    #[test]
    fn prefixes_are_applied() {
        let e5 = default_model();
        assert_eq!(e5.query_text("fox"), "query: fox");
        assert_eq!(e5.passage_text("fox"), "passage: fox");
        let bge = model("bge-small-zh-v1.5").expect("bge");
        assert!(bge.passage_text("fox") == "fox");
        assert!(bge.query_text("fox").ends_with("fox"));
    }
}
