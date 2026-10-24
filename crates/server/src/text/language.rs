//! Language helpers (spec/03 §8, spec/09 §1).
//!
//! `is_unspaced` decides CJK-aware joining; `display_name` is for logs and diagnostics only —
//! the label a model sees is rendered by the prompt pack (`shared.label {enum: "language"}`),
//! so no model-facing wording lives here.

/// Languages written without spaces between words (spec/03 §8).
const UNSPACED: [&str; 6] = ["zh", "ja", "th", "lo", "km", "my"];

/// English display names for the languages the UI offers, used by logs and diagnostics.
const DISPLAY_NAMES: [(&str, &str); 23] = [
    ("ar", "Arabic"),
    ("de", "German"),
    ("en", "English"),
    ("es", "Spanish"),
    ("fr", "French"),
    ("hi", "Hindi"),
    ("id", "Indonesian"),
    ("it", "Italian"),
    ("ja", "Japanese"),
    ("km", "Khmer"),
    ("ko", "Korean"),
    ("lo", "Lao"),
    ("my", "Burmese"),
    ("nl", "Dutch"),
    ("pl", "Polish"),
    ("pt", "Portuguese"),
    ("ru", "Russian"),
    ("sv", "Swedish"),
    ("th", "Thai"),
    ("tr", "Turkish"),
    ("uk", "Ukrainian"),
    ("vi", "Vietnamese"),
    ("zh", "Chinese"),
];

/// The first subtag of a BCP-47 tag, lowercased: `"zh-CN"` → `"zh"`, `"EN"` → `"en"`.
pub fn primary_subtag(language: &str) -> String {
    language
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// True when the language is written without spaces between words (spec/03 §8).
pub fn is_unspaced(language: &str) -> bool {
    UNSPACED.contains(&primary_subtag(language).as_str())
}

/// The English display name of a language tag, or the tag itself when it is not in the table.
pub fn display_name(language: &str) -> String {
    let primary = primary_subtag(language);
    match DISPLAY_NAMES.iter().find(|(tag, _)| *tag == primary) {
        Some((_, name)) => (*name).to_owned(),
        None => language.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unspaced_languages_table() {
        let cases: &[(&str, bool)] = &[
            ("zh", true),
            ("zh-CN", true),
            ("zh-Hant-TW", true),
            ("ja", true),
            ("th", true),
            ("lo", true),
            ("km", true),
            ("my", true),
            ("en", false),
            ("en-US", false),
            ("de-DE", false),
            ("ko", false),
            ("", false),
        ];
        for (language, expected) in cases {
            assert_eq!(is_unspaced(language), *expected, "language {language:?}");
        }
    }

    #[test]
    fn primary_subtag_normalizes_case_and_separators() {
        assert_eq!(primary_subtag("zh-CN"), "zh");
        assert_eq!(primary_subtag("EN"), "en");
        assert_eq!(primary_subtag("pt_BR"), "pt");
        assert_eq!(primary_subtag(""), "");
    }

    #[test]
    fn display_names_are_english_and_fall_back_to_the_tag() {
        assert_eq!(display_name("zh-CN"), "Chinese");
        assert_eq!(display_name("en"), "English");
        assert_eq!(display_name("ja"), "Japanese");
        assert_eq!(display_name("xx-YY"), "xx-YY");
    }
}
