//! The knowledge base (`docs/product-spec.md` §12.2): entries the user keeps for the
//! agents, sorted into spaces. Spaces belong to no project.

use sea_orm::FromJsonQueryResult;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    embedding::EmbeddingFailure,
    field::{Field, FieldError, FieldProblem, multi_line_text, single_line, single_line_text},
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

/// The non-whitespace characters a chunk aims for.
const CHUNK_TARGET: usize = 600;
/// The characters the previous chunk lends the next one.
const CHUNK_OVERLAP: usize = 80;
/// The characters of a chunk without its overlap and paragraph break, so that a whole
/// chunk never exceeds 1000 characters.
const CHUNK_BODY: usize = 1_000 - CHUNK_OVERLAP - 2;

/// Splits an entry's content into the chunks that are embedded (§12.2): whole paragraphs
/// while they fit about [`CHUNK_TARGET`] non-whitespace characters, long paragraphs on
/// character boundaries, and each chunk after the first starting with the last
/// [`CHUNK_OVERLAP`] characters of the one before.
#[must_use]
pub fn chunks(content: &str) -> Vec<String> {
    let weight = |text: &str| {
        text.chars()
            .filter(|character| !character.is_whitespace())
            .count()
    };
    // Pieces of at most CHUNK_BODY characters, and whether each continues the previous
    // piece's paragraph.
    let mut pieces: Vec<(String, bool)> = Vec::new();
    for paragraph in content
        .split("\n\n")
        .map(str::trim)
        .filter(|paragraph| !paragraph.is_empty())
    {
        let characters: Vec<char> = paragraph.chars().collect();
        let mut start = 0;
        while start < characters.len() {
            let mut end = start;
            let mut filled = 0;
            while end < characters.len() && end - start < CHUNK_BODY && filled < CHUNK_TARGET {
                if !characters[end].is_whitespace() {
                    filled += 1;
                }
                end += 1;
            }
            pieces.push((characters[start..end].iter().collect(), start > 0));
            start = end;
        }
    }
    let mut bodies: Vec<(String, bool)> = Vec::new();
    for (piece, continues) in pieces {
        if let Some((body, _)) = bodies.last_mut() {
            let fits = weight(body) + weight(&piece) <= CHUNK_TARGET
                && body.chars().count() + 2 + piece.chars().count() <= CHUNK_BODY;
            if !continues && fits {
                body.push_str("\n\n");
                body.push_str(&piece);
                continue;
            }
        }
        bodies.push((piece, continues));
    }
    let mut chunks = Vec::with_capacity(bodies.len());
    let mut previous: Option<&str> = None;
    for (body, continues) in &bodies {
        chunks.push(match previous {
            None => body.clone(),
            Some(previous) => {
                let skip = previous.chars().count().saturating_sub(CHUNK_OVERLAP);
                let overlap: String = previous.chars().skip(skip).collect();
                let separator = if *continues { "" } else { "\n\n" };
                format!("{overlap}{separator}{body}")
            }
        });
        previous = Some(body);
    }
    chunks
}

multi_line_text!(
    /// What a semantic search looks for.
    SearchQuery,
    Field::SearchQuery,
    2_000
);

/// How many results a search returns: 5 unless asked otherwise, at most 10 (§12.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct TopK(u32);

impl TopK {
    pub const DEFAULT: Self = Self(5);
    pub const MAX: u32 = 10;

    pub fn new(value: u32) -> Result<Self, FieldError> {
        if (1..=Self::MAX).contains(&value) {
            Ok(Self(value))
        } else {
            Err(FieldError {
                field: Field::TopK,
                problem: FieldProblem::OutOfRange {
                    min: 1,
                    max: Self::MAX,
                },
            })
        }
    }

    #[must_use]
    pub fn get(self) -> usize {
        self.0 as usize
    }
}

/// Why semantic search cannot run now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum SearchUnavailable {
    /// No embedding model is set.
    NotConfigured,
    /// A new embedding model is still indexing the entries; `remaining` tasks are open.
    Indexing { remaining: u32 },
}

/// How far an entry's index is for the current embedding model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum IndexState {
    /// No embedding model is set, so nothing is indexed.
    NotConfigured,
    Pending,
    Running,
    Ready,
    /// Indexing failed; the user can index the entry again.
    Failed {
        failure: EmbeddingFailure,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_paragraphs_share_a_chunk_and_long_ones_are_split_with_overlap() {
        assert_eq!(chunks("One.\n\n\n\nTwo."), ["One.\n\nTwo."]);
        assert_eq!(chunks("  \n\n "), Vec::<String>::new());

        let long = "a".repeat(1_500);
        let split = chunks(&long);
        assert_eq!(split.len(), 3);
        assert_eq!(split[0].chars().count(), 600);
        // The second starts with the last 80 characters of the first, then continues.
        assert_eq!(split[1].chars().count(), 80 + 600);
        assert_eq!(split[2].chars().count(), 80 + 300);

        // Spaces do not count toward the target, but the hard limit holds.
        let spaced = "a ".repeat(1_000);
        assert!(
            chunks(&spaced)
                .iter()
                .all(|chunk| chunk.chars().count() <= 1_000)
        );
    }

    #[test]
    fn keywords_are_trimmed_and_unique() {
        let keywords =
            Keywords::parse(&[" lamp ".to_owned(), String::new(), "lamp".to_owned()]).unwrap();
        assert_eq!(keywords.as_slice(), ["lamp"]);
        let many: Vec<String> = (0..21).map(|index| format!("k{index}")).collect();
        assert!(Keywords::parse(&many).is_err());
    }
}
