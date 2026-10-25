//! jieba-segmented full-text search helpers (spec/07 §5, D-003).
//!
//! The same function runs at index time (title and body) and at query time: FTS5 stores
//! space-joined `jieba.cut_for_search` tokens over `unicode61`, and a query becomes an OR of
//! quoted tokens. Exact names stay findable because the indexer and the query use the same
//! segmentation, and unsegmented CJK would be a single token that FTS5 cannot split.

use std::sync::LazyLock;

use jieba_rs::Jieba;

/// The dictionary is initialized once per process (spec/07 §5).
static JIEBA: LazyLock<Jieba> = LazyLock::new(Jieba::new);

/// The maximum number of distinct query tokens (spec/07 §5).
pub const MAX_QUERY_TOKENS: usize = 16;

/// Segments text into lowercase tokens that contain a letter or digit, joined with spaces.
///
/// Used for `fts_book.title`/`body` and `fts_knowledge.title`/`body` at index time
/// (spec/07 §2) and for the query string (spec/07 §5).
pub fn segment(text: &str) -> String {
    JIEBA
        .cut_for_search(text, true)
        .into_iter()
        .filter_map(|token| {
            let lower = token.word.to_lowercase();
            // Punctuation and whitespace tokens carry no signal (spec/07 §5).
            if lower.chars().any(|ch| ch.is_alphanumeric()) {
                Some(lower)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The distinct query tokens, in order, capped at [`MAX_QUERY_TOKENS`] (spec/07 §5).
pub fn query_tokens(query: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    for token in segment(query).split_whitespace() {
        if tokens.iter().any(|existing| existing == token) {
            continue;
        }
        tokens.push(token.to_owned());
        if tokens.len() == MAX_QUERY_TOKENS {
            break;
        }
    }
    tokens
}

/// The FTS5 query string: each token double-quoted (inner quotes doubled), joined with ` OR `
/// (spec/07 §5). `None` when the query has no usable token.
pub fn query_string(query: &str) -> Option<String> {
    let tokens = query_tokens(query);
    if tokens.is_empty() {
        return None;
    }
    Some(
        tokens
            .iter()
            .map(|token| format!("\"{}\"", token.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" OR "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segmentation_joins_tokens_with_spaces() {
        let segmented = segment("林凡在剑冢第三层修炼");
        assert!(
            segmented.contains(' '),
            "CJK text must be segmented: {segmented}"
        );
        assert!(segmented.contains("林凡"), "{segmented}");
        assert!(segmented.contains("剑冢"), "{segmented}");
    }

    #[test]
    fn segmentation_drops_punctuation_and_lowercases() {
        let segmented = segment("Hello, World!");
        assert_eq!(segmented, "hello world");
        let segmented = segment("……（！）");
        assert!(segmented.is_empty(), "{segmented}");
    }

    #[test]
    fn query_string_quotes_and_ors_the_unique_tokens() {
        let query = query_string("剑冢 剑冢 禁制").expect("tokens");
        assert_eq!(query, "\"剑冢\" OR \"禁制\"");
    }

    #[test]
    fn query_string_escapes_inner_quotes() {
        // A token never normally contains a quote, but the escaping keeps the FTS5 syntax valid.
        let value = format!("\"{}\"", "abc").replace('"', "\"\"");
        assert_eq!(value, "\"\"abc\"\"");
    }

    #[test]
    fn a_query_without_tokens_has_no_query_string() {
        assert!(query_string("……？!").is_none());
        assert!(query_string("").is_none());
    }

    #[test]
    fn query_tokens_are_deduplicated_and_capped() {
        let mut words = Vec::new();
        for index in 0..40 {
            words.push(format!("word{index}"));
        }
        let tokens = query_tokens(&words.join(" "));
        assert_eq!(tokens.len(), MAX_QUERY_TOKENS);
        assert_eq!(tokens[0], "word0");
        let tokens = query_tokens("林凡 林凡 林凡");
        assert_eq!(tokens, vec!["林凡"]);
    }
}
