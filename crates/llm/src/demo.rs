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
                // Drafting, revision planning and revising arrive with M4 (writer tools).
                text_plan(lang)
            }
        }
        // M4: submit_revision_plan → replace_text → finish_revision.
        "chapter_revise" => text_plan(lang),
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

/// Planning step: submit a three-scene outline. M5 adds the leading `get_entity` call for the
/// protagonist.
fn chapter_write_planning(lang: &LangCorpus) -> DemoPlan {
    DemoPlan {
        tools: vec![tool(
            "submit_outline",
            json!({ "outline": lang.outline, "reference_notes": "" }),
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
