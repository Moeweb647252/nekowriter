//! Segment normalization, paragraph splitting and CJK-aware joining (spec/03 §8).
//!
//! Agent text arrives as segments that may contain line breaks. Before it reaches a paragraph it
//! is normalized: `\r\n` becomes `\n`, runs of blank lines collapse into one break, and the
//! segment's edges are trimmed. A normalized segment splits on single line breaks into
//! paragraphs; empty paragraphs are dropped.
//!
//! Joining continues an open tail paragraph. A single space separates the two texts unless the
//! writing language is unspaced (`zh`, `ja`, `th`, `lo`, `km`, `my`) or either side already
//! carries whitespace at the seam.

use crate::db::json::Paragraph;

use super::content::{TextError, allocate_pid};
use super::language::is_unspaced;
use crate::api::dto::ContentOp;

/// Normalizes a raw segment: `\r\n`/`\r` → `\n`, runs of line breaks collapse into one break,
/// edges trimmed.
pub fn normalize_segment(segment: &str) -> String {
    let unified = segment.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(unified.len());
    let mut pending_breaks = 0usize;
    for ch in unified.chars() {
        if ch == '\n' {
            pending_breaks += 1;
            continue;
        }
        if pending_breaks > 0 {
            pending_breaks = 0;
            if !out.is_empty() {
                out.push('\n');
            }
        }
        out.push(ch);
    }
    out.trim().to_owned()
}

/// Splits a raw segment into trimmed, non-empty paragraph texts (one per line break).
pub fn split_paragraphs(segment: &str) -> Vec<String> {
    normalize_segment(segment)
        .split('\n')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Joins `addition` onto `existing` (spec/03 §8): one space unless the language is unspaced or
/// either side already has whitespace at the seam.
pub fn join_texts(existing: &str, addition: &str, unspaced: bool) -> String {
    if addition.is_empty() {
        return existing.to_owned();
    }
    if existing.is_empty() {
        return addition.to_owned();
    }
    let seam_has_space = existing
        .chars()
        .next_back()
        .is_some_and(char::is_whitespace)
        || addition.chars().next().is_some_and(char::is_whitespace);
    if unspaced || seam_has_space {
        format!("{existing}{addition}")
    } else {
        format!("{existing} {addition}")
    }
}

/// Appends a raw segment to the content, continuing an open tail paragraph (spec/03 §8).
///
/// The first paragraph of the segment either extends the open tail (joining with
/// [`join_texts`]) or starts a new paragraph after the current last one; every following
/// paragraph becomes a new paragraph. Returns the ops that describe the change, in application
/// order.
pub fn append_segment(
    content: &mut crate::db::json::ChapterContent,
    segment: &str,
    language: &str,
) -> Result<Vec<ContentOp>, TextError> {
    let parts = split_paragraphs(segment);
    if parts.is_empty() {
        return Ok(Vec::new());
    }
    let unspaced = is_unspaced(language);
    let mut ops = Vec::new();
    let mut next_part = 0usize;

    if content.tail_open {
        match content.paragraphs.last_mut() {
            Some(last) => {
                last.text = join_texts(&last.text, &parts[0], unspaced);
                ops.push(ContentOp::Update {
                    pid: last.pid.clone(),
                    text: last.text.clone(),
                });
                next_part = 1;
            }
            None => content.tail_open = false,
        }
    }

    let mut after = content
        .paragraphs
        .last()
        .map(|paragraph| paragraph.pid.clone());
    for part in &parts[next_part..] {
        let pid = allocate_pid(content);
        let op = ContentOp::Insert {
            after: after.clone(),
            pid: pid.clone(),
            text: part.clone(),
        };
        content.paragraphs.push(Paragraph {
            pid: pid.clone(),
            text: part.clone(),
        });
        after = Some(pid);
        ops.push(op);
    }
    Ok(ops)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::json::ChapterContent;

    #[test]
    fn normalization_table() {
        let cases: &[(&str, &str)] = &[
            ("a\r\nb", "a\nb"),
            ("  hello  ", "hello"),
            ("a\n\n\n\nb", "a\nb"),
            ("\n\nleading", "leading"),
            ("trailing\n\n", "trailing"),
            ("a\n\n\n", "a"),
            ("", ""),
            ("   \n\t\n ", ""),
            ("第一段\r\n第二段", "第一段\n第二段"),
        ];
        for (input, expected) in cases {
            assert_eq!(normalize_segment(input), *expected, "input {input:?}");
        }
    }

    #[test]
    fn paragraph_splitting_drops_empty_segments() {
        assert_eq!(
            split_paragraphs("one\n\ntwo\n \nthree"),
            vec!["one".to_owned(), "two".to_owned(), "three".to_owned()]
        );
        assert!(split_paragraphs("\n\n  \n").is_empty());
        assert_eq!(split_paragraphs("  单段  "), vec!["单段".to_owned()]);
    }

    #[test]
    fn joining_table() {
        let cases: &[(&str, &str, bool, &str)] = &[
            ("hello", "world", false, "hello world"),
            ("hello", "world", true, "helloworld"),
            ("hello ", "world", false, "hello world"),
            ("hello", " world", false, "hello world"),
            ("", "world", false, "world"),
            ("hello", "", false, "hello"),
            ("结尾。", "下一句。", true, "结尾。下一句。"),
            ("条", "款", true, "条款"),
        ];
        for (existing, addition, unspaced, expected) in cases {
            assert_eq!(
                join_texts(existing, addition, *unspaced),
                *expected,
                "joining {existing:?} + {addition:?} (unspaced={unspaced})"
            );
        }
    }

    fn content(paragraphs: &[(&str, &str)], next_pid: i32, tail_open: bool) -> ChapterContent {
        ChapterContent {
            paragraphs: paragraphs
                .iter()
                .map(|(pid, text)| Paragraph {
                    pid: (*pid).to_owned(),
                    text: (*text).to_owned(),
                })
                .collect(),
            next_pid,
            tail_open,
        }
    }

    #[test]
    fn append_closed_tail_adds_paragraphs() {
        let mut doc = content(&[("p1", "first")], 2, false);
        let ops = append_segment(&mut doc, "second\n\nthird", "en").expect("append");
        assert_eq!(
            ops,
            vec![
                ContentOp::Insert {
                    after: Some("p1".to_owned()),
                    pid: "p2".to_owned(),
                    text: "second".to_owned(),
                },
                ContentOp::Insert {
                    after: Some("p2".to_owned()),
                    pid: "p3".to_owned(),
                    text: "third".to_owned(),
                },
            ]
        );
        assert_eq!(doc.paragraphs.len(), 3);
        assert_eq!(doc.paragraphs[2].text, "third");
        assert!(!doc.tail_open);
    }

    #[test]
    fn append_continues_open_tail_without_space_for_cjk() {
        let mut doc = content(&[("p7", "他转过头")], 8, true);
        let ops = append_segment(&mut doc, "看见了剑光", "zh-CN").expect("append");
        assert_eq!(
            ops,
            vec![ContentOp::Update {
                pid: "p7".to_owned(),
                text: "他转过头看见了剑光".to_owned(),
            }]
        );
        assert_eq!(doc.paragraphs.len(), 1);
        assert!(doc.tail_open);
    }

    #[test]
    fn append_continues_open_tail_with_space_for_spaced_languages() {
        let mut doc = content(&[("p1", "He turned")], 2, true);
        append_segment(&mut doc, "and saw the blade", "en").expect("append");
        assert_eq!(doc.paragraphs[0].text, "He turned and saw the blade");
    }

    #[test]
    fn append_split_segment_keeps_tail_open_for_the_last_paragraph() {
        let mut doc = content(&[("p1", "开头")], 2, true);
        append_segment(&mut doc, "继续\n\n新段", "ja").expect("append");
        assert_eq!(doc.paragraphs[0].text, "开头继续");
        assert_eq!(doc.paragraphs[1].text, "新段");
        assert_eq!(doc.paragraphs[1].pid, "p2");
        assert!(doc.tail_open);
    }

    #[test]
    fn append_with_empty_segment_changes_nothing() {
        let mut doc = content(&[("p1", "text")], 2, false);
        let ops = append_segment(&mut doc, "  \n\n ", "en").expect("append");
        assert!(ops.is_empty());
        assert_eq!(doc.paragraphs.len(), 1);
    }
}
