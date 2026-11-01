# Fix Plan: Steering Messages Carry the 【用户留言】 Header

> Milestone **M17** in `docs/PLAN.md`, decision **D-160**. The spec sections in §6 were amended
> together with this plan. M17 is independent of M15 and M16 and may run before either. It is
> one task plus a smoke check.

## 1. The bug

An author message sent to a session (a **steering message**: the composer while the agent runs,
the reply to an idle conception session, or feedback given when resuming a paused session) is
stored as an `agent_inbox` row of kind `user_message`. When the runner consumes the row, it
delivers `item.content`, the author's raw text, **without** the prompt-pack template
`notice.user_message`:

| Path | Code | What the model gets |
|---|---|---|
| A. Sealing a batch: appended to the last tool result | `agent/runner.rs` ≈ 1946, `seal::inbox_appendix(item.content.clone(), item.id)` | the raw text after the tool output, separated by a blank line |
| B. Nothing to seal, or a batch whose request never finished: a new user message | `runner.rs` ≈ 1976, `sections.push(item.content.clone())` | the raw text as one block |
| C. Text-only turn handling (an idle conception reply, spec/03 §5.6) | `runner.rs` ≈ 2158, same | the raw text as one block |

The contract says otherwise:
- spec/03 §5.3 slot 4 says inbox items are delivered as `notice.user_message`;
- the template exists (`prompts/notices.toml`: 【用户留言】 then the text);
- `writer.system` tells the model that author content arrives under the 【用户反馈】,
  【用户修改了正文】 and 【用户留言】 headers.

In path A, the model therefore cannot tell the author's instruction from the tool's output, and a
steering message is easy to ignore. The key's only occurrence in `crates/server/src` is a doc
comment in `agent/seal.rs`. Nothing ever rendered it, from M3 (`1d2a058`) on. Runtime notices
(`InboxKind::Notice`) are not affected: their content is rendered when they are queued
(`services/sessions.rs` ≈ 1851).

## 2. Decision (D-160)

1. **Render when consumed.** Each `user_message` inbox row is delivered as
   `notice.user_message {text: content}`, rendered with the session's `PromptSet` (book override
   → global override → default) at the moment it is consumed, in all three paths.
2. **The inbox row keeps the raw text.** `agent_inbox.content` stays what the author typed, so:
   - the queued rows of D-157 (`InboxItemDto.content`) still show the author's words;
   - a rollover moves raw rows to the child (spec/03 §9), which renders them when it consumes
     them;
   - `author_notes` in `writer.kickoff.rollover` stays raw text under its own heading.

   Rendering at insert time was rejected: it would put model-facing text in the queued-row UI and
   double the heading in `author_notes`.
3. **Persisted, never re-rendered.** The rendered text is written once into the appendix (path A)
   or the user-message block (paths B and C). It is never recomputed at request time: the history
   stays append-only and byte-stable for prompt caching (spec/03 §1.3, §11).
4. **The author's text is never lost.** If rendering fails after the registry's own fallback (a
   broken override falls back to the default with `prompt.override_failed`), log at `warn` and
   deliver the raw text.
5. **Unchanged:**
   - `notice.*` inbox rows;
   - every template, and therefore the `prompts_registry` snapshot;
   - the kickoff baselines;
   - the schema and the API shapes;
   - rows already sealed (append-only; the fix applies to rows consumed after the upgrade,
     including rows queued before it).
6. **UI:** `NoticeItem` already shows appendix text as delivered (for example 【用户反馈】), and
   the activity stream shows user-message blocks as delivered. The header therefore appears there
   too, which is the stream's rule: it shows what the model saw. No UI change.

## 3. Implementation

- `agent/seal.rs`: add a renderer for the author-message appendix, e.g.
  `author_message_text(prompts, content) -> String`, implementing decision 4. Keep `compose` and
  the canonical order unchanged: author messages stay slot 4 in arrival order, interleaved with
  runtime notices exactly as now.
- `agent/runner.rs`: in paths A, B and C, map each consumed item through one helper, the
  rendered notice for `InboxKind::UserMessage` and `content` as is for `InboxKind::Notice`. Use it
  in all three places so they cannot drift again. The `PromptSet` is already available
  (`text_only_inner` takes `prompts`; give the sealing paths the same).
- Fix the comment in `crates/server/tests/agent_write_rollover.rs` (≈ 135), which already assumes
  the header, by turning it into an assertion (§4).
- Update the `seal.rs` module doc so slot 1's example no longer names a key M16 removes. Name
  `writer.notice.revision_plan_accepted` instead.

## 4. Tests (write them first; they must fail on the current code)

`steering_header.rs`, with the `ScriptedProvider`:

1. **Path A**: a running write session, and the author sends a message during a slow step. The
   next tool call's `result_text` is the raw result, a blank line, and exactly the rendered
   `notice.user_message` (header plus text). The appendix has kind `inbox` with that text and the
   row's `inbox_id`. The next provider request carries it. Before consumption, the snapshot's
   `inbox[0].content` is the raw text.
2. **Path B**: the author's message arrives while nothing is pending to seal, and the next user
   message's block is the rendered notice. With an edit report in the same message, the report
   comes first, then the rendered message, as separate blocks.
3. **Path C**: an idle conception session gets a reply, and the new user message block is the
   rendered notice.
4. **Paused feedback** (`resume {feedback}` on a `paused` session) arrives rendered.
5. **Notices untouched**: a granularity change queued as `InboxKind::Notice` is delivered exactly
   as before, with no 【用户留言】.
6. **Overrides**: a book override of `notice.user_message` is the text delivered. A deliberately
   broken override delivers the default, with the `prompt.override_failed` warning the registry
   already emits.
7. **Rollover**: an unconsumed message moved to the child is delivered rendered in the child, and
   the child's kickoff `author_notes` still holds no copy of it
   (`agent_write_rollover.rs`, now asserted).
8. **Several messages in one batch** keep arrival order and each gets its own header.

Plus:
- `kickoff_baseline` and `prompts_registry` snapshots are unchanged.
- Existing tests that compared a delivered result with raw steering text change only by the
  header. List them in the commit message.

## 5. Acceptance
- §4 passes.
- `scripts/check.sh` is green.
- Smoke check with the Demo provider: write a chapter at paragraph granularity, send a message
  while it runs, and open the next tool card. Its appendix reads 【用户留言】 plus your text, and
  the queued row showed your plain text before that.

## 6. Documents amended with this plan
- **spec/03 §5.3**: slot 4 states when and how author messages are rendered, and that the inbox
  row keeps the raw text. §5.6 and §6 say that a user message built from inbox items renders
  author items the same way.
- **spec/11 §5.3**: the stream shows delivered text, so author messages show the header.
- **DECISIONS**: D-160. **PLAN**: the M17 table.

## 7. Out of scope
- Adding the header contract to the system prompts of roles other than the writer. The header is
  self-explanatory, and a system-prompt change would alter frozen prompts of new sessions; a
  prompt-quality pass can decide later.
- Rewriting already sealed results (append-only history).
