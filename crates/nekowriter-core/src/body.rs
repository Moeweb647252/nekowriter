//! Changes to a chapter's body: `append_content` and `replace_content`
//! (`docs/product-spec.md` §8.5).
//!
//! The body is plain text with `\n` line endings. Edits are normalized the same way and then
//! matched exactly: no Unicode normalization, trimming, or case or whitespace folding.

use serde::{Deserialize, Serialize};

use crate::field::normalize_line_endings;

/// The longest body a chapter may have, in characters.
pub const BODY_MAX_CHARS: u32 = 200_000;

/// A change to the body, as the model asked for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BodyEdit {
    /// Adds `content` to the end of the body, verbatim.
    Append { content: String },
    /// Replaces the only occurrence of `old_str` with `new_str`.
    Replace { old_str: String, new_str: String },
}

/// Why an edit left the body unchanged.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum EditProblem {
    /// There is nothing to append.
    EmptyContent,
    /// There is nothing to look for.
    EmptyOldStr,
    /// `old_str` does not occur in the body.
    NoMatch,
    /// `old_str` occurs several times, counting overlapping occurrences.
    AmbiguousMatch { matches: u32 },
    /// The body would exceed [`BODY_MAX_CHARS`].
    BodyTooLong { max_chars: u32 },
}

/// What an edit does to the body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditResult {
    /// The body the edit produces.
    Changed(String),
    /// `old_str` equals `new_str`: nothing changes and no revision is made.
    NoChange,
}

/// Applies `edit` to `body`, or says why it cannot be applied.
pub fn apply(body: &str, edit: &BodyEdit) -> Result<EditResult, EditProblem> {
    let changed = match edit {
        BodyEdit::Append { content } => {
            let content = normalize_line_endings(content);
            if content.is_empty() {
                return Err(EditProblem::EmptyContent);
            }
            format!("{body}{content}")
        }
        BodyEdit::Replace { old_str, new_str } => {
            let old_str = normalize_line_endings(old_str);
            let new_str = normalize_line_endings(new_str);
            if old_str.is_empty() {
                return Err(EditProblem::EmptyOldStr);
            }
            if old_str == new_str {
                return Ok(EditResult::NoChange);
            }
            let start = match occurrences(body, &old_str).as_slice() {
                [] => return Err(EditProblem::NoMatch),
                [start] => *start,
                more => {
                    return Err(EditProblem::AmbiguousMatch {
                        matches: u32::try_from(more.len()).unwrap_or(u32::MAX),
                    });
                }
            };
            let mut changed = String::with_capacity(body.len() - old_str.len() + new_str.len());
            changed.push_str(&body[..start]);
            changed.push_str(&new_str);
            changed.push_str(&body[start + old_str.len()..]);
            changed
        }
    };
    if changed.chars().count() > BODY_MAX_CHARS as usize {
        return Err(EditProblem::BodyTooLong {
            max_chars: BODY_MAX_CHARS,
        });
    }
    Ok(EditResult::Changed(changed))
}

/// The byte offsets where `needle` starts in `haystack`, overlapping occurrences included.
fn occurrences(haystack: &str, needle: &str) -> Vec<usize> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(offset) = haystack[from..].find(needle) {
        let start = from + offset;
        found.push(start);
        // Continue one character later, so overlapping occurrences count too.
        from = start + haystack[start..].chars().next().map_or(1, char::len_utf8);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replace(old_str: &str, new_str: &str) -> BodyEdit {
        BodyEdit::Replace {
            old_str: old_str.to_owned(),
            new_str: new_str.to_owned(),
        }
    }

    #[test]
    fn content_is_appended_verbatim_with_normalized_line_endings() {
        let edit = BodyEdit::Append {
            content: "Night.\r\nRain.\r".to_owned(),
        };
        assert_eq!(
            apply("Dusk. ", &edit),
            Ok(EditResult::Changed("Dusk. Night.\nRain.\n".to_owned()))
        );
        assert_eq!(
            apply(
                "x",
                &BodyEdit::Append {
                    content: String::new()
                }
            ),
            Err(EditProblem::EmptyContent)
        );
    }

    #[test]
    fn a_replacement_needs_exactly_one_match_counting_overlaps() {
        assert_eq!(
            apply("The lamp was dark.", &replace("dark", "lit")),
            Ok(EditResult::Changed("The lamp was lit.".to_owned()))
        );
        assert_eq!(
            apply("aaa", &replace("aa", "b")),
            Err(EditProblem::AmbiguousMatch { matches: 2 })
        );
        assert_eq!(
            apply("a b a b a", &replace("a", "c")),
            Err(EditProblem::AmbiguousMatch { matches: 3 })
        );
        assert_eq!(
            apply("Dark.", &replace("dark", "lit")),
            Err(EditProblem::NoMatch)
        );
        assert_eq!(apply("x", &replace("", "y")), Err(EditProblem::EmptyOldStr));
    }

    #[test]
    fn replacing_with_the_same_text_changes_nothing() {
        assert_eq!(
            apply("Dark.", &replace("Dark", "Dark")),
            Ok(EditResult::NoChange)
        );
    }

    #[test]
    fn matching_respects_character_boundaries_and_deletion_is_allowed() {
        // Two ideographs ("night rain"); deleting the first leaves the second.
        assert_eq!(
            apply("\u{591c}\u{96e8}", &replace("\u{591c}", "")),
            Ok(EditResult::Changed("\u{96e8}".to_owned()))
        );
        assert_eq!(
            apply("line\nnext", &replace("line\r\nnext", "one")),
            Ok(EditResult::Changed("one".to_owned()))
        );
    }
}
