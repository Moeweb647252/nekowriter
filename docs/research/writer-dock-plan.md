# Implementation Plan: The writer dock (one input under the draft)

> Milestone **M25** in `docs/PLAN.md`. Read `AGENTS.md` first. The spec sections listed in §7 are
> amended together with this plan, and the decision is recorded as **D-173**. The spec is
> authoritative; this plan explains the reasons, the order and the tests. M25 comes after M24 and
> depends on neither M23 nor M24.

## 0. What the author asked for, and the answers

The author's request, in their words:

> pending-write-dock放到最下面，改成一个 dock 纯输入框，右下角是一个圆形发送按钮（如果是自动暂停模式且
> 输入框没字则变成批准按钮），如果里面有字，按回车或点发送就是发送意见。
> 如果开启了自动暂停，那么里面没字的时候按回车或者点批准按钮的话就是批准动作
> 如果没开自动暂停，那么输入框获得焦点时触发暂停，失去焦点时回复。提交意见后自动失去焦点。

"失去焦点时回复" is read as 恢复 (resume): it pairs with 暂停 (pause) in the same sentence.

| Question | Binding answer |
|---|---|
| Where is the dock? | **At the very bottom** of the draft pane, below the scrolling text, never over it |
| What is it? | **A plain input** with a **round button** at its bottom-right. No title, no button row |
| Auto-pause on, input empty | The button is **Approve**; Enter or a click approves |
| Text in the input | The button is **Send**; Enter or a click sends the feedback |
| Auto-pause off | **Focus pauses** the writer; **blur resumes** it. Sending feedback blurs the input |
| Auto-pause off: a note sent while the writer's latest write is waiting | **Keep the write; the note guides what follows** ("保留写入，意见作为指导") |
| Auto-pause off, unsent text, and the author clicks elsewhere | **Stay paused** until the text is sent or cleared ("保持暂停，等发送或清空") |
| Auto-pause off, and the author moves from the dock into the draft text to edit it | **Stay paused** ("焦点移到正文时保持暂停") |
| When does a pause made by focusing the dock take effect? | **After the current step** ("等这一步写完再停"). The step's write lands in the text, and the writer stops before its next step, so the text is editable. The panel's **Pause** keeps its meaning: hold the next call |

The last answer came from the review of the first draft. A model request takes most of a writer's
time, and the existing pause holds the call that request produces. A pause made by focusing the
dock therefore almost always stopped on a staged write, whose merged view is read-only (D-141), so
"pause from the dock, then edit the text" could not work.

## 1. Decisions at a glance

| # | Decision |
|---|---|
| W1 | **One dock for the writer session.** `WriterDock` replaces `PendingWriteDock`. It is shown while the chapter's open writer session (`chapter_write` or `chapter_revise`) is in `drafting` or `revising` and is `running`, `held` or `paused`, not only while a write is held: focus has to be able to pause a running writer. It sits below the scroll region of the Text tab and lines up with the reading column. |
| W2 | **A plain input**: an auto-growing textarea (1–6 lines) with one round icon button at its bottom-right. The state is told only through the placeholder, the button's icon and label, and an `aria-describedby` sentence (§3.3). The old title, tool summary, body text, button row and feedback disclosure are gone. |
| W3 | **Keys**: Enter (or ⌘/Ctrl+Enter) submits; Shift+Enter inserts a newline. Both are ignored during IME composition. Escape blurs the input. Feedback is the trimmed text, and a blank text is never sent as feedback (the server would read it as an approval). |
| W4 | **Auto-pause on** (the author reviews each write). With an empty input the button is **Approve**: `resume` for a held write (current: apply; superseded: continue without it) or a paused session. It is disabled while nothing waits. Text is **Send**: `resume {feedback}` on a hold or pause, i.e. request changes as today, which discards a staged write; while running it is a steering `message`. Focus and blur do nothing. |
| W5 | **A step pause.** `pause {after_step: true}` sets a new flag, `pause_after_step`, beside `pause_requested`. A step pause does not hold an ordinary call: the step finishes, its write is applied, and the session pauses before the next model request (the loop's existing `pause_before_request` path). It still holds a call that would **complete** the session, so a note can be given before the chapter goes to review. The panel's **Pause** stays a hold pause, and a hold pause wins over a step pause. |
| W6 | **Auto-pause off** (the author interjects): focus on a running writer with no pause pending sends `pause {after_step: true}`, and the dock remembers that this pause is **its own**. Text sends `resume {feedback}`. It cancels the dock's pending pause, or resumes the paused session, and in both cases the note is queued as an author message for the step's result. **Leaving the draft region** (W10) with an empty input sends `resume`, but only for the dock's own pause (pending, or landed as `paused` without an error). |
| W7 | **Holds are always decided explicitly.** A dock-owned hold (a staged write) is decided with the round button whatever the auto-pause setting: Approve on an empty input, Send with text. Leaving the region never decides it. With auto-pause off, a hold comes only from the panel's Pause or from a completing step under a step pause. For such a hold, Send keeps the write (`resume {feedback, keep_write: true}`), which follows the author's "keep the write" answer. |
| W8 | **After sending** text the input clears and blurs. This blur never resumes (the request already did). After **approving**, focus stays, so a run of auto-pause holds can be approved with Enter, Enter, Enter. |
| W9 | **`resume` gains two cases** (spec/03 §3, §6). (a) On a `running` session with a pause pending, it clears both flags; its feedback, if any, is queued as an inbox `user_message`, as for a paused session. Running with no pause pending stays `409 session.invalid_state`. (b) `keep_write: true` with feedback on a held call keeps a staged content write and applies it as approval would. Every other deferred effect is revoked as with any feedback: a completion gets `notice.completion_revoked`, so the session goes on and the note is read. |
| W10 | **The draft region** is the Text tab's reading column (the paragraphs, `ParagraphEditor`, `FullTextEditor` and their toggles) plus the dock. The dock's own pause lasts while focus stays in the region, so the author can pause from the dock, click a paragraph, edit it, and come back. Leaving means moving to the header, the tabs, the navigator, the agent panel or the page background. Before resuming, the dock waits for the draft's in-flight save, so the edit is in the chapter before the writer runs again and reaches it through the edit report (spec/03 §7). |
| W11 | **Not a blur**: a window or tab switch (`document.hasFocus()` is false), leaving with text in the input, and the blur after a send. |
| W12 | **The runner reads the pause flags fresh.** The hold decision reads them inside the step's transaction, and the `pause_before_request` transition is guarded on a pause still being requested. Otherwise a pause cancelled by `resume` would still hold the step whose session copy was loaded before the cancel. Today `step_tool` decides with the loop-top copy (`runner.rs`, `session_tx`). |
| W13 | **One request at a time**: the dock's control calls (pause, resume, message) are serialized in order. A `409 session.invalid_state` on a focus pause or a leave resume means the state moved on: the session is refetched and nothing is shown. A send that gets `409` because the session meanwhile runs with no pause pending is re-sent once as a steering `message`. Any other send error shows the usual toast and keeps the text. |
| W14 | **One decision surface stays** (D-141). The dock decides only holds that `heldDecisionSurface` gives it, i.e. staged writes. A panel-owned hold (`submit_revision_plan`, a call that staged nothing, `finish_draft` held by a pause) disables the dock with "Decide the held step in the agent panel". |

## 2. Facts this plan rests on (code at `b65b7cb`)

- `PendingWriteDock` (`web/src/features/chapter/PendingWriteDock.tsx`) is a card with a title, the
  tool's summary, a body, **Apply and continue** / **Continue without it** and **Request changes**
  (a `HeldFeedback` disclosure). It is `sticky bottom-0` inside the Text tab panel, below
  `ChapterText` (`ChapterPage.tsx:166-186`), and rendered only when the writer holds a staged write.
  It therefore cannot pause a running writer, and a sticky card covers the last lines of the text.
- Its decisions come from `useHeldDecision` (`features/agent/useHeldDecision.ts:45`):
  `approveHeldInput` = `resume` without `feedback`, and `requestChangesInput` = `resume` with the
  trimmed text, refusing a blank one (`features/agent/heldCall.ts:243-254`).
  `heldDecisionSurface` (`heldCall.ts:99`) gives the dock only writes that staged content.
- **Where a pause lands.**
  - `pause` sets `pause_requested` on a running session (`services/sessions.rs:1271`).
  - `hold_condition` holds **any** call while it is set (`runner.rs:2012`).
  - The loop pauses before a model request when it is set (`runner.rs:159`, `pause_before_request`).
  - A writer spends most of its time in the model request, so a pause usually holds the write that
    request produces. That write is staged (D-139), and its merged view is read-only (D-141).
- **The runner's session copy is stale.** `step_tool` decides the hold with the session copy loaded
  at the loop top (`session_tx`, `runner.rs`), not with a read inside its transaction. A change to
  `pause_requested` committed after that load is seen only at the next step.
- **No way to cancel a pending pause**: `resume` accepts only `held` and `paused` (spec/03 §3), and
  `PATCH` has no pause field (spec/03 §6). `transition` clears `pause_requested` when the session
  leaves `running` (`agent/session.rs:276`). `pause_requested` is a boolean
  (`db/entities/agent_sessions.rs:39`), and the table's shape is 0012's (spec/02 §1).
- **Feedback discards a staged write.** `resume_inner` (`services/sessions.rs:1340-1530`) revokes
  **all** deferred effects when feedback is present (`ToolEffects::default()`, `:1501`). It appends
  `notice.completion_revoked` before `notice.user_feedback` when the call completes
  (`:1455-1487`). `insert_text` with `chapter_end` carries both `complete` and `pending_content`
  (spec/05 §3).
- `ResumeInput` has only `feedback` (`api/dto/sessions.rs:182`).
- `notice.user_feedback` (`prompts/notices.toml:4`) fits a kept write as well as a discarded one,
  and a paused session's feedback arrives as an author message (`【用户留言】`, D-160). No prompt
  changes.
- `SessionDto` carries `status`, `phase`, `auto_pause`, `pause_requested`, `error` and
  `held_tool_call_id` (spec/10 §2.8).
- **Saves.** Paragraph and full-text saves are the `PATCH content` / `PUT content/text` mutations of
  `useContentMutations` (`web/src/api/hooks/chapters.ts:127`), used by `ChapterText`. They have no
  `mutationKey`, and the chapter-document store exposes no save promise. A `ParagraphEditor` saves
  on blur (PROJECT §8.5). While a writer runs, author edits are refused (spec/03 §14).
- PROJECT §8.5 and F4b still describe the pre-D-141 inline hold bar for a held insertion. Since
  D-141, `HoldBar` never renders for a chapter write (spec/11 §5.2), so that text is stale and is
  replaced here.

## 3. Design

### 3.1 Server (spec/02 §1, §2; spec/03 §3, §5.1, §5.2, §5.4, §6, §14; spec/10 §2.7, §2.8)

**Schema** (`0014_step_pause.sql`, a Rust-managed transaction like 0008–0013):

```sql
ALTER TABLE agent_sessions ADD COLUMN pause_after_step INTEGER NOT NULL DEFAULT 0;
```

The guard is that the session count is unchanged and every row has 0. `0001` is not edited. The
entity gains `pause_after_step: bool`, and `SessionDto` gains `pause_after_step: boolean`.
`ToolCallDto` gains `completes: boolean`, true while a deferred call's effects carry `complete`
(the `held_completes` placeholder, §3.3).

**`pause {after_step?: boolean}`** (running only, as today):
- without `after_step`: `pause_requested ← 1`, `pause_after_step ← 0`. A hold pause, which also
  upgrades a pending step pause;
- with `after_step: true`: when no pause is pending, both flags ← 1; when one is pending, nothing
  changes, because a hold pause is never weakened.

**The runner**:
- `hold_condition` replaces its first rule:
  ```text
  p ← fresh pause flags of s (read in this transaction)
  p.pause_requested and not p.pause_after_step                    → true   # hold pause
  p.pause_requested and p.pause_after_step and outcome completes  → true   # completing call
  ```
- The loop's `NeedsModel` branch transitions to `paused` with
  `… WHERE id = ? AND status = 'running' AND pause_requested = 1`. A miss means the pause was
  cancelled, and the loop goes on.
- `transition` out of `running` clears both flags.

**`resume {feedback?, keep_write?}`**:
- **running with a pause pending**: a guarded `pause_requested ← 0, pause_after_step ← 0 WHERE
  status = 'running' AND pause_requested = 1`. The trimmed feedback, if any, is queued as an inbox
  `user_message`, and `session.updated` is emitted. On a miss the session is re-read and the request
  is handled by its new status (`held` or `paused` as below). Running with no pause pending →
  `409 session.invalid_state`.
- **held, with `keep_write` and feedback**:
  - the appendices are composed as for any feedback (completion or rejection notice first, then
    `notice.user_feedback`);
  - the revoke keeps `effects.pending_content` and clears the rest, and `apply_pending_content` runs
    as on approval, so a superseded write is still dropped (D-141);
  - without feedback, or on a call that staged nothing, `keep_write` changes nothing.
- **held / paused otherwise**: as today.

```rust
pub struct ResumeInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback: Option<String>,
    /// D-173: keep a staged content write while giving feedback (the writer dock, auto-pause off).
    #[serde(default)]
    pub keep_write: bool,
}
pub struct PauseInput {
    /// D-173: let the current step finish and pause before the next model request.
    #[serde(default)]
    pub after_step: bool,
}
```

The body of `pause` is optional, so an empty `POST` keeps today's meaning.

### 3.2 The dock's state (`features/chapter/writerDock.ts`, pure)

`writerDockState({ session, held, surface, standing, ownPause, text })` returns
`{ visible, disabled, button: "approve" | "send", buttonEnabled, submit, onFocus, onLeave,
placeholder, description }`. Here:
- `surface` comes from `heldDecisionSurface`;
- `standing` is `current | superseded` from `pendingDraftView`;
- `ownPause` is true while the dock's own step pause is pending, or has landed as `paused` without
  an error.

"Empty" means the trimmed input is empty, and "leave" means leaving the draft region (W10, W11).

| Session state | Auto-pause on, empty | On, text | Auto-pause off, empty | Off, text |
|---|---|---|---|---|
| `running`, no pause pending | Approve, disabled | Send → `message` | Send disabled; **focus** → `pause {after_step}` (`ownPause`) | Send → `resume {feedback}` once the queued pause is in |
| `running`, the dock's step pause pending | Approve, disabled | Send → `message` | Send disabled; **leave** → `resume` (cancels) | Send → `resume {feedback}` (cancels, queues the note) |
| `running`, another pause pending | Approve, disabled | Send → `message` | Send disabled; leave does nothing | Send → `message` |
| `paused`, `ownPause` | Approve → `resume` | Send → `resume {feedback}` | Send disabled; **leave** → `resume` | Send → `resume {feedback}` |
| `paused`, other (panel pause, or an `error`) | Approve → `resume` | Send → `resume {feedback}` | Send disabled; leave does nothing | Send → `resume {feedback}` |
| `held`, dock-owned, current | Approve → `resume` (applies) | Send → `resume {feedback}` (discards) | Approve → `resume` (applies) | Send → `resume {feedback, keep_write: true}` (keeps) |
| `held`, dock-owned, superseded | Approve ("Continue without it") → `resume` | Send → `resume {feedback}` | Approve ("Continue without it") → `resume` | Send → `resume {feedback, keep_write: true}` |
| `held`, panel-owned (W14) | disabled | disabled | disabled | disabled |

- `ownPause` is cleared by any resume, send or leave, and when the session reaches `held`,
  `completed` or any other status. A reload forgets it, so leaving then resumes nothing. The panel's
  **Continue** still works.
- With auto-pause on, a step pause pending (set while auto-pause was off) is left alone, and the
  dock steers with `message`.
- The auto-pause and session values are read at each event.

### 3.3 The component (`features/chapter/WriterDock.tsx`)

- **Placement**:
  - the Text tab panel becomes a column: the scrolling text above, the dock as a non-scrolling
    footer (`shrink-0`, top border, the page background);
  - inside it, `mx-auto max-w-[42em]` lines the input up with the reading column;
  - the text no longer scrolls under a sticky card;
  - `data-testid="writer-dock"` and `data-state` (for example `held-current`).
- **Input**: a rounded textarea with right padding for the button, `aria-label` "Message the
  writer", and `aria-describedby` pointing to a visually hidden sentence that repeats the
  placeholder. The superseded sentence names both versions, as D-141 requires.
- **Round button** (`size-8 rounded-full`, primary):
  - Lucide `Check` for Approve, labeled "Apply and continue", or "Continue without it" when the
    write is superseded;
  - Lucide `ArrowUp` for Send, labeled "Send";
  - a spinner while a dock request is in flight.

  Disabled buttons keep their label and gain `aria-disabled`.
- **Placeholders** (`chapter:dock.placeholder.<state>`, en / zh-CN):

  | State | English |
  |---|---|
  | `on_running` | "Message the writer; it reads it with its next step" |
  | `held` | "Enter applies the highlighted text · type to request changes" |
  | `held_keep` (auto-pause off) | "Enter applies the highlighted text · type a note; the text is kept" |
  | `held_completes` | "Enter finishes the chapter with the highlighted text · type a note to keep writing" |
  | `superseded` | "The draft changed (v{{expected}} → v{{draft}}), so this text won't be applied · Enter continues" |
  | `on_paused` | "Enter continues · type feedback" |
  | `off_running` | "Click here to pause the writer after this step and give feedback" |
  | `off_pausing` | "Pausing after this step… type your feedback" |
  | `off_paused` | "Paused · edit the text or type feedback; click outside the draft to continue" |
  | `other_paused` | "Paused · type feedback, or continue from the agent panel" |
  | `panel_hold` | "Decide the held step in the agent panel" |

  `held_completes` applies when the held call would complete the session. Its input cannot tell,
  because in `chapter` granularity every `insert_text` completes (spec/05 §3), so `ToolCallDto`
  gains `completes: boolean`, true while the deferred call's effects carry `complete`. With
  auto-pause off the note keeps the text and revokes the completion (W9 b).
- **Requests**: through the existing session mutations (`pause` gains `after_step`, `resume` gains
  `keep_write`, `message`), in a small serial queue (W13).
- **Keyboard**: W3, using the shared hotkey helper and `event.isComposing`. Alt+P keeps its global
  meaning (a hold pause), and the dock does not override it.
- **Draft region** (`useDraftRegionFocus`, W10):
  - the reading column and the dock share one container marked `data-draft-region`;
  - leaving is detected from `focusout` with a `relatedTarget` outside the container, or, when
    `relatedTarget` is null (a click on something unfocusable, such as a paragraph before its
    editor opens), from the target of the `pointerdown` that caused the blur. A click on a
    paragraph stays, and a click on the header leaves;
  - before the resume the hook waits for the chapter's content saves. The `useContentMutations`
    mutations gain a shared `mutationKey`, `keys.chapters.contentWrite(chapterId)`. The hook checks
    one task after the leaving event, so it sees a save the leaving editor starts on its own blur,
    and then waits until `queryClient.isMutating({ mutationKey })` is 0, through the mutation
    cache's subscription;
  - it is active only while `ownPause` is true.
- **Merged preview**: `pendingDraftView` and `PendingText` are unchanged. The highlighted text above
  the dock is what Approve applies, or what a keep-write send keeps.

### 3.4 What else changes

- `ChapterPage` renders `WriterDock` from `useWriterState` (session, held call, surface, standing)
  instead of `PendingWriteDock`. `useHeldDecision` stays for `HoldBar` and the panel.
- The agent panel's `SessionControls` shows "Pausing after this step…" instead of "Pausing…" while
  `pause_after_step` is set. Otherwise the panel is unchanged:
  - `HeldWriteNotice` still points to the draft pane;
  - the composer is still disabled with `agent:composer.disabled_dock` for a dock-owned hold;
  - while running, both the panel composer and the dock can steer, and only the dock pauses on
    focus.
- The `chapter:pending_*` keys of the old card are removed, except those `PendingText` and the
  header still use. The new keys go under `chapter:dock.*`, in both locales.

## 4. Tasks

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 25.1 | **Docs**: done with the plan (D-173). Re-read the amended sections; fix any a code fact contradicts and note it in D-173. | plan §7 | `check.sh` green (docs only) |
| 25.2 | **Server**: `0014_step_pause` and its guard; the entity, `SessionDto.pause_after_step` and `ToolCallDto.completes`; `PauseInput.after_step`; fresh pause flags in the hold decision and a guarded `pause_before_request`; `resume` on a running session with a pause pending (guarded clear, feedback to the inbox, re-read on a miss); `ResumeInput.keep_write`; bindings. | 02 §1, §2; 03 §3, §5.1, §5.2, §5.4, §6, §14; 10 §2.7, §2.8 | plan §5.1; `entity_schema.rs` fresh and upgraded |
| 25.3 | **Web**: `writerDock.ts` (the §3.2 table), `WriterDock.tsx`, `useDraftRegionFocus` and the content `mutationKey`, the Text tab column layout, the serial queue, the panel's "Pausing after this step…", removing `PendingWriteDock` and its unused keys; both locales. | 11 §5.2, §5.3, §9; PROJECT §6.4, §8.5, §8.13 | plan §5.2 Vitest; `i18n:check` |
| 25.4 | **Smoke**: smoke item 20 with the Demo provider. | PLAN §M11.5 | the checklist passes |

## 5. Tests

### 5.1 Server (`agent_control.rs`, `agent_write_holds.rs`, `entity_schema.rs`)
- **Migration**: `0014` on a database at `0013` keeps every session and sets the flag to 0; the
  fresh and upgraded schemas match.
- **Step pause**:
  - with `pause {after_step: true}` during a scripted `insert_text` step, the call is **not**
    held: the paragraphs are applied, and the session becomes `paused` before the next request;
  - with a completing call (`insert_text` with `chapter_end`, or `finish_draft`), it is held;
  - a hold pause sent after a step pause upgrades it (the next call holds), and a step pause sent
    after a hold pause changes nothing;
  - `transition` out of `running` clears both flags.
- **Cancel**:
  - `resume` on a running session with a pause pending clears both flags, adds no inbox row, and
    the session neither holds nor pauses;
  - with feedback, exactly one inbox `user_message` is added and delivered with the next sealed
    result;
  - running with no pause pending → `409 session.invalid_state`.
- **Fresh flags (W12)**: a cancel committed after the runner loaded its session copy, but before the
  step's transaction, is honored: the step does not hold. A cancel that arrives right before the
  `pause_before_request` transition makes the guarded transition miss, and the loop goes on.
- **Guard miss**: a step pause lands (scripted) between the request's read and its guarded update.
  The request is then handled as `paused`, and its feedback is queued.
- **Keep write**:
  - a held `insert_text` with `keep_write` and feedback: the chapter gains the paragraphs
    (`content_version` + 1, one `chapter.content` event), and the result ends with
    `notice.user_feedback`;
  - with `chapter_end`: the text is applied, `notice.completion_revoked` precedes the feedback, the
    session keeps running, and the chapter stays `drafting`;
  - a superseded write is dropped as on approval, and the feedback is appended;
  - `keep_write` without feedback behaves as approval;
  - plain feedback still discards the write (regression).
- `PauseInput` and `ResumeInput` without the new fields deserialize, and an empty `pause` body still
  works.

### 5.2 Vitest
- `writerDock.ts`: every cell of §3.2, the `ownPause` lifecycle (set, cleared by each event, lost on
  reload), and the placeholder per state, including `held_keep` and `held_completes`.
- `WriterDock`:
  - the dock is outside the scroll region, is present while running, held and paused, and is absent
    in `revision_planning`, `stopped` and `completed`;
  - keys: Enter, ⌘/Ctrl+Enter and Shift+Enter; Enter during composition does nothing; Escape blurs;
  - auto-pause on: Enter on an empty input approves and focus stays; text plus Enter sends
    `resume {feedback}` while held or paused and `message` while running, then clears and blurs;
  - auto-pause off:
    - focus on a running writer sends `pause {after_step: true}` once;
    - text plus Enter sends `resume {feedback}`, then blurs without a second `resume`;
    - leaving the region empty sends `resume` for the dock's own pause and nothing for another
      pause;
    - leaving with text, or by a window blur, sends nothing;
    - a held write shows Approve, and a note sends `keep_write: true`;
  - the draft region:
    - dock → paragraph sends nothing;
    - editing it and then clicking the header sends `resume` only after the save resolved;
    - dock → agent panel sends `resume`;
  - the serial queue: a send typed before the focus pause returned waits for it; the silent 409 and
    the `message` fallback; a failed send keeps the text;
  - a panel-owned hold disables the dock;
  - the panel shows "Pausing after this step…" while `pause_after_step` is set.
- Both locales have every `chapter:dock.*` key; the removed keys are gone from both.

## 6. Smoke item 20 (PLAN §M11.5)

- Demo, auto-pause **on**: write a chapter.
  - Each write holds, and the dock's round button shows ✓. Enter applies it, focus stays, and Enter
    applies the next one.
  - Type "更紧凑一点" and press Enter: the highlighted text is discarded, the input clears and loses
    focus, and the agent rewrites.
- Auto-pause **off**: click into the dock.
  - The panel shows "Pausing after this step…". The current step's paragraphs land in the text
    without a highlight, then the writer is paused.
  - Type "下一段转到对手视角" and press Enter: the input loses focus, and the writer continues and
    follows the note.
- Click into the dock, wait for the pause, click a paragraph and edit it, then click the chapter
  header: the edit is saved, the writer resumes, and its next result carries the edit report.
- Click into the dock and out with the input empty: the writer resumes. With a word typed, it stays
  paused until the word is sent or cleared. Switching browser tabs resumes nothing.
- With auto-pause off, press **Pause** in the panel during a write: the write holds and the dock
  shows ✓. A note keeps the paragraphs.
- Edit the draft during a held write and approve from the dock: the superseded placeholder names
  both versions, and Enter continues without the staged text.

## 7. Amended documents (with this plan)

- `docs/PROJECT.md`:
  - §6.4 (the dock and the step pause);
  - F4b step 2 (the stale hold-bar text replaced);
  - §8.5 Text tab (the dock, which replaces the stale held-insertion hold bar);
  - §8.13 (Enter in the dock);
  - §10 #37;
  - §11 (the participation row).
- `docs/spec/02-data-model.md`: §1 (0014), §2 (`pause_after_step`).
- `docs/spec/03-agent-runtime.md`:
  - §3 (the table rows);
  - §5.1 (the guarded pause before a request);
  - §5.2 (`hold_condition`);
  - §5.4 (keep write);
  - §6 (`pause`, `resume`);
  - §14 (fresh pause flags);
  - §15 (tests).
- `docs/spec/10-api.md`: §2.7 (the `pause` and `resume` bodies), §2.8 (`SessionDto`, `ToolCallDto`).
- `docs/spec/11-frontend.md`: §5.2 (`WriterDock` replaces `PendingWriteDock`), §5.3 (the panel
  label and note), §9 (Vitest).
- `docs/PLAN.md`: M25 and smoke item 20. `docs/README.md`: M0–M25.
- `docs/DECISIONS.md`: D-173.

## 8. Risks

- **"After this step" can be one step late.** A step pause requested after the loop loaded its
  session copy, while the model request is in flight, is first read at that step's hold decision
  (fresh, W12). It does not hold an ordinary call, so the pause lands before the following request.
  At worst the writer finishes the step it is generating, which is what the author asked for.
- **Editing while "Pausing after this step…"** is refused as before (the writer still runs). Once
  paused, the text is editable.
- **A completing step under a step pause holds**, with the highlighted text and `held_completes`.
  The author must decide it explicitly; leaving the dock does not finish the chapter.
- **Detecting the region.** `relatedTarget` is null for a click on unfocusable text, so the hook
  also uses the last `pointerdown`. A keyboard user who tabs out of the dock leaves the region by
  that tab, which is the expected result.
- **Accidental focus.** With auto-pause off, tabbing into the dock asks for a step pause, and
  tabbing out with nothing typed cancels it or resumes. That is the requested behavior.

## 9. Out of scope

- Moving the agent panel composer, or making the panel pause on focus.
- Changing the panel's Pause.
- A dock for the outline or for the revision plan (both are reviewed elsewhere, D-141, D-146).
- Per-paragraph accept and reject inside a staged write.
- Remembering unsent dock text, or the dock's own pause, across reloads.
