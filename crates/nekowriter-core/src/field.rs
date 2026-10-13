//! Validation of user-editable fields.
//!
//! A rejected value is reported as data: which field, and what is wrong with it. The web UI
//! turns that into a sentence in the user's language.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

/// A user-editable field that validation can reject.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Field {
    ProjectName,
    NovelTitle,
    NovelBrief,
    ReviewFeedback,
    ApiBaseUrl,
    ApiKey,
    ModelId,
    ContextWindow,
    MaxOutputTokens,
}

/// What is wrong with a rejected field value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, Error)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum FieldProblem {
    #[error("is empty")]
    Empty,
    #[error("is longer than {max_chars} characters")]
    TooLong { max_chars: u32 },
    #[error("contains a line break or another control character")]
    ControlCharacter,
    #[error("is not an http or https URL without query or fragment")]
    InvalidUrl,
    #[error("is not between {min} and {max}")]
    OutOfRange { min: u32, max: u32 },
}

/// A field value that failed validation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, Error)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
#[error("{field:?} {problem}")]
pub struct FieldError {
    pub field: Field,
    pub problem: FieldProblem,
}

/// Validates a single-line text value: surrounding whitespace is trimmed, and the result
/// must be non-empty, free of control characters and at most `max_chars` characters long
/// (counted as Unicode scalar values).
pub fn single_line(field: Field, value: &str, max_chars: u32) -> Result<String, FieldError> {
    let reject = |problem| Err(FieldError { field, problem });
    let value = value.trim();
    if value.is_empty() {
        return reject(FieldProblem::Empty);
    }
    if value.chars().any(char::is_control) {
        return reject(FieldProblem::ControlCharacter);
    }
    if value.chars().count() > max_chars as usize {
        return reject(FieldProblem::TooLong { max_chars });
    }
    Ok(value.to_owned())
}

/// Validates a multi-line text value: line endings become `\n`, surrounding whitespace is
/// trimmed, and the result must be non-empty, free of control characters other than line
/// breaks and tabs, and at most `max_chars` characters long.
pub fn multi_line(field: Field, value: &str, max_chars: u32) -> Result<String, FieldError> {
    let reject = |problem| Err(FieldError { field, problem });
    let value = normalize_line_endings(value);
    let value = value.trim();
    if value.is_empty() {
        return reject(FieldProblem::Empty);
    }
    if value
        .chars()
        .any(|character| character.is_control() && character != '\n' && character != '\t')
    {
        return reject(FieldProblem::ControlCharacter);
    }
    if value.chars().count() > max_chars as usize {
        return reject(FieldProblem::TooLong { max_chars });
    }
    Ok(value.to_owned())
}

/// Replaces `\r\n` and lone `\r` with `\n`.
#[must_use]
pub fn normalize_line_endings(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\r', "\n")
}

/// Defines a validated single-line text type: a newtype over `String` whose values have
/// passed [`single_line`]. It is stored as a text column and sent as a JSON string, and it
/// re-validates when deserialized.
macro_rules! single_line_text {
    ($(#[$meta:meta])* $name:ident, $field:expr, $max_chars:literal) => {
        $(#[$meta])*
        #[derive(
            Clone, Debug, PartialEq, Eq, serde::Serialize, ts_rs::TS, sea_orm::DeriveValueType,
        )]
        #[ts(export)]
        pub struct $name(String);

        impl $name {
            pub const MAX_CHARS: u32 = $max_chars;

            pub fn parse(value: &str) -> Result<Self, $crate::field::FieldError> {
                $crate::field::single_line($field, value, Self::MAX_CHARS).map(Self)
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let value = String::deserialize(deserializer)?;
                Self::parse(&value).map_err(serde::de::Error::custom)
            }
        }
    };
}

pub(crate) use single_line_text;

/// Defines a validated multi-line text type: a newtype over `String` whose values have
/// passed [`multi_line`].
macro_rules! multi_line_text {
    ($(#[$meta:meta])* $name:ident, $field:expr, $max_chars:literal) => {
        $(#[$meta])*
        #[derive(
            Clone, Debug, PartialEq, Eq, serde::Serialize, ts_rs::TS, sea_orm::DeriveValueType,
        )]
        #[ts(export)]
        pub struct $name(String);

        impl $name {
            pub const MAX_CHARS: u32 = $max_chars;

            pub fn parse(value: &str) -> Result<Self, $crate::field::FieldError> {
                $crate::field::multi_line($field, value, Self::MAX_CHARS).map(Self)
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let value = String::deserialize(deserializer)?;
                Self::parse(&value).map_err(serde::de::Error::custom)
            }
        }
    };
}

pub(crate) use multi_line_text;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_line_trims_and_accepts_a_valid_value() {
        assert_eq!(
            single_line(Field::ProjectName, "  Rain  ", 10).unwrap(),
            "Rain"
        );
    }

    #[test]
    fn single_line_rejects_empty_and_whitespace_only_values() {
        for value in ["", " \t "] {
            assert_eq!(
                single_line(Field::ProjectName, value, 10)
                    .unwrap_err()
                    .problem,
                FieldProblem::Empty
            );
        }
    }

    #[test]
    fn single_line_rejects_control_characters() {
        assert_eq!(
            single_line(Field::NovelTitle, "Rain\nNight", 20)
                .unwrap_err()
                .problem,
            FieldProblem::ControlCharacter
        );
    }

    #[test]
    fn multi_line_keeps_inner_line_breaks_and_normalizes_them() {
        assert_eq!(
            multi_line(Field::NovelBrief, "  first\r\nsecond\rthird\n\n", 100).unwrap(),
            "first\nsecond\nthird"
        );
        assert_eq!(
            multi_line(Field::NovelBrief, "bell\u{7}", 100)
                .unwrap_err()
                .problem,
            FieldProblem::ControlCharacter
        );
    }

    #[test]
    fn single_line_counts_characters_not_bytes() {
        // Three ideographs are nine UTF-8 bytes but three characters.
        let three_ideographs = "\u{96e8}\u{591c}\u{4fe1}";
        assert!(single_line(Field::NovelTitle, three_ideographs, 3).is_ok());
        assert_eq!(
            single_line(Field::NovelTitle, three_ideographs, 2)
                .unwrap_err()
                .problem,
            FieldProblem::TooLong { max_chars: 2 }
        );
    }
}
