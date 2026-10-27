# Implementation Plan: Split "Writer — Write Chapter" into an Outline Session and a Write Session

**Input:** `docs/research/session-split-requirements.md` (R1–R13, constraints §5, acceptance §7,
questions Q1–Q8).
**Date:** 2026-10-28. **Status:** plan, not implemented.
**Readers:** the executing agent (follow `AGENTS.md` §5 per task) and the author, who should confirm
§9 before task 12.1 starts.

Every code reference below was checked against the tree at `2d5e230`. Line numbers drift; the
function names are the stable anchors.

---

## 0. Decisions at a glance

| Topic | Decision |
|---|---|
| **R1, Q1** kinds and roles | A new session kind **`chapter_outline`**, run by a new sixth role **`planner`** with its own system prompt, fixed tool list and model assignment. `chapter_write` keeps its name and becomes drafting-only. |
| **R2** outline staging | The outline session **stages** its outline. `chapters.outline`, `reference_notes` and an empty `title` change only in the transaction that finalizes an accepted `submit_outline`. New invariant: `chapters.outline` holds only an accepted or author-written outline. |
| **Q3** what ends it | `submit_outline` is the terminal tool, with effect `complete: outline` (new `CompletionKind`). With `pause_after_outline` on, the call holds and the completion applies on approval. Feedback revokes it, and the session keeps working. There is no `finish_outline` tool. |
| **R3** write session | Starts in `drafting` and needs a non-empty outline (otherwise `409 chapter.outline_missing`). Its tools exclude `submit_outline`. Its kickoff carries the outline (`block.chapter_draft`) and the granularity rule. |
| **Redraft** | Send back → Redraft starts a **write** session that drafts from the accepted outline with the author's feedback. Re-planning is a separate outline step, available while the chapter is in review (§2.4). |
| **Q2, R12** chapter status | No new status. An outline session never changes the status. `planned → drafting` happens where it happens today: when a write session is created, or when the author edits the text. |
| **Q4** manual outline | An outline the author writes by hand counts as accepted. There is no way to skip the outline for an agent write. Writing prose by hand never needs an outline. |
| **Q5** re-outline after prose | Allowed on any non-official chapter that has no open planner or writer session. Approval replaces only the outline, the reference notes and an empty title. The text, the versions and every session's history are untouched. |
| **R6** concurrency | At most one open session across the planner and writer roles per chapter, so outline, write and revise sessions exclude each other. Sessions on different chapters never block each other. |
| **R5, Q1** routing | `RoleModels` gains `planner`. The planner resolves `book.planner → global.planner → book.writer → global.writer`. No stored setting is rewritten, and an unset planner behaves exactly as today. |
| **R7, Q6** `pause_after_outline` | Kept: same storage, same default (`true`). Its meaning narrows to "hold the outline session's `submit_outline` for approval; when off, the outline is accepted immediately and the session completes". The label changes, the Write dialog loses the switch, and the new Outline dialog gains it. |
| **R8** next action | New kinds `write_outline` and `write_chapter`. An open outline session counts as work in progress. Planned chapters are offered before `write_next` creates a new chapter. |
| **R10, Q8** open legacy sessions | A `chapter_write` session that is open in `planning` at the upgrade finishes under its frozen prompt through a small legacy branch. Nothing about it is migrated. |
| **R11** prompt pack | New `planner.*` keys. `writer.kickoff.chapter` becomes the drafting kickoff (its title 「写手 · 撰写章节」 is finally accurate). `writer.kickoff.redraft` is retired. Three writer notices stay, marked legacy. Overrides of `writer.kickoff.chapter` move to `planner.kickoff`. |
| **Q7** ordering | This change lands first; the writing-skills change rebases on it (§2.14). |
| **Migration** | Yes. `0004_outline_sessions` rebuilds `agent_sessions`, whose CHECK constraints list roles and kinds, and moves one class of prompt overrides. `0001_init.sql` is edited for fresh databases. |

---

## 1. Facts this plan rests on

These were verified in the code; the requirements document does not state them all.

1. **Tool lists are per role**, not per kind (D-007; `agent/tools/registry.rs::tools_for_role`),
   and the frozen list bounds what a session may run (D-072). R2/R3's "must not include" therefore
   needs either a second role or per-kind lists. A second role keeps D-007 intact.
2. **`submit_outline` writes the chapter in `execute`** (`agent/tools/writer.rs`,
   `SubmitOutlineTool`): outline, reference notes and title. Only `phase_to` is deferred. Today a
   held or rejected outline is therefore already stored in the chapter.
3. **`agent_sessions.role` and `.kind` are CHECK-constrained** (`0001_init.sql`), and SQLite cannot
   alter a CHECK in place. Ten foreign keys reference the table:
   - `ON DELETE CASCADE`: `agent_messages`, `agent_tool_calls`, `agent_inbox`;
   - `ON DELETE SET NULL`: `chapter_versions`, `setting_proposals`, `change_log`,
     `research_tasks`, `review_reports`, `chapter_feedback`, and the table's own
     `parent_session_id`.

   The table also references `agent_tool_calls` through `held_tool_call_id`.
4. **Overrides of a key the pack no longer has are inert.** `prompts/mod.rs::build_environment`
   iterates the pack's keys, so such an override is neither rendered nor listed.
5. **Notices and results render from the current pack**; only the system prompt, the kickoff and
   the tool definitions are frozen. A legacy session still needs every notice key it uses.
6. **Redraft creates a planning session today**: `services/reviews.rs` send-back plus
   `writer.kickoff.redraft`.
7. **Rollover copies the parent's frozen prompt and tools** (`agent/context/mod.rs::rollover`), so
   the child of a legacy session stays legacy.
8. **The rollover's drafting branch tests `{% if feedback %}`**, which is always true:
   `feedback_context` always returns a `{text, issues}` struct. This is the defect writing-skills
   §9.2 describes.
9. **The chapter header offers Write only for `planned`** (`ChapterHeader.tsx`). A `drafting`
   chapter whose session was abandoned has no header action that brings the agent back.
10. **`write_next` always creates a new chapter** (`NextActionCard.tsx`, `WriteNextButton`): the
    engine (`services/books.rs`, rules 4–7) never looks at `planned` chapters.
11. **Pre-existing discrepancy, not fixed here:** spec/04 §3 resolves `cache_ttl: auto` to `1h`
    for writer sessions that hold, while `services/sessions.rs::model_snapshot` resolves it to `5m`
    for every session. This plan only rewords the spec sentence (§5); the author decides whether
    the code or the spec is right.

---

## 2. Design

### 2.1 Roles, kinds, phases (R1)

| Kind | Role | Phases | Ends with | Frozen tools |
|---|---|---|---|---|
| `chapter_outline` (new) | `planner` (new) | `none` (phase-free, like `chapter_review`) | `submit_outline` → `complete: outline` | RETRIEVAL (10), `read_draft`, `submit_outline` |
| `chapter_write` | `writer` | `drafting` | `insert_text(chapter_end=true)` or `finish_draft` | the writer list minus `submit_outline` (19) |
| `chapter_write`, legacy (open at the upgrade) | `writer` | `planning` → `drafting` | as before | its own frozen 20-tool list |
| `chapter_revise` | `writer` | unchanged | unchanged | the writer list (19) |

- **Why a role, not per-kind lists.** The role is already the unit of system prompt, tool list and
  model routing. All three are what R1, R2/R3 and R5 ask to separate, and "as distinct as
  `chapter_review` is from `chapter_write`" is literally a different role. Per-kind lists would
  break D-007 and still need a second routing mechanism for R5.
- **The writer list keeps RETRIEVAL**, because `chapter_revise` still plans revisions with it.
  Drafting refuses it by phase, as today (PROJECT §10.3).
- **Phase `none`**: the planner's frozen list *is* its gate, so `is_permitted` treats
  `chapter_outline` like the other phase-free kinds.
- **Parallel calls**: the rule `parallel_tool_calls: role ≠ writer` (spec/03 §11) lets the planner
  issue several retrieval calls per round. This is a deliberate change from today's one-call
  planning, which has no granularity to protect. A `submit_outline` in a batch completes the
  session, and later siblings end as `error.after_completion` (spec/03 §5.4).
- **Labels**: `planner` / "Planner" / 「策划」. The session title is "Outlining · Chapter N" /
  「细纲 · 第N章」 and the icon is Lucide `ListTree`. These are labels only (§9).

### 2.2 The outline session (R2, Q3)

**Start**: `POST /api/chapters/:id/outline {guidance?, pause_after_outline?, force?}` →
`{session_id, warnings}`.

| Precondition | Failure |
|---|---|
| the chapter is not `official` | `409 chapter.invalid_state` |
| no open planner or writer session on the chapter | `409 chapter.session_exists` (`details.session_id`) |
| the previous chapter is ready, unless `force` (the same rule as Write) | `409 chapter.previous_not_ready` |
| a model resolves for `planner` (§2.6) | `409 model.missing_role` (`details.role = "planner"`) |

Effects: stores the guidance, as Write does today, and creates the session. The chapter status does
not change.

**Kickoff**: `block.settings`⁎ · `block.memory` · `planner.kickoff`.
- `planner.kickoff` includes `block.chapter_draft` when the chapter has text, as the old
  `writer.kickoff.chapter` did.
- When the chapter already has an accepted outline, the kickoff shows it as the outline to replace.
- `block.memory` never renders the chapter outline for this kind (spec/06 §4.2).

**`submit_outline` in an outline session:**
- **`execute`** validates the input and stages `pending_outline {outline, reference_notes,
  title?}`. Its effects are `{complete: outline, pending_outline}`, and nothing is written to the
  chapter. The result renders with `staged = true`: "submitted; takes effect when the author
  accepts it". It must not say the outline was saved.
- **Hold**: `hold_condition` is unchanged: `pause_requested`, or `submit_outline` with
  `pause_after_outline`.
- **Finalize**, on the non-held path or on approval:
  - `apply_pending_outline` writes the outline and the reference notes, plus the title if the
    chapter's title is still empty at apply time;
  - it emits `chapter.updated {outline_changed: true}`;
  - the `outline` completion handler runs (no further domain effect);
  - the session becomes `completed`.

  All of this happens in one transaction. No further model request is made.
- **Feedback**: the effects are revoked and the staged outline is discarded. The call gets these
  appendices, in canonical order:
  1. `planner.notice.outline_rejected`;
  2. an edit report, if the author edited the chapter meanwhile;
  3. the feedback.

  `notice.completion_revoked` is not added when the tool has its own rejection notice (today no
  tool has both). The session keeps working and the planner resubmits.
- **Abandon** discards the staged outline along with the call. The chapter keeps whatever outline
  it had, which is what makes "redo the outline" safe.
- **Locks and edit reports**: the chapter lock (423) also applies while an outline session is
  `running`. `transition` captures `ViewSnapshot::Chapter` for this kind, so edits made while the
  session is held, paused or stopped reach the planner as `notice.user_edits`. Author edits are
  allowed whenever the session is not running (the uniform rule of spec/08 §1.4). Approving still
  replaces the outline with exactly the proposal the author saw.

### 2.3 The write session (R3)

`POST /api/chapters/:id/write {guidance?, granularity?, auto_pause?, force?}`.
`pause_after_outline` is removed from `ChapterWriteInput`; the server and the SPA ship together.

| Precondition | Failure |
|---|---|
| status `planned` or `drafting` (unchanged) | `409 chapter.invalid_state` |
| `outline.trim()` is not empty | `409 chapter.outline_missing` (new code, spec/10 §4) |
| no open planner or writer session | `409 chapter.session_exists` |
| the previous chapter is ready, unless `force` (unchanged) | `409 chapter.previous_not_ready` |

- **Creation**: as today, the chapter becomes `drafting`. `initial_phase(chapter_write)` becomes
  `drafting`. The session stores `pause_after_outline = 0`, because the flag does nothing for this
  kind.
- **Kickoff**: `block.settings`⁎ · `block.memory` · `block.chapter_draft` ·
  `writer.kickoff.chapter`.
  - `block.chapter_draft` carries the outline, the reference notes and the text with pids.
  - `writer.kickoff.chapter` carries the drafting procedure and includes
    `writer.rules.granularity`: the content `writer.notice.outline_accepted` used to deliver at the
    phase change.
  - When the chapter already has text, the kickoff says to continue from its end, and to adjust
    earlier paragraphs with the edit tools where they contradict a changed outline.
- **`submit_outline` is not frozen**, so a call to it gets `error.tool_not_allowed` and has no
  effect.

### 2.4 Redraft

- **Send back with `mode: redraft`** keeps today's side effects: a `backup` version, the text
  cleared with `next_pid` kept, status `drafting`, and a `chapter_feedback` row. It now starts a
  **`chapter_write` session in `drafting`** whose kickoff is `writer.kickoff.chapter` with its
  feedback branch: the previous text is archived, the feedback overrides the outline, and the
  issues are listed by quote.
- **How the kickoff finds the feedback**: the send-back passes the row through
  `kickoff_extra.feedback_id`. The row is linked to the session (`chapter_feedback.session_id`)
  only after `create_inner` returns, so the kickoff must not depend on that link.
- **New precondition**: a non-empty outline. Without one the send-back answers
  `409 chapter.outline_missing`, and the dialog offers "Revise in place" or writing an outline.
  Only legacy or hand-made chapters can reach this case.
- **Feedback in rollovers**: the rollover kickoff's `feedback` becomes the row whose `session_id`
  is in the session's chain, or `null` when there is none. This fixes Fact 8: a plain drafting
  rollover no longer prints a dangling 「本章是按作者的意见重写的」.
- **Re-planning before a redraft**: "Redo the outline" on the in-review chapter, approve it, then
  Send back → Redraft.

**Why not "redraft = re-plan, then write" (today's semantics in two sessions)?** It would force a
planning round on every prose-level redraft. It would also leave the write session with no link to
the feedback row, because that row would point at the outline session. The split exists so that each
step can be redone on its own, and redraft becomes a prose operation. This changes behaviour, so it
is listed in §9.

### 2.5 Concurrency and locks (R6)

**One open session across the `planner` and `writer` roles per chapter.**
`services/sessions.rs::chapter_session_role` becomes a lock group, re-checked inside the creation
transaction (D-121). This yields every rule R6 asks for:
- at most one open outline session;
- at most one open write or revise session;
- an outline session and a write session exclude each other on the same chapter.

**R6 is self-contradictory as written.** Its first clause, "an open outline session must not block
starting a write session for a chapter that already has an outline", conflicts with its second for
the same chapter. This plan reads the first clause as cross-chapter: outlining chapter 13 never
blocks writing chapter 12. Confirm in §9.

The same group replaces "writer session" everywhere a chapter rule names one:

| Rule | After the change |
|---|---|
| Edit lock (`services::running_writer_session`) | `423 chapter.locked` while a planner **or** writer session of the chapter is `running` |
| Approve | needs no open planner or writer session (`409 chapter.session_exists`) |
| Manual submit | a running one → 409; an open, non-running one is abandoned after the UI confirms (the confirmation text names the session) |
| Restore a version | needs no running planner or writer session |
| `ChapterDto.open_sessions` (`CHAPTER_ROLES`) | gains `planner` |

### 2.6 Model routing (R5)

- `AgentRole::Planner`. `RoleModels` gains `planner: Option<i32>` (`#[serde(default)]`), so old
  `roles.defaults` and `books.model_overrides` rows load unchanged.
- **Resolution** for the planner: `book.model_overrides.planner` → `roles.defaults.planner` →
  `book.model_overrides.writer` → `roles.defaults.writer`; nothing → `409 model.missing_role`
  (`role: planner`). Other roles are unchanged.
  - The order respects an explicit planner choice at either level.
  - An unset planner reproduces today's behaviour exactly, at upgrade and for every later book.
  - No data migration is needed.
- **Settings › Agent roles**: six rows; the planner sits between Researcher and Writer. The
  planner's unset option reads "Same as Writer", and "Clear all roles" clears it too.
- **Book settings › Model overrides**: six rows. The planner's hint names its effective model:
  - "Default: {model}" when a global planner is set;
  - otherwise "Follows the Writer: {model}", where the model is the book's writer override or the
    global writer;
  - "none" when neither exists.
- `roleModels.ts` (`AGENT_ROLES`, `roleSelection`, `ROLE_LABEL_KEYS`) is the single place the two
  screens share, so both change together.

### 2.7 `pause_after_outline` (R7, Q6)

- **Kept**: same `BookSettings` field, same column, same default (`true`), same `hold_condition`.
  Its meaning is now "the outline session holds `submit_outline` for approval; when off, the
  outline is accepted immediately and the session completes".
- **Stored values need no handling.** Book settings, `book_defaults` and existing session rows
  keep their values, and those values now mean the narrower thing. Legacy planning sessions read
  the flag exactly as before.
- **Session PATCH**: outline sessions accept `pause_after_outline` only. Write and revise sessions
  accept the three fields as today. Any other combination is `409 session.invalid_state`.
- **UI**: the Book settings label becomes "Approve each chapter outline before it takes effect",
  with a hint: "When off, the planner's outline is accepted as soon as it is submitted". The Write
  dialog drops the switch; the Outline dialog has it.
- **Not built**: auto-starting the write session after approval. It would chain sessions, and the
  author asked for two visible steps (PROJECT §9.2 spirit).

### 2.8 Chapter status and the chapter list (R12, Q2)

- **No new status.**
  - The outline session leaves the status alone: a planned chapter stays `planned` through
    outlining and acceptance.
  - `planned → drafting` happens only on write-session creation or an author text edit, as today.
  - The redraft path is unchanged: `in_review → drafting`.
- **The outline's state is derived, not stored.**
  - Missing: `outline` is empty.
  - In progress: an open outline session exists.
  - Awaiting approval: that session is `held` on `submit_outline`.
  - Accepted: the outline is non-empty and no outline session is open.

  This works only because of the staging invariant in §2.2.
- **Chapter list**: the status shows as today. `ChapterListItemDto` gains `has_outline: bool`, and
  the navigator draws a planned row that has an outline with a distinct icon and the tooltip
  "Planned · outline ready". `chapter.updated` already carries the list item, so the icon stays
  live. This is the only list change, and it can be dropped without touching anything else (§9).

### 2.9 Next-action engine (R8)

Rules are first match; new and changed ones are in bold.

1. `conceiving` → `continue_conception`. *(unchanged)*
2. Seed → `seed_canon`. *(unchanged)*
3. Ready research → `run_research`. *(unchanged)*
4. A chapter `in_review` → `review_chapter`. *(unchanged)*
5. **Work in progress**: the first chapter in `ordered_chapters` that has an open outline session or
   is `drafting`/`revising`:
   - with an open outline session → **`write_outline {chapter, session_id}`**;
   - otherwise → `continue_chapter {chapter, session_id?}`, as today; `session_id` is the open
     write or revise session.
6. Unsynced latest official main chapter → `sync_canon`. *(unchanged)*
7. **The first `planned` chapter** in `ordered_chapters`:
   - an empty outline → **`write_outline {chapter}`**;
   - otherwise → **`write_chapter {chapter}`**.
8. `write_next {track_id}`. *(unchanged)*

- **"No outline yet" versus "outline exists"** is `chapters.outline.trim()` empty or not. This is
  exact because nothing writes an unapproved outline there (§2.2). Legacy sessions are the one
  exception, and they are `drafting`, so rule 5 catches them first.
- **Rule 7 is placed after `sync_canon`** because writing chapter N+1 before chapter N's canon is
  synced trips `previous_not_ready` anyway. An open outline session for a later chapter still shows
  first through rule 5 and in `attention[]`.
- **Card behaviour**:
  - `write_outline` and `write_chapter` are links to the chapter page (`?tab=outline` and
    `?tab=text`), whose header holds the primary action.
  - Labels: "Outline Chapter N" / "Write Chapter N".
  - The `switch` in `NextActionCard.tsx` must handle both new kinds explicitly: its `default`
    renders nothing, so an unhandled kind would show a label with no button.

### 2.10 Prompt pack (R11)

| Key | Change | `title_en` / `title_zh` | Notes |
|---|---|---|---|
| `planner.system` | new | Planner — system prompt / 策划 · 系统提示词 | Static. Covers the role, retrieval discipline, the outline and reference-notes contract (the reference notes must carry everything drafting will need, because drafting cannot retrieve), author feedback and edits, and boundaries. |
| `planner.kickoff` | new | Planner — write the chapter outline / 策划 · 撰写细纲 | Vars `book, track, chapter, draft`, a superset of the old `writer.kickoff.chapter`'s, so moved overrides render. Branches: first outline, replacing an accepted outline, chapter already has text. |
| `planner.notice.outline_rejected` | new | Planner — outline needs changes / 策划 · 细纲未通过 | Slot-1 revoke notice. |
| `planner.notice.nudge` | new | Planner — nudge / 策划 · 提醒 | `nudge_key(chapter_outline, _)`. |
| `planner.notice.retrieval_budget` | new | Planner — retrieval budget reached / 策划 · 检索次数提醒 | Var `used`. The runtime picks the budget key by role; which sessions get a budget notice is unchanged. |
| `writer.system` | changed | unchanged | Remove the planning phase and its `submit_outline` contract. The phases left are drafting, revision planning and revising. `# 正文写作要求` stays, because writing-skills owns that section. |
| `writer.kickoff.chapter` | **new job** | Writer — write the chapter / 写手 · 撰写章节 | Drafting kickoff. Vars `book, track, chapter, granularity, feedback`. Includes `writer.rules.granularity`; branches for existing text and for redraft feedback (`{% if feedback %}`, now null-safe). |
| `writer.kickoff.redraft` | retired | — | Redraft is the feedback branch of `writer.kickoff.chapter`. Its overrides become inert (Fact 4). Legacy redraft sessions froze their kickoff and do not need the key. |
| `writer.kickoff.rollover` | context only | unchanged | `feedback` follows the chain rule (§2.4) and is `null` when none applies, as `variables.toml` already documents. |
| `writer.notice.outline_accepted`, `writer.notice.outline_rejected`, `writer.notice.nudge_planning` | legacy | append "(sessions started before the outline split)" / 「（拆分前的会话）」 | Only legacy sessions render them (§2.12). |
| `tool.submit_outline` | changed | — | Stops saying drafting begins after submission. Says the outline takes effect when the author accepts it, and that this ends the session. |
| `result.writer_control` | changed | — | New var `staged`. When true it says the outline is submitted and awaiting acceptance, not "saved". |

- **Drafting granularity**: the session's starting rule moves into the write kickoff, as
  `writer.kickoff.rollover` already does. Mid-session switches stay the inbox notice
  `writer.notice.granularity`.
- **Terminology**: entries this change writes or rewrites call the chapter outline 细纲, the
  author's term. Untouched entries keep 章纲. Both name the same artifact (`chapters.outline`,
  `<chapter_outline>`), and a pack-wide rename is out of scope.
- **Single sources (audit D1, D6)**: the outline contract now lives in `planner.system`,
  `planner.kickoff` and `tool.submit_outline`, and the drafting contract in `writer.system` and
  `writer.kickoff.chapter`. Guard tests (§6) pin both sides, including the negative ones: no
  planning in the writer's entries, no prose tools in the planner's.

**Overrides of changed keys:**

| Key | What happens to its overrides |
|---|---|
| `writer.kickoff.chapter` | `0004` moves them to `planner.kickoff`. Their text is planning instructions, which is the planner's job now, and the vars are compatible. A render failure falls back to the default with `prompt.override_failed` (spec/09 §3). |
| `writer.kickoff.redraft` | Inert: the key is retired. |
| `writer.system` | Kept. They still apply to write and revise sessions, where the stale planning paragraph cannot be acted on (no planning phase, no `submit_outline`). |
| `tool.submit_outline` and the three legacy notices | Kept. |

Task 12.7 runs a pre-ship check that reports every one of these on a copy of the author's database.

### 2.11 UI (R9)

**Entry points and gating.** "Open session" below means an open planner or writer session of the
chapter.

| Where | State | Offers |
|---|---|---|
| `ChapterHeader` | `planned`, no outline, no open session | **Write the outline** (Outline dialog) |
| | `planned`, outline accepted, no open session | **Write the chapter** (Write dialog); secondary **Redo the outline** |
| | `drafting`, no open session | **Write the chapter** if it has an outline, otherwise **Write the outline**; secondary Submit for review (existing) and **Redo the outline** |
| | an open session | no start action; the agent panel owns the session |
| | `in_review`, `revising`, `official` | unchanged |
| Outline tab | missing | an empty state with **Write the outline**; the fields stay editable, so the author can write it by hand |
| | in progress | the fields are read-only; "The planner is working", with a link that focuses the session in the panel |
| | awaiting approval | the proposal (outline, reference notes, proposed title), read from the held call's `input`; the `HoldBar` with **Approve outline** / **Request changes**; the current accepted outline below it, read-only, labelled "replaced when you approve" |
| | accepted | the editable fields as today, plus **Redo the outline** when the chapter is not official and has no open session |
| | legacy | a held `submit_outline` in a `chapter_write` session keeps today's path: the fields are editable and are saved, then resumed |
| `NextActionCard` | `write_outline`, `write_chapter` | links to the chapter page (§2.9) |
| Navigator | "+ Next chapter" | unchanged: creates a planned chapter and opens it, and the header then offers Write the outline |

**Other UI surfaces:**
- **Outline dialog** (new): guidance, the approval switch (defaulting to the book setting) and
  Start. It shares the `previous_not_ready` block and **Write anyway** with the Write dialog;
  extract that block into one component.
- **Write dialog**: guidance, granularity, auto-pause and Start; the description now says it drafts
  from the accepted outline. On `409 chapter.outline_missing` (a race) it shows the reason and a
  **Write the outline** button.
- **One derivation**: `outlineState(chapter, calls)` in `features/chapter/outlineState.ts` returns
  `missing | in_progress | awaiting_approval | accepted`. The header and the Outline tab both use it
  (the `pendingDraftView` pattern of D-141), so they cannot disagree.
- **`useWriterState`** already filters `chapter_write | chapter_revise`, so the dock (D-141) stays
  bound to the write session. A sibling `openOutlineSession(chapter)` serves the Outline tab.
  `HeldCallReview` already reads the outline from the call input, so it works unchanged for the
  outline session.
- **Session lists and header**: `agent:session_title.chapter_outline`, the planner role icon, and
  the session switcher's kind filter. `session.updated` already invalidates the chapter (D-144), so
  agent-driven outline holds reach the tab without a reload.
- **Send-back dialog**: the redraft description changes to "the text is archived and the writer
  drafts again from the current outline, following your feedback; to change the plan, redo the
  outline first".
- **Prompts page**: a Planner group label.
- **i18n**: every string in `en` and `zh-CN` in the same commit. The dynamic keys
  `agent:session_title.*` and `errors:*` are covered by the existing dynamic-key tests
  (spec/11 §9). `settings:role.planner` is enforced by the typed `ROLE_LABEL_KEYS` record in
  `roleModels.ts`.

### 2.12 Backwards compatibility (R10, Q8)

- **Historical sessions** render unchanged, because rendering never consults the runtime. The
  `submit_outline` tool renderer stays, and no stored payload is rewritten.
- **Open legacy planning sessions finish under their frozen prompt.** The branch is keyed on
  `(kind = chapter_write, phase = planning)`, which new code never creates. It keeps:
  - the planning arm of `is_permitted`;
  - in `SubmitOutlineTool`, an immediate write plus `phase_to: drafting` when the session kind is
    `chapter_write`;
  - `phase_notice` → `writer.notice.outline_accepted`;
  - `rejected_notice` chosen by session kind (writer or planner key);
  - `nudge_key(chapter_write, planning)`;
  - the Demo provider's planning arm;
  - the legacy Outline-tab path;
  - the three legacy notice keys.

  After `submit_outline` such a session is an ordinary write session, and its rollover children
  inherit its frozen prompt (Fact 7).
- **Why not migrate those sessions instead?** Abandoning them in SQL would bypass `transition()`
  and leave a held outline in limbo. Converting them would give a session a role its frozen prompt
  does not describe. Keeping them honours R13's prompt freeze at the cost of about 30 marked lines.
- **Removal**: a follow-up task deletes the branch, its keys and its test once this query returns
  0 on the author's database:

  ```sql
  SELECT count(*) FROM agent_sessions
  WHERE kind = 'chapter_write' AND phase = 'planning'
    AND status NOT IN ('completed', 'superseded', 'abandoned')
  ```

- **Chapters that already have an outline** are never sent through the outline step again. The
  write precondition only reads `chapters.outline`.

### 2.13 Invariants that must not regress (R13)

| Invariant | How it holds |
|---|---|
| Staged prose writes apply only on approval (D-139) | The write path is unchanged. The outline gains the same discipline through `pending_outline`. |
| Append-only histories | The two sessions have separate rows. Re-running the outline session creates a new session and never touches the write session's messages or tool calls (test in §6). |
| One write lock | Every new write goes through `Db::write`. `apply_pending_outline` runs inside the finalizing transaction, like `apply_pending_content`. |
| Prompt freeze per session | New sessions freeze the new prompts. Legacy sessions keep theirs (§2.12). |
| The reviewer reads the chapter text | `chapter_review` is unchanged. |
| Canon maintenance runs after a chapter is official | Unchanged. |

### 2.14 Sequencing with the writing-skills change (Q7)

**This change lands first**, and writing-skills rebases on it. It restructures what writing-skills
plugs into: roles, kickoff block lists and `writer.system`. Its own additions (blocks, tables, one
section moved out of `writer.system`) are additive. On rebase, the writing-skills design must:

1. Re-cut its §6.1 block lists and §6.2 table per session kind, adding a `chapter_outline` row (a
   suggestion for that design to decide: recent chapters yes, skills no). It must also state that
   `chapter_write` now always carries `block.chapter_draft`.
2. Apply its §5.1 edit to the new `writer.system`, which no longer describes planning. Its migration
   that strips `# 正文写作要求` from `writer.system` overrides is unaffected.
3. Drop its §9.2 rollover fix. This change implements that same chain rule (§2.4), because the
   redraft branch needs it.
4. Take migration number `0005` and the next free `D-` number.

Developing them in parallel would collide on `writer.system`, `writer.kickoff.chapter`, the
rollover context and the migration number.

---

## 3. Data, API and type changes

**Migration `0004_outline_sessions`** (`use_transaction() = Some(false)`, modelled on `0002`):
1. Snapshot the row counts of `agent_sessions` and its three cascade children, and the non-null
   session-reference counts in the seven SET NULL children (Fact 3).
2. With `PRAGMA foreign_keys = OFF`: rebuild `agent_sessions` with
   `role IN (…, 'planner')` and `kind IN (…, 'chapter_outline')` (create, copy, drop, rename,
   recreate its three indexes).
3. Restore the pragma. Fail loudly if any count changed or `PRAGMA foreign_key_check` returns a row.
4. Move overrides:

   ```sql
   UPDATE prompt_overrides SET key = 'planner.kickoff'
   WHERE key = 'writer.kickoff.chapter'
     AND NOT EXISTS (
       SELECT 1 FROM prompt_overrides o
       WHERE o.key = 'planner.kickoff'
         AND o.scope = prompt_overrides.scope
         AND o.book_id IS prompt_overrides.book_id
     )
   ```

The same CHECK values go into `0001_init.sql` for fresh databases. spec/02 §1 permits that edit
while no release is tagged (`git tag` is empty).

**Rust enums and types** (all exported with ts-rs; regenerate the bindings):
- `AgentRole::Planner`, `SessionKind::ChapterOutline`, `CompletionKind::Outline`,
  `NextActionKind::{WriteOutline, WriteChapter}`, `ErrorCode::ChapterOutlineMissing`;
- `ToolEffects.pending_outline: Option<PendingOutline {outline, reference_notes, title: Option<String>}>`;
- `RoleModels.planner`, `ChapterListItemDto.has_outline`;
- `ChapterOutlineInput {guidance?, pause_after_outline?, force?}`;
- `ChapterWriteInput` without `pause_after_outline`.

**Endpoints:**
- new `POST /api/chapters/:id/outline`;
- changed: `/write` preconditions, `/send-back` redraft, `/approve`, `/submit`, version restore,
  `PATCH /sessions/:id` (all as in §2).

There are no new SSE events: `chapter.updated {outline_changed}` and `session.updated` (D-144)
already refresh everything the UI needs.

---

## 4. Tasks — milestone M12

To be appended to `docs/PLAN.md` by task 12.1. One commit per task. Each task ends with
`scripts/check.sh` green.

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 12.1 | **Docs first.** Amend every section listed in §5, append D-146 (or the next free number) recording §0 and the §9 answers, and append this table to PLAN.md. | §5 | `check.sh` green (docs only) |
| 12.2 | **Schema and types.** Migration `0004` plus the `0001` edit; the enums and DTOs of §3; regenerate the bindings. | 02 §1–§5, 10 §2.8, §4 | `outline_split_migration.rs`: a pre-0004 fixture holding every kind, a held call, a parent chain, inbox rows, reports, feedback, proposals, versions, change log and research tasks upgrades with every row and reference intact; a deliberately broken copy fails loudly with no data lost; the override move and the `NOT EXISTS` guard; the new values are accepted. JSON round trips for `ToolEffects` and `RoleModels`. |
| 12.3 | **The outline session, additive.** Planner role and tool list (`registry.rs`); `planner.*` keys plus contexts, samples, `variables.toml` and the validator's role tool lists and samples; kickoff; staging plus `apply_pending_outline` in both finalize paths; `outline` completion; revoke-notice precedence; nudge and budget keys by role; view snapshot and edit lock for the kind; planner model fallback; `POST /chapters/:id/outline`; Demo `chapter_outline` arm. The old write flow is untouched. | 03 §1, §4–§7, §10, §14; 05 §2, §4; 04 §7.2 | `outline_session.rs`: hold → approve applies outline, notes and title, the session is `completed`, no further request, the status is still `planned`; feedback → rejection notice, edit report and feedback in order, no `completion_revoked`, resubmit works; flag off → completes immediately; abandon while held → previous outline intact; the chapter is byte-identical while held; author edits while held are reported. `planner_routing.rs`: the four-level precedence and `missing_role`. Registry permission matrix. `prompts_registry.rs`: snapshot plus planner guard tests. |
| 12.4 | **The write session becomes drafting-only.** Writer list without `submit_outline`; `initial_phase` → drafting; new `writer.system` and `writer.kickoff.chapter` (granularity rule included); `block.chapter_draft` in the write kickoff; `writer.kickoff.redraft` retired; redraft via `kickoff_extra.feedback_id`; rollover feedback chain rule; `outline_missing` on write and redraft; legacy branch kept and marked; legacy notice titles; `tool.submit_outline` and `result.writer_control` (`staged`). | 03 §1, §4, §9–§10, §15; 05 §2, §4; 08 §1.1, §2–§3; 09 §2 | `write_session_split.rs`: a fresh chapter → outline (approve) → write starts in drafting without `submit_outline` frozen, the kickoff carries the outline, notes and granularity rule, and a scripted `submit_outline` call is refused with no effect (AC1); re-outline after prose leaves text, versions and the write session's messages byte-identical (AC2); separate usage (AC3); redraft → drafting session with the feedback in the kickoff; a plain drafting rollover has no redraft sentence; `outline_missing` (empty and whitespace-only). `legacy_planning_session.rs`: a fixture `chapter_write`/`planning` session plans, holds, approves (notice), drafts and completes, and feedback gives the legacy rejection notice (AC5). Rewrite spec/03 §15 tests 1, 2 and 18 for the outline session; update the kickoff snapshots. |
| 12.5 | **Overview, locks, DTOs.** Next-action rules 5 and 7; `has_outline`; `CHAPTER_ROLES` + planner; lock group in creation, approve, submit, restore and the edit lock; PATCH per kind. | 08 §1, §7; 10 §2 | `overview_actions.rs`: one case per stage — no outline, outline in progress, held, approved, drafting, in review, official unsynced, official synced → `write_next`; a planned chapter is offered before `write_next` (AC6). `outline_concurrency.rs`: the R6 matrix and a concurrent-creation race (D-121 style); 423 while an outline session runs. |
| 12.6 | **Web.** Role lists and the follows-writer hint; Outline dialog; Write dialog; header action table; Outline tab states plus the legacy path; `outlineState`; next-action kinds; navigator `has_outline`; session titles and icons; send-back wording; Prompts group; both locales. | 11 §5–§6, §9; PROJECT §8 | Vitest: an `outlineState` table; header actions per state; Outline tab approve and request-changes calls `resume` with and without feedback, and the legacy held path still saves then resumes; the card's targets and labels; the Roles and Book settings pages render six rows and the planner hint; the Write dialog's `outline_missing` state; `i18n:check` (AC4, AC7). |
| 12.7 | **Smoke and pre-ship.** Update smoke items 3, 5 and 8 for the two steps and run them with the Demo provider. Run `0004` on a **copy** of the author's database and report open legacy planning sessions and the overrides of every key in §2.10 with their fate. Record the results in the D-entry's consequences. | PLAN §M11.5 | the checklist passes; the report is in the final message |

---

## 5. Documents to amend (task 12.1)

| Document | Sections |
|---|---|
| `PROJECT.md` | §4 glossary (Phase example, Auto-pause); §5 roles (Planner row, Writer row); §6.4; §6.8; §6.12; F4 split into "write the outline" and "write the chapter"; F5 (redraft); F12; §8.5 (header table, Outline tab, hold bar); §8.6 (role icons); §8.12 (six roles; the Book settings label); §10.7; §11 |
| `spec/02` | §1 (migration 0004); §2 `agent_sessions` CHECKs; §3 `AgentRole`, `SessionKind`, `Phase` (`planning` is legacy only); §4 `BookSettings` comment, `RoleModels`, `ToolEffects`; §5 invariant 5 (lock group) |
| `spec/03` | §1.1 table; §1.2; §4 (resolution fallback, initial fields, side effects); §5.2–§5.4 (staged outline, revoke precedence); §5.6; §5.7; §6 PATCH; §7; §9 (feedback chain rule); §10 table; §11 (planner parallel calls); §12; §14; §15 (tests 1, 2 and 18 rewritten; outline, write-session and legacy tests added) |
| `spec/04` | §3 `cache_ttl` sentence (planner sessions with `pause_after_outline`); §7.2 Demo table |
| `spec/05` | §2 role lists and phase table; §4 the `submit_outline` row per session kind; §1.5 result note |
| `spec/06` | §4.2 (which kickoffs render the chapter outline) |
| `spec/08` | §1 diagram; §1.1 (Outline row; Write, Send back, Approve, Submit and Restore preconditions); §1.4; §2 (outline, then write); §3.6; §7 rules |
| `spec/09` | §1 `agent` values; §2 keys (new, retired, legacy) |
| `spec/10` | §2.2 (`/outline`, `/write`); §2.7 PATCH; §2.8 DTOs; §4 `chapter.outline_missing` |
| `spec/11` | §5.2 (header, Outline dialog, Outline tab, Write dialog); §5.3 (icon, held review ownership); §6 titles |
| `PLAN.md` | append M12 (§4) |
| `DECISIONS.md` | append one entry (context, decision, consequences) for the split, covering §0 and the §9 answers |
| `README.md`, `NOTES.md` | search for "five roles" and the planning→drafting description, and fix any hits |

---

## 6. Acceptance criteria → proof

| AC | Proof |
|---|---|
| 1 Outline, approve, write without re-planning and without `submit_outline` | `write_session_split.rs` (fresh-chapter flow); smoke item 3 |
| 2 Re-run after prose keeps the prose history; plan states what happens | `write_session_split.rs` (byte-identical messages, content and versions); §2.2, §0 Q5 |
| 3 Separate histories and accounting | `write_session_split.rs` (two sessions, two usage totals; the book total is their sum) |
| 4 Different models; both screens; old settings intact | `planner_routing.rs`; the Vitest role pages; `RoleModels` round trip with old JSON |
| 5 Historical sessions render; open planning sessions follow a stated rule | `legacy_planning_session.rs`; the migration test; §2.12 |
| 6 The right next action at each stage | `overview_actions.rs` stage cases |
| 7 Checks, snapshot, bindings, both locales | `scripts/check.sh`; the snapshot diff limited to the keys in §2.10; `i18n:check` |

**Guard tests** (`prompts_registry.rs`, the D-142 style, with the negative invariants the audit
asks for in its §3.2):
- `planner.system` and `planner.kickoff` name `submit_outline` and no DRAFT_EDIT tool.
- `writer.system` and `writer.kickoff.chapter` contain no planning phase and no `submit_outline`.
- `writer.kickoff.chapter` includes the granularity rule.
- `tool.submit_outline` no longer promises drafting.

---

## 7. Risks

| Risk | Mitigation |
|---|---|
| Rebuilding `agent_sessions` is the riskiest migration so far: a `DROP` with foreign keys on would cascade away every message | FKs off around the rebuild, count guards on all ten referencing tables, `foreign_key_check`, a test with a broken copy, and a dry run on a copy of the author's database (12.7) |
| An override written for a key's old job keeps applying | Per-key policy in §2.10; the pre-ship report names every affected override |
| The legacy branch outlives its purpose | Explicit removal query (§2.12); the code is marked; its test is deleted with it |
| The two-step flow adds clicks | Accepted by the requirement (R3, R8). The overview always shows the concrete next step; no auto-chaining |
| A write session cannot re-plan when the outline proves wrong mid-draft | By design: stop it, redo the outline, continue. The kickoff tells the writer to reconcile existing text with a changed outline |
| Planner and writer no longer share a cached tools+system prefix | Each role caches its own. The write and revise sessions of a chapter still share theirs |
| Parallel planner calls emit `submit_outline` before reading results | Normal model behaviour. The author's approval and feedback are the guard, and siblings after a completion never run |

---

## 8. Out of scope

- Review criteria, writing skills, multi-chapter generation and new providers (requirements §6).
- Outline version history. Earlier agent outlines remain readable in their sessions' tool calls.
- A `finish_outline` tool, auto-start of writing after approval, and an "outline pending" status.
- Fixing Fact 11 (the `cache_ttl` discrepancy), or which sessions get a retrieval-budget notice.

---

## 9. Confirm with the author before 12.1

These choices change behaviour the author will notice. The plan proceeds with the first option of
each unless told otherwise.

1. **Role labels**: "Planner" / 「策划」, session title 「细纲」. The alternative is 「细纲写手」,
   which stays closer to the author's 「写手.撰写细纲」.
2. **R6's first clause** is read as cross-chapter (§2.5).
3. **Redraft** redrafts prose from the current outline; re-planning first is a separate step
   (§2.4). Today's redraft re-plans and drafts in one go.
4. **An unset planner follows the writer's model** (§2.6), rather than being a required sixth
   assignment.
5. **Re-outlining is allowed in review** (any non-official chapter), so "re-plan, then redraft" is
   possible.
6. **The navigator outline icon** (`has_outline`) is included (§2.8); it is optional.
7. **The planner may make parallel tool calls** (§2.1).
