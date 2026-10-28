# 06 — Canon and Tiered Memory

Canon is the structured state of the story: entities, relations, appearances, timeline events,
memory notes (facts, world rules), foreshadowing, and chapter summaries. Together with the
setting materials it is what writer and reviewer rely on. All canon writes go through services
(`services/canon/*`), which validate, record `change_log`, emit `canon.changed`, and enqueue
re-indexing (spec/07 §4).

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

## 4. Tiered memory

`services/memory.rs` builds a `MemorySnapshot` for a **target position** (track, chapter) from
canon as it is now. It is used by `block.memory` and by the Memory page API.

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

## 5. Setting proposals

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

## 6. Maintenance procedure (archivist, `chapter_maintenance`)

Triggered by approval (when `auto_maintenance` is on) or manually. Kickoff: settings, memory
(for this chapter's position), cast index, the official text with pids, and the
`archivist.kickoff.maintenance` task. Expected steps (the prompt explains them; `finish_maintenance`
validates the required ones):

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
9. **Settings**: `propose_setting_change` only when the chapter contradicts the setting materials
   or establishes something substantial that belongs in them (never in isolated side stories).
10. `finish_maintenance(report)`.

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
follows the maintenance rule (`synced`, or `stale` if the text changed again meanwhile).

## 9. Manual canon editing (UI)

The UI uses the same services with `actor: user`. Additional author-only operations:
- **Merge entities** (A ← B), in one transaction: move B's relations to A, dropping relations
  between A and B (they would become self-relations) and ones that would duplicate an active
  relation of A; move B's appearances (where both appear in a chapter, A keeps the more
  prominent role); move B's event, note, and foreshadowing links (skipping duplicates), its
  character design link, side tracks' `focus_entity_id`, and B's `is_protagonist` flag. Union the
  aliases (B's name becomes an alias), delete B, and record one `update` change on A and one
  `delete` on B (both with the note `#B → #A`). A shared entity cannot be merged into an
  isolated track's private entity (`400 validation.failed`); a private one can be merged into a
  shared one, and its moved private rows keep their `track_id`. Merges are not revertible as a
  whole; the UI warns.
- **Set protagonist**: moves the `is_protagonist` flag.
- **Reorder timeline** by drag and drop: one `update` change per moved event (position).
