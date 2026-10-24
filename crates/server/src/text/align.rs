//! Paragraph alignment for full-text saves (spec/10 §2.2 `PUT /chapters/:id/content/text`).
//!
//! The author edits one large textarea; paragraphs are separated by blank lines. Saving must
//! keep the pids the agent and the UI already know where it can: unchanged and lightly edited
//! paragraphs keep their pid, new paragraphs get fresh pids, removed ones disappear. `similar`
//! aligns the two paragraph sequences; inside a replaced run, paragraphs are paired by text
//! similarity so a lightly edited paragraph is an update rather than a delete plus an insert.

use similar::{Algorithm, ChangeTag, TextDiff, capture_diff_slices};

use crate::api::dto::ContentOp;
use crate::db::json::{ChapterContent, Paragraph};

use super::content::{allocate_pid, content_word_count, repair_next_pid};

/// The result of a full-text save.
#[derive(Debug, Clone, PartialEq)]
pub struct AlignedContent {
    /// The ops describing the change, ready to broadcast.
    pub ops: Vec<ContentOp>,
    /// The word count of the new text.
    pub word_count: i32,
    /// The number of paragraphs whose pid was kept (unchanged or edited in place).
    pub kept_pids: usize,
}

/// Minimum similarity for a replaced paragraph to keep its pid.
const KEEP_SIMILARITY: f32 = 0.4;

/// Splits a full-text document into trimmed, non-empty paragraphs (blank lines separate them).
pub fn split_document(text: &str) -> Vec<String> {
    let unified = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut paragraphs = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in unified.split('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !current.is_empty() {
                paragraphs.push(current.join("\n"));
                current.clear();
            }
            continue;
        }
        current.push(trimmed);
    }
    if !current.is_empty() {
        paragraphs.push(current.join("\n"));
    }
    paragraphs
}

/// Replaces the content with the paragraphs of a full text, keeping pids where it can.
///
/// The tail closes: a full-text save writes whole paragraphs (spec/08 §1.1).
pub fn set_full_text(content: &mut ChapterContent, text: &str) -> AlignedContent {
    repair_next_pid(content);
    let old_paragraphs = content.paragraphs.clone();
    let new_texts = split_document(text);
    let alignment = align_paragraphs(&old_paragraphs, &new_texts);

    let mut paragraphs = Vec::with_capacity(new_texts.len());
    let mut ops = Vec::new();
    let mut kept_pids = 0usize;
    let mut after: Option<String> = None;
    for slot in alignment {
        match slot {
            Slot::Keep { old_index, text } => {
                let pid = old_paragraphs[old_index].pid.clone();
                if old_paragraphs[old_index].text != text {
                    ops.push(ContentOp::Update {
                        pid: pid.clone(),
                        text: text.clone(),
                    });
                }
                kept_pids += 1;
                after = Some(pid.clone());
                paragraphs.push(Paragraph { pid, text });
            }
            Slot::Insert { text } => {
                let pid = allocate_pid(content);
                ops.push(ContentOp::Insert {
                    after: after.clone(),
                    pid: pid.clone(),
                    text: text.clone(),
                });
                after = Some(pid.clone());
                paragraphs.push(Paragraph { pid, text });
            }
        }
    }

    // Deleted paragraphs are reported after the inserts that replaced them, so a client applying
    // the ops in order never references a pid that is already gone.
    for old in &old_paragraphs {
        if !paragraphs.iter().any(|paragraph| paragraph.pid == old.pid) {
            ops.push(ContentOp::Delete {
                pid: old.pid.clone(),
            });
        }
    }
    let tail_changed = content.tail_open;
    content.paragraphs = paragraphs;
    content.tail_open = false;
    if tail_changed {
        ops.push(ContentOp::Tail { open: false });
    }

    AlignedContent {
        ops,
        word_count: content_word_count(content),
        kept_pids,
    }
}

/// One paragraph of the aligned result.
#[derive(Debug, Clone, PartialEq)]
enum Slot {
    /// The paragraph keeps the pid of `old_index` with the (possibly edited) `text`.
    Keep { old_index: usize, text: String },
    /// The paragraph is new and needs a pid.
    Insert { text: String },
}

/// Aligns the old paragraphs with the new texts.
fn align_paragraphs(old: &[Paragraph], new_texts: &[String]) -> Vec<Slot> {
    let old_texts: Vec<&str> = old.iter().map(|p| p.text.as_str()).collect();
    let new_refs: Vec<&str> = new_texts.iter().map(String::as_str).collect();
    let ops = capture_diff_slices(Algorithm::Myers, &old_texts, &new_refs);

    let mut slots: Vec<Slot> = Vec::new();
    let mut pending_old: Vec<usize> = Vec::new();
    let mut pending_new: Vec<usize> = Vec::new();
    let flush =
        |slots: &mut Vec<Slot>, pending_old: &mut Vec<usize>, pending_new: &mut Vec<usize>| {
            let pairs = pair_replaced(pending_old, pending_new, old, new_texts);
            slots.extend(pairs);
            pending_old.clear();
            pending_new.clear();
        };

    for op in ops {
        match op {
            similar::DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => {
                flush(&mut slots, &mut pending_old, &mut pending_new);
                for offset in 0..len {
                    slots.push(Slot::Keep {
                        old_index: old_index + offset,
                        text: new_texts[new_index + offset].clone(),
                    });
                }
            }
            similar::DiffOp::Delete {
                old_index, old_len, ..
            } => {
                pending_old.extend(old_index..old_index + old_len);
            }
            similar::DiffOp::Insert {
                new_index, new_len, ..
            } => {
                pending_new.extend(new_index..new_index + new_len);
            }
            similar::DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                pending_old.extend(old_index..old_index + old_len);
                pending_new.extend(new_index..new_index + new_len);
            }
        }
    }
    flush(&mut slots, &mut pending_old, &mut pending_new);
    slots
}

/// Pairs removed and added paragraphs inside one replaced run by text similarity.
fn pair_replaced(
    pending_old: &[usize],
    pending_new: &[usize],
    old: &[Paragraph],
    new_texts: &[String],
) -> Vec<Slot> {
    if pending_new.is_empty() {
        // Pure deletion: nothing to emit (the caller reports deleted pids).
        return Vec::new();
    }
    if pending_old.is_empty() {
        return pending_new
            .iter()
            .map(|index| Slot::Insert {
                text: new_texts[*index].clone(),
            })
            .collect();
    }

    // Greedy best-match pairing, one old paragraph per new paragraph.
    let mut used_old = vec![false; pending_old.len()];
    let mut slots = Vec::new();
    for new_index in pending_new {
        let text = &new_texts[*new_index];
        let mut best: Option<(usize, f32)> = None;
        for (slot_index, old_index) in pending_old.iter().enumerate() {
            if used_old[slot_index] {
                continue;
            }
            let similarity = text_similarity(&old[*old_index].text, text);
            if similarity >= KEEP_SIMILARITY && best.is_none_or(|(_, score)| similarity > score) {
                best = Some((slot_index, similarity));
            }
        }
        match best {
            Some((slot_index, _)) => {
                used_old[slot_index] = true;
                slots.push(Slot::Keep {
                    old_index: pending_old[slot_index],
                    text: text.clone(),
                });
            }
            None => slots.push(Slot::Insert { text: text.clone() }),
        }
    }
    slots
}

/// Character-level similarity of two paragraphs, in `0.0..=1.0`.
fn text_similarity(a: &str, b: &str) -> f32 {
    if a == b {
        return 1.0;
    }
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let diff = TextDiff::from_chars(a, b);
    let mut matched = 0usize;
    for change in diff.iter_all_changes() {
        if change.tag() == ChangeTag::Equal {
            matched += change.value().chars().count();
        }
    }
    let total = a.chars().count().max(b.chars().count());
    if total == 0 {
        1.0
    } else {
        matched as f32 / total as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(paragraphs: &[(&str, &str)]) -> ChapterContent {
        ChapterContent {
            paragraphs: paragraphs
                .iter()
                .map(|(pid, text)| Paragraph {
                    pid: (*pid).to_owned(),
                    text: (*text).to_owned(),
                })
                .collect(),
            next_pid: paragraphs.len() as i32 + 1,
            tail_open: false,
        }
    }

    fn pid_of<'a>(content: &'a ChapterContent, text: &str) -> &'a str {
        content
            .paragraphs
            .iter()
            .find(|paragraph| paragraph.text == text)
            .map(|paragraph| paragraph.pid.as_str())
            .unwrap_or_else(|| panic!("no paragraph with text {text:?}"))
    }

    #[test]
    fn splitting_table() {
        let cases: &[(&str, Vec<&str>)] = &[
            ("one\n\ntwo", vec!["one", "two"]),
            ("one\n\ntwo\n\n\nthree", vec!["one", "two", "three"]),
            ("  one  \n\n  two  ", vec!["one", "two"]),
            ("single", vec!["single"]),
            ("", vec![]),
            ("\n\n\n", vec![]),
            (
                "line one\nline two\n\nnext",
                vec!["line one\nline two", "next"],
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(split_document(input), *expected, "input {input:?}");
        }
    }

    #[test]
    fn unchanged_paragraphs_keep_their_pids() {
        let mut content = doc(&[("p1", "one"), ("p2", "two"), ("p3", "three")]);
        let aligned = set_full_text(&mut content, "one\n\ntwo\n\nthree");
        assert_eq!(aligned.kept_pids, 3);
        assert!(aligned.ops.is_empty(), "no ops for an unchanged save");
        assert_eq!(
            content
                .paragraphs
                .iter()
                .map(|p| p.pid.as_str())
                .collect::<Vec<_>>(),
            vec!["p1", "p2", "p3"]
        );
    }

    #[test]
    fn edited_paragraph_keeps_its_pid() {
        let mut content = doc(&[("p1", "The wind blew through the valley."), ("p2", "Two.")]);
        let aligned = set_full_text(
            &mut content,
            "The wind blew through the quiet valley.\n\nTwo.",
        );
        assert_eq!(aligned.kept_pids, 2);
        assert_eq!(
            pid_of(&content, "The wind blew through the quiet valley."),
            "p1"
        );
        assert_eq!(aligned.ops.len(), 1);
        assert_eq!(
            aligned.ops[0],
            ContentOp::Update {
                pid: "p1".to_owned(),
                text: "The wind blew through the quiet valley.".to_owned(),
            }
        );
    }

    #[test]
    fn new_paragraph_gets_a_fresh_pid_and_deleted_ones_disappear() {
        let mut content = doc(&[("p1", "one"), ("p2", "two"), ("p3", "three")]);
        let aligned = set_full_text(&mut content, "one\n\nbrand new\n\nthree");
        assert_eq!(aligned.kept_pids, 2);
        assert_eq!(pid_of(&content, "brand new"), "p4");
        assert!(!content.paragraphs.iter().any(|p| p.pid == "p2"));
        assert_eq!(
            aligned.ops,
            vec![
                ContentOp::Insert {
                    after: Some("p1".to_owned()),
                    pid: "p4".to_owned(),
                    text: "brand new".to_owned(),
                },
                ContentOp::Delete {
                    pid: "p2".to_owned(),
                },
            ]
        );
        // Pids are never reused: p2 will not come back.
        let aligned_again = set_full_text(&mut content, "one\n\nbrand new\n\nthree\n\ntwo");
        assert_eq!(pid_of(&content, "two"), "p5");
        assert_eq!(aligned_again.ops.len(), 1);
    }

    #[test]
    fn a_few_lightly_edited_paragraphs_keep_their_pids() {
        let mut content = doc(&[
            ("p1", "第一段文字内容。"),
            ("p2", "第二段文字内容。"),
            ("p3", "第三段文字内容。"),
        ]);
        let aligned = set_full_text(
            &mut content,
            "第一段文字内容。\n\n第二段文字内容，略有修改。\n\n第三段文字内容。",
        );
        assert_eq!(aligned.kept_pids, 3);
        assert_eq!(
            content
                .paragraphs
                .iter()
                .map(|p| p.pid.as_str())
                .collect::<Vec<_>>(),
            vec!["p1", "p2", "p3"],
            "lightly edited paragraphs keep their pids"
        );
    }

    #[test]
    fn a_completely_different_paragraph_is_a_new_one() {
        let mut content = doc(&[("p1", "The wind blew.")]);
        let aligned = set_full_text(&mut content, "Completely unrelated sentence about taxes.");
        assert_eq!(aligned.kept_pids, 0);
        assert_eq!(
            content
                .paragraphs
                .iter()
                .map(|p| p.pid.as_str())
                .collect::<Vec<_>>(),
            vec!["p2"],
            "a dissimilar replacement gets a fresh pid"
        );
        assert_eq!(
            aligned.ops,
            vec![
                ContentOp::Insert {
                    after: None,
                    pid: "p2".to_owned(),
                    text: "Completely unrelated sentence about taxes.".to_owned(),
                },
                ContentOp::Delete {
                    pid: "p1".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn full_text_save_closes_the_tail() {
        let mut content = doc(&[("p1", "open")]);
        content.tail_open = true;
        let aligned = set_full_text(&mut content, "open");
        assert!(!content.tail_open);
        assert_eq!(aligned.ops, vec![ContentOp::Tail { open: false }]);
    }

    #[test]
    fn word_count_uses_the_shared_rule() {
        let mut content = doc(&[]);
        let aligned = set_full_text(&mut content, "风吹过山谷\n\nThe wind blew");
        assert_eq!(aligned.word_count, 5 + 3);
    }

    #[test]
    fn pids_of_an_old_document_are_never_reissued_after_a_save() {
        let mut content = doc(&[("p1", "alpha"), ("p7", "beta")]);
        content.next_pid = 2; // a hand-edited row: the counter is behind
        set_full_text(&mut content, "alpha\n\ngamma\n\nbeta");
        let pids: Vec<&str> = content.paragraphs.iter().map(|p| p.pid.as_str()).collect();
        assert_eq!(pids, vec!["p1", "p8", "p7"]);
        assert_eq!(content.next_pid, 9);
    }

    #[test]
    fn empty_full_text_clears_the_document() {
        let mut content = doc(&[("p1", "alpha")]);
        let aligned = set_full_text(&mut content, "   \n\n  ");
        assert!(content.paragraphs.is_empty());
        assert_eq!(
            aligned.ops,
            vec![ContentOp::Delete {
                pid: "p1".to_owned(),
            }]
        );
        assert_eq!(aligned.word_count, 0);
    }
}
