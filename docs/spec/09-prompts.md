# 09 — Prompt Registry

Every text that reaches a model is a template in the registry: system prompts, task
instructions, context blocks, runtime notices, tool results and errors, and tool descriptions.
The shipped pack is Chinese (`prompts/`). The book's writing language tells the model which
language to write in. Code contains no model-facing text (the one exception is the fixed
`ping` of the provider connection test, spec/04 §8, which is not part of any agent workflow).

## 1. Pack layout (`prompts/`, embedded with rust-embed)

| File | Kind | Content |
|---|---|---|
| `system.toml` | `system` | one system prompt per role |
| `tasks.toml` | `task` | kickoff and follow-up instructions per scenario |
| `blocks.toml` | `block` | context blocks (settings, memory, draft, …) |
| `notices.toml` | `notice` | runtime notices appended to tool results or sent as messages |
| `results.toml` | `result` | tool result and error renderings |
| `shared.toml` | `shared` | helpers: `shared.label` (enum value → display label) and `shared.chapter_label` (chapter references) |
| `tools.toml` | `tool` | tool definitions: `description`, `input_schema`, `examples` |
| `variables.toml` | — | documentation of every template variable (shown in the UI) |

**Labels**: code never formats display text. Every `*_label` variable (entity kinds, statuses,
appearance roles, document names, granularity and phase names, language names, chapter labels)
is produced by rendering `shared.label {enum, value}` or `shared.chapter_label {track_kind,
track_title, number, title, with_title}` through the same registry, cached per `PromptSet`; the
one exception is a character design's `role_label`, which is stored text (主角, 反派). The label
enums are `entity_kind`, `entity_status`, `appearance_role`, `note_kind`,
`foreshadowing_status`, `relation_status`, `setting_doc`, `granularity`, `phase`, `severity`,
`category`, `verdict`, `target_type` (including `goal`, `plan`, `task`), `search_kind`,
`goal_level`, and `language`; an unknown enum or value renders the raw value. Lists (aliases, field names, pids, tool names, scope names) reach templates
as arrays and are joined there (`| join("、")`), and a pid range as its pids, so no separator or
other punctuation of the model-facing language lives in code. Entity references use the
language-neutral form `名字(#id)` (spec/05 §1.5).

**Validation of the shipped pack**: `scripts/validate_prompts.py` (already in the repository;
run with `python3 -I scripts/validate_prompts.py .`) parses every file, checks key uniqueness
against this document, validates every tool example against its schema, and renders every
template with sample contexts under strict undefined (Jinja2 as a stand-in for MiniJinja). The
Rust test suite is the authoritative check (§2). The script is a fast pre-commit check for
prompt edits.

Entry format (all files except `tools.toml` and `variables.toml`):

```toml
[[prompt]]
key = "writer.system"
kind = "system"
agent = "writer"                 # conception | researcher | planner | writer | reviewer | archivist | shared
vars = []                        # top-level variables the template may use (documented in variables.toml)
title_en = "Writer — system prompt"
title_zh = "写手 · 系统提示词"
text = '''
……模板正文……
'''
```

`tools.toml` entries are `[tools.<name>]` with `description` (the default for key
`tool.<name>`), `input_schema` (JSON Schema as TOML tables), and `examples` (array of valid
inputs, used by tests). Their UI titles are the tool names.

TOML literal multi-line strings (`'''`) are used so backslashes and Jinja syntax need no
escaping.

## 2. Keys

| Group | Keys |
|---|---|
| System | `conception.system`, `researcher.system`, `planner.system`, `writer.system`, `reviewer.system`, `archivist.system`, `assistant.system` (the book agent, D-169). D-170 changes `conception.system` (deliverable 1 plans by phases and ordering relations, without chapter numbers unless the author asks; deliverable 4 by stage) and `archivist.system` (a 时间线 rule for entries, sub-entries and planned versus occurred; a 伏笔 rule for anchors and a direction-only `planned_payoff`) |
| Tasks | `conception.kickoff` (its `conception` variable gains `scopes[]`, `knowledge_report` and `research_tasks[]`, D-158; `research_tasks[]` gains `entries_created`, and the deliverable becomes supplementary topics, D-168), `researcher.kickoff`, `researcher.kickoff.prep` (the knowledge check, spec/07 §8.1; variable `prep`. Since D-168 the check lists topics instead of saving entries: `prep` gains `research_tasks[]` (`topic`, `requirements`, `scope_name`, `status`) and `max_topics`, and loses `max_entries`), `planner.kickoff`, `writer.kickoff.chapter` (drafting; its `feedback` branch is today's redraft), `writer.kickoff.revise`, `writer.kickoff.rollover`, `reviewer.kickoff`, `reviewer.rereview`, `archivist.kickoff.maintenance`, `archivist.kickoff.seed`, `archivist.kickoff.resync`, `assistant.kickoff` (the book agent's first message, variable `request`, D-169). D-170 changes three archivist tasks: `archivist.kickoff.seed` (planned entries from the outline with no cap, foreshadowing anchored and only where stated), `archivist.kickoff.maintenance` (step 5 promotes, nests and abandons planned entries; step 7 re-anchors; step 9 considers `complete` for goal nodes linked to a promoted entry) and `archivist.kickoff.resync` (an unsupported occurred entry is edited or deleted, never set back to planned). `writer.kickoff.redraft` is retired; existing overrides of it are inert |
| Blocks | `block.settings`, `block.canon_systems`, `block.writing_skills`, `block.memory`, `block.recent_chapters`, `block.goals`, `block.chapter_draft`, `block.cast_index`, `block.chapter_diff`, `block.anchor`, `block.book_brief`, `block.book_index` (the book agent's map of the book, variable `index`; also rendered by `book_overview`, D-169). `block.canon_systems` and `block.goals` render blank unless a canon system is off or a goal is relevant (spec/06 §10.3, §11.5). `block.memory`, `archivist.kickoff.maintenance`, `archivist.kickoff.seed` and `archivist.kickoff.resync` receive `systems`; the maintenance task also receives `goals` (D-150). Since D-170 `memory.counts` gains `event_planned` (rendered "（其中计划中 M）" when M > 0, and `event` excludes the root and abandoned entries), `block.goals` marks planned event links `（计划中）` and drops abandoned ones, and `block.canon_systems`' Timeline-off line adds that foreshadowing takes no payoff anchor |
| Notices | `notice.user_feedback`, `notice.user_message`, `notice.user_edits`, `notice.setting_edits`, `notice.completion_revoked`, `notice.tool_not_allowed`, `notice.output_truncated` (these two are rendered inside `result.error`), `notice.web_content`, `notice.nudge_generic`, `notice.research_budget`, `planner.notice.outline_rejected`, `planner.notice.nudge`, `planner.notice.retrieval_budget`, `writer.notice.revision_plan_accepted`, `writer.notice.revision_plan_rejected`, `writer.notice.granularity`, `writer.notice.nudge_drafting`, `writer.notice.nudge_revision_planning`, `writer.notice.nudge_revising`, `writer.notice.retrieval_budget` (the retrieval-budget notice of every non-planner session that retrieves: a revise session's `revision_planning`, the reviewer), `writer.rules.granularity` (partial, included by other notices). The legacy notice keys removed with the legacy planning branch are recorded in D-159; an override of a removed key is inert, like one of the retired `writer.kickoff.redraft` |
| Shared | `shared.label`, `shared.chapter_label` |
| Results | `result.error`, `result.truncated`, `result.entity_card`, `result.entity_list`, `result.relations`, `result.timeline`, `result.foreshadowing_list`, `result.search_hits`, `result.knowledge_entry`, `result.chapter_text`, `result.draft_text`, `result.draft_edit`, `result.writer_control`, `result.submit_review`, `result.canon_write`, `result.finish_maintenance`, `result.ask_user`, `result.request_review`, `result.conception_write`, `result.web_search`, `result.web_fetch`, `result.knowledge_write`, `result.target_scope` (`set_target_scope`: `scope`, `created`, `entry_count`; kept for check sessions frozen before D-168), `result.finish_research`. `result.conception_write` renders `set_research_topics` with `kept[]`, the topics it left in place, in conception and in a knowledge check alike (D-158, D-168) The book agent (D-169) adds `result.book_overview`, `result.chapter_view`, `result.setting_doc_view`, `result.characters_view`, `result.goal_list`, `result.research_list`, `result.session_list`, `result.chapter_write`, `result.goal_write`, `result.research_write`, `result.book_agent_write`, `result.session_start` and `result.change_declined` (a change the author declined in review mode), plus the `result.error` branches `chapter_locked`, `session_conflict`, `start_refused` (one sentence per `reason`), `system_off` and `goal_open_children`; its reused tools keep their result keys. The timeline tree (D-170) gives `result.timeline` items `parent_id` and `planned` (plus a line explaining the marker), `result.foreshadowing_list` items `anchor {id, title, position, planned}` or `anchor_abandoned`, `result.search_hits` event hits `planned`, and `result.canon_write` `anchor_planned` (resolve) and `abandoned_count` (abandon), plus the `result.error` branches `timeline_conflict` (one sentence per `reason`: `occurred_ancestor`, `planned_descendant`, `occurred_descendant`, `backward`) and `timeline_has_children` (`count`). `tool.update_foreshadowing` is new |
| Tools | `tool.<name>` for every tool in spec/05 §2 |

`result.error` renders every tool error from a `code` plus code-specific fields. Codes:
`invalid_json`, `validation`, `entity_not_found`, `entity_ambiguous`, `entity_exists`,
`pid_not_found`, `text_not_found`, `text_ambiguous`, `granularity`, `draft_empty`,
`isolated_readonly`, `chapter_not_found`, `chapter_not_official`, `no_knowledge_scope`,
`scope_not_found`, `scope_excluded` (`name`), `target_scope_unset`, `not_found`, `entry_exists`,
`find_failed`, `maintenance_incomplete`, `search_not_configured`, `search_failed`, `fetch_failed`,
`unsupported_content_type`, `character_not_found`, `character_exists`, `reorder_mismatch`,
`relation_not_found`, `seed_only`, `exactly_one`, `mutually_exclusive`, `tool_not_allowed`,
`output_truncated`, `after_completion`, `timeline_conflict`, `timeline_has_children` (D-170),
`system_off`, `refusal`, and anything else → the internal-error text. `refusal` is not a tool failure: it is the
provider refusing the turn, so the calls of that response are finished unexecuted and the text says
so; a model may retry the same call after revising its input. The fields per code are in the sample
contexts of `scripts/validate_prompts.py` and must match the Rust `ToolErrorKind` enum (one variant
per code).

The variables each key receives are listed in its `vars` and documented in `variables.toml`.
The Rust side has one context struct per key family (`prompts/context.rs`), serialized with
`minijinja::Value::from_serialize`. A test renders every key with each of its
`sample_contexts(key)` and snapshots the output (`insta`), so any variable mismatch fails CI.

## 3. Resolution and rendering

- `PromptSet::for_book(book_id)` builds a MiniJinja `Environment`:
  - for each key: book override → global override → shipped default;
  - `UndefinedBehavior::Strict`, `trim_blocks = true`, `lstrip_blocks = true`, no autoescape;
  - every key is registered as a template under its key name, so `{% include
    "writer.rules.granularity" %}` resolves to the effective (possibly overridden) partial.
- The environment is cached per book and invalidated on override changes for that book or global.
- Rendering output is trimmed. Three or more consecutive newlines collapse to two.
- If an **overridden** template fails to render at runtime (a branch the samples did not
  cover), the shipped default of that key is rendered instead, a warning is logged, and the
  session gets `warnings: ["prompt.override_failed"]` with the key. An override can therefore
  never stop an agent.

## 4. Overrides (API and UI)

- `GET /api/prompts?book_id=` returns, per key: metadata, the shipped default, the global
  override, the book override (when `book_id` is given), the effective source, and the variable
  docs.
- `PUT /api/prompts/:key {scope, book_id?, content}` validates, then saves:
  1. template syntax (MiniJinja compile, reported with line and column);
  2. rendering with every sample context of the key (`sample_contexts(key)`, which covers each
     branch of the shipped template, like the validator's variants) under strict undefined
     (unknown variables are errors);
  3. for `tool.*`: non-empty, at most 4,000 characters.
- `DELETE /api/prompts/:key?scope=&book_id=` resets that level.
- Saved changes apply to sessions created afterwards (spec/03 §1.3).

## 5. Authoring rules for the shipped pack (and guidance shown in the UI)

1. **System prompts are static** (no variables), so the tools + system prefix is identical across
   sessions of a role and caches well. Everything book-, chapter-, or session-specific goes into
   blocks and tasks. That includes a book's disabled canon systems: the system prompts keep
   describing all of them, and `block.canon_systems` tells the model which ones this book does not
   use (D-150).
2. **Writing language**: `block.settings` and every kickoff task state the language to write in
   (`本书写作语言：…`; the research task says `撰写语言：…` with the task's language). All prose,
   outlines, summaries, canon records, and knowledge entries must be in that language. The prompts
   themselves stay Chinese.
3. **Tool protocol** in system prompts: when to use which tool, and that drafting text goes only
   through tools, never in the reply body.
4. **Feedback priority**: author feedback and edits override everything else, including the
   outline.
5. **Notices** appended to results or sent as messages start with a bracketed title
   (`【用户反馈】`, `【系统通知】`, `【用户修改了正文】`) so the model can tell them apart from tool
   output. The exceptions are partials rendered inside other text: `writer.rules.granularity`
   and the two notices that `result.error` includes.
6. **Untrusted content**: web content is always wrapped by `notice.web_content`.
7. Keep results compact. Long data is paginated with explicit continuation hints.
8. **Embedded documents are tagged**: author- or model-written Markdown (setting documents,
   character profiles, chapter outlines, reference notes, chapter text, writing-skill bodies, goal
   purposes) is wrapped in XML-style tags, for example `<book_outline>…</book_outline>`,
   `<character rank=… name=…>`, `<chapter_outline>`, `<reference_notes>`, `<chapter_text>`,
   `<writing_skill name=… category=…>`, `<goal id=… title=…>` / `<plan …>` / `<task …>` with a
   `<purpose>` child, `<document>`, so its own headings can never be read as
   the template's structure. A tag line is always followed by a line break (the validator checks
   it).
