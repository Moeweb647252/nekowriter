//! Entity reference resolution (spec/05 §1.7).
//!
//! Parameters named `entity`, `source`, `target`, `other` (strings) and `entities` (string
//! arrays) are resolved against **visible** entities in this order: `#12` or `12` → id; exact
//! name; exact alias; case-insensitive name or alias; unique substring of a name or alias.
//! No match → `error.entity_not_found` with up to 5 suggestions; several → `error.entity_ambiguous`
//! listing the candidates.
//!
//! The canon tables and their visibility land in M5, so this module works on a caller-supplied
//! snapshot of the visible entities. That keeps the resolution order unit-testable now (PLAN
//! M3.2) and lets the canon tools of M5 reuse it unchanged.

use serde::Serialize;

use super::{ToolError, ToolErrorKind};
use crate::prompts::PromptSet;

/// One visible entity, as the resolver sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VisibleEntity {
    pub id: i32,
    pub name: String,
    pub aliases: Vec<String>,
    /// A `shared.label` (`entity_kind`) rendering, used by `error.entity_ambiguous`.
    pub kind_label: String,
}

/// Resolves entity parameters against the visible entities (spec/05 §1.7).
pub struct EntityResolver {
    entities: Vec<VisibleEntity>,
}

impl EntityResolver {
    pub fn new(entities: Vec<VisibleEntity>) -> Self {
        Self { entities }
    }

    /// Resolves one reference, or returns the error the tool hands to the model.
    pub fn resolve(&self, reference: &str) -> Result<&VisibleEntity, ToolError> {
        let trimmed = reference.trim();
        if trimmed.is_empty() {
            return Err(not_found(trimmed, self.suggestions(trimmed)));
        }

        // 1. `#12` or `12` → id.
        if let Some(id) = parse_id(trimmed)
            && let Some(found) = self.entities.iter().find(|entity| entity.id == id)
        {
            return Ok(found);
        }

        // 2. Exact name.
        if let Some(found) = self.entities.iter().find(|entity| entity.name == trimmed) {
            return Ok(found);
        }

        // 3. Exact alias.
        if let Some(found) = self
            .entities
            .iter()
            .find(|entity| entity.aliases.iter().any(|alias| alias == trimmed))
        {
            return Ok(found);
        }

        // 4. Case-insensitive name or alias.
        let folded = trimmed.to_lowercase();
        let case_matches: Vec<&VisibleEntity> = self
            .entities
            .iter()
            .filter(|entity| {
                entity.name.to_lowercase() == folded
                    || entity
                        .aliases
                        .iter()
                        .any(|alias| alias.to_lowercase() == folded)
            })
            .collect();
        match case_matches.len() {
            1 => return Ok(case_matches[0]),
            n if n > 1 => return Err(ambiguous(trimmed, &case_matches)),
            _ => {}
        }

        // 5. Unique substring of a name or alias.
        let substring_matches: Vec<&VisibleEntity> = self
            .entities
            .iter()
            .filter(|entity| {
                entity.name.to_lowercase().contains(&folded)
                    || entity
                        .aliases
                        .iter()
                        .any(|alias| alias.to_lowercase().contains(&folded))
            })
            .collect();
        match substring_matches.len() {
            1 => Ok(substring_matches[0]),
            n if n > 1 => Err(ambiguous(trimmed, &substring_matches)),
            _ => Err(not_found(trimmed, self.suggestions(trimmed))),
        }
    }

    /// Up to 5 entities that share characters with the reference (spec/05 §1.7).
    fn suggestions(&self, reference: &str) -> Vec<(&VisibleEntity, usize)> {
        let mut scored: Vec<(&VisibleEntity, usize)> = self
            .entities
            .iter()
            .filter_map(|entity| {
                let score = shared_chars(&entity.name, reference);
                let alias_score = entity
                    .aliases
                    .iter()
                    .map(|alias| shared_chars(alias, reference))
                    .max()
                    .unwrap_or(0);
                let score = score.max(alias_score);
                if score == 0 {
                    None
                } else {
                    Some((entity, score))
                }
            })
            .collect();
        scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.id.cmp(&b.0.id)));
        scored.truncate(5);
        scored
    }
}

/// `error.entity_not_found` with the rendered suggestions (spec/05 §1.7).
fn not_found(reference: &str, suggestions: Vec<(&VisibleEntity, usize)>) -> ToolError {
    let suggestions: Vec<serde_json::Value> = suggestions
        .into_iter()
        .map(|(entity, _)| serde_json::json!({ "id": entity.id, "name": entity.name }))
        .collect();
    ToolError::new(ToolErrorKind::EntityNotFound)
        .with("query", reference.to_owned())
        .with("suggestions", suggestions)
}

/// `error.entity_ambiguous` listing the candidates (spec/05 §1.7).
fn ambiguous(reference: &str, candidates: &[&VisibleEntity]) -> ToolError {
    let candidates: Vec<serde_json::Value> = candidates
        .iter()
        .map(|entity| {
            serde_json::json!({
                "id": entity.id,
                "name": entity.name,
                "kind_label": entity.kind_label,
            })
        })
        .collect();
    ToolError::new(ToolErrorKind::EntityAmbiguous)
        .with("query", reference.to_owned())
        .with("candidates", candidates)
}

/// Parses `#12` or `12`.
fn parse_id(reference: &str) -> Option<i32> {
    let digits = reference.strip_prefix('#').unwrap_or(reference);
    if digits.is_empty() || !digits.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    digits.parse::<i32>().ok()
}

/// The number of characters the two strings share (a cheap suggestion score).
fn shared_chars(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    left.chars().filter(|ch| right.contains(ch)).count()
}

/// Renders the resolution failure through `result.error` so a caller can return it directly.
pub fn render_error(prompts: &PromptSet, error: &ToolError) -> String {
    super::ToolOutcome::error(prompts, error).text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(id: i32, name: &str, aliases: &[&str]) -> VisibleEntity {
        VisibleEntity {
            id,
            name: name.to_owned(),
            aliases: aliases.iter().map(|alias| (*alias).to_owned()).collect(),
            kind_label: "character".to_owned(),
        }
    }

    fn resolver() -> EntityResolver {
        EntityResolver::new(vec![
            entity(12, "Lin Fan", &["A-Fan"]),
            entity(15, "Zhang San", &[]),
            entity(21, "Su Wan", &["Wan'er"]),
            entity(30, "Lin", &[]),
        ])
    }

    #[test]
    fn ids_resolve_with_and_without_the_hash() {
        assert_eq!(resolver().resolve("#12").expect("id").id, 12);
        assert_eq!(resolver().resolve("12").expect("id").id, 12);
        assert_eq!(resolver().resolve(" #15 ").expect("id").id, 15);
    }

    #[test]
    fn exact_name_wins_over_an_alias() {
        assert_eq!(resolver().resolve("Zhang San").expect("name").id, 15);
        assert_eq!(resolver().resolve("A-Fan").expect("alias").id, 12);
        assert_eq!(resolver().resolve("Wan'er").expect("alias").id, 21);
    }

    #[test]
    fn case_insensitive_matches_are_used_after_exact_ones() {
        assert_eq!(resolver().resolve("lin fan").expect("case").id, 12);
        assert_eq!(resolver().resolve("a-fan").expect("case alias").id, 12);
    }

    #[test]
    fn a_unique_substring_matches() {
        assert_eq!(resolver().resolve("Zhang").expect("substring").id, 15);
    }

    #[test]
    fn several_substring_matches_are_ambiguous() {
        let error = resolver().resolve("an").expect_err("ambiguous");
        assert_eq!(error.kind, ToolErrorKind::EntityAmbiguous);
        let candidates = error
            .fields
            .get("candidates")
            .and_then(|value| value.as_array())
            .expect("candidates");
        assert!(candidates.len() >= 2, "{candidates:?}");
    }

    #[test]
    fn no_match_offers_suggestions() {
        let error = resolver().resolve("Lin Fann").expect_err("not found");
        assert_eq!(error.kind, ToolErrorKind::EntityNotFound);
        let suggestions = error
            .fields
            .get("suggestions")
            .and_then(|value| value.as_array())
            .expect("suggestions");
        assert!(!suggestions.is_empty(), "a near miss should suggest");
        assert!(
            suggestions
                .iter()
                .any(|candidate| candidate["id"] == serde_json::json!(12)),
            "{suggestions:?}"
        );
    }

    #[test]
    fn no_match_at_all_has_empty_suggestions() {
        let error = resolver().resolve("zzzz").expect_err("not found");
        assert_eq!(error.kind, ToolErrorKind::EntityNotFound);
        let suggestions = error
            .fields
            .get("suggestions")
            .and_then(|value| value.as_array())
            .expect("suggestions");
        assert!(suggestions.is_empty());
    }

    #[test]
    fn an_unknown_id_is_not_found() {
        let error = resolver().resolve("#999").expect_err("not found");
        assert_eq!(error.kind, ToolErrorKind::EntityNotFound);
    }

    #[test]
    fn suggestions_are_capped_at_five() {
        let many: Vec<VisibleEntity> = (1..=10)
            .map(|id| entity(id, &format!("Lin{id}"), &[]))
            .collect();
        let resolver = EntityResolver::new(many);
        let error = resolver.resolve("Linzz").expect_err("not found");
        let suggestions = error
            .fields
            .get("suggestions")
            .and_then(|value| value.as_array())
            .expect("suggestions");
        assert!(suggestions.len() <= 5, "{suggestions:?}");
    }
}
