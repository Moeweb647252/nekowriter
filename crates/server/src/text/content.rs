//! `ChapterContent` operations, pid allocation and the author's content edits (spec/02 §4,
//! spec/03 §8, spec/10 §3).
//!
//! The author edits content with `ContentOpInput` operations; the server allocates pids and
//! returns the resulting `ContentOp`s, which are broadcast as `chapter.content` so other tabs
//! apply exactly what was stored. Author edits never open the tail: they close it when the open
//! last paragraph disappears or stops being last (spec/08 §1.1).

use crate::api::dto::{ContentOp, ContentOpInput};
use crate::db::json::{ChapterContent, Paragraph};

use super::join::split_paragraphs;
use super::wordcount::count_words;

/// Why a content edit was rejected. Services map these to `AppError`s.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TextError {
    /// A pid referenced by an operation does not exist in the chapter.
    #[error("pid {0} does not exist")]
    PidNotFound(String),
    /// A pid does not have the `p<n>` form.
    #[error("pid {0} is not a paragraph id")]
    InvalidPid(String),
    /// An insert or update carried no text.
    #[error("{0} needs text")]
    EmptyText(&'static str),
    /// An operation produced a malformed pid.
    #[error("paragraph ids are exhausted")]
    PidExhausted,
}

/// The outcome of applying author content operations.
#[derive(Debug, Clone, PartialEq)]
pub struct AppliedContent {
    /// The stored operations, in application order.
    pub ops: Vec<ContentOp>,
    /// The chapter's word count after the change (UAX #29, spec/11 §8).
    pub word_count: i32,
}

/// Allocates the next paragraph id and advances `next_pid` (pids are never reused).
pub fn allocate_pid(content: &mut ChapterContent) -> String {
    let value = content.next_pid.max(1);
    content.next_pid = value.saturating_add(1);
    format!("p{value}")
}

/// Parses `p<n>` into `n`.
pub fn parse_pid(pid: &str) -> Option<i32> {
    pid.strip_prefix('p')
        .and_then(|digits| digits.parse::<i32>().ok())
        .filter(|value| *value > 0)
}

/// Keeps `next_pid` at least one above every existing pid, so old rows can never hand out a pid
/// that is still in use.
pub fn repair_next_pid(content: &mut ChapterContent) {
    let highest = content
        .paragraphs
        .iter()
        .filter_map(|paragraph| parse_pid(&paragraph.pid))
        .max()
        .unwrap_or(0);
    if content.next_pid <= highest {
        content.next_pid = highest.saturating_add(1);
    }
}

/// Keeps `next_pid` at least as high as it was, when a write replaces the whole document with one
/// that carries fewer paragraphs.
///
/// Pids are never reused (spec/02 §5 invariant 3: "`next_pid` only grows"), but a whole-document
/// replacement can carry fewer pids than the state it replaces: a redraft clears the text, and a
/// restore puts back an older version. Without this the counter went *backwards* — after a redraft
/// of a three-paragraph chapter it was 1 again, so the writer's next `insert_text` handed out the
/// pid `p1` that the archived text still used and that the old version still holds. `previous` is
/// the document being replaced; its own counter was already repaired when it was stored.
pub fn keep_next_pid(content: &mut ChapterContent, previous: &ChapterContent) {
    repair_next_pid(content);
    if content.next_pid < previous.next_pid {
        content.next_pid = previous.next_pid;
    }
}

/// The content's total word count.
pub fn content_word_count(content: &ChapterContent) -> i32 {
    content.paragraphs.iter().fold(0i32, |total, paragraph| {
        total.saturating_add(count_words(&paragraph.text))
    })
}

/// The ops that take a client from `before` to `after`, replayable by [`apply_ops`].
///
/// Used by the full-text save and by version restore, both of which can move paragraphs around.
/// `ContentOp` can only express "insert after a pid", so a paragraph that moved is reported as a
/// delete plus an insert at its new position: replaying the result reproduces `after` exactly,
/// which is what spec/10 §3 requires of `chapter.content` ("apply the ops if version = local + 1").
/// Emitting a bare `update` for a paragraph whose position changed would leave every other tab
/// with the client's old order.
pub fn transform_ops(
    before: &[Paragraph],
    after: &[Paragraph],
    tail_before: bool,
    tail_after: bool,
) -> Vec<ContentOp> {
    let before_index: std::collections::HashMap<&str, usize> = before
        .iter()
        .enumerate()
        .map(|(index, paragraph)| (paragraph.pid.as_str(), index))
        .collect();

    // A pid can stay where it is when it is present in both documents and its old position takes
    // part in a longest increasing subsequence of the surviving pids. Everything outside that
    // subsequence moved, so the client needs it re-inserted at its new place.
    let surviving: Vec<(usize, usize)> = after
        .iter()
        .enumerate()
        .filter_map(|(new_index, paragraph)| {
            before_index
                .get(paragraph.pid.as_str())
                .map(|old_index| (new_index, *old_index))
        })
        .collect();
    let anchored = increasing_run(&surviving);
    let moved: std::collections::HashSet<usize> = surviving
        .iter()
        .map(|(new_index, _)| *new_index)
        .filter(|new_index| !anchored.contains(new_index))
        .collect();

    let mut ops = Vec::new();
    let mut after_pid: Option<String> = None;
    for (new_index, paragraph) in after.iter().enumerate() {
        match before_index.get(paragraph.pid.as_str()) {
            Some(old_index) if !moved.contains(&new_index) => {
                if before[*old_index].text != paragraph.text {
                    ops.push(ContentOp::Update {
                        pid: paragraph.pid.clone(),
                        text: paragraph.text.clone(),
                    });
                }
            }
            Some(_) => {
                // Survived but moved: drop it where it was and re-add it here. `ContentOp` has no
                // move form and an `update` would leave the client's order unchanged (spec/10 §3).
                ops.push(ContentOp::Delete {
                    pid: paragraph.pid.clone(),
                });
                ops.push(ContentOp::Insert {
                    after: after_pid.clone(),
                    pid: paragraph.pid.clone(),
                    text: paragraph.text.clone(),
                });
            }
            None => ops.push(ContentOp::Insert {
                after: after_pid.clone(),
                pid: paragraph.pid.clone(),
                text: paragraph.text.clone(),
            }),
        }
        after_pid = Some(paragraph.pid.clone());
    }
    // Deleted paragraphs go last, so no insert above references a pid that is already gone.
    for paragraph in before {
        if !after.iter().any(|kept| kept.pid == paragraph.pid) {
            ops.push(ContentOp::Delete {
                pid: paragraph.pid.clone(),
            });
        }
    }
    if tail_before != tail_after {
        ops.push(ContentOp::Tail { open: tail_after });
    }
    ops
}

/// The new-index positions that form a longest increasing subsequence of the old positions.
///
/// O(n log n): `tails` holds, for each run length, the smallest old index that can end it.
fn increasing_run(surviving: &[(usize, usize)]) -> std::collections::HashSet<usize> {
    let mut tails: Vec<usize> = Vec::new(); // indices into `surviving`
    let mut previous: Vec<Option<usize>> = vec![None; surviving.len()];
    for (slot, (_, old_index)) in surviving.iter().enumerate() {
        let position = tails.partition_point(|tail| surviving[*tail].1 < *old_index);
        if position > 0 {
            previous[slot] = Some(tails[position - 1]);
        }
        if position == tails.len() {
            tails.push(slot);
        } else {
            tails[position] = slot;
        }
    }
    let mut anchored = std::collections::HashSet::new();
    let mut cursor = tails.last().copied();
    while let Some(slot) = cursor {
        anchored.insert(surviving[slot].0);
        cursor = previous[slot];
    }
    anchored
}

/// Applies the author's operations in order, allocating pids for inserts (spec/10 §3).
///
/// A single operation may carry several paragraphs (line breaks separate them); each becomes its
/// own paragraph. The open tail closes when the paragraph that was last before the edit is gone
/// or no longer last.
pub fn apply_inputs(
    content: &mut ChapterContent,
    inputs: &[ContentOpInput],
) -> Result<AppliedContent, TextError> {
    repair_next_pid(content);
    let last_before = content.paragraphs.last().map(|p| p.pid.clone());
    let mut ops = Vec::new();

    for input in inputs {
        match input {
            ContentOpInput::Insert { after, text } => {
                // An empty insert is the author's "start a new, empty paragraph" action
                // (spec/11 §5.2). Whitespace-only text is still rejected: trimming would
                // otherwise silently discard the paragraph the author asked for.
                let texts = if text.is_empty() {
                    vec![String::new()]
                } else {
                    split_paragraphs(text)
                };
                if texts.is_empty() {
                    return Err(TextError::EmptyText("insert"));
                }
                let mut anchor = after.clone();
                if let Some(pid) = &anchor
                    && !content.paragraphs.iter().any(|p| &p.pid == pid)
                {
                    return Err(TextError::PidNotFound(pid.clone()));
                }
                let index = match &anchor {
                    None => 0,
                    Some(pid) => content
                        .paragraphs
                        .iter()
                        .position(|p| &p.pid == pid)
                        .map(|position| position + 1)
                        .ok_or_else(|| TextError::PidNotFound(pid.clone()))?,
                };
                for (offset, text) in texts.into_iter().enumerate() {
                    let pid = allocate_pid(content);
                    content.paragraphs.insert(
                        index + offset,
                        Paragraph {
                            pid: pid.clone(),
                            text,
                        },
                    );
                    ops.push(ContentOp::Insert {
                        after: anchor.clone(),
                        pid: pid.clone(),
                        text: content.paragraphs[index + offset].text.clone(),
                    });
                    anchor = Some(pid);
                }
            }
            ContentOpInput::Update { pid, text } => {
                let texts = split_paragraphs(text);
                let Some(first) = texts.first().cloned() else {
                    return Err(TextError::EmptyText("update"));
                };
                let index = content
                    .paragraphs
                    .iter()
                    .position(|p| &p.pid == pid)
                    .ok_or_else(|| TextError::PidNotFound(pid.clone()))?;
                content.paragraphs[index].text = first.clone();
                ops.push(ContentOp::Update {
                    pid: pid.clone(),
                    text: first,
                });
                // Further paragraphs of an update become new paragraphs after the target.
                let mut anchor = pid.clone();
                for text in texts.into_iter().skip(1) {
                    let new_pid = allocate_pid(content);
                    let position = content
                        .paragraphs
                        .iter()
                        .position(|p| p.pid == anchor)
                        .map(|position| position + 1)
                        .ok_or_else(|| TextError::PidNotFound(anchor.clone()))?;
                    content.paragraphs.insert(
                        position,
                        Paragraph {
                            pid: new_pid.clone(),
                            text: text.clone(),
                        },
                    );
                    ops.push(ContentOp::Insert {
                        after: Some(anchor.clone()),
                        pid: new_pid.clone(),
                        text,
                    });
                    anchor = new_pid;
                }
            }
            ContentOpInput::Delete { pid } => {
                let index = content
                    .paragraphs
                    .iter()
                    .position(|p| &p.pid == pid)
                    .ok_or_else(|| TextError::PidNotFound(pid.clone()))?;
                content.paragraphs.remove(index);
                ops.push(ContentOp::Delete { pid: pid.clone() });
            }
        }
    }

    // Author edits never open the tail; they close it once the open paragraph changes position.
    if content.tail_open {
        let still_last = match &last_before {
            Some(pid) => content
                .paragraphs
                .last()
                .is_some_and(|last| &last.pid == pid),
            None => false,
        };
        if !still_last {
            content.tail_open = false;
            ops.push(ContentOp::Tail { open: false });
        }
    }

    Ok(AppliedContent {
        ops,
        word_count: content_word_count(content),
    })
}

/// Applies operations that already carry pids (an agent's stored ops, or a replayed event).
///
/// Unlike [`apply_inputs`] this does not trim, split or allocate: the ops are the exact change.
pub fn apply_ops(content: &mut ChapterContent, ops: &[ContentOp]) -> Result<(), TextError> {
    repair_next_pid(content);
    for op in ops {
        match op {
            ContentOp::Insert { after, pid, text } => {
                let index = match after {
                    None => 0,
                    Some(after_pid) => content
                        .paragraphs
                        .iter()
                        .position(|p| &p.pid == after_pid)
                        .map(|position| position + 1)
                        .ok_or_else(|| TextError::PidNotFound(after_pid.clone()))?,
                };
                if content.paragraphs.iter().any(|p| &p.pid == pid) {
                    return Err(TextError::InvalidPid(pid.clone()));
                }
                if parse_pid(pid).is_none() {
                    return Err(TextError::InvalidPid(pid.clone()));
                }
                content.paragraphs.insert(
                    index,
                    Paragraph {
                        pid: pid.clone(),
                        text: text.clone(),
                    },
                );
            }
            ContentOp::Update { pid, text } => {
                let paragraph = content
                    .paragraphs
                    .iter_mut()
                    .find(|p| &p.pid == pid)
                    .ok_or_else(|| TextError::PidNotFound(pid.clone()))?;
                paragraph.text = text.clone();
            }
            ContentOp::Delete { pid } => {
                let index = content
                    .paragraphs
                    .iter()
                    .position(|p| &p.pid == pid)
                    .ok_or_else(|| TextError::PidNotFound(pid.clone()))?;
                content.paragraphs.remove(index);
            }
            ContentOp::Tail { open } => content.tail_open = *open,
        }
    }
    repair_next_pid(content);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(paragraphs: &[(&str, &str)], next_pid: i32, tail_open: bool) -> ChapterContent {
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

    fn pids(content: &ChapterContent) -> Vec<&str> {
        content
            .paragraphs
            .iter()
            .map(|paragraph| paragraph.pid.as_str())
            .collect()
    }

    #[test]
    fn allocate_pid_only_grows() {
        let mut content = doc(&[("p3", "text")], 4, false);
        assert_eq!(allocate_pid(&mut content), "p4");
        assert_eq!(allocate_pid(&mut content), "p5");
        assert_eq!(content.next_pid, 6);
    }

    #[test]
    fn repair_next_pid_never_lowers_the_counter() {
        let mut content = doc(&[("p9", "text")], 2, false);
        repair_next_pid(&mut content);
        assert_eq!(content.next_pid, 10);
        let mut content = doc(&[("p1", "text")], 7, false);
        repair_next_pid(&mut content);
        assert_eq!(content.next_pid, 7);
    }

    #[test]
    fn keep_next_pid_survives_a_whole_document_replacement() {
        // A redraft: the replacement is empty, and the counter must not restart at 1.
        let previous = doc(&[("p1", "a"), ("p2", "b"), ("p3", "c")], 4, false);
        let mut cleared = ChapterContent::default();
        keep_next_pid(&mut cleared, &previous);
        assert_eq!(cleared.next_pid, 4);
        assert_eq!(allocate_pid(&mut cleared), "p4");

        // A restore: the older document's own counter is lower than the current one.
        let current = doc(
            &[("p1", "a"), ("p2", "b"), ("p3", "c"), ("p4", "d")],
            5,
            false,
        );
        let mut restored = doc(&[("p1", "a"), ("p2", "b")], 3, false);
        keep_next_pid(&mut restored, &current);
        assert_eq!(restored.next_pid, 5);

        // A document whose own pids are above its counter is repaired first, then kept monotonic.
        let mut repaired = doc(&[("p9", "a")], 1, false);
        keep_next_pid(&mut repaired, &previous);
        assert_eq!(repaired.next_pid, 10);
    }

    #[test]
    fn parse_pid_table() {
        assert_eq!(parse_pid("p1"), Some(1));
        assert_eq!(parse_pid("p12"), Some(12));
        assert_eq!(parse_pid("p0"), None);
        assert_eq!(parse_pid("p"), None);
        assert_eq!(parse_pid("1"), None);
        assert_eq!(parse_pid("px"), None);
    }

    #[test]
    fn insert_at_start_middle_and_end() {
        // Start: after = null.
        let mut content = doc(&[("p1", "one")], 2, false);
        let applied = apply_inputs(
            &mut content,
            &[ContentOpInput::Insert {
                after: None,
                text: "zero".to_owned(),
            }],
        )
        .expect("insert at start");
        assert_eq!(pids(&content), vec!["p2", "p1"]);
        assert_eq!(
            applied.ops,
            vec![ContentOp::Insert {
                after: None,
                pid: "p2".to_owned(),
                text: "zero".to_owned(),
            }]
        );

        // Middle: after = p1 → right after it.
        let applied = apply_inputs(
            &mut content,
            &[ContentOpInput::Insert {
                after: Some("p1".to_owned()),
                text: "one and a half".to_owned(),
            }],
        )
        .expect("insert in the middle");
        assert_eq!(pids(&content), vec!["p2", "p1", "p3"]);
        assert_eq!(content.paragraphs[2].text, "one and a half");
        assert_eq!(applied.ops.len(), 1);

        // End: after = last pid.
        apply_inputs(
            &mut content,
            &[ContentOpInput::Insert {
                after: Some("p3".to_owned()),
                text: "two".to_owned(),
            }],
        )
        .expect("insert at end");
        assert_eq!(pids(&content), vec!["p2", "p1", "p3", "p4"]);
        assert!(!content.tail_open);
    }

    #[test]
    fn insert_with_line_breaks_creates_several_paragraphs() {
        let mut content = doc(&[("p1", "one")], 2, false);
        let applied = apply_inputs(
            &mut content,
            &[ContentOpInput::Insert {
                after: Some("p1".to_owned()),
                text: "two\n\nthree\r\n\r\nfour".to_owned(),
            }],
        )
        .expect("multi-paragraph insert");
        assert_eq!(
            content
                .paragraphs
                .iter()
                .map(|p| p.text.as_str())
                .collect::<Vec<_>>(),
            vec!["one", "two", "three", "four"]
        );
        assert_eq!(applied.ops.len(), 3);
        assert_eq!(
            applied.ops[1],
            ContentOp::Insert {
                after: Some("p2".to_owned()),
                pid: "p3".to_owned(),
                text: "three".to_owned(),
            }
        );
    }

    #[test]
    fn update_and_delete_keep_the_paragraph_list_consistent() {
        let mut content = doc(&[("p1", "one"), ("p2", "two"), ("p3", "three")], 4, false);
        apply_inputs(
            &mut content,
            &[
                ContentOpInput::Update {
                    pid: "p2".to_owned(),
                    text: "  TWO  ".to_owned(),
                },
                ContentOpInput::Delete {
                    pid: "p1".to_owned(),
                },
            ],
        )
        .expect("update and delete");
        assert_eq!(pids(&content), vec!["p2", "p3"]);
        assert_eq!(content.paragraphs[0].text, "TWO");
        assert_eq!(content.next_pid, 4);
    }

    #[test]
    fn unknown_pid_is_rejected() {
        let mut content = doc(&[("p1", "one")], 2, false);
        let error = apply_inputs(
            &mut content,
            &[ContentOpInput::Update {
                pid: "p99".to_owned(),
                text: "x".to_owned(),
            }],
        )
        .expect_err("unknown pid");
        assert_eq!(error, TextError::PidNotFound("p99".to_owned()));
        let error = apply_inputs(
            &mut content,
            &[ContentOpInput::Insert {
                after: Some("p99".to_owned()),
                text: "x".to_owned(),
            }],
        )
        .expect_err("unknown anchor");
        assert_eq!(error, TextError::PidNotFound("p99".to_owned()));
        let error = apply_inputs(
            &mut content,
            &[ContentOpInput::Delete {
                pid: "p42".to_owned(),
            }],
        )
        .expect_err("unknown delete pid");
        assert_eq!(error, TextError::PidNotFound("p42".to_owned()));
    }

    #[test]
    fn empty_text_is_rejected() {
        let mut content = doc(&[("p1", "one")], 2, false);
        let error = apply_inputs(
            &mut content,
            &[ContentOpInput::Insert {
                after: None,
                text: "   \n\n ".to_owned(),
            }],
        )
        .expect_err("empty insert");
        assert_eq!(error, TextError::EmptyText("insert"));
    }

    #[test]
    fn an_empty_insert_creates_an_empty_placeholder_paragraph() {
        let mut content = doc(&[("p1", "one")], 2, false);
        let applied = apply_inputs(
            &mut content,
            &[ContentOpInput::Insert {
                after: Some("p1".to_owned()),
                text: String::new(),
            }],
        )
        .expect("empty insert is a valid new paragraph");
        assert_eq!(pids(&content), vec!["p1", "p2"]);
        assert_eq!(content.paragraphs[1].text, "");
        assert_eq!(
            applied.ops,
            vec![ContentOp::Insert {
                after: Some("p1".to_owned()),
                pid: "p2".to_owned(),
                text: String::new(),
            }]
        );
    }

    #[test]
    fn deleting_the_open_tail_closes_it() {
        let mut content = doc(&[("p1", "one"), ("p2", "open")], 3, true);
        let applied = apply_inputs(
            &mut content,
            &[ContentOpInput::Delete {
                pid: "p2".to_owned(),
            }],
        )
        .expect("delete tail");
        assert!(!content.tail_open);
        assert_eq!(
            applied.ops.last(),
            Some(&ContentOp::Tail { open: false }),
            "the tail closure is part of the broadcast ops"
        );
    }

    #[test]
    fn inserting_after_the_open_tail_closes_it() {
        let mut content = doc(&[("p1", "one")], 2, true);
        apply_inputs(
            &mut content,
            &[ContentOpInput::Insert {
                after: Some("p1".to_owned()),
                text: "two".to_owned(),
            }],
        )
        .expect("insert after open tail");
        assert!(!content.tail_open);
    }

    #[test]
    fn editing_the_open_tail_keeps_it_open() {
        let mut content = doc(&[("p1", "one"), ("p2", "ope")], 3, true);
        let applied = apply_inputs(
            &mut content,
            &[ContentOpInput::Update {
                pid: "p2".to_owned(),
                text: "open".to_owned(),
            }],
        )
        .expect("edit open tail");
        assert!(content.tail_open);
        assert_eq!(
            applied.ops,
            vec![ContentOp::Update {
                pid: "p2".to_owned(),
                text: "open".to_owned(),
            }]
        );
    }

    #[test]
    fn editing_an_earlier_paragraph_keeps_the_tail_open() {
        let mut content = doc(&[("p1", "one"), ("p2", "open")], 3, true);
        apply_inputs(
            &mut content,
            &[ContentOpInput::Update {
                pid: "p1".to_owned(),
                text: "uno".to_owned(),
            }],
        )
        .expect("edit earlier paragraph");
        assert!(content.tail_open);
    }

    #[test]
    fn word_count_is_recomputed_from_the_paragraphs() {
        let mut content = doc(&[("p1", "The wind")], 2, false);
        let applied = apply_inputs(
            &mut content,
            &[ContentOpInput::Insert {
                after: Some("p1".to_owned()),
                text: "风吹过山谷".to_owned(),
            }],
        )
        .expect("insert");
        assert_eq!(applied.word_count, count_words("The wind") + 5);
    }

    #[test]
    fn apply_ops_round_trips_through_the_store() {
        let mut content = doc(&[("p1", "one")], 2, false);
        let applied = apply_inputs(
            &mut content,
            &[
                ContentOpInput::Insert {
                    after: None,
                    text: "zero".to_owned(),
                },
                ContentOpInput::Update {
                    pid: "p1".to_owned(),
                    text: "uno".to_owned(),
                },
                ContentOpInput::Delete {
                    pid: "p2".to_owned(),
                },
            ],
        )
        .expect("edits");

        // A client holding the old content must reach the same state by applying the ops.
        let mut rebuilt = doc(&[("p1", "one")], 2, false);
        apply_ops(&mut rebuilt, &applied.ops).expect("apply ops");
        assert_eq!(rebuilt.paragraphs, content.paragraphs);
        assert_eq!(rebuilt.tail_open, content.tail_open);
        // The stored counter has moved past the deleted paragraphs: a later insert must not
        // reuse p2 or p3 (spec/02 §5.3).
        assert_eq!(content.next_pid, 3);
        assert_eq!(allocate_pid(&mut content), "p3");
    }

    #[test]
    fn apply_ops_rejects_a_duplicate_pid() {
        let mut content = doc(&[("p1", "one")], 2, false);
        let error = apply_ops(
            &mut content,
            &[ContentOp::Insert {
                after: None,
                pid: "p1".to_owned(),
                text: "duplicate".to_owned(),
            }],
        )
        .expect_err("duplicate pid");
        assert_eq!(error, TextError::InvalidPid("p1".to_owned()));
    }
}
