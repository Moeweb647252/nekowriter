//! The Demo provider: a deterministic fake agent for manual testing without API keys
//! (spec/04 §7.2, DECISIONS D-019).
//!
//! Behaviors per session kind and phase are filled in as milestones add the tools they drive.
//! All generated text lives in `demo/corpus.toml` (non-English text is data, never code;
//! DECISIONS D-041) and is chosen by `demo_hints.language`.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::retry::LlmError;
use crate::types::{
    AssistantTurn, ChatRequest, DemoHints, ProviderPayload, RemoteModel, StopReason, StreamEvent,
    ToolCallOut, Usage,
};

/// The embedded corpus (Chinese and English samples).
const CORPUS_TOML: &str = include_str!("../demo/corpus.toml");

/// Parsed corpus. Every field is a list so behaviors can cycle through samples.
#[derive(Debug, Clone, Deserialize)]
pub struct Corpus {
    #[serde(default)]
    pub zh: LangCorpus,
    #[serde(default)]
    pub en: LangCorpus,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LangCorpus {
    #[serde(default)]
    pub sentences: Vec<String>,
    #[serde(default)]
    pub paragraphs: Vec<String>,
    #[serde(default)]
    pub outline: String,
    /// `submit_outline` reference notes (the demo writes a short list).
    #[serde(default)]
    pub reference_notes: String,
    /// The chapter title the demo proposes in `submit_outline`.
    #[serde(default)]
    pub chapter_title: String,
    /// The short ask-for-more text when a completion is withheld.
    #[serde(default)]
    pub finish_note: String,
    #[serde(default)]
    pub setting_docs: SettingDocs,
    #[serde(default)]
    pub characters: Vec<CharacterSample>,
    #[serde(default)]
    pub research_topics: Vec<TopicSample>,
    #[serde(default)]
    pub knowledge_entries: Vec<KnowledgeSample>,
    #[serde(default)]
    pub questions: Vec<QuestionSample>,
    #[serde(default)]
    pub closing_message: String,
    #[serde(default)]
    pub review_summary: String,
    #[serde(default)]
    pub review_issue: String,
    #[serde(default)]
    pub revision_plan: String,
    #[serde(default)]
    pub revision_edit: String,
    /// The reviewer's issue quote (spec/04 §7.2).
    #[serde(default)]
    pub review_quote: String,
    /// The reviewer's concrete suggestion.
    #[serde(default)]
    pub review_suggestion: String,
    /// The reviewer's consolidated feedback for the writer.
    #[serde(default)]
    pub review_feedback: String,
    /// `update_entity`'s note during maintenance (a human-readable delta).
    #[serde(default)]
    pub entity_note: String,
    #[serde(default)]
    pub timeline_description: String,
    #[serde(default)]
    pub maintenance_report: String,
    #[serde(default)]
    pub research_report: String,
    #[serde(default)]
    pub proposal_rationale: String,
    #[serde(default)]
    pub summary_template: String,
    #[serde(default)]
    pub entity_state: String,
    #[serde(default)]
    pub timeline_title: String,
    #[serde(default)]
    pub proposal_text: String,
    /// The `request_review` message of the conception flow (spec/04 §7.2).
    #[serde(default)]
    pub review_request: String,
    /// The note appended to a document the author asked to revise (spec/04 §7.2).
    #[serde(default)]
    pub doc_revision: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SettingDocs {
    #[serde(default)]
    pub outline: String,
    /// `submit_outline` reference notes (the demo writes a short list).
    #[serde(default)]
    pub reference_notes: String,
    /// The chapter title the demo proposes in `submit_outline`.
    #[serde(default)]
    pub chapter_title: String,
    /// The short ask-for-more text when a completion is withheld.
    #[serde(default)]
    pub finish_note: String,
    #[serde(default)]
    pub world: String,
    #[serde(default)]
    pub protagonist: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CharacterSample {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub role_label: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub profile: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TopicSample {
    #[serde(default)]
    pub topic: String,
    #[serde(default)]
    pub requirements: String,
    #[serde(default)]
    pub scope: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct KnowledgeSample {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub source_urls: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct QuestionSample {
    #[serde(default)]
    pub question: String,
    #[serde(default)]
    pub options: Vec<String>,
}

/// The Demo provider.
#[derive(Debug, Clone)]
pub struct DemoProvider {
    corpus: Corpus,
}

impl DemoProvider {
    pub fn new() -> Self {
        let corpus = toml::from_str(CORPUS_TOML).unwrap_or_else(|err| {
            tracing::error!(error = %err, "the demo corpus is not valid TOML");
            Corpus {
                zh: LangCorpus::default(),
                en: LangCorpus::default(),
            }
        });
        Self { corpus }
    }

    /// Selects the corpus variant for the book's writing language.
    pub fn lang(&self, hints: Option<&DemoHints>) -> &LangCorpus {
        let language = hints.map(|h| h.language.as_str()).unwrap_or("zh-CN");
        if language.to_ascii_lowercase().starts_with("zh") {
            &self.corpus.zh
        } else {
            &self.corpus.en
        }
    }

    pub async fn run(
        &self,
        request: &ChatRequest,
        on_event: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<AssistantTurn, LlmError> {
        let hints = request.demo_hints.clone().unwrap_or_default();
        let lang = self.lang(request.demo_hints.as_ref());
        let plan = choose_action(&hints, lang);

        let mut text = String::new();
        let mut calls: Vec<(String, String, serde_json::Value)> = Vec::new();
        if let Some(message) = plan.text.as_ref() {
            text.push_str(message);
        }
        calls.extend(plan.tools);

        // Stream like a real provider so ghost text and previews are visible (spec/04 §7.2).
        if !text.is_empty() {
            for chunk in crate::mock::chunk_text(&text, 30) {
                on_event(StreamEvent::TextDelta {
                    index: 0,
                    text: chunk,
                });
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            }
        }
        let mut outputs = Vec::new();
        for (index, (id, name, arguments)) in calls.iter().enumerate() {
            let input_text = serde_json::to_string(arguments).unwrap_or_else(|_| "{}".to_owned());
            on_event(StreamEvent::ToolCallStart {
                call_index: index as u32,
                id: id.clone(),
                name: name.clone(),
            });
            for chunk in crate::mock::chunk_text(&input_text, 20) {
                on_event(StreamEvent::ToolCallArgsDelta {
                    call_index: index as u32,
                    fragment: chunk,
                });
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            }
            outputs.push(ToolCallOut {
                id: id.clone(),
                name: name.clone(),
                input_text,
            });
        }

        let usage = Usage {
            input_tokens: estimate_tokens(request),
            cached_input_tokens: 0,
            cache_write_tokens: 0,
            output_tokens: estimate_tokens_text(&text),
            reasoning_tokens: 0,
            estimated: true,
        };
        on_event(StreamEvent::Usage(usage));

        let stop = if outputs.is_empty() {
            StopReason::EndTurn
        } else {
            StopReason::ToolUse
        };
        let replay_calls: Vec<serde_json::Value> = outputs
            .iter()
            .enumerate()
            .map(|(index, call)| {
                json!({
                    "index": index,
                    "id": call.id,
                    "type": "function",
                    "function": { "name": call.name, "arguments": call.input_text },
                })
            })
            .collect();

        Ok(AssistantTurn {
            text: text.clone(),
            reasoning: String::new(),
            tool_calls: outputs,
            stop_reason: stop,
            usage,
            payload: ProviderPayload::Openai {
                content: if text.is_empty() { None } else { Some(text) },
                reasoning_content: None,
                tool_calls: replay_calls,
            },
            request_id: Some("demo".to_owned()),
        })
    }

    pub async fn list_models(&self) -> Result<Vec<RemoteModel>, LlmError> {
        Ok(vec![RemoteModel {
            id: "demo-model".to_owned(),
            display_name: Some("Demo model".to_owned()),
            context_window: Some(200_000),
            max_output_tokens: Some(16_384),
        }])
    }
}

impl Default for DemoProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// What the demo decides to do for one step.
#[derive(Debug, Default)]
pub struct DemoPlan {
    pub text: Option<String>,
    pub tools: Vec<(String, String, serde_json::Value)>,
}

fn tool(name: &str, arguments: serde_json::Value) -> (String, String, serde_json::Value) {
    (format!("call_{name}"), name.to_owned(), arguments)
}

/// Decides the next step from `demo_hints` (spec/04 §7.2).
///
/// The dispatch is one arm per session kind with the phase nested inside, so later milestones
/// only add (or replace) an arm body of their own kind. Behaviors that need tools introduced by
/// a later milestone are marked with that milestone and fall back to a short text reply for
/// now.
pub fn choose_action(hints: &DemoHints, lang: &LangCorpus) -> DemoPlan {
    // The debug session of spec/10 §2.7 freezes the test-only tools. The Demo provider drives
    // them so the agent panel can be exercised end to end without any real tool: one write, one
    // echo, then the completing call.
    if hints.tools.iter().any(|tool| tool == "test_finish") {
        return debug_plan(hints, lang);
    }
    match hints.kind.as_str() {
        // M12/D-146: an outline session runs the same retrieval-then-outline script; the
        // `chapter_write`/planning arm below stays for sessions started before the split.
        "chapter_outline" => chapter_write_planning(hints, lang),
        "chapter_write" => {
            if hints.phase == "planning" {
                chapter_write_planning(hints, lang)
            } else {
                chapter_write_drafting(hints, lang)
            }
        }
        // M4 writes drafting too; M6 owns the revision step (D-077).
        "chapter_revise" => chapter_revise_plan(hints, lang),
        // M6.1: one issue on `p2` on the first attempt, a pass later (spec/04 §7.2).
        "chapter_review" => chapter_review_plan(hints, lang),
        // M6.3: canon writes, one world-setting proposal, finish_maintenance.
        "chapter_maintenance" => maintenance_plan(hints, lang),
        "chapter_resync" => resync_plan(hints, lang),
        // M9: create_entity per character design (the rank-1 one as the protagonist), then
        // finish_maintenance (spec/04 §7.2, spec/06 §7).
        "canon_seed" => seed_plan(hints, lang),
        // M8: two knowledge entries saved from the corpus, then `finish_research` (spec/04 §7.2 —
        // no network: the demo researcher works from its own material).
        "research" => research_plan(hints, lang),
        // M9: ask (2 questions) → the documents, the cast and the topics → request_review →
        // revise what was not approved → a closing message (spec/04 §7.2).
        "conception" => conception_plan(hints, lang),
        _ => text_plan(lang),
    }
}

/// The debug session's behavior: echo, one test write, then finish (spec/10 §2.7).
///
/// The steps are driven by `step_in_phase` so the same session walks the whole runtime path:
/// a plain call, a write call (which holds under `auto_pause`), and the completing call.
fn debug_plan(hints: &DemoHints, lang: &LangCorpus) -> DemoPlan {
    let sentence = lang
        .sentences
        .first()
        .cloned()
        .unwrap_or_else(|| "The first sentence.".to_owned());
    match hints.step_in_phase % 4 {
        0 => DemoPlan {
            text: Some(lang.closing_message.clone()),
            tools: vec![tool("echo", json!({ "text": sentence }))],
        },
        1 => DemoPlan {
            text: None,
            tools: vec![tool("test_write", json!({ "text": sentence }))],
        },
        _ => DemoPlan {
            text: None,
            tools: vec![tool("test_finish", json!({ "text": lang.closing_message }))],
        },
    }
}

/// Planning step: look the protagonist up, then submit a three-scene outline (spec/04 §7.2).
///
/// Step 0 is the `get_entity` call, but only when the book has a protagonist to look up; step 1
/// submits the outline. The demo never writes prose while planning.
fn chapter_write_planning(hints: &DemoHints, lang: &LangCorpus) -> DemoPlan {
    if hints.step_in_phase == 0
        && let Some(protagonist) = &hints.protagonist
        && !protagonist.is_empty()
    {
        return DemoPlan {
            tools: vec![tool("get_entity", json!({ "entity": protagonist }))],
            ..DemoPlan::default()
        };
    }
    let mut arguments = json!({
        "outline": lang.outline,
        "reference_notes": lang.reference_notes,
    });
    if !lang.chapter_title.is_empty() {
        arguments["title"] = json!(lang.chapter_title);
    }
    DemoPlan {
        tools: vec![tool("submit_outline", arguments)],
        ..DemoPlan::default()
    }
}

/// The drafting step: write one unit matching the granularity, complete after ~8 paragraphs, and
/// confirm a withheld short completion with `finish_draft` (spec/04 §7.2).
///
/// `step_in_phase` counts the steps of the whole session, and drafting begins after the outline
/// step, so the drafting index is `step_in_phase - 1`. The demo cycles the corpus so a long
/// chapter keeps producing fresh text.
///
/// The chapter is completed once the demo has written [`DRAFTING_STEPS_BEFORE_COMPLETION`] units:
/// the completing call sets `chapter_end`, or `finish_draft` is used when the granularity's
/// completion is a separate tool call. A withheld short completion (spec/03 §8, D-029) then costs
/// one extra `finish_draft`, which the next step provides.
fn chapter_write_drafting(hints: &DemoHints, lang: &LangCorpus) -> DemoPlan {
    let index = hints.step_in_phase.saturating_sub(1) as usize;
    let paragraph = index % lang.paragraphs.len().max(1);
    let sentence = index % lang.sentences.len().max(1);
    let target = lang.paragraphs.get(paragraph).cloned().unwrap_or_default();
    let granularity = hints.granularity.as_deref().unwrap_or("paragraph");

    // The last drafting unit asks to complete the chapter (`chapter_end`), and the step after it
    // confirms with `finish_draft` in case that completion was withheld as short (spec/03 §8,
    // spec/04 §7.2). `chapter` granularity completes through its own single call, so it never
    // needs the extra step.
    let completing_unit = index + 1 == DRAFTING_STEPS_BEFORE_COMPLETION;
    if index >= DRAFTING_STEPS_BEFORE_COMPLETION && granularity != "chapter" {
        // The first `finish_draft` may be withheld as short; the next one confirms it.
        return DemoPlan {
            text: Some(lang.finish_note.clone()),
            tools: vec![tool("finish_draft", json!({}))],
        };
    }

    match granularity {
        "sentence" => {
            // A sentence is its own segment; `paragraph_end` closes it every few sentences so the
            // chapter has several paragraphs rather than one long one.
            let text = lang.sentences.get(sentence).cloned().unwrap_or_default();
            let paragraph_end = index % 3 == 2;
            let mut arguments = json!({ "text": text, "paragraph_end": paragraph_end });
            if completing_unit {
                arguments["chapter_end"] = json!(true);
            }
            DemoPlan {
                tools: vec![tool("insert_text", arguments)],
                ..DemoPlan::default()
            }
        }
        "paragraph" => {
            let mut arguments = json!({ "text": target });
            if completing_unit {
                arguments["chapter_end"] = json!(true);
            }
            DemoPlan {
                tools: vec![tool("insert_text", arguments)],
                ..DemoPlan::default()
            }
        }
        "free" => {
            // Two paragraphs per call, with the tail closed so the next call starts a new one.
            let second = lang
                .paragraphs
                .get((paragraph + 1) % lang.paragraphs.len().max(1))
                .cloned()
                .unwrap_or_default();
            let text = format!("{target}\n\n{second}");
            let mut arguments = json!({ "text": text, "paragraph_end": true });
            if completing_unit {
                arguments["chapter_end"] = json!(true);
            }
            DemoPlan {
                tools: vec![tool("insert_text", arguments)],
                ..DemoPlan::default()
            }
        }
        // `chapter`: one call writes the rest of the chapter and completes it (`chapter_end` is
        // implied by the granularity, spec/03 §8).
        _ => {
            let mut paragraphs: Vec<String> = Vec::new();
            for offset in 0..MIN_CHAPTER_PARAGRAPHS {
                let position = (paragraph + offset) % lang.paragraphs.len().max(1);
                if let Some(text) = lang.paragraphs.get(position) {
                    paragraphs.push(text.clone());
                }
            }
            DemoPlan {
                tools: vec![tool(
                    "insert_text",
                    json!({ "text": paragraphs.join("\n\n") }),
                )],
                ..DemoPlan::default()
            }
        }
    }
}

/// How many drafting units the demo writes before it completes the chapter, so the manual check
/// sees a finished chapter instead of an endless stream (spec/04 §7.2 "chapter_end after ~8
/// paragraphs"). Counted in calls, not paragraphs: `free` writes two paragraphs per call.
const DRAFTING_STEPS_BEFORE_COMPLETION: usize = 8;

/// How many paragraphs the `chapter` granularity writes in one call (spec/04 §7.2 "~8").
const MIN_CHAPTER_PARAGRAPHS: usize = 8;

/// Revision planning: submit the plan; thus revising: one `replace_text` and then finish
/// (spec/04 §7.2). The `draft` hint carries the current paragraphs, so the `old` text is the
/// first paragraph and is guaranteed to occur exactly once (D-077).
fn chapter_revise_plan(hints: &DemoHints, lang: &LangCorpus) -> DemoPlan {
    if hints.phase == "revision_planning" {
        return DemoPlan {
            tools: vec![tool(
                "submit_revision_plan",
                json!({ "plan": lang.revision_plan }),
            )],
            ..DemoPlan::default()
        };
    }
    // The first revising step rewrites one paragraph; every later step finishes. A withheld
    // completion does not exist for revisions, so one finish is enough.
    let step = hints.step_in_phase;
    if step <= 1
        && let Some(paragraph) = hints.draft.first()
        && !paragraph.text.is_empty()
        && !lang.revision_edit.is_empty()
    {
        return DemoPlan {
            tools: vec![tool(
                "replace_text",
                json!({ "pid": paragraph.pid, "old": paragraph.text, "new": lang.revision_edit }),
            )],
            ..DemoPlan::default()
        };
    }
    DemoPlan {
        tools: vec![tool(
            "finish_revision",
            json!({ "summary": lang.revision_plan }),
        )],
        ..DemoPlan::default()
    }
}

/// The reviewer: `revise` with one issue on `p2` on the first attempt, `pass` afterwards
/// (spec/04 §7.2). `p2` may not exist in a short chapter; `submit_review` then drops the pid
/// from the issue with a note (spec/05 §5).
fn chapter_review_plan(hints: &DemoHints, lang: &LangCorpus) -> DemoPlan {
    let pass = hints.review_attempt > 1;
    let summary = if lang.review_summary.is_empty() {
        lang.closing_message.clone()
    } else {
        lang.review_summary.clone()
    };
    let feedback = if lang.review_feedback.is_empty() {
        summary.clone()
    } else {
        lang.review_feedback.clone()
    };
    let issues = if pass {
        Vec::new()
    } else {
        vec![json!({
            "severity": "minor",
            "category": "prose",
            "pids": ["p2"],
            "quote": lang.review_quote,
            "problem": lang.review_issue,
            "suggestion": lang.review_suggestion,
        })]
    };
    DemoPlan {
        tools: vec![tool(
            "submit_review",
            json!({
                "verdict": if pass { "pass" } else { "revise" },
                "summary": summary,
                "issues": issues,
                "feedback": feedback,
            }),
        )],
        ..DemoPlan::default()
    }
}

/// The archivist after approval (spec/04 §7.2, spec/06 §6): appearances → protagonist state →
/// timeline → summary → one world-setting proposal → `finish_maintenance`.
///
/// `step_in_phase` walks the sequence. When the book has no protagonist yet (a book created
/// without conception and seeding), the archivist first creates the character entity it will
/// record, so the required steps exist without a key. In an isolated track no setting proposal
/// is made (spec/06 §6 step 9) and the shared protagonist's changes would be a track-local note;
/// the demo skips the proposal only.
/// The demo researcher (spec/04 §7.2): save the corpus's two knowledge entries, then report.
///
/// The corpus entries carry `source_urls`, so `save_knowledge_entry`'s citation rule is exercised
/// without touching the network; a run with a task that already has entries skips straight to the
/// report, so a resumed session finishes instead of duplicating work.
fn research_plan(hints: &DemoHints, lang: &LangCorpus) -> DemoPlan {
    let saved = hints.saved_entries as usize;
    let entries: Vec<&KnowledgeSample> = lang
        .knowledge_entries
        .iter()
        .skip(saved)
        .take(2 - saved.min(2))
        .collect();
    if let Some(entry) = entries.first() {
        let arguments = json!({
            "title": entry.title,
            "content": entry.content,
            "source_urls": entry.source_urls,
        });
        return DemoPlan {
            tools: vec![tool("save_knowledge_entry", arguments)],
            ..DemoPlan::default()
        };
    }
    DemoPlan {
        tools: vec![tool(
            "finish_research",
            json!({ "report": lang.research_report }),
        )],
        ..DemoPlan::default()
    }
}

fn maintenance_plan(hints: &DemoHints, lang: &LangCorpus) -> DemoPlan {
    let target = hints
        .protagonist
        .clone()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| {
            lang.characters
                .first()
                .map(|character| character.name.clone())
                .unwrap_or_default()
        });
    let frozen = |name: &str| hints.tools.is_empty() || hints.tools.iter().any(|tool| tool == name);
    if target.is_empty() {
        // No corpus character to anchor the canon steps: finish directly.
        return DemoPlan {
            tools: vec![tool(
                "finish_maintenance",
                json!({ "report": lang.maintenance_report }),
            )],
            ..DemoPlan::default()
        };
    }
    let has_protagonist = hints
        .protagonist
        .as_deref()
        .map(|name| !name.is_empty())
        .unwrap_or(false);
    let step = hints.step_in_phase as usize;
    // Step 0 creates the entity when the book has no protagonist yet and the Entities tool is
    // frozen (no canon_seed in M6).
    let create_first = !has_protagonist && frozen("create_entity");
    let offset = usize::from(create_first);
    if create_first && step == 0 {
        let first = lang.characters.first().cloned().unwrap_or_default();
        let mut arguments = json!({
            "kind": "character",
            "name": target,
            "summary": first.summary,
            "state": lang.entity_state,
        });
        if !first.role_label.is_empty() {
            arguments["attributes"] = json!({ "role": first.role_label });
        }
        return DemoPlan {
            tools: vec![tool("create_entity", arguments)],
            ..DemoPlan::default()
        };
    }
    if step < offset {
        return DemoPlan::default();
    }
    let stage = step - offset;
    let report = lang.maintenance_report.clone();
    // One entry per possible maintenance step; a disabled system's tool is skipped, so the demo
    // never calls a tool that was not frozen into the session (plan §4.6).
    let mut steps: Vec<(&str, Value)> = vec![
        (
            "record_appearances",
            json!({ "appearances": [ { "entity": target, "role": "pov" } ] }),
        ),
        (
            "update_entity",
            json!({
                "entity": target,
                "state": lang.entity_state,
                "note": lang.entity_note,
            }),
        ),
        (
            "add_timeline_event",
            json!({
                "title": lang.timeline_title,
                "description": lang.timeline_description,
            }),
        ),
        (
            "set_chapter_summary",
            json!({ "summary": lang.summary_template }),
        ),
    ];
    if hints.canon_mode != "isolated" {
        steps.push((
            "propose_setting_change",
            json!({
                "target": "world",
                "append": lang.proposal_text,
                "rationale": lang.proposal_rationale,
            }),
        ));
    }
    steps.push(("finish_maintenance", json!({ "report": report })));
    let steps: Vec<(&str, Value)> = steps.into_iter().filter(|(name, _)| frozen(name)).collect();
    let (name, arguments) = steps
        .get(stage.min(steps.len().saturating_sub(1)))
        .cloned()
        .unwrap_or((
            "finish_maintenance",
            json!({ "report": lang.maintenance_report }),
        ));
    DemoPlan {
        tools: vec![tool(name, arguments)],
        ..DemoPlan::default()
    }
}

/// The archivist after an author edit of official text (spec/06 §8): re-record appearances and
/// the summary, then finish. The kickoff carries only the diff.
fn resync_plan(hints: &DemoHints, lang: &LangCorpus) -> DemoPlan {
    let target = hints
        .protagonist
        .clone()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| {
            lang.characters
                .first()
                .map(|character| character.name.clone())
                .unwrap_or_default()
        });
    let report = lang.maintenance_report.clone();
    let frozen = |name: &str| hints.tools.is_empty() || hints.tools.iter().any(|tool| tool == name);
    if target.is_empty() {
        return DemoPlan {
            tools: vec![tool("finish_maintenance", json!({ "report": report }))],
            ..DemoPlan::default()
        };
    }
    let mut steps: Vec<(&str, Value)> = Vec::new();
    if frozen("record_appearances") {
        steps.push((
            "record_appearances",
            json!({ "appearances": [ { "entity": target, "role": "pov" } ] }),
        ));
    }
    if frozen("set_chapter_summary") {
        steps.push((
            "set_chapter_summary",
            json!({ "summary": lang.summary_template }),
        ));
    }
    steps.push(("finish_maintenance", json!({ "report": report })));
    let step = hints.step_in_phase as usize;
    let (name, arguments) = steps
        .get(step.min(steps.len().saturating_sub(1)))
        .cloned()
        .unwrap_or((
            "finish_maintenance",
            json!({ "report": lang.maintenance_report }),
        ));
    DemoPlan {
        tools: vec![tool(name, arguments)],
        ..DemoPlan::default()
    }
}

/// The conception behavior of spec/04 §7.2, driven by `step_in_phase` and the approvals.
///
/// 0. `ask_user` with two corpus questions (the answers arrive through the answer API);
/// 1. the three setting documents, two character designs and the research topics — one assistant
///    turn with six calls, so a manual check sees the studio fill up at once;
/// 2. `request_review`;
/// 3. and later: revise every document whose revision is not approved (rewrite it with the corpus text and
///    the corpus's revision note), then send one closing text message. When everything was
///    approved the closing message comes immediately.
fn conception_plan(hints: &DemoHints, lang: &LangCorpus) -> DemoPlan {
    match hints.step_in_phase {
        0 => DemoPlan {
            tools: vec![tool(
                "ask_user",
                json!({
                    "questions": lang.questions.iter().enumerate().map(|(index, question)| {
                        json!({
                            "id": format!("q{}", index + 1),
                            "question": question.question,
                            "options": question.options.iter().map(|option| json!({ "label": option })).collect::<Vec<_>>(),
                        })
                    }).collect::<Vec<_>>(),
                }),
            )],
            ..DemoPlan::default()
        },
        1 => DemoPlan {
            tools: vec![
                tool(
                    "write_setting_doc",
                    json!({ "doc": "outline", "content": lang.setting_docs.outline }),
                ),
                tool(
                    "write_setting_doc",
                    json!({ "doc": "world", "content": lang.setting_docs.world }),
                ),
                tool(
                    "write_setting_doc",
                    json!({ "doc": "protagonist", "content": lang.setting_docs.protagonist }),
                ),
                tool("upsert_character", character_argument(lang, 0, 1)),
                tool("upsert_character", character_argument(lang, 1, 2)),
                tool(
                    "set_research_topics",
                    json!({
                        "topics": lang.research_topics.iter().map(|topic| json!({
                            "topic": topic.topic,
                            "requirements": topic.requirements,
                            "scope": topic.scope,
                        })).collect::<Vec<_>>(),
                    }),
                ),
            ],
            ..DemoPlan::default()
        },
        2 => DemoPlan {
            text: Some(lang.review_request.clone()),
            tools: vec![tool(
                "request_review",
                json!({ "message": lang.review_request }),
            )],
        },
        _ => {
            // Revise the documents the author did not approve (one rewrite each, carrying the
            // corpus's revision note); otherwise the closing message.
            let unapproved: Vec<&str> = ["outline", "world", "protagonist"]
                .into_iter()
                .filter(|kind| !hints.approved_docs.iter().any(|approved| approved == kind))
                .collect();
            if hints.step_in_phase >= 4 || unapproved.is_empty() {
                return text_plan(lang);
            }
            let mut tools = Vec::new();
            for kind in unapproved {
                let base = match kind {
                    "world" => &lang.setting_docs.world,
                    "protagonist" => &lang.setting_docs.protagonist,
                    _ => &lang.setting_docs.outline,
                };
                tools.push(tool(
                    "write_setting_doc",
                    json!({
                        "doc": kind,
                        "content": format!("{base}\n\n{}", lang.doc_revision),
                    }),
                ));
            }
            DemoPlan {
                tools,
                ..DemoPlan::default()
            }
        }
    }
}

/// One `upsert_character` call from the corpus (the designs the book needs).
fn character_argument(lang: &LangCorpus, index: usize, rank: i32) -> serde_json::Value {
    let character = lang.characters.get(index).cloned().unwrap_or_default();
    json!({
        "name": character.name,
        "role_label": character.role_label,
        "summary": character.summary,
        "profile": character.profile,
        "rank": rank,
    })
}

/// The canon-seed behavior of spec/04 §7.2: one `create_entity` per character design (the rank-1
/// design with `is_protagonist`), then `finish_maintenance`.
///
/// The character designs travel in `demo_hints.characters`; `step_in_phase` walks them.
fn seed_plan(hints: &DemoHints, lang: &LangCorpus) -> DemoPlan {
    if !(hints.tools.is_empty() || hints.tools.iter().any(|tool| tool == "create_entity")) {
        return DemoPlan {
            tools: vec![tool(
                "finish_maintenance",
                json!({ "report": lang.maintenance_report }),
            )],
            ..DemoPlan::default()
        };
    }
    let step = hints.step_in_phase as usize;
    if step < hints.characters.len() {
        let character = &hints.characters[step];
        let mut arguments = json!({
            "kind": "character",
            "name": character.name,
            "summary": character.summary,
            "description": character.profile,
        });
        if step == 0 {
            arguments["is_protagonist"] = json!(true);
            // Rank 1 is the most important (spec/06 §7: importance from rank).
            arguments["importance"] = json!(5);
        }
        if !character.role_label.is_empty() {
            arguments["attributes"] = json!({ "role": character.role_label });
        }
        return DemoPlan {
            tools: vec![tool("create_entity", arguments)],
            ..DemoPlan::default()
        };
    }
    DemoPlan {
        tools: vec![tool(
            "finish_maintenance",
            json!({ "report": lang.maintenance_report }),
        )],
        ..DemoPlan::default()
    }
}

/// A plain text reply: the demo's closing message in the book's language.
fn text_plan(lang: &LangCorpus) -> DemoPlan {
    DemoPlan {
        text: Some(lang.closing_message.clone()),
        ..DemoPlan::default()
    }
}

/// Rough token estimate for usage accounting (CJK characters × 1.0 + other characters ÷ 4).
pub fn estimate_tokens(request: &ChatRequest) -> i64 {
    let mut cjk = 0i64;
    let mut other = 0i64;
    let mut account = |text: &str| {
        for ch in text.chars() {
            if is_cjk(ch) {
                cjk += 1;
            } else {
                other += 1;
            }
        }
    };
    account(&request.system);
    for message in &request.messages {
        match message {
            crate::types::Message::User { blocks } => {
                for block in blocks {
                    account(&block.text);
                }
            }
            crate::types::Message::Assistant { .. } => {}
            crate::types::Message::ToolResults { results } => {
                for result in results {
                    account(&result.content);
                }
            }
        }
    }
    cjk + (other + 3) / 4
}

fn estimate_tokens_text(text: &str) -> i64 {
    let mut cjk = 0i64;
    let mut other = 0i64;
    for ch in text.chars() {
        if is_cjk(ch) {
            cjk += 1;
        } else {
            other += 1;
        }
    }
    cjk + (other + 3) / 4
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
}
