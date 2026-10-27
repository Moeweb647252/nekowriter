# Prompt and Context Audit — NekoWriter

**Date:** 2026-10-28
**Task:** team task-54 — inventory the shipped prompt pack, map per-session context assembly, find
contradictions and missing context, design a prompt-development loop, and design the prompt/context
alternative to a deterministic 去AI味 gate.
**Scope:** read-only. No code, schema, prompt or database change was made. The only file written is
this report.
**Method:** all eight TOML files in `prompts/` were read in full (2,312 lines); every rendered
template was measured through `python3 -I scripts/validate_prompts.py . <key>`; the context
assembly, memory, version, feedback and prompt-freeze code was read; the two prompt test files were
read; `docs/research/ai-novel-projects-survey.md` and `docs/DECISIONS.md` D-129…D-145 were read.

**Evidence tags used below:** **[V]** = verified by reading the prompt, code, schema or test in this
repository. **[I]** = inferred, or measured on a synthetic fixture rather than the author's data.

---

## 0. Executive summary

1. **The pack is exactly what the Lead measured: 8 files, 2,312 lines, 69 prompt entries, 48 tool
   entries, 227 validator renders.** Sizes quoted below are Unicode code points of the validator's
   normalised render. The Lead's baseline sizes for the five system prompts are **UTF-8 byte**
   counts: `writer.system` is 2,083 code points / 5,081 bytes today (the baseline's 5,138 is the same
   render plus the validator's 57-byte summary line). Chinese text is ~2.4 bytes per character, so
   the two measures must not be mixed in a budget calculation. **[V]**
2. **Context cost is bottom-heavy at serial scale.** The sample book used by the validator is tiny
   (outline is 9 characters), so sample kickoff totals understate every real book. On a synthetic
   150-chapter state, `ChapterWrite` is ~46,200 characters of kickoff blocks and
   `ChapterMaintenance` is ~69,600 — before system prompt and tool schemas. `block.settings` alone
   is ~35,100 characters. The writer has plenty of static setting material and very little
   *positional* context (where this chapter sits in the arc, what state it must enter and leave,
   what the POV knows, what the book's own prose sounds like). **[I]** (fixture in Appendix A)
3. **A deterministic phrase gate is not the fix, and the pack has no such gate to remove.** The only
   AI-tone instruction in the shipped prompts is the single word `AI 腔` in the reviewer's prose
   dimension (`prompts/system.toml:85`). The survey's idea #2 is the blacklist class the author
   rejected; it is evidence of the symptom, not a design to keep. See §7. **[V]**
4. **One live contradiction remains from the D-142 class.** `prompts/tasks.toml:250` still lists
   `描述发生变化` as a routine reason to call `update_entity`, one line above the D-142 rule that
   `description` is a stable reference and not a chapter log (`tasks.toml:251-252`,
   `system.toml:124`, spec/06 §6 step 2). The D-142 guard test only asserts that `120 字` appears,
   so this contradiction passes CI. See §3.1 C1. **[V]**
5. **Five highest-value changes, in order:** (1) add a `block.voice` voice-anchor block built from
   the book's own approved prose (S); (2) add an author-editable `style_guide` to `BookSettings` and
   render a structural voice contract (S); (3) clean the pack's contradictions and add negative
   guard tests (S); (4) add a structured chapter contract with entry/exit state and knowledge
   boundary (M); (5) add a thin arc/volume compass as an outline-document convention plus a writer
   excerpt (S; full arc data objects remain excluded by PROJECT §9.2). Details in §6.

---

## 1. Prompt inventory

### 1.1 What is counted

- **Entries:** 69 prompt entries (system 5, task 11, block 7, notice 21, result 23, shared 2) plus
  48 `tool.*` entries whose overridable part is the description and whose `input_schema` is a
  fixed contract (spec/09 §1). Total = 117 keys.
- **Size:** Unicode code points of the validator's normalised render (`out_norm`, trimmed and with
  3+ newlines collapsed). For multi-branch templates the first sample is shown; a range follows
  when another branch is longer.
- **When:** `system` = every request (system message); `task` = kickoff (the last block of the
  kickoff user message) unless noted; `block` = kickoff only; `notice` = appended to a tool result
  or sent as a user message when the runtime event happens; `result` = tool result / error
  payload; `tool` = every request as part of the frozen tool list.
- **Override column:** `Y` = a sensible per-book author control; `C` = technically possible, only
  wording should change (structure/protocol must be preserved); `N` = not recommended for author
  editing. Every key is technically overridable by `prompt_overrides` (spec/09 §4); this column is
  about whether an author-facing per-book edit is a good idea. **[V]**

### 1.2 Conception

| Key | Kind | When | Chars | Override |
|---|---|---|---:|---|
| `conception.system` | system | every request | 1,418 | Y |
| `conception.kickoff` | task | kickoff | 168–174 | Y |
| `notice.setting_edits` | notice | author edits a setting doc during conception | 272 | C |
| `result.ask_user` | result | after `ask_user` | 51 | N |
| `result.request_review` | result | after `request_review` | 82 | N |
| `result.conception_write` | result | after a conception write | 21 | N |

Tool list: `ask_user`, `web_search`, `web_fetch`, `write_setting_doc`, `edit_setting_doc`,
`upsert_character`, `remove_character`, `reorder_characters`, `set_research_topics`,
`request_review` (10 tools; 4,732 chars of serialized name+description+schema).

### 1.3 Researcher

| Key | Kind | When | Chars | Override |
|---|---|---|---:|---|
| `researcher.system` | system | every request | 726 | Y |
| `researcher.kickoff` | task | kickoff | 150 | Y |
| `block.book_brief` | block | kickoff only when the task belongs to a book | 62 | C |
| `notice.research_budget` | notice | budget reached | 67 | N |
| `result.knowledge_write` | result | after a save/update | 24 | N |
| `result.finish_research` | result | after `finish_research` | 78 | N |

Tool list: `web_search`, `web_fetch`, `search_knowledge`, `read_knowledge_entry`,
`save_knowledge_entry`, `update_knowledge_entry`, `finish_research` (7 tools; 3,160 chars).

### 1.4 Writer

| Key | Kind | When | Chars | Override |
|---|---|---|---:|---|
| `writer.system` | system | every request | 2,083 | Y |
| `writer.kickoff.chapter` | task | kickoff (new chapter; includes `block.chapter_draft` when a draft exists) | 678 | Y |
| `writer.kickoff.redraft` | task | kickoff (redraft) | 312–347 | Y |
| `writer.kickoff.revise` | task | kickoff (revise) | 348–466 | Y |
| `writer.kickoff.rollover` | task | kickoff (context rollover) | 429–484 | Y |
| `notice.user_edits` | notice | appended after the author edits the draft | 142–233 | N |
| `notice.completion_revoked` | notice | when feedback revokes a completion | 50 | N |
| `writer.rules.granularity` | notice (partial) | included by the notices below | 168–171 | N |
| `writer.notice.outline_accepted` | notice | outline approved → drafting | 402–405 | N |
| `writer.notice.outline_rejected` | notice | outline rejected | 86 | N |
| `writer.notice.revision_plan_accepted` | notice | revision plan accepted | 387–390 | N |
| `writer.notice.revision_plan_rejected` | notice | revision plan rejected | 102 | N |
| `writer.notice.granularity` | notice | author changes granularity | 196–199 | N |
| `writer.notice.nudge_planning` | notice | model replies without a tool while planning | 78 | N |
| `writer.notice.nudge_drafting` | notice | model replies without a tool while drafting | 244–247 | N |
| `writer.notice.nudge_revision_planning` | notice | same, revision planning | 83 | N |
| `writer.notice.nudge_revising` | notice | same, revising | 55 | N |
| `writer.notice.retrieval_budget` | notice | retrieval-call budget reached | 35 | N |
| `result.draft_text` | result | after `read_draft` | 54 | N |
| `result.draft_edit` | result | after a text write | 37–97 | N |
| `result.writer_control` | result | after `submit_outline` / plan / finish | 22–79 | N |

Tool list: 20 tools (8,040 chars): the ten retrieval tools, `submit_outline`,
`submit_revision_plan`, `read_draft`, `insert_text`, `insert_paragraphs`, `replace_text`,
`rewrite_paragraph`, `delete_paragraphs`, `finish_draft`, `finish_revision`.

### 1.5 Reviewer

| Key | Kind | When | Chars | Override |
|---|---|---|---:|---|
| `reviewer.system` | system | every request | 1,125 | Y |
| `reviewer.kickoff` | task | kickoff (review; includes `prior` for an old-version re-review) | 186–343 | Y |
| `reviewer.rereview` | task | follow-up user message after the author's comment | 113 | Y |
| `result.submit_review` | result | after `submit_review` | 56 | N |

Tool list: 11 tools (5,777 chars).

### 1.6 Archivist

| Key | Kind | When | Chars | Override |
|---|---|---|---:|---|
| `archivist.system` | system | every request | 1,530 | Y |
| `archivist.kickoff.maintenance` | task | kickoff (chapter maintenance) | 1,059–1,263 | Y |
| `archivist.kickoff.seed` | task | kickoff (canon seed) | 844 | Y |
| `archivist.kickoff.resync` | task | kickoff (re-sync after edit) | 591–741 | Y |
| `block.cast_index` | block | kickoff (maintenance / resync) | 74 | C |
| `block.chapter_diff` | block | kickoff (resync) | 75 | C |
| `result.canon_write` | result | after a canon write | 14–30 | N |
| `result.finish_maintenance` | result | after `finish_maintenance` | 90 | N |

Tool list: 21 tools (13,382 chars — the largest tool payload).

### 1.7 Shared / cross-role

| Key | Kind | When | Chars | Override |
|---|---|---|---:|---|
| `block.settings` | block | every writer/reviewer/archivist kickoff | 437 (sample) / ~35,100 (synthetic 150-chapter book) | C |
| `block.memory` | block | every writer/reviewer/archivist kickoff | 515–544 (sample) / ~10,600 (synthetic) | C |
| `block.chapter_draft` | block | chapter revise/review/maintenance, and a write with an existing draft | 202 | C |
| `block.anchor` | block | every writer/reviewer/archivist task on a side-track chapter | 149 | C |
| `shared.label` | shared | label rendering only | 2–5 | N |
| `shared.chapter_label` | shared | chapter-label rendering only | 8–11 | N |
| `notice.user_feedback` | notice | feedback on a held step | 63 | N |
| `notice.user_message` | notice | author steering message while running | 16 | N |
| `notice.tool_not_allowed` | notice | inside `result.error` | 62–79 | N |
| `notice.output_truncated` | notice | inside `result.error` | 51 | N |
| `notice.web_content` | notice | wraps every web search/fetch result | 64 | N |
| `notice.nudge_generic` | notice | model replies without a tool | 43 | N |
| `result.error` | result | every tool error | 40–90 | N |
| `result.truncated` | result | appended to a truncated result | 39 | N |
| `result.entity_card` | result | after `get_entity` | 218 | N |
| `result.entity_list` | result | after `list_entities` | 73 | N |
| `result.relations` | result | after `get_relations` | 37 | N |
| `result.timeline` | result | after `query_timeline` | 59 | N |
| `result.foreshadowing_list` | result | after `list_foreshadowing` | 26 | N |
| `result.search_hits` | result | after `search_canon` / `search_knowledge` / `search_chapters` | 56–88 | N |
| `result.knowledge_entry` | result | after `read_knowledge_entry` | 81 | N |
| `result.chapter_text` | result | after `read_chapter` | 68 | N |
| `result.web_search` | result | after `web_search` | 73 | N |
| `result.web_fetch` | result | after `web_fetch` | 69 | N |

`notice.user_edits` and `notice.setting_edits` are role-specific and appear under Writer and
Conception above; they are not duplicated here.

### 1.8 Tools (48 entries; `description` is overridable, `input_schema` is fixed)

| Tool | Roles | Desc chars |
|---|---|---:|
| `get_entity` | writer, reviewer, archivist | 84 |
| `list_entities` | writer, reviewer, archivist | 53 |
| `get_relations` | writer, reviewer, archivist | 53 |
| `query_timeline` | writer, reviewer, archivist | 71 |
| `list_foreshadowing` | writer, reviewer, archivist | 43 |
| `search_canon` | writer, reviewer, archivist | 67 |
| `search_knowledge` | writer, reviewer, researcher | 55 |
| `read_knowledge_entry` | writer, reviewer, researcher | 26 |
| `search_chapters` | writer, reviewer | 45 |
| `read_chapter` | writer, reviewer, archivist | 33 |
| `submit_outline` | writer | 39 |
| `submit_revision_plan` | writer | 47 |
| `read_draft` | writer | 37 |
| `insert_text` | writer | 77 |
| `insert_paragraphs` | writer | 67 |
| `replace_text` | writer | 63 |
| `rewrite_paragraph` | writer | 65 |
| `delete_paragraphs` | writer | 10 |
| `finish_draft` | writer | 64 |
| `finish_revision` | writer | 20 |
| `submit_review` | reviewer | 34 |
| `create_entity` | archivist | 119 |
| `update_entity` | archivist | 130 |
| `set_relation` | archivist | 221 |
| `end_relation` | archivist | 35 |
| `record_appearances` | archivist | 38 |
| `add_timeline_event` | archivist | 66 |
| `update_timeline_event` | archivist | 24 |
| `add_memory_note` | archivist | 71 |
| `retire_memory_note` | archivist | 21 |
| `add_foreshadowing` | archivist | 12 |
| `resolve_foreshadowing` | archivist | 14 |
| `set_chapter_summary` | archivist | 51 |
| `propose_setting_change` | archivist | 85 |
| `finish_maintenance` | archivist | 17 |
| `ask_user` | conception | 56 |
| `web_search` | conception, researcher | 62 |
| `web_fetch` | conception, researcher | 46 |
| `write_setting_doc` | conception | 83 |
| `edit_setting_doc` | conception | 39 |
| `upsert_character` | conception | 41 |
| `remove_character` | conception | 9 |
| `reorder_characters` | conception | 25 |
| `set_research_topics` | conception | 82 |
| `request_review` | conception | 29 |
| `save_knowledge_entry` | researcher | 39 |
| `update_knowledge_entry` | researcher | 18 |
| `finish_research` | researcher | 15 |

The **serialized tool payload per role** (`name` + `description` + `input_schema` as compact
JSON) is: writer 8,040 chars; reviewer 5,777; archivist 13,382; conception 4,732; researcher
3,160. This is paid once per request as a cached prefix, but it still occupies the context
window. **[V]**

### 1.9 Override candidacy, briefly justified

- **System prompts (Y):** the natural per-book override target; changing them changes behaviour
  without touching code. They are frozen per session, so an edit affects only new sessions.
- **Tasks (Y):** per-book workflow wording; safe because the variable contract stays fixed.
- **Blocks (C):** they render structured state the tools and tests rely on. An author may reword
  headings, but should not delete fields or change the tag structure (spec/09 §5).
- **Notices/results (N):** they encode the state machine and error protocol; a wording change can
  turn a recoverable error into an unusable one. Keep them as code-owned text.
- **Tool descriptions (C):** overridable up to 4,000 chars (spec/09 §4); the schema is fixed, and
  a description that promises a capability the tool lacks is worse than the shipped text.
- **`shared.label` / `shared.chapter_label` (N):** helper renders used by every label in the app;
  an override is global breakage, not book style.

---

## 2. Context assembly map

### 2.1 Per-session-kind assembly (sample book)

`kickoff_blocks()` is `crates/server/src/agent/context/mod.rs:45-84`. The two task templates that
include other blocks are `block.anchor` (side tracks) and `block.chapter_draft` (a write with an
existing draft). A side-track session adds 149 chars for `block.anchor`. **[V]**

| Session kind | Role | Ordered kickoff blocks | Sample kickoff chars | System chars | Tool-def chars | Sample request chars |
|---|---|---|---:|---:|---:|---:|
| conception | conception | `conception.kickoff` | 168 | 1,418 | 4,732 | 6,318 |
| research | researcher | `block.book_brief`, `researcher.kickoff` | 212 | 726 | 3,160 | 4,098 |
| chapter_write | writer | `block.settings`, `block.memory`, `writer.kickoff.chapter` | 1,630 | 2,083 | 8,040 | 11,753 |
| chapter_write (redraft) | writer | `block.settings`, `block.memory`, `writer.kickoff.redraft` | 1,264 | 2,083 | 8,040 | 11,387 |
| chapter_revise | writer | `block.settings`, `block.memory`, `block.chapter_draft`, `writer.kickoff.revise` | 1,502 | 2,083 | 8,040 | 11,625 |
| chapter_review | reviewer | `block.settings`, `block.memory`, `block.chapter_draft`, `reviewer.kickoff` | 1,340 | 1,125 | 5,777 | 8,242 |
| chapter_maintenance | archivist | `block.settings`, `block.memory`, `block.cast_index`, `block.chapter_draft`, `archivist.kickoff.maintenance` | 2,287 | 1,530 | 13,382 | 17,199 |
| canon_seed | archivist | `block.settings`, `archivist.kickoff.seed` | 1,281 | 1,530 | 13,382 | 16,193 |
| chapter_resync | archivist | `block.settings`, `block.memory`, `block.cast_index`, `block.chapter_diff`, `archivist.kickoff.resync` | 1,692 | 1,530 | 13,382 | 16,604 |
| rollover | writer | `block.settings`, `block.memory`, `block.chapter_draft`, `writer.kickoff.rollover` | 1,583 | 2,083 | 8,040 | 11,706 |

**These sample numbers are misleading at serial scale.** The sample settings are placeholders:
`settings.outline` is 9 characters long, there are 2 character designs and 2 summaries. They are
branch-coverage fixtures, not a book.

### 2.2 What each block contributes

| Block | Built by | What it contributes | Size driver / cap |
|---|---|---|---|
| `block.settings` | `settings_context()` (`context/mod.rs:424-499`) | Book title/genre/language; the three setting documents (`outline`, `world`, `protagonist`); every character design (rank, name, role label, summary, profile); attached knowledge scopes (name + description) | Unbounded document and profile lengths; this is the dominant block in a real book (~35,100 synthetic chars) |
| `block.memory` | `memory_context()` → `services/memory::snapshot` (`context/mod.rs:502-522`) | Short-term: 5 recent summaries, previous chapter's tail (800 chars), protagonist state/attributes. Mid-term: up to 40 change notes, 30 appearance rows, 10 absent-important characters. Long-term index: 15 key notes, 20 open foreshadowing, 40 cast rows, archive counts | Capped in `services/memory.rs:35-56`; ~10,600 synthetic chars |
| `block.chapter_draft` | `chapter_context()` + `draft_context()` (`context/mod.rs:197-200`, `:353-406`, `:700-731`) | Chapter label/title, outline, reference notes, full current text with pids, tail-open marker, word count | Full chapter text: ~5,460 synthetic chars for a 30-paragraph chapter |
| `block.cast_index` | `cast_index_context()` (`context/mod.rs:579-614`) | Up to 300 entities: id, name, kind, status, aliases, summary; sorted by importance then last appearance | Truncated at 300; ~17,395 synthetic chars |
| `block.chapter_diff` | `chapter_diff_context()` (`context/mod.rs:618-697`) | Paragraph-level diff between the last synced version and the current official version | Proportional to the author's edit size |
| `block.anchor` | `track_context()` (`context/mod.rs:297-338`) | Side-track canon mode and time anchor; tells a writer/reviewer/archivist what the side story may know | One short paragraph (~149 chars typical) |
| `block.book_brief` | `book_context()` (`context/mod.rs:267-294`) | Title, genre, writing language and the author's idea for a standalone research task | ~62 chars sample |

### 2.3 Same map on a synthetic 150-chapter state **[I]**

Assumptions: 12 character designs at 800-char profiles plus summaries; outline 12,000 chars,
world 8,000, protagonist 4,000; memory at its documented caps (5 summaries × 300 chars, 800-char
previous tail, 40 change notes, 30 appearance rows, 15 key notes, 20 open foreshadowing, 40 cast);
300-entity cast index; a 30-paragraph chapter (3,500 chars). The fixture script is in Appendix A;
this is a synthetic book, not the author's database.

| Session kind | Kickoff block chars | + system + tools | Spec-estimator tokens (CJK×1 + other÷4) |
|---|---:|---:|---:|
| chapter_write | 46,201 | 56,324 | ≈ 52,000 |
| chapter_revise | 51,733 | 61,856 | ≈ 57,000 |
| chapter_review | 51,422 | 58,324 | ≈ 54,000 |
| chapter_maintenance | 69,621 | 84,533 | ≈ 78,000 |
| chapter_resync | 63,729 | 78,641 | ≈ 73,000 |

Component sizes: `block.settings` 35,106 chars; `block.memory` 10,600; `block.cast_index` 17,395;
`block.chapter_draft` 5,460. The **fixed** part of the writer request (system 2,083 chars + tool
definitions 8,040 chars) is small compared with `block.settings`; the stable setting pack is the
budget problem, not the system prompt.

`context.large_kickoff` does **not** see this: `kickoff_too_large()` counts only the kickoff
blocks, not the system prompt or the tool schemas (`crates/server/src/services/sessions.rs:391-409`).
A full request can therefore be above the 50%-of-window line while no warning is emitted. **[V]**

### 2.4 What the writer does not receive for a 150-chapter serial

Concrete list, with the evidence that each is absent today:

1. **Arc / volume position.** `block.settings` carries the whole `settings.outline`; there is no
   volume/arc object and no "where this chapter sits" marker. Conception asks for `按卷或阶段划分的
   主要情节节点` in a prose document (`prompts/system.toml:161`), but nothing extracts the
   current volume. At 150 chapters the writer must locate itself in a ~12k-character outline while
   also receiving every character profile. **[V]**
2. **Chapter contract with entry/exit state.** `chapters.outline` and `chapters.reference_notes`
   are free `TEXT` (`crates/migration/src/sql/0001_init.sql:77-79`). There is no `start_state`,
   `end_state`, `must_include`, `must_avoid`, `pov`, or target boundary. The reviewer can only
   infer "what was supposed to happen"; the next chapter cannot prefill its entry state from the
   previous chapter's exit state. **[V]**
3. **Knowledge boundary (who knows what).** `add_memory_note` can store a sentence such as
   `苏婉不知道林凡身怀剑灵` (`prompts/tools.toml:361`), but there is no `known_by` / `unknown_by` /
   `revealed_chapter` structure and no POV-scoped retrieval. The writer must infer it from free
   notes and the protagonist's state; the reviewer has no query for it. **[V]**
4. **Retention / hook history.** `foreshadowing` tracks plot promises with `planned_payoff`, not
   reader-experience promises; there is no hook type, strength, cool-point, micro-payoff or debt
   ledger. The reviewer's `pacing` dimension (`prompts/system.toml:84`) is a subjective judgement,
   not a ledger. **[V]**
5. **Voice anchor.** The only prose by the author in the writer's context is the previous chapter's
   last 800 characters (`previous_tail`, `services/memory.rs:35`). No approved chapter is injected
   as a style example, and no style document exists. **[V]**
6. **A mid-range recap.** Short-term memory is `recent_summary_count` (default 5) summaries; the
   mid-term tier carries change notes, not summaries (`docs/spec/06-canon-memory.md` §4.2–§4.3).
   At chapter 150 the writer knows the last five chapters in summary and the last ten in fragmentary
   change notes; it has no 20–30 chapter digest. **[V]**
7. **Canon freshness.** If an earlier official chapter's maintenance is pending/failed, its summary
   is `（尚无摘要）`; `block.memory` gives no `canon_status` line and does not say "chapter N's canon
   has not been synced". The writer can read a missing summary as "the chapter had no summary"
   instead of "the archive is stale". **[V]**
8. **The author's recurring editorial preferences.** `author_notes` exists only in the rollover
   task (`crates/server/src/agent/context/mod.rs:890-979`); a fresh chapter write does not receive
   the feedback the author gave on earlier chapters. The same objection can be made chapter after
   chapter. **[V]**
9. **Other characters' current state beyond the protagonist.** `block.memory` gives the
   protagonist's state and mid-term change notes, not the current `state` of every character who
   appears in the coming chapter. The planning phase can query `get_entity`, but after
   `submit_outline` retrieval is disabled (`prompts/system.toml:27-28`), so the writer must have
   written it into reference notes itself. This is workable, but it is delegated to the model's
   planning discipline rather than guaranteed by context. **[V]**

---

## 3. Contradictions, duplication, dead weight, ambiguity

### 3.1 Contradictions

**C1 — The D-142 description rule is contradicted by its own outer sentence (High).**
`prompts/tasks.toml:250` says:

> 2. 实体：新出现且有意义的人物、组织、地点、物品，用 create_entity 创建；已有实体的状态、属性、身份或描述发生变化，用 update_entity 更新，并写明 note。

The very next bullet (`tasks.toml:251-252`) says `description 是稳定档案…不是章节日志…不要把它们追加到 description`, and `prompts/system.toml:124` says `它不是章节日志——本章发生的事不进这里`. Spec/06 §6 step 2 explicitly says maintenance "is not a place to extend `description` with what happened in the chapter". The outer sentence still names `描述` as a routine trigger, so a model can read it as permission to do the exact thing the sub-bullets forbid. This is the same failure class as D-142 itself; the guard test
(`prompts_registry.rs::the_entity_description_ceiling_is_stated_where_entities_are_written`) passes because it checks for the presence of `120 字`, not the absence of the contradictory trigger. **This is the single most important contradiction in the pack.** **[V]**

**C2 — The rollover feedback section always renders, even with no feedback (High).**
`prompts/tasks.toml:177` guards the drafting branch with `{% if feedback %}`. But
`rollover_context()` always passes `feedback_context(...)`, which returns a struct with two keys
`{text, issues}` and is never `None` (`crates/server/src/agent/context/mod.rs:855`,
`feedback_context` at `:739-765`). A non-empty mapping is truthy even when `text` is empty. So an
ordinary drafting rollover — the common context-overflow path — renders:
`本章是按作者的意见重写的，这些意见仍然有效：` followed by nothing. `variables.toml:75` says
`续写时为会话正在处理的意见，可能为空`, confirming the intent was a nullable value. This is a
template/context-shape mismatch, not a wording preference. **[V]**

**C3 — `block.memory` tells the model to use tools in a phase where tools are disabled (Medium).**
`prompts/blocks.toml:104` says `## 长期记忆索引（详情请用工具检索）`. In a drafting-phase rollover
the same block is included, while `writer.kickoff.rollover` says `你正处于【正文阶段】`
(`tasks.toml:176`) and the phase gate permits only `read_draft`, `finish_draft` and the five write
tools (`crates/server/src/agent/tools/registry.rs:180-182`); the notice says `检索工具已停用`
(`prompts/notices.toml:197`). A model that follows the memory block's promise gets
`tool_not_allowed` noise and may burn a turn. **[V]**

**C4 — `block.cast_index` presents itself as a complete entity index but is silently truncated
(Medium).** `prompts/blocks.toml:156` labels the block `实体索引（维护时请复用已有实体，不要重复创建）`
with no count or truncation notice. `cast_index_context` sorts by importance/last appearance and
`rows.truncate(300)` (`crates/server/src/agent/context/mod.rs:601`; `CAST_INDEX_LIMIT = 300` at
`services/memory.rs:56`). `list_entities` has `limit` max 100 and no offset (`prompts/tools.toml:25`),
so beyond 300 entities the model cannot enumerate or page the full cast. `result.entity_list`
does the opposite and says `（共 {{ total }} 个，只显示前 {{ shown }} 个…）`
(`prompts/results.toml:133`). At 150 chapters the "don't duplicate" instruction can be defeated by
the block itself. **[V]**

**C5 — The reviewer's dimensions do not match the issue taxonomy it must submit (Medium).**
`prompts/system.toml:79-86` defines one dimension `设定与连续性（canon / continuity）`, but
`submit_review.issues.category` has separate enum values `canon` and `continuity` with no
definition (`prompts/tools.toml:235`). Likewise `pacing` is one dimension covering `详略` and
`场景转换`, while the category meanings are not explained. The reviewer must choose a
category the prompt never defined, and two reviewers can classify the same issue differently.
**[V]**

**C6 — `block.settings` calls the materials "author-approved", but finalize permits unapproved
documents (Low–Medium).** `prompts/blocks.toml:17` says `以下是作者审定的设定资料，是本书创作的根本依据`.
`conception::finalize_inner` says `Unapproved items are allowed: the UI confirms`
(`crates/server/src/services/conception.rs:259-263`) and only checks that the three docs are
non-empty and a character exists. A writer told the docs are "author-approved" may over-trust a
document the author never approved. The UI confirmation may make this acceptable in practice, but
the prompt claim is stronger than the state. **[V]**

**C7 — "详细描述" naming is stale after D-142 (Low).** `result.entity_card` still prints
`详细描述：` (`prompts/results.toml:111`), while the tool schemas and archivist prompts now call the
same field `稳定档案` and explicitly say it is not a biography. The label pulls the model back
toward the pre-D-142 mental model on every `get_entity` result. **[V]**

**C8 — Memory lists are capped without saying so (Low).** `block.memory` headings
`未回收的伏笔` and `主要实体` (`prompts/blocks.toml:109-115`) render at most 20 and 40 items
(`services/memory.rs:50,53`), and `近期章节摘要` is capped by `recent_summary_count`. Only the
`档案规模` line at `blocks.toml:116` gives totals, and the model must compare list lengths to them
itself. At serial scale a cut-off list can hide the one fact that matters. Contrast
`result.entity_list`'s explicit `前 {{ shown }} 个` transparency. **[V]**

### 3.2 Duplication

| # | Instruction | Places | Risk |
|---|---|---|---|
| D1 | Planning/drafting/revision phase rules and `submit_outline` contract | `system.toml:25-33`; `tasks.toml:69-76` (and `139-142`); `notices.toml:196-201` (and `221-228`); `tools.toml:118-129` | Four copies; the phase gate and tool lists are code, the words are prompt. Any phase change needs four edits; a missed copy is a new D-142. |
| D2 | `description` ceiling and field hierarchy | `system.toml:124`; `tasks.toml:251-252`, `278-280`, `304`; `tools.toml:244`, `254`, `263`, `275-276` | Eight copies, ≈1,300 rendered chars; already drifted (C1 and C7). The guard test checks presence only. |
| D3 | Knowledge-scope 1–2 budget | `system.toml:165`; `tools.toml:569`; `tools.toml:591` | D-145 deliberately put it in three places; all three must change together. A positive guard test exists for two of them, not the third's exact wording. |
| D4 | `finish_revision` / `finish_draft` completion rules | `system.toml:31,33`; `tasks.toml:142,174`; `notices.toml:198-201,228,296`; `results.toml:261` | Six-plus copies; `chapter_end` semantics differ between granularity branches. |
| D5 | Entity lists in maintenance: `block.memory` cast (40) and `block.cast_index` (300) | `blocks.toml:112-115`, `156-160` | Both list name/kind/summary for the same entities in the same kickoff; ~duplicate cost at scale. Only `cast_index` adds aliases/status. |
| D6 | "正文阶段无法再检索" / "检索工具已停用" | `system.toml:27`; `tasks.toml:74`; `notices.toml:197,224` | Reinforces the same rule four times; no drift today, but C3 shows the rule is not applied to `block.memory`. |

Duplication is not automatically wrong — repetition is how a long system prompt keeps salience —
but this pack has no single source of truth and no negative guard tests. The D-142 incident is
evidence that "the rule is somewhere in the pack" is not enough.

### 3.3 Dead weight / stale text

- **The `档案规模` line** (`blocks.toml:116`) is a ~150-character count dump (`人物 180，组织 25，…`)
  that gives the model no decision. It is useful on the Memory page; in every kickoff it is a
  small tax. At a 150-chapter book it is noise next to lists it cannot reconcile.
- **Full `block.settings` in every session** is the dominant cost (35,106 synthetic chars) and is
  90% static. It is not "dead" — the writer cannot retrieve setting documents with any tool — but
  it is the least *per-chapter-relevant* context in the request. The budget section of §5 treats
  this as a measurement problem first, not a deletion target.
- **`writer.system:36`** (`先想清楚再写…下笔之前想清楚这一句（这一段）在场景中的作用`) is repeated in
  `writer.rules.granularity` (`notices.toml:173,175,177`), which is included in the drafting notice
  and in every nudge. It is harmless but is the kind of sentence that survives only because nobody
  owns it.
- **`reviewer.system:92`** (`不要吹毛求疵，也不要为了显得认真而凑数`) is a subjective instruction
  with no observable behaviour; it does not tell the reviewer what "凑数" is. It is a candidate for
  replacement by the voice contract / issue taxonomy rather than a standalone rule.
- **`block.settings:17`** ("author-approved") is stale relative to the finalize code (C6).
- **`result.entity_card:111`** `详细描述` is stale relative to D-142 (C7).

**Decision cross-check D-129…D-145.** D-129–D-141 are runtime/UI changes; they do not leave dead
prompt text (the held-write and draft-pane changes of D-139/D-141 are represented in code, not in a
prompt that still describes the old flow). D-142 and D-145 are the only prompt-content decisions in
the range, and both left residual drift: C1/C7 for D-142 and D3 for D-145. D-143's one-active-
relation-per-ordered-pair rule is carried only by `tool.set_relation`'s description
(`prompts/tools.toml:284`); `archivist.system` explains relation direction but not uniqueness. That
is not a contradiction because the tool description is what the model sees at call time, but it is
a single place a future edit could orphan. D-144 is cache invalidation only. No prompt text in the
range still claims a UI or workflow that no longer exists. **[V]**

### 3.4 Unresolvable ambiguity

1. **`AI 腔`** (`prompts/system.toml:85`) — the reviewer is told to detect a category that is never
   defined, has no examples, and has no relation to the book's own prose. Every model fills this
   with its own current tics. This is the central ambiguity behind §7. **[V]**
2. **`major 问题较多时`** (`system.toml:93`) — no threshold, no count, no weight. Two reviews of the
   same chapter can reach different verdicts. **[V]**
3. **`重要场景写足，过渡简洁`** (`system.toml:40`) — the outline does not mark which scene is
   "important"; the writer has no signal other than its own judgement. **[V]**
4. **`语言干净…反复出现的词语与句式`** (`system.toml:42`) — "反复" has no window, count or threshold;
   a model can always claim a word is not repeated. **[V]**
5. **`主角（即记忆中"主角当前状态"所示的人物）`** (`tasks.toml:253`) — if `block.memory` has no
   `protagonist` (no protagonist entity yet, or a seed failed), the referent does not exist. The
   careful wording shows the author of the prompt knew the link was indirect; the context does not
   guarantee it. **[V]**
6. **`不要问你可以合理决定的小事`** (`conception.system`) — no definition of "small"; an agent cannot
   tell an important product question from a detail. **[V]**

### 3.5 What is already guarded (and what is not)

| Guard | File | Covers | Does not cover |
|---|---|---|---|
| Every key renders with every sample | `crates/server/tests/prompts_registry.rs::every_key_renders_with_every_sample` | variable/undefined failures | meaning, contradictions, size |
| Rendered snapshot | `prompts_registry.rs::every_rendered_key_matches_its_snapshot` | any text change | whether the change is good |
| Spec-key coverage | `prompts_registry.rs::every_spec_key_exists_and_every_key_is_documented` | key set | — |
| Context struct shapes | `prompts_registry.rs::context_structs_match_the_sample_shapes` | renamed fields | same as above |
| D-142 description guard | `prompts_registry.rs::the_entity_description_ceiling_is_stated_where_entities_are_written` | positive presence of `120 字` | **negative presence of `描述发生变化` (C1)** |
| D-145 scope guard | `prompts_registry.rs::the_knowledge_scope_budget_is_stated_where_research_topics_are_set` | positive presence of 1–2 rule | pack-wide consistency |
| Override freeze | `crates/server/tests/m11_prompt_freeze.rs` | session keeps its frozen prompt | prompt quality |
| Validator | `scripts/validate_prompts.py` | syntax, keys, examples, every branch renders | semantics, quality, budget |

**The gap is exactly the one the task names:** there is no behavioural or quality evaluation, and
no test renders a `block.*` against a book with hundreds of entities or a 150-chapter memory. All
current tests would pass while C1–C8 are true.

---

## 4. Missing context, ranked

Rank is value / cost for NekoWriter's actual product (single author, long Chinese serial, local
app). "Context-only" means the data already exists; "new data" means a schema or UI change.

| Rank | Candidate | Type | Sessions | Added size (spec estimator) | What changes in the writer's behaviour | Cost |
|---:|---|---|---|---|---|---|
| 1 | **Voice anchor** — excerpts of the book's own approved prose | Context-only | writer (chapter/redraft/revise/rollover), reviewer | 1,200–2,000 chars ≈ 900–1,500 tokens | Shows the target rhythm, paragraph shape, dialogue density and specificity instead of forbidding symptoms; model-agnostic; updates with each approval | **S** |
| 2 | **Voice contract** — author-editable structural style specification | Context + one `BookSettings` field (JSON, no migration) | writer (all), reviewer | 600–1,200 chars ≈ 450–900 tokens | Turns "生动/有画面感/语言干净" into checkable rules: sentence variation, paragraph shape, when emotion may be named, no restatement, beat variety, concrete detail, POV limits | **S** |
| 3 | **Chapter contract** — `goal`, `pov`, `start_state`, `end_state`, `must_include`, `must_avoid`, `knowledge_boundary` | New data (chapter fields or `chapter_contracts` table) | writer planning, reviewer, maintenance | 500–900 chars ≈ 400–700 tokens | Makes the chapter boundary auditable; the next chapter can prefill its entry state from the previous exit state; the reviewer can check "did it leave the state it promised?" | **M** |
| 4 | **Arc / volume compass** — current volume goal + next 10-chapter direction | Context-only thin version (Markdown in the existing outline; no arc table); full data objects excluded by PROJECT §9.2 | writer planning, conception | 600–1,500 chars ≈ 450–1,100 tokens | Gives the writer "where am I" without reading a 12k-character book outline; prevents a chapter that is locally fine but moves no long line | **S–M** (thin) / **L** (full) |
| 5 | **Knowledge boundary** — per-fact `known_by` / `unknown_by` / `revealed_chapter` | New data (extend memory notes or a `knowledge_facts` table) | writer planning, reviewer | 300–800 chars ≈ 250–600 tokens | Stops a POV character acting on information they could not know; gives the reviewer a query instead of an impression | **M** |
| 6 | **Multi-resolution recap** — 20–30-chapter digest built from existing summaries | Context-only | writer planning, reviewer | 1,000–2,000 chars ≈ 800–1,500 tokens | Restores the shape of the last 20–30 chapters, not just the last 5; makes the middle of a long serial legible | **S** |
| 7 | **Retention / hook ledger** — hook type, strength, cool-point, micro-payoff, debt | New data (JSON column or small table) | writer planning, reviewer | 200–500 chars ≈ 150–400 tokens | Makes 追读力 visible across chapters; prevents three identical cliffhangers in a row | **M** |
| 8 | **Canon freshness line** — `canon_status` of prior chapters in `block.memory` | Context-only | writer, reviewer | 80–150 chars | Prevents a stale summary being read as "no summary"; tells the writer why a chapter is missing from memory | **S** |
| — | **Context budget rebalance** — measure and cap `block.settings` / `block.cast_index` per session | Context engineering (enabler) | writer, reviewer, archivist | saves 10k–40k chars at chapter 150 | Makes room for 1–8; without it every added block competes with 35k of static settings | **M** |

**Separate note on the ranked items:** items 1, 2, 4, 6 and 8 are genuine context engineering;
items 3, 5 and 7 need new data. Item 4's full form is explicitly out of scope (PROJECT §9.2:
"Volumes or arcs as data objects"); the thin version uses the existing `setting_docs.outline`
Markdown and a writer excerpt, not a new table. Do not describe the thin version as an arc
feature in the UI.

**Why the writer needs these at chapter 150 more than at chapter 1.** At chapter 1, `block.settings`
and a blank memory are sufficient. At chapter 150, the writer receives ~35,100 chars of static
setting material and ~10,600 chars of tiered memory, but nothing that says "you are in the third
volume, the current arc's promise is X, the POV must not know Y, and this is what the author's prose
sounds like". The absence is not a small model-quality issue; it is why long serials drift even when
each chapter's outline is good.

---

## 5. A prompt-development loop

The goal is to make prompt changes measurable without building a prompt lab. PROJECT §9.2 excludes
a prompt lab, live-data preview and A/B testing in the product; everything below is developer-side
tooling in the existing `scripts/` + cargo-test culture. **No new service and no UI.**

### 5.1 Render against a realistic book state

**What exists today.** `prompts/samples.json` is branch coverage, not state coverage: 51 keys have a
variant table, 208 sample cases in total, and all 69 prompt keys render at least once. But the
base book is one tiny fixture (`青云劫`, chapter 13, a 9-character outline, 2 summaries, 2
character designs). The validator's variant table is similar. It is enough to prove every template
branch renders; it cannot reveal size, duplicate sections or missing context at chapter 150.

**What to add.** One test-only fixture builder in `crates/server/tests/` (the existing
`agent_write_harness` already creates a real `AppState` and temp SQLite; reuse it) that seeds a
**book-state matrix** through the real services, then calls `assemble_kickoff` for each session
kind and snapshots the blocks:

| Fixture | What it proves |
|---|---|
| `first_chapter` | the "no earlier text" branches are truthful; settings incomplete is visible |
| `mid_book_40` | memory caps and normal sizes |
| `serial_150` | `block.settings`/`block.memory`/`block.cast_index` size and list truncation; `context.large_kickoff` behaviour |
| `side_isolated_anchor` | `block.anchor` and side-track visibility |
| `stale_canon` | `（尚无摘要）` vs `canon_status` behaviour |
| `english_book` | `writing_language` and the non-CJK branches |

The fixture is deterministic and uses no network. It is not a second implementation of the
templates: it calls the production `assemble_kickoff` and `PromptSet::render`. The snapshot per
fixture is the reviewable artefact. **This is a test, not a prompt lab.**

### 5.2 How to compare two prompt versions

Hold fixed: the book fixture, session kind, chapter position, model snapshot (protocol, context
window, options), temperature/seed, tool set, and the preceding message history. Vary only the
template under test (shipped vs `prompt_overrides` row, or two code branches for a new block).

Two layers of scoring:

1. **Mechanical (offline, always run):**
   - rendered chars / spec-estimator tokens per session kind and the delta vs baseline;
   - block order, presence and count; no duplicated section (e.g. chapter outline appears once);
   - no empty-variable artefacts (`{{`, `None`, `（尚未填写）` in a non-empty book);
   - required facts appear in the right block (e.g. the protagonist state is in `block.memory`);
   - tool list exactly equals the role list and schemas are unchanged;
   - cache flags: `block.settings` is the only `cache: true` block;
   - "negative" probes: forbidden phrase pairs from §3.1 are absent (e.g. a maintenance render
     must not contain `描述发生变化`).
2. **Behavioural (real provider, manually run):** see §5.3.

The comparison is paired: variant A and variant B run on the same fixture and the same seeded
input; report A/B deltas, not absolute scores. A single sample is not statistical evidence; use 3–5
generations per variant and report the distribution. Keep the eval results in
`target/prompt-eval/<template-hash>/` (untracked), with the git commit and a hash of every prompt
file in the report.

### 5.3 Offline (`ScriptedProvider`) vs real provider

**Offline, no network — belongs in `cargo test` / `scripts/check.sh`:**
- all of §5.2's mechanical layer;
- request-shape integration: `ScriptedProvider` records `bodies()`; assert the writer's request
  contains the new `block.voice`, the system prompt is the frozen one, the tools are the role's
  fixed list, and the block order matches the map in §2;
- protocol flows: script `submit_outline` → assert the drafting notice; script a write →
  assert the staged content; script `finish_draft` → assert completion. `ScriptedProvider` is a
  queue, so these are deterministic;
- override resolution and prompt freeze: create a session after a book override, assert the new
  request; create one before, assert it keeps the old one (`m11_prompt_freeze.rs` is the model);
- style *measurement* on fixed fixture text: compute sentence-length distributions and n-gram
  overlap on the book corpus, with golden numbers. This validates the diagnostic, not a model.

**Real provider, never in `cargo test`:** the repo rule is "tests must never use the network"
(AGENTS.md §5). Use a developer-only example binary, for example
`crates/server/examples/prompt_eval.rs`, which reads a provider profile from the environment/CLI
and is not built into the shipped binary. It generates prose for a fixture, computes deterministic
prose metrics against the book's own official corpus, and writes a Markdown report. It can also run
a reviewer/archivist fixture with planted defects and score detection. If the team prefers no
example binary, the same script can live in `scripts/` and call the server's CLI, but do not put it
in `cargo test`.

What needs the author's real provider: does the writer's prose actually sound like the book; does
a model update change the output; does the reviewer catch planted continuity errors; does the
archivist extract the right entities. None of these can be judged by `ScriptedProvider`; it only
returns the script we wrote.

### 5.4 Where the regression boundary sits

| Layer | Artefact | Runs in `scripts/check.sh` | Catches |
|---|---|---|---|
| Snapshot | `prompts_registry__rendered_prompt_pack.snap` and the new fixture snapshots | yes | any unintended text change; forces a reviewable diff |
| Guard test | positive invariants (D-142, D-145) and new **negative** invariants (no `描述发生变化` in maintenance; no `详情请用工具检索` in a drafting rollover; no duplicate outline) | yes | regression of explicit decisions |
| Context assembly test | fixture matrix from §5.1; budget assertions | yes | scale bugs, missing blocks, duplicated blocks, oversized kickoff |
| Offline eval | mechanical scoring only | yes (fast) | template-level quality proxies (size, duplication, ordering) |
| Live eval | real-provider generation, blind author read | **no — manual** | "is B better than A"; model-update drift |

The rule of thumb: **a snapshot says "the bytes changed"; a guard test says "a decision still
holds"; an eval says "this version is better".** Never let an eval become a CI gate: it is noisy,
model-dependent and not reproducible across providers.

### 5.5 `prompt_overrides` in the loop

- An override is a **variant**. `PromptSet::for_book` resolves `book override → global override →
  shipped default`; an eval should install a book-scope override in a temp database, create new
  sessions (prompts freeze at creation), and capture the requests. Reusing one temp DB across A/B
  is fine if each run starts a new session after setting/deleting the override.
- An override **cannot add a variable or a block**. It can only change the text of an existing key.
  Therefore a template-only voice contract can be tested as an override, but the voice-anchor
  block (new data + new context struct) needs a code branch. State this in any eval report.
- Author overrides are valuable as eval inputs: they are the author's own variant. The same
  mechanical checks apply, plus a live run. Because overrides are stored per book/global and are
  not versioned, record the exact content hash in the eval report; a later edit invalidates the
  result.
- Do **not** add a "compare overrides" endpoint or UI. The eval uses the existing `PUT/DELETE
  /api/prompts` in a temp DB or constructs the `PromptSet` directly in the test.

---

## 6. The five highest-value prompt/context changes

Ranked by expected value for this product and cost; voice work is first because it is the
author's explicit direction and the cheapest.

| # | Change | One-sentence justification | Estimate |
|---:|---|---|---|
| 1 | **Add a `block.voice` voice-anchor block** — 1,200–2,000 chars of the book's own approved prose, selected from author-touched official chapters, injected for writer/reviewer/revise/rollover. | This is the only lever that shows the target instead of naming symptoms, it costs no new author work because every approved chapter already exists, and it does not expire when the model changes. | **S** |
| 2 | **Add `BookSettings.style_guide` and render a structural voice contract** (same `block.voice`, contract section). | It makes the style author-editable in the existing Book Settings page and replaces taste adjectives with concrete rules the model can follow in any language; it is one JSON field, no migration. | **S** |
| 3 | **Fix the pack's contradictions and add negative guard tests** — C1, C2, C3, C7, the silent caps, and a "no duplicate rule" test. | These are the exact class that has already bitten twice (D-142, D-145); every one is a one-line prompt change or a test that prevents the next drift, and the fixes cost almost nothing compared with a model call. | **S** |
| 4 | **Add a structured chapter contract** (`goal`, `pov`, `start_state`, `end_state`, `must_include`, `must_avoid`, `knowledge_boundary`) to `submit_outline` and both reviewer and next-chapter kickoffs. | At 150 chapters the hard failure is state drift at chapter boundaries, and free-text outlines give the model no entry/exit state or knowledge boundary to check against; this is the smallest structured change that makes long-range continuity auditable. | **M** |
| 5 | **Add a thin arc/volume compass** — require the conception outline to contain volume headings and a current-volume goal block, then include a small "current volume" excerpt in the writer planning kickoff. | It gives the writer a position without building arc data objects (explicitly excluded by PROJECT §9.2) and attacks the largest structural gap in the survey. | **S–M** (thin) / L (full data model, out of scope) |

Not in the top five but worth a follow-up after the loop exists: multi-resolution recap (S) and
the retention/hook ledger (M). The knowledge-boundary part of change 4 is the highest-value
sub-item; if it must be split, ship the contract first with `knowledge_boundary` as free text, then
structure it.

---

## 7. Voice/style engineering — the author's ruling

### 7.1 Why the blacklist class fails

The author's ruling is the premise:

> 「去 AI 味闸门是治标不治本的，模型更新之后，之前的规则可能都没用了，我们得从提示词和上下文入手去除 AI 味」

A phrase-list gate encodes **today's** model tics. The survey's concrete example is
`show-me-the-story`'s `story-deslop` / `humanizer-zh` skills: `宛如/犹如/仿佛 > 2 per 1,000
characters → FAIL`, `微微/缓缓/徐徐/悄然 > 3 per 1,000`, `连续 3 句以上使用『主语+说道』`, `连续 3 段以人名开头`,
`段落平均长度偏差 < 20 %`, `模板化表情 > 3 per 1,000`, plus a replacement table (`宛如 → 好像`,
`瞳孔微缩 → 一愣`, `心中暗道 → 直接写心理`) — survey lines 119 and 85. Those are exactly the
lexical tics of one model generation. When the next model changes its favourite words, every rule
still passes and the prose still reads machine-written. The gate reports "clean" while the book
gets worse. That is the definition of treating symptoms.

There is a second, less obvious failure: **most of what makes Chinese web prose feel AI-written is
structural, not lexical.** The concrete list:

1. uniform sentence and paragraph length;
2. explaining an emotion immediately after showing it;
3. restating what the reader just read;
4. shaping every beat as action → dialogue → reaction;
5. generic detail where a precise, book-specific concrete detail would do.

A word list cannot see any of these. A statistical gate can partly see 1 (paragraph-length
variance) and fragments of 4 (`主语+说道`), but it cannot see 2, 3 or 5 — and even its structural
thresholds are fitted to current outputs, so they expire with the model.

**Verification against the actual reviewer prompt.** The reviewer's entire prose guidance is:

> 6. 文字（prose）：陈词滥调、重复用词、AI 腔、说教、过度概括、视角混乱等。
> — `prompts/system.toml:85`

And the verdict rule:

> 结论：存在 critical 问题，或 major 问题较多时，给出 revise；否则给出 pass。
> — `prompts/system.toml:93`

So today the reviewer is asked to detect `AI 腔` with **no definition, no examples and no reference
to the book's own prose**, and to aggregate an undefined "较多". Nothing in the pack names any of
the five structural patterns above. The only concrete style rule in the whole writer prompt is
`用中文写作时：对话使用中文引号“”，以短段为主（通常一到四句）…` (`system.toml:43`) — which is a
good start and proof that the pack can state structural rules when it tries. This is the gap the
voice contract and voice anchor fill.

**Verification against the survey.** The survey's idea #2 (lines 85, 115-125) is precisely the
deterministic gate class. It is useful evidence of the symptom — its own text says "the most
genre-specific quality problem in Chinese web fiction" and "the model will happily use the same
clichés it is supposed to flag" — but its recommendation to build a `services/style_check.rs` with
a versioned built-in phrase list is **superseded by the author's ruling**. The survey also records
the risk that matters: false positives, genre differences, "over-correction into flat prose"
(line 125). We should cite the survey for the problem and the failure modes, not re-propose its
gate.

### 7.2 What a voice contract must contain

A style specification is actionable only if a model can decide, while writing each sentence,
whether it complies. That means observable structural rules, not taste adjectives such as
`生动`, `有画面感`, `文字优美`, `节奏好`. The shipped prompt currently uses
`场景化叙事`, `节奏张弛有度`, `语言干净` (`prompts/system.toml:38-42`) — all taste judgements.

Minimum content of a voice contract (the exact wording is the author's; this is the scaffold):

1. **Sentence rhythm.** State a target distribution, not an average: short sentences below N
   characters must appear within each scene; long sentences above M must appear; three consecutive
   sentences of similar length are a defect to rewrite.
2. **Paragraph shape.** One to four sentences per paragraph normally; no paragraph over six
   sentences; do not start three consecutive paragraphs with a character name.
3. **Emotion handling.** Show the observable action/physiological detail first; do not name the
   emotion in the same paragraph; at most one explicit emotion label per scene, and only when the
   POV cannot express it through action.
4. **No restatement.** Do not summarize what the reader just read (`他知道…`, `这意味着…`); end a
   scene on an action, image or line of dialogue, not on a summary of its meaning.
5. **Beat variety.** Do not run every scene through action → dialogue → reaction; allow a scene to
   end on action, on an image, or on an unanswered line; omit a reaction where the action carries
   it.
6. **Specificity.** Every scene needs at least one detail that only this book can have — a place
   name, an object, a form of address, a number, a habit. Ban generic filler (`空气凝固`,
   `时间静止`, `心中一紧`) as a class, not as a word list.
7. **Dialogue attribution.** Distinguish speakers by diction, sentence length and address; prefer
   action beats to `说道` tags; do not use the same attribution pattern for three consecutive
   lines.
8. **POV limits.** Only what the POV can perceive and know; no entry into another character's
   mind; no narrator explaining motives the POV cannot infer.
9. **Chapter-end contract.** Do not explain the chapter's meaning at the end; close on an
   unresolved action, new information or a choice.
10. **Book-specific fields the author fills:** narrator distance (close third / first), tense,
    taboo words or images, chapter-length and paragraph-length preferences, genre conventions
    (e.g. 爽点 cadence, 章末钩子).

The rules should be written in the book's writing language or in Chinese with the model instructed
to apply them in the book language; the rest of the pack is Chinese, and `block.settings` states
`本书写作语言`.

### 7.3 Where the voice contract lives

`BookSettings` today (`crates/server/src/db/json.rs:434-445`) has **no style/voice field**:
`granularity`, `auto_pause`, `pause_after_outline`, `target_words_min/max`,
`recent_summary_count`, `midterm_window`, `auto_maintenance`. `writing_language` is a column on
`books`, not a `BookSettings` field (`0001_init.sql:38`). So the author cannot store a style
specification today.

Options considered:

| Option | Cost | Why / why not |
|---|---|---|
| Put it in `writer.system` (shipped) | zero | It is one global voice for every book; the system prompt is static by design for cache sharing and cannot use variables (spec/09 §5.1); a per-book system override can hold style, but then style is buried in a prompt editor and cannot be composed with anchors |
| Per-book `writer.system` prompt override | zero (already exists) | Works today, but invisible as a book property, unversioned, and a pack update never reaches the override; also too powerful a surface for a prose preference. Keep as the escape hatch, not the product answer |
| New `setting_docs` kind `style` | M | Requires a CHECK migration, a new tab, approval bookkeeping; the archivist does not maintain it; overkill |
| **`BookSettings.style_guide: String`** | **S** | **Best.** `books.settings` is JSON with `#[serde(default)]` (`json.rs:431-432`), so adding a field is backward compatible for old rows and needs no migration. `PATCH /books/:id` already sends the whole `BookSettings` (`api/dto/books.rs:88-90`), so the API surface does not change. The existing Book Settings page (`web/src/features/settings-materials/BookSettingsPage.tsx`) gets one textarea. The field is per book, author-owned and survives prompt-pack updates. |

Render it in a new `block.voice` (rather than editing `block.settings`) because:
- `block.settings` is the cached setting-material prefix; voice is a different concern and should
  be independently editable and inspectable;
- a dedicated block can also hold the generated voice anchors without conflating "author-approved
  setting docs" with "style instruction";
- an empty `block.voice` is skipped by `assemble_kickoff` (it skips empty renders), so books that
  do not use it pay nothing.

The smallest implementation is therefore: one new `style_guide` field, one template
`block.voice`, one context struct, one line in `kickoff_blocks`, one textarea + i18n keys. No
migration.

### 7.4 Voice anchor — the strongest lever

**The idea.** The book's own approved prose is the ground truth of the author's voice. Every
chapter the author approves adds to it for free. Instead of saying "don't use 宛如", show 1,500–
2,000 characters of what the author did write and let in-context learning do the rest. This is
model-agnostic: the anchor does not encode any model's tics, and it does not expire when the model
changes. It also carries structural information a word list cannot: sentence rhythm, paragraph
shape, dialogue density, how much explanation follows an emotion, how specific the detail is.

**Selection.** Verified data sources: `chapters.status = 'official'`, `chapters.official_version_id`
→ `chapter_versions` row `kind = 'official'`; `chapter_versions.content` is the chapter's
`ChapterContent`; `chapter_versions.kind` also has `draft`, `revision`, `manual`, `backup`
(`0001_init.sql:94-106`, `services/versions.rs`). Selection algorithm:

1. **Candidates:** official chapters of the target's track with `ord < target.ord`. For a side
   track, use the track's own official chapters; if there are fewer than two, top up from main-track
   official chapters up to the anchor (the same visibility rule as memory, D-038).
2. **Author-touched first:** for each candidate, compare its `official` version content with the
   immediately preceding `draft` or `revision` version of the same chapter. If they differ, the
   author edited the text after the agent's last submission — this is the highest-value voice
   sample. Rank `author_touched` above merely `approved`.
3. **Recency:** within the same class, prefer the most recent chapters. The author's voice evolves;
   chapters 130–145 beat chapter 3.
4. **Two windows per chapter:** choose one passage with the highest dialogue density and one with
   the lowest (narration), each contiguous paragraphs of 500–900 characters. This shows both
   dialogue attribution and narrative texture.
5. **Avoid duplication:** skip any window that overlaps `block.memory`'s `previous_tail` (last 800
   characters of the previous chapter). The anchor exists to add signal, not repeat what is
   already in memory.
6. **Fallback:** if the book has no official chapter yet, render nothing (the block is skipped).
   If only one chapter exists, take both windows from it.

Pseudocode for the server-side selection function:

```text
select_voice_excerpts(track, target_chapter) -> Vec<Excerpt>
  candidates = official_chapters(same_track, ord < target.ord)
  if candidates.len() < 2 and track.side { append main_track_official(up_to anchor) }
  score = author_touched(official) ? 2 : 1, tie-break ord DESC
  take top 2
  for each: choose dialogue_heavy_window and narration_window (500–900 chars)
  dedupe against previous_tail; cap total at 2,000 chars
  return []
```

**Size budget.** 1,200–2,000 characters ≈ 900–1,500 tokens by the spec's estimator. That is ~3–4%
of the synthetic `chapter_write` request (56k chars) and ~2–3% of `chapter_maintenance`; it is small
enough that the kickoff budget objection is not a reason to skip it. Contrast: `block.settings`
alone is 35,100 synthetic chars. The anchor budget should be a named constant
(`VOICE_ANCHOR_LIMIT = 2_000`) so an author override cannot grow it without review.

**Where and for which sessions.** A new `block.voice` included after `block.memory` in:
`chapter_write` (both normal and redraft), `chapter_revise`, `chapter_review`, and `rollover`.
Not in `chapter_maintenance`, `chapter_resync`, `canon_seed`, `conception`, or `research`: the
archivist is recording facts, not writing prose, and those kickoffs are already the largest.

**Why it is model-agnostic.** The anchor is a sample of the target distribution, not a list of
forbidden outputs. A model update changes the model's priors; the anchor still shows the same
author's prose. The same block works for a new model with no rule maintenance. When the author's
voice changes, newly approved chapters replace old anchors automatically. This is the property the
author asked for: no rule set to expire.

**What it does not do.** It cannot prove the generated prose matches; it can only bias toward it.
That is why §7.6 keeps a small drift diagnostic, not a gate.

### 7.5 Style signal from revisions

The app already produces the highest-value style data in the product: (before text, author
feedback, after text) triples from revision sessions. **They are recoverable with today's tables,
with one caveat.**

**What persists — verified:**

- `chapter_feedback` stores `chapter_id`, `content_version`, `review_report_id`, `text`, `issues`
  (JSON `ReviewIssue[]`: `severity`, `category`, `pids`, `quote`, `problem`, `suggestion`), `mode`
  (`revise`/`redraft`), `session_id`, `created_at` (`0001_init.sql:512-522`).
- `send_back` writes the feedback row and creates the session in one transaction, then sets
  `chapter_feedback.session_id` to the created revision/redraft session (`services/reviews.rs:315-365`).
- A revise/redraft session saves a `backup` version of the current text **before** the session
  starts (`services/sessions.rs:588-650`); a completed revision/draft saves a `revision`/`draft`
  version with `session_id = the feedback session` (`agent/completion.rs:39-100`).
- Therefore: **after** = `chapter_versions` where `kind IN ('revision','draft')` and
  `session_id = chapter_feedback.session_id`. **Before** = the `backup` version created at that
  session's start: the `backup` with the greatest `version_no` below the after version (and
  `created_at` at/just before the session start). The feedback text and its ticked `issues` are
  the "why".
- `chapter_versions.note` for a revision is the `finish_revision` summary ("what changed, which
  feedback item") — additional signal.

**The caveat / what needs a schema change.** `chapter_feedback.content_version` counts *every*
content change, while `chapter_versions.version_no` counts *saved versions*; they are different
sequences (`0001_init.sql:82` vs `:97`), so there is **no direct join**. The before/after
association is a heuristic based on session id + version order + timestamps, and an author's manual
version saved mid-session could confuse it. The clean fix is one small migration:
`chapter_feedback.before_version_id` and `chapter_feedback.after_version_id` (nullable FKs to
`chapter_versions`, set in the same transactions that already create the backup and the revision
version). That makes triples exact and queryable; it is **S** and does not change any prompt.

**How to distil them:**

1. **Prompt guidance digest (author-facing).** Filter feedback whose ticked `issues` include
   `category = prose` (the enum already exists in `prompts/tools.toml:235`). Count recurring
   complaints (e.g. "结尾总在解释", "对话里总用'说道'", "每段都写心理"), and produce a short
   Markdown digest. The author reviews it and pastes the accepted rules into `style_guide`. This is
   not automatic prompt injection; the author stays the editor.
2. **Before → after anchor pairs.** For each prose-category feedback row, diff the before and after
   paragraph by paragraph (pids persist within a chapter). Keep only small, localized
   within-paragraph rewrites, not whole-scene replacements. Render 1–3 pairs in `block.voice` as:

   ```text
   【作者修改示例】
   原句：……
   作者改为：……
   （对照意见：结尾太仓促）
   ```

   This is the strongest possible demonstration of the author's taste because it shows the exact
   correction, not a generic rule. It is also model-agnostic for the same reason as the anchor.
3. **A "style feedback" summary per book.** Once enough triples exist, compute a small digest
   (top recurring objections + one representative pair each) and inject it as text, not as a
   separate agent task. Keep it under 600 characters; it is a pointer, not a second canon.

**Feasibility summary:** triple reconstruction is **verified feasible today** with a
session-id/version-order query; exactness needs the two FK columns (S). Distillation into
`style_guide` is author-reviewed manual/scripted work; into anchor pairs it can be automatic with
the FK. Redraft pairs are noisy (whole-chapter rewrites); prefer revise pairs for style. Mark
`category = prose` as the filter; a `prose` issue is the closest thing to a style label the schema
already has.

### 7.6 What a check may still legitimately do

A deterministic check may remain as a **diagnostic that tells you the voice drifted**, never as the
enforcement mechanism. Its job is to answer "did this chapter move away from the book's own
baseline?", not "is this AI-written?". That distinction is what makes it model-agnostic enough to
be useful: it uses the book's own official chapters as the baseline, not a global word list.

Recommended minimal diagnostic (offline script or an ignored developer command, never a review
gate):

- **Inputs:** the last N official chapters (the book's baseline) and the new chapter.
- **Metrics:** sentence-length mean/std, paragraph-length distribution, dialogue ratio, proportion
  of paragraphs of 1–4 sentences, top-8-gram overlap with the previous 10 chapters, repeated
  paragraph openings, and a generic-filler ratio computed from a *small, author-visible* list
  (this list is a smoke alarm, not a rule set; it is allowed to be incomplete and wrong).
- **Output:** "chapter 151 is 2.7σ from your last 20 chapters in sentence rhythm; paragraph-length
  variance dropped 38%; 14% of 8-grams already appeared in the previous 10 chapters". No verdict,
  no block, no auto-rewrite.
- **Model-update use:** run it after switching a model or provider; a sudden shift in these
  statistics is evidence that the new model changed the output. That is exactly the "notice the
  drift" role.
- **Explicitly out of scope:** `AI 腔` score, AIGC ratio, auto-approval blocking, auto-rewrite, and
  a phrase blacklist. Those are the gate class the author rejected.

If the team later wants this in the app, a read-only diagnostic view would be a separate product
decision; the prompt/context work here should start with a developer script only.

### 7.7 How voice work interacts with the prompt-development loop

The voice contract and anchor are exactly the kind of change §5 must evaluate:

- **Mechanical:** assert `block.voice` appears in the right session kinds, its size is within
  `VOICE_ANCHOR_LIMIT`, its excerpts come from official chapters of the same track, and it does not
  duplicate `previous_tail`.
- **Offline:** the diagnostic metrics on fixed fixture texts; unit tests for the selection
  function (author-touched preference, side-track fallback, no spoilers after the anchor).
- **Live:** generate a chapter with and without the anchor under the same fixture/model; score
  against the book's own baseline and a short author checklist (does it sound like my book? would I
  have written this sentence?). The author's blind read is the ground truth; do not replace it with
  an LLM judge, especially not with the reviewer prompt that currently has no voice definition.
- **Regression boundary:** the anchor-selection function and block presence are guard tests; the
  quality comparison is an eval, not CI.

This also gives the author a concrete answer to "did the prompt change help?": compare the same
fixture and model with and without `block.voice`, and show the diagnostic deltas plus the generated
prose side by side.

---

## Appendix A — measurements and the synthetic fixture

### A.1 Validator

- Full pack: `python3 -I scripts/validate_prompts.py .` → `templates: 69 prompt entries, 48 tools,
  227 renders`, `OK`.
- Every key measured with `python3 -I scripts/validate_prompts.py . <key>` (the same script used to
  produce the inventory). The parser counted `len(out_norm)` in Python, i.e. Unicode code points.
- The five system prompts measured directly (summary line excluded): `writer.system` 2,083 chars
  / 5,081 UTF-8 bytes; `reviewer.system` 1,125 / 2,853; `archivist.system` 1,530 / 3,654;
  `conception.system` 1,418 / 3,670; `researcher.system` 726 / 1,818.

### A.2 Synthetic 150-chapter fixture used for §2.3

A throwaway script (not committed; `/tmp/render_synth.py` in this audit) loaded the real
`prompts/*.toml`, built the same `jinja2.Environment` settings as `scripts/validate_prompts.py`
(`trim_blocks=True`, `lstrip_blocks=True`, `StrictUndefined`), and rendered `block.settings`,
`block.memory`, `block.chapter_draft`, `block.cast_index`, and the kickoff tasks with:

- 12 character designs, each with a 40-char summary and 800-char profile;
- outline 12,000 chars, world 8,000, protagonist 4,000;
- memory at its documented caps (5 summaries × 300 chars; 800-char previous tail; 40 recent
  changes; 30 appearance rows; 15 key notes; 20 open foreshadowing; 40 cast rows);
- 300 entities in `block.cast_index`; a 30-paragraph, 3,500-char chapter;
- `target_min/max` 3,000/4,000.

The synthetic totals in §2.3 are the sum of the rendered blocks; system/tool figures are the
sample measurements. This is an illustration of scale, **not** a measurement of the author's
database or of any real book. The fixture is deliberately conservative on the outline (12k chars
for 150 chapters) — a real 200万字 serial can be larger.

### A.3 Code references read

`crates/server/src/agent/context/mod.rs` (`kickoff_blocks`, `block_context`, `memory_context`,
`cast_index_context`, `feedback_context`, `author_notes`, `build_request`),
`crates/server/src/prompts/context.rs`, `crates/server/src/prompts/pack.rs`,
`crates/server/src/services/memory.rs`, `crates/server/src/services/versions.rs`,
`crates/server/src/services/chapters.rs`, `crates/server/src/services/reviews.rs`,
`crates/server/src/services/sessions.rs`, `crates/server/src/services/conception.rs`,
`crates/server/src/agent/completion.rs`, `crates/server/src/agent/tools/registry.rs`,
`crates/server/tests/prompts_registry.rs`, `crates/server/tests/m11_prompt_freeze.rs`,
`crates/migration/src/sql/0001_init.sql`, `docs/spec/09-prompts.md`, `docs/spec/06-canon-memory.md`,
`docs/spec/03-agent-runtime.md`, `docs/PROJECT.md` §9.2, `docs/DECISIONS.md` D-129…D-145,
`docs/research/ai-novel-projects-survey.md`, `prompts/samples.json`, `scripts/validate_prompts.py`.

---

## Appendix B — one-page prioritisation (for the author)

1. **Do first (S):** add the voice contract field and the voice anchor block; fix C1/C2/C3/C7. These
   are small, reversible, and directly target the author's stated priority.
2. **Do next (M):** chapter contract + knowledge boundary; then measure it with the fixture matrix.
3. **Do when the loop exists:** arc compass thin (no new tables), multi-resolution recap, retention
   ledger.
4. **Never as a gate:** any deterministic style score. Keep it as a diagnostic only; the author's
   blind read and the voice anchor are the quality mechanism.
