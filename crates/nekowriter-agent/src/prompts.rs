//! The built-in prompt texts (`docs/product-spec.md` §13.1), embedded at compile time.

use nekowriter_core::prompt::{PromptKey, PromptRole};

/// The built-in text of one half of a prompt pair.
#[must_use]
pub fn builtin(key: PromptKey, role: PromptRole) -> &'static str {
    macro_rules! pair {
        ($name:literal) => {
            match role {
                PromptRole::System => {
                    include_str!(concat!("../resources/prompts/", $name, ".system.md"))
                }
                PromptRole::User => {
                    include_str!(concat!("../resources/prompts/", $name, ".user.md"))
                }
            }
        };
    }
    match key {
        PromptKey::PreparationGenerate => pair!("preparation.generate"),
        PromptKey::ChapterOutline => pair!("chapter.outline"),
        PromptKey::ChapterWrite => pair!("chapter.write"),
        PromptKey::ReviewPreparationBackground => pair!("review.preparation.background"),
        PromptKey::ReviewPreparationResearch => pair!("review.preparation.research"),
        PromptKey::ReviewPreparationCharacters => pair!("review.preparation.characters"),
        PromptKey::ReviewPreparationOutline => pair!("review.preparation.outline"),
        PromptKey::ReviewChapterOutline => pair!("review.chapter.outline"),
        PromptKey::ReviewChapterEdit => pair!("review.chapter.edit"),
        PromptKey::ReviewChapterFinal => pair!("review.chapter.final"),
    }
}

#[cfg(test)]
mod tests {
    use nekowriter_core::template::Template;

    use super::*;

    #[test]
    fn every_builtin_prompt_uses_exactly_its_declared_variables() {
        for key in PromptKey::ALL {
            for role in [PromptRole::System, PromptRole::User] {
                let template = Template::parse(builtin(key, role)).unwrap();
                template
                    .check(&key.template_variables(role))
                    .unwrap_or_else(|error| panic!("{key:?} {role:?}: {error}"));
            }
        }
    }
}
