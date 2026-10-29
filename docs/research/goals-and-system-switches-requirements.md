# Requirements: Layered Writing Goals and Per-System Switches

**Purpose of this document.** These are the requirements for a change to NekoWriter. An engineer or
an AI assistant should be able to read this and produce an implementation plan without further
questions about intent. It states the current behaviour, what must become true, the constraints the
plan must respect, and the questions the plan must answer.

Status: **proposal, not implemented.** No code has been written for this.

---

## 1. Product context

NekoWriter is a **local, single-user** web app for writing long Chinese web serials. One author, one
SQLite database. Rust (axum + SeaORM + SQLite) backend; React SPA frontend.

The author manages a book's **canon** through four sibling systems, all served by the same
Archivist agent during chapter maintenance:

| System | Tables | What it holds |
|---|---|---|
| **Entities** | `entities`, `relations`, `appearances` | characters, organizations, locations, items; their descriptions, states, attributes, relationships, and who appears where |
| **Timeline** | `timeline_events`, `timeline_event_entities` | plot-relevant events in story order, linked to the entities involved |
| **Memory** | `memory_notes`, `memory_note_entities`, `chapters.summary` | facts, world rules, chapter summaries; rendered as tiered memory (short / mid / long-term index) |
| **Foreshadowing** | `foreshadowing`, `foreshadowing_entities` | planted hooks and their payoffs |

All canon rows carry `track_id` (`NULL` = shared canon, non-NULL = private to an isolated side
track), are written through `Db::write`, and are recorded in `change_log` so every change is
revertible.

Relevant vocabulary:

- **Chapter status**: `planned`, `drafting`, `in_review`, `revising`, `official`.
- **Session kind**: `conception`, `research`, `chapter_outline`, `chapter_write`, `chapter_revise`,
  `chapter_review`, `chapter_maintenance`, `canon_seed`, `chapter_resync`.
- **Canon status** (per chapter): `none`, `pending`, `syncing`, `synced`, `stale`, `failed`.
- **Agent role**: `conception`, `researcher`, `planner`, `writer`, `reviewer`, `archivist`.

---

## 2. Current behaviour (verified against the code)

### 2.1 The four systems reach the agent fused into one prompt block

There is exactly one context block for canon, `block.memory` (`prompts/blocks.toml`). It renders, in
order:

- **short-term**: the current chapter label, recent chapter summaries, the previous chapter's tail;
- **the protagonist's current state** (an entity);
- **the current chapter's outline** (a chapter field, not canon);
- **mid-term**: recent state/relationship changes (memory notes), appearance counts per character
  (entities), important characters absent for a long time (entities);
- **long-term index**: key facts and world rules (memory notes), unresolved foreshadowing, the main
  cast (entities), and a row of archive counts covering entities, timeline events, memory notes and
  open foreshadowing.

The Timeline system contributes only counts today; Entities, Memory and Foreshadowing contribute
both content and counts. `block.memory` is emitted for the chapter-outline planner, the drafting
writer, the revision writer, the reviewer, chapter maintenance and re-sync; the rollover child gets
it through its own hard-coded block list.

### 2.2 The Archivist maintains all four in one session kind

`chapter_maintenance` (`archivist.kickoff.maintenance`) carries ten ordered steps: appearances,
entities, protagonist state, relations, timeline events, memory notes, foreshadowing, chapter
summary, setting proposals, finish. `finish_maintenance` validates the required ones. Approval,
manual start and retry are the three entry points.

### 2.3 The book settings today

`books.settings` is a JSON `BookSettings` with `granularity`, `auto_pause`, `pause_after_outline`,
`target_words_min`, `target_words_max`, `recent_summary_count`, `midterm_window`,
`recent_chapter_count`, `auto_maintenance`. There is **no per-system switch**. The only existing
partial precedent is `recent_chapter_count = 0`, which disables `block.recent_chapters`.

### 2.4 The UI today

The book workspace has a canon group: Entities (`canon/entities`), Timeline (`canon/timeline`),
Memory (`canon/memory`), Foreshadowing (`canon/foreshadowing`), and Changes (`canon/changes`).
Book settings lives in the settings area.

### 2.5 What does not exist

- There is no concept of a **writing goal**, a **plan**, or a **task**. The only forward-looking
  artefact is `chapters.outline`, which is per-chapter and short-horizon.
- There is no way to turn a canon system off.

---

## 3. Requirement A — a layered writing-goal system

### A1. Three levels

The system has exactly three levels, ordered from longest to shortest horizon:

| Level | Horizon | Question it answers |
|---|---|---|
| **Goal** | an arc/volume or the entire book | What should this book ultimately make the reader feel, understand, or believe? |
| **Plan** | the next few tens of chapters | What has to change, across this stretch of chapters, for the goal to become reachable? |
| **Task** | the next few chapters | In which concrete scene or event does that change happen, and how? |

Every plan belongs to **at most one** goal. Every task belongs to **at most one** plan. A goal may
have no plans yet; a plan may have no tasks yet. The tree is at most three levels deep — a task
cannot have children.

### A2. Mandatory writing purpose

**Every node at every level carries an explicit, mandatory `purpose`**: one short paragraph, in the
author's own words, stating *why this exists* and *what it is for*. It is a separate field from the
title and from any summary. It is never empty, and it is never generated silently: the author writes
it, or an agent proposes it and the author approves it.

The purpose is what the agent must see. An agent told "write scene 12" writes a generic scene; an
agent told "this scene exists so the reader stops trusting the mentor" writes a different scene.
The purpose is the primary payload of this system, not metadata.

### A3. Worked example (the author's own)

```
Goal   「让林昭成为一个读者会心疼、但不会同情的角色」
       purpose: 她的每一次选择都要让读者理解她为什么这么做，但不认同那个理由。
  └── Plan 「角色弧光 · 一：从被保护者到施害者」
          purpose: 在 40–70 章之间，让她的保护者身份逐步失效，并被她自己亲手打破一次。
        └── Task 「在『城门口对峙』事件中，让她第一次主动撒谎保护仇人」
                purpose: 这是她第一次为了保护别人而背叛自己的原则；读者要在这里第一次
                         意识到她已经在变成她害怕的那种人。
```

### A4. Links to the other four systems

A goal, plan or task may reference, and must be able to render as a link to:

- **entities** — the characters/organizations/locations/items the node is about;
- **timeline events** — events that must happen for it, or that it is staged around;
- **foreshadowing** — hooks whose planting or payoff it depends on;
- **chapter range** — an optional `[from, to]` chapter range within its track.

These links are what make a node *findable* from the canon side: opening an entity, an event or a
foreshadowing entry must show which goals/plans/tasks reference it.

### A5. Readiness: a node can wait for the plot

The author's central observation: **a character beat often cannot be written until the plot has
reached a certain point, or until a foreshadowing has been paid off.** A task that says "in the
city-gate confrontation, let her lie to protect her enemy" is meaningless at chapter 12 if the
confrontation is a chapter-60 event.

Therefore:

- A node may be **`waiting`**, with a non-empty list of **conditions**.
- A condition is a reference to state that the other systems own. The minimum closed set:
  - `chapter_at_least(n)` — the node's track has reached chapter *n* (`official` or later);
  - `foreshadowing_resolved(id)` — that hook has been paid off;
  - `timeline_event_exists(id)` — that event has been recorded;
  - `entity_status_is(id, status)` — the entity has reached that status.
- Readiness is **derived from current canon state**, not stored as a flag that can go stale. A node
  whose conditions are all satisfied is shown as **ready**; a node with unsatisfied conditions is
  shown with *what it is waiting for*, in the author's language, not as raw ids.
- When a waiting node becomes ready, it must surface where the author will see it — at minimum on
  the Goals page, and through the overview's next-action surface. The author must not have to
  remember that a hook paid off three chapters ago.

This is the reason the system has to be a sibling of the other four rather than a free-text note:
its readiness is computed **from** them.

### A6. Lifecycle

Each node has a stored status:

| Status | Meaning |
|---|---|
| `active` | being worked on now |
| `waiting` | not yet actionable; has conditions (A5) |
| `done` | achieved |
| `dropped` | deliberately abandoned |

`ready` is **not** a stored status; it is `waiting` plus all conditions satisfied.

Rules:

- Only `active` and `waiting` nodes are injected into agent context.
- Marking a task `done` does not automatically mark its plan done. Marking every task of a plan done
  **surfaces** the plan as ready to close; the author decides.
- A parent cannot be `done` or `dropped` while a child is still `active` or `waiting` without an
  explicit confirmation that names the open children.
- `dropped` nodes are kept and visible in a collapsed section; they are never injected.

### A7. Agent context injection

- The **planner** and the **drafting writer** must receive, for the current chapter: the `active`
  short-term tasks whose chapter range covers or precedes this chapter, each with its **purpose**;
  the plan each belongs to, with its purpose; and the goal above that plan, with its purpose.
- The **reviewer** must receive the same set, so it can judge whether the chapter serves the stated
  purpose.
- The set is **filtered to the current chapter's position**: a task for chapters 60–62 is not
  injected at chapter 20, and a node still `waiting` is never injected.
- If a node's chapter range is absent, the node is considered relevant from the current chapter
  onward until it is done.
- Injection must be its own context block (`block.goals` or equivalent), not merged into
  `block.memory`, so that Requirement B can switch it off independently and so its ordering relative
  to the other blocks is explicit. `block.settings` must remain the first block and the cache
  breakpoint.

### A8. Maintenance and authorship

- The author creates and edits goals, plans and tasks directly in the UI.
- The **Archivist** must additionally be able to propose changes: a new task when a chapter creates
  an opportunity, a status change when it observes that a purpose has been achieved, and a note when
  a task looks unreachable. Proposals follow the existing proposal/approval shape — they never
  silently rewrite the author's intent.
- The Archivist's maintenance step for goals must be **optional-but-expected** in the same sense as
  the existing steps: `finish_maintenance` must not fail when the chapter simply did not move a goal.
- Whether goal maintenance is a new step in `chapter_maintenance` or a separate session kind is a
  question for the plan (§6).

### A9. UI

- A fifth page in the canon group, `canon/goals`, rendered as a three-level tree (goal → plan →
  task) with the **purpose visible without opening a node**.
- Each node shows: title, purpose (truncated), status, chapter range, linked canon chips, and — when
  `waiting` — the conditions in readable form with a satisfied/unsatisfied mark.
- A **ready** filter, and a badge in the navigation when any node is ready.
- Creating/editing a node must let the author pick linked entities, events and foreshadowing from
  the existing canon data, not type ids.
- Entity, timeline-event and foreshadowing detail pages gain a "Referenced by" section listing the
  goals/plans/tasks that link to them.

### A10. Change log and revert

Goal/plan/task changes are canon changes. They must be recorded in `change_log` with a new
`target_type`, snapshotted like the other canon types, and revertible through the existing Changes
page. This requires extending the `change_log.target_type` CHECK constraint, which in SQLite means a
table rebuild in a new migration.

### A11. Non-goals

- Not a project-management tool: no assignees, estimates, sprints, dependencies between tasks, or
  burndown.
- Not a replacement for `chapters.outline`: an outline is the plan for one chapter's prose; a task is
  a narrative intention that may span several chapters.
- No automatic prose generation from a goal. The system informs the agent; it does not drive the
  agent without the author.
- No new "volume" or "arc" entity. A goal describes its own span; the optional chapter range is
  enough.

---

## 4. Requirement B — per-system switches

### B1. The five systems

The switches cover exactly: **Entities**, **Timeline**, **Memory**, **Foreshadowing**, and the new
**Goals**.

### B2. What a switch controls

One switch per system, per book. Turning a system **off** must:

1. **Stop injecting it.** None of its content appears in any agent context block.
2. **Stop maintaining it.** The Archivist's maintenance step for it is skipped, and the system's
   `canon_status` contributions do not mark a chapter stale or failed because of it.
3. **Remove its tools.** Agent tools that read or write it are not offered in new sessions.
4. **Hide its surface.** Its page/section is hidden from the book workspace navigation.

And must **not**:

5. **Delete anything.** All rows remain; turning the switch back on restores full behaviour with the
   data intact.
6. **Affect frozen sessions.** A session already running when the switch changes keeps its frozen
   prompt and tool list; the switch affects sessions started afterwards.
7. **Break other systems that reference it.** See B3.

### B3. The block-splitting problem (the crux)

Today the four systems are **fused into one prompt block**: `block.memory` renders entity state,
entity appearance counts, the cast, memory notes, foreshadowing, and archive counts together. A
per-system switch therefore cannot be implemented by skipping a block — the block itself has to be
assembled from per-system fragments, each independently omittable.

Requirements:

- Each system's contribution must be an independently omittable fragment, including the counts row,
  which today mixes all four.
- The block must not render a section header with no content beneath it, and must not render an
  empty block at all (the existing "drop blank blocks" rule applies).
- **Memory is load-bearing for continuity** and is not only a canon domain. `block.memory` also
  carries non-memory scaffolding: the current task label and the current chapter's outline echo.
  The plan must state exactly which parts of the current block belong to the Memory system and which
  are session scaffolding, and must keep the scaffolding when Memory is off. Turning Memory off must
  not silently remove the chapter outline from the writer's prompt.
- Turning **Entities** off must not remove the protagonist's current state from a session that still
  needs it for continuity — either that state is scaffolding, or the plan must say what replaces it.

### B4. Scope, defaults and migration

- The switch is **per book**, stored in `BookSettings`, so it is copied from `app_settings`'
  `book_defaults` for new books like every other setting.
- Default for every system: **on**. An existing database upgrades with all five on, and behaves
  exactly as today.
- The settings UI must show the five switches together with a one-line explanation of what turning
  one off does, and must not require a save-and-reload to take effect for the next session.

### B5. Interaction with existing settings

- `recent_chapter_count = 0` already disables `block.recent_chapters`. That is a **different**
  control: it is a length/recency setting, not a system switch. The plan must state whether it stays
  independent (it should) and how the UI presents the two without confusing them.
- `auto_maintenance` stays independent: it decides *when* maintenance runs, not *what* it maintains.
- A system that is off must not make the overview report a chapter as `stale` or `failed` because
  that system's data was not refreshed.

### B6. UI

- The five switches live in book settings, in a dedicated "Canon systems" group.
- Turning a system off hides its navigation entry immediately; its data stays reachable through a
  "show disabled systems" affordance or by re-enabling it — it must not become unreachable with no
  way back.
- If the author turns off a system that has data, the UI says how much data is retained, and offers
  no destructive action.

### B7. Non-goals

- No per-chapter or per-session overrides.
- No partial switches (e.g. "inject but do not maintain") in the first version; if the plan believes
  one is essential it must say why and keep the single switch as the primary control.
- No deletion or archiving of a disabled system's data.

---

## 5. Constraints the plan must respect

- `AGENTS.md` is binding: English in code/identifiers/comments/docs; model-facing text only in
  `prompts/` (Chinese); UI text only in the i18n files with `en` and `zh-CN` updated together; every
  write through `Db::write`; append-only agent history; tests need no network (`wiremock`,
  `ScriptedProvider`, `FakeEmbedder`); `pnpm` only.
- Migrations are append-only SQL files. The next number is **0006**. Extending the
  `change_log.target_type` CHECK requires a table rebuild.
- Adding or changing a prompt variable means updating `variables.toml`, the Rust context struct, its
  sample contexts, and `scripts/validate_prompts.py`, then running the validator.
- `block.settings` must remain the first block and the only cache breakpoint.
- A new canon domain must follow the existing patterns: `track_id` visibility, `change_log`
  entries, `Db::write` mutation, and the same UI grammar as Entities/Timeline/Memory/Foreshadowing.
- Do not change the behaviour of frozen sessions.

---

## 6. Questions the plan must answer

1. **Schema.** Tables for goals/plans/tasks — one table with a `level` column or three tables? How
   are parent links enforced? How are the condition lists stored (a JSON column like
   `BookSettings`, or a join table)?
2. **Scope.** Are goals book-scoped or track-scoped (with `track_id` like canon)? What does a
   side-track goal mean, and how is leakage into the main track prevented?
3. **Trigger evaluation.** Computed on read, or materialised? Where does the "ready" calculation
   live so both the Goals page and the overview agree? What invalidates it?
4. **Block design.** One `block.goals` or a fragment inside `block.memory`? Where in the block order
   does it sit, and what exactly does it render (purpose text, chapter range, linked canon)?
5. **Maintenance.** A new step in `chapter_maintenance` (raising the required-step count), a new
   session kind, or author-only in the first version? The author wants the Archivist to help, so the
   plan must say how.
6. **Toggle granularity.** Confirm the B2 capability set, and state precisely which fragment of
   `block.memory` each switch removes.
7. **Next-action integration.** Should a ready goal produce an overview action, and if so, which —
   a new action kind, or a hint on the existing `write_next`?
8. **Migration and back-fill.** How does an existing database with chapters and canon get its
   `BookSettings` defaults, and does anything need back-filling for existing books?
9. **Change log.** The exact `target_type` values and snapshot shapes, and how revert of a task that
   has children behaves.

---

## 7. Acceptance criteria

The change is done when all of the following hold, each with a test that fails before and passes
after:

**Goals**

1. A goal, a plan and a task can be created, edited and linked into a tree; each refuses to save
   without a non-empty `purpose`.
2. A task cannot be attached to a second plan, and a plan cannot be attached to a second goal.
3. A `waiting` node with `foreshadowing_resolved(id)` becomes **ready** as soon as that hook is
   resolved, without any manual refresh step.
4. A `waiting` node with `chapter_at_least(n)` becomes ready when chapter *n* reaches `official`.
5. The planner's and drafting writer's kickoff for chapter *k* contains the purpose of every
   `active` task relevant at *k*, its plan's purpose, and its goal's purpose — and contains **none**
   of a task whose range starts after *k*, and **none** of a `waiting` node.
6. The reviewer's kickoff contains the same set.
7. Every goal/plan/task mutation appears in `change_log` and is revertible from the Changes page.
8. The Goals page renders the tree with purposes, statuses, chapter ranges, linked canon and the
   readable waiting conditions; a ready badge appears when a node becomes ready.

**Switches**

9. With Entities off, a new session's prompt contains no entity-derived text, the Archivist's
   entity step is skipped, entity tools are absent from the tool list, and the Entities navigation
   entry is hidden — while the `entities` rows are unchanged in the database.
10. The same four checks hold for Timeline, Memory, Foreshadowing and Goals.
11. With Memory off, the writer's prompt still contains the current chapter's outline (scaffolding
    survives) and contains no summaries, previous tail, memory notes or archive counts.
12. Turning a system off and on again restores its full injection, maintenance, tools and page, with
    the data intact.
13. A database created at the previous migration upgrades with all five switches on and behaves
    identically for an unchanged book.
14. A session that is already running when a switch flips keeps its frozen prompt and tool list.
15. The overview never reports a chapter `stale` or `failed` because a disabled system was not
    refreshed.
