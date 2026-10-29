# 06 — Canon and Tiered Memory

Canon is the structured state of the story: entities, relations, appearances, timeline events,
memory notes (facts, world rules), foreshadowing, and chapter summaries. Together with the
setting materials it is what writer and reviewer rely on. All canon writes go through services
(`services/canon/*`), which validate, record `change_log`, emit `canon.changed`, and enqueue
re-indexing (spec/07 §4).

**Writing goals** (§11) sit beside canon: goals, plans and tasks that state the author's narrative
intent, each with a mandatory purpose. They are not story facts. They are kept, scoped, logged and
reverted like canon, because their readiness is computed from canon and they link to it. Goal
writes go through `services/goals.rs` and are not indexed for search.

Canon falls into **five systems**: Entities, Timeline, Memory, Foreshadowing and Goals. Each can be
switched off per book (§10). A disabled system keeps all its rows but stops reaching agents.

## 1. Field semantics

### 1.1 Entities
| Field | Meaning | Example |
|---|---|---|
| `name` | the title, unique among visible entities | 林凡 |
| `aliases` | other names used in text | 凡哥, 林师弟 |
| `summary` | one line: who or what it is | 青云宗外门弟子，身怀残破剑灵 |
| `description` | durable profile (appearance, personality, background, abilities) as plain text, **about 120 characters (2–4 sentences)** — stable reference, not a chapter log (D-142) | 青云宗外门弟子，父母早亡，由祖母抚养；寡言，剑术天赋极高，左手有旧伤 |
| `state` | the **current, immediate** state: location, condition, possessions in hand, goal, mood, open situation | 身处剑冢第三层，左臂受伤，持青冥剑，急于寻找师妹 |
| `attributes` | key facts as key/value strings, overwritten when they change; one field per fact, never prose | 修为=筑基中期；年龄=17；所属=青云宗 |
| `importance` | 5 core (protagonist, main antagonist, central item) · 4 major · 3 recurring · 2 minor (default) · 1 background | |
| `status` | `active`, `inactive` (out of the story for now), `dead`, `destroyed`, `missing`, `unknown` | |
| `is_protagonist` | at most one per book; its `state` is short-term memory | |
| `first_chapter_id` / `last_chapter_id` | maintained from appearances (shared entities: shared-canon chapters only; private entities: their track), ordered by (track `ord`, chapter `ord`); for display ("last seen"). Recomputed for every affected entity whenever a chapter's appearance set changes: `record_appearances`, a revert of it, a merge, or deleting the chapter | |

Kinds: `character`, `organization` (sects, nations, companies), `location`, `item`, `other`
(techniques, species, concepts).

The four text fields are a hierarchy, not four places to write the same thing: `summary` is the
one-line identity, `state` the instant condition at the end of the chapter, `attributes` the
discrete facts, and `description` the durable reference text. Per-chapter developments belong to
`state`, `add_memory_note` and `add_timeline_event` — **never appended to `description`**. Only a
permanent redefinition (a true identity reveal, a life-changing fact) may extend a `description`,
and when the text is already near the ceiling the agent rewrites it tighter instead of appending
(D-142). The ceiling is prompt-level guidance in `prompts/tools.toml` and the archivist prompts,
not a schema constraint: a hard cap would either drop information or force a rewrite loop, and the
archivist cannot ask the author what to cut.

### 1.2 Relations
- Directed. `kind` says what **source** is to **target** ("林凡 是 张三 的 师兄" → source 林凡,
  target 张三, kind 师兄). `inverse_kind` says what target is to source (师弟). Symmetric
  relations use the same word for both (同学 / 同学).
- `kind` is **data about a bond, not part of its identity.** At most one **active** relation may
  exist per ordered `(source, target)` pair in shared canon, and one per isolated track. A pair
  that already has an active relation and is written again with different wording —
  `师兄` → `同门师兄`, or a note from a new perspective — **edits that row**; it never opens a
  second active one. A `POST` that would create a second active relation for a pair returns
  `conflict.exists` with `details.id` naming the existing row, so the caller can edit it instead.
  Without this rule an archivist that re-words a relationship leaves two active lines that mean
  the same thing, which is what the author saw (D-143).
- The pair is **ordered**: `(A, B, 父亲)` and `(B, A, 嫡长子)` are two legitimate rows, because
  each describes a different direction of the same bond and `inverse_kind` cannot carry both
  wordings. A relation `A → B` and a relation `B → A` may therefore both be active. Treating the
  pair as unordered would reject canon this product considers correct.
- Ending a relation keeps it (status `ended`, `until_chapter_id`), so history stays queryable. An
  `ended` row never blocks a new active relation for its pair: a bond that lapses and resumes is a
  new row, and the `ended` ones are the history of that bond.
- Tool and UI rendering always phrases a relation from the queried entity's perspective, using
  `inverse_kind` when the entity is the target (falling back to "张三 是 林凡 的 师兄").
- Several relations of different kinds per pair are still not expressible in one row; a pair that
  genuinely carries two simultaneous bonds (`师兄` and `情敌`) keeps the more important wording in
  the relation and records the other as a `description` clause or a memory note.

### 1.3 Appearances
One row per (chapter, entity) with a role: `pov`, `major`, `minor`, `mentioned`. The archivist
replaces the set per chapter. They drive mid-term memory and `first/last_chapter_id`.

### 1.4 Timeline events
- `position` is **story chronology**, not chapter order. A flashback in chapter 40 to events
  20 years earlier gets a position near the start.
- `story_time` is a free label for humans and models ("天元历三千年春", "Day 3, night").
- Insertion between neighbors uses the midpoint of their positions. Renormalize when a gap is
  below 1e-6 (spec/02 §1).
- `chapter_id` is where the event was narrated (or established).

### 1.5 Memory notes and foreshadowing
- `fact`: something established in text that must stay true ("林凡的玉佩是母亲遗物",
  "苏婉不知道林凡的真实身份"). Knowledge asymmetries are facts tagged with the entities involved.
- `world_rule`: a rule of the world as established in text ("筑基之前无法御剑飞行"). The world
  setting holds the designed rules; notes hold what the text has actually established or added.
- `other`: anything else worth remembering (recurring jokes, promises, debts).
- Notes are **retired**, not deleted, when the text overturns them. The change note says why.
- Foreshadowing: `open` → `resolved` (with resolution and chapter) or `abandoned`.

### 1.6 Chapter summaries
150–400 characters per official chapter, written by maintenance. They cover what happened, who
was involved, and how the chapter ended. Summaries are short-term memory for later chapters and
are indexed for search.

## 2. Visibility (side stories)

```rust
pub struct Visibility { pub book_id: i32, pub private_track: Option<i32> }   // Some(T) only for isolated side tracks
```

| Session track | Reads see | Writes go to |
|---|---|---|
| main track | rows with `track_id IS NULL` | `track_id = NULL` |
| side track, `canon` mode | rows with `track_id IS NULL` | `track_id = NULL` |
| side track, `isolated` mode (T) | rows with `track_id IS NULL OR track_id = T` | new rows: `track_id = T`. Updates to shared rows are rejected (`error.isolated_readonly`); updates to T's own rows are allowed |

- A helper `visible_condition(vis, column)` builds the SeaORM `Condition`. Every canon query
  used by tools, memory, and search must use it, and tests cover it.
- The UI's canon pages show shared rows by default, with a track filter that adds an isolated
  track's private rows (badged).
- **Anchors** (spec/08 §6) do not filter rows. They add "after the anchor" markers: a change
  note, event, passage, or summary whose chapter is a main-track chapter with
  `ord > anchor_chapter.ord` is marked `（锚点之后）`. For `before_start`, every main-track item is
  marked. For `after_end`, nothing is.
- **Goal nodes** use the same `track_id` rule for storage, links and the Goals page. Injection does
  not follow read visibility: it follows the node's **storyline**, so a side story never receives
  main-story plans or tasks (§11.2).

## 3. Change log and revert

- Every create, update, or delete of a canon or setting target writes one `change_log` row with
  full `before`/`after` snapshots (spec/02 §4) and a `note`. Agents must give notes for updates.
- `note` only ever holds written text, never generated wording, because notes are shown to
  models and must not be in a UI language:
  - agent changes: the agent's note;
  - author edits: the optional **change note** field of the edit forms (empty by default);
  - accepted proposals: the proposal's rationale;
  - merges: the language-neutral `#B → #A`;
  - reverts, reorders, and other system changes: empty.
  The UI shows a localized label derived from `actor`, `op`, `target_type`, and `reverts_id`
  ("Edited by you", "Reverted #12", …) next to the note. Model-facing renderings (entity cards,
  mid-term memory) include only rows with a non-empty note.
- `appearance_set` and `chapter_summary` targets are keyed by chapter id.
- **Revert** `POST /api/changes/:id/revert`:
  1. Allowed only if it is the newest non-reverted change of its target. Otherwise
     `409 change.not_latest` (the UI says "revert the later changes first" and links them).
  2. `create` → delete the target. `update` → restore the `before` snapshot. `delete` →
     recreate from `before` (a new id; join rows such as entity links are restored when their
     entities still exist; relations of a deleted entity are not restored, and the UI states it).
  3. The revert itself is a new change with `reverts_id`. The original gets `reverted_by_id`.
- Reverted changes do not appear in mid-term memory.
- **Goal nodes** (§11) are logged as `goal`, `plan` or `task`, the node's level, which never
  changes. `target_id` is `goal_nodes.id` and the snapshot is `SnapshotGoalNode` (spec/02 §4).
  Revert follows the rules above, with three additions:
  - reverting a `create` of a node that now has children is refused with `409 goal.has_children`;
  - a restored or recreated node gets its parent back only when that parent still exists in the
    same scope (otherwise it becomes top-level), and its links only for targets that still exist;
  - a restored `done` or `dropped` needs no open-children confirmation, because a revert is an
    explicit undo.

  Goal changes never reach model-facing memory.

## 4. Tiered memory

`services/memory.rs` builds a `MemorySnapshot` for a **target position** (track, chapter) from
canon as it is now. It is used by `block.memory` and by the Memory page API. "Tiered memory" is
the assembly of all canon systems plus session scaffolding, not the Memory system alone. The
snapshot leaves the parts of disabled systems empty and skips their queries (§4.6).

```rust
pub struct MemorySnapshot {
    // short-term
    pub chapter: ChapterBrief,                    // label, title, guidance, outline (if any), target words
    pub recent_summaries: Vec<SummaryItem>,       // oldest → newest
    pub previous_tail: Option<TailItem>,          // ending of the previous chapter
    pub protagonist: Option<EntityStateItem>,     // name, state, attributes, status
    // mid-term
    pub recent_changes: Vec<ChangeItem>,          // entity and relation change notes in the window
    pub appearances: Vec<AppearanceItem>,         // characters seen in the window
    pub absent_important: Vec<AbsenceItem>,
    // long-term index
    pub key_notes: Vec<NoteItem>,                 // facts and world rules with importance ≥ 4
    pub open_foreshadowing: Vec<ForeshadowItem>,
    pub cast: Vec<CastItem>,
    pub counts: CanonCounts,
    pub anchor: Option<AnchorInfo>,               // side tracks
}
```

### 4.1 Window and ordering
- **Prior chapters** P = official chapters of the target's track with `ord` < target `ord`. For
  a side track, if P has fewer than max(N, M) chapters, main-track official chapters up to the
  anchor are added in front until it has max(N, M) (labeled with their main-track label).
- **Short-term window** = the last N chapters of P (`recent_summary_count`, default 5).
- **Mid-term window** W = the last M chapters of P (`midterm_window`, default 10).
- **Recent chapters in full** (`block.recent_chapters`, design §7) use the same P and top-up
  rule, with `wanted = recent_chapter_count` (default 2; `0` disables the block). Only official
  chapters are candidates; a draft, an in-review chapter and a rejected version are not approved
  prose and are never top-up sources. The target is excluded (`ord` < target). Chapters with
  empty text are skipped. The per-chapter cap is `target_words_max × 1.5` (6,000 characters at
  the 4,000 default, and 6,000 when the target is unset); the total budget is
  `recent_chapter_count × cap`, filled newest-first. A chapter over its cap keeps its ending and
  loses its front at a paragraph boundary; when the remaining budget is smaller it is cut to the
  remaining budget and older chapters get nothing. A front-cut chapter is rendered as partial.

### 4.2 Short-term
- `recent_summaries`: label, title, and summary of each chapter in the short-term window. A
  missing summary renders as "（尚无摘要）".
- `previous_tail`: the last paragraphs of the latest chapter in P, as many as fit in 800
  characters (at least one paragraph). For the first chapter of a side story with an
  `after_chapter` anchor, the anchor chapter's tail.
- `protagonist`: the side track's `focus_entity_id` when set (a side story about another
  character), otherwise the visible entity with `is_protagonist = 1`. Its `state`, `status`, and
  its attributes (all of them, since they are key facts). In a side track whose anchor lies before
  the latest main chapter, `after_anchor` is set when the newest change of the state came from a
  main chapter after the anchor, so the model knows the state may not apply yet (the same flag
  marks the state in `get_entity`).
- `chapter`: the target chapter's label, title, the author's guidance, and the target length.
  `block.memory` renders only the label and title of these, because every writer and reviewer
  task states the guidance and the target length itself (the Memory page shows all of them).
  Its outline is included only when the kickoff has no `block.chapter_draft` (which renders the
  outline itself: revise, review, maintenance, rollover, and every write session) and is not a
  redraft (whose task shows the feedback and the accepted outline's replacement), so it never
  appears twice. A `chapter_outline` kickoff never renders the outline in `block.memory`: its
  task shows the outline to replace, and the chapter draft block carries the existing text.

### 4.3 Mid-term (structured recent evidence)
- `recent_changes`: visible (`change_log.track_id`, §2), non-reverted rows with
  `target_type ∈ {entity, relation}`, `chapter_id ∈ W`, and a non-empty note. Ordered by
  chapter, then id; at most 40 (the newest are kept). Rendered as
  `第N章 · 林凡(#12)：修为 筑基初期→筑基中期`.
- `appearances`: characters with appearances in W. For each: chapter labels with roles and a
  count. Sorted by count desc; at most 30.
- `absent_important`: active characters with importance ≥ 4 that appear in P but in no chapter
  of W (computed from `appearances`, so it works across tracks). At most 10, with the label of
  their latest appearance in P.

### 4.4 Long-term (index only; details via tools)
- `key_notes`: active facts and world rules with importance ≥ 4, at most 15.
- `open_foreshadowing`: open items sorted by importance desc, then planted chapter asc; at most 20.
- `cast`: visible entities sorted by importance desc, then last appearance desc; at most 40
  (`#id 名字【类型】— 简介`).
- `counts`: entities per kind, timeline events, active notes per kind, open foreshadowing.
- Everything else (full entity cards, relations, the timeline, all notes, past chapters,
  knowledge) is reached through retrieval tools.

### 4.5 Rendering
`block.memory` (prompts/blocks.toml) renders the snapshot in three titled sections
(短期记忆 / 中期记忆 / 长期记忆索引). Empty parts are rendered explicitly (a first chapter is
told that there is no earlier text; a later chapter written before its predecessors are official
is told that none of them is official yet), so the model never has to guess. `block.cast_index`
(maintenance) lists up to 300 visible entities with aliases, so the archivist matches existing
entities instead of duplicating them.

`GET /books/:id/memory` returns the exact `block.memory` text plus `recent_chapters`, the chapters
`block.recent_chapters` injects in full for the same position (id, label, title, characters and
whether the chapter was front-cut). The Memory page renders the `block.memory` variant and lists
those chapters, so the promise that the page shows what the writer receives stays true even though
the full text travels in its own kickoff block (design §7). It renders with the book's current
`canon_systems` and returns them as `systems`, so the page leaves out the sections of disabled
systems exactly as the next session will.

### 4.6 Fragments and canon systems

`block.memory` receives `systems` (the session's frozen `CanonSystems`, spec/03 §4). Every part
belongs to the session (**scaffolding**, always rendered) or to one canon system (rendered only
when that system is on):

| Part | Owner |
|---|---|
| block header, `短期记忆` tier header, the current-task line (chapter label and title) | scaffolding |
| `is_first` lines (first chapter; none official yet) | Memory |
| recent chapter summaries | Memory |
| previous chapter's tail | Memory |
| protagonist / focus character state, status, attributes, after-anchor flag | Entities |
| chapter outline echo (only when no `block.chapter_draft` renders, §4.2) | scaffolding |
| `中期记忆` tier header and all three mid-term parts (state and relationship change notes, which come from `change_log` rows of entities and relations, not from memory notes; appearance records; absent important characters), including the "no changes recorded" line | Entities |
| `长期记忆索引` tier header | any of Memory, Foreshadowing, Entities, Timeline |
| key facts and world rules | Memory |
| unresolved foreshadowing | Foreshadowing |
| main cast | Entities |
| archive counts row | one fragment per system: entity kinds (Entities), events (Timeline), facts and world rules (Memory), open hooks (Foreshadowing). The template joins the enabled fragments and omits the row when none is enabled |

- A tier header renders only when something under it renders. The block itself always renders,
  because the scaffolding is always there.
- With the four canon systems on, the block's text is byte-identical to the pre-D-150 template.
- The outline echo is scaffolding so that the IR-01 fallback survives Memory off: when an override
  blanks `block.chapter_draft`, the outline still reaches the writer through `block.memory`.
- **Continuity without Entities:** the protagonist's state is entity data. With Entities off it is
  not injected, and it would go stale, because maintenance no longer updates it. Continuity then
  rests on the previous tail and the summaries (Memory), `block.recent_chapters`, and the
  protagonist document in `block.settings`.
- `block.cast_index` belongs to Entities and is not emitted while Entities is off.

## 5. Setting proposals and goal proposals

Both kinds live in `setting_proposals`. Setting proposals come first; goal proposals follow at the
end of this section.

- Created only by `propose_setting_change` (archivist). The proposal stores the edits exactly.
- The UI shows the target document's current text with the edit applied as a diff, and the
  rationale.
- **Accept**: in one transaction, re-check that each `find` matches exactly once in the current
  text (for `character:<name>` targets, the design's `profile`). Apply the edits, `append`, or
  `create_character` (rank = last). Bump the revision, record a change (`actor: user`, note = the
  proposal's rationale), and set `accepted`. If a `find` no longer matches, the target character
  design was deleted or renamed away, or a new character's name has been taken meanwhile →
  status `conflict`, nothing applied; the author edits by hand and dismisses it (Reject).
- **Reject**: status `rejected` (from `pending` or `conflict`).
- Accepted changes affect sessions created afterwards (frozen kickoffs, spec/03 §1.3).

**Goal proposals** (D-150) are created only by `propose_goal_change` (§11.6) and use the same
lifecycle:
- **`goal_node`**: `target_id` = the node, edits `{kind: "set_goal_status", status: done |
  dropped}`. **Accept** re-checks that the node still exists and is `active` or `waiting`;
  otherwise `conflict`. When the node has open descendants, accept answers `409
  goal.open_children` (§11.4) unless its body says `{confirm_open_children: true}`.
- **`new_goal_task`**: `target_id` = the parent plan or NULL, edits `{kind: "create_goal_task",
  task: NewGoalTask}`. **Accept** re-checks that the parent, when given, still exists, is a plan,
  is open and is in the same scope; otherwise `conflict`. Links whose targets are gone are dropped.
  The task is created `active`.
- Both record the change with `actor: user`, the rationale as note, and the proposal's chapter.
  Reject is unchanged.
- The proposal's `preview` stays empty for goal targets. `ProposalDto.goal_target` carries the
  node, or the new task's parent, for display.

## 6. Maintenance procedure (archivist, `chapter_maintenance`)

Triggered by approval (when `auto_maintenance` is on) or manually. Kickoff: settings, the
disabled-systems notice, memory (for this chapter's position), cast index, the relevant goals, the
official text with pids, and the `archivist.kickoff.maintenance` task.

Expected steps (the prompt explains them; `finish_maintenance` validates the required ones). A
step renders, and its tools are frozen, only while its canon system is on in the session's
`canon_systems` (§10.1):
- steps 1–4: Entities;
- step 5: Timeline;
- steps 6 and 8: Memory;
- step 7: Foreshadowing;
- step 9: Goals, and only when the chapter has relevant goal nodes (§11.6).

The template numbers the rendered steps contiguously, so without step 9 the settings step is 9 and
`finish_maintenance` is 10, as before D-150. With everything on and no goals, the text is the
pre-D-150 text.

1. **Appearances** (required): `record_appearances` for every entity that appears or is
   mentioned.
2. **Entities**: `create_entity` for new named entities that matter. `update_entity` for changed
   state, attributes, status, or a permanent redefinition of the entity. Each update has a note.
   This step is **not** a place to extend `description` with what happened in the chapter: the
   chapter's developments go to `state`, `add_memory_note` and `add_timeline_event`, and a
   `description` that has grown past its ceiling is rewritten tighter (§1.1, D-142).
3. **Protagonist state** (expected when they appear): `update_entity(protagonist, state=…)`
   reflecting the end of the chapter (in a side track with a focus character, that character).
   In an isolated track a shared protagonist is read-only, so their in-story changes become a
   track-local note (`add_memory_note`), and `finish_maintenance` does not ask about the state.
4. **Relations**: `set_relation` / `end_relation` with notes for relationship changes.
5. **Timeline**: `add_timeline_event` for plot-relevant events, positioned chronologically
   relative to existing events (use `query_timeline` to find neighbors).
6. **Notes**: `add_memory_note` for new facts and world rules; `retire_memory_note` for overturned
   ones.
7. **Foreshadowing**: `add_foreshadowing` for planted hooks; `resolve_foreshadowing` for payoffs.
8. **Summary** (required): `set_chapter_summary`.
9. **Goals** (optional, D-150): `propose_goal_change` against the goals block. Propose `complete`
   for a node whose purpose the chapter achieved, `drop` with the reason for one it made
   unreachable, and `create` for a new task where the chapter opened an opportunity under a listed
   plan. Skipped when the chapter moved no goal.
10. **Settings**: `propose_setting_change` only when the chapter contradicts the setting materials
    or establishes something substantial that belongs in them (never in isolated side stories).
11. `finish_maintenance(report)`.

**Required steps follow the session's systems.** Appearances are required only with Entities,
the summary only with Memory, and the protagonist-state confirmation runs only with Entities. A
disabled system therefore never leaves a chapter `syncing` or `failed`. With all five systems off,
maintenance still runs when `auto_maintenance` says so (it may still propose setting changes),
completes with `finish_maintenance`, and leaves the chapter `synced`. The isolated-track
fallback of step 3 (record a shared protagonist's change as a track note) is offered only while
Memory is on.

**Completion handler**: `synced_version_id = official_version_id`; `canon_status = synced` if
the chapter's `content_version` still equals the session's `base_content_version`, otherwise
`stale` (the author edited the text during maintenance). Passages were indexed at approval
(spec/07 §2); the summary doc is upserted by `set_chapter_summary`.

**Failure**: when the session fails, `canon_status = failed` and the chapter shows **Retry
maintenance** (which continues that session); continuing sets it back to `syncing`. Abandoning
an open maintenance or re-sync session, in any status, sets (or leaves) `canon_status = failed`,
so the chapter never stays `syncing` without a session; Retry then starts a new session. Changes
made before the failure remain (they are individually revertible).

## 7. Canon seeding (`canon_seed`, at conception finalize)

Kickoff: settings and `archivist.kickoff.seed`. The archivist creates the core entities from the
setting materials: characters from the character designs (the protagonist, i.e. the rank-1
design that the protagonist document describes, with `is_protagonist: true`; importance from
rank), key organizations, locations, and items from the world setting, plus relations from the
designs (family, sect, rivalries) and world rules as notes (importance from emphasis). It
records backstory events from the outline as timeline events. It does not invent facts the
materials do not state. `create_entity` links each character entity to the design with the same
name (spec/05 §6); at completion the service links any remaining designs by exact name or alias.

The seed task (`systems` variable) lists only the steps of enabled systems: characters,
organizations, places, items and relations (Entities); world rules and facts (Memory); backstory
events (Timeline); planned hooks (Foreshadowing). Finalize starts `canon_seed` only when at least
one of those four systems is on. Goals are never seeded.

## 8. Re-sync after editing official text (`chapter_resync`)

When the author edits an official chapter whose canon is `synced`, `canon_status` becomes
`stale`. **Re-sync canon** first saves the current text as a new `official` version V
(`official_version_id = V`; an existing official version with identical content is reused), then
starts an archivist session whose kickoff contains `block.chapter_diff`: the paragraph-level
diff between `synced_version_id` and V (inserted, deleted, and modified paragraphs with pids,
both texts for modifications). The task (`archivist.kickoff.resync`) asks it to update canon so
it matches the new text: adjust entities, relations, events, and notes, retire contradicted
notes, and update the summary if needed. The kickoff shows only the diff, so the task tells it to
read the whole chapter with `read_chapter` before it re-records appearances (always the complete
set) or rewrites the summary. On completion `synced_version_id = V`, and the status
follows the maintenance rule (`synced`, or `stale` if the text changed again meanwhile). The
re-sync task (`systems` variable) names only the record kinds of enabled systems. Its
re-record-appearances bullet needs Entities and its summary bullet needs Memory. It has no goals
step.

## 9. Manual canon editing (UI)

The UI uses the same services with `actor: user`. Additional author-only operations:
- **Merge entities** (A ← B), in one transaction: move B's relations to A, dropping relations
  between A and B (they would become self-relations) and ones that would duplicate an active
  relation of A; move B's appearances (where both appear in a chapter, A keeps the more
  prominent role); move B's event, note, foreshadowing and goal-node links (skipping duplicates),
  rewrite goal conditions `entity_status_is(B, …)` to A, move its
  character design link, side tracks' `focus_entity_id`, and B's `is_protagonist` flag. Union the
  aliases (B's name becomes an alias), delete B, and record one `update` change on A and one
  `delete` on B (both with the note `#B → #A`). A shared entity cannot be merged into an
  isolated track's private entity (`400 validation.failed`); a private one can be merged into a
  shared one, and its moved private rows keep their `track_id`. Merges are not revertible as a
  whole; the UI warns.
- **Set protagonist**: moves the `is_protagonist` flag.
- **Reorder timeline** by drag and drop: one `update` change per moved event (position).

## 10. Canon system switches (D-150)

### 10.1 What a system owns

| | Entities | Timeline | Memory | Foreshadowing | Goals |
|---|---|---|---|---|---|
| Rows | entities, relations, appearances | timeline events and their entity links | memory notes and their links, `chapters.summary` | foreshadowing and its links | goal nodes and their links (§11) |
| Tools (spec/05 §2) | `get_entity`, `list_entities`, `get_relations`, `create_entity`, `update_entity`, `set_relation`, `end_relation`, `record_appearances` | `query_timeline`, `add_timeline_event`, `update_timeline_event` | `add_memory_note`, `retire_memory_note`, `set_chapter_summary` | `list_foreshadowing`, `add_foreshadowing`, `resolve_foreshadowing` | `propose_goal_change` |
| `search_canon` kinds | `entity` | `event` | `note`, `summary` | `foreshadowing` | — |
| Context | its `block.memory` fragments (§4.6), `block.cast_index` | its counts fragment | its `block.memory` fragments | its `block.memory` fragments | `block.goals` |
| Maintenance (§6) | steps 1–4, required appearances, protagonist confirmation | step 5 | steps 6 and 8, required summary | step 7 | step 9 |
| Seed (§7) | characters, organizations, places, items, relations | backstory events | world rules and facts | planned hooks | — |
| UI | Entities page | Timeline page | Memory page | Foreshadowing page | Goals page, Referenced-by sections, ready badge, overview hint |

These belong to no system and are never switched off: setting materials, writing skills,
`block.recent_chapters` (a length setting), the chapter text blocks, `block.anchor`, chapter
reading and search, the knowledge tools, `propose_setting_change`, `finish_maintenance`, and the
Changes and Proposals pages.

### 10.2 Storage, defaults and freezing

- `BookSettings.canon_systems` holds one switch per system, all on by default. New books copy
  `book_defaults`. Rows written before D-150 load with all five on, and nothing is back-filled.
- Every session freezes the book's value at creation (`agent_sessions.canon_systems`, spec/03 §4).
  A rollover child inherits its parent's. NULL (sessions created before 0006) means all on.
- All runtime decisions read the frozen value: the tool list, the kickoff blocks and fragments,
  `search_canon`'s kinds, and `finish_maintenance`'s required steps. A switch therefore never
  changes a running session (its prompt, tools, behaviour or rollover children). Retrying a failed
  maintenance continues the same frozen session; a new session reads the current settings.

### 10.3 Effects of turning a system off

1. **Not injected.** Its fragments and blocks are not rendered (§4.6, spec/03 §10).
   `block.canon_systems` names the disabled canon systems among Entities, Timeline, Memory and
   Foreshadowing, because the static system prompts still describe them (spec/09 §5.1). It says:
   - their records are absent from the context;
   - their tools are not in the tool list;
   - `search_canon` will not return their kinds;
   - with Entities off, the `entities` parameters of other tools stay empty.

   It renders blank when all four are on. Goals has no static mention and is never listed.
2. **Not maintained.** Its steps are not rendered, its tools are not frozen, and its required
   steps are not required (§6–§8). No chapter becomes `stale` or `failed` because of it.
3. **No tools.** Spec/05 §2.
4. **No navigation entry.** The page stays reachable through Book settings › View data and by URL,
   under a banner that offers **Turn on**.
5. **Nothing is deleted.** Its rows stay. Author edits through its page, revert and the Changes
   page keep working.
6. **No back-fill on re-enable.** Chapters maintained while it was off keep their
   `canon_status`, and the system resumes from the next maintained chapter. Re-running older
   chapters would regress current state, for example an older protagonist state over a newer one.

Other rules:
- Overview rule 2 (`seed_canon`) applies only while Entities is on (spec/08 §7).
- `recent_chapter_count`, `auto_maintenance` and the memory windows stay independent (PROJECT
  §10.24).

## 11. Writing goals (D-150)

### 11.1 Fields

| Field | Meaning | Example |
|---|---|---|
| `level` | `goal` (an arc, a volume or the whole book: what the book should make the reader feel, understand or believe), `plan` (the next tens of chapters: what must change for the goal to become reachable), `task` (the next few chapters: in which scene or event the change happens, and how) | |
| `title` | short name, ≤ 120 characters | 角色弧光 · 一：从被保护者到施害者 |
| `purpose` | **mandatory**, ≤ 1,000 characters: why the node exists and what it is for, in the author's words. It is never generated by code: the author writes it, or the author accepts an agent's proposal that carries it. It is the payload agents receive | 在 40–70 章之间，让她的保护者身份逐步失效，并被她自己亲手打破一次。 |
| `status` | `active`, `waiting`, `done`, `dropped` | |
| `conditions` | what a `waiting` node waits for (§11.3) | |
| `chapter_from` / `chapter_to` | optional range of chapter numbers in the node's storyline (§11.2) | 60–62 |
| links | entities, timeline events and foreshadowing the node is about or staged around | |

Vocabulary: the system is 写作目标, and the levels are 目标 · 阶段计划 · 情节点 with a 目的
(purpose), in prompts and in the zh-CN UI. 「任务」 already means a session's task and 「计划」 the
writer's revision plan. In the English UI: Goals; Goal, Plan, Task; Purpose.

### 11.2 Structure and scope

- **Parents:** a goal has none; a plan has none or a goal; a task has none or a plan. Tasks have no
  children. One `parent_id` column makes a second parent impossible: attaching a node elsewhere
  moves it. `level` and the scope never change.
- **Scope** (`track_id`) follows §2: NULL for shared nodes, which steer the main story, and `T`
  for nodes private to isolated side track `T`. Canon-mode side tracks have no private nodes.
- **Storyline:** the main track for a shared node, `T` for a private one. Ranges and
  `chapter_at_least` count chapters of the storyline. They are **positions**: they may name
  chapters that do not exist yet, and reordering or deleting chapters does not renumber them.
- **Same scope:** a node's parent is in its own scope. Links and conditions may only name rows
  visible from that scope (a shared node: shared canon only).
- **Storylines for injection** (§11.5), for a session on track *S*:

  | *S* | Candidates |
  |---|---|
  | main track | shared nodes |
  | isolated side track `T` | `T`'s private nodes, plus the **book-wide goals**: shared, level `goal`, no chapter range |
  | canon-mode side track | the book-wide goals |

  Private nodes therefore never reach another track, and main-story plans and tasks never reach a
  side story.

### 11.3 Conditions and readiness

| Condition | Satisfied when | Broken when |
|---|---|---|
| `chapter_at_least {chapter}` | the storyline has an official chapter numbered ≥ `chapter` (a later chapter approved first after a forced write counts) | never |
| `foreshadowing_resolved {foreshadowing_id}` | the row exists, is visible from the node's scope, and is `resolved` | it is missing, invisible, or `abandoned` |
| `entity_status_is {entity_id, status}` | the entity exists, is visible, and has that `status` | it is missing or invisible |

- A broken condition cannot become true without an edit. The Goals page says why.
- At most 10 conditions per node; duplicates are dropped. Targets are checked when written.
- There is no timeline condition. An event can only be picked once it exists, so "the event
  exists" would be true at once. Timeline events are still linkable.
- **Derived states** (never stored):
  - `ready` = `waiting` and every condition satisfied;
  - `closable` = `active` with ≥ 1 child, no `active` or `waiting` child, and ≥ 1 `done` child.
- **One evaluator,** `services::goals::evaluate`, computes them on read for the Goals API, the
  overview and, through the list, the navigator badge. They therefore always agree and can never
  be stale. Clients refetch on `canon.changed` (every canon or goal write and revert) and on
  `chapter.updated` (approval moves `chapter_at_least`).
- A ready node is not activated automatically: the author activates it.

### 11.4 Lifecycle rules

- **`waiting`** needs ≥ 1 condition. Conditions are kept when a node leaves `waiting`, but only a
  waiting node's conditions are evaluated.
- **Closing a node with open descendants** (setting `done` or `dropped` while a descendant is
  `active` or `waiting`) is `409 goal.open_children`. `details.children` names each open
  descendant `{id, level, title, status}`. The request succeeds when repeated with
  `confirm_open_children: true`. The descendants keep their status but stop reaching agents while
  an ancestor is closed (§11.5). The confirmation says so.
- **Delete** a node with children (any status): `409 goal.has_children`. Move or delete the
  children first. The same applies to reverting a node's `create`.
- `dropped` nodes are kept. The Goals page shows them in a collapsed section, and they are never
  injected.

### 11.5 Injection: `block.goals`

For a session on chapter *k* of track *S*, a candidate (§11.2) is **injected** when:
1. it is `active`;
2. `chapter_from` is NULL or ≤ *k*. A range that ended before *k* still counts, marked overdue;
3. no ancestor is `done` or `dropped`.

Each injected node's `active` ancestors are added, so a task brings its plan and its goal. A
`waiting` node is never rendered, not even as an ancestor: a task whose plan waits renders at top
level.

`block.goals` (vars `goals`, `systems`) renders:
- a short preface: these are the author's goals for this chapter; each purpose says why the node
  exists; the outline and the prose serve the tasks' purposes; a purpose is achieved through the
  story and never stated in the prose; the reviewer checks it; the archivist refers to nodes by id;
- the tree as nested `<goal>`, `<plan>` and `<task>` elements (spec/09 §5.8: author text is
  tagged), each with `id`, `title`, `chapters` and an overdue flag when it has a range, and a
  `<purpose>` child;
- one line per link kind, only for enabled systems.

Goals come first, then plans and tasks under their parents; siblings sort by `chapter_from` (NULL
first), then id, and top-level plans and tasks follow the goals. At most 30 nodes are rendered:
tasks are kept by earliest `chapter_from`, together with their ancestors, and the block states how
many relevant nodes were left out. When nothing qualifies, the block is blank and dropped.

Kickoffs that carry it (spec/03 §10): outline, write, revise, review, maintenance and rollover.
Re-sync does not.

### 11.6 The archivist

- **Gate:** a `chapter_maintenance` session, Goals on in its systems, and a non-empty §11.5 set
  for its chapter. Only under the gate are `propose_goal_change` frozen, `block.goals` rendered
  and maintenance step 9 shown.
- Goals are never required by `finish_maintenance`.
- `propose_goal_change` (spec/05 §6) only proposes. The author decides on the Proposals page or
  inline on the Goals page (§5).
- A note that a task "looks unreachable" is a `drop` proposal whose rationale is the note: accept
  drops the task, reject keeps it.

### 11.7 Interactions

- **Deleting a linked row** removes the link (cascade). A deleted condition target shows as
  broken.
- **Merging entities** moves goal links and rewrites `entity_status_is` conditions (§9).
- **Deleting an isolated track** deletes its private nodes and their history.
- **Search:** goal nodes are not indexed, `search_canon` does not return them, and no retrieval
  tool reads them. `block.goals` is the agents' only view.
