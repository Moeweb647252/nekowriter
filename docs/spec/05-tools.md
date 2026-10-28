# 05 — Tools

## 1. Framework

### 1.1 Sources of truth
- **Model-facing definitions** (name, Chinese description, JSON Schema with Chinese parameter
  descriptions): `prompts/tools.toml`. Each `[tools.<name>]` has `description` and `input_schema`.
- **Model-facing results and errors** (Chinese): templates in `prompts/results.toml`. In this
  document, `error.<code>` means `result.error` rendered with `code = "<code>"` (the code list
  is in spec/09 §2). Code contains no Chinese text.
- **Behavior**: this document and the Rust implementation in `crates/server/src/agent/tools/`.

### 1.2 Rust shape

```rust
#[async_trait::async_trait]                       // or a boxed-future helper; tools are stored as Box<dyn ToolHandler>
pub trait ToolHandler: Send + Sync {
    fn name(&self) -> &'static str;
    fn kind(&self) -> ToolKind;                   // Retrieval | Draft | Canon | Interactive | Web | Knowledge | Control
    fn preview_field(&self) -> Option<&'static str>;   // spec/03 §12
    /// Slow, read-only work outside any transaction: web requests, query embeddings.
    /// Cancellable (the runner races it against the stop token). Default: nothing to prepare.
    async fn prepare(&self, ctx: &PrepareCtx<'_>, input: &serde_json::Value) -> Result<Prepared, ToolError> {
        Ok(Prepared::None)
    }
    /// Atomic work inside the call's transaction: database reads and writes only, no network.
    async fn execute(&self, ctx: &mut ToolCtx<'_>, input: serde_json::Value, prepared: Prepared)
        -> Result<ToolOutcome, ToolError>;
}
// Each handler deserializes `input` into its own `#[derive(Deserialize)] #[serde(deny_unknown_fields)]` struct.
// Prepared::{None, QueryVector(Vec<f32>), WebSearch(Vec<SearchHit>), WebPage(FetchedPage)}

pub struct PrepareCtx<'a> {                       // a pooled connection, no transaction
    pub db: &'a DatabaseConnection,
    pub session: &'a agent_sessions::Model,
    pub services: &'a Services,                   // embedder, web clients, page cache
}

pub struct ToolCtx<'a> {
    pub tx: &'a DatabaseTransaction,
    pub session: &'a agent_sessions::Model,
    pub book: Option<&'a books::Model>,
    pub chapter: Option<&'a chapters::Model>,
    pub track: Option<&'a tracks::Model>,
    pub visibility: Visibility,                   // spec/06 §2
    pub request_meta: RequestMeta,                // phase + granularity when the call was generated
    pub prompts: &'a PromptSet,                   // resolved for this book
    pub services: &'a Services,
    pub events: Vec<ServerEvent>,                 // emitted after commit
}

pub struct ToolOutcome {
    pub text: String,                             // rendered from result.* templates
    pub is_error: bool,
    pub ui: serde_json::Value,                    // ToolUi (structured, for the frontend)
    pub effects: ToolEffects,                     // deferred (spec/03 §5.4)
    pub await_input: Option<AwaitKind>,           // AskUser | RequestReview
}
```

`ToolError` is for user-correctable problems (not found, validation, granularity, phase) and
becomes an `is_error` outcome rendered with an `error.*` template. Internal failures (database
errors) roll the transaction back; a second, minimal transaction then records the call as
`error.internal`, and the session continues; the logs keep the details.

The split keeps the rule "never hold a transaction across a network request" (spec/01 §7):
`web_search` and `web_fetch` do their requests in `prepare`; the search tools embed the query
there (spec/07 §5). Every other tool only implements `execute`.

### 1.3 Definition rendering
At session creation, for each tool in the role's list (§2): take the definition from
`tools.toml`, replace its top-level `description` with the resolved prompt key `tool.<name>`
(book override → global override → shipped default), and freeze the result into
`agent_sessions.tools`. Parameter schemas and their descriptions are shipped, not overridable
(they are part of the code contract). A unit test checks every tool's `examples` (also in
`tools.toml`) against both the JSON Schema (`jsonschema` validation) and the Rust input struct.

### 1.4 Input handling
1. Strict JSON parse of the raw argument text. On failure: `error.invalid_json` (includes the
   first 500 characters of the raw text).
2. JSON Schema validation (compiled once per tool). On failure: `error.validation` with the
   first three errors (path + message). These messages come from the validator library and are
   English; like the raw arguments shown by `error.invalid_json`, they are data, not wording.
3. Deserialize into the typed struct, then semantic validation in the handler. Unique names
   are checked there for creates and renames alike, so a collision never reaches a database
   constraint (which would surface as `error.internal`): the name or an alias of another
   visible entity → `error.entity_exists`, another character design's name →
   `error.character_exists`, another entry's title in the scope → `error.entry_exists`.

### 1.5 Results
- Results are short and information-dense. Never echo back long text the model just wrote
  (an insertion result reports pids and counts, not the text).
- Default cap of 12,000 characters per result; tools with a `max_chars` parameter
  (`read_chapter`, `read_draft`, `web_fetch`) use that value instead. Longer results are cut at a
  paragraph or line boundary and end with `result.truncated` (offset hint) or the tool's own
  continuation hint.
- Every reference to a chapter is rendered as its label (`第12章《标题》`, side tracks
  `番外《轨道名》第2章`) via the `chapter_label` helper.
- Entities are rendered as `名字(#id)`.
- `result.writer_control` also receives `staged` (boolean). For `submit_outline` in a
  `chapter_outline` session it is true and the result says the outline was submitted and takes
  effect when the author accepts it — never that it was saved. For a legacy `chapter_write` /
  `planning` session it is false and the result describes the immediate save.
- Errors that the tables below do not name: an unknown or invisible id (`event_id`,
  `after_event`, `before_event`, `around_event`, `move_after`, `move_before`, `note_id`,
  `foreshadowing_id`, `entry_id`) → `error.not_found` with the `target_type` label as
  `what_label`; an unknown pid (`pid`, `pids`, `after`, `from_pid`, `to_pid`) →
  `error.pid_not_found` listing them, with the tool's name (only writers have `read_draft`);
  `finish_draft` on an empty draft → `error.draft_empty`; `end_relation` without that active
  relation → `error.relation_not_found`; `reorder_characters` with a list that is not a
  permutation of all designs → `error.reorder_mismatch` with the current names;
  `remove_character` with an unknown name → `error.character_not_found`; a failed `web_search`
  or `web_fetch` → `error.search_failed` (spec/07 §6) or `error.fetch_failed` (spec/07 §7).
- Result template per tool:

  | Template | Tools |
  |---|---|
  | `result.entity_card` | `get_entity` |
  | `result.entity_list` | `list_entities` |
  | `result.relations` | `get_relations` |
  | `result.timeline` | `query_timeline` |
  | `result.foreshadowing_list` | `list_foreshadowing` |
  | `result.search_hits` (`corpus` = `canon` / `knowledge` / `chapters`) | `search_canon`, `search_knowledge`, `search_chapters` |
  | `result.knowledge_entry` | `read_knowledge_entry` |
  | `result.chapter_text` | `read_chapter` |
  | `result.draft_text` | `read_draft` |
  | `result.draft_edit` (`action` = tool) | DRAFT_EDIT tools |
  | `result.writer_control` (`action` = tool) | `submit_outline`, `submit_revision_plan`, `finish_draft`, `finish_revision` |
  | `result.submit_review` | `submit_review` |
  | `result.canon_write` (`action` = tool) | archivist write tools except `finish_maintenance` |
  | `result.finish_maintenance` | `finish_maintenance` |
  | `result.ask_user` / `result.request_review` | the answers to `ask_user` / `request_review` |
  | `result.conception_write` (`action` = tool) | `write_setting_doc`, `edit_setting_doc`, `upsert_character`, `remove_character`, `reorder_characters`, `set_research_topics` |
  | `result.web_search`, `result.web_fetch` (each wrapped in `notice.web_content`) | `web_search`, `web_fetch` |
  | `result.knowledge_write` / `result.finish_research` | `save_knowledge_entry`, `update_knowledge_entry` / `finish_research` |
  | `result.error` | every error |

### 1.6 `ToolUi` payloads
Structured data for the agent panel. The frontend builds a localized one-line summary from
`name + ui` (i18n keys `agent:tools.<name>.summary`). Payloads carry data only (ids, numbers,
enum values, and names or titles as stored), never labels rendered by the prompt pack, so the
UI can localize them. The shapes are listed per tool below. Unknown fields must be ignored by
the frontend.

### 1.7 Entity references
Parameters named `entity`, `source`, `target`, `other` (strings, also nested ones such as
`appearances[].entity`) and `entities` (string arrays) of the retrieval and canon tools are
resolved against **visible** entities (`propose_setting_change.target` is not an entity: it
names a setting document) in this order: `#12` or `12` → id; exact name; exact
alias; case-insensitive name or alias; unique substring of a name or alias. No match →
`error.entity_not_found` with up to 5 suggestions. Several matches → `error.entity_ambiguous`
listing the candidates with ids.

## 2. Tool lists per role (fixed order) and phase permissions

| Role | Tools (this exact order) |
|---|---|
| planner | `get_entity`, `list_entities`, `get_relations`, `query_timeline`, `list_foreshadowing`, `search_canon`, `search_knowledge`, `read_knowledge_entry`, `search_chapters`, `read_chapter`, `read_draft`, `submit_outline` |
| writer | `get_entity`, `list_entities`, `get_relations`, `query_timeline`, `list_foreshadowing`, `search_canon`, `search_knowledge`, `read_knowledge_entry`, `search_chapters`, `read_chapter`, `submit_revision_plan`, `read_draft`, `insert_text`, `insert_paragraphs`, `replace_text`, `rewrite_paragraph`, `delete_paragraphs`, `finish_draft`, `finish_revision` |
| reviewer | `get_entity`, `list_entities`, `get_relations`, `query_timeline`, `list_foreshadowing`, `search_canon`, `search_knowledge`, `read_knowledge_entry`, `search_chapters`, `read_chapter`, `submit_review` |
| archivist | `get_entity`, `list_entities`, `get_relations`, `query_timeline`, `list_foreshadowing`, `search_canon`, `read_chapter`, `create_entity`, `update_entity`, `set_relation`, `end_relation`, `record_appearances`, `add_timeline_event`, `update_timeline_event`, `add_memory_note`, `retire_memory_note`, `add_foreshadowing`, `resolve_foreshadowing`, `set_chapter_summary`, `propose_setting_change`, `finish_maintenance` |
| conception | `ask_user`, `web_search`, `web_fetch`, `write_setting_doc`, `edit_setting_doc`, `upsert_character`, `remove_character`, `reorder_characters`, `set_research_topics`, `request_review` |
| researcher | `web_search`, `web_fetch`, `search_knowledge`, `read_knowledge_entry`, `save_knowledge_entry`, `update_knowledge_entry`, `finish_research` |

**RETRIEVAL** = the first ten writer tools (`get_entity` … `read_chapter`).
**DRAFT_EDIT** = `insert_text`, `insert_paragraphs`, `replace_text`, `rewrite_paragraph`,
`delete_paragraphs`.

| Writer phase | Permitted |
|---|---|
| `planning` (legacy only) | RETRIEVAL, `read_draft`, `submit_outline` |
| `drafting` | `read_draft`, DRAFT_EDIT, `finish_draft` |
| `revision_planning` | RETRIEVAL, `read_draft`, `submit_revision_plan` |
| `revising` | `read_draft`, DRAFT_EDIT, `finish_revision` |

A `chapter_outline` session is **phase-free**: its frozen planner list is its gate, so it permits
all twelve tools (retrieval, `read_draft`, `submit_outline`). The writer list has no
`submit_outline`; only a legacy `chapter_write` session (frozen before the split) still has it.

Archivist by session kind: `canon_seed` forbids `record_appearances`, `set_chapter_summary`,
`read_chapter`; the other kinds allow all of their tools. Reviewer, conception, planner, and
researcher sessions allow all of their tools.

## 3. Retrieval tools

| Tool | Parameters | Behavior and result | `ui` |
|---|---|---|---|
| `get_entity` | `entity`* | Entity card (`result.entity_card`): kind, name, aliases, importance, status, summary, description, **state**, attributes, relations in both directions phrased from this entity ("林凡(#12) 是 张三(#15) 的 师兄"), the latest 10 change notes with chapter labels, appearances (first, last, count in the mid-term window), and the linked character design's role and summary. In side tracks with an anchor, notes from chapters after the anchor are marked `（锚点之后）`, and so is the state when its newest change came from such a chapter (spec/06 §4.2) | `{entity_id, name, kind}` |
| `list_entities` | `kind`, `query`, `status`, `limit` (30, max 100) | One line per entity: `#id 【类型】名字（别名…）— 简介 [状态]`, sorted by importance desc then last appearance desc | `{count}` |
| `get_relations` | `entity`*, `other`, `include_ended` (false) | Relation lines with descriptions and since/until chapter labels | `{entity_id, count}` |
| `query_timeline` | `entity`, `query`, `around_event` (id), `from_chapter`, `to_chapter` (numbers in the main track), `limit` (20, max 50), `order` (`chronological` \| `recent_first`) | Ordered events: `[#id] (story_time) 标题 — 描述 · 第N章 · 涉及：A、B`. With `around_event`: the 5 events before and after it. Anchor marking as above | `{count}` |
| `list_foreshadowing` | `status` (`open`), `entity`, `limit` (20, max 50) | `[#id] 标题 — 描述 · 埋设于第N章 · 计划回收：…` | `{count}` |
| `search_canon` | `query`*, `kinds` (subset of `entity`,`note`,`foreshadowing`,`event`,`summary`), `limit` (8, max 20) | Hybrid search (spec/07 §5) over visible search docs except passages: `【类型】标题(#ref) — snippet` | `{query, count}` |
| `search_knowledge` | `query`*, `scope` (name), `limit` (6, max 15) | Hybrid search over the book's visible scopes (researcher: its target scope only), or over the one named by `scope`, which must be one of them (`error.scope_not_found`, listing the visible names). `[条目#id]《标题》（范围）— snippet ≤400 chars`. No visible scope → `error.no_knowledge_scope` | `{query, count}` |
| `read_knowledge_entry` | `entry_id`*, `offset` (0) | Title, scope, sources, content from `offset`, ≤ 6,000 characters, plus a continuation hint. The scope must be visible | `{entry_id, title}` |
| `search_chapters` | `query`*, `limit` (6, max 15) | Hybrid search over official-chapter passages in visible tracks: `第N章《标题》 p12–p15：snippet`. Main-story passages after a side story's anchor are marked | `{query, count}` |
| `read_chapter` | `chapter`* (`"12"` = number in the session's track, `"main 12"` = main track, `"#345"` = id), `from_pid`, `max_chars` (8,000, max 20,000) | Official text with pids. Non-official chapters → `error.chapter_not_official`; chapters of other isolated tracks are not visible (`error.chapter_not_found`) | `{chapter_id, track_id, number}` |

## 4. Writer tools

| Tool | Parameters | Behavior | Effects | `ui` |
|---|---|---|---|---|
| `submit_outline` | `outline`*, `reference_notes`*, `title` | **In a `chapter_outline` session**: validates and stages `pending_outline {outline, reference_notes, title?}`; nothing is written to the chapter. Result `result.writer_control` (`action = submit_outline`, `staged = true`). **In a legacy `chapter_write`/`planning` session**: saves the outline and reference notes immediately, setting the title only when the author has not set one; `staged = false` | outline session: `complete: outline` + `pending_outline` (applied by `apply_pending_outline` in the finalizing transaction); legacy: `phase_to: drafting` (the accepted notice is appended at finalize) | `{outline_chars, notes_chars}` |
| `submit_revision_plan` | `plan`* | Stores the plan in the call result (visible in history) | `phase_to: revising` | `{plan_chars}` |
| `read_draft` | `from_pid`, `to_pid`, `max_chars` (12,000) | The current text with pids, the tail-open marker, word count and target | — | `{paragraphs}` |
| `insert_text` | `text`*, `paragraph_end` (true), `chapter_end` (false) | Append per granularity (spec/03 §8). Result `result.draft_edit` (`action = insert_text`): what was written (new pids / appended to pid), whether the tail is open, word count vs target. A short completion is withheld once (`short_of_target`, spec/03 §8) | drafting + `chapter_end` (or `chapter` mode) → `complete: draft` | `{inserted: [pid], appended_to?: pid, tail_open, word_count}` |
| `insert_paragraphs` | `after`* (pid or `"start"`), `text`* | Inserts complete paragraphs at the position (one paragraph under `sentence`/`paragraph` granularity, spec/03 §8). Inserting after an open tail closes it first | — | `{inserted: [pid], after}` |
| `replace_text` | `pid`*, `old`*, `new`* (may be empty) | `old` must occur exactly once in the paragraph. Otherwise `error.text_not_found` (shows the paragraph text) or `error.text_ambiguous`. An empty result deletes the paragraph | — | `{pid, before, after}` |
| `rewrite_paragraph` | `pid`*, `text`* | Replaces the paragraph. Line breaks split the text (not allowed under `sentence`/`paragraph`); the first part keeps the pid, the rest get new pids after it | — | `{pid, inserted: [pid], before}` |
| `delete_paragraphs` | `pids`* (≥1) | Deletes the paragraphs. Unknown pids → error, and nothing is deleted | — | `{deleted: [pid]}` |
| `finish_draft` | `note` | The draft must be non-empty. Closes an open tail. Below 60% of `target_words_min` the first attempt does not complete: the result (`confirm_needed`) asks the agent to keep writing or call `finish_draft` again to confirm (spec/03 §8) | `complete: draft` | `{word_count, confirm_needed}` |
| `finish_revision` | `summary`* | Records the revision summary (shown to the author) | `complete: revision` | `{word_count}` |

Every successful DRAFT_EDIT call increments `content_version`, recomputes `word_count`, and emits
`chapter.content` with the exact ops (spec/10 §3).

## 5. Reviewer tool

| Tool | Parameters | Behavior | Effects | `ui` |
|---|---|---|---|---|
| `submit_review` | `verdict`* (`pass` \| `revise`), `summary`*, `issues`* (array of `{severity, category, pids, quote, problem, suggestion}`, may be empty), `feedback`* | Creates a `review_reports` row (`attempt` = count + 1, `content_version` = the session's `base_content_version`, `user_comment` = the session's `pending_comment`, which is then cleared). Unknown pids are dropped from issues, with a note in the result | `complete: review` | `{report_id, verdict, issues: n}` |

## 6. Archivist tools

All writes go through canon services: they record `change_log` rows (`actor: agent`,
`chapter_id` = session chapter, `session_id`), emit `canon.changed`, and enqueue re-indexing.
**Isolated tracks** create rows with `track_id = T` and cannot update or end shared rows
(`error.isolated_readonly`, which suggests recording a track-local note instead). Setting
materials are shared too, so `propose_setting_change` returns `error.isolated_readonly` there.

| Tool | Parameters | Behavior |
|---|---|---|
| `create_entity` | `kind`*, `name`*, `summary`*, `aliases`, `description`, `state`, `attributes` (object of strings), `importance` (1–5, default 2), `status`, `is_protagonist` (canon_seed only, otherwise `error.seed_only`), `note` | Fails if a visible entity already has that name or alias (`error.entity_exists` → use `update_entity`). Sets `first_chapter_id` and `last_chapter_id` to the session chapter. A shared character whose name equals an unlinked character design gets linked (`character_designs.entity_id`) |
| `update_entity` | `entity`*, `note`*, `name`, `aliases_add`, `aliases_remove`, `summary`, `description` (replace), `description_append`, `state`, `attributes_set` (object), `attributes_remove` (keys), `importance`, `status` | Partial update; `description` and `description_append` are mutually exclusive (`error.mutually_exclusive`). `note` is the human-readable delta and feeds mid-term memory ("修为：筑基初期→筑基中期") |
| `set_relation` | `source`*, `target`*, `kind`*, `inverse_kind`, `description`, `note` | Upsert of the visible active relation of the ordered `(source, target)` pair: the pair identifies it and `kind` is data (spec/06 §1.2), so a different `kind` **rewrites** the row the pair already has instead of creating a second active one. `inverse_kind`/`description` are replaced only when given, so a re-word keeps the existing detail. In an isolated track, an existing shared one is read-only (`error.isolated_readonly`) and a new one becomes a private row. `since_chapter_id` = session chapter on create |
| `end_relation` | `source`*, `target`*, `kind`*, `note`* | status → ended, `until_chapter_id` = session chapter. The `kind` must still match the pair's active relation, so the wording is checked: a stale one (re-worded by an earlier `set_relation`) is `error.relation_not_found` rather than ending an unrelated bond |
| `record_appearances` | `appearances`* (array of `{entity, role, note}`) | Replaces this chapter's appearance set (appearances belong to the chapter, so isolated tracks may record shared entities). Recomputes `first_chapter_id`/`last_chapter_id` of every affected entity, ordered by (track `ord`, chapter `ord`): a shared entity counts only shared-canon chapters (main track and canon-mode side tracks), so isolated chapters never change it; a private entity counts its own track's chapters |
| `add_timeline_event` | `title`*, `description`, `story_time`, `after_event` (id), `before_event` (id), `entities`, `importance` | Inserts at the chronological position: between the given neighbor and the next one; with neither, at the end. Both given → `error.mutually_exclusive`. `chapter_id` = session chapter |
| `update_timeline_event` | `event_id`*, `title`, `description`, `story_time`, `entities` (replace), `importance`, `move_after`, `move_before` | Partial update and/or move; `move_after` / `move_before` place the event like `after_event` / `before_event`, and both given → `error.mutually_exclusive` |
| `add_memory_note` | `kind`* (`fact` \| `world_rule` \| `other`), `content`*, `entities`, `importance` | `chapter_id` = session chapter |
| `retire_memory_note` | `note_id`*, `reason`* | status → retired (reason recorded as the change note) |
| `add_foreshadowing` | `title`*, `description`*, `planned_payoff`, `entities`, `importance` | `planted_chapter_id` = session chapter |
| `resolve_foreshadowing` | `foreshadowing_id`*, `status`* (`resolved` \| `abandoned`), `resolution`* | `resolved_chapter_id` = session chapter |
| `set_chapter_summary` | `summary`* | Writes `chapters.summary` (150–400 characters recommended; hard limit 2,000) |
| `propose_setting_change` | `target`* (`outline` \| `world` \| `protagonist` \| `character:<name>` \| `new_character`), `rationale`*, `edits` (array of `{find, replace}`), `append`, `character` (`{name, role_label, summary, profile}` for `new_character`) | Exactly one of `edits` / `append` / `character` (`error.exactly_one`). For `character:<name>`, `edits` and `append` apply to that design's `profile` (unknown name → `error.character_not_found`). Each `find` must currently match exactly once (`error.find_failed`). A `new_character` name that a design already has → `error.character_exists`. Creates a `pending` proposal and emits `proposal.changed` |
| `finish_maintenance` | `report`* | `chapter_maintenance` requires a summary and recorded appearances; otherwise `error.maintenance_incomplete` lists what is missing. In `chapter_maintenance` only: if the protagonist (for a side track, its focus character when set; spec/06 §4.2) appeared, this session may update them (a shared entity is read-only in an isolated track), and no `update_entity` in this session touched their `state`, the **first** call does not complete: it returns `result.finish_maintenance` with `confirm_needed = true` (not an error; sets `counters.finish_warned`), and the next call completes. Effects: `complete: maintenance` |

`ui` for canon writes: `{change_ids: [id], target_type, target_id, name}`, where `name` is the
target's stored name or title (an entity's name, an event's or a foreshadowing's title, a note's
first 40 characters, a relation's kind) and is empty for appearance sets and chapter summaries,
whose `target_id` is the chapter; for proposals `{proposal_id, target}`; for
`finish_maintenance` `{changes: n, proposals: n}`.

## 7. Conception tools

| Tool | Parameters | Behavior | `ui` |
|---|---|---|---|
| `ask_user` | `questions`* (1–4 × `{id, question, options: [{label, description}] (0–6), multi_select (false), allow_other (true)}`) | `await_input: ask_user`. The answer payload is `{answers: [{id, selected: [label], other?}]}`, validated against the question ids and labels and rendered with `result.ask_user` | `{questions}` (full, for the form) |
| `web_search` | `query`*, `max_results` (settings default, max 10) | spec/07 §6, in `prepare`. Results: title, URL, snippet, wrapped in `notice.web_content` | `{query, results: [{title, url}]}` |
| `web_fetch` | `url`*, `offset` (0), `max_chars` (8,000, max 20,000) | spec/07 §7, in `prepare` (a cached page needs no request). Markdown of the main content, wrapped in `notice.web_content`, with a continuation hint | `{url, title, chars, total_chars}` |
| `write_setting_doc` | `doc`* (`outline` \| `world` \| `protagonist`), `content`* | Replaces the document (revision + 1, change log `actor: agent`), emits `settings.changed` | `{doc, revision, chars}` |
| `edit_setting_doc` | `doc`*, `edits`* (`{find, replace}`; each `find` must match exactly once, else `error.find_failed`) | Applies all edits atomically, or none | `{doc, revision}` |
| `upsert_character` | `name`*, `new_name`, `role_label`, `summary`, `profile`, `rank` | Creates or updates a character design. A new design goes last unless `rank` is given; `rank` moves it (others shift) | `{character_id, name, rank}` |
| `remove_character` | `name`* | Deletes the character design | `{name}` |
| `reorder_characters` | `names`* (the complete order) | Must be a permutation of all names | `{order}` |
| `set_research_topics` | `topics`* (array of `{topic, requirements, scope}`) | Replaces the book's research tasks in status `draft` with these | `{count}` |
| `request_review` | `message`* | `await_input: request_review`. The answer payload is `{docs: {outline?, world?, protagonist?, characters?, research_topics?: {approved: bool, feedback?: string}}, general?: string}`. Approved items record their revision (`setting_docs.approved_revision`, `ConceptionState`). Rendered with `result.request_review` | `{message}` |

Every conception write records `change_log` rows (`actor: agent`; research topics are tasks,
not canon, so they have none) and emits `settings.changed` with the matching kind
(`setting_doc`, `characters`, `research_topics`). Edits the author makes in the studio while
the session is not running are reported with `notice.setting_edits` (a per-document diff
summary, like spec/03 §7), at the next sealing. Character reorders by drag and drop are reported
the same way.

## 8. Researcher tools

| Tool | Parameters | Behavior | `ui` |
|---|---|---|---|
| `web_search`, `web_fetch` | as in §7 | | |
| `search_knowledge` | as in §3 | Restricted to the task's target scope (to avoid duplicates) | |
| `read_knowledge_entry` | as in §3 | Target scope only | |
| `save_knowledge_entry` | `title`*, `content`*, `source_urls`* (≥1 URL; `[]` allowed only when `content` states the source is general knowledge), `tags` | Creates an entry in the target scope (`source: research`, `research_task_id`). A title already used in the scope → `error.entry_exists` (suggest `update_knowledge_entry`). Chunks the entry in the same transaction (keyword search works at once); embedding follows asynchronously | `{entry_id, title}` |
| `update_knowledge_entry` | `entry_id`*, `title`, `content` (replace), `edits` (`{find, replace}`), `source_urls_add`, `tags` | Target scope only; `content` and `edits` are mutually exclusive (`error.mutually_exclusive`); each `find` must match the content exactly once, and all edits apply or none (`error.find_failed`, whose `doc_label` is the `target_type` label of `knowledge_entry`); re-chunks like `save_knowledge_entry` | `{entry_id, title}` |
| `finish_research` | `report`* | Stores the report on the task (`done`; `entries_created` counts the entries this task has created over all its runs). The result reports how many entries this session saved or updated. Effects: `complete: research` | `{entries: n}` (this session) |

Research budget (shown in the kickoff, enforced softly): 40 tool calls and 25 entries per task.
Past the budget, every result gets `notice.research_budget`.

## 9. Required tests
- Each tool: a happy path, each documented error, and the `ui` payload shape.
- `tools.toml` examples validate against the schema and the Rust struct (compile-time list
  equals the TOML list; no orphan definitions).
- Phase gating matrix (every tool × every phase).
- Entity resolution order and ambiguity.
- Isolated-track write rules.
- Granularity matrix for `insert_text` (mode × tail state × flags), including CJK and English
  joining; single-paragraph limits of `insert_paragraphs` / `rewrite_paragraph` under
  `sentence` and `paragraph`; the short-completion confirmation.
- `prepare` runs outside the transaction (a scripted slow web provider plus a concurrent write
  from another session both succeed).
