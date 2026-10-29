# Implementation Plan: Layered Writing Goals and Per-System Switches

**Input:** `docs/research/goals-and-system-switches-requirements.md` (Requirement A: A1–A11,
Requirement B: B1–B7, constraints §5, questions Q1–Q9 in §6, acceptance criteria AC1–AC15 in §7).
**Date:** 2026-10-29. **Status:** plan, not implemented. The spec amendments it lists in §8 are
written (D-150); the code is not.
**Readers:** the executing agent (follow `AGENTS.md` §5 per task) and the author, who should
answer §12 before task 13.3 starts.

Every code reference was checked against the tree at `2b60d27`. Line numbers drift; function names
are the stable anchors.

---

## 0. Decisions at a glance

| Topic | Decision |
|---|---|
| **Q1** schema | One table `goal_nodes` with `level` (`goal`/`plan`/`task`) and a nullable `parent_id`. The service enforces the parent's level, and CHECK constraints back it up. Conditions are a typed JSON column (`GoalCondition[]`). Links are three join tables with cascading foreign keys, like `memory_note_entities`. |
| **Q2** scope | Canon visibility: `track_id NULL` = shared (the main story), `T` = private to isolated side track `T`. A node's chapter range and chapter condition count in its own storyline. Main-track sessions get shared nodes. Side-track sessions get their track's private nodes plus the **book-wide goals** (shared, level `goal`, no range). A parent is always in the child's scope. |
| **Q3** readiness | Computed on read by one function, `services::goals::evaluate`. The Goals API, the overview and the navigator badge all use it, so they cannot disagree. Nothing is stored, so nothing goes stale. Clients refetch on `canon.changed` and `chapter.updated`. |
| **Q4** block | Its own block, `block.goals`, after `block.memory`/`block.recent_chapters` and before `block.chapter_draft`. It carries the tree of relevant **active** nodes: title, the full purpose, chapter range, an overdue mark, and links (shown only for enabled systems). It renders blank when nothing is relevant and is then dropped. `block.settings` stays first and stays the only cache breakpoint. |
| **Q5** maintenance | A new **optional** step in `chapter_maintenance` with one new tool, `propose_goal_change` (create a task / mark a node done / drop a node). It is never required. The step and the tool exist only when the maintained chapter has relevant goal nodes. |
| **Q6** switches | B2's four effects confirmed: no injection, no maintenance, no tools, no page. They come from one system→tools and system→fragment map (§3.1). `block.memory` is assembled from fragments owned by a system or by the session (scaffolding). Each session **freezes** its system set (`agent_sessions.canon_systems`). |
| **Q7** next action | No new action kind: a first-match rule would block writing until the author acted on a goal. Instead `next_action.ready_goals` is a hint on `write_outline`, `write_chapter` and `write_next`, and the attention list gets a `ready_goals` row. |
| **Q8** migration | `0006_goals_and_switches` rebuilds `change_log` and `setting_proposals` (CHECK lists), creates the goal tables and adds `agent_sessions.canon_systems`. **No back-fill:** serde defaults give every existing book, `book_defaults` and session all five systems on. |
| **Q9** change log | `target_type` values `goal`, `plan`, `task` (the level, which is immutable), all keyed by `goal_nodes.id`. One snapshot shape, `GoalNode`. Deleting (or reverting the create of) a node that has children is refused with `409 goal.has_children`. |
| AC13 | **Upgrade guarantee:** for a book with all five systems on and no goal nodes, every kickoff, frozen tool list and system prompt is byte-identical to today. Task 13.2 pins this before any change. |
| System prompts | **Unchanged.** Switch notices and goal guidance live in blocks and tasks that render blank when unused (spec/09 §5.1). |
| `timeline_event_exists` | **Dropped.** Only an already-recorded event can be referenced, so the condition is true the moment it is written (§1, R-3). |
| Protagonist state | Belongs to **Entities**. With Entities off it is not injected (it would go stale anyway). Continuity comes from the previous tail, recent chapters, summaries and the protagonist setting document (§3.5). |
| Proposals | Reuse `setting_proposals`, adding target types `goal_node` and `new_goal_task`. The Proposals page, badge, attention row and `proposal.changed` event all stay single. |

---

## 1. Review of the requirements

Findings from checking the document against the code and the spec. Each has the resolution this
plan adopts.

| # | Where | Finding | Resolution |
|---|---|---|---|
| R-1 | §2.1, B3 | "mid-term: recent state/relationship changes (memory notes)" is wrong. `memory.recent_changes` are `change_log` rows with `target_type ∈ {entity, relation}` and a non-empty note (`services/memory.rs::recent_changes`). The whole mid-term tier belongs to **Entities**, not Memory. | The fragment table (§3.5) assigns them to Entities. With Memory off they stay. |
| R-2 | §2.1, B3, AC11 | The premise that `block.memory` carries the writer's outline is outdated. Since D-146 every write session carries the outline in `block.chapter_draft`. `block.memory` echoes it only when no `block.chapter_draft` renders (`KickoffPlan::renders_outline_elsewhere`): in `chapter_resync` kickoffs, on the Memory page, and as the IR-01 fallback when an override blanks `block.chapter_draft`. | The echo is classified as scaffolding, which keeps the IR-01 fallback working with Memory off. AC11 is proved on the write kickoff, where the outline comes from `block.chapter_draft`, and on a kickoff whose `block.chapter_draft` override renders blank. |
| R-3 | A5 | `timeline_event_exists(id)` cannot express "wait until X happens". A condition is built by picking an existing event ("not type ids", A9), so it is satisfied when written. An event with no source chapter means backstory from the setting (canon seed), not "planned", so there is no "not yet happened" state to wait on. | Dropped from the closed set. Plot gates are `chapter_at_least` and `foreshadowing_resolved`; timeline **links** (A4) stay. Planned (unnarrated) events would need Timeline semantics this change does not have (§11). Confirm in §12. |
| R-4 | A6 vs A7/AC5 | A6: "Only `active` and `waiting` nodes are injected". A7 and AC5: "a node still waiting is never injected". | Only `active` nodes are injected. A6 is read as "done and dropped never are". |
| R-5 | B3 vs AC9 | B3 asks to keep the protagonist's state with Entities off. AC9 forbids any entity-derived text with Entities off. | The state is entity data: Entities off removes it. Nothing replaces it as data. Continuity comes from `previous_tail` (Memory), `block.recent_chapters`, summaries, and the protagonist document in `block.settings`. The state would also go stale, because the archivist stops updating entities. Book settings warns when Memory is off and `recent_chapter_count = 0` (§3.9). |
| R-6 | AC11 | "contains no … archive counts" conflicts with B3's per-system counts row: with Entities on, the entity counts must stay. | Read as "no **Memory** counts" (facts and world rules). |
| R-7 | B2.2, B5, AC15 | "canon_status contributions" do not exist per system: `canon_status` is per chapter, `stale` comes only from text edits, `failed` only from a failed or abandoned session. The real per-system hazard is `finish_maintenance`'s required steps (`record_appearances`, `set_chapter_summary`) and the protagonist confirmation. If their tools are absent, they produce `error.maintenance_incomplete` until the tool-error streak pauses the session or nudges fail it. | Required steps follow the session's **frozen** systems (§3.7). Re-enabling a system never changes any chapter's `canon_status`. |
| R-8 | — (gap) | Overview rule 2 offers `seed_canon` while the book has character designs, no entities and no completed seed. With Entities off that stays true forever. | Rule 2 requires Entities on. `finalize` starts `canon_seed` only when at least one of Entities, Timeline, Memory, Foreshadowing is on (§3.8). |
| R-9 | — (gap) | `search_canon` reads four systems at once, so "remove its tools" does not say what happens to it. | Offered while any of its four systems is on. Kinds of disabled systems are filtered out of every query, using the session's frozen set (§3.4). |
| R-10 | B2.6, AC14 | A rollover child is a *new* session that copies its parent's frozen prompt and tools (`agent/context/mod.rs` rollover). If its kickoff read the book's *current* switches, it could disagree with its own frozen tools. | The system set is frozen per session and inherited by rollover children (§3.3). |
| R-11 | §5, A10 | The requirements name the `change_log` rebuild. Reusing the proposal system (A8: "the existing proposal/approval shape") also needs a `setting_proposals` rebuild for its `target_type` CHECK. | Both rebuilds go in 0006. Neither table is referenced by another table (F9), so both are simpler than 0004. |
| R-12 | A7 | "If a node's chapter range is absent, the node is considered relevant from the current chapter onward until it is done" has no reference point. | A range-less active node is relevant at every chapter. A range with only `to` behaves the same until done. |
| R-13 | A6 | "Marking every task of a plan done surfaces the plan as ready to close" does not cover dropped tasks, goals, or open children of a closed parent. | `closable` = an active parent with ≥ 1 child, no open child and ≥ 1 done child, at every level. The children of a done or dropped parent are not injected (§4.5), and the closing confirmation says so. |
| R-14 | A8 | "a note when a task looks unreachable" has no effect on acceptance, but the proposal shape is accept/reject. | The note is a `drop` proposal whose rationale carries it: accept drops the node, reject keeps it. Confirm in §12. |
| R-15 | A9 | "Entity, timeline-event and foreshadowing **detail pages**": only entities have a detail pane. Events and foreshadowing are rows with edit forms. | "Referenced by" goes in the entity detail pane and in the event and foreshadowing edit dialogs. |
| R-16 | §2.4 | "Book settings lives in the settings area" is wrong: it is the book workspace route `books/:id/book-settings`, linked from the navigator. | Noted. The switches live there (B6). |
| R-17 | AC13 | "behaves identically" is not testable as written. | Made strict and pinned: §0 AC13 row, task 13.2. |
| R-18 | — (gap) | Chinese vocabulary collides with the prompt pack. 「任务」 already means the session's task (`block.memory`'s 「当前任务」, every kickoff header 【…任务】). 「计划」 is the writer's 修订计划. | Recommend 目标 · 阶段计划 · 情节点 in prompts and the zh-CN UI. Confirm in §12. |
| R-19 | — (gap) | Deleted or merged canon referenced by goals, chapter renumbering, and track deletion are not covered. | §4.9. |

The rest was checked and holds: the block list of §2.1, the ten maintenance steps of §2.2, the
`BookSettings` fields of §2.3, migration number 0006, and the rule that the `change_log` CHECK needs
a rebuild.

---

## 2. Facts this plan rests on

Verified in the code at `2b60d27`. Cited elsewhere as F1–F18.

1. **`block.memory` fragments** (`prompts/blocks.toml`, `services/memory.rs::snapshot`):
   short-term = task label, `is_first` lines, summaries, previous tail, protagonist, outline echo;
   mid-term = `recent_changes`, `appearances`, `absent_important`; long-term = `key_notes`,
   `open_foreshadowing`, `cast`, and the `counts` row.
2. **Outline echo** (R-2): `KickoffPlan::renders_outline_elsewhere` is true for outline sessions and
   for write/revise/review/maintenance whenever `block.chapter_draft` renders.
3. **Frozen tool lists are the gate.** `registry.rs::tools_for_role` returns one fixed list per role.
   `is_permitted` answers any unfrozen tool with `error.tool_not_allowed` and no effect, so a static
   system prompt that names a removed tool cannot cause anything.
4. **System prompts name canon tools.** `planner.system` lists `get_entity`, `get_relations`,
   `query_timeline`, `list_foreshadowing`. `reviewer.system` cites `get_entity` and
   `query_timeline` as examples. `archivist.system` describes every system. They are static
   (spec/09 §5.1).
5. **`finish_maintenance`** (`agent/tools/archivist.rs`) requires `record_appearances` and
   `set_chapter_summary` calls in `chapter_maintenance`, and withholds once when the protagonist
   appeared and their `state` was not touched.
6. **Overview rule 2** (`services/books.rs::next_action`): no completed `canon_seed`, designs > 0,
   entities = 0 → `seed_canon`.
7. **Rollover** builds its own block list (`settings · writing_skills · memory · chapter_draft ·
   writer.kickoff.rollover`) and copies the parent's frozen system prompt and tools.
8. **Session creation** (`services/sessions.rs`) reads the book's `BookSettings` once and freezes
   `system_prompt` and `tools` from `tools_for_role(role)`.
9. **Foreign keys**: `change_log` is referenced only by itself (`reverts_id`, `reverted_by_id`).
   Nothing references `setting_proposals`.
10. **Proposals**: `setting_proposals.target_type ∈ {setting_doc, character_design, new_character}`.
    `ProposalDto` carries a text `preview`. `result.finish_maintenance` counts every proposal row
    of the session as a 设定修改建议.
11. **`BookSettings`** is `#[serde(default)]` at struct level. `PATCH /books/:id` replaces it as a
    unit. The Book settings page edits a draft and saves the whole form with one **Save** button.
12. **`search_canon`** kinds: `entity`, `note`, `foreshadowing`, `event`, `summary`
    (`search.rs::CANON_KINDS`).
13. **Demo provider**: `maintenance_plan` is a fixed call sequence. `DemoHints.tools` carries the
    frozen tool names, and the planning arm calls `get_entity` on the protagonist.
14. **Snapshots**: `agent_write_context__writer_kickoff_{first,later}_chapter.snap` pin the write
    kickoff. `prompts_registry__rendered_prompt_pack.snap` pins every key's samples. No other kind's
    kickoff is pinned yet.
15. **SSE**: `canon.changed` invalidates `keys.canon.all`, the change log and the overview.
    `chapter.updated` invalidates neither canon keys nor the overview (`web/src/api/events.ts`).
16. **Entity merge** moves note, foreshadowing and event links with `move_link_rows`, without
    per-row changes (`services/canon/entity.rs`).
17. **`shared.label`** has a `target_type` enum, used by `error.not_found`.
18. The chapter page never shows `chapters.summary`. Only the Memory page does.

---

## 3. Design — per-system switches (Requirement B)

### 3.1 What each system owns

| | Entities | Timeline | Memory | Foreshadowing | Goals |
|---|---|---|---|---|---|
| Tables | `entities`, `relations`, `appearances` | `timeline_events`, `timeline_event_entities` | `memory_notes`, `memory_note_entities`, `chapters.summary` | `foreshadowing`, `foreshadowing_entities` | `goal_nodes` + three link tables |
| Tools | `get_entity`, `list_entities`, `get_relations`, `create_entity`, `update_entity`, `set_relation`, `end_relation`, `record_appearances` | `query_timeline`, `add_timeline_event`, `update_timeline_event` | `add_memory_note`, `retire_memory_note`, `set_chapter_summary` | `list_foreshadowing`, `add_foreshadowing`, `resolve_foreshadowing` | `propose_goal_change` |
| `search_canon` kinds | `entity` | `event` | `note`, `summary` | `foreshadowing` | — |
| Context | protagonist state, mid-term tier, cast, entity counts, `block.cast_index` | event count | `is_first` lines, summaries, previous tail, key notes, fact/rule counts | open hooks, open-hook count | `block.goals` |
| Maintenance steps | 1 appearances, 2 entities, 3 protagonist state, 4 relations | 5 timeline | 6 notes, 8 summary | 7 foreshadowing | new goals step |
| Seed steps | 1–3 | 5 | 4 | 6 | — |
| UI | Entities page, entity chips elsewhere | Timeline page | Memory page | Foreshadowing page | Goals page, "Referenced by" sections, ready badge, overview hint |

These belong to **no** system and are never switched off: `block.settings`, `block.writing_skills`,
`block.recent_chapters` (a length setting, B5), `block.chapter_draft`, `block.chapter_diff`,
`block.anchor`, `read_chapter`, `search_chapters`, the knowledge tools, `propose_setting_change`,
`finish_maintenance`, and the Changes and Proposals pages.

The map lives in one place in Rust (`agent/tools/registry.rs`: `system_of_tool`,
`CanonSystem::search_kinds`; `agent/context/mod.rs`: `system_of_block`). The prompt validator holds
the same tool map as data.

### 3.2 Storage and defaults

`BookSettings.canon_systems: CanonSystems { entities, timeline, memory, foreshadowing, goals }`,
all `bool`. The struct is `#[serde(default)]` and its `Default` is all `true`. So:

- an existing `books.settings` or `app_settings.book_defaults` row without the field loads with all
  five on;
- a partial object (`{"goals": false}`) loads with the missing fields on;
- a new book copies `book_defaults` (B4).

`PATCH /books/:id` validates nothing new: every combination is legal, including all off. Nothing
is back-filled; a row gains the field the next time its settings are saved.

### 3.3 Freezing per session (B2.6, AC14)

- **New column `agent_sessions.canon_systems`** (`CanonSystems` JSON, nullable). Session creation
  copies the book's `settings.canon_systems` into it. `NULL` (sessions created before 0006,
  book-less sessions) means all on.
- **Every runtime decision about systems reads the session's value, never the book's current
  settings**: the frozen tool list, the kickoff blocks and fragments, `search_canon`'s kind filter,
  and `finish_maintenance`'s required steps.
- **Rollover** copies the parent's value to the child, and the child's kickoff renders with it, so
  the child's context matches its inherited frozen tools.
- Retrying maintenance continues the same session (frozen). A new session (Retry after abandon,
  re-review on a changed text, the next chapter) reads the current settings.

### 3.4 Tool lists (B2.3)

- Each role keeps **one fixed, ordered list** (D-007). A session freezes that list **minus the tools
  of its disabled systems**, in the same order. Two extra rules:
  - `search_canon` is removed only when Entities, Timeline, Memory and Foreshadowing are all off.
  - `propose_goal_change` is frozen only into a `chapter_maintenance` session whose systems include
    Goals **and** whose chapter has relevant goal nodes (§4.6).
- The archivist's list gains `propose_goal_change` between `set_chapter_summary` and
  `propose_setting_change` (22 tools). Today's book (all on, no goal nodes) therefore freezes exactly
  today's 21.
- **`search_canon`** uses `requested kinds (or all) ∩ enabled kinds`. When the intersection is empty
  the result is the ordinary empty-hits result, and `block.canon_systems` has already told the model
  why (§3.6). The schema's `kinds` enum is unchanged; tool schemas are not overridable.
- Calls to a tool that is not frozen keep today's answer: `error.tool_not_allowed`, no effect.
- **Prompt caching:** tool lists now differ between books with different switches, and between
  maintenance sessions with and without goals. Within a book the planner, writer and reviewer
  prefixes stay identical across sessions. This cost is accepted.

### 3.5 Splitting `block.memory` (B3)

The block keeps its key, so overrides keep working. It gains the variable `systems`, and every
fragment is guarded by its owner. The snapshot builder skips the queries of disabled systems.

| Part of today's `block.memory` | Owner |
|---|---|
| `# 记忆` block header | scaffolding (always rendered: the block always has scaffolding) |
| `## 短期记忆` tier header | scaffolding |
| 当前任务 line (chapter label and title) | **scaffolding** |
| `is_first` lines (first chapter / none official yet) | Memory |
| recent chapter summaries | Memory |
| previous chapter's tail | Memory (AC11) |
| protagonist (focus character) state, status, attributes, `after_anchor` | **Entities** (R-5) |
| chapter outline echo | **scaffolding** (R-2) |
| `## 中期记忆（近 N 章）` tier header | Entities (every mid-term part is Entities) |
| recent state and relationship changes, including the explicit "none recorded" line | **Entities** (R-1) |
| character appearance records | Entities |
| important characters absent for long | Entities |
| `## 长期记忆索引` tier header | rendered when any of Memory, Foreshadowing, Entities or Timeline is on |
| key facts and world rules | Memory |
| unresolved foreshadowing | Foreshadowing |
| main cast | Entities |
| archive counts row | per-fragment: entity kinds → Entities; events → Timeline; facts and world rules → Memory; open hooks → Foreshadowing; joined in the template; omitted when no fragment renders |

Rules:

- A tier header renders only when something under it renders. The block never renders blank: the
  scaffolding is always there.
- **Byte-identity:** with all four canon systems on, the block renders exactly today's text, counts
  row included. The kickoff snapshots of task 13.2 prove it.
- The Memory page (`GET /books/:id/memory`) renders with the book's current systems, so it still
  shows exactly what the next session receives. `MemoryViewDto` gains `systems`, so the page can
  omit the sections of disabled systems.
- `block.cast_index` belongs to Entities. The kickoff skips it when Entities is off (§3.1
  `system_of_block`), with no template change.

### 3.6 `block.canon_systems` — telling the model

System prompts stay static (F4), so the model needs one sentence about what is off. **New block
`block.canon_systems`** (vars `systems`), emitted right after `block.settings` in every chapter and
seed kickoff (§5). It renders **blank when Entities, Timeline, Memory and Foreshadowing are all
on**, so today's kickoffs keep their shape. Otherwise it names the canon systems this book does not
use, says:

- their records are not in the context;
- their tools are not in the tool list;
- `search_canon` will not return their kinds;
- `entities` parameters of other tools must be left empty while Entities is off.

Goals is not listed: no static text refers to it.

### 3.7 Maintenance, seeding, re-sync (B2.2, B5, AC15)

- **`archivist.kickoff.maintenance`** gets `systems` and `goals`. Each numbered step renders only
  when its system is on (§3.1), and the steps are renumbered contiguously. The goals step sits
  before the setting-proposal step and renders only when `goals` is non-empty (§4.6). The isolated
  track's "record the shared protagonist as a track note" clause needs Memory too. With all on and
  no goals, the text is today's.
- **`finish_maintenance`** requires `appearances` only when the session's systems include Entities,
  and `summary` only when they include Memory. The protagonist confirmation runs only with
  Entities. A disabled system can therefore never hold a chapter in `syncing` or push it to
  `failed`.
- **`archivist.kickoff.seed`** gets `systems`: entity, relation and character steps (Entities),
  world rules and facts (Memory), backstory events (Timeline), planned hooks (Foreshadowing).
  `finalize` starts `canon_seed` only when at least one of those four is on.
- **`archivist.kickoff.resync`** gets `systems`. Its bullet list names only enabled kinds, and the
  `record_appearances` and summary bullets need Entities and Memory respectively.
- **All five off** is legal. Maintenance still runs when `auto_maintenance` says so (B5). It is left
  with setting proposals and `finish_maintenance`, completes, and leaves the chapter `synced`, so
  `previous_not_ready` never blocks the next chapter.
- **No back-fill on re-enable.** Chapters maintained while a system was off keep `synced`. The
  re-enabled system resumes from the next maintained chapter. The switch's hint says so (§3.10).
  Re-running old chapters would regress current state (an older chapter's protagonist state would
  overwrite a newer one).

### 3.8 Overview and finalize

- Rule 2 (`seed_canon`) applies only when Entities is on.
- `attention[]`, rules 4–8 and `canon_status` are unchanged (R-7).

### 3.9 Interaction with existing settings (B5)

| Setting | Relation to the switches |
|---|---|
| `recent_chapter_count` | Independent: it sets how much prior official prose is injected and belongs to no system. It stays in "Memory windows". With Memory off it is the only previous-chapter text, so the switch row warns (non-blocking) when Memory is off and `recent_chapter_count = 0`. |
| `recent_summary_count` | Only Memory reads it: the input is disabled, with a hint, while Memory is off. |
| `midterm_window` | Only Entities reads it (the mid-term tier, `get_entity`'s window counts): disabled while Entities is off. |
| `auto_maintenance` | Independent: when maintenance runs, not what it maintains. |

### 3.10 UI (B4, B6)

- **Book settings › Canon systems**, a section of its own above "Memory windows":
  - five switches, each with a one-line effect (`book:canon_systems.<system>_hint`), in the form
    draft and saved with **Save** like `auto_maintenance`. No page reload is needed: the save
    invalidates the book and the navigator re-renders.
  - A switch the draft turns **off** shows what is kept, from
    `GET /books/:id/canon/counts` ("42 entities, 30 relations and 120 appearances stay in the book.
    Agents stop reading and maintaining them."). It offers no destructive action.
  - A disabled system shows **View data**, a link to its page.
  - The section footnote: turning a system off deletes nothing; sessions already running keep what
    they started with; chapters approved while a system is off are not recorded into it later.
- **Navigator** hides the canon entries of disabled systems. The Changes and Proposals entries stay.
  QuickSwitch omits the hidden pages.
- **A disabled system's page** (reached through View data or a deep link) renders normally under a
  banner: "This system is off for this book: agents neither read nor maintain it." with **Turn on**
  (a PATCH of that one flag). Edits on the page keep working (the data is the author's).
- **Goals off** also hides the "Referenced by" sections, the ready badge, the overview hint and the
  `ready_goals` attention row.

---

## 4. Design — writing goals (Requirement A)

### 4.1 Schema (Q1)

```sql
CREATE TABLE goal_nodes (
  id            INTEGER PRIMARY KEY,
  book_id       INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  track_id      INTEGER REFERENCES tracks(id) ON DELETE CASCADE,      -- NULL = shared (main story); T = isolated side track T
  level         TEXT NOT NULL CHECK (level IN ('goal','plan','task')),
  parent_id     INTEGER REFERENCES goal_nodes(id) ON DELETE CASCADE,  -- services refuse to delete a node with children;
                                                                       -- CASCADE only acts in a book or track delete
  title         TEXT NOT NULL CHECK (length(trim(title)) > 0),
  purpose       TEXT NOT NULL CHECK (length(trim(purpose)) > 0),
  status        TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','waiting','done','dropped')),
  conditions    TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(conditions)),   -- GoalCondition[]
  chapter_from  INTEGER CHECK (chapter_from IS NULL OR chapter_from >= 1),
  chapter_to    INTEGER CHECK (chapter_to IS NULL OR chapter_to >= 1),
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL,
  CHECK (level <> 'goal' OR parent_id IS NULL),
  CHECK (chapter_from IS NULL OR chapter_to IS NULL OR chapter_to >= chapter_from),
  CHECK (status <> 'waiting' OR json_array_length(conditions) > 0)
);
CREATE INDEX goal_nodes_book ON goal_nodes(book_id, status);
CREATE INDEX goal_nodes_parent ON goal_nodes(parent_id);
CREATE TABLE goal_node_entities (
  node_id    INTEGER NOT NULL REFERENCES goal_nodes(id) ON DELETE CASCADE,
  entity_id  INTEGER NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
  PRIMARY KEY (node_id, entity_id)
);
CREATE INDEX goal_node_entities_entity ON goal_node_entities(entity_id);
-- goal_node_events (node_id, event_id → timeline_events) and
-- goal_node_foreshadowing (node_id, foreshadowing_id → foreshadowing): the same shape and index
```

**Why one table.**
- The three levels share every field, rule, link, condition, change-log path and UI row. Three
  tables would triple the CRUD, the DTOs, the join tables (nine) and the revert arms for one
  `level` check.
- The parent-level rule (plan → goal, task → plan) is a service invariant, like every other
  cross-row invariant here (spec/02 §5). The CHECK constraints catch the cheap cases: a goal with a
  parent, an inverted range, `waiting` without conditions, a blank title or purpose.
- A single `parent_id` makes "at most one parent" structural (AC2): attaching a task to another
  plan *moves* it.

**Why conditions are JSON and links are join tables.** Links need reverse lookups ("Referenced
by", the entity merge) and cascade with their targets, like every other entity link. Conditions are
a short typed list read as a whole. A deleted target has to stay visible as a **broken** condition,
not disappear: a cascade would silently make a waiting node ready.

### 4.2 Scope and storylines (Q2)

- `track_id` follows canon visibility (spec/06 §2): `NULL` = shared, `T` = private to isolated side
  track `T`. Canon-mode side tracks have no private nodes, as with every other canon row.
- A node's **storyline** is the main track when shared, `T` when private. `chapter_from`,
  `chapter_to` and `chapter_at_least` are chapter numbers in that storyline. They are **positions,
  not chapter references**: they may name chapters that do not exist yet, and they do not follow a
  reorder or delete.
- **Parent, links and conditions are scope-checked:** the parent is in the same scope (a shared
  child has a shared parent, a `T` child a `T` parent). Linked and conditioned rows must be visible
  from the node's scope. A shared node can only reference shared canon.
- **Injection** for a session on chapter *k* of track *S*:

  | *S* | Candidates |
  |---|---|
  | main | shared nodes |
  | isolated side track `T` | `T`'s private nodes, plus **book-wide goals**: shared, level `goal`, no chapter range |
  | canon-mode side track | book-wide goals only |

  Then the relevance rules of §4.5. **Leakage is impossible by construction:** a main-track session
  never reads a private row, and a side track never reads main-story plans, tasks or ranged goals.

### 4.3 Field rules and lifecycle (A2, A6)

- `title`: non-empty after trimming, ≤ 120 characters. `purpose`: non-empty after trimming
  (Unicode whitespace), ≤ 1,000 characters. It is never generated by code: the author types it, or
  an accepted proposal carries the agent's text (A2).
- `level` and `track_id` are fixed at creation. Changing either is `400 validation.failed`.
- **Parents:** a goal has none; a plan has none or a goal; a task has none or a plan. Tasks
  therefore have no children.
- **Status** is `active` (default), `waiting`, `done` or `dropped`, freely editable by the author.
  - `waiting` needs ≥ 1 condition. Conditions are kept when a node leaves `waiting`, but only a
    waiting node's conditions are evaluated.
  - **Closing a node with open descendants:** setting `done` or `dropped` on a node with an
    `active` or `waiting` descendant is `409 goal.open_children`. `details.children` lists each
    open descendant `{id, level, title, status}`. The request succeeds when it repeats with
    `confirm_open_children: true`. The descendants keep their status, but a node under a closed
    ancestor is no longer injected (§4.5), and the confirmation says so.
- **Delete** a node that has children (any status): `409 goal.has_children`
  (`details.children_count`). Move or delete the children first.
- **Derived, never stored:**
  - `ready` = `waiting` and every condition satisfied (§4.4);
  - `closable` = `active`, ≥ 1 child, no open child, ≥ 1 `done` child (R-13).

### 4.4 Conditions and readiness (A5, Q3)

```ts
type GoalCondition =
  | { kind: "chapter_at_least"; chapter: number }                      // ≥ 1, in the node's storyline
  | { kind: "foreshadowing_resolved"; foreshadowing_id: number }
  | { kind: "entity_status_is"; entity_id: number; status: EntityStatus };
```

| Kind | Satisfied when | Broken (cannot become true without an edit) when |
|---|---|---|
| `chapter_at_least` | the storyline has an official chapter numbered ≥ `chapter`. "Reached" includes a later chapter approved first after a forced write. | never |
| `foreshadowing_resolved` | the row exists, is visible from the node's scope, and is `resolved` | missing, invisible, or `abandoned` |
| `entity_status_is` | the entity exists, is visible, and has that `status` | missing or invisible |

- At most 10 conditions per node; duplicates are dropped. Targets are checked when written (exist,
  visible), and broken again only if the target later disappears.
- **One evaluator**, `services::goals::evaluate(conn, book_id, nodes)`, returns per node:
  - `ready` and `closable`;
  - per condition: `satisfied`, `broken`, and the target's stored name or title (`target_label`),
    which the UI uses to compose "Waiting for 「玉佩之谜」 to be resolved" (A5: the author's
    language, not ids).

  Callers: the goals list API, the overview (attention row and hint), and through the list the
  navigator badge.
- **Invalidation:** nothing is materialized, so the server never has a stale flag. The goals query
  keys live under `keys.canon`, so `canon.changed` (any canon or goal write, a revert, a hook
  resolution, an entity status change) already refreshes them. `chapter.updated` additionally
  invalidates `keys.canon.goals(book)` and the overview (F15), which covers `chapter_at_least` on
  approval. AC3 and AC4 need no manual refresh.
- Ready nodes are not activated automatically (A6: `ready` is not a status). The author activates
  them; the Goals page and the overview surface them (§4.10, §4.11).

### 4.5 Injection: `block.goals` (A7, Q4)

**Relevance at chapter *k*** (the session's chapter number in its track). Among the candidates of
§4.2, a node is injected when:

1. it is `active`;
2. `chapter_from` is NULL or ≤ *k* — a range that ended before *k* still counts and is marked
   overdue (A7: "covers or precedes");
3. no ancestor is `done` or `dropped`.

Then each injected node's **active** ancestors are added, so a task brings its plan and its goal
(AC5). A `waiting` node is never rendered, not even as an ancestor; a task whose plan waits renders
at top level.

**Rendering** (`block.goals`, vars `goals`, `systems`):
- A short preface. These are the author's writing goals relevant to this chapter; each **purpose**
  says why the node exists and what it is for. The outline and the prose must serve the purposes of
  the tasks. A purpose is achieved through the story and never stated in the prose. The reviewer
  judges whether the chapter serves them. The archivist refers to nodes by id.
- The tree in nested tags, `<goal>` / `<plan>` / `<task>`, each with `id`, `title`, and a
  `chapters` range plus an `overdue` flag when present, and a `<purpose>` child (spec/09 §5.8:
  author-written text is tagged).
- Links on one line each, **only for enabled systems** (entity names `名字(#id)`, events `#id
  标题`, hooks `#id 标题`), so Entities off leaves no entity name here either (AC9).
- Order: goals, then plans, then tasks under their parents. Siblings sort by `chapter_from`
  (NULL first), then `id`. Top-level plans and tasks follow the goals.
- **Cap:** 30 rendered nodes. Tasks are kept by earliest `chapter_from`, each with its ancestors,
  and the block states how many relevant nodes were left out.
- Blank when no node qualifies; the kickoff then drops it, as for every blank block.

**Where:**

| Kickoff | Blocks (new ones in bold) |
|---|---|
| `chapter_outline` | settings⁎ · **canon_systems** · memory · recent_chapters · **goals** · `planner.kickoff` |
| `chapter_write` | settings⁎ · **canon_systems** · writing_skills · memory · recent_chapters · **goals** · chapter_draft · `writer.kickoff.chapter` |
| `chapter_revise` | settings⁎ · **canon_systems** · writing_skills · memory · **goals** · chapter_draft · `writer.kickoff.revise` |
| `chapter_review` | settings⁎ · **canon_systems** · writing_skills · memory · **goals** · chapter_draft · `reviewer.kickoff` |
| `chapter_maintenance` | settings⁎ · **canon_systems** · memory · cast_index · **goals** · chapter_draft · `archivist.kickoff.maintenance` |
| `canon_seed` | settings⁎ · **canon_systems** · `archivist.kickoff.seed` |
| `chapter_resync` | settings⁎ · **canon_systems** · memory · cast_index · chapter_diff · `archivist.kickoff.resync` |
| rollover | settings⁎ · **canon_systems** · writing_skills · memory · **goals** · chapter_draft · `writer.kickoff.rollover` |

- Revise and rollover get the goals too. A revision must keep serving the purposes, and a drafting
  rollover must not lose them halfway through a chapter. Re-sync does not: it realigns canon with
  edited text, and goal proposals there would duplicate the maintenance run.
- `block.goals` sits after memory and recent chapters, so the purposes are the last context before
  the chapter itself and its task.

### 4.6 Archivist: the goals step and `propose_goal_change` (A8, Q5)

- **Gate:** `chapter_maintenance`, Goals on in the session's systems, and a non-empty `block.goals`
  node list for the chapter (the §4.5 set). Under the gate:
  - the tool is frozen (§3.4);
  - `block.goals` renders;
  - the task's goals step renders. Its text: compare the chapter with the goals block; propose
    `complete` for a node whose purpose this chapter achieved, `drop` (with the reason) for one this
    chapter made unreachable, and `create` for a new task where the chapter opened an opportunity
    under a listed plan. These are proposals for the author. Skip the step when the chapter moved
    no goal.

  Without the gate, none of the three exists, which is what keeps an unchanged book identical
  (AC13).
- **Never required:** `finish_maintenance` does not look at goals (A8: "optional-but-expected").
- **`propose_goal_change`** (Canon kind):

  | Parameter | Meaning |
  |---|---|
  | `action`* | `create` \| `complete` \| `drop` |
  | `rationale`* | why. For `drop`, this is the "looks unreachable" note (R-14) |
  | `node` | `complete`/`drop`: the id shown in the goals block |
  | `parent` | `create`: the id of a plan in the goals block (optional; a standalone task otherwise) |
  | `title`, `purpose` | `create`: required |
  | `chapter_from`, `chapter_to` | `create`: optional |
  | `entities` (names), `events`, `foreshadowing` (ids) | `create`: optional links, resolved like other tools' references |

  - **Validation:** the fields of the action are present; `node` or `parent` exists and is visible
    (else `error.not_found`, labelled by its level); `node` is open; `parent` is an open plan in the
    session's scope; title, purpose and range follow §4.3.
  - **Isolated sessions** may propose only on their track's own nodes. A shared node gives
    `error.isolated_readonly`, the same rule as `propose_setting_change`.
  - **Effect:** a `pending` proposal (§4.7) and `proposal.changed`. A pending proposal with the same
    node and status already exists → no new row, and the result names the existing one.
  - **Result:** `result.canon_write`, `action = propose_goal_change`.
  - **`ui`:** `{proposal_id, action, node_id?}`.
- `result.finish_maintenance` gains `goal_proposals`. The setting count stops counting goal
  proposals as setting changes (F10). With no goal proposals the text is today's.
- **Demo provider:** the maintenance and planning scripts skip any step whose tool is not frozen.
  When `propose_goal_change` is frozen, maintenance proposes `complete` for the first injected task
  (new hint `DemoHints.goal_task_id`).

### 4.7 Proposals

- `setting_proposals` holds goal proposals too (its name predates goals).
  - Target type `goal_node` for status changes: `target_id` = the node, edits
    `{kind: "set_goal_status", status: "done" | "dropped"}`.
  - Target type `new_goal_task` for creations: `target_id` = the parent plan or NULL, edits
    `{kind: "create_goal_task", task: NewGoalTask}`. `NewGoalTask` carries title, purpose, range,
    link ids, parent and `track_id`.
  - `ProposalDto` gains `goal_target: {id, level, title, status} | null` (the node, or the parent
    of a new task) for rendering. `preview` stays empty for goal targets.
- **Accept** (one transaction, `actor: user`, note = the rationale, `chapter_id` = the proposal's):
  - `set_goal_status`: the node still exists and is open, otherwise `conflict`. Open descendants
    give the §4.3 `409 goal.open_children`, unless the accept body says
    `{confirm_open_children: true}`.
  - `create_goal_task`: the parent, when given, still exists, is a plan, is open, and is in scope,
    otherwise `conflict`. Links whose targets are gone are dropped (the revert rule). The task is
    created `active`.
- **Reject** as today. The Proposals page renders goal proposals with their own card. The Goals page
  shows a node's pending status proposals inline on the node, and pending new tasks at the top of
  their parent.

### 4.8 Change log and revert (A10, Q9)

- **Target types:** `goal`, `plan`, `task`, keyed by `goal_nodes.id` (level is immutable, so a
  node's history has one target type). `change_log.track_id` = the node's `track_id`, as for every
  canon row. Every create, update (fields, status, parent, links, conditions) and delete writes one
  row with snapshots and a note. Author forms have the optional change note; accepted proposals
  write the rationale.
- **Snapshot** `GoalNode {level, parent_id, title, purpose, status, conditions, chapter_from,
  chapter_to, entity_ids, event_ids, foreshadowing_ids, track_id}`.
- **Revert:**
  - `create` → delete. Refused with `409 goal.has_children` when the node now has children.
  - `update` → restore `before`. The parent is restored when it still exists in scope, otherwise
    the node becomes top-level. Links are restored for targets that still exist. Conditions are
    restored as stored; vanished targets show as broken. A restored `done`/`dropped` needs no
    open-children confirmation: a revert is an explicit undo.
  - `delete` → recreate with a new id, with the same parent and link rules. A deleted node never
    had children (§4.3).
- `canon.changed {target_type: goal|plan|task}` after every write and revert. Goal changes never
  reach model-facing memory (`recent_changes` reads entities and relations only).

### 4.9 Interactions (R-19)

- **Deleted canon:** link rows cascade, as for notes and hooks. Conditions keep the id and show as
  broken.
- **Entity merge (A ← B):** moves B's goal links to A, skipping duplicates, and rewrites
  `entity_status_is` conditions from B to A in the same transaction. No per-node change rows, the
  same rule as the links merge already moves (F16).
- **Chapter reorder and delete:** ranges and `chapter_at_least` are positions and are not renumbered
  (§4.2).
- **Isolated track delete:** its nodes, links and change rows cascade with it.
- **Book delete:** everything cascades.
- **Search:** goal nodes are not indexed. `search_canon` does not return them, and no retrieval
  tool reads them. The block is the interface (§11).

### 4.10 Overview (A5, Q7)

- `attention[]` gains `{kind: "ready_goals", count}` when Goals is on and at least one node of the
  book (any scope) is `ready` or `closable`. It links to `canon/goals?filter=ready`. The library
  card's attention count includes it.
- `NextActionDto.ready_goals: number | null` is set on `write_outline`, `write_chapter` and
  `write_next` when that count is > 0. The card shows it as a secondary line ("2 writing goals are
  ready to act on") with the same link.
- **No new action kind.** Ready nodes stay ready until the author acts. A rule would hide the next
  chapter behind them, the dead end that `defer` had to fix for research (D-138).

### 4.11 UI (A9)

- **Navigator:** "Goals" in the canon group after Foreshadowing, with a badge counting `ready` +
  `closable` nodes. Hidden when Goals is off.
- **Route `canon/goals`** (`?filter=ready`, `?node=<id>` scrolls to and opens a node).
- **The page** follows the grammar of the other canon pages:
  - Toolbar: a track filter (shared by default; adds an isolated track's private nodes, badged), a
    **Ready** filter, and **+ Goal**.
  - The tree (goal → plan → task) of non-dropped nodes. Each row shows:
    - the level icon (Lucide `Target` / `Route` / `Flag`);
    - the title and the **purpose** (two lines, visible without opening; A9);
    - the status badge;
    - the range chip ("Ch 40–70", overdue tinted);
    - linked canon chips;
    - when `waiting`: each condition as a sentence with a satisfied ✓ or open ○ mark, and broken
      ones flagged;
    - the `ready` badge, or a `closable` prompt with **Mark done**;
    - the row menu: edit, add child, activate, mark done, drop, delete.
  - Done nodes stay in the tree, dimmed. Dropped nodes live in a collapsed **Dropped (n)** section
    (A6).
  - Children of a closed ancestor are marked "not sent to agents".
- **Node dialog:**
  - level (fixed when adding a child), parent select (same scope), title, **purpose** (required,
    with the hint "Why this exists and what it is for");
  - status, chapter range;
  - conditions editor: kind, plus a target picker (chapter number, foreshadowing picker, entity
    picker and status);
  - link pickers over the existing canon (`EntityPicker`, an event picker, a foreshadowing picker;
    never typed ids);
  - change note.
  - The dialog surfaces `409 goal.open_children` as a confirmation that names the open descendants.
- **"Referenced by"** in the entity detail pane and the event and foreshadowing edit dialogs:
  `GET /books/:id/goals?entity_id=` (or `event_id`, `foreshadowing_id`) lists the nodes that link to
  the row or wait on it.

---

## 5. Prompt pack

| Key | Change |
|---|---|
| `block.canon_systems` | **new** (block, `vars = ["systems"]`). Blank when the four canon systems are on (§3.6). Title "Block — disabled canon systems" / 「上下文 · 未启用的档案系统」. |
| `block.goals` | **new** (block, `vars = ["goals", "systems"]`). §4.5. Title "Block — writing goals" / 「上下文 · 写作目标」. |
| `block.memory` | `vars` += `systems`; fragments guarded per §3.5; byte-identical when all are on. |
| `archivist.kickoff.maintenance` | `vars` += `systems`, `goals`; conditional, renumbered steps; goals step (§3.7, §4.6). |
| `archivist.kickoff.seed` | `vars` += `systems`; conditional steps. |
| `archivist.kickoff.resync` | `vars` += `systems`; conditional bullets. |
| `result.canon_write` | verb for `propose_goal_change`. |
| `result.finish_maintenance` | `vars` += `goal_proposals`. |
| `tool.propose_goal_change` | **new** tool in `tools.toml` with schema and examples. |
| `shared.label` | `target_type` += `goal`, `plan`, `task`; new enum `goal_level`. |
| every `*.system` | **unchanged** (AC13). |

`variables.toml` documents `systems`, `goals` and `goal_proposals`. The context structs in
`prompts/context.rs`, `samples.json` and `scripts/validate_prompts.py` gain the same variables, the
two keys, and the archivist's 22-tool list. Each changed key's samples include all-on, each system
off one at a time, all off, and with and without goals. Chinese vocabulary is §12 item 1.

---

## 6. Data, API and type changes

**Migration `0006_goals_and_switches`** (`use_transaction() = Some(false)`, modelled on 0004):
1. Count `change_log` rows, rows with `reverts_id`, rows with `reverted_by_id`, and
   `setting_proposals` rows into a temp table.
2. `PRAGMA foreign_keys = OFF; BEGIN;`
   - rebuild `change_log` with `target_type` += `'goal','plan','task'` and recreate its three
     indexes;
   - rebuild `setting_proposals` with `target_type` += `'goal_node','new_goal_task'` and recreate
     its index;
   - create the goal tables (§4.1);
   - `ALTER TABLE agent_sessions ADD COLUMN canon_systems TEXT CHECK (canon_systems IS NULL OR
     json_valid(canon_systems));` (no rebuild).
3. `COMMIT; PRAGMA foreign_keys = ON;`. Guard the counts with `CHECK (lost = 0)` rows, and
   `pragma_foreign_key_check` must be empty.
4. `0001_init.sql` gets the same final schema for fresh databases (spec/02 §1). No row is
   back-filled.

**Rust types** (ts-rs exported; regenerate the bindings):
- `CanonSystems`, `CanonSystem`, `BookSettings.canon_systems`;
- `GoalLevel`, `GoalStatus`, `GoalCondition`;
- `ChangeTargetType::{Goal, Plan, Task}`, `Snapshot::GoalNode`;
- `ProposalTargetType::{GoalNode, NewGoalTask}`, `ProposalEdits::{SetGoalStatus, CreateGoalTask}`,
  `NewGoalTask`;
- `AttentionKind::ReadyGoals`, `NextActionDto.ready_goals`;
- `ErrorCode::{GoalOpenChildren, GoalHasChildren}`;
- the SeaORM entities of the four tables, and `agent_sessions.canon_systems`.

**Endpoints:**

| Method | Path | Body / query | Response |
|---|---|---|---|
| GET | `/books/:id/goals` | `?track_id&status&entity_id&event_id&foreshadowing_id` | `GoalNodeDto[]` (derived fields included) |
| POST | `/books/:id/goals` | `GoalNodeInput` | `GoalNodeDto` |
| PATCH / DELETE | `/goals/:id` | partial `GoalNodeInput` + `note?` + `confirm_open_children?` | `GoalNodeDto` / 204 |
| GET | `/books/:id/canon/counts` | | `CanonCountsDto` (per-system totals, every scope and status) |
| POST | `/proposals/:id/accept` | now optional `{confirm_open_children?}` | `ProposalDto` |
| GET | `/books/:id/memory` | | `MemoryViewDto` + `systems` |

```ts
type GoalNodeDto = {
  id: number; book_id: number; track_id: number | null; level: GoalLevel; parent_id: number | null;
  title: string; purpose: string; status: GoalStatus;
  chapter_from: number | null; chapter_to: number | null;
  conditions: { condition: GoalCondition; satisfied: boolean; broken: boolean; target_label: string | null }[];
  entities: { id: number; name: string }[]; events: { id: number; title: string }[];
  foreshadowing: { id: number; title: string; status: ForeshadowingStatus }[];
  ready: boolean; closable: boolean; created_at: string; updated_at: string;
};
type GoalNodeInput = {
  track_id?: number | null; level: GoalLevel; parent_id?: number | null; title: string; purpose: string;
  status?: GoalStatus; chapter_from?: number | null; chapter_to?: number | null;
  conditions?: GoalCondition[]; entity_ids?: number[]; event_ids?: number[]; foreshadowing_ids?: number[];
};
```

**Error codes:**
- `goal.open_children` (409, `details.children`);
- `goal.has_children` (409, `details.children_count`).

**Events:** no new type. Goal writes emit `canon.changed`; goal proposals emit `proposal.changed`.
On the web side `chapter.updated` also invalidates the goals keys and the overview.

---

## 7. Tasks — milestone M13

One commit per task, each ending with `scripts/check.sh` green. Already appended to `docs/PLAN.md`.

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 13.1 | **Docs** — done with this plan (D-150). Re-check them against the author's §12 answers. | §8 | `check.sh` green |
| 13.2 | **Pin the baseline** before any code change: snapshot every chapter-kind kickoff (outline, write, revise, review, maintenance, seed, resync, rollover) and every role's frozen tool list for an all-on fixture book with canon data and no goal nodes. | 03 §10 | `kickoff_baseline.rs` and its `.snap` files, committed on unchanged code. Every later task must leave them untouched (AC13). |
| 13.3 | **Schema and types:** migration 0006 plus the 0001 edit; the entities, enums, JSON types, DTOs and error codes of §6; regenerate the bindings. | 02 §1–§5; 10 §2.4, §2.8, §4 | `goals_migration.rs`: a pre-0006 fixture (changes with revert links, every proposal type, sessions) upgrades with every row and link intact; a broken copy fails loudly with no data lost; new values are accepted; old sessions read `canon_systems = NULL`. JSON round trips: `BookSettings` without the field → all on; partial `canon_systems` → missing on. |
| 13.4 | **Switches in the runtime:** freeze `canon_systems`; filter tool lists; `block.canon_systems`; `block.memory` fragments; `system_of_block`; the `search_canon` filter; `finish_maintenance` required steps; conditional archivist tasks; seed start and overview rule 2; rollover inheritance; Demo skips unfrozen tools; `GET /canon/counts`; `MemoryViewDto.systems`; prompt pack, variables, samples, validator. | 03 §1.2, §4, §9, §10; 05 §2–§3, §6; 06 §4, §6–§8, §10; 08 §7; 09 §2 | `canon_switches.rs`: for each system off, a new session's kickoff has none of its data (fixture names, states, notes, counts fragment, section titles), its tools are not frozen, its maintenance step is absent, and its rows are unchanged (AC9, AC10). Memory off: the write kickoff keeps the outline, and an outline in `block.memory` survives when a `block.chapter_draft` override renders blank (AC11). Off then on restores everything (AC12). A switch flipped mid-session leaves the frozen prompt, tools, `search_canon` behaviour and a rollover child's kickoff unchanged (AC14). Maintenance with Memory or Entities off completes `synced`, and all five off still completes (AC15). Seed rule and finalize gate. `kickoff_baseline` unchanged (AC13). Registry and validator: 22-tool archivist list; filtered lists keep the order. |
| 13.5 | **Goals service and API:** `services/goals.rs` (validation, scope, parents, status rules, open-children confirmation, delete refusal, change log, the evaluator); revert arms; entity merge; endpoints. | 02 §5; 06 §1.7, §3, §9, §11; 10 §2.4 | `goals_api.rs`: create, edit and link a tree, refused without a purpose (AC1); a second parent is impossible and re-parenting moves the node; a task cannot get children (AC2); every mutation is in the change log and revertible, a delete with children is refused (AC7). `goals_readiness.rs`: `foreshadowing_resolved` turns ready with no refresh step (AC3); `chapter_at_least(n)` turns ready on approval of chapter n (AC4); broken targets; `closable`; scope checks; merge moves links and conditions. |
| 13.6 | **Goals in agent context:** `block.goals` and the relevance rules; kickoff lists; the gate; `propose_goal_change` (`tools.toml`, handler, result branches, `goal_proposals` count); goal proposals accept and reject; Demo arm. | 03 §10; 05 §2, §6; 06 §5, §6, §11; 09 §2; 10 §2.3 | `goals_context.rs`: the planner and write kickoffs at *k* contain each relevant task's purpose with its plan's and goal's purposes, and nothing from a task starting after *k* or from a waiting node (AC5). The review kickoff has the same set (AC6). Closed-ancestor exclusion; side-track scope and book-wide goals; no leakage of private nodes; the cap; Entities off removes entity links from the block. `goals_proposals.rs`: create, complete and drop; conflict on accept; dedupe; open-children confirmation on accept; the isolated rule. A maintenance session without relevant goals freezes 21 tools. |
| 13.7 | **Overview:** `ready_goals` attention row and next-action hint, gated by Goals. | 08 §7 | `overview_actions.rs`: a ready node gives the row and the hint on write kinds only; Goals off gives neither; the library count includes the row. |
| 13.8 | **Web — switches:** Canon systems section (hints, retained counts, View data, warnings, disabled window inputs); navigator and QuickSwitch hiding; disabled-system page banner with Turn on; Memory page sections. | 11 §5.1, §5.5; PROJECT §8.12 | Vitest: switches save with the form; navigator entries hide and show (AC9–AC10, UI part); retained counts render; banner and Turn on; `i18n:check`. |
| 13.9 | **Web — goals:** the Goals page, node dialog and pickers, readable conditions, ready filter and badge, inline proposals, the Proposals card, Referenced-by sections, the overview row and hint, `chapter.updated` invalidation. | 11 §3, §4, §5.5; PROJECT §8.8 | Vitest: tree with purposes, statuses, ranges, chips and condition sentences; ready badge (AC8); the dialog refuses an empty purpose; open-children confirmation; Referenced by; the SSE invalidation; the tool card renderer; `i18n:check`. |
| 13.10 | **Smoke and pre-ship:** smoke items for goals and switches; run 0006 on a **copy** of the author's database and report the counts guards and the result. | PLAN M11.5 | Checklist passes; report in the final message. |

---

## 8. Documents amended (task 13.1, done with this plan)

| Document | Sections |
|---|---|
| `PROJECT.md` | §4 glossary; §5 Archivist; §6.5; new §6.16 Writing goals and §6.17 Canon system switches; F6, F7, F12; §8.4 navigator; §8.8 Goals page; §8.12 Canon systems; §9; §10 items 18–25; §11 |
| `spec/02` | §1 (0006), §2 (goal tables, `change_log` and `setting_proposals` CHECKs, `agent_sessions.canon_systems`), §3 enums, §4 JSON types, §5 invariants 13–17 |
| `spec/03` | §1.2, §4 (freeze), §9 (rollover), §10 (kickoff table, two new blocks, system gating), §15 (tests 26–31) |
| `spec/05` | §1.5, §2 (filtering, 22-tool archivist list), §3 (`search_canon`), §6 (`propose_goal_change`, `finish_maintenance`), §9 |
| `spec/06` | intro, §1.7, §2, §3, §4, §5, §6, §7, §8, §9, new §10 switches, new §11 goals |
| `spec/08` | §1.1 (finalize seed gate), §7 (rule 2, `ready_goals`) |
| `spec/09` | §1 label enums, §2 keys |
| `spec/10` | §2.3, §2.4, §2.8, §4 |
| `spec/11` | §3, §4.1, §4.2, §5.1, §5.5, §9 |
| `PLAN.md` | M13 |
| `DECISIONS.md` | D-150 |

---

## 9. Acceptance criteria → proof

| AC | Proof |
|---|---|
| 1 create/edit/link, purpose required | `goals_api.rs`; the CHECK as backstop |
| 2 one parent | `goals_api.rs` (structural `parent_id`; re-parenting moves) |
| 3 hook resolved → ready | `goals_readiness.rs`; web SSE invalidation test |
| 4 chapter official → ready | `goals_readiness.rs`; `chapter.updated` invalidation test |
| 5 planner and writer injection | `goals_context.rs` |
| 6 reviewer injection | `goals_context.rs` |
| 7 change log and revert | `goals_api.rs`; Changes page Vitest |
| 8 Goals page | `goalsPage.test.tsx` |
| 9–10 each switch | `canon_switches.rs` (prompt, tools, step, rows); navigator Vitest |
| 11 Memory off keeps the outline | `canon_switches.rs` (write kickoff and the IR-01 fallback) |
| 12 off and on again | `canon_switches.rs` |
| 13 upgrade is identical | `goals_migration.rs` + unchanged `kickoff_baseline` snapshots |
| 14 frozen sessions | `canon_switches.rs` (mid-session flip, rollover child, `search_canon`) |
| 15 never stale or failed because of a disabled system | `canon_switches.rs` (maintenance completes `synced`; re-enable leaves `canon_status`) |

---

## 10. Risks

| Risk | Mitigation |
|---|---|
| Two table rebuilds in one migration | 0004's proven procedure, count guards, `foreign_key_check`, a broken-copy test, and a dry run on a copy of the author's database (13.10). Neither table has incoming foreign keys. |
| `block.memory`'s conditional template drifts from today's text | The baseline snapshots of 13.2 are committed before any change. Every sample combination is rendered by the validator and the snapshot test. |
| A book override of `block.memory` or an archivist task predates `systems` | It still renders: an unused variable is harmless. It does not hide disabled fragments, but the snapshot builder leaves them empty, so it shows empty sections at worst. Overrides of these keys are reported in the 13.10 pre-ship check. |
| Static system prompts still name tools a book switched off | `block.canon_systems` says so, and `error.tool_not_allowed` makes a stray call harmless (F3). |
| Purposes make kickoffs longer | Relevance filter, a 30-node cap, a 1,000-character purpose cap, and the existing `context.large_kickoff` warning. |
| Authors disable Memory and Entities and lose continuity | Hints, the `recent_chapter_count = 0` warning, and recent chapters stay independent. |
| Tool lists now vary by book and by maintenance session | Within a book the planner, writer and reviewer prefixes are stable. Archivist sessions rarely share a cache window. |

---

## 11. Out of scope

- Planned (unnarrated) timeline events and a condition that waits for them (R-3).
- A goals retrieval tool, search indexing of goal nodes, and "upcoming tasks" injection ahead of
  their range.
- Seeding goals from the book outline at finalize.
- A `purpose` review-issue category (the reviewer reports misses under the existing categories).
- Manual sibling ordering, drag-and-drop re-parenting, and cascading status changes.
- Per-chapter or per-session switches, partial switches, and back-filling a re-enabled system
  (B7, §3.7).

---

## 12. Confirm with the author before 13.3

Each item changes behaviour the author will notice. The spec already follows the first option.

1. **Chinese vocabulary.** In prompts and the zh-CN UI: 写作目标 for the system, 目标 · 阶段计划 ·
   情节点 for the levels, 目的 for purpose. This avoids 「任务」 (already the session's task in every
   kickoff) and a bare 「计划」 (the writer's 修订计划). The alternative is the literal 目标 · 计划 ·
   任务. English UI: Goals, Goal / Plan / Task.
2. **`timeline_event_exists` is dropped** (R-3). The alternative is planned timeline events, a
   Timeline change of its own.
3. **"Looks unreachable" is a `drop` proposal** (R-14). The alternative is a separate note kind with
   an acknowledge-only card.
4. **Book-wide goals reach side stories** (range-less shared goals; §4.2). The alternative is that
   side stories see only their own private nodes, and canon-mode side stories none.
5. **The archivist's goal step appears only when the chapter has relevant goal nodes** (§4.6), and
   may propose closing plans and goals, not only tasks.
6. **Children of a closed node stop reaching the agents** without being closed themselves (§4.3).
   The alternative is to cascade the status.
7. **Revise and rollover kickoffs carry the goals block too** (§4.5), beyond the planner, writer and
   reviewer the requirements name.
8. **Switches save with the Book settings form**, not instantly per toggle (§3.10). The navigator
   updates on Save without a reload.
