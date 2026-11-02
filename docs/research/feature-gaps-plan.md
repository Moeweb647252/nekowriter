# Implementation Plan: Close the Feature Gaps Found by the Cleanup Survey

> Milestone **M18** in `docs/PLAN.md`, decision **D-161**. The spec sections in §5 were amended
> together with this plan. The M16 survey found these gaps (cleanup plan §3.1): each has a working
> server endpoint and an existing client hook, but the UI that should call them was never built.
> **M18 is web-only.** It adds no server change, schema change or prompt change.

## 1. The gaps (verified at `2880a77`)

| # | Feature | Required by | API (exists, tested) | Hook (exists, unused) | What is missing |
|---|---|---|---|---|---|
| G1 | Create an entity by hand | PROJECT F7 ("Create"), §8.8 | `POST /books/:id/entities` (`EntityInput`) | `useCreateEntity` | `EntitiesPage` has no create action |
| G2 | Merge two entities by hand | F7 ("merge by hand (rename and move relations)"), spec/06 §9 | `POST /entities/:id/merge {from_id}` | `useMergeEntity` | no merge UI anywhere |
| G3 | Edit a timeline event | F7 ("add or edit events"); spec/11 §5.5 already speaks of "the timeline … edit dialogs" | `PATCH /timeline-events/:id` (`TimelineEventPatch`) | `usePatchTimelineEvent` | `TimelinePage` creates, moves and deletes events, but cannot edit one |
| G4 | Edit a relation | F7 (the detail pane edits "relations") | `PATCH /relations/:id` (`RelationPatch`: kind, inverse kind, description, status, since/until chapter, note) | `usePatchRelation` | `RelationsEditor` adds and deletes only; `canon:relation_status.active` is unused |
| G5 | An entity's full history | §8.8 ("history (change log entries)") | `GET /entities/:id/history` (latest 100) | `useEntityHistory` | the detail pane shows only `recent_changes`, the latest 20 (`DETAIL_CHANGE_LIMIT`) |
| G6 | Session scrollback | spec/11 §5.3 ("scrolling to the top loads older messages with `next_cursor`") | `GET /sessions/:id/messages?cursor&limit` | `useOlderMessages` | the snapshot holds the last 200 messages; `AgentPanel`'s "Load older" button has **no handler** and sits at the bottom; the studio and the research task page have no scrollback at all |

## 2. Design

Shared rules:
- Every write sends the optional **change note** (spec/06 §3), as the other canon edit forms do.
- Every mutation invalidates through the existing `useCanon` hooks and keys. The query keys come
  only from `api/keys.ts`.
- Errors show through `useErrorToast` with the localized error code.
- A canon page whose system is off keeps the behavior of spec/06 §10.3. The new actions follow
  their page: G1, G2, G4 and G5 belong to Entities, and G3 to Timeline.
- Shortcuts ignore IME composition. Every new string is in `en` and `zh-CN`.

### 2.1 G1 — New entity
- An **+ Entity** button sits in the `EntitiesPage` filter bar and opens `EntityCreateDialog`.
- **Fields**:
  - kind* (`EntityKind` select);
  - name*;
  - aliases (tags input, as in the detail pane);
  - summary;
  - importance (the stars of the detail pane);
  - status (`EntityStatus`, default `active`);
  - **storyline**: "Shared canon", or one of the book's **isolated** side tracks (sets
    `track_id`, spec/06 §2). Canon-mode side tracks are not offered: their rows are shared;
  - change note.

  Description, state and attributes are left to the detail pane: the dialog stays short, and the
  detail pane already edits them with history.
- `is_protagonist` is not in the dialog. **Set protagonist** stays the detail pane's action, which
  moves the flag.
- On success, navigate to `canon/entities/:id` (the new detail) and invalidate the entities list.
- `409`/`422` errors (for example a duplicate name, if the service refuses one) appear inline on
  the field named in `details`, otherwise as a toast.

### 2.2 G2 — Merge
- In the detail pane header's **⋯** menu: **Merge another entity into this one…**, opening
  `EntityMergeDialog`. This entity is A, the survivor.
- **Pick B** with `features/canon/EntityPicker` over the book's entities, excluding A, with each
  row's kind, track badge and appearance count.
- **The dialog states, before confirming**, from spec/06 §9:
  - what moves: relations, appearances, event, note, foreshadowing and goal links, goal
    conditions, the character design link, side tracks' focus, and the protagonist flag;
  - that B's name becomes an alias of A;
  - that B is deleted;
  - that **a merge cannot be reverted as a whole**. The change log records one update on A and
    one delete on B.

  A shared B with an isolated A is refused by the server (`400 validation.failed`). The dialog
  explains that case before the request when both rows are loaded: B shared, A with a `track_id`.
  The server stays the authority.
- **Confirm** is a destructive button that names both entities.
- On success:
  - invalidate the entities list, A's detail, the relations and the timeline;
  - if the route showed B anywhere, replace it with A;
  - a toast says "Merged «B» into «A»".

### 2.3 G3 — Edit a timeline event
- Each `TimelineRow` gets an **Edit** action that opens `TimelineEventDialog`.
- The dialog has the fields of the insert form:
  - title*;
  - description;
  - story time;
  - chapter (the existing chapter picker);
  - linked entities (`EntityPicker`, multi);
  - importance;
  - change note.
- Position is not edited here; drag and drop and **Move** stay the way to reorder. The dialog
  never sends `after_id` / `before_id`.
- Submit sends only the changed fields.
- The dialog contains the **Referenced by** goals section that spec/11 §5.5 already places in "the
  timeline … edit dialogs" (`GoalsReferencedBy eventId`, hidden while Goals is off). The row keeps
  its inline section too.

### 2.4 G4 — Edit a relation
- Each `RelationRow` gets an **Edit** toggle that turns the row into an inline form:
  - kind;
  - inverse kind;
  - description;
  - status (`active` / `ended`, using `canon:relation_status.*`);
  - since and until chapter (the chapter picker, both optional);
  - change note.
- **Perspective**: the row is phrased from this entity's side (spec/06 §1.2). The form edits the
  stored `kind` and `inverse_kind` as they are, labelled from the stored source's point of view
  ("«source» → «target»"), so editing from either end changes the same fields.
- Save sends only the changed fields. Cancel restores the row.

### 2.5 G5 — Full history
- Under the detail pane's history, when `recent_changes.length === 20` (`DETAIL_CHANGE_LIMIT`),
  a **Show full history** button replaces the list with `useEntityHistory`'s result. That is the
  latest 100, and the same `ChangeList` renders it with revert.
- When 100 rows come back, a one-line note says only the latest 100 are shown and links to
  `canon/changes` filtered to the entity. If the Changes page cannot filter by entity id, the link
  goes to the unfiltered page; the note still holds.
- The toggle resets when the selected entity changes.

### 2.6 G6 — Session scrollback
- **Where**: inside `ActivityStream`, so every surface gets it: the agent panel, the conception
  studio and the research task page.
- **Input**: the snapshot's `next_cursor`. Older pages come from the messages endpoint.
  `useOlderMessages` becomes an infinite query: `useInfiniteQuery` with `getNextPageParam: page =>
  page.next_cursor`, keyed under `keys.sessions.detail(id)` as today.
- **Trigger**:
  - a **Load older messages** button at the **top** of the stream, shown while a cursor
    remains;
  - and, when the author scrolls to within one row of the top, the next page loads by itself.
  - Remove the dead bottom button from `AgentPanel`.
- **Merge**: older pages' `{message, tool_calls}` rows are prepended before the snapshot's
  messages, de-duplicated by message id (a live message can never be older than the snapshot, but
  the rule keeps it safe). Live buffers, D-157 queued rows and the stick-to-bottom rule are
  unchanged.
- **Scroll anchoring**: after prepending, the first previously visible row stays where it was
  (adjust `scrollTop` by the added height through the virtualizer). Reduced motion needs no
  special case.
- **States**:
  - while loading, a spinner row at the top;
  - on error, an inline retry row;
  - with no cursor left, a quiet "start of the conversation" marker.
- **Session switch**: switching sessions drops the older pages, because the key includes the
  session id.

## 3. Tasks — milestone M18

| # | Task | Tests / acceptance |
|---|---|---|
| 18.1 | **Docs**: done with this plan (D-161, §5). | `check.sh` green (docs only) |
| 18.2 | **G6 scrollback** in `ActivityStream`; remove `AgentPanel`'s dead button. | Vitest: with a cursor the top button shows, a click calls the endpoint with the cursor and prepends the page in order; duplicate ids are dropped; no cursor shows the start marker; an error shows retry; a session switch clears the pages. Manual: a research session with more than 200 messages scrolls back to its kickoff |
| 18.3 | **G1 create** and **G2 merge**. | Vitest: the create dialog requires kind and name, offers only isolated tracks, sends `track_id` only for one, and navigates to the new entity; the merge dialog excludes A, shows the warning and the shared-into-isolated explanation, sends `{from_id}`, and replaces a route that showed B. Manual: merge two entities that share a chapter; the change log shows the two notes `#B → #A` |
| 18.4 | **G4 relation edit** and **G5 full history**. | Vitest: edit sends only the changed fields and the note; status `ended` shows the ended label; cancel restores the row; the full-history button appears only at 20 rows, loads the history, and resets on entity change |
| 18.5 | **G3 timeline edit**. | Vitest: the dialog pre-fills, sends only the changed fields, never `after_id`/`before_id`, and shows Referenced-by only with Goals on |
| 18.6 | **Wrap-up**: `i18n:check`; remove from the cleanup plan's §3.1 any hook still unused (there should be none); smoke item 13 (§4) with the Demo provider. | the checklist passes |

## 4. Smoke item 13 (append to PLAN §M11.5)
1. Create an entity "Test sword" (item, shared), then a second one "The sword". Merge "The sword"
   into "Test sword": its name becomes an alias, and the Changes page shows the merge notes.
2. Edit a relation's description and end it. The row shows "ended", and the Changes page shows
   the relation change with your note (the entity history lists entity changes only).
3. Edit a timeline event's story time and linked entities. Its position is unchanged.
4. On an entity with more than 20 changes, **Show full history** lists the older ones.
5. Open a long session (for example a research run) and scroll to the top. The older messages
   load, and the view does not jump.

## 5. Documents amended with this plan
- **PROJECT §8.8**:
  - Entities: the **+ Entity** button and dialog; the **⋯ › Merge** action and its warning;
    relation rows edit in place; full history;
  - Timeline: each row's **Edit** dialog.
- **spec/11**:
  - §5.3: scrollback lives in `ActivityStream`, with the button at the top and the auto-load near
    the top;
  - §5.5: `EntityCreateDialog`, `EntityMergeDialog`, `TimelineEventDialog`, the relation inline
    edit, full history;
  - §9: the tests.
- **DECISIONS**: D-161. **PLAN**: the M18 table and smoke item 13.
- The cleanup plan's §3.1 points here, and keeps these six hooks and `canon:relation_status.active`.

## 6. Out of scope
- Server changes: every endpoint exists and is tested; if one misbehaves, fix it in its own
  commit with a test.
- Merging across books, undoing a merge, or bulk operations.
- Editing a timeline event's position in the dialog.
- Paging the entity history beyond the endpoint's 100 rows. The Changes page covers the rest.

## 7. Known follow-ups
- **Per-row appearance count in the merge picker (18.3-F1).** `EntityListItemDto` carries no
  appearance count, so the picker cannot render one from the list query; the dialog shows the
  selected entity's count from its `EntityDto` detail instead. A per-row count needs a
  server-side aggregate on the entity list (a server change outside M18's web-only scope).
