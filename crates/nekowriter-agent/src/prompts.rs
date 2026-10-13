//! The built-in prompt texts (`docs/product-spec.md` §13.1), embedded at compile time.

use nekowriter_core::prompt::{PromptKey, PromptRole};

/// The built-in text of one half of a prompt pair.
#[must_use]
pub fn builtin(key: PromptKey, role: PromptRole) -> &'static str {
    match (key, role) {
        (PromptKey::PreparationGenerate, PromptRole::System) => {
            include_str!("../resources/prompts/preparation.generate.system.md")
        }
        (PromptKey::PreparationGenerate, PromptRole::User) => {
            include_str!("../resources/prompts/preparation.generate.user.md")
        }
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
