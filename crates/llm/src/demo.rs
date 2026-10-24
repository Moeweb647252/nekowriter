//! The Demo provider: a deterministic fake agent for manual testing without API keys
//! (spec/04 §7.2, DECISIONS D-019).
//!
//! Behaviors per session kind and phase are filled in as milestones add the tools they drive.
//! All generated text lives in `demo/corpus.toml` (non-English text is data, never code;
//! DECISIONS D-041) and is chosen by `demo_hints.language`.

use serde::Deserialize;
use serde_json::json;

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
    #[serde(default)]
    pub summary_template: String,
    #[serde(default)]
    pub entity_state: String,
    #[serde(default)]
    pub timeline_title: String,
    #[serde(default)]
    pub proposal_text: String,
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
        "chapter_write" => {
            if hints.phase == "planning" {
                chapter_write_planning(lang)
            } else {
                chapter_write_drafting(hints, lang)
            }
        }
        // M4 writes drafting too; a revision plan and a revision arrive with M6.
        "chapter_revise" => chapter_revise_plan(hints, lang),
        // M6: submit_review with one issue on `p2`.
        "chapter_review" => text_plan(lang),
        // M6: canon writes, one world-setting proposal, finish_maintenance.
        "chapter_maintenance" | "chapter_resync" => text_plan(lang),
        // M9: create_entity per character design, then finish_maintenance.
        "canon_seed" => text_plan(lang),
        // M9 (conception) and M8 (research) arrive with their own milestones.
        "conception" | "research" => text_plan(lang),
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

/// Planning step: submit a three-scene outline (spec/04 §7.2). M5 adds the leading `get_entity`
/// call for the protagonist.
fn chapter_write_planning(lang: &LangCorpus) -> DemoPlan {
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

/// Revision planning: submit the plan (spec/04 §7.2 `chapter_revise`).
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
    // Revising: finish. M6 owns the revise flow end to end, and it is the milestone that can do a
    // `replace_text` safely: the demo provider has no access to the chapter text, so a made-up
    // `old` would always be a `text_not_found` error (spec/04 §7.2's "one replace_text on p1"
    // belongs with the flow that reads the draft first).
    DemoPlan {
        tools: vec![tool(
            "finish_revision",
            json!({ "summary": lang.revision_plan }),
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
