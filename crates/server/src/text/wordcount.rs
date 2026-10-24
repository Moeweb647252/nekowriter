//! UAX #29 word count (spec/02 §4 `word_count`, spec/11 §8).
//!
//! The count is Unicode UAX #29 word segmentation (`unicode-segmentation`), except that CJK
//! text counts the way authors count it: every Han ideograph and every Hiragana character is
//! one word, while a run of Katakana is one word. The web client mirrors this rule for live
//! typing; `wordcount_vectors.json` holds the cases both sides assert.

/// Ideographs that count one each. U+3007 (ideographic number zero) is a Han ideograph by
/// Unicode's script property, so an author counts it like one; leaving it out made the server
/// split "一九〇〇年" into six words while the client said five.
const HAN_RANGES: [(u32, u32); 6] = [
    (0x3400, 0x4dbf),   // CJK Unified Ideographs Extension A
    (0x4e00, 0x9fff),   // CJK Unified Ideographs
    (0xf900, 0xfaff),   // CJK Compatibility Ideographs
    (0x20000, 0x2fa1f), // CJK Unified Ideographs Extensions B..F
    (0x3005, 0x3005),   // Ideographic iteration mark
    (0x3007, 0x3007),   // Ideographic number zero
];

/// Hiragana (including the iteration marks, but not the prolonged sound mark).
const HIRAGANA_RANGES: [(u32, u32); 1] = [(0x3041, 0x3096)];

/// Katakana, including `ー` (prolonged sound mark) and the iteration marks. The halfwidth
/// Katakana block belongs to the same script, so it counts as Katakana too.
const KATAKANA_RANGES: [(u32, u32); 4] = [
    (0x30a1, 0x30fa), // Katakana
    (0x30fc, 0x30ff), // ー and the iteration marks
    (0xff66, 0xff9d), // Halfwidth Katakana
    (0xff9e, 0xff9f), // Halfwidth voiced sound marks
];

fn in_ranges(code: u32, ranges: &[(u32, u32)]) -> bool {
    ranges
        .iter()
        .any(|(start, end)| code >= *start && code <= *end)
}

pub fn is_han(ch: char) -> bool {
    in_ranges(ch as u32, &HAN_RANGES)
}

pub fn is_hiragana(ch: char) -> bool {
    in_ranges(ch as u32, &HIRAGANA_RANGES)
}

pub fn is_katakana(ch: char) -> bool {
    in_ranges(ch as u32, &KATAKANA_RANGES)
}

/// Counts the words of `text` (UAX #29 with the CJK rule above).
pub fn count_words(text: &str) -> i32 {
    use unicode_segmentation::UnicodeSegmentation;

    let mut total: i32 = 0;
    for word in text.unicode_words() {
        total = total.saturating_add(count_word(word));
    }
    total
}

/// Counts one UAX #29 word (already free of surrounding punctuation).
fn count_word(word: &str) -> i32 {
    let mut count: i32 = 0;
    let mut chars = word.chars().peekable();
    while let Some(ch) = chars.next() {
        if is_han(ch) || is_hiragana(ch) {
            count += 1;
        } else if is_katakana(ch) {
            // A run of Katakana is one word.
            count += 1;
            while chars
                .peek()
                .is_some_and(|next| is_katakana(*next) && !is_han(*next))
            {
                chars.next();
            }
        } else {
            // A run of letters, marks and digits in any other script is one word.
            count += 1;
            while chars
                .peek()
                .is_some_and(|next| !is_han(*next) && !is_hiragana(*next) && !is_katakana(*next))
            {
                chars.next();
            }
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct Vector {
        text: String,
        words: i32,
    }

    #[derive(Debug, Deserialize)]
    struct Vectors {
        cases: Vec<Vector>,
    }

    fn vectors() -> Vectors {
        serde_json::from_str(include_str!("wordcount_vectors.json")).expect("wordcount vectors")
    }

    #[test]
    fn shared_vectors() {
        let vectors = vectors();
        assert!(
            vectors.cases.len() >= 12,
            "the shared vector file needs enough cases"
        );
        for case in vectors.cases {
            assert_eq!(
                count_words(&case.text),
                case.words,
                "text {:?} should count as {} word(s)",
                case.text,
                case.words
            );
        }
    }

    #[test]
    fn cjk_words_table() {
        let cases: &[(&str, i32)] = &[
            ("风吹过山谷", 5),
            ("“你好，世界。”", 4),
            ("第12章", 3),
            ("ひらがな", 4),
            ("カタカナ", 1),
            ("彼はカタカナを書いた", 7),
            ("漢字とひらがな", 7),
        ];
        for (text, words) in cases {
            assert_eq!(count_words(text), *words, "text {text:?}");
        }
    }

    #[test]
    fn english_words_table() {
        let cases: &[(&str, i32)] = &[
            ("The wind blew through the valley", 6),
            ("don't e.g. 3.14 1,000", 4),
            ("well-known", 2),
            ("café naïve", 2),
            ("  ,.!? \n\t ", 0),
            ("", 0),
        ];
        for (text, words) in cases {
            assert_eq!(count_words(text), *words, "text {text:?}");
        }
    }

    /// The classes where the client's approximation and the server's UAX #29 count used to
    /// disagree (spec/11 §8). Each case also lives in the shared vector file, so both sides are
    /// pinned to the same number.
    #[test]
    fn script_edge_cases_table() {
        let cases: &[(&str, i32)] = &[
            // Ideographic number zero is an ideograph, not a splitter.
            ("一九〇〇年", 5),
            ("二〇二五年三月", 7),
            // Halfwidth Katakana is Katakana: one run, not one word per character.
            ("ｱｲｳｴｵ", 1),
            ("ﾃﾞｰﾀ", 1),
            ("iPhoneｱﾌﾟﾘ", 2),
            // ExtendNumLet joins letter runs.
            ("snake_case_name", 1),
            ("port_name_2", 1),
            // Latin words must not swallow an adjacent CJK paragraph character.
            ("你好world", 3),
            ("第12章", 3),
        ];
        for (text, words) in cases {
            assert_eq!(count_words(text), *words, "text {text:?}");
        }
    }

    #[test]
    fn ideographic_zero_is_han() {
        assert!(is_han('〇'));
        assert!(is_han('一'));
    }

    #[test]
    fn halfwidth_katakana_is_katakana() {
        for ch in ['ｱ', 'ﾟ', 'ｰ'] {
            assert!(is_katakana(ch), "{ch} should count as Katakana");
            assert!(!is_han(ch));
        }
    }

    #[test]
    fn script_ranges_are_disjoint() {
        for code in [0x4e00_u32, 0x3042, 0x30ab, 0x30fc] {
            let ch = char::from_u32(code).expect("char");
            let matches = [is_han(ch), is_hiragana(ch), is_katakana(ch)];
            assert!(
                matches.iter().filter(|hit| **hit).count() <= 1,
                "U+{code:04X} matched more than one script"
            );
        }
    }
}
