# 08 — Workflows

## 1. Chapter lifecycle

```
 planned ── outline accepted (agent outline session or author) ──► planned (outline ready)
    │                                                                   │
    │ manual edit                                                       │ write (agent)
    ▼                                                                   ▼
 drafting ── draft complete ──► in_review ── approve ──► official ── edit text ──► official (canon stale) ── re-sync ──► official (synced)
    ▲                             │    │
    └── send back (redraft) ──────┘    └── send back (revise) ──► revising ── finish_revision ──► in_review
```

An outline session leaves the chapter `planned`; the write session is what moves `planned →
drafting`, exactly as a manual text edit does.

### 1.1 Operations

| Operation | Endpoint | Preconditions | Effects |
|---|---|---|---|
| Create chapter | `POST /api/tracks/:id/chapters {title?, guidance?}` | the book is `active` (otherwise `409 book.invalid_state` with `details.status`). `conception/finalize` is what activates a book and writes the setting materials and canon the writer's kickoff renders, so a chapter created before it would be written against an empty book; an `archived` book is refused by the same rule (D-130) | `planned`, `ord` = last + 1, empty content |
| Write the outline | `POST /api/chapters/:id/outline {guidance?, pause_after_outline?, force?}` | the chapter is not `official`; no open planner or writer session; the previous chapter rule is the write rule, unless `force` | stores guidance; `chapter_outline` session. The chapter status does not change. The session stages its outline; acceptance applies `chapters.outline`, `reference_notes` and an empty-title title in one transaction |
| Write | `POST /api/chapters/:id/write {guidance?, granularity?, auto_pause?, force?}` | status `planned` or `drafting`; `chapters.outline.trim()` is not empty (otherwise `409 chapter.outline_missing`); no open planner or writer session; the previous chapter of the track (and, for the first side-story chapter, the anchor chapter) is `official` with `canon_status = synced`, unless `force` (otherwise `409 chapter.previous_not_ready`; the Write dialog shows the reason and offers **Write anyway**, which sends `force`) | stores guidance; chapter → `drafting`; `chapter_write` session in `drafting`. The kickoff always carries the outline, reference notes and any existing text, and asks the writer to continue from the end and reconcile earlier paragraphs with a changed outline |
| Submit for review (manual) | `POST /api/chapters/:id/submit` | `drafting` or `revising`; non-empty; no `running` planner or writer session (an open, non-running one is abandoned after the UI confirms) | version `draft`/`revision`, status `in_review` |
| Draft complete (agent) | completion handler | — | closes the open tail, version `draft` (note = the `finish_draft` note), status `in_review`, toast event |
| Ask the reviewer | `POST /api/chapters/:id/review` | `in_review` | returns the open reviewer session if its `base_content_version` is the current `content_version`; otherwise abandons that session (if any) and starts `chapter_review` |
| Re-review | `POST /api/review-reports/:id/rereview {comment}` | chapter `in_review`; the report's session `completed` | spec/03 §6 |
| Send back | `POST /api/chapters/:id/send-back {feedback, mode, review_report_id?, issue_indexes?}` | `in_review`; feedback non-empty; `issue_indexes` refer to that report's issues; `redraft` also needs a non-empty outline (`409 chapter.outline_missing`) | abandons an open reviewer session; `chapter_feedback` row; `revise` → `chapter_revise` session, status `revising`; `redraft` → `backup` version, content cleared, `chapter_write` session in `drafting` whose kickoff carries the feedback through `kickoff_extra.feedback_id`, status `drafting` |
| Revision complete | completion handler | — | version `revision` (note = summary), status `in_review` |
| Approve | `POST /api/chapters/:id/approve` | `in_review`; no open planner or writer session | abandons an open reviewer session; version `official`; `official_version_id`; status `official`; `canon_status = pending`; passages indexed; with `auto_maintenance`, start `chapter_maintenance` (when no archivist model is configured, it stays `pending` and the response carries `warnings: ["model.missing_role"]`) |
| Maintain / retry | `POST /api/chapters/:id/maintain` | `official`; `canon_status ∈ {pending, failed}` | continues the chapter's open archivist session if there is one (failed); otherwise starts a new session of the kind that failed last (`chapter_resync` after an abandoned re-sync, so canon is not maintained twice), or `chapter_maintenance` |
| Re-sync | `POST /api/chapters/:id/resync` | `official`; `canon_status = stale`; no open archivist session | a new `official` version of the current text becomes `official_version_id`; `chapter_resync` session; on completion it is also `synced_version_id` (spec/06 §8) |
| Edit text | `PATCH /api/chapters/:id/content` / `PUT …/content/text` | not locked (spec/03 §14: no running planner or writer session) | content + `content_version`; official + synced → `stale`; open planner or writer session not running → reported via edit report. Author edits never open the tail; they close it when the open last paragraph is deleted or a paragraph is inserted after it |
| Reorder | `POST /api/tracks/:id/chapters/reorder {ids}` | only chapters that are not official may move, and no non-official chapter may end up before an official one | renumber |
| Delete | `DELETE /api/chapters/:id` | no open session; not the anchor of a side track (`409 chapter.is_anchor` with `details.track_ids`: change those anchors first) | cascades; later chapters of the track are renumbered; the first and last appearances of the entities that appeared in it are recomputed (spec/06 §1.1); for official chapters the UI warns that canon keeps its records (chapter references become null) |

### 1.2 Review reports and versions
- A report is **current** when its `content_version` equals the chapter's. Older reports are
  shown as "for an earlier version".
- Version kinds and when they are created: `draft` (draft complete or manual submit from
  drafting), `revision` (revision complete or manual submit from revising), `official`
  (approve, resync completion), `manual` (author's "Save version"), `backup` (before redraft,
  before revise, before restore).
- **Restore** `POST /api/chapter-versions/:id/restore`: creates a `backup` of the current content,
  then replaces the content with the version's (allowed when no planner or writer session is
  running; it counts as an author edit, so on an official chapter with synced canon it makes canon
  `stale`).

### 1.3 Archiving and deleting books and tracks
- **Archive** (`PATCH /books/:id {status: "archived"}`, active books only; otherwise
  `409 book.invalid_state`) only hides the book from the default Library view (a filter shows
  archived books); everything else keeps working, and unarchiving sets it back to `active`.
- **Delete book / delete track**: the book's (or track's) open sessions are stopped, their
  runners awaited, and the sessions abandoned before the cascade, so no runner writes to deleted
  rows. The confirmation dialog says so. Side tracks can be deleted at any time; the main track
  cannot be deleted (`400 validation.failed`).

### 1.4 Locking summary
- Author content edits are rejected (423) only while a planner or writer session of the chapter is
  `running` — the two roles form one lock group. The same applies to the chapter's outline and
  reference notes; while the session is held (e.g. on `submit_outline`), the author may edit them
  and the agent receives an edit report (spec/03 §7).
- Setting documents, character designs, and draft research topics are rejected
  (`423 settings.locked`) only while the book's conception session is `running`. The studio
  disables editing then, so the agent never overwrites a concurrent author edit. Edits made
  while it is not running are reported at the next sealing. After finalize they are never
  locked.

## 2. Outlining and writing a chapter (agent)

1. **Outline kickoff** (`planner.kickoff`): chapter label, title, guidance, target length, track
   info (side story premise and anchor), the existing text if any (through `block.chapter_draft`),
   the current accepted outline to replace when there is one, and the planning procedure.
   Retrieval is allowed and encouraged; drafting tools are not in the planner's list.
2. **Outline**: retrieval (as many calls as needed within the soft budget) → `submit_outline`,
   which stages the outline, reference notes and optional title. With `pause_after_outline` the
   call holds (spec/03 §5.2); approve applies the proposal and completes the session (the held
   call's result already says the outline takes effect when accepted, `staged = true`).
   Request changes discards the staged outline,
   yields `planner.notice.outline_rejected` plus the feedback, and the planner resubmits. The
   chapter stays `planned` throughout.
3. **Write kickoff** (`writer.kickoff.chapter`): `block.chapter_draft` carries the accepted
   outline, the reference notes and any existing text; the task states the drafting procedure,
   the start-from-the-end rule and the granularity rules, and its `feedback` branch carries a
   redraft's feedback.
4. **Drafting**: DRAFT_EDIT tools under the granularity rules, with participation (pause,
   feedback, steering, auto-pause, edits while held, granularity switches). Then
   `insert_text(chapter_end = true)` or `finish_draft`. `submit_outline` is not in the writer's
   frozen list, so a call to it is refused with no effect.
5. **Completion**: draft version, `in_review`, toast "Chapter N is ready for your review".

## 3. Review

1. The author reads the chapter (Text tab) and the Review tab.
2. **Approve** → official (§1.1).
3. **Ask the reviewer** → `chapter_review` session. Kickoff (`reviewer.kickoff`): settings, memory
   for this chapter's position (canon before this chapter), the chapter outline, the reference
   notes, the text with pids, and the review procedure. The reviewer looks up what it needs and
   calls `submit_review`. The report appears in the Review tab.
4. With a report the author can:
   - **Send back with this feedback**: the dialog is prefilled with `report.feedback`, and the
     report's issues are listed with checkboxes. The author edits the text, ticks the issues the
     writer must address, and confirms the mode. Ticked issues travel as data
     (`issue_indexes`), are copied into `chapter_feedback.issues`, and reach the writer through
     the kickoff's `feedback.issues` list (rendered by the prompt pack), so nothing is pasted
     into the text in the UI language and nothing arrives twice.
   - **Discuss**: a comment → re-review → new report (same session if the content is unchanged).
   - **Approve** regardless of the verdict.
5. **Send back (revise)** → `chapter_revise`. Kickoff (`writer.kickoff.revise`): settings,
   memory, the chapter draft block, the chapter's guidance and target length, the author's
   feedback (and the ticked issues from `chapter_feedback.issues`), and the revision procedure:
   revision planning with retrieval → `submit_revision_plan` → revising with edit tools →
   `finish_revision(summary)`.
6. **Send back (redraft)** → a `chapter_write` session in `drafting` whose `writer.kickoff.chapter`
   carries the feedback (found through `kickoff_extra.feedback_id`) and drafts again from the
   current accepted outline; the previous text is archived. The ticked issues are listed by their
   quotes, because their pids belong to the archived text and new paragraphs never reuse them.
   To change the plan itself, the author uses **Redo the outline** on the in-review chapter first
   (any non-official chapter without an open session can be re-outlined).

Delegation depth is one: the reviewer never starts other agents, and its reports never go to
another agent for evaluation. The author judges them (PROJECT §10.9).

## 4. Maintenance and settings evolution
See spec/06 §6–§8. After maintenance, the overview's attention list shows new pending proposals.
Proposals never block writing.

## 5. Conception

| Step | What happens |
|---|---|
| Create | `POST /api/books {title, idea, genre, writing_language, start_conception: true}` → book `conceiving`, main track (empty title; the UI shows its localized name, "Main story" / "正文"), three empty setting docs, `conception` session started (with no conception model configured, the book is still created, the response carries `warnings: ["model.missing_role"]`, and `conception/start` starts the session later) |
| Kickoff | `conception.kickoff`: title, idea, genre, writing language, the deliverables, and the process |
| Rounds | The agent asks (`ask_user`), researches (`web_search`/`web_fetch`), writes documents and characters, proposes research topics, and calls `request_review`. The author answers, reviews, messages, or edits documents directly (edits reported via `notice.setting_edits`) |
| Text-only turn | The session becomes `idle`; the author's next message continues it |
| Finalize | `POST /api/books/:id/conception/finalize` |

**Finalize** (one transaction plus a session start):
1. Preconditions: the book is `conceiving` (otherwise `409 book.invalid_state`); `outline`,
   `world`, `protagonist` non-empty, and at least one character design. Otherwise
   `409 conception.incomplete` listing what is missing. Unapproved items are allowed (the UI
   confirms).
2. The open conception session → `completed` (a running one is stopped first and its runner
   awaited; a pending `ask_user` or `request_review` form is simply left unanswered).
3. Book → `active`; `ConceptionState.finalized_at`.
4. Research tasks in `draft` → `ready`. For each distinct `scope_name`, find or create the scope
   and attach it to the book. A new scope's description is the topics of its tasks joined with
   ` / ` (model-written text and a language-neutral separator, so no wording is generated in
   code); the author can edit it.
5. Start `canon_seed` if an archivist model is configured (otherwise return
   `warnings: ["model.missing_role"]`; the overview offers **Seed canon** later) and at least one
   of Entities, Timeline, Memory and Foreshadowing is on in the book's `canon_systems`. With all
   four off there is nothing to seed: no session and no warning (spec/06 §7).
6. Response: `{book, seed_session_id?, warnings}`. The UI navigates to the book overview.

Books created with `start_conception: false` start `active`, with empty setting materials that the
author writes by hand.

**Start conception** (`POST /books/:id/conception/start`, used when a conceiving book has no open
conception session: it was created while no conception model was configured, or its session was
abandoned): the book is `conceiving` (otherwise `409 book.invalid_state`); it returns the open
conception session if there is one, otherwise it starts one.

**Seed canon** (`POST /books/:id/canon/seed`, the overview's `seed_canon` action): the book is
`active` (otherwise `409 book.invalid_state`); it returns the open `canon_seed` session if there
is one, otherwise it starts one.

## 6. Side stories

- **Create**: `POST /api/books/:id/tracks {title, synopsis, canon_mode, anchor_kind,
  anchor_chapter_id?, focus_entity_id?}`. Defaults: `isolated`, `after_chapter` = the latest
  official main chapter (`before_start` when none exists). An `after_chapter` anchor must be a
  main-track chapter, and a focus entity must be a visible character (`400 validation.failed`
  otherwise). The focus character is the side story's protagonist for short-term memory
  (spec/06 §4.2); without one, the book's protagonist is used.
- `canon_mode` cannot be changed once the track has an official chapter (`409
  track.mode_locked`). The anchor can be changed at any time; it affects only new sessions.
- **Writing**: identical to the main track, including the create-chapter precondition: a side
  story of a book that is not `active` gets no chapter either (`409 book.invalid_state`, §1.1).
  The first chapter's "previous chapter" rule uses the anchor chapter (`after_chapter`) or nothing
  (`before_start`, `after_end`).
- **Context**: memory per spec/06 §4.1. The task block includes `block.anchor`
  (prompts/blocks.toml): the premise, the canon mode (an isolated story reads canon but does not
  change it), and the anchor notice (events of main chapters after the anchor have not happened
  yet for the side story's characters, and results marked （锚点之后） must not be known to them).
- **Maintenance**: visibility rules (spec/06 §2). Isolated tracks write private rows only.

## 7. Book overview: next action and attention list

`GET /api/books/:id/overview` returns:
- `next_action` (first match):
  1. `conceiving` → `continue_conception`;
  2. Entities is on in the book's `canon_systems`, no `canon_seed` session has completed (never
     run, failed, or abandoned), and the book has character designs but no entities →
     `seed_canon`. Without the Entities condition, a book with Entities off would be offered
     seeding forever (D-150);
  3. at least one research task of the book is `ready` → `run_research {ready_research,
     research_task_id?}`. This is the knowledge-accumulation step of PROJECT §8.4 F2 step 6: after
     finalize the overview suggests running the research tasks the conception created, because the
     writer's kickoff and canon maintenance read knowledge (spec/07 §5) and an empty knowledge base
     makes them improvise from canon alone. Only `ready` counts — a `draft` topic is unconfirmed
     and cannot be run, and a `deferred` task was explicitly postponed by the author, which is the
     release valve that lets the book be written before its research is done (spec/07 §8, D-138).
     `ready_research` is the number of runnable tasks (the card says "Run 8 research tasks");
     `research_task_id` is set only when exactly one is runnable, so the card can open that task's
     page instead of the list (D-136);
  4. a chapter `in_review` → `review_chapter {chapter_id}`;
  5. work in progress: the first chapter in `ordered_chapters` that has an open outline session
     or is `drafting`/`revising`:
     - with an open outline session → `write_outline {chapter, session_id}`;
     - otherwise → `continue_chapter {chapter_id, session_id?}` (the open write or revise
       session);
  6. latest main chapter official with `canon_status ∈ {pending, failed, stale}` →
     `sync_canon {chapter_id}`;
  7. the first `planned` chapter in `ordered_chapters`:
     - an empty outline → `write_outline {chapter}`;
     - otherwise → `write_chapter {chapter}`;
  8. otherwise → `write_next {track_id}`.

  Rule 3 sits before every chapter rule, matching PROJECT's sentence order ("Run research tasks"
  before "Write chapter 1"). Consequence: while `ready` tasks remain, the overview keeps suggesting
  research and does not suggest the next chapter — a chapter in review or a drafting chapter waits
  behind it, and the `attention[]` rows for both keep showing that work (D-136). The way out is
  `defer` (spec/07 §8): once every runnable task is deferred, rule 3 no longer matches and the
  overview falls through to the chapter rules, so the author chooses "later" without paying for a
  run or discarding the task (D-138).
- `attention[]`: open sessions in `held`, `awaiting_input`, `paused`, `failed`, `interrupted`,
  `stopped`; chapters in review; canon not synced; pending proposals (count); ready research tasks
  (count, `ready` only — the same set rule 3 counts, so a deferred task produces no row: it does
  not demand action, the author already decided to postpone it); **ready goals** (count of goal
  nodes of the book, any scope, that are `ready` or `closable` per spec/06 §11.3; only while Goals
  is on; links to the Goals page's Ready filter).
- **`next_action.ready_goals`**: the same count, set on `write_outline`, `write_chapter` and
  `write_next` when it is > 0 and Goals is on. The card shows it as a secondary line with the same
  link. Ready goals are deliberately **not** a next-action rule. A node stays ready until the
  author acts on it, so a first-match rule would hide the next chapter behind it: the dead end
  research tasks needed `defer` to escape (D-138, D-150).
- `progress`: chapter counts per status and track, official word count, total word count.
- `recent_changes`: the latest 10 change log entries.
- `usage`: the token totals of the book's sessions.
