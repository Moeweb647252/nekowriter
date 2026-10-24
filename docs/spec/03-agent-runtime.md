# 03 — Agent Runtime

The runtime turns persisted sessions into model requests and tool executions. It is a
**resumable state machine over a persisted, append-only conversation**. A paused or held
session has no live task and no open model request, so it survives restarts and costs nothing
while it waits.

## 1. Concepts

### 1.1 Roles, session kinds, phases

| Session kind | Role | Phases (in order) | Ends with |
|---|---|---|---|
| `conception` | conception | `none` | author's Finalize (agent turns end with text → `idle`) |
| `research` | researcher | `none` | `finish_research` |
| `chapter_write` | writer | `planning` → `drafting` | `insert_text(chapter_end=true)` or `finish_draft` |
| `chapter_revise` | writer | `revision_planning` → `revising` | `finish_revision` |
| `chapter_review` | reviewer | `none` | `submit_review` (re-review continues the same session unless the text changed, §6) |
| `chapter_maintenance` | archivist | `none` | `finish_maintenance` |
| `canon_seed` | archivist | `none` | `finish_maintenance` |
| `chapter_resync` | archivist | `none` | `finish_maintenance` |

A **redraft** is a `chapter_write` session whose kickoff carries the author's feedback. A
**rollover** continuation (§9) is a new session of the same kind and phase.

### 1.2 Fixed tool lists, phase-gated permissions

Each role has **one fixed, ordered tool list** (spec/05 §2), frozen into the session at creation
(`agent_sessions.tools`). The list never changes during a session, so the provider's cached
prefix (tools → system → messages) stays valid. Phases restrict which tools may run. A call
to a tool that is not permitted in the current phase is answered with an error result
(`error.tool_not_allowed`, listing the permitted tools), and the call has no effect.

### 1.3 Append-only history

- Persisted messages (`agent_messages`) are never modified or deleted.
- A tool call's delivered text (`result_text`) is composed once, when the call is **sealed**
  (included in the next model request), and is immutable afterwards.
- The system prompt and tool definitions are frozen per session (`system_prompt`, `tools`).
  Editing prompts affects only new sessions.
- Reasoning and thinking content is replayed exactly as the provider returned it
  (`provider_payload`). Anthropic thinking blocks are required unchanged in the tool loop, and
  current Claude models reject requests whose earlier turns were edited (preserved thinking).
  DeepSeek thinking mode returns a 400 if `reasoning_content` is dropped.
- The only way to "shrink" a conversation is a **rollover** into a new session (§9).

## 2. Data recap

Tables: `agent_sessions`, `agent_messages`, `agent_tool_calls`, `agent_inbox` (spec/02). Key
fields: `status`, `phase`, `granularity`, `auto_pause`, `pause_after_outline`,
`pause_requested`, `held_tool_call_id`, `base_content_version`, `pending_comment`,
`view_snapshot`, `counters`, `usage`.

Each tool call row moves `pending → executing → (held | awaiting_input)? → done | error`. It keeps
`raw_result_text` (the tool's own output), `appendices` (sections to append, in canonical order
§5.3), `effects` (deferred phase change or completion), and `sealed`.

## 3. Session state machine

```
                ┌──── pause_requested at the next boundary · 5 tool errors in a row ────┐
                │                                                                        ▼
 create ─► running ──tool needs input──► awaiting_input ──answer──► running       held / paused
                │  ▲                                                         ▲          │
                │  └──────────── approve / feedback / continue ───────────────┴──────────┘
                │
                ├── terminal tool finalized ─────────────────────────────► completed
                ├── text-only reply (conception) ────────────────────────► idle ──message──► running
                ├── Stop ────────────────────────────────────────────────► stopped ──continue──► running
                ├── unrecoverable error / refusal / no-tool-call breaker ─► failed ──continue──► running
                ├── server restart while running ────────────────────────► interrupted ──continue──► running
                └── rollover ────────────────────────────────────────────► superseded (child session runs)
 any non-terminal ── Abandon ────────────────────────────────────────────► abandoned
 completed (chapter_review only) ── re-review comment ───────────────────► running
```

Terminal statuses: `completed`, `superseded`, `abandoned`. A session is **open** in every other
status. **Stop** exists only for `running` sessions: a held, paused, awaiting, or idle session
has no request in flight, so its options are to continue (resume, feedback, the form, a reply)
or to abandon it.

| From | Operation | To | Notes |
|---|---|---|---|
| running | pause | running (`pause_requested=1`) | takes effect at the next tool boundary or before the next model request |
| running | stop | stopped | cancels the in-flight model request or a tool's prepare phase (spec/05 §1.2); partial output is discarded |
| running | message | running | queued in the inbox; delivered with the next sealed tool result |
| held | resume (approve) | running | held call finalized unchanged |
| held | resume (feedback) | running | feedback appended to the held call's result |
| held / paused | message | same | queued; delivered at the next sealing, after the held call's own feedback |
| any open status | abandon | abandoned | a running session's request is cancelled first |
| awaiting_input | answer | running | the answer becomes the call's result |
| paused | resume (± feedback) | running | feedback queued as an inbox user message |
| idle | message | running | |
| stopped / failed / interrupted | continue | running | |
| stopped / failed / interrupted | message | same | queued; delivered when the session continues (§5.3: with the unsealed batch, or as a new user message) |
| completed (review) | rereview(comment) | running | appends a user message |

Every transition goes through one function `transition(session, to, reason)` that, in the same
transaction:
- writes the status (and `finished_at` for terminal statuses);
- when leaving `running`: clears `pause_requested` (a pause is consumed by the hold or pause it
  caused), and for writer and conception sessions stores `view_snapshot` (what the agent last
  saw, §7) unless an unreported snapshot is already stored (the oldest unreported view wins);
- emits `session.updated` after commit.

## 4. Creating a session

1. **Resolve the model**: `book.model_overrides[role]` → `app_settings["roles.defaults"][role]`.
   None → `409 model.missing_role`. Copy the profile and provider into `model_snapshot` (without
   the API key; the key is resolved from `providers` at every call so key rotation works).
   Resolve `cache_ttl: "auto"` here (spec/04 §3), so the TTL never changes within a session.
2. **Freeze prompts**: render the role's system prompt (`<role>.system`) and the tool
   definitions (spec/05 §1.3) through the prompt registry, with book overrides applied. Store
   them in `system_prompt` and `tools`.
3. **Initial fields**: phase (`planning` / `revision_planning` / `none`); writer sessions copy
   `granularity`, `auto_pause`, and `pause_after_outline` from book settings, or from the start
   request when given. Chapter sessions store `base_content_version` = the chapter's
   `content_version` (after the creation side effects below).
4. **Kickoff**: build the kickoff user message (§10) and persist it as `seq 0`, `kind kickoff`.
5. Set the status to `running` and spawn the runner. Emit `session.updated` and `session.message`.

Domain side effects on creation (same transaction): `chapter_write` sets the chapter to
`drafting`. `chapter_revise` sets it to `revising` and saves a `backup` version first.
`chapter_write` in redraft mode saves a `backup` version and clears the content. Maintenance
sets `canon_status` to `syncing`. Re-sync also saves the current text as a new `official`
version (spec/06 §8). Research sets the task to `running`.

## 5. The runner

### 5.1 Loop

```text
drive(session_id):                         # at most one runner per session (SessionManager)
  loop:
    s ← load(session_id)
    if s.status ≠ running: return
    if cancel_token.cancelled: transition(s, stopped); return

    # A. execute pending tool calls one at a time, in call_index order
    if tc ← first tool call of s with status = pending:
        step_tool(s, tc); continue

    tail ← classify_tail(s)
    match tail:
      NeedsModel:                           # last message is user/notice/nudge, or all calls of the last
                                            # assistant message are resolved
        if s.pause_requested: transition(s, paused, "pause_before_request"); return
        if rollover_needed(s): rollover(s); return            # §9
        seal_latest_batch(s)                                  # §5.3 (adds a user message instead if
                                                              # the batch is sealed and items are pending)
        step_model(s)                                         # §5.5
      AssistantTextOnly:                    # last assistant message has no tool calls
        handle_text_only(s)                                   # §5.6
```

The cancellation token carries a reason. The pseudo-code writes `transition(s, stopped)` for
brevity: the reason `stop` gives `stopped`, `shutdown` (spec/01 §5) gives `interrupted`, and
`abandon` makes the runner exit without a transition, because the abandon operation then sets
`abandoned` itself.

`classify_tail`:
- last message is a user message (any kind) → `NeedsModel`;
- last message is assistant with tool calls, all `done`/`error` → `NeedsModel`;
- last message is assistant without tool calls → `AssistantTextOnly`.
(`held`/`awaiting_input` calls never reach this point because the status is not `running`.)

### 5.2 `step_tool`

```text
step_tool(s, tc):
  mark tc executing (started_at); emit session.tool_call
  if not permitted(s.kind, s.phase, tc.name):
      outcome ← error("tool_not_allowed", {tool, phase_label, allowed})
  else:
      input ← strict JSON parse of tc.input           → on failure: outcome ← error("invalid_json", raw prefix)
      schema + typed validation (spec/05 §1.4)        → on failure: outcome ← error("validation", …)
      prepared ← select!(tool.prepare(pctx, input), cancel_token)    # no transaction (spec/05 §1.2)
         on cancel: reset tc to pending; transition(s, stopped); return
         on ToolError: outcome ← error(…)
  BEGIN
    if no outcome yet: outcome ← tool.execute(ctx(tx, s), input, prepared)   # reads + writes, no I/O waits
    tc.raw_result_text, tc.ui, tc.effects, tc.is_error ← outcome
    update counters (tool_calls, retrieval_calls, consecutive_tool_errors: +1 on error, 0 on success)
    if outcome.await_input:            tc.status ← awaiting_input; s.held_tool_call_id ← tc; transition(s, awaiting_input)
    elif hold_condition(s, tc, outcome): tc.status ← held;         s.held_tool_call_id ← tc; transition(s, held)
    else:
        finalize(s, tc)                                            # §5.4, same transaction
        if s is still running and counters.consecutive_tool_errors ≥ 5:
            s.error ← {code: "agent.tool_error_streak"}; transition(s, paused, "needs_attention")
  COMMIT
  emit domain events from outcome (chapter.content, canon.changed, …), session.tool_call, session.updated
```

Every path, including rejected and invalid calls, goes through the same transaction, counters,
hold check, and finalize, so error results can be held and count toward the streak. A call's
domain effects, its result, and its final status (`done`, `error`, `held`, or
`awaiting_input`) are committed together, so a call found in `executing` after a crash
provably had no effect.

`hold_condition(s, tc, outcome)`:
```text
  s.pause_requested                                                        → true
  s.auto_pause and tc.name ∈ WRITE_TOOLS and not outcome.is_error          → true
  tc.name = "submit_outline" and s.pause_after_outline and not is_error    → true
  otherwise                                                                → false
WRITE_TOOLS = {insert_text, insert_paragraphs, replace_text, rewrite_paragraph, delete_paragraphs}
```

### 5.3 Appendices and sealing

The text delivered for a tool call is `raw_result_text` followed by its appendices, separated by
a blank line, in this **canonical order**:

1. **Effect notices** about the call itself: `writer.notice.outline_accepted` (rendered with
   the current granularity rules), `writer.notice.revision_plan_accepted`,
   `writer.notice.outline_rejected`, `writer.notice.revision_plan_rejected`,
   `notice.completion_revoked`, and the budget notices (`writer.notice.retrieval_budget`,
   `notice.research_budget`). The revoke notices carry no feedback text; the feedback follows in
   slot 3.
2. **Edit report** (`notice.user_edits` or `notice.setting_edits`, §7): the author's edits since
   the agent last saw the chapter or the setting materials.
3. **Author feedback** given on a held call (`notice.user_feedback`).
4. **Inbox items** in arrival order: steering messages (`notice.user_message`) and runtime
   notices (e.g. `writer.notice.granularity`).

`seal_latest_batch(s)` runs right before a model request:
```text
  report ← edit_report(s.view_snapshot, current state) if s.view_snapshot is set     # §7
  items  ← pending inbox of s (FIFO)
  batch  ← tool calls of the latest assistant message, if any are unsealed
  if batch is not empty:
      last ← batch[max call_index]
      if report: last.appendices += report
      for item in items: last.appendices += render(item); mark consumed by last
      for tc in batch: tc.result_text ← compose(tc.raw_result_text, tc.appendices); tc.sealed ← 1
  elif report or items:                    # the batch was sealed by an earlier request that was
      insert a user message (kind user if any author message, else notice) of the rendered
      sections; mark the items consumed by it   # stopped, failed, or interrupted
  s.view_snapshot ← NULL
```
A text-only tail is handled the same way by §5.6 before this point. `compose` orders appendices
by kind in the canonical order above, keeping arrival order within a kind (feedback is stored at
resume, before the edit report is computed at sealing). When nothing is pending, Continue after
a stop or failure re-sends exactly the earlier request (test 7); otherwise the new user message
carries what the author did meanwhile, and no history is edited.

### 5.4 `finalize` and atomicity

```text
finalize(s, tc):            # inside step_tool's transaction, or its own when a held/awaiting call is resumed/answered
  apply tc.effects:
    phase_to   → s.phase ← phase_to; tc.appendices += phase notice (outline_accepted / revision_plan_accepted)
    complete   → run the completion handler for s.kind (spec/08): e.g. finish_draft → chapter in_review +
                 'draft' version; finish_revision → in_review + 'revision' version; submit_review → report
                 visible; finish_maintenance → canon_status synced or stale (spec/06 §6); finish_research
                 → task done
  tc.status ← error if tc.is_error else done; tc.finished_at ← now
  if effects.complete:
      finish every remaining pending call of the same assistant message as error.after_completion
        (not executed, never held)
      seal_latest_batch(s); transition(s, completed)
```

Every call therefore has a sealed result even in a completed session, which a re-review
(§6) needs, because it continues the reviewer's conversation.

- **Atomicity**: a tool's domain effects, its result row, and its status are written in one
  transaction (deferred effects are applied by the transaction that finalizes the call). On
  restart, a call left in `executing` had no committed effects and is reset to `pending`
  (spec/01 §5). The prepare phase only reads (web, search, embeddings), so re-execution is safe.
- **Revoking deferred effects**: when the author gives feedback on a held call whose effects
  include `complete` or `phase_to`, the effects are cleared before finalizing, and the matching
  revoke notice is appended (`writer.notice.outline_rejected` for `submit_outline`,
  `writer.notice.revision_plan_rejected` for `submit_revision_plan`, `notice.completion_revoked`
  for a completion), followed by the feedback. The agent keeps working in the same phase.

### 5.5 `step_model`

```text
step_model(s):
  req ← build_request(s)                                   # §11
  meta ← {phase: s.phase, granularity: s.granularity}       # stored with the assistant message
  turn ← provider.run(req, on_event) with retry (spec/04 §6), raced against cancel_token
  on_event:
     text/reasoning delta   → accumulate; emit session.delta (coalesced every 50 ms)
     tool call start/delta  → accumulate args; feed preview extractor (§12) → session.tool_preview
     usage / stop           → record
  on cancel: drop the stream, persist nothing, transition(s, stopped); return
  on mid-stream retryable error: discard partial output, retry the whole request (≤ 4 attempts total, spec/04 §6)
  on non-retryable error: s.error ← classified error; transition(s, failed); return
  BEGIN
    insert assistant message (content blocks, provider_payload, usage, stop_reason, request_meta = meta)
    insert tool calls (status pending, input = raw argument text) in order
    s.usage += usage; counters.steps += 1; if tool calls: counters.consecutive_nudges ← 0
  COMMIT; emit session.message, session.tool_call (each), session.updated (usage)
  stop reason handling:
     max_tokens with tool calls → every call whose input is not valid JSON is finished as error
                                   (error.output_truncated); valid ones run normally
     refusal                    → finish all its calls as error (not executed);
                                   s.error ← {code: "provider.refusal", message}; transition(s, failed)
     pause_turn                 → treated as end_turn (we use no server-side tools)
```

### 5.6 Text-only replies

```text
handle_text_only(s):
  pending ← edit report (if view_snapshot is set and differs, §7) + pending inbox items
  if pending: insert user message (kind = user if any author message else notice) composed of the
              rendered sections; consume inbox; clear view_snapshot; return   (loop continues → NeedsModel)
  match s.kind:
    conception → transition(s, idle)                      # the author replies when ready
    otherwise  → if counters.consecutive_nudges < 2:
                    insert user message kind nudge = render(nudge key for kind/phase); counters.consecutive_nudges += 1
                 else: s.error ← {code: "agent.no_tool_call"}; transition(s, failed)
```

Nudge keys by writer phase: `planning` → `writer.notice.nudge_planning`, `drafting` →
`writer.notice.nudge_drafting`, `revision_planning` → `writer.notice.nudge_revision_planning`,
`revising` → `writer.notice.nudge_revising`; reviewer, archivist, and researcher sessions get
`notice.nudge_generic`.

### 5.7 Errors and circuit breakers

| Condition | Handling |
|---|---|
| Provider network error, timeout, 429, 5xx, overloaded | retry with backoff 2 s, 8 s, 30 s (honor `Retry-After`), then `failed` (`provider.unavailable`, retryable) |
| 401/403 | `failed` (`provider.auth`) |
| 400 context length exceeded | rollover if eligible (§9), else `failed` (`provider.context_overflow`) |
| Other 400/404 | `failed` (`provider.bad_request` with the provider's message) |
| The provider row of the model snapshot was deleted, so its key cannot be read | `failed` (`model.missing_provider`); the author abandons the session and starts a new one |
| `refusal` stop reason | `failed` (`provider.refusal`); the author may edit settings or feedback and Continue |
| 5 consecutive tool errors | `paused` with `agent.tool_error_streak` (the author reads the errors, gives feedback, resumes) |
| 2 consecutive nudges without a tool call | `failed` (`agent.no_tool_call`) |
| Planning retrieval budget exceeded (30 retrieval calls in a writer session) | from then on, each retrieval result gets the appendix `writer.notice.retrieval_budget`; calls still work |

`failed`, `stopped` and `interrupted` sessions show **Continue**, which re-enters the loop. Because
history is intact, continuing re-sends exactly the request that failed, plus a user message
when the author edited or wrote something meanwhile (§5.3).

## 6. Control operations

All operations are `POST /api/sessions/:id/<op>` (spec/10) and are validated against the status
table in §3. Invalid ones return `409 session.invalid_state`.

| Operation | Effect |
|---|---|
| `pause` | `pause_requested ← 1` (running only). The UI shows "Pausing…" until the status changes |
| `resume {feedback?}` | **held**: if feedback is present, append `notice.user_feedback` to the held call's appendices (and revoke deferred effects per §5.4); finalize the call; status → running; spawn the runner. **paused**: feedback (if any) → inbox `user_message`; clear `error` if it was a circuit-breaker pause; status → running; spawn |
| `stop` | running only. Cancels the runner's token and returns at once (the UI shows "Stopping…"). The runner is the only writer of a running session, so it performs `transition(stopped)` at its next await point: it drops a model stream without persisting it, returns a call in its prepare phase to `pending`, or, if a tool transaction is in progress, lets it commit and stops at the loop top |
| `continue` | stopped, failed, or interrupted → running (clear `error`); spawn |
| `message {text}` | append an inbox `user_message`. If the status is `idle`, transition to running and spawn. Rejected while `awaiting_input` (the form must be answered) and in terminal statuses |
| `answer {tool_call_id, payload}` | awaiting_input only. Validates the payload against the call (spec/05 §7); `raw_result_text ← render(answer)`; finalize; status → running; spawn |
| `abandon` | any open status → abandoned (a running session is stopped first: cancel, wait for the runner to exit). Releases chapter locks; writer sessions leave the chapter content as it is; an archivist session on a chapter sets its `canon_status` to `failed` (spec/06 §6) |
| `PATCH {granularity?, auto_pause?, pause_after_outline?}` | open writer sessions only. A granularity change during `drafting`/`revising` queues inbox notice `writer.notice.granularity` (rendered for the new mode). The other fields need no notice |

Re-review: `POST /api/review-reports/:id/rereview {comment}` appends a user message rendered from
`reviewer.rereview` to the report's session, stores the comment in `pending_comment` (copied
into the next report's `user_comment`), and sets the session to running. That session must be
`completed` (otherwise `409 session.invalid_state`: it is still working). If the chapter's
`content_version` changed since that report, it creates a new `chapter_review` session instead,
whose kickoff carries the previous report and the comment (`reviewer.kickoff` with `prior`), with
the same `pending_comment`.

## 7. Edit reports (keeping the agent's view consistent)

- When a writer or conception session leaves `running`, `view_snapshot` stores what the agent
  last saw (spec/02 `ViewSnapshot`): for a writer, the chapter content, outline, and reference
  notes; for conception, the three setting documents, the character designs, and the draft
  research topics. Server recovery sets it for interrupted sessions too.
- While the session is not running, the author may edit them (spec/08 §1.4 locking). The most
  common case is editing the outline while `submit_outline` is held, then approving it.
- When the session next talks to the model (sealing, or a text-only tail), the runtime compares
  `view_snapshot` with the current state.
- **Writer** (`notice.user_edits`), paragraph diff:
  - **modified**: same pid, different text;
  - **deleted**: pid missing now;
  - **inserted**: new pid, located by the preceding pid (`after p7`, or "at the start");
  - **tail**: change of `tail_open`;
  - **outline / reference notes**: when changed, the full current text (the agent drafts from
    them).
  Inserted and modified paragraphs show their full new text; old text is truncated to 120
  characters. If the paragraph part of the report exceeds 4,000 characters, the compact variant
  replaces it: counts per change type and the instruction to call `read_draft`.
- **Conception** (`notice.setting_edits`), per changed item: a document whose revision changed
  shows a line diff (`similar`) when at most 40 lines changed, otherwise its full current text;
  characters show the current ranked list, plus the full profile of each character whose profile
  changed; research topics show the current list.
- `view_snapshot` is cleared after it is reported.

## 8. Granularity

| Mode | `insert_text` rule | `paragraph_end` | `chapter_end` |
|---|---|---|---|
| `sentence` | exactly one sentence: no line breaks, ≤ 400 characters | closes the paragraph when true; otherwise the next insert appends to it | marks completion |
| `paragraph` | exactly one paragraph: no line breaks, ≤ 3,000 characters | ignored (always closed) | marks completion |
| `free` | any text; line breaks separate paragraphs; ≤ 60,000 characters | state of the last segment | marks completion |
| `chapter` | the rest of the chapter in one call; line breaks separate paragraphs; ≤ 60,000 characters | ignored (closed) | implied true |

- **Open tail**: in every mode, the first segment continues an open tail paragraph (so in
  `paragraph` mode the call completes the open paragraph and closes it). The notices say so
  whenever the tail is open.
- **Other text-adding tools**: under `sentence` and `paragraph`, `insert_paragraphs` and
  `rewrite_paragraph` also take a single paragraph (no line breaks, ≤ 3,000 characters), so
  granularity cannot be bypassed; under `free` and `chapter`, they take several paragraphs
  (≤ 60,000 characters). `replace_text` and `delete_paragraphs` are not limited.
- **Short completion** (drafting only; revisions finish with `finish_revision`): a completion
  (`chapter_end`, `chapter` mode, or `finish_draft`) below 60% of `target_words_min` is withheld
  once. The text is kept, the result says the chapter is short and asks the agent to continue or
  confirm with `finish_draft`, and `counters.finish_warned` is set; the next completion goes
  through.
- **Joining**: when appending to an open paragraph, a single space is inserted between texts
  unless the writing language is unspaced (`zh`, `ja`, `th`, `lo`, `km`, `my`) or either side
  already has whitespace. Leading and trailing whitespace of segments is trimmed. Empty segments
  are dropped. `\r\n` is normalized to `\n`, and runs of blank lines count as one break.
- **Which rule applies**: the granularity stored in the assistant message's `request_meta`
  (the mode in effect when the model generated the call), so a switch never rejects a call
  that was already being generated.
- **Switching**: `PATCH` changes `granularity`. In drafting or revising, the inbox notice
  `writer.notice.granularity` (one variant per mode, plus the open-tail hint) tells the model
  the new rule from its next call.
- **Validation errors** return `error.granularity` (the mode and the reason: a line break, too
  long, or empty) without applying anything.
- In `revising`, `chapter_end` is ignored, and the result appends a hint to call
  `finish_revision`.

## 9. Rollover (context management without editing history)

- **Eligibility**: writer sessions in `drafting` or `revising`.
- **Trigger** (checked before every model step): `usage.last_prompt_tokens + max_output_tokens
  > 0.85 × context_window`, or the previous request failed with context overflow.
- **Procedure** (one transaction):
  1. Create a child session (same kind, role, chapter, phase, granularity, auto-pause flags,
     model snapshot; `parent_session_id` = old). Its kickoff is
     `[block.settings][block.memory][block.chapter_draft][writer.kickoff.rollover]`.
     `block.chapter_draft` holds the current outline, reference notes, and text. The task
     block holds the chapter's guidance, the feedback being addressed (a revision's or a redraft's,
     with its ticked issues), the accepted revision plan (revise), the granularity rules, and
     **the author's earlier feedback and steering messages from the parent chain** in order
     (`author_notes`: `user_feedback` appendices, sealed or not, and consumed `user_message`
     inbox items; newest last; at most 20 items / 6,000 characters, keeping the newest).
     Feedback given on the parent's last held call is therefore not lost.
  2. Move unconsumed inbox items to the child (delivered at its first sealing, not duplicated in
     `author_notes`). No edit report is needed, because the kickoff contains the current text.
  3. Old session → `superseded`. Child → `running`; spawn.
- The UI follows `parent_session_id` links. The agent panel switches to the child and shows a
  divider "Context renewed".
- Non-eligible sessions that overflow fail with `provider.context_overflow`, and the message
  suggests a model with a larger context window.

## 10. Context assembly (kickoff)

Ordering principle: **stable first**. System prompt (static per role) → setting materials (stable
per book revision) → memory (changes per chapter) → task (changes per session). Explicit cache
breakpoints sit at the end of the system prompt (shared by every session of the role) and at the
end of the settings block (shared by every session of the book); see spec/04 §3.

| Kind | Kickoff blocks (in order) |
|---|---|
| conception | `conception.kickoff` |
| research | `block.book_brief` (only if linked to a book) · `researcher.kickoff` |
| chapter_write | `block.settings`⁎ · `block.memory` · `writer.kickoff.chapter` (or `writer.kickoff.redraft`) |
| chapter_revise | `block.settings`⁎ · `block.memory` · `block.chapter_draft` · `writer.kickoff.revise` |
| chapter_review | `block.settings`⁎ · `block.memory` · `block.chapter_draft` · `reviewer.kickoff` (with `prior` when it replaces a re-review of an older version, §6) |
| chapter_maintenance | `block.settings`⁎ · `block.memory` · `block.cast_index` · `block.chapter_draft` · `archivist.kickoff.maintenance` |
| canon_seed | `block.settings`⁎ · `archivist.kickoff.seed` |
| chapter_resync | `block.settings`⁎ · `block.memory` · `block.cast_index` · `block.chapter_diff` · `archivist.kickoff.resync` |
| rollover | `block.settings`⁎ · `block.memory` · `block.chapter_draft` · `writer.kickoff.rollover` |

⁎ `cache: true` on that block. Each block is one `text` block of the kickoff message.

Task templates pull in two more blocks through template includes, so the table stays short:
`block.anchor` (every writer, reviewer, and archivist task of a side-track chapter) and
`block.chapter_draft` (`writer.kickoff.chapter` when the chapter already has text).

- `block.memory` is built for the target chapter's position (spec/06 §4), i.e. the state of
  canon before this chapter. For maintenance and review that is the current canon, because this
  chapter has not been maintained yet. It leaves out the chapter outline when the kickoff
  already shows it (spec/06 §4.2).
- `block.chapter_draft` renders the chapter outline, reference notes, and the full text with
  pids (`[p12] …`), marking an open tail paragraph.
- **Budget warning**: estimate kickoff tokens (CJK characters × 1.0 + other characters ÷ 4,
  rounded up). If the estimate exceeds 50% of the context window, the session still starts and
  `session.updated` carries `warnings: ["context.large_kickoff"]`.

## 11. Request building

```text
build_request(s) → nekowriter_llm::ChatRequest {
  model, max_output_tokens (from model_snapshot),
  system: s.system_prompt,
  tools: s.tools (frozen, fixed order),
  parallel_tool_calls: role ≠ writer,          # writer: one call per round
  messages: for m in agent_messages by seq:
     user      → Message::user(blocks with cache flags)
     assistant → Message::assistant_replay(m.provider_payload)
                 followed by tool results for its calls in call_index order:
                 ToolResult { call_id: provider_call_id, content: result_text, is_error }
  cache: CacheHints { ttl: model_snapshot.options.cache_ttl }   # resolved at creation (§4); Anthropic only
  options: model_snapshot.options
  demo_hints: Some({kind, phase, granularity, step_in_phase, language}) when the protocol is mock, else None
}
```
Invariant checked before sending: every assistant tool call has a sealed result, and roles
alternate after the adapter merges adjacent user content (spec/04). If a provider ignores
`parallel_tool_calls = false` and returns several calls, they run one after another as usual
(each can hold); nothing is dropped.

## 12. Streaming previews

The author sees long tool arguments while they are generated.

| Tool | Preview field | Rendered in |
|---|---|---|
| `insert_text` | `text` | chapter ghost text at the cursor |
| `insert_paragraphs`, `rewrite_paragraph` | `text` | tool card (and a ghost under the target pid) |
| `submit_outline` | `outline` | Outline tab and tool card |
| `submit_revision_plan` | `plan` | tool card |
| `write_setting_doc` | `content` | conception document tab |
| `save_knowledge_entry` | `content` | tool card |
| `submit_review` | `summary` | tool card |
| `set_chapter_summary` | `summary` | tool card |

`agent/preview.rs` implements an incremental extractor: `PreviewExtractor::new(field)` and then
`feed(&str) -> Option<String>` returns newly decoded characters of the top-level string field
`field` as raw JSON fragments arrive. It handles nesting, other keys appearing first, all JSON
escapes (`\n \t \" \\ \/ \b \f \r \uXXXX` incl. surrogate pairs), and escape sequences split
across chunks. Tests feed sample payloads split at every byte boundary. Preview events identify
the in-flight call by `(session_id, seq of the assistant message being generated, call_index)`.
The frontend discards a preview when the persisted tool call arrives.

## 13. Usage accounting

Each assistant message stores its `Usage`. The session accumulates totals and
`last_prompt_tokens` (input + cached + cache-write for the latest request). The agent panel shows
the cumulative input (with the cached part) and output tokens. Book-level totals are a sum over
its sessions (computed on demand for the overview).

## 14. Concurrency and locking rules

- One runner per session (SessionManager). Spawning an already-running session is a no-op.
- **Session row writes are column-wise and guarded**, never a write-back of a stale model:
  `UPDATE agent_sessions SET <changed columns> WHERE id = ? AND status = <expected>`. While a
  session is `running`, the API writes only `pause_requested`, `granularity`, `auto_pause`,
  `pause_after_outline`, and inbox rows; the runner writes everything else and re-reads the row at
  every loop step, so a concurrent pause or granularity change is never lost. The other
  operations of §6 (resume, answer, continue, abandon, message to an idle session) act on a
  session that is not running, inside one transaction guarded by its expected status, before a
  runner is spawned, so a double click gets `409 session.invalid_state` instead of a second
  runner.
- **Chapter locks**: at most one open writer session per chapter (`409 chapter.session_exists`,
  returning the session id). Author edits are rejected with `423 chapter.locked` while that
  session is `running`; they are allowed in every other status and reported via §7.
- Archivist sessions on a chapter never touch its text, so author edits to an official chapter
  during maintenance are allowed. The completion handler compares `content_version` with the
  session's `base_content_version` and marks the chapter `stale` when they differ (spec/06 §6).
- Reviewer sessions do not lock the chapter either. A report records the version it reviewed,
  so edits made during a review make it "for an earlier version" (spec/08 §1.2).
- Conception: one open session per book. Its documents may be edited by the author while the
  session is not running; edits are reported with `notice.setting_edits` (§7).

## 15. Required runtime tests (`crates/server/tests/agent_*.rs`)

With `ScriptedProvider` (spec/04 §7) and a temp database:
1. Plan → `submit_outline` → (pause_after_outline) held → approve → drafting notice contains the
   granularity rule → `insert_text` ×3 → `chapter_end` → chapter `in_review` with a draft version.
2. Held outline + feedback → phase stays `planning`; the result contains the rejection notice and
   the feedback; resubmission works.
3. Auto-pause: each insert holds; approve continues. Feedback is appended to that call's result
   only.
4. Pause request during streaming → holds after the call completes. Pause with a text-only
   response → `paused`.
5. Steering while running → appended to the next sealed result, in FIFO order.
6. Author edits while held (modify, delete, insert) → exact edit report. A large edit → compact
   report.
7. Stop mid-stream → nothing persisted from the partial response; continue re-sends an identical
   request (compare request bodies). Stop, edit a paragraph, send a message, continue → the
   request gains exactly one user message carrying the edit report and the message.
8. Simulated crash (`executing` row with no effects) → recovery resets it to pending → re-executed
   once.
9. A retrieval tool during drafting → error result, no effect.
10. Granularity switch while the model generates → the call validates against the old mode; the
    next result carries the switch notice.
11. Text-only replies → two nudges → `failed`.
12. Five consecutive invalid calls → `paused` with `agent.tool_error_streak`.
13. Rollover when `last_prompt_tokens` is near the limit → child session with the draft in its
    kickoff; parent superseded; inbox moved; feedback given on the parent's last held call
    appears in the child's `author_notes`.
14. Append-only: after any scenario, re-building the request for the previous step yields a
    byte-identical prefix of the current request (serialize both and compare).
15. `stop` on a held, paused, or awaiting session → `409 session.invalid_state`. A pause
    request is cleared by the hold it causes: resume does not pause again.
16. A response with a completing call followed by another call → the second call gets
    `error.after_completion` without executing; every call of the batch is sealed.
17. Stop while a call is in its prepare phase (a slow scripted web fetch) → the call is back to
    `pending`, nothing was written; continue executes it once.
18. Author edits the outline while `submit_outline` is held, then approves → the result
    carries the outline-accepted notice and an edit report with the new outline.
19. Short completion: `chapter_end` below 60% of the target → text kept, completion withheld,
    `finish_warned` set; the following `finish_draft` completes.
20. Pause → held; author edits a paragraph, then resumes with feedback → the sealed result is
    raw text, edit report, feedback, in that order. A crash injected after `execute` but before
    commit leaves the call `executing` with no effect; recovery re-runs it exactly once.
21. A pause request and a granularity change committed while the runner is inside a tool
    transaction are both honored at the next boundary (no lost update); two concurrent
    `continue` calls start one runner and the second gets `409 session.invalid_state`.
