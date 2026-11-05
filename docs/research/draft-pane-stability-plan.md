# Implementation Plan: A steady draft pane (no blank, no flash, no jump)

> Milestone **M26** in `docs/PLAN.md`. Read `AGENTS.md` first. The spec sections listed in §7 are
> amended together with this plan, and the decision is recorded as **D-174**. The spec is
> authoritative; this plan explains the reasons, the order and the tests. M26 is web-only: no
> migration, no endpoint, no prompt, no binding.

## 0. What the author reported

The author writes by **sentence** with **auto-pause on**, so every sentence goes through the same
cycle: the writer streams it, the call holds with a staged write, the author approves, and the
next request starts. Their reports, in order:

> 流式显示会在句子完成后消失，然后突然蹦出来一段带高亮的草稿，看起来观感很不好

Commit `08b0ee6` ("keep the writer preview through the write handoff") answered that. Then:

> 修了还是有闪烁，再查一下

> 还有高度突变，其中一个高度突变是因为那个蓝色的写作中提示出现又消失

> 写吧

So the bar is: from the first streamed character to the text after approval, **the sentence never
leaves the screen, only the sentence changes its look, and nothing moves**.

## 1. Decisions at a glance

| # | Decision |
|---|---|
| S1 | **Only the staged text is revealed.** The merged view's entrance animates the parts a held write adds (an `added` paragraph and the appended span of a `changed` one) from the ghost's ink to full ink and from no tint to their waiting tint. Confirmed text never animates. |
| S2 | **An event's cache write is never undone by an older response.** Every write an SSE event makes into a query cache re-issues a fetch of that key already in flight, so any response that lands afterwards was requested after the event. |
| S3 | **A tool call is written wherever the snapshot holds it**: the flat `tool_calls` and the `tool_calls` of its message (`message_id`), which is what the activity stream renders. |
| S4 | **A writer preview stays until its replacement is on screen.** At `session.message` a writer tool's preview is pinned (no caret) with the draft version it was written against. The pane renders it while that version is still the draft's and no merged view replaces the list. The store drops it when its call ends (`done` or `error`), when the session leaves `running` and `held`, and on a reconnect; never on `held` and never on `chapter.content`. |
| S5 | **A resolved call is not the held call.** The chapter pane's held call is the call `held_tool_call_id` names only while that call's own status is `held`. |
| S6 | **The lock notice lives in the status row.** The blue banner above the text goes. The line that shows the word count says the writer is working, with a lock icon. That line is one line high in every state. |
| S7 | **Submit is disabled, not hidden, while the owner session runs.** The header's action row keeps its height through running ↔ held. |
| S8 | **An empty chapter streams too.** While a preview exists, the reading column renders the prose block with the ghost instead of the empty state, so the first sentence of a new chapter appears as it is written. |
| S9 | **Two invariants, each with a regression test**: the sentence is in the draft column after every DOM mutation of a write cycle; and no block above the paragraphs, and nothing in the header's action row, mounts or unmounts when the session moves between `running` and `held`. Inline content may change (the lock icon, the header's projected `→ N`), because it does not change a height. |

## 2. Facts this plan rests on (code at `eca5a47`)

### 2.1 Why it still flickers

- **The whole chapter fades on every hold.** `PendingText` wraps the merged view in
  `<div className="pending-text">` (`PendingText.tsx:27`). `prose.css:76-78` gives that wrapper
  `animation: pending-reveal 180ms`, a `color` animation from 45 % ink to `inherit`. Every
  paragraph inherits `color`, so each time a held write appears, i.e. once per sentence, the whole
  chapter turns grey and back. The intent was to fade only the new sentence.
  - `pending-amber` runs with `animation-fill-mode: both` and ends at 15 %. It therefore overrides
    the `bg-status-waiting/30` that `dac8c48` gave a changed paragraph's appended span: that span
    now settles at 15 %.
- **An older snapshot response overwrites the hold.**
  - `session.message` still invalidates `keys.sessions.detail(id)` (`events.ts:592`). With the
    agent panel on this session that query is active, so a GET starts at once.
  - The hold commits a few milliseconds later. In the author's session, tool call 317 was
    persisted at `17:59:49.6405` and the session was held at `17:59:49.6487`: 8 ms. The GET can
    read the database before that.
  - `updateSessionSnapshot`, `updateCachedToolCall` and `updateChapterOpenSession`
    (`events.ts:54-120`) write with `setQueryData` and leave an in-flight fetch alone. TanStack's
    `Query.fetch` stores its result with `setData` and does not compare ages
    (`query-core/build/modern/query.js:446`). When the old GET lands after the hold's events, the
    snapshot says the call is still `executing`, with no `pending_content`.
  - The effects:
    - the merged view disappears, and the sentence with it;
    - `heldDecisionSurface` gives the call to the panel, so the dock is disabled;
    - the panel thinks the session runs.
  - No later event refetches, so this lasts until a reload or the session's next event. It is a
    race, and it explains flicker that comes and goes.
- **The pinned preview is cleared before its replacement can render.**
  - `session.updated` with `held` clears the session's previews (`events.ts:620-629`).
  - That is a Zustand write, and Zustand reaches React synchronously. The cache writes beside it
    reach React only when TanStack's `notifyManager` flushes, on `setTimeout(0)`
    (`query-core/build/modern/notifyManager.js`).
  - So at least one render shows neither the ghost nor the merged view.
  - When the snapshot is not cached at all, the blank lasts a full GET. `useWriterState.ts:75`
    subscribes only while the session is held. The agent panel stays mounted when collapsed
    (`BookLayout.tsx:213-225`), so its query keeps the snapshot loaded, except when the panel is
    pinned to another session, or the page opened during the step and the first GET has not
    returned.
- **Other clearing gaps.**
  - `chapter.content` clears every preview of the chapter's open sessions (`events.ts:404-407`).
    Nothing visible depends on it today: `useWriterState` shows one preview per seq, the first key
    of the newest seq (`useWriterState.ts:93-107`). A second write call of the same turn therefore
    never streams; it appears when it lands. Under S4 this clear is redundant, because the version
    check hides a landed preview in the same update.
  - `session.tool_call` clears only an errored call (`events.ts:643-648`). A call finished by
    feedback was cleared only by the `held` branch.
- **The panel's cards go stale.** `updateCachedToolCall` patches only the flat `tool_calls`. The
  activity stream renders `messages[].tool_calls` (`ActivityStream.tsx:279`). Since
  `session.tool_call` no longer invalidates, a card keeps the status of the last refetch.
- **The dock flashes "decide in the panel" on approval.**
  - `findHeldCall` returns the call `held_tool_call_id` names, whatever that call's own status
    (`heldCall.ts:206-226`).
  - After an approval, `session.tool_call(done)` and `session.updated(running)` are separate
    events. Between them the call is `done` with no staged content, so `heldDecisionSurface`
    answers `panel` and the dock renders `panel_hold`.

- **A new chapter shows no ghost.** A chapter starts with no paragraphs (`ChapterContent::default`,
  `services/chapters.rs:125`). `ChapterText.tsx:257` then renders `EmptyState`, and the ghost
  lives only in the other branch. The first sentence of a chapter therefore never streams. The
  empty state stays up until the held write's merged view replaces it, with a different height.

### 2.2 Why the page jumps

- **The lock banner.** `ChapterText.tsx:220-228` renders a bordered `bg-status-running/10` banner
  above the text only while the session is `running` (`locked`). It is about 34 px high, plus a
  12 px margin. It mounts when a step starts and unmounts when the step holds: twice per sentence.
- **Submit.**
  - `canSubmit = (drafting || revising) && !ownerRunning` (`ChapterHeader.tsx:91`): the button
    shows while held and hides while running (`:216-226`).
  - During a writer session it is the only thing in the action row (`:191`), so the header grows
    and shrinks by 32 px with it. That changes the height of the text's scroll area below.
- **The slide.**
  - Paragraphs are `motion.p layout` (`ChapterText.tsx`, `Paragraph`).
  - After an approval, the list remounts on `session.tool_call(done)`, and the banner mounts on
    `session.updated(running)`, one event later.
  - Motion then animates every paragraph down by the banner's height.

### 2.3 Spec drift

`08b0ee6` changed no spec. The following no longer describe the code:

- spec/10 §3, rows `session.updated`, `session.message` and `session.tool_call` (they say
  "invalidate");
- spec/11 §4.2 and §4.3 (invalidation and `clearSeq`);
- spec/03 §12:
  - the last sentence says the frontend discards a preview when the persisted call arrives;
  - the table puts an `insert_paragraphs` / `rewrite_paragraph` ghost under its target pid, which
    the preview cannot know (`GhostText.tsx` renders it as a block of its own);
- spec/11 §5.2:
  - it still describes a note above the merged view, which `d4716e6` removed;
  - it still says whole changed paragraphs are tinted, where `dac8c48` tints only the appended
    part.

## 3. Design

### 3.1 S1: the reveal

- `PendingText` marks what is new with one class, `pending-new`:
  - on every `added` paragraph;
  - on the appended `<span>` of a `changed` paragraph.

  The wrapper keeps `pending-text` as a hook for tests and has no animation.
- `prose.css` (replacing `pending-reveal` / `pending-amber`):

  ```css
  /* The staged text's entrance (D-174): from the ghost's ink and no tint to the element's own
     ink and tint. Only staged parts carry the class; confirmed text never animates. */
  .pending-new {
    animation: pending-reveal 180ms ease-out;
  }

  @keyframes pending-reveal {
    from {
      color: color-mix(in oklch, currentColor 45%, transparent);
      background-color: transparent;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .pending-new {
      animation: none;
    }
  }
  ```

  - The `from` keyframe is the ghost's look (`.ghost-text` uses the same `color-mix`).
  - With no `to` keyframe, the animation ends on the element's own computed values: full ink, and
    `bg-status-waiting/15` (added) or `/30` (appended span), as `dac8c48` intended.
  - No fill mode: there is no delay, so the first frame already shows `from`, and after the end
    the classes apply again. The fill `both` of `08b0ee6` is what pinned the span at 15 %.

### 3.2 S2 and S3: event writes

- One helper in `api/events.ts` does every event write:

  ```ts
  /** Writes an event into a cache. A fetch of the same key already in flight was requested
   *  before the event and may carry the state before it, so it is cancelled and requested again:
   *  any response that lands afterwards reflects the event (D-174). */
  function patchQuery<T>(queryClient: QueryClient, key: QueryKey, update: (cached: T) => T): void {
    const cached = queryClient.getQueryData<T>(key)
    if (cached !== undefined) {
      queryClient.setQueryData<T>(key, update(cached))
    }
    const inFlight = queryClient.isFetching({ queryKey: key, exact: true }) > 0
    if (inFlight) {
      void queryClient.cancelQueries({ queryKey: key, exact: true })
    }
    if (inFlight || cached === undefined) {
      void queryClient.invalidateQueries({ queryKey: key, exact: true, refetchType: "all" })
    }
  }
  ```

  - The explicit cancel matters when nothing is cached yet. `invalidateQueries` cancels an
    in-flight fetch (`cancelRefetch`) only when the query **has data**. Without data, it reuses the
    running fetch, so a first GET that started before the event would land as is (`query.js`,
    `fetch`).
  - `cancelQueries` reverts synchronously to the state before that fetch began. A `setQueryData`
    also moves that revert point to the state it wrote (`query.js`, the `success` reducer sets
    `#revertState` on a manual write), so the patch survives the cancel. The invalidation then
    starts a fresh fetch.
  - `refetchType: "all"` covers a query whose last observer unmounted while its fetch was in
    flight. A query that never had data and has no observer is skipped by TanStack, and the next
    mount fetches it fresh.
  - `exact: true` keeps the session's child keys (`…, "messages"`, `…, "messages-gap"`) out of
    it. Those keys hold older pages that an event never changes.
- `updateChapterOpenSession`, `updateSessionSnapshot`, `updateCachedToolCall` and the
  `chapter.content` write into `keys.chapters.detail(id)` all go through `patchQuery`.
  - The `chapter.content` case matters because `invalidateSession` refetches the chapter after
    every control mutation.
- `updateCachedToolCall` upserts the call into `tool_calls` **and** into the `tool_calls` of the
  `messages` item whose `message.id` equals `call.message_id`. When that message is not in the
  window, only the flat list changes.
- `session.message` keeps invalidating the snapshot: the event carries no tool calls, and the
  activity stream needs the new message. Under S2, a later event write re-issues that fetch
  whenever the fetch is still in flight.

### 3.3 S4: the preview's lifetime

- **Store** (`stores/live.ts`).
  - `Preview` gains `pinned?: { version: number }`.
  - `clearSeq(sessionId, seq, pinVersion?)`: when `pinVersion` is given, a preview of
    `insert_text`, `insert_paragraphs` or `rewrite_paragraph` in that seq is kept as
    `{ ...preview, pinned: { version: pinVersion } }`, and every other preview of the seq is
    dropped.
  - `clearPreview(sessionId, seq, callIndex)` stays. `clearSessionPreviews(sessionId)` stays.
  - `clearAllPreviews()` is new, for a reconnect.
- **Events.**
  - `session.message`: `flushLiveDeltas()`, `dropPending`, then
    `clearSeq(sessionId, seq, useChapterDoc.getState().version)`. The recorded version is
    meaningful only when the session writes the open chapter, and that is the only case where the
    preview is rendered.
  - `session.tool_call` whose `status` is `done` or `error`: `clearPreview(sessionId, seq,
    call_index)`. Approval, feedback, an error result and a stop all end this way.
  - `session.updated` whose status is neither `running` nor `held`: `clearSessionPreviews`. The
    `held` case no longer clears anything.
  - `chapter.content` clears nothing.
  - `onopen` after `onerror`, beside the existing `invalidateQueries()`: `clearAllPreviews()`.
    Events are not replayed, so a pinned preview could otherwise wait forever.
- **Rendering** (`useWriterState`).
  - While the session is `running`: the newest-seq preview of the session, live or pinned.
  - While it is `held`: only a pinned preview.
  - A pinned preview is returned only while `useChapterDoc.version === pinned.version`. When the
    write lands (`chapter.content` from a step pause, or with auto-pause off), the text arrives and
    the ghost goes in the **same** Zustand update, so in the same render.
  - A held write's merged view replaces the paragraph list, and the ghost is part of that list.
    The ghost therefore leaves in the same render as the merged view arrives, whichever store
    notified last.
  - `GhostText` renders a pinned preview without the caret (as today).
- **The rule behind it**: something on screen is removed only by a condition computed from the
  same store as its replacement, or in the same render as it. Removing it from the other store
  leaves a render with neither.

### 3.4 S5: the held call

- `useWriterState`: `held = found?.status === "held" ? found : undefined`, where `found` is the
  call `findHeldCall` returns.
  - `findHeldCall` itself is unchanged, because the agent panel shares it.
- Between `session.tool_call(done)` and `session.updated(running)`, the dock then sees a session
  marked `held` with no held call. It renders its running state, which is the state it is about to
  enter, instead of `panel_hold`.

### 3.5 S6: the status row

- The banner block (`ChapterText.tsx:220-228`) goes.
- The row under it (`mb-4 flex items-center justify-between`, the word count and **Full-text
  edit**) is the status row:

  | State | Left side |
  |---|---|
  | `locked` (the writer runs) | Lucide `Lock` (`size-3.5`, `text-status-running`) + `chapter:locked_live_words` |
  | a merged view is up | `chapter:pending_live_words` (unchanged) |
  | otherwise | `chapter:live_words` (unchanged) |

  - The left side is `flex min-w-0 items-center gap-1`, and its text is a `truncate` span with the
    full sentence in `title`, so it never wraps to a second line.
  - The row's height comes from the `size="sm"` button and is the same in every state.
  - `role="status"` moves from the banner to this text, so a screen reader still hears the lock.
- New key `chapter:locked_live_words`:
  - en: `The writer is working — pause to edit · {{words}} words in this version`
  - zh-CN: `写作者正在写，暂停后可修改 · 当前版本 {{words}} 字`
- `chapter:locked_banner` stays: `FullTextEditor` (`ChapterText.tsx:643-648`) still uses it, and
  that editor cannot be opened while the writer runs (§9).

### 3.6 S7: Submit

- `showSubmit = drafting || revising` and `submitBlocked = ownerRunning`.
- The button renders whenever `showSubmit` is true. It is disabled when `submitBlocked`, while a
  submit is pending, or when the chapter is empty.
- A disabled button gets no pointer events, so it is wrapped in a `span` that is the
  `TooltipTrigger` (the `WriterControls.tsx` pattern). The tooltip shows
  `chapter:submit_running_hint` while `submitBlocked`:
  - en: `The writer is running. Pause or stop it to submit.`
  - zh-CN: `写作者正在运行，暂停或停止后才能提交。`
- The submit rule is unchanged (spec/08 §1.1): a running session blocks it, and an open
  non-running one is ended after the confirmation.

### 3.7 S8: the first sentence

- `ChapterText` renders `EmptyState` only when the draft has no paragraphs **and**
  `writer.preview` is `undefined`.
- With no paragraphs and a preview, it renders the prose block with only the ghost (`GhostText`
  as a block of its own, since there is no tail to append to). The merged view then replaces that
  block as it replaces the list.

## 4. Tasks

| # | Task | Files | Done when |
|---|---|---|---|
| 26.1 | **Docs**: done with the plan (D-174). Re-read the amended sections against the code at `eca5a47`; fix any fact that disagrees and note it in D-174. | plan §7 | `check.sh` green (docs only) |
| 26.2 | **Events and previews**: `patchQuery` and its four users; the call upserted into `messages[].tool_calls`; the pinned version in `live.ts`; the clearing rules of §3.3 (`done`/`error`, not running or held, reconnect; none on `held` or `chapter.content`); `useWriterState` rendering a pinned preview by version, and returning only a `held` call. | `api/events.ts`, `stores/live.ts`, `features/chapter/useWriterState.ts` | §5.1 and §5.2 tests |
| 26.3 | **Reveal and layout**: `pending-new` in `PendingText` and `prose.css`; the status row replacing the banner; Submit disabled with its tooltip; the ghost on an empty chapter; the two keys in both locales. | `PendingText.tsx`, `styles/prose.css`, `ChapterText.tsx`, `ChapterHeader.tsx`, `i18n/locales/*/chapter.json` | §5.3 and §5.4 tests; `i18n:check` |
| 26.4 | **Smoke**: smoke item 21 with the Demo provider. Record what was seen in D-174. | PLAN §M11.5 | the checklist passes |

26.2 and 26.3 are independent. 26.2 fixes the blank and the stale state, and 26.3 fixes the flash
and the jumps.

## 5. Tests (Vitest)

### 5.1 Events (`api/events.test.ts`)

- **A stale fetch does not win.**
  - Seed a snapshot whose query function resolves from a list of deferred promises. Start a fetch
    (as `session.message` does).
  - Apply `session.tool_call(held)` and `session.updated(held)`. A second fetch has started.
  - Resolve the first with the pre-hold state, then the second with the held state. The cache ends
    held, and the pre-hold data is never visible after the patch.
  - Repeat for `keys.chapters.detail` with `chapter.content`.
  - The same with **nothing cached**: a first GET in flight when the event arrives is cancelled
    and requested again, and its pre-event response is never stored.
- `session.tool_call` updates the call inside `messages[].tool_calls` as well as `tool_calls`.
- **Pinning.**
  - `session.message` pins a writer preview with the open draft's version, and drops a
    non-writer preview of the same seq.
  - `session.updated(held)` keeps it.
  - `session.tool_call` `done` or `error` removes exactly that `(seq, call_index)`.
  - `session.updated` `stopped` (or any status other than running and held) removes the
    session's previews.
  - `chapter.content` removes none.
  - A reconnect removes all.

### 5.2 Pane (`features/chapter`)

- **`useWriterState`.**
  - A pinned preview is returned while running and while held, and not once the draft's version
    moved.
  - A live preview is not returned while held.
  - `held` is `undefined` when the named call is `done`.
- **The handoff never blanks** (`writerHandoff.test.tsx`, new).
  - Setup:
    - render `ChapterPage` with a real `QueryClient` and the endpoints stubbed;
    - drive the real `applyServerEvent` with one sentence's recorded event sequence:
      - `session.tool_preview` deltas;
      - `session.message`;
      - `session.tool_call` `pending`, `executing` and `held`;
      - `session.updated` `running` and `held`;
      - then, after Approve: `chapter.content`, `chapter.updated`, `session.tool_call(done)` and
        `session.updated(running)`;
    - run all timers between events, and `flushLiveDeltas()` after each preview delta (previews
      are coalesced per animation frame).
  - The assertion: a `MutationObserver` on the draft column checks after **every** mutation that
    the text streamed so far is in the column. After the last delta, that is the whole sentence,
    through approval.
  - Three variants:
    - the snapshot is cached;
    - it is not cached, and its GET resolves 50 ms later;
    - a GET started at `session.message` resolves with pre-hold data after the hold's events.
  - The same test asserts the dock never renders `panel_hold`.
- **The step-pause path.** With auto-pause off, `chapter.content` replaces the pinned ghost with
  the landed text in one mutation batch: the sentence appears exactly once in every observed
  state.

### 5.3 Reveal (`pendingPane.test.tsx`)

- Only `added` paragraphs and the appended span carry `pending-new`. No confirmed paragraph, and
  no wrapper, does.
- `prose.css` (imported with `?raw`) declares `pending-reveal` only in the `.pending-new` rule,
  and turns it off under `prefers-reduced-motion`.

### 5.4 Layout

- `ChapterText`, rendered in `running` and in `held`:
  - the elements before the first paragraph are the same (the status row only; no lock banner);
  - while running, the status text is `chapter:locked_live_words`, with `role="status"`.
- `ChapterHeader`, in a drafting chapter with a writer session `running`, then `held`: Submit is
  present in both, disabled with the hint while running, and enabled while held.
- An empty chapter with a running writer and a preview shows the ghost, not the empty state. With
  no preview, it shows the empty state.

## 6. Smoke

Smoke item **21. Steady draft pane (M26, plan §6)**, Demo provider, a chapter with an accepted
outline, granularity **sentence**, auto-pause **on**:

- Watch five sentences.
  - Each streams in grey with a caret, keeps its place without the caret when it finishes, then
    settles to black on amber in place.
  - The rest of the chapter never changes color.
  - Enter applies the sentence: the amber goes, and the text stays where it is.
- Throughout, the chapter header and the line above the text keep their height.
  - The line reads "The writer is working — pause to edit" while the writer runs.
  - Submit stays in the header, disabled while running.
  - The text does not move when a step starts or holds.
- Pin the agent panel to another session (its session list) and watch three more sentences: the
  same. The pane then loads the held call itself.
- Start a new chapter: its first sentence streams in place of the empty state.
- In the panel, each write's card shows **Done** after its approval, with no reload.
- With **Reduce motion** on in the OS, a held sentence appears with its tint and no fade.

## 7. Documents amended with this plan

- PROJECT:
  - §8.5: the header table (Submit with an open session); the Text tab (the lock moves into the
    word-count line; the ghost keeps its place; only the held text fades in);
  - F11: the lock line;
  - §10: #38;
  - §11: the participation row.
- spec/03 §12: the last sentence (a writer preview is kept until its replacement is on screen),
  and the table row of `insert_paragraphs` / `rewrite_paragraph` (a ghost block, not under the
  target).
- spec/10 §3: the `session.updated`, `session.message`, `session.tool_call` and `chapter.content`
  rows.
- spec/11:
  - §4.2: event writes and `patchQuery`, the preview rules, the reconnect;
  - §4.3: `live.ts` with the pinned version;
  - §5.2:
    - `ChapterText`'s status row, `GhostText` and `PendingText` (also correcting the drift of
      §2.3);
    - `useWriterState.held`;
    - the header's Submit;
  - §7: the reveal motion;
  - §9: the tests of §5.
- DECISIONS: D-174.
- PLAN: M26 and smoke item 21.
- README: M0–M26.

## 8. Risks

- **A turn with two write calls.** Only the first call's text streams, as today (one preview per
  seq). The second appears when it lands. Streaming both would need the pane to render several
  previews, which no writer turn needs in practice.
- **One more GET per event while a fetch is in flight.** It replaces a cancelled one, so the
  number of concurrent requests does not grow.
- **The tooltip on a disabled Submit** relies on the wrapping `span`. Without it, the hint never
  shows, but nothing breaks.

## 9. Out of scope

- The full-text editor's lock line. It can only appear if a session starts while that editor is
  open (its toggle is disabled while running), and it does not move the reading column.
- Agent highlights (`chapterDoc.highlights`). They are cleared by the `load` that follows every
  `chapter.content`, and they do not change layout.
- Any server change. The 8 ms between the persisted message and the hold is fine. The client must
  simply not lose the race.
