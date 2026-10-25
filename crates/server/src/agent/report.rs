//! Edit reports: telling the agent exactly what the author changed while the session was not
//! running (spec/03 §7, D-010, D-027).
//!
//! A writer or conception session stores what the agent last saw in `view_snapshot` when it
//! leaves `running`. The next time the session talks to the model, the runtime compares that
//! snapshot with the current state:
//!
//! - **writer** (`notice.user_edits`): a paragraph diff by pid — modified (same pid, different
//!   text), deleted (pid gone), inserted (new pid, located by the preceding one), the tail flag,
//!   and the full outline/reference notes when they changed. Old text is truncated to 120
//!   characters. Above 4,000 characters a compact variant replaces the paragraph part.
//! - **conception** (`notice.setting_edits`): per changed document a line diff when at most 40
//!   lines changed (otherwise its full text), the current ranked character list plus the profile
//!   of each changed design, and the current topic list.
//!
//! `view_snapshot` is cleared after it is reported.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use similar::{ChangeTag, TextDiff};

use sea_orm::ConnectionTrait;

use crate::db::json::{ChapterContent, ConceptionDocSnapshot, ViewSnapshot};
use crate::error::AppError;
use crate::prompts::PromptSet;
use crate::prompts::context::{
    DeletedParagraphContext, EditCountsContext, InsertedParagraphContext, ModifiedParagraphContext,
    ParagraphContext,
};

/// The maximum length of an old paragraph quoted in the report (spec/03 §7).
pub const OLD_TEXT_LIMIT: usize = 120;

/// The paragraph part above this length becomes the compact variant (spec/03 §7).
pub const COMPACT_THRESHOLD: usize = 4_000;

/// The maximum number of changed lines rendered as a diff before the full text is shown
/// (spec/03 §7, conception).
pub const MAX_DIFF_LINES: usize = 40;

/// The current chapter state the report compares against.
#[derive(Debug, Clone, PartialEq)]
pub struct ChapterState {
    pub content: ChapterContent,
    pub outline: String,
    pub reference_notes: String,
}

/// The result of comparing a snapshot with the current state.
#[derive(Debug, Clone)]
pub struct ChapterEditReport {
    pub modified: Vec<ModifiedParagraphContext>,
    pub deleted: Vec<DeletedParagraphContext>,
    pub inserted: Vec<InsertedParagraphContext>,
    pub tail_changed: bool,
    pub tail_open: bool,
    pub outline_changed: bool,
    pub outline: String,
    pub notes_changed: bool,
    pub reference_notes: String,
    /// The compact variant (paragraph counts plus "call read_draft").
    pub compact: bool,
    /// The length of the paragraph part (before compaction).
    pub paragraph_chars: usize,
}

impl ChapterEditReport {
    /// True when nothing at all changed.
    pub fn is_empty(&self) -> bool {
        self.modified.is_empty()
            && self.deleted.is_empty()
            && self.inserted.is_empty()
            && !self.tail_changed
            && !self.outline_changed
            && !self.notes_changed
    }

    /// The rendered length of the paragraph part of the report (spec/03 §7).
    ///
    /// The spec measures the **report's** paragraph part, so the measure is the text the template
    /// actually prints for the modified/deleted/inserted lines: the prompt punctuation lives in
    /// `prompts/notices.toml` and must not be duplicated in code. The trick is to render twice
    /// with the same context except for the paragraph lists, and with a tail change present in
    /// both so the "no change" branch never fires: the difference is exactly the paragraph lines.
    pub fn paragraph_part_chars(
        &self,
        prompts: &PromptSet,
    ) -> Result<usize, crate::error::AppError> {
        let mut probe = self.clone();
        probe.compact = false;
        probe.outline_changed = false;
        probe.notes_changed = false;
        probe.tail_changed = true;
        probe.tail_open = true;
        let with_paragraphs = self.render_with(prompts, &probe)?.chars().count();
        let mut bare = probe.clone();
        bare.modified.clear();
        bare.deleted.clear();
        bare.inserted.clear();
        let without = self.render_with(prompts, &bare)?.chars().count();
        Ok(with_paragraphs.saturating_sub(without))
    }

    /// True when the compact variant replaces the paragraph part (spec/03 §7).
    pub fn is_compact(&self, prompts: &PromptSet) -> Result<bool, crate::error::AppError> {
        if self.compact {
            return Ok(true);
        }
        Ok(self.paragraph_part_chars(prompts)? > COMPACT_THRESHOLD)
    }

    /// Renders `notice.user_edits` for a variant of this report (compaction measurement).
    fn render_with(
        &self,
        prompts: &PromptSet,
        variant: &ChapterEditReport,
    ) -> Result<String, crate::error::AppError> {
        let context = json!({
            "compact": variant.compact,
            "counts": EditCountsContext {
                modified: variant.modified.len() as i32,
                deleted: variant.deleted.len() as i32,
                inserted: variant.inserted.len() as i32,
            },
            "modified": if variant.compact { Value::Array(Vec::new()) } else { clone_json(&variant.modified) },
            "deleted": if variant.compact { Value::Array(Vec::new()) } else { clone_json(&variant.deleted) },
            "inserted": if variant.compact { Value::Array(Vec::new()) } else { clone_json(&variant.inserted) },
            "tail_changed": variant.tail_changed,
            "tail_open": variant.tail_open,
            "outline_changed": variant.outline_changed,
            "outline": variant.outline,
            "notes_changed": variant.notes_changed,
            "reference_notes": variant.reference_notes,
        });
        prompts
            .render_value(
                "notice.user_edits",
                crate::prompts::template_value(&context),
            )
            .map(|rendered| rendered.text)
            .map_err(|err| err.into_app_error())
    }

    /// Renders `notice.user_edits` (spec/03 §7). Labels are rendered by the registry.
    ///
    /// Above 4,000 characters of paragraph report the compact variant replaces the paragraph part
    /// with counts and the instruction to call `read_draft`.
    pub fn render(&self, prompts: &PromptSet) -> Result<String, crate::error::AppError> {
        if self.is_compact(prompts)? {
            let mut compact = self.clone();
            compact.compact = true;
            return compact.render_with(prompts, &compact);
        }
        let context = json!({
            "compact": self.compact,
            "counts": EditCountsContext {
                modified: self.modified.len() as i32,
                deleted: self.deleted.len() as i32,
                inserted: self.inserted.len() as i32,
            },
            "modified": if self.compact { Value::Array(Vec::new()) } else { clone_json(&self.modified) },
            "deleted": if self.compact { Value::Array(Vec::new()) } else { clone_json(&self.deleted) },
            "inserted": if self.compact { Value::Array(Vec::new()) } else { clone_json(&self.inserted) },
            "tail_changed": self.tail_changed,
            "tail_open": self.tail_open,
            "outline_changed": self.outline_changed,
            "outline": self.outline,
            "notes_changed": self.notes_changed,
            "reference_notes": self.reference_notes,
        });
        prompts
            .render_value(
                "notice.user_edits",
                crate::prompts::template_value(&context),
            )
            .map(|rendered| rendered.text)
            .map_err(|err| err.into_app_error())
    }
}

fn clone_json<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// Compares what a writer session last saw with the chapter's current state (spec/03 §7).
pub fn chapter_report(snapshot: &ViewSnapshot, current: &ChapterState) -> ChapterEditReport {
    let ViewSnapshot::Chapter {
        content,
        outline,
        reference_notes,
    } = snapshot
    else {
        return empty_report(current);
    };

    let before: BTreeMap<&str, &str> = content
        .paragraphs
        .iter()
        .map(|paragraph| (paragraph.pid.as_str(), paragraph.text.as_str()))
        .collect();
    let after: BTreeMap<&str, &str> = current
        .content
        .paragraphs
        .iter()
        .map(|paragraph| (paragraph.pid.as_str(), paragraph.text.as_str()))
        .collect();

    let mut modified = Vec::new();
    for paragraph in &current.content.paragraphs {
        if let Some(old) = before.get(paragraph.pid.as_str())
            && *old != paragraph.text
        {
            modified.push(ModifiedParagraphContext {
                pid: paragraph.pid.clone(),
                old: truncate(old, OLD_TEXT_LIMIT),
                new: paragraph.text.clone(),
            });
        }
    }

    let mut deleted = Vec::new();
    for paragraph in &content.paragraphs {
        if !after.contains_key(paragraph.pid.as_str()) {
            deleted.push(DeletedParagraphContext {
                pid: paragraph.pid.clone(),
                old: truncate(&paragraph.text, OLD_TEXT_LIMIT),
            });
        }
    }

    let mut inserted = Vec::new();
    let mut previous: Option<String> = None;
    for paragraph in &current.content.paragraphs {
        if !before.contains_key(paragraph.pid.as_str()) {
            inserted.push(InsertedParagraphContext {
                pid: paragraph.pid.clone(),
                after: previous.clone(),
                text: paragraph.text.clone(),
            });
        }
        previous = Some(paragraph.pid.clone());
    }

    ChapterEditReport {
        modified,
        deleted,
        inserted,
        tail_changed: content.tail_open != current.content.tail_open,
        tail_open: current.content.tail_open,
        outline_changed: outline != &current.outline,
        outline: current.outline.clone(),
        notes_changed: reference_notes != &current.reference_notes,
        reference_notes: current.reference_notes.clone(),
        // The compaction decision needs the rendered template, so it is made by `render`/
        // `is_compact` (spec/03 §7); `chapter_report` stays a pure comparison.
        compact: false,
        paragraph_chars: 0,
    }
}

/// A report for a chapter the session never saw a snapshot of (defensive: nothing to report).
fn empty_report(current: &ChapterState) -> ChapterEditReport {
    ChapterEditReport {
        modified: Vec::new(),
        deleted: Vec::new(),
        inserted: Vec::new(),
        tail_changed: false,
        tail_open: current.content.tail_open,
        outline_changed: false,
        outline: current.outline.clone(),
        notes_changed: false,
        reference_notes: current.reference_notes.clone(),
        compact: false,
        paragraph_chars: 0,
    }
}

/// Truncates to `limit` characters, appending an ellipsis when anything was cut.
pub fn truncate(text: &str, limit: usize) -> String {
    let count = text.chars().count();
    if count <= limit {
        return text.to_owned();
    }
    let mut truncated: String = text.chars().take(limit).collect();
    truncated.push('…');
    truncated
}

// ───────────────────────────── conception ─────────────────────────────

/// What changed in the setting materials while a conception session was not running
/// (spec/03 §7).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ConceptionEditReport {
    pub items: Vec<Value>,
}

impl ConceptionEditReport {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn render(&self, prompts: &PromptSet) -> Result<String, crate::error::AppError> {
        let context = json!({ "items": self.items });
        prompts
            .render_value(
                "notice.setting_edits",
                crate::prompts::template_value(&context),
            )
            .map(|rendered| rendered.text)
            .map_err(|err| err.into_app_error())
    }
}

/// One current character design, as the report renders it.
#[derive(Debug, Clone, PartialEq)]
pub struct CurrentCharacter {
    pub id: i32,
    pub rank: i32,
    pub name: String,
    pub role_label: String,
    pub summary: String,
    pub profile: String,
}

/// One current research topic.
#[derive(Debug, Clone, PartialEq)]
pub struct CurrentTopic {
    pub topic: String,
    pub requirements: String,
    pub scope: String,
}

/// The current setting materials the conception report compares against.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ConceptionState {
    pub docs: BTreeMap<String, ConceptionDocSnapshot>,
    pub characters: Vec<CurrentCharacter>,
    pub topics: Vec<CurrentTopic>,
}

/// Compares what a conception session last saw with the current setting materials (spec/03 §7).
pub fn conception_report(
    snapshot: &ViewSnapshot,
    current: &ConceptionState,
    prompts: &PromptSet,
) -> ConceptionEditReport {
    let ViewSnapshot::Conception {
        docs,
        characters,
        topics,
    } = snapshot
    else {
        return ConceptionEditReport::default();
    };
    let mut report = ConceptionEditReport::default();

    // Documents: a changed revision renders a line diff (≤ 40 changed lines) or the full text.
    for (kind, before) in docs {
        let Some(after) = current.docs.get(kind) else {
            continue;
        };
        if before.revision == after.revision && before.content == after.content {
            continue;
        }
        let label = prompts.label("setting_doc", kind);
        let lines = diff_lines(&before.content, &after.content);
        let changed_lines: usize = lines
            .iter()
            .filter(|line| line["op"] == json!("+") || line["op"] == json!("-"))
            .count();
        let full_text = if changed_lines <= MAX_DIFF_LINES {
            String::new()
        } else {
            after.content.clone()
        };
        report.items.push(json!({
            "doc_label": label,
            "kind": "doc",
            "full_text": full_text,
            "lines": if full_text.is_empty() { lines } else { Vec::new() },
            "characters": [],
            "topics": [],
        }));
    }

    // Characters: the current ranked list, plus the profile of each changed design.
    let characters_changed = characters
        .iter()
        .map(|design| {
            (
                design.id,
                design.rank,
                design.name.clone(),
                design.role_label.clone(),
                design.summary.clone(),
                design.profile.clone(),
            )
        })
        .collect::<Vec<_>>()
        != current
            .characters
            .iter()
            .map(|design| {
                (
                    design.id,
                    design.rank,
                    design.name.clone(),
                    design.role_label.clone(),
                    design.summary.clone(),
                    design.profile.clone(),
                )
            })
            .collect::<Vec<_>>();
    let profile_changed = |id: i32| -> Option<String> {
        let before = characters.iter().find(|design| design.id == id);
        let after = current.characters.iter().find(|design| design.id == id)?;
        match before {
            Some(before) if before.profile == after.profile => None,
            _ => Some(after.profile.clone()),
        }
    };
    if characters_changed {
        let rendered: Vec<Value> = current
            .characters
            .iter()
            .map(|design| {
                json!({
                    "rank": design.rank,
                    "name": design.name,
                    "role_label": design.role_label,
                    "summary": design.summary,
                    "profile": design.profile,
                    "changed_profile": profile_changed(design.id).unwrap_or_default(),
                })
            })
            .collect();
        report.items.push(json!({
            "doc_label": prompts.label("target_type", "character_design"),
            "kind": "characters",
            "full_text": "",
            "lines": [],
            "characters": rendered,
            "topics": [],
        }));
    }

    // Research topics: the current list, when it changed.
    let topics_changed = topics
        .iter()
        .map(|topic| {
            (
                topic.topic.clone(),
                topic.requirements.clone(),
                topic.scope.clone(),
            )
        })
        .collect::<Vec<_>>()
        != current
            .topics
            .iter()
            .map(|topic| {
                (
                    topic.topic.clone(),
                    topic.requirements.clone(),
                    topic.scope.clone(),
                )
            })
            .collect::<Vec<_>>();
    if topics_changed {
        let rendered: Vec<Value> = current
            .topics
            .iter()
            .map(|topic| {
                json!({
                    "topic": topic.topic,
                    "scope": topic.scope,
                    "requirements": topic.requirements,
                })
            })
            .collect();
        report.items.push(json!({
            "doc_label": prompts.label("target_type", "research_task"),
            "kind": "topics",
            "full_text": "",
            "lines": [],
            "characters": [],
            "topics": rendered,
        }));
    }
    report
}

/// A line diff of two texts as `{op, text}` entries (spec/03 §7).
fn diff_lines(before: &str, after: &str) -> Vec<Value> {
    let diff = TextDiff::from_lines(before, after);
    let mut lines = Vec::new();
    for change in diff.iter_all_changes() {
        let op = match change.tag() {
            ChangeTag::Insert => "+",
            ChangeTag::Delete => "-",
            ChangeTag::Equal => continue,
        };
        lines.push(json!({
            "op": op,
            "text": change.value().trim_end_matches('\n'),
        }));
    }
    lines
}

/// The paragraph texts of a draft, as `block.chapter_draft` sees them.
pub fn paragraph_contexts(content: &ChapterContent) -> Vec<ParagraphContext> {
    content
        .paragraphs
        .iter()
        .map(|paragraph| ParagraphContext {
            pid: paragraph.pid.clone(),
            text: paragraph.text.clone(),
        })
        .collect()
}

/// Reads the current setting materials for the conception edit report (spec/03 §7).
pub async fn current_conception_state(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<ConceptionState, AppError> {
    use crate::db::entities::{character_designs, research_tasks, setting_docs};
    use sea_orm::{ColumnTrait, EntityTrait as _, QueryFilter, QueryOrder};

    let docs = setting_docs::Entity::find()
        .filter(setting_docs::Column::BookId.eq(book_id))
        .all(conn)
        .await?;
    let mut doc_map = BTreeMap::new();
    for doc in docs {
        doc_map.insert(
            doc.kind,
            ConceptionDocSnapshot {
                revision: doc.revision,
                content: doc.content,
            },
        );
    }
    let characters = character_designs::Entity::find()
        .filter(character_designs::Column::BookId.eq(book_id))
        .order_by_asc(character_designs::Column::Rank)
        .all(conn)
        .await?
        .into_iter()
        .map(|row| CurrentCharacter {
            id: row.id,
            rank: row.rank,
            name: row.name,
            role_label: row.role_label,
            summary: row.summary,
            profile: row.profile,
        })
        .collect();
    let topics = research_tasks::Entity::find()
        .filter(research_tasks::Column::BookId.eq(book_id))
        .filter(research_tasks::Column::Status.eq("draft"))
        .order_by_asc(research_tasks::Column::Id)
        .all(conn)
        .await?
        .into_iter()
        .map(|row| CurrentTopic {
            topic: row.topic,
            requirements: row.requirements,
            scope: row.scope_name,
        })
        .collect();
    Ok(ConceptionState {
        docs: doc_map,
        characters,
        topics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::json::Paragraph;

    fn content(paragraphs: &[(&str, &str)], tail_open: bool) -> ChapterContent {
        ChapterContent {
            paragraphs: paragraphs
                .iter()
                .map(|(pid, text)| Paragraph {
                    pid: (*pid).to_owned(),
                    text: (*text).to_owned(),
                })
                .collect(),
            next_pid: 100,
            tail_open,
        }
    }

    fn state(paragraphs: &[(&str, &str)], tail_open: bool) -> ChapterState {
        ChapterState {
            content: content(paragraphs, tail_open),
            outline: String::new(),
            reference_notes: String::new(),
        }
    }

    fn snapshot(paragraphs: &[(&str, &str)], tail_open: bool) -> ViewSnapshot {
        ViewSnapshot::Chapter {
            content: content(paragraphs, tail_open),
            outline: String::new(),
            reference_notes: String::new(),
        }
    }

    #[test]
    fn an_unchanged_chapter_reports_nothing() {
        let report = chapter_report(
            &snapshot(&[("p1", "a")], false),
            &state(&[("p1", "a")], false),
        );
        assert!(report.is_empty());
    }

    #[test]
    fn modifications_deletions_and_insertions_are_classified_by_pid() {
        let report = chapter_report(
            &snapshot(&[("p1", "one"), ("p2", "two"), ("p3", "three")], false),
            &state(
                &[("p1", "one"), ("p2", "two changed"), ("p4", "four")],
                false,
            ),
        );
        assert_eq!(report.modified.len(), 1);
        assert_eq!(report.modified[0].pid, "p2");
        assert_eq!(report.modified[0].old, "two");
        assert_eq!(report.modified[0].new, "two changed");
        assert_eq!(report.deleted.len(), 1);
        assert_eq!(report.deleted[0].pid, "p3");
        assert_eq!(report.inserted.len(), 1);
        assert_eq!(report.inserted[0].pid, "p4");
        assert_eq!(
            report.inserted[0].after.as_deref(),
            Some("p2"),
            "an inserted paragraph names its predecessor"
        );
    }

    #[test]
    fn an_insertion_at_the_start_has_no_predecessor() {
        let report = chapter_report(
            &snapshot(&[("p1", "one")], false),
            &state(&[("p9", "zero"), ("p1", "one")], false),
        );
        assert_eq!(report.inserted.len(), 1);
        assert_eq!(report.inserted[0].pid, "p9");
        assert_eq!(report.inserted[0].after, None);
    }

    #[test]
    fn the_tail_flag_is_reported_when_it_changes() {
        let report = chapter_report(
            &snapshot(&[("p1", "a")], true),
            &state(&[("p1", "a")], false),
        );
        assert!(report.tail_changed);
        assert!(!report.tail_open);
        assert!(!report.is_empty());
    }

    #[test]
    fn outline_and_notes_changes_carry_their_full_text() {
        let mut current = state(&[("p1", "a")], false);
        current.outline = "new outline".to_owned();
        current.reference_notes = "new notes".to_owned();
        let report = chapter_report(&snapshot(&[("p1", "a")], false), &current);
        assert!(report.outline_changed);
        assert_eq!(report.outline, "new outline");
        assert!(report.notes_changed);
        assert_eq!(report.reference_notes, "new notes");
    }

    #[test]
    fn old_text_is_truncated_to_120_characters() {
        let long = "x".repeat(200);
        let report = chapter_report(
            &snapshot(&[("p1", &long)], false),
            &state(&[("p1", "short")], false),
        );
        assert_eq!(report.modified[0].old.chars().count(), OLD_TEXT_LIMIT + 1);
        assert!(report.modified[0].old.ends_with('…'));
    }

    #[test]
    fn new_text_is_never_truncated() {
        let long = "y".repeat(500);
        let report = chapter_report(
            &snapshot(&[("p1", "short")], false),
            &state(&[("p1", &long)], false),
        );
        assert_eq!(report.modified[0].new, long);
    }

    #[test]
    fn a_large_edit_becomes_the_compact_variant() {
        let before: Vec<(String, String)> = (1..=60)
            .map(|index| {
                (
                    format!("p{index}"),
                    "old text that is fairly long ".repeat(3),
                )
            })
            .collect();
        let after: Vec<(String, String)> = (1..=60)
            .map(|index| {
                (
                    format!("p{index}"),
                    "new text that is also fairly long ".repeat(3),
                )
            })
            .collect();
        let before_refs: Vec<(&str, &str)> = before
            .iter()
            .map(|(pid, text)| (pid.as_str(), text.as_str()))
            .collect();
        let after_refs: Vec<(&str, &str)> = after
            .iter()
            .map(|(pid, text)| (pid.as_str(), text.as_str()))
            .collect();
        let report = chapter_report(&snapshot(&before_refs, false), &state(&after_refs, false));
        assert_eq!(report.modified.len(), 60);
        let prompts = crate::prompts::PromptRegistry::embedded()
            .expect("pack")
            .defaults_only();
        assert!(
            report.is_compact(&prompts).expect("compaction"),
            "paragraph part was {} characters",
            report.paragraph_part_chars(&prompts).expect("chars")
        );
    }

    #[test]
    fn a_small_edit_stays_detailed() {
        let report = chapter_report(
            &snapshot(&[("p1", "old")], false),
            &state(&[("p1", "new")], false),
        );
        let prompts = crate::prompts::PromptRegistry::embedded()
            .expect("pack")
            .defaults_only();
        assert!(!report.is_compact(&prompts).expect("compaction"));
    }

    /// spec/03 §7: the compact variant replaces the paragraph part only above 4,000 characters.
    /// The boundary is exclusive, and the measure is the rendered report (prompt punctuation
    /// included), not a character sum computed in code.
    #[test]
    fn the_compaction_threshold_is_the_rendered_paragraph_part() {
        let prompts = crate::prompts::PromptRegistry::embedded()
            .expect("pack")
            .defaults_only();
        // One inserted paragraph whose rendered line is exactly the threshold is NOT compact.
        let candidate = || {
            let mut report = chapter_report(&snapshot(&[], false), &state(&[("p1", "")], false));
            report.inserted.clear();
            report.modified.clear();
            report.deleted.clear();
            report.tail_changed = true;
            report
        };
        // Measure the overhead of one inserted line by fixing the text length.
        let mut probe = candidate();
        probe.inserted.push(InsertedParagraphContext {
            pid: "p1".to_owned(),
            after: None,
            text: String::new(),
        });
        let overhead = probe.paragraph_part_chars(&prompts).expect("chars");
        assert!(overhead > 0, "an inserted line prints something");
        // The largest text that still fits, and one more character.
        let fits = COMPACT_THRESHOLD.saturating_sub(overhead);
        let mut at_limit = candidate();
        at_limit.inserted.push(InsertedParagraphContext {
            pid: "p1".to_owned(),
            after: None,
            text: "x".repeat(fits),
        });
        assert_eq!(
            at_limit.paragraph_part_chars(&prompts).expect("chars"),
            COMPACT_THRESHOLD,
            "the probe measures exactly the rendered line length"
        );
        assert!(
            !at_limit.is_compact(&prompts).expect("compaction"),
            "exactly 4,000 characters is still detailed"
        );
        let mut over = candidate();
        over.inserted.push(InsertedParagraphContext {
            pid: "p1".to_owned(),
            after: None,
            text: "x".repeat(fits + 1),
        });
        assert_eq!(
            over.paragraph_part_chars(&prompts).expect("chars"),
            COMPACT_THRESHOLD + 1
        );
        assert!(
            over.is_compact(&prompts).expect("compaction"),
            "4,001 characters switches to the compact variant"
        );
    }

    #[test]
    fn diff_lines_marks_insertions_and_deletions() {
        let lines = diff_lines("a\nb\n", "a\nc\n");
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(lines[0]["op"], json!("-"));
        assert_eq!(lines[0]["text"], "b");
        assert_eq!(lines[1]["op"], json!("+"));
        assert_eq!(lines[1]["text"], "c");
    }

    // ───────────────────────────── conception (spec/03 §7) ─────────────────────────────

    fn prompts() -> crate::prompts::PromptSet {
        crate::prompts::PromptRegistry::embedded()
            .expect("the embedded pack")
            .defaults_only()
    }

    fn conception_snapshot(
        docs: &[(&str, i32, &str)],
        characters: &[CurrentCharacter],
        topics: &[CurrentTopic],
    ) -> ViewSnapshot {
        ViewSnapshot::Conception {
            docs: docs
                .iter()
                .map(|(kind, revision, content)| {
                    (
                        (*kind).to_owned(),
                        ConceptionDocSnapshot {
                            revision: *revision,
                            content: (*content).to_owned(),
                        },
                    )
                })
                .collect(),
            characters: characters
                .iter()
                .map(|character| crate::db::json::ConceptionCharacterSnapshot {
                    id: character.id,
                    rank: character.rank,
                    name: character.name.clone(),
                    role_label: character.role_label.clone(),
                    summary: character.summary.clone(),
                    profile: character.profile.clone(),
                })
                .collect(),
            topics: topics
                .iter()
                .map(|topic| crate::db::json::ConceptionTopicSnapshot {
                    topic: topic.topic.clone(),
                    requirements: topic.requirements.clone(),
                    scope: topic.scope.clone(),
                })
                .collect(),
        }
    }

    #[test]
    fn a_conception_report_renders_a_small_document_edit_as_a_line_diff() {
        let before = conception_snapshot(&[("outline", 1, "a\nb\n")], &[], &[]);
        let mut current = ConceptionState::default();
        current.docs.insert(
            "outline".to_owned(),
            ConceptionDocSnapshot {
                revision: 2,
                content: "a\nc\n".to_owned(),
            },
        );
        let report = conception_report(&before, &current, &prompts());
        assert_eq!(report.items.len(), 1);
        let item = &report.items[0];
        assert_eq!(item["kind"], json!("doc"));
        assert_eq!(item["full_text"], json!(""), "a small edit stays a diff");
        let lines = item["lines"].as_array().cloned().unwrap_or_default();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(lines[0]["op"], json!("-"));
        assert_eq!(lines[0]["text"], json!("b"));
        assert_eq!(lines[1]["op"], json!("+"));
        assert_eq!(lines[1]["text"], json!("c"));
        // The label is the prompt pack's, never an English literal.
        assert_eq!(
            item["doc_label"].as_str(),
            Some(prompts().label("setting_doc", "outline").as_str())
        );
    }

    #[test]
    fn a_conception_report_shows_the_full_text_above_forty_changed_lines() {
        let before_text: String = (0..MAX_DIFF_LINES + 5)
            .map(|index| format!("old line {index}\n"))
            .collect();
        let after_text: String = (0..MAX_DIFF_LINES + 5)
            .map(|index| format!("new line {index}\n"))
            .collect();
        let before = conception_snapshot(&[("world", 1, &before_text)], &[], &[]);
        let mut current = ConceptionState::default();
        current.docs.insert(
            "world".to_owned(),
            ConceptionDocSnapshot {
                revision: 2,
                content: after_text.clone(),
            },
        );
        let report = conception_report(&before, &current, &prompts());
        let item = &report.items[0];
        assert_eq!(
            item["full_text"].as_str(),
            Some(after_text.as_str()),
            "above the limit the agent gets the whole current text"
        );
        assert_eq!(item["lines"], json!([]));
    }

    #[test]
    fn a_conception_report_lists_the_cast_and_only_changed_profiles() {
        let before = conception_snapshot(
            &[],
            &[
                CurrentCharacter {
                    id: 1,
                    rank: 1,
                    name: "沈砚".to_owned(),
                    role_label: "主角".to_owned(),
                    summary: "游侠".to_owned(),
                    profile: "old".to_owned(),
                },
                CurrentCharacter {
                    id: 2,
                    rank: 2,
                    name: "柳照".to_owned(),
                    role_label: "女主".to_owned(),
                    summary: "副使".to_owned(),
                    profile: "same".to_owned(),
                },
            ],
            &[],
        );
        let current = ConceptionState {
            characters: vec![
                CurrentCharacter {
                    id: 2,
                    rank: 1,
                    name: "柳照".to_owned(),
                    role_label: "女主".to_owned(),
                    summary: "副使".to_owned(),
                    profile: "same".to_owned(),
                },
                CurrentCharacter {
                    id: 1,
                    rank: 2,
                    name: "沈砚".to_owned(),
                    role_label: "主角".to_owned(),
                    summary: "游侠".to_owned(),
                    profile: "new profile".to_owned(),
                },
            ],
            ..ConceptionState::default()
        };
        let report = conception_report(&before, &current, &prompts());
        let item = report
            .items
            .iter()
            .find(|item| item["kind"] == json!("characters"))
            .expect("a characters item");
        let characters = item["characters"].as_array().cloned().unwrap_or_default();
        assert_eq!(characters.len(), 2, "the whole ranked list");
        assert_eq!(characters[0]["name"], "柳照");
        assert_eq!(characters[0]["rank"], 1, "the new ranking");
        assert_eq!(
            characters[0]["changed_profile"], "",
            "an unchanged profile is not repeated"
        );
        assert_eq!(characters[1]["changed_profile"], "new profile");
    }

    #[test]
    fn a_conception_report_lists_the_current_topics() {
        let before = conception_snapshot(
            &[],
            &[],
            &[CurrentTopic {
                topic: "旧主题".to_owned(),
                requirements: "".to_owned(),
                scope: "背景".to_owned(),
            }],
        );
        let current = ConceptionState {
            topics: vec![CurrentTopic {
                topic: "城门规制".to_owned(),
                requirements: "细节".to_owned(),
                scope: "背景设定".to_owned(),
            }],
            ..ConceptionState::default()
        };
        let report = conception_report(&before, &current, &prompts());
        let item = report
            .items
            .iter()
            .find(|item| item["kind"] == json!("topics"))
            .expect("a topics item");
        let topics = item["topics"].as_array().cloned().unwrap_or_default();
        assert_eq!(topics.len(), 1);
        assert_eq!(topics[0]["topic"], "城门规制");
        assert_eq!(topics[0]["scope"], "背景设定");
    }

    #[test]
    fn an_unchanged_conception_state_reports_nothing() {
        let snapshot = conception_snapshot(&[("outline", 1, "same")], &[], &[]);
        let mut current = ConceptionState::default();
        current.docs.insert(
            "outline".to_owned(),
            ConceptionDocSnapshot {
                revision: 1,
                content: "same".to_owned(),
            },
        );
        assert!(conception_report(&snapshot, &current, &prompts()).is_empty());
    }

    #[test]
    fn truncate_keeps_short_text_and_cuts_long_text() {
        assert_eq!(truncate("abc", 10), "abc");
        assert_eq!(truncate("abcdef", 3), "abc…");
        assert_eq!(truncate("中文中文", 2), "中文…");
    }
}
