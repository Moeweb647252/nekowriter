//! The knowledge base (`docs/product-spec.md` §12.2): entries the user keeps for the
//! agents, sorted into spaces. Spaces belong to no project.

use sea_orm::FromJsonQueryResult;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::field::{
    Field, FieldError, FieldProblem, multi_line_text, single_line, single_line_text,
};

single_line_text!(
    /// The name of a knowledge space.
    SpaceName,
    Field::SpaceName,
    100
);

single_line_text!(
    /// The title of a knowledge entry.
    EntryTitle,
    Field::EntryTitle,
    200
);

multi_line_text!(
    /// The text of a knowledge entry.
    EntryContent,
    Field::EntryContent,
    50_000
);

/// The keywords of a knowledge entry, without duplicates, as a stored column.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, FromJsonQueryResult)]
#[ts(export)]
pub struct Keywords(Vec<String>);

impl Keywords {
    pub const MAX_KEYWORDS: usize = 20;
    pub const MAX_CHARS: u32 = 50;

    /// Trims every keyword and drops empty ones and repeats.
    pub fn parse(values: &[String]) -> Result<Self, FieldError> {
        let mut keywords: Vec<String> = Vec::new();
        for value in values {
            let keyword = match single_line(Field::Keyword, value, Self::MAX_CHARS) {
                Ok(keyword) => keyword,
                Err(FieldError {
                    problem: FieldProblem::Empty,
                    ..
                }) => continue,
                Err(error) => return Err(error),
            };
            if !keywords.contains(&keyword) {
                keywords.push(keyword);
            }
        }
        if keywords.len() > Self::MAX_KEYWORDS {
            return Err(FieldError {
                field: Field::Keyword,
                problem: FieldProblem::OutOfRange {
                    min: 0,
                    max: u32::try_from(Self::MAX_KEYWORDS).unwrap_or(u32::MAX),
                },
            });
        }
        Ok(Self(keywords))
    }

    #[must_use]
    pub fn as_slice(&self) -> &[String] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_are_trimmed_and_unique() {
        let keywords =
            Keywords::parse(&[" lamp ".to_owned(), String::new(), "lamp".to_owned()]).unwrap();
        assert_eq!(keywords.as_slice(), ["lamp"]);
        let many: Vec<String> = (0..21).map(|index| format!("k{index}")).collect();
        assert!(Keywords::parse(&many).is_err());
    }
}
