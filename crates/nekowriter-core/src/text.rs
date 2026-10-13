//! Measures of text (`docs/product-spec.md` §15).

use unicode_segmentation::UnicodeSegmentation;

/// The number of words by UAX #29 word segmentation: each CJK ideograph is one word,
/// other scripts count their words, and punctuation and whitespace count nothing.
#[must_use]
pub fn word_count(text: &str) -> u32 {
    u32::try_from(text.unicode_words().count()).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ideographs_count_one_each_and_punctuation_counts_nothing() {
        // "Night rain." in Chinese: two ideographs and a full stop.
        assert_eq!(word_count("\u{591c}\u{96e8}\u{3002}"), 2);
        assert_eq!(word_count("Rain falls, softly."), 3);
        assert_eq!(word_count("Rain \u{591c}\u{96e8} -- 42"), 4);
        assert_eq!(word_count(" \n\u{3001}\u{201c}\u{201d} "), 0);
        assert_eq!(word_count(""), 0);
    }
}
