//! Byte-equality of the subdivided kickoff blocks (M24 24.2, D-172).
//!
//! The old parent templates are kept as fixtures, never shipped, so this proves the new child
//! templates concatenate to the old rendering rather than comparing a template with itself.

use minijinja::syntax::SyntaxConfig;
use minijinja::{AutoEscape, Environment, UndefinedBehavior};
use nekowriter::agent::context::{join_text_blocks, kickoff_blocks};
use nekowriter::db::json::SessionKind;
use nekowriter::prompts::{PromptRegistry, template_value};

const OLD_SETTINGS: &str = include_str!("fixtures/old_block_settings.jinja");
const OLD_MEMORY: &str = include_str!("fixtures/old_block_memory.jinja");

fn old_environment() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.set_auto_escape_callback(|_| AutoEscape::None);
    env.set_syntax(
        SyntaxConfig::builder()
            .trim_blocks(true)
            .lstrip_blocks(true)
            .build()
            .unwrap_or_default(),
    );
    env.add_template_owned("block.settings".to_owned(), OLD_SETTINGS)
        .unwrap();
    env.add_template_owned("block.memory".to_owned(), OLD_MEMORY)
        .unwrap();
    env
}

fn normalize(text: &str) -> String {
    let trimmed = text.trim();
    let mut out = String::new();
    let mut newlines = 0;
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

fn render_children(
    registry: &PromptRegistry,
    keys: &[&str],
    context: &serde_json::Value,
) -> String {
    let set = registry.defaults_only();
    let blocks: Vec<nekowriter::db::json::StoredBlock> = keys
        .iter()
        .filter_map(|key| {
            let text = set.render_value(key, template_value(context)).unwrap().text;
            (!text.is_empty()).then(|| nekowriter::db::json::StoredBlock::Text {
                text,
                cache: None,
                source: Some((*key).to_owned()),
            })
        })
        .collect();
    join_text_blocks(&blocks)
        .into_iter()
        .map(|block| block.text)
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[test]
fn child_kickoff_blocks_are_byte_identical_to_the_old_parents() {
    let registry = PromptRegistry::embedded().unwrap();
    let old_env = old_environment();
    for kind in [
        SessionKind::ChapterOutline,
        SessionKind::ChapterWrite,
        SessionKind::ChapterRevise,
        SessionKind::ChapterReview,
        SessionKind::ChapterMaintenance,
        SessionKind::CanonSeed,
        SessionKind::ChapterResync,
    ] {
        let keys = kickoff_blocks(kind, false, false);
        let settings_keys: Vec<&str> = keys
            .iter()
            .copied()
            .filter(|key| key.starts_with("block.settings."))
            .collect();
        if !settings_keys.is_empty() {
            let context = registry
                .sample_contexts(settings_keys[0])
                .into_iter()
                .next()
                .unwrap();
            let old = normalize(
                &old_env
                    .get_template("block.settings")
                    .unwrap()
                    .render(template_value(&context))
                    .unwrap(),
            );
            let new = render_children(&registry, &settings_keys, &context);
            assert_eq!(new, old, "settings for {kind:?}");
        }
        let memory_keys: Vec<&str> = keys
            .iter()
            .copied()
            .filter(|key| key.starts_with("block.memory."))
            .collect();
        if !memory_keys.is_empty() {
            let context = registry
                .sample_contexts(memory_keys[0])
                .into_iter()
                .next()
                .unwrap();
            let old = normalize(
                &old_env
                    .get_template("block.memory")
                    .unwrap()
                    .render(template_value(&context))
                    .unwrap(),
            );
            let new = normalize(&render_children(&registry, &memory_keys, &context));
            assert_eq!(new, old, "memory for {kind:?}");
        }
    }
}
