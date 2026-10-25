//! Chunking (spec/07 §1.1, §2): Markdown knowledge entries and official chapter passages.
//!
//! Knowledge chunks record their Markdown heading path and split on paragraph boundaries with a
//! target of [`CHUNK_TARGET_CHARS`]; a paragraph over [`LONG_PARAGRAPH_CHARS`] is split at sentence
//! ends (or hard-split at [`HARD_SPLIT_CHARS`]). There is no overlap. The embedding input is
//! `heading + "\n" + text` (the title stands in for an empty heading), and the content hash is
//! `blake3(heading + text)` — computed by the callers, see [`content_hash`].
//!
//! Passages use the same accumulation over consecutive chapter paragraphs and record the covered
//! pids, so `search_chapters` can show `p N–M` (spec/05 §3).

/// The target character count of one chunk (spec/07 §1.1).
pub const CHUNK_TARGET_CHARS: usize = 700;

/// A paragraph longer than this is split at sentence ends (spec/07 §1.1).
pub const LONG_PARAGRAPH_CHARS: usize = 1_000;

/// The hard split length when no sentence end is available (spec/07 §1.1).
pub const HARD_SPLIT_CHARS: usize = 1_000;

/// Sentence terminators the splitter recognises (spec/07 §1.1).
const SENTENCE_ENDS: [char; 8] = ['。', '！', '？', '!', '?', '.', ';', '；'];

/// Closing quotes that may follow a sentence terminator (spec/07 §1.1: "followed by an optional
/// closing quote").
const CLOSING_QUOTES: [char; 6] = ['”', '"', '』', '」', '’', '）'];

/// One knowledge chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// The heading path, e.g. `宋代服饰 > 男子常服`; empty when the content has no heading.
    pub heading: String,
    /// The chunk text (paragraphs joined by a newline).
    pub text: String,
}

/// The hash of a chunk's identity content (spec/07 §1.1 item 4). The title is not part of it:
/// the embedding input substitutes the title for an empty heading, but a rename must not
/// re-embed a whole entry.
pub fn content_hash(heading: &str, text: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(heading.as_bytes());
    hasher.update(text.as_bytes());
    hasher.finalize().to_hex().to_string()
}

/// Splits Markdown into chunks (spec/07 §1.1).
pub fn chunk_markdown(markdown: &str) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let mut stack: Vec<(usize, String)> = Vec::new();
    let mut heading = String::new();
    let mut paragraphs: Vec<String> = Vec::new();

    let flush = |heading: &str, paragraphs: &mut Vec<String>, chunks: &mut Vec<Chunk>| {
        for text in accumulate(paragraphs.drain(..)) {
            chunks.push(Chunk {
                heading: heading.to_owned(),
                text,
            });
        }
    };

    let mut paragraph_lines: Vec<String> = Vec::new();
    let flush_paragraph = |lines: &mut Vec<String>, paragraphs: &mut Vec<String>| {
        if !lines.is_empty() {
            paragraphs.push(lines.join("\n"));
            lines.clear();
        }
    };

    for raw in markdown.lines() {
        let line = raw.trim_end();
        if let Some((level, text)) = parse_heading(line) {
            flush_paragraph(&mut paragraph_lines, &mut paragraphs);
            flush(&heading, &mut paragraphs, &mut chunks);
            while stack.last().is_some_and(|(depth, _)| *depth >= level) {
                stack.pop();
            }
            stack.push((level, text.to_owned()));
            heading = stack
                .iter()
                .map(|(_, text)| text.as_str())
                .collect::<Vec<_>>()
                .join(" > ");
            continue;
        }
        if line.trim().is_empty() {
            flush_paragraph(&mut paragraph_lines, &mut paragraphs);
            continue;
        }
        paragraph_lines.push(line.to_owned());
    }
    flush_paragraph(&mut paragraph_lines, &mut paragraphs);
    flush(&heading, &mut paragraphs, &mut chunks);
    chunks
}

/// One chapter passage (spec/07 §2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassageChunk {
    pub text: String,
    /// The pids covered by this chunk (a split long paragraph appears in several chunks).
    pub pids: Vec<String>,
}

/// Groups consecutive chapter paragraphs into passage chunks (spec/07 §2).
pub fn chunk_passages(paragraphs: &[(String, String)]) -> Vec<PassageChunk> {
    let mut pieces: Vec<(String, String)> = Vec::new();
    for (pid, text) in paragraphs {
        for piece in split_long_paragraph(text) {
            pieces.push((pid.clone(), piece));
        }
    }
    let mut chunks: Vec<PassageChunk> = Vec::new();
    let mut current = String::new();
    let mut pids: Vec<String> = Vec::new();
    for (pid, text) in pieces {
        let added = if current.is_empty() {
            text.chars().count()
        } else {
            current.chars().count() + 1 + text.chars().count()
        };
        if !current.is_empty() && added > CHUNK_TARGET_CHARS {
            chunks.push(PassageChunk {
                text: std::mem::take(&mut current),
                pids: std::mem::take(&mut pids),
            });
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(&text);
        if pids.last() != Some(&pid) {
            pids.push(pid);
        }
    }
    if !current.is_empty() {
        chunks.push(PassageChunk {
            text: current,
            pids,
        });
    }
    chunks
}

/// Builds the embedding input of a knowledge chunk: `heading + "\n" + text`, with the entry title
/// standing in for an empty heading (spec/07 §1.1 item 3).
pub fn knowledge_embedding_input(title: &str, heading: &str, text: &str) -> String {
    let context = if heading.is_empty() { title } else { heading };
    format!("{context}\n{text}")
}

// ───────────────────────────── helpers ─────────────────────────────

/// Parses an ATX Markdown heading (`#`…`######` plus whitespace).
fn parse_heading(line: &str) -> Option<(usize, &str)> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &line[hashes..];
    if !rest.is_empty() && !rest.starts_with(' ') && !rest.starts_with('\t') {
        return None;
    }
    Some((hashes, rest.trim()))
}

/// Accumulates paragraphs into chunks of at most the target length (spec/07 §1.1).
fn accumulate(paragraphs: impl Iterator<Item = String>) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    for paragraph in paragraphs {
        for piece in split_long_paragraph(&paragraph) {
            let added = if current.is_empty() {
                piece.chars().count()
            } else {
                current.chars().count() + 1 + piece.chars().count()
            };
            if !current.is_empty() && added > CHUNK_TARGET_CHARS {
                chunks.push(std::mem::take(&mut current));
            }
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(&piece);
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// Splits a paragraph that exceeds [`LONG_PARAGRAPH_CHARS`] at sentence ends, falling back to a
/// hard cut at [`HARD_SPLIT_CHARS`] (spec/07 §1.1).
fn split_long_paragraph(paragraph: &str) -> Vec<String> {
    if paragraph.chars().count() <= LONG_PARAGRAPH_CHARS {
        return vec![paragraph.to_owned()];
    }
    let mut pieces = Vec::new();
    let mut remaining = paragraph;
    while remaining.chars().count() > LONG_PARAGRAPH_CHARS {
        let cut = sentence_cut(remaining, HARD_SPLIT_CHARS);
        let (head, tail) = remaining.split_at(cut);
        pieces.push(head.to_owned());
        remaining = tail.trim_start_matches('\n');
    }
    if !remaining.is_empty() {
        pieces.push(remaining.to_owned());
    }
    pieces
}

/// The byte index of the best sentence boundary within the first `limit` characters, or the byte
/// index of the `limit`-th character when there is none. Always > 0 for a non-empty input, so the
/// splitter makes progress.
fn sentence_cut(text: &str, limit: usize) -> usize {
    let limit_index = text
        .char_indices()
        .nth(limit)
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    let window = &text[..limit_index];
    let mut best: Option<usize> = None;
    let mut chars = window.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if !SENTENCE_ENDS.contains(&ch) {
            continue;
        }
        // Consume an optional closing quote right after the terminator.
        let mut end = index + ch.len_utf8();
        if let Some((next_index, next)) = chars.peek()
            && CLOSING_QUOTES.contains(next)
        {
            end = next_index + next.len_utf8();
        }
        if end > 0 {
            best = Some(end);
        }
    }
    best.unwrap_or(limit_index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_build_a_path_per_chunk() {
        let chunks = chunk_markdown(
            "# 宋代服饰\n\n宋代男子常穿圆领袍。\n\n## 男子常服\n\n颜色以青、白为主。",
        );
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].heading, "宋代服饰");
        assert_eq!(chunks[0].text, "宋代男子常穿圆领袍。");
        assert_eq!(chunks[1].heading, "宋代服饰 > 男子常服");
        assert_eq!(chunks[1].text, "颜色以青、白为主。");
    }

    #[test]
    fn paragraphs_accumulate_up_to_the_target() {
        let paragraph = "字".repeat(300);
        let markdown = format!("# H\n\n{paragraph}\n\n{paragraph}\n\n{paragraph}");
        let chunks = chunk_markdown(&markdown);
        assert_eq!(
            chunks.len(),
            2,
            "two 300-char paragraphs fit, the third starts a chunk"
        );
        let first = chunks[0].text.chars().count();
        assert!(first <= CHUNK_TARGET_CHARS, "chunk is {first} chars");
        assert_eq!(chunks[0].text.lines().count(), 2);
    }

    #[test]
    fn a_single_long_paragraph_splits_at_sentence_ends() {
        // 600 CJK chars then a terminator, then 600 more: the first piece is cut at the
        // sentence end inside the first 1,000 characters.
        let first = format!("{}{}", "字".repeat(600), "。");
        let second = "字".repeat(600);
        let chunks = chunk_markdown(&format!("{first}{second}"));
        assert_eq!(chunks.len(), 2, "{chunks:?}");
        assert!(chunks[0].text.ends_with('。'));
        assert_eq!(chunks[0].text.chars().count(), 601);
        assert_eq!(chunks[1].text.chars().count(), 600);
    }

    #[test]
    fn a_long_paragraph_without_sentence_ends_hard_splits() {
        let chunks = chunk_markdown(&"字".repeat(2_500));
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].text.chars().count(), HARD_SPLIT_CHARS);
        assert_eq!(chunks[1].text.chars().count(), HARD_SPLIT_CHARS);
        assert_eq!(chunks[2].text.chars().count(), 500);
    }

    #[test]
    fn an_english_sentence_end_is_a_split_point() {
        let text = format!(
            "{}. {}",
            "word ".repeat(150).trim_end(),
            "tail ".repeat(150).trim_end()
        );
        let chunks = chunk_markdown(&text);
        assert!(chunks.len() >= 2);
        assert!(chunks[0].text.ends_with('.'));
    }

    #[test]
    fn content_hash_covers_heading_and_text() {
        let a = content_hash("H", "text");
        let b = content_hash("H", "text");
        let c = content_hash("H2", "text");
        let d = content_hash("H", "text2");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
    }

    #[test]
    fn knowledge_embedding_input_uses_the_title_without_a_heading() {
        assert_eq!(
            knowledge_embedding_input("Title", "", "body"),
            "Title\nbody"
        );
        assert_eq!(
            knowledge_embedding_input("Title", "H > H2", "body"),
            "H > H2\nbody"
        );
    }

    #[test]
    fn passages_cover_consecutive_paragraphs_with_their_pids() {
        let paragraphs = vec![
            ("p1".to_owned(), "one".to_owned()),
            ("p2".to_owned(), "two".to_owned()),
        ];
        let chunks = chunk_passages(&paragraphs);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "one\ntwo");
        assert_eq!(chunks[0].pids, vec!["p1", "p2"]);
    }

    #[test]
    fn passages_split_on_the_char_budget() {
        let paragraphs = vec![
            ("p1".to_owned(), "a".repeat(400)),
            ("p2".to_owned(), "b".repeat(400)),
        ];
        let chunks = chunk_passages(&paragraphs);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].pids, vec!["p1"]);
        assert_eq!(chunks[1].pids, vec!["p2"]);
    }

    #[test]
    fn a_split_passage_paragraph_repeats_its_pid_only_where_needed() {
        let paragraphs = vec![("p1".to_owned(), "字".repeat(1_200))];
        let chunks = chunk_passages(&paragraphs);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].pids, vec!["p1"]);
        assert_eq!(chunks[1].pids, vec!["p1"]);
    }

    #[test]
    fn empty_markdown_has_no_chunks() {
        assert!(chunk_markdown("").is_empty());
        assert!(chunk_markdown("\n\n").is_empty());
    }
}
