# Implementation Plan: The timeline as a tree of history and plan

> Milestone **M22** in `docs/PLAN.md`. Read `AGENTS.md` first. The spec sections listed in §7 are
> amended together with this plan, and the decision is recorded as **D-170**. The spec is
> authoritative; this plan explains the reasons, the order and the tests. M22 comes after M21
> (D-169): it changes tools the book agent reuses.

## 0. What the author asked for, and the answers

The author's complaint: with the first chapter unwritten, the canon already listed foreshadowing
payoffs as "卷三第 50—53 章", chapter numbers planned dozens of chapters ahead. Their words:
"写作应该是渐进式的，这样精准的全局规划根本不可能". Then: "这些关系应该有一些机制来约束，和时间线
功能联动一下我觉得挺好". The constraint must be structural, not only prompt wording.

| Question | Binding answer |
|---|---|
| How do we say "this pays off later" without a chapter number? | Anchor it to a timeline node and a position: `before` / `after` / `at` |
| Is the timeline only what happened? | No: a node is `occurred`, `planned` or `abandoned`. History and plan share one tree |
| May a node contain several sub-entries? | Yes, and depth is unlimited |
| Does the topmost node hold content? | No. Each book gets one real, empty `root` row that only anchors the tree |
| Where do planned nodes come from? | The archivist seeds them from what the conception outline states, with no cap |
| What happens to abandoned nodes? | The author still sees them (greyed); agents never do |
| A foreshadowing resolved while its anchor is still planned? | A notice in the tool result, never an error |
| How do planned nodes relate to writing goals? | **They complement each other** ("计划节点和写作目标是相辅相成的"): planned nodes say what the story is expected to go through, goals say why, and they link (T13) |
| May an `occurred` node be abandoned? | **No** ("不允许"). Only a planned node is abandoned. Wrong history is edited or deleted |

## 1. Decisions at a glance

| # | Decision |
|---|---|
| T1 | **One tree per book.** `timeline_events` gains `parent_id` (self FK, **no cascade**), `kind` (`root` \| `entry`) and `status`. Each book has exactly one `root` row (a partial unique index), created by the migration for every book and by book creation. It has empty content, `track_id`, `chapter_id` and `parent_id` NULL, and position 0. It is never listed, rendered, indexed, logged, changed or deleted, and **no API or tool returns it**: a top-level entry shows `parent_id: null`, and the service stores the root's id. |
| T2 | **Depth is unlimited; `position` keeps its meaning.** `position` stays the **book-wide story chronology** (spec/06 §1.4). The tree groups entries; it does not reorder them. Siblings are listed by `position`. `after_event` / `before_event` / `move_after` / `move_before` keep their meaning and never change the parent. A new child given neither goes right after the last node of its parent's subtree (the parent itself if it has none); a new top-level entry goes at the end, as today. Every position query filters `kind = 'entry'`. |
| T3 | **Every entry has a status**: `occurred` \| `planned` \| `abandoned`. A planned entry has `chapter_id IS NULL`, so the service does **not** fill in the session chapter for it. `story_time` may be empty. |
| T4 | **Status moves only forward**: `planned → occurred` and `planned → abandoned`. An `occurred` entry is never abandoned; it is edited or deleted. `abandoned` is final. A change-log **revert** is an undo, not a move: it may restore any snapshot that T5 and T6 still allow. |
| T5 | **Consistency**: no `occurred` entry has a `planned` descendant **in its own scope** (same `track_id`). The root is exempt. This is checked when an entry is created as planned, when a subtree moves to a new parent, when an entry is promoted, and on revert. A breach is refused and names the conflicting entry, which the caller can always see. |
| T6 | **Scope**: an entry's parent is the root, a shared entry, or an entry of the entry's own track. A shared entry is never placed under a private one. An isolated session cannot change shared entries anyway (`isolated_readonly`). An entry may not get a new child or a new parent that is abandoned. |
| T7 | **Abandoning** a planned entry also abandons its planned descendants in the same scope, in one transaction, with one change each. It is refused when that scope's part of the subtree holds an occurred entry. |
| T8 | **Abandoned entries are invisible to agents**: `query_timeline`, `search_canon` (their search docs are removed), the `block.memory` counts, `block.goals` event links and the book agent's `block.book_index` counts leave them out. A visible entry whose parent is hidden is shown at the top level. The author sees abandoned entries greyed and struck through. |
| T9 | **Deletion**: an entry with children (any status) is never deleted, by the API, a tool or a revert of its `create`: `409 timeline.has_children`, mirroring goal nodes. Deleting an entry first clears the anchors of the foreshadowing that point at it (a logged foreshadowing update each). Reverting a delete recreates the entry under its old parent when that parent still exists, is in scope and is not abandoned, otherwise at the top level. Anchors are not restored. |
| T10 | **Foreshadowing anchors to a timeline entry**: `payoff_event_id` and `payoff_position` (`before` \| `after` \| `at`) are both NULL or both set. The anchor is an entry visible from the foreshadowing's scope and not abandoned when it is set. `planned_payoff` becomes **direction only** ("揭示黑衣人与林家的渊源"): no position, no chapter number. An unanchored foreshadowing is legal. If the anchor is abandoned later, agents see "回收锚点已放弃" without its title, and the author sees the greyed anchor. |
| T11 | **Changing an anchor**: a new tool `update_foreshadowing` (archivist and book agent) and the author's PATCH. |
| T12 | **Resolving against a planned anchor is a reminder.** `resolve_foreshadowing` succeeds, and the result adds a notice that the anchor is still planned. |
| T13 | **Planned entries and goals complement each other.** Planned entries are canon: written by the archivist without approval, logged and revertible. Goals stay the author's intent records, with their approval rule. They connect three ways: (a) goal links may name planned entries, and `block.goals` marks them `（计划中）`; (b) a new goal condition, `event_occurred {event_id}`, so a waiting plan or task becomes ready when its planned event happens; (c) when maintenance promotes an entry that a relevant goal node links, step 9 considers proposing `complete` for that node. It remains a proposal. Goals are still never seeded. |
| T14 | **Conception plans relationally.** `conception.system` deliverable 1 drops "要具体到足以指导逐章写作" and describes phases and ordering relations (what comes before what, what happens together), with no chapter numbers unless the author asks for chapter-level planning. Deliverable 4 describes growth by stage, without chapter numbers. Chapter detail stays with the per-chapter outline sessions. |
| T15 | **The seed adds planned entries.** Besides backstory (occurred), the archivist seeds planned entries from what the outline states, with **no cap**: one entry per stated phase, with that phase's stated events as children. It records foreshadowing only where the outline states it, anchored to the most specific entry, with a direction-only `planned_payoff`. |
| T16 | **Maintenance and re-sync.** Maintenance promotes the planned entries the chapter reached, adds the chapter's events as sub-entries of the right entry, abandons planned entries the chapter made impossible, and re-anchors foreshadowing whose plan moved. Re-sync edits or deletes an occurred entry the edited text no longer supports; it never sets one back to planned. |
| T17 | **With Timeline off**, the anchor parameters must stay empty (`error.system_off`), `list_foreshadowing` renders no anchors, and the `block.canon_systems` Timeline line says so. |
| T18 | **The migration is append-only** (`0013`). It adds the columns, creates one root per book, and reparents every existing row to its book's root as `occurred`. Existing foreshadowing keeps its text. There is no automatic conversion of existing books. The author edits them, or asks the book agent (M21) to build planned entries from the outline, re-anchor the foreshadowing and take the chapter numbers out of the outline. |

## 2. Facts this plan rests on (code at `c67bb4a`, the author's database read-only on 2026-11-04)

- **`timeline_events`** (`crates/server/src/db/entities/timeline_events.rs`, 0001): `id`,
  `book_id`, `track_id` (NULL = shared, T = private to isolated track T, `ON DELETE CASCADE`),
  `position REAL` ("chronological story order"), `story_time`, `title` (non-empty, checked by
  the service), `description`, `chapter_id` (`SET NULL`), `importance`, timestamps. Links:
  `timeline_event_entities`, `goal_node_events`. It is flat: no parent, no status.
- **Positions** (`services/canon/timeline.rs`): `position_for`, `neighbor_positions`,
  `last_position`, `next_after`, `first_before` and `renormalize` all work per **book**. The
  archivist's `timeline_neighbors`, `query_timeline` (`order_by position, id`), `around_event`
  and the API list all rely on book-wide order. This is why T2 keeps it.
- **Session chapter**: `timeline::create_inner` stores `input.chapter_id.or(write.chapter_id)`,
  so a maintenance session's new entry gets the session chapter automatically (T3 must skip it).
- **Delete and revert**: `delete_inner` writes one `delete` change with a snapshot and emits
  `canon.changed`. Revert recreates the entry with a **new id** (spec/02 §5.10).
  `search_docs` has no foreign key to its source, and `sync_timeline_event`
  (`search/maintenance.rs`) runs per `canon.changed`. A cascaded delete would leave the children's
  docs searchable and their history unrecorded. This is why T9 refuses deleting a parent.
- **Goal nodes** already follow the same rules: `409 goal.has_children` for delete and for
  reverting a create, and a restored node gets its parent back only when it still exists in
  scope (spec/06 §3, §11.4). Spec/06 §11.3 says "There is no timeline condition", because an event
  could only be picked once it existed. Planned entries change that (T13b).
- **Tools**:
  - `add_timeline_event` (`title`, `description`, `story_time`, `after_event`, `before_event`,
    `entities`, `importance`);
  - `update_timeline_event` (`event_id`, `title`, `description`, `story_time`, `entities`,
    `importance`, `move_after`, `move_before`; no `note`);
  - `query_timeline` (`entity`, `query`, `around_event`, `from_chapter`, `to_chapter`, `limit`,
    `order`);
  - `list_foreshadowing`;
  - `add_foreshadowing` (`planned_payoff` is free text);
  - `resolve_foreshadowing`.

  No agent tool edits an existing foreshadowing. The archivist has 22 tools today
  (`finish_maintenance` included).
- **Context**: no prompt block lists the timeline. `block.memory` renders the count
  (`memory.counts.event`, `prompts/blocks.toml:139`). Agents see entries through
  `result.timeline`, `search_canon` hits (`event` docs: title, story_time · description) and the
  `#id title` event links in `block.goals`. There is no `block.canon` and no `list_timeline` tool
  (`list_timeline` is the API handler).
- **API**: `GET/POST /books/:id/timeline`, `PATCH/DELETE /timeline-events/:id`,
  `POST /timeline-events/:id/move`; `GET/POST /books/:id/foreshadowing`,
  `PATCH/DELETE /foreshadowing/:id`. The patches already use double options
  (`chapter_id: Option<Option<i32>>`).
- **Prompts**: `conception.system` deliverable 1 ends with "要具体到足以指导逐章写作"
  (`prompts/system.toml:153`); deliverable 4 has no such sentence. The seed task already asks for
  "planned_payoff 写回收方向", but the outline it reads numbers chapters, so the numbers came
  through.
- **The author's book #1**:
  - `active`, with no chapters yet;
  - 3 timeline rows (positions 1024–3072, no chapter);
  - 12 foreshadowing rows whose `planned_payoff` names chapters ("卷三第 50—53 章：…");
  - the outline numbers every volume ("第 1—22 章" … "第 91—110 章"), and so does the
    protagonist document.

  The seed has run, so T14 and T15 do not reach this book. T18's path does.
- **Scope note**: PROJECT §9.2 excludes "volumes or arcs as data objects". Timeline entries group
  **story events in story time**. They hold no chapters and no chapter range, so they are not
  volumes of the book. §9.2 gains that sentence.

## 3. Design

### 3.1 Schema: `0013_timeline_tree.sql`

```sql
ALTER TABLE timeline_events ADD COLUMN parent_id INTEGER REFERENCES timeline_events(id);
ALTER TABLE timeline_events ADD COLUMN kind TEXT NOT NULL DEFAULT 'entry'
  CHECK (kind IN ('root','entry'));
ALTER TABLE timeline_events ADD COLUMN status TEXT NOT NULL DEFAULT 'occurred'
  CHECK (status IN ('occurred','planned','abandoned'));
-- one root per book, with ids above every id the table or the change log has used (§5.10)
INSERT INTO timeline_events (id, book_id, track_id, position, story_time, title, description,
                             chapter_id, importance, kind, status, created_at, updated_at)
SELECT base.top + ROW_NUMBER() OVER (ORDER BY b.id), b.id, NULL, 0, '', '', '', NULL, 2,
       'root', 'occurred', :now, :now
FROM books b,
     (SELECT max(coalesce((SELECT max(id) FROM timeline_events), 0),
                 coalesce((SELECT max(target_id) FROM change_log
                           WHERE target_type = 'timeline_event'), 0)) AS top) base;
UPDATE timeline_events
   SET parent_id = (SELECT r.id FROM timeline_events r
                     WHERE r.book_id = timeline_events.book_id AND r.kind = 'root')
 WHERE kind = 'entry';
CREATE UNIQUE INDEX timeline_events_root ON timeline_events(book_id) WHERE kind = 'root';
CREATE INDEX timeline_events_parent ON timeline_events(parent_id);

ALTER TABLE foreshadowing ADD COLUMN payoff_event_id INTEGER
  REFERENCES timeline_events(id) ON DELETE SET NULL;
ALTER TABLE foreshadowing ADD COLUMN payoff_position TEXT
  CHECK (payoff_position IN ('before','after','at'));
```

- Rust-managed transaction (`use_transaction() = Some(false)`, like 0008–0011). The module binds
  `:now` (RFC 3339, as `stamp()`). Guards: the `entry` count is unchanged; the root count equals
  the book count; no `entry` has a NULL `parent_id`. A failed guard rolls back to a retryable
  pre-0013 database. `0001` is not edited.
- `parent_id` has **no** `ON DELETE` action: the services refuse to delete a parent (T9), and the
  foreign key is the backstop. Book and track deletion still work. A book cascade removes all of
  its rows in one statement. A track cascade removes only private rows, whose descendants are
  private to the same track (T6).
- `payoff_event_id`'s `SET NULL` is a backstop too. The service clears both fields before it
  deletes an entry, and readers treat a position without an event as unanchored.
- Book creation inserts the root in the book's transaction (`services::books::create`).
- Entity: `parent_id: Option<i32>`, `kind: TimelineKind`, `status: TimelineStatus`, the self
  relation, and `foreshadowing.payoff_event_id` / `payoff_position: Option<PayoffPosition>`.
- Snapshots: `SnapshotTimelineEvent` gains `parent_id` (the stored parent, the root's id
  included) and `status`; `SnapshotForeshadowing` gains `payoff_event_id` and `payoff_position`.
  All have serde defaults, so pre-0013 rows load as top level, `occurred`, unanchored.

### 3.2 The tree rules (one module, `services/canon/timeline_tree.rs`)

- `root_of(book)`, `parent_for(write, book, parent)` (resolves `None` → root and checks T6),
  `subtree(entry)` (an iterative walk; no recursive CTE needed, but one is fine),
  `check_consistency(…)` (T5) and `abandon(…)` (T7). Every service write calls them, and so does
  revert.
- **Create**: `parent` (omitted = top level) and `status` (`occurred` or `planned`; `abandoned`
  is not creatable). A planned entry under an occurred same-scope ancestor → conflict
  `occurred_ancestor`. Default placement per T2.
- **Update**: `parent` (or `top_level`) moves the entry **and its subtree**. The new parent may not
  be the entry itself, one of its descendants, abandoned, or out of scope. T5 is checked for the
  moved subtree under its new ancestors. `status`:
  - `planned → occurred`: `chapter_id` = the given chapter, else the session chapter, else NULL.
    A planned same-scope descendant → conflict `planned_descendant`.
  - `planned → abandoned`: T7. An occurred same-scope descendant → conflict
    `occurred_descendant`.
  - Anything else → conflict `backward`.
- **Delete**: refused with children; clears anchors first (T9).
- **Revert** (spec/06 §3): reverting an update restores `parent_id` and `status` too. If that would
  break T5 or T6 now, it is `409 timeline.status_conflict`. Reverting a create of an entry with
  children is `409 timeline.has_children`. A recreated entry gets its parent per T9.
- **Visibility for agents** (T8): every agent read filters `kind = 'entry' AND status !=
  'abandoned'`, plus the track visibility it already applies.

### 3.3 Tools

| Tool | Change |
|---|---|
| `add_timeline_event` | `parent` (entry id; omitted = top level), `status` (`occurred` default \| `planned`). `chapter_id` = session chapter for an occurred entry, NULL for a planned one |
| `update_timeline_event` | `parent` (entry id) or `top_level` (true), mutually exclusive; `status` (`occurred` \| `abandoned`); `chapter` (with `status: occurred`; a chapter number in the main track, as `from_chapter`; default = the session chapter); `note` (the change note, required with `status`) |
| `query_timeline` | `under` (entry id: only its descendants). Items stay in story order and gain `parent` (`↳ #12`) and `planned` (`（计划中）`). Abandoned entries and the root never appear. `from_chapter`/`to_chapter` keep matching by chapter, so planned entries, which have none, fall outside them |
| `list_foreshadowing` | Each item shows its anchor (`回收锚点：#12 标题 之后（计划中）`, or `（回收锚点已放弃）`) before the direction. Without Timeline, no anchor |
| `add_foreshadowing` | `payoff_event` (entry id) and `payoff_position` (`before` \| `after` \| `at`), both or neither; `planned_payoff` is the direction only |
| `update_foreshadowing` (new) | `foreshadowing_id`*, `note`*, `planned_payoff`, `payoff_event` + `payoff_position`, `clear_anchor` (true; exclusive with `payoff_event`). Shared rows are read-only in isolated tracks. Result `result.canon_write` |
| `resolve_foreshadowing` | Unchanged parameters; the result adds `anchor_planned` (`#id 标题`) when the anchor is planned |

Errors, each writing nothing:
- an unknown, foreign, root, abandoned or out-of-scope `parent` / `payoff_event` → `error.not_found`
  (`what_label` = timeline event);
- one of `payoff_event` / `payoff_position` without the other → `error.validation`;
- `parent` with `top_level`, or `payoff_event` with `clear_anchor` → `error.mutually_exclusive`;
- a T4/T5 breach → `error.timeline_conflict` (`reason`: `occurred_ancestor`, `planned_descendant`,
  `occurred_descendant`, `backward`; plus the conflicting `#id 标题`);
- deleting a parent (the book agent's `delete_canon_record`) → `error.timeline_has_children`
  (`count`);
- an anchor parameter while the session's Timeline is off → `error.system_off`.

Tool lists (spec/05 §2): `update_foreshadowing` goes after `add_foreshadowing` for the archivist
(22 → 23 tools) and the assistant (43 → 44). It belongs to Foreshadowing in the freeze map.
Sessions frozen before M22 keep their stored definitions. The handlers accept the old argument
shapes, because every new parameter is optional.

### 3.4 Context and prompts

- `block.memory`: `memory.counts.event` counts visible `entry` rows that are not abandoned. A new
  `memory.counts.event_planned` renders "（其中计划中 M）" when M > 0. The variable goes into
  `variables.toml`, the context struct, the samples and the validator.
- `result.timeline`: the items gain `parent_id` and `planned`. When any item is planned, a line
  says that entries marked （计划中） have not happened in the text yet.
- `result.foreshadowing_list`: the items gain `anchor {id, title, position, planned}` or
  `anchor_abandoned`.
- `result.search_hits`: event hits gain `planned`.
- `result.canon_write` gains `anchor_planned` (resolve) and `abandoned_count` (abandon).
- `result.error`: new branches `timeline_conflict` (one sentence per `reason`) and
  `timeline_has_children`.
- `block.goals`: event links of planned entries carry `（计划中）`; abandoned ones are left out.
- `block.canon_systems`: the Timeline-off line adds that foreshadowing takes no payoff anchor.
- `conception.system` deliverables 1 and 4 (T14).
- `archivist.system`:
  - a **时间线** rule: record by entry and sub-entry; planned versus occurred; promote when the
    text reaches an entry; abandon only planned entries;
  - a **伏笔** rule: anchor to the most specific entry; `planned_payoff` is the direction only,
    never a chapter number.
- `archivist.kickoff.seed`: the Timeline step adds planned entries from the outline with no cap
  (T15). The Foreshadowing step anchors, and records only what the outline states.
- `archivist.kickoff.maintenance`: step 5 promotes, nests and abandons; step 7 re-anchors; step 9
  considers `complete` for goal nodes linked to an entry the chapter promoted.
- `archivist.kickoff.resync`: occurred entries that the edited text no longer supports are
  edited or deleted.
- `tools.toml`: the parameters above; the new `update_foreshadowing` entry; the example payoffs
  carry no chapter numbers.

### 3.5 Goals (T13)

- `GoalCondition` gains `{ kind: "event_occurred"; event_id }`:
  - satisfied when the entry exists, is visible from the node's scope and is `occurred`;
  - broken when it is missing, invisible or `abandoned`;
  - otherwise (still planned) unmet.

  `services::goals::evaluate` gains one arm. Goal links and conditions may not name the root, and
  links may not name an abandoned entry when written.
- The Goals page condition editor gains **Event happened** with an event combobox (planned
  entries first).

### 3.6 Web

- **Timeline page**:
  - a tree with indentation and expand/collapse;
  - planned rows dashed with a **Planned** badge; abandoned rows greyed and struck through, behind
    a **Show abandoned** toggle (on by default);
  - the root is never rendered.
  - Row menu:
    - **Edit**;
    - **Add sub-entry**;
    - **Mark as happened** (planned only; optional chapter);
    - **Abandon** (planned only). The confirmation names the planned sub-entries it abandons too;
    - **Delete**, disabled with the reason while the entry has sub-entries.
  - Drag and drop reorders the story order among siblings (the move endpoint with sibling
    neighbours) and never changes the parent.
  - "+ Add" between rows inserts a sibling.
  - The dialog gains a **parent** select (scope-limited, excluding the entry and its subtree) and
    a **status** control limited to the allowed moves.
- **Foreshadowing page**: "Pays off after «event»", with a Planned or Abandoned badge, then the
  direction. A legacy row with no anchor shows its `planned_payoff`. The dialog gains an anchor
  picker (event combobox and position; clearable).
- **Goals**: the new condition kind; event link chips carry the planned marker.
- Both locales; `i18n:check`.

### 3.7 API

- `TimelineEventDto` gains `parent_id` (null = top level), `status` and `child_count`. `GET`
  returns every entry, abandoned included and the root never, in story order. The UI builds the
  tree.
- `TimelineEventInput` gains `parent_id?` and `status?` (`occurred` \| `planned`).
  `TimelineEventPatch` gains `parent_id?` (double option: `null` = top level) and `status?`
  (`occurred` \| `abandoned`); promotion uses the existing `chapter_id`.
- `ForeshadowingDto` gains `payoff_event_id`, `payoff_position`, `payoff_event_title` and
  `payoff_event_status`. Input and patch gain `payoff_event_id` and `payoff_position`, both or
  neither; a patch's `payoff_event_id: null` clears both.
- Errors: `409 timeline.has_children` (`details.children_count`) on DELETE and on reverting a
  create; `409 timeline.status_conflict` (`details.event_id`, `details.reason`) on POST, PATCH and
  revert. Unknown or out-of-scope parents and anchors: `400 validation.failed`.
- Bindings regenerated.

## 4. Tasks

Execute in order. One commit per task, with `scripts/check.sh` green before each commit.

| # | Task | Acceptance |
|---|---|---|
| 22.1 | **Docs**: this plan, D-170 and the §7 amendments. Re-read them against the code and fix any fact the code contradicts (note it in D-170) | `check.sh` green (docs only) |
| 22.2 | **Schema and types** (§3.1, §3.7): `0013` and its module (guards, roots, reparenting); root insert at book creation; entity, enums, snapshots with serde defaults; DTOs; bindings | §5.1; `entity_schema.rs` on fresh and upgraded databases |
| 22.3 | **Tree rules** (§3.2): `timeline_tree.rs`; the `kind = 'entry'` filter in every position query and reader; create, update, move, delete, revert; foreshadowing anchors and the delete-time clearing; API errors | §5.2 |
| 22.4 | **Tools** (§3.3): the parameter changes, `update_foreshadowing` (registry, freeze map, both role lists, validator map), the error kinds, the agent-visibility filter, search docs (root never indexed, abandoned removed, `planned` hit flag) | §5.3 |
| 22.5 | **Goals, context and prompts** (§3.4, §3.5): `event_occurred`, counts and markers, every prompt change, variables, samples, validator | §5.4; `prompts_registry` snapshots; each `kickoff_baseline` change justified |
| 22.6 | **Web** (§3.6): timeline tree, dialogs, foreshadowing anchors, goal condition, both locales | §5.5; `i18n:check` |
| 22.7 | **Migration and smoke**: smoke item 17 with the Demo provider. Run `0013` on a **copy** of the author's database; report the guard counts, the root id, that the 3 rows are top level and `occurred`, and that the 12 foreshadowing rows are unanchored with their text unchanged | the checklist passes; the report is in D-170 and the final message |

## 5. Tests

No test needs the network.

### 5.1 `timeline_tree_migration.rs`
- A pre-0013 fixture with two books (one without timeline rows) and a deleted event in the change
  log upgrades:
  - each book gets exactly one root, whose id is above the logged id;
  - every old row becomes top level and `occurred`;
  - foreshadowing keeps its text and has NULL anchors.
- A second root for a book violates the unique index.
- A broken copy rolls back and retries; `0001` is untouched; fresh and upgraded databases pass
  `entity_schema.rs`.
- A new book has its root.

### 5.2 `timeline_tree.rs`
- **Nesting.** Three levels deep. A child given no neighbour lands after its parent's subtree in
  story order. `after_event` across parents keeps the parent.
- **Statuses.**
  - A planned entry has no chapter even in a maintenance session.
  - Promotion records the session chapter, or the given one.
  - `occurred → abandoned` and `abandoned → *` are `backward`.
- **Consistency.**
  - A planned child under an occurred parent → `occurred_ancestor`.
  - Promoting with a planned descendant → `planned_descendant`.
  - A move that would break T5 is refused.
  - The root never counts.
  - A private planned child under a shared occurred entry is allowed.
- **Abandon.** It abandons the planned subtree with one change each, and is refused over an
  occurred descendant.
- **Scope.** A shared entry under a private one, a parent in another isolated track, an abandoned
  parent, a cycle: all refused.
- **Delete.**
  - A parent → `409 timeline.has_children`.
  - A leaf clears its anchors (logged).
  - Revert of the delete recreates it under its parent, or at top level when the parent is gone.
  - Reverting a create with children → `409`.
  - Reverting a promotion restores `planned` unless T5 now forbids it (→ `409
    timeline.status_conflict`).
- **API.** The root never appears in `GET` or a DTO, and `parent_id: null` round-trips.

### 5.3 `timeline_tree_tools.rs`
- `query_timeline` omits abandoned entries and the root, marks planned ones and parents, and
  supports `under`. A child of an abandoned entry shows at top level.
- `search_canon` returns no abandoned or root event, and marks planned hits.
- `add_foreshadowing` / `update_foreshadowing`:
  - anchor, re-anchor, clear;
  - half an anchor → `validation`;
  - an abandoned or foreign anchor → `not_found`;
  - Timeline off → `system_off`.
- `resolve_foreshadowing` against a planned anchor succeeds **and** carries the notice.
- Frozen lists: the archivist has 23 tools with every system on and relevant goal nodes (22
  without them), and Foreshadowing off removes `update_foreshadowing`.
- A pre-M22 frozen session still runs `add_timeline_event` without the new parameters.

### 5.4 `goals` / `prompts` / context
- `event_occurred`:
  - unmet while the entry is planned;
  - satisfied after promotion (the node becomes `ready`);
  - broken after abandonment;
  - broken when the entry is invisible from the node's scope.
- `block.memory` counts exclude the root and abandoned entries, and show the planned count.
- `block.goals` marks planned links and drops abandoned ones.
- `conception.system` deliverable 1 contains the relational rule and not "逐章写作" (asserted
  absent, so an edit cannot quietly restore it). The seed task asks for planned entries with no
  cap and forbids invented foreshadowing.

### 5.5 Vitest
- Timeline tree: nesting, expand/collapse, the planned badge, the abandoned strike-through and
  toggle, the root never rendered, adding a sub-entry, Delete disabled with sub-entries, the
  Abandon confirmation naming the sub-entries, drag and drop never changing the parent.
- Dialog: parent select excludes the subtree; the status control offers only allowed moves; a
  `409` surfaces its reason.
- Foreshadowing: anchored rendering with badges, legacy text-only rendering, the anchor picker.
- Goals: the **Event happened** condition.

## 6. Smoke item 17 (PLAN §M11.5)

17. Timeline as history and plan (M22):
    - Demo, a fresh book with conception: the canon seed creates planned entries from the outline,
      nested under phase entries, and anchors a foreshadowing to one of them; no chapter number
      appears on the Timeline or Foreshadowing page.
    - Write and approve chapter 1. Maintenance promotes one planned entry to occurred with
      chapter 1 and adds a sub-entry.
    - A goal task waiting on **Event happened** for that entry is badged ready afterwards.
    - Abandon a planned entry: its planned sub-entries go with it, it stays greyed on the page,
      and a session's `query_timeline` no longer returns it. **Abandon** is not offered on an
      occurred entry.
    - **Delete** is disabled on an entry with sub-entries; deleting a leaf and reverting it
      brings it back under its parent.

## 7. Amended documents (with this plan)

- `docs/PROJECT.md`: glossary (timeline event, foreshadowing), §6.7, §6.16, §8.8, §9.2, §10 #35,
  §11.
- `docs/spec/02-data-model.md`: §1 (0013), §2 `timeline_events` and `foreshadowing`, §3 enums,
  §4 `GoalCondition` and snapshots, §5 invariants 23–26.
- `docs/spec/05-tools.md`: §2 lists and freeze map, §3 `query_timeline` and
  `list_foreshadowing`, §6 timeline and foreshadowing tools, §9 tests, §10 `delete_canon_record`.
- `docs/spec/06-canon-memory.md`: §1.4, §1.5, §3, §6 steps 5, 7 and 9, §7, §8, §9, §10.3, §11.3,
  §11.7.
- `docs/spec/07-knowledge-search.md`: §2 the `event` doc row.
- `docs/spec/08-workflows.md` §5: the conception outline's content.
- `docs/spec/09-prompts.md` §2: the prompt table.
- `docs/spec/10-api.md` §2.4 and §4.
- `docs/spec/11-frontend.md` §5.5 and §9.
- `docs/PLAN.md`: M22 and smoke item 17. `docs/README.md`: M0–M22.
- `docs/DECISIONS.md`: **D-170**.

## 8. Risks

- **Position semantics.** T2 keeps `position` global on purpose. The risk is a tree view whose
  sibling order looks odd when a child is placed chronologically before its parent. That is
  allowed; the UI shows siblings in story order and does not reorder parents.
- **Seed volume.** "No cap" means a 110-chapter outline can produce well over a hundred planned
  entries. Reads stay capped (`query_timeline` limit 50, `under`), and nothing renders the whole
  timeline into a prompt.
- **Frozen sessions.** Old sessions keep old tool definitions; every new parameter is optional,
  and old snapshots load with defaults.
- **T5 across scopes** counts only the entry's own scope, so a side story can plan under
  main-story history. A side story cannot promote shared entries anyway.

## 9. Out of scope

- Converting existing books automatically (T18 names the manual and book-agent path).
- Chapter ranges on timeline entries, or entries as containers of chapters (PROJECT §9.2).
- Deriving an entry's status from its children.
- Seeding goals (unchanged: goals are never seeded).
