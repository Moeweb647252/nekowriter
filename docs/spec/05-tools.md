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
- `result.finish_maintenance` receives `proposals` (setting proposals of the session) and
  `goal_proposals` (goal proposals of the session) separately, so a goal proposal is never
  reported as a setting change. With no goal proposals the text is unchanged.
- `result.writer_control` also receives `staged` (boolean). For `submit_outline` in a
  `chapter_outline` session it is true and the result says the outline was submitted and takes
  effect when the author accepts it — never that it was saved. It is false for every other
  writer-control tool.
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
  | `result.canon_write` (`action` = tool) | archivist write tools except `finish_maintenance`, including `propose_goal_change` |
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
| archivist | `get_entity`, `list_entities`, `get_relations`, `query_timeline`, `list_foreshadowing`, `search_canon`, `read_chapter`, `create_entity`, `update_entity`, `set_relation`, `end_relation`, `record_appearances`, `add_timeline_event`, `update_timeline_event`, `add_memory_note`, `retire_memory_note`, `add_foreshadowing`, `update_foreshadowing`, `resolve_foreshadowing`, `set_chapter_summary`, `propose_goal_change`, `propose_setting_change`, `finish_maintenance` |
| conception | `ask_user`, `search_knowledge`, `read_knowledge_entry`, `web_search`, `web_fetch`, `write_setting_doc`, `edit_setting_doc`, `upsert_character`, `remove_character`, `reorder_characters`, `set_research_topics`, `request_review` |
| researcher | `web_search`, `web_fetch`, `search_knowledge`, `read_knowledge_entry`, `set_research_topics`, `save_knowledge_entry`, `update_knowledge_entry`, `finish_research` |
| assistant | `book_overview`, `open_chapter`, `read_setting_doc`, `read_characters`, `get_entity`, `list_entities`, `get_relations`, `query_timeline`, `list_foreshadowing`, `search_canon`, `list_goals`, `search_chapters`, `search_knowledge`, `read_knowledge_entry`, `list_research`, `list_sessions`, `write_setting_doc`, `edit_setting_doc`, `upsert_character`, `remove_character`, `reorder_characters`, `create_chapter`, `edit_chapter`, `edit_chapter_text`, `create_entity`, `update_entity`, `set_relation`, `end_relation`, `add_timeline_event`, `update_timeline_event`, `add_memory_note`, `retire_memory_note`, `add_foreshadowing`, `update_foreshadowing`, `resolve_foreshadowing`, `delete_canon_record`, `create_goal`, `update_goal`, `create_research_task`, `update_research_task`, `delete_research_task`, `add_knowledge_entry`, `update_knowledge_entry`, `start_session` (the book agent, §10, D-169; `update_foreshadowing` since D-170) |

**RETRIEVAL** = the first ten writer tools (`get_entity` … `read_chapter`).
**DRAFT_EDIT** = `insert_text`, `insert_paragraphs`, `replace_text`, `rewrite_paragraph`,
`delete_paragraphs`.

**What a session freezes** (D-150). The table above is each role's full list. A session freezes
that list, in that order, minus:
- the tools of every canon system that is off in its `canon_systems` (spec/06 §10.1):

  | System | Tools |
  |---|---|
  | Entities | `get_entity`, `list_entities`, `get_relations`, `create_entity`, `update_entity`, `set_relation`, `end_relation`, `record_appearances` |
  | Timeline | `query_timeline`, `add_timeline_event`, `update_timeline_event` |
  | Memory | `add_memory_note`, `retire_memory_note`, `set_chapter_summary` |
  | Foreshadowing | `list_foreshadowing`, `add_foreshadowing`, `update_foreshadowing`, `resolve_foreshadowing` |
  | Goals | `propose_goal_change` |

- `search_canon`, only when Entities, Timeline, Memory and Foreshadowing are all off;
- `propose_goal_change`, unless the session is `chapter_maintenance`, Goals is on, and the
  chapter has relevant goal nodes (the `block.goals` set, spec/06 §11.5);
- for a research session, by task kind (spec/07 §8.1, D-168):

  | Task kind | Frozen tools |
  |---|---|
  | `topic` | Every tool except `set_research_topics`: the pre-D-158 seven tools, in the same order |
  | `preparation` (a knowledge check) | Only `search_knowledge`, `read_knowledge_entry`, `set_research_topics` and `finish_research`. A check plans and collects nothing |

- for the assistant (§10, D-169), the goal tools `list_goals`, `create_goal` and `update_goal`
  belong to Goals, so they leave with it. `delete_canon_record`, like `search_canon`, stays
  unless Entities, Timeline, Memory and Foreshadowing are all off;
- `set_target_scope` (D-158) is retired. No session created after D-168 freezes it. The handler,
  its `tools.toml` entry, `result.target_scope` and the errors `target_scope_unset` and
  `scope_excluded` stay registered, so a check session frozen before D-168 still runs it.

A book with all five systems on and no relevant goal nodes therefore freezes exactly the pre-D-150
lists, except that the archivist's has `update_foreshadowing` since D-170 (22 tools). The map lives
once in `agent/tools/registry.rs`; `scripts/validate_prompts.py` keeps the same map as data.

| Writer phase | Permitted |
|---|---|
| `planning` (historical rows only, D-159) | nothing: no session runs in it |
| `drafting` | `read_draft`, DRAFT_EDIT, `finish_draft` |
| `revision_planning` | RETRIEVAL, `read_draft`, `submit_revision_plan` |
| `revising` | `read_draft`, DRAFT_EDIT, `finish_revision` |

A `chapter_outline` session is **phase-free**: its frozen planner list is its gate, so it permits
all twelve tools (retrieval, `read_draft`, `submit_outline`). The writer list has no
`submit_outline`; only historical `chapter_write` rows (frozen before the split) list it.

Archivist by session kind: `canon_seed` forbids `record_appearances`, `set_chapter_summary`,
`read_chapter`; the other kinds allow all of their tools. Reviewer, conception, planner, and
researcher sessions allow all of their tools.

## 3. Retrieval tools

| Tool | Parameters | Behavior and result | `ui` |
|---|---|---|---|
| `get_entity` | `entity`* | Entity card (`result.entity_card`): kind, name, aliases, importance, status, summary, description, **state**, attributes, relations in both directions phrased from this entity ("林凡(#12) 是 张三(#15) 的 师兄"), the latest 10 change notes with chapter labels, appearances (first, last, count in the mid-term window), and the linked character design's role and summary. In side tracks with an anchor, notes from chapters after the anchor are marked `（锚点之后）`, and so is the state when its newest change came from such a chapter (spec/06 §4.2) | `{entity_id, name, kind}` |
| `list_entities` | `kind`, `query`, `status`, `limit` (30, max 100) | One line per entity: `#id 【类型】名字（别名…）— 简介 [状态]`, sorted by importance desc then last appearance desc | `{count}` |
| `get_relations` | `entity`*, `other`, `include_ended` (false) | Relation lines with descriptions and since/until chapter labels | `{entity_id, count}` |
| `query_timeline` | `entity`, `query`, `around_event` (id), `under` (entry id: only its descendants, D-170), `from_chapter`, `to_chapter` (numbers in the main track), `limit` (20, max 50), `order` (`chronological` \| `recent_first`) | Ordered events: `[#id] (story_time) 标题 — 描述 · 第N章 · 涉及：A、B`, in story order. Since D-170 an item also shows its parent entry (`↳ #12`; omitted at the top level or when the parent is hidden) and `（计划中）` for a planned entry, with one line explaining the marker when any is shown. The root and abandoned entries never appear; planned entries have no chapter, so `from_chapter`/`to_chapter` leave them out. With `around_event`: the 5 events before and after it. Anchor marking as above | `{count}` |
| `list_foreshadowing` | `status` (`open`), `entity`, `limit` (20, max 50) | `[#id] 标题 — 描述 · 埋设于第N章 · 回收锚点：#12 标题 之后（计划中） · 回收方向：…`. The anchor (D-170) renders only while Timeline is on; an abandoned anchor renders `（回收锚点已放弃）` without its title, and an unanchored row shows only the direction | `{count}` |
| `search_canon` | `query`*, `kinds` (subset of `entity`,`note`,`foreshadowing`,`event`,`summary`), `limit` (8, max 20) | Hybrid search (spec/07 §5) over visible search docs except passages: `【类型】标题(#ref) — snippet`. The kinds searched are the requested ones (or all) **intersected with the enabled kinds of the session's frozen `canon_systems`**: `entity` needs Entities, `event` Timeline, `note` and `summary` Memory, `foreshadowing` Foreshadowing. An empty intersection gives the ordinary empty result; `block.canon_systems` has told the model which systems are off. The timeline root and abandoned entries have no search doc, and a planned entry's hit is marked `（计划中）` (D-170) | `{query, count}` |
| `search_knowledge` | `query`*, `scope` (name), `limit` (6, max 15) | Hybrid search over the book's visible scopes (spec/07 §1: every scope the author did not uncheck; a topic research session: its target scope only; a knowledge check: the book's visible scopes), or over the one named by `scope`, which must be one of them (`error.scope_not_found`, listing the visible names). `[条目#id]《标题》（范围）— snippet ≤400 chars`. No visible scope → `error.no_knowledge_scope` | `{query, count}` |
| `read_knowledge_entry` | `entry_id`*, `offset` (0) | Title, scope, sources, content from `offset`, ≤ 6,000 characters, plus a continuation hint. The scope must be visible | `{entry_id, title}` |
| `search_chapters` | `query`*, `limit` (6, max 15) | Hybrid search over official-chapter passages in visible tracks: `第N章《标题》 p12–p15：snippet`. Main-story passages after a side story's anchor are marked | `{query, count}` |
| `read_chapter` | `chapter`* (`"12"` = number in the session's track, `"main 12"` = main track, `"#345"` = id), `from_pid`, `max_chars` (8,000, max 20,000) | Official text with pids. Non-official chapters → `error.chapter_not_official`; chapters of other isolated tracks are not visible (`error.chapter_not_found`) | `{chapter_id, track_id, number}` |

## 4. Writer tools

| Tool | Parameters | Behavior | Effects | `ui` |
|---|---|---|---|---|
| `submit_outline` | `outline`*, `reference_notes`*, `title` | **In a `chapter_outline` session**: validates and stages `pending_outline {outline, reference_notes, title?}`; nothing is written to the chapter. Result `result.writer_control` (`action = submit_outline`, `staged = true`) | `complete: outline` + `pending_outline` (applied by `apply_pending_outline` in the finalizing transaction) | `{outline_chars, notes_chars}` |
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
| `add_timeline_event` | `title`*, `description`, `story_time`, `after_event` (id), `before_event` (id), `entities`, `importance`, `parent` (entry id; omitted = top level), `status` (`occurred` default \| `planned`) | Inserts at the chronological position: between the given neighbor and the next one. With neither, a child goes right after the last node of its parent's subtree (the parent itself if it has none) and a top-level entry at the end (D-170). Neighbors never change the parent. Both given → `error.mutually_exclusive`. `chapter_id` = session chapter for an `occurred` entry; a `planned` one has none. A planned entry under an occurred ancestor of its scope → `error.timeline_conflict` (`occurred_ancestor`) |
| `update_timeline_event` | `event_id`*, `title`, `description`, `story_time`, `entities` (replace), `importance`, `move_after`, `move_before`, `parent` (entry id) or `top_level` (true), `status` (`occurred` \| `abandoned`), `chapter` (with `status: occurred`; a main-track number, default the session chapter), `note` (required with `status`) | Partial update and/or move; `move_after` / `move_before` place the event like `after_event` / `before_event`, and both given → `error.mutually_exclusive` (so are `parent` and `top_level`). `parent`/`top_level` move the entry with its subtree (D-170). `status` follows spec/06 §1.4: `planned → occurred` records the chapter; `planned → abandoned` also abandons the planned descendants of its scope and reports how many; anything else → `error.timeline_conflict` (`backward`). A planned descendant blocks promotion (`planned_descendant`) and an occurred one blocks abandoning (`occurred_descendant`) |
| `add_memory_note` | `kind`* (`fact` \| `world_rule` \| `other`), `content`*, `entities`, `importance` | `chapter_id` = session chapter |
| `retire_memory_note` | `note_id`*, `reason`* | status → retired (reason recorded as the change note) |
| `add_foreshadowing` | `title`*, `description`*, `planned_payoff` (the direction only, never a chapter number), `entities`, `importance`, `payoff_event` (entry id) and `payoff_position` (`before` \| `after` \| `at`), both or neither (D-170) | `planted_chapter_id` = session chapter. Half an anchor → `error.validation` |
| `update_foreshadowing` | `foreshadowing_id`*, `note`*, `planned_payoff`, `payoff_event` + `payoff_position`, `clear_anchor` (true) (D-170) | Changes the direction and re-anchors or clears the anchor; `payoff_event` with `clear_anchor` → `error.mutually_exclusive`. Any status. Shared rows are read-only in isolated tracks (`error.isolated_readonly`) |
| `resolve_foreshadowing` | `foreshadowing_id`*, `status`* (`resolved` \| `abandoned`), `resolution`* | `resolved_chapter_id` = session chapter. Resolving while the anchor is still planned succeeds, and the result says so (`anchor_planned`, D-170) |
| `set_chapter_summary` | `summary`* | Writes `chapters.summary` (150–400 characters recommended; hard limit 2,000) |
| `propose_setting_change` | `target`* (`outline` \| `world` \| `protagonist` \| `character:<name>` \| `new_character`), `rationale`*, `edits` (array of `{find, replace}`), `append`, `character` (`{name, role_label, summary, profile}` for `new_character`) | Exactly one of `edits` / `append` / `character` (`error.exactly_one`). For `character:<name>`, `edits` and `append` apply to that design's `profile` (unknown name → `error.character_not_found`). Each `find` must currently match exactly once (`error.find_failed`). A `new_character` name that a design already has → `error.character_exists`. Creates a `pending` proposal and emits `proposal.changed` |
| `propose_goal_change` | `action`* (`create` \| `complete` \| `drop`), `rationale`*, `node` (id; `complete`/`drop`), `parent` (id of a plan; `create`, optional), `title`, `purpose` (`create`, required), `chapter_from`, `chapter_to` (`create`), `entities` (names), `events`, `foreshadowing` (ids) (`create`) | Proposes a change to the author's writing goals (spec/06 §11.6). `complete` / `drop`: the node exists, is visible and is `active` or `waiting`. `create`: a new **task**; `parent`, when given, is an open plan in the session's scope; title, purpose and range follow spec/06 §11.1; `events` may name planned timeline entries, but not abandoned ones (D-170). Missing fields for the action → `error.validation`; an unknown or invisible `node`/`parent` → `error.not_found` (`what_label` = the level's `target_type` label). In an isolated track only that track's private nodes may be targeted; a shared node → `error.isolated_readonly`. Creates a `pending` proposal (`goal_node` or `new_goal_task`, spec/06 §5) and emits `proposal.changed`. A pending proposal of the same node and status already exists → no new row, and the result names it. Result `result.canon_write` (`action = propose_goal_change`) |
| `finish_maintenance` | `report`* | `chapter_maintenance` requires recorded appearances when the session's `canon_systems` include **Entities**, and a summary when they include **Memory**; otherwise `error.maintenance_incomplete` lists what is missing. Goals are never required. In `chapter_maintenance` only, and only with Entities in the session's systems: if the protagonist (for a side track, its focus character when set; spec/06 §4.2) appeared, this session may update them (a shared entity is read-only in an isolated track), and no `update_entity` in this session touched their `state`, the **first** call does not complete: it returns `result.finish_maintenance` with `confirm_needed = true` (not an error; sets `counters.finish_warned`), and the next call completes. Effects: `complete: maintenance` |

`ui` for canon writes: `{change_ids: [id], target_type, target_id, name}`, where `name` is the
target's stored name or title (an entity's name, an event's or a foreshadowing's title, a note's
first 40 characters, a relation's kind) and is empty for appearance sets and chapter summaries,
whose `target_id` is the chapter; for proposals `{proposal_id, target}`; for goal proposals
`{proposal_id, action, node_id?}`; for `finish_maintenance` `{changes: n, proposals: n,
goal_proposals: n}`.

**Timeline tree** (D-170, spec/06 §1.4). An entry named by `parent`, `payoff_event`,
`around_event` or `under` must be an entry of the book that the session can see and that is not
abandoned. A parent must also be in the new entry's scope and outside the moved entry's own
subtree. Anything else is `error.not_found` (`what_label` = timeline event). The root is never
nameable. While the session's Timeline is off, `payoff_event` / `payoff_position` →
`error.system_off`. `error.timeline_conflict` carries `reason` and the conflicting entry
(`#id 标题`), which the session can always see.

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
| `set_research_topics` | `topics`* (array of `{topic, requirements, scope}`) | Replaces **the agent's own** topics that have not run. In a conception session these are the **supplementary** drafts: it deletes the book's `draft` tasks with `kind = 'topic'`, `origin = 'conception'` and `check_id IS NULL`, then inserts these as `conception` drafts. In a knowledge check it works on the check's topic list instead (§8). It never deletes or edits an `author` task (one the author created, or one the author edited) in any status, nor a task that has run (D-158). The result lists the topics it kept (`kept[]`), so the model does not repeat them | `{count, kept}` |
| `request_review` | `message`* | `await_input: request_review`. The answer payload is `{docs: {outline?, world?, protagonist?, characters?, research_topics?: {approved: bool, feedback?: string}}, general?: string}`. Approved items record their revision (`setting_docs.approved_revision`, `ConceptionState`). Rendered with `result.request_review` | `{message}` |

**Knowledge** (D-158, D-168): `search_knowledge` and `read_knowledge_entry` behave as in §3, over
the book's visible scopes. The kickoff lists those scopes and carries the latest knowledge check's
report (spec/07 §8.1). It also lists the research topics `set_research_topics` would keep, with
their status and `entries_created`, including the check's topics that ran before conception. So
conception builds on what was researched, and proposes supplementary topics only for knowledge
that came up during conception and is still missing.

**Ownership of research topics.** `origin` records who owns a task. Every task the author creates
through the API is `author`. An author PATCH of a `conception` task sets it to `author` in the same
write, because editing adopts it, and no agent tool turns it back.

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
| `search_knowledge` | as in §3 | A topic task: restricted to its target scope (to avoid duplicates). A knowledge check: the book's visible scopes | |
| `read_knowledge_entry` | as in §3 | A topic task: target scope only. A knowledge check: the book's visible scopes | |
| `set_research_topics` | as in §7 | Knowledge checks only (§2, D-168). Validates like the conception path, plus: more than **8** topics → `error.validation` naming the limit; a `scope` naming a scope the author excluded for the book → `error.scope_excluded` (`name`). Either error writes nothing. Deletes the check's **waiting agent topics** (`check_id` = this check, `kind = 'topic'`, `origin = 'conception'`, `status = 'ready'`, no session), then inserts each topic as `ready`, `origin = 'conception'`, `check_id` = this check, in the book's writing language, with `scope_id` unresolved. No `topics_revision`, `change_log` or `settings.locked`. `kept[]` lists the check's topics it left in place (added or adopted by the author, deferred, running or run). Rendered with `result.conception_write` | `{count, kept}` |
| `set_target_scope` | `name`*, `description` | **Retired** (D-168): frozen only by check sessions created before D-168. Knowledge checks only (§2). Sets the scope that `save_knowledge_entry` and `update_knowledge_entry` write to; may be called again to switch, and saved entries stay where they are. An existing scope the author excluded for the book → `error.scope_excluded`. An existing visible scope is used as is (`description` ignored: an agent never rewrites a shared description). A new name needs `description` (else `error.validation`); the scope is created (visible to every book, `knowledge.changed`) | `{scope, created, entry_count}` |
| `save_knowledge_entry` | `title`*, `content`*, `source_urls`* (≥1 URL; `[]` allowed only when `content` states the source is general knowledge), `tags` | Creates an entry in the target scope (`source: research`, `research_task_id`). A title already used in the scope → `error.entry_exists` (suggest `update_knowledge_entry`). Chunks the entry in the same transaction (keyword search works at once); embedding follows asynchronously | `{entry_id, title}` |
| `update_knowledge_entry` | `entry_id`*, `title`, `content` (replace), `edits` (`{find, replace}`), `source_urls_add`, `tags` | Target scope only; `content` and `edits` are mutually exclusive (`error.mutually_exclusive`); each `find` must match the content exactly once, and all edits apply or none (`error.find_failed`, whose `doc_label` is the `target_type` label of `knowledge_entry`); re-chunks like `save_knowledge_entry` | `{entry_id, title}` |
| `finish_research` | `report`* | Stores the report on the task (`done`; `entries_created` counts the entries this task has created over all its runs). A knowledge check finishes without a target scope (a check created before D-168 may have one). The commit triggers the topic queue (spec/07 §8.1 step 6). The result reports how many entries this session saved or updated. Effects: `complete: research` | `{entries: n}` (this session) |

In a knowledge check frozen before D-168, `save_knowledge_entry` and `update_knowledge_entry`
before any `set_target_scope` return `error.target_scope_unset` (call `set_target_scope` first)
instead of `error.no_knowledge_scope`. A check created after D-168 freezes neither tool.

Research budget (shown in the kickoff, enforced softly): the session's frozen
`research_max_tool_calls` (default 40) and `research_max_entries` (default 25). Each topic of a
check's queue is its own session with its own budget. A knowledge check's kickoff shows only the
tool-call budget and the topic cap, because a check saves no entries. Both are app settings and must
be at least 1; unlike the tool-error limit, `0` is not unlimited. Past either budget, the **first**
result gets `notice.research_budget` once; later results do not repeat the identical notice (D-164).

## 9. Required tests
- Each tool: a happy path, each documented error, and the `ui` payload shape.
- `tools.toml` examples validate against the schema and the Rust struct (compile-time list
  equals the TOML list; no orphan definitions).
- Phase gating matrix (every tool × every phase).
- Frozen lists per role × canon systems: each system off removes exactly its tools in order;
  `search_canon` survives until all four are off; `propose_goal_change` follows its gate.
- `search_canon` never returns a kind of a system that is off in the session's `canon_systems`.
- `propose_goal_change`: each action's happy path, missing fields, an unknown node or parent, a
  closed node, a parent that is not a plan, the isolated rule, the duplicate rule.
- Entity resolution order and ambiguity.
- Timeline tree (D-170): nesting and default placement, each `timeline_conflict` reason, the
  scope and cycle refusals, abandoning a subtree, `under`, the root and abandoned entries absent
  from `query_timeline` and `search_canon`, anchors (set, re-anchor, clear, half an anchor,
  abandoned or foreign anchor, Timeline off), the `anchor_planned` notice, and a pre-D-170 frozen
  session calling the old shapes.
- Isolated-track write rules.
- Granularity matrix for `insert_text` (mode × tail state × flags), including CJK and English
  joining; single-paragraph limits of `insert_paragraphs` / `rewrite_paragraph` under
  `sentence` and `paragraph`; the short-completion confirmation.
- `prepare` runs outside the transaction (a scripted slow web provider plus a concurrent write
  from another session both succeed).
- Knowledge (D-158, D-168):
  - A topic research session freezes exactly the old seven researcher tools. A knowledge check
    freezes exactly `search_knowledge`, `read_knowledge_entry`, `set_research_topics` and
    `finish_research`. No new session freezes `set_target_scope`, but a pre-D-168 frozen check
    still runs it.
  - Conception's `search_knowledge` never returns an excluded scope.
  - `set_research_topics` in conception keeps every `author` task and every check topic, and
    reports the kept drafts.
  - `set_research_topics` in a check:
    - inserts `ready` topics with `check_id`;
    - replaces only its waiting agent topics, and keeps author, adopted, deferred, running and run
      ones in `kept[]`;
    - 9 topics → `validation`;
    - an excluded scope → `scope_excluded`;
    - in either error case, nothing is written.
- Book agent (D-169):
  - **Frozen list.** It equals the assistant list with every system on, and each system off
    removes exactly its tools. No tool approves, sends back, submits or finalizes a chapter,
    accepts a proposal, reverts, merges or deletes chapters, which is asserted by name.
  - **Review mode.** Every action tool holds with its preview kind.
  - **Reused tools.**
    - Canon writes record `actor: agent` with a `NULL` chapter.
    - Conception's setting tools write after finalize.
    - `update_knowledge_entry` reaches any visible entry.
  - **New tools.** Each has a happy path and its documented errors (`chapter_locked`,
    `session_conflict`, `start_refused` per reason, `system_off`, `goal_open_children`).
  - **Chapter edits.** `edit_chapter_text` keeps pids and makes one backup per chapter and
    session.
  - **`start_session`.** It never nests a write and spawns after the commit.

## 10. Book agent tools (D-169)

The `assistant` role works for the author on a whole book, in a `book_agent` session (spec/03
§1.1, spec/08 §8).

**Visibility.** It has no track:
- **canon** is seen as the main story sees it (shared rows only; the private rows of isolated
  side stories are out of reach);
- **chapters** of every track can be read and edited;
- **knowledge** is the book's visible scopes.

**Two kinds of tools.**
- **Read tools** never hold.
- **Action tools** (`ASSISTANT_ACTION_TOOLS`: every tool from `write_setting_doc` to
  `start_session` in the §2 list) hold for approval when the session's `write_mode` is `review`.
  Holding happens before `execute`, with a preview (spec/03 §5.2). In `full` mode they execute at
  once.
- Every action tool has a trivial `prepare` and a read-only `preview(ctx, input) →
  ToolPreview`, stored in `ui.preview`:

  | `kind` | Payload | Used by |
  |---|---|---|
  | `diff` | `{label, before, after}` | text edits (the touched paragraphs with one paragraph of context) |
  | `fields` | `{label, rows: [{field, before?, after?}], shared?}` | record writes. `shared` marks a knowledge entry seen by other books |
  | `action` | `{label, text}` | starts, deletes and reorders |

**Reused tools.** These keep their names, schemas and behavior: the retrieval tools (§3),
conception's setting tools (§7), the archivist's canon tools (§6) and `update_knowledge_entry`
(§8). In a book agent session:
- the "session chapter" is `NULL`, so `chapter_id`, `since_chapter_id`, `until_chapter_id`,
  `planted_chapter_id`, `resolved_chapter_id`, `first_chapter_id` and `last_chapter_id` are left
  `NULL`, as for an author's manual record without a chapter;
- `is_protagonist` → `error.seed_only`;
- setting tools are never `settings.locked`, because the book is not conceiving;
- `update_knowledge_entry` accepts any entry of a visible scope;
- every write records `change_log` rows with `actor: agent` and this session where the target
  has a change log.

**New tools:**

| Tool | Parameters | Behavior | `ui` |
|---|---|---|---|
| `book_overview` | — | Renders `block.book_index` (spec/03 §10) with current data | `{chapters, sessions}` |
| `open_chapter` | `chapter`* (`"12"` = main track, `"#345"` = id), `part` (`text` \| `outline` \| `reference_notes` \| `guidance` \| `summary`; default `text`), `from_pid`, `max_chars` (8,000, max 20,000) | Any status, any track. The header shows the label, title, status, `content_version`, canon status, word count and open sessions. `text` has pids and a continuation hint. Unknown → `error.not_found` | `{chapter_id, part}` |
| `read_setting_doc` | `doc`* (`outline` \| `world` \| `protagonist`), `offset` (0) | Up to 12,000 characters from `offset`, with the revision and a continuation hint | `{doc, revision}` |
| `read_characters` | `name` | Without `name`: the ranked designs (rank, name, role label, summary, linked entity). With it: the full design. Unknown → `error.character_not_found` | `{count}` |
| `list_goals` | `status` (`open` = active + waiting, default; or a single status), `level` | Shared-scope nodes: `[#id] 【层级】标题 — 目的 · 状态 · 第a–b章`, with ready and closable marks (spec/06 §11.3) | `{count}` |
| `list_research` | `status` | The book's research tasks: id, kind, topic, scope, status, entries | `{count}` |
| `list_sessions` | `status` (`open`, default; or one status), `kind` | The book's sessions: id, kind, chapter label, status, created time | `{count}` |
| `create_chapter` | `track` (main by default; `"#id"`), `title`, `guidance` | A `planned` chapter at the end of the track. The rules of `POST /tracks/:id/chapters` apply | `{chapter_id, number}` |
| `edit_chapter` | `chapter`*, `title`, `guidance`, `outline` \| `outline_edits` (`{find, replace}`), `reference_notes` \| `notes_edits` | The pairs are mutually exclusive (`error.mutually_exclusive`), and each `find` matches exactly once (`error.find_failed`). While a planner or writer session of the chapter is `running` → `error.chapter_locked` (with `session_id`); otherwise the change is reported to that chapter's open session (spec/03 §7). `summary` is not editable here | `{chapter_id, fields}` |
| `edit_chapter_text` | `chapter`*, `edits`* (≥1 `{find, replace}`) | Matched in order against the current text (paragraphs joined by a blank line), exactly once each, otherwise `error.find_failed` and nothing is written. The new text goes through the `PUT …/content/text` service: pid alignment, `content_version` + 1, word count, `chapter.content`, and an official synced chapter → `stale`. Locks and reports as for `edit_chapter`. The first text edit of a chapter in the session first saves a `backup` version (`session_id` set) | `{chapter_id, version, changed_pids}` |
| `delete_canon_record` | `target_type`* (`entity` \| `relation` \| `timeline_event` \| `memory_note` \| `foreshadowing`), `id`*, `note`* | Deletes through the author's DELETE service, with a change log entry, so revert recreates the record (spec/06 §3). A type whose system is off in the session → `error.system_off`. A private row of an isolated track → `error.not_found`, and so is an abandoned timeline entry. A timeline entry with children → `error.timeline_has_children` (D-170) | `{change_ids, target_type, target_id, name}` |
| `create_goal` | `level`*, `parent`, `title`*, `purpose`*, `status`, `chapter_from`, `chapter_to`, `conditions`, `entities`, `events`, `foreshadowing`, `note` | The `POST /books/:id/goals` rules (spec/06 §11.1–§11.4), shared scope | `{goal_id, level}` |
| `update_goal` | `node`*, `note`*, `title`, `purpose`, `status`, `parent`, `chapter_from`, `chapter_to`, `conditions`, `entities`, `events`, `foreshadowing`, `confirm_open_children` | The `PATCH /goals/:id` rules. `level` cannot change. Closing a node with open descendants without `confirm_open_children` → `error.goal_open_children` | `{goal_id}` |
| `create_research_task` | `topic`*, `requirements`*, `scope`* | A `ready` `topic` task of the book, `origin: author` (the book agent acts for the author, so no agent replaces it) | `{task_id}` |
| `update_research_task` | `task`*, `topic`, `requirements`, `scope`, `status` (`ready` \| `deferred`) | PATCH rules, plus `status` through defer and resume (spec/07 §8). An open task → `error.session_conflict`. A knowledge check accepts `requirements` only (`error.validation`) | `{task_id, status}` |
| `delete_research_task` | `task`* | DELETE rules. An open task → `error.session_conflict` | `{task_id}` |
| `add_knowledge_entry` | `scope`* (a visible scope's name), `title`*, `content`*, `source_urls`, `tags` | The `save_knowledge_entry` rules, in the named scope, with `source: manual`. Unknown or not visible → `error.scope_not_found` | `{entry_id, title}` |
| `start_session` | `action`* (`outline` \| `maintain` \| `resync` \| `research` \| `knowledge_check`), `chapter` (chapter actions), `task` (`research`), `guidance` (`outline`; the focus of `knowledge_check`) | Creates the session through the **in-transaction** form of the matching endpoint's service (spec/08 §1.1, spec/07 §8, §8.1), with the endpoint's preconditions and **no `force`**. The book settings supply `pause_after_outline`. Chapter **write**, **revise** and **review** sessions are not actions for now; the author starts them (D-169). An action that would have to stop or abandon another session → `error.session_conflict` (with `session_id`). Another refusal → `error.start_refused` (`reason` = the API error code). No model → `error.model_missing_role`. The session is spawned after the commit (`ToolOutcome.spawn`). The agent does not wait for it | `{session_id, kind, chapter_id?, task_id?, existing}` |

**No author-only operations.** These have no tool (D-169): starting a chapter write, revise or
review session (for now); accepting a held outline or revision plan; submitting, approving or
sending back a chapter; finalizing; accepting or rejecting proposals; reverting changes; merging
entities; deleting chapters, tracks or the book; book and app settings; controlling another
session.

