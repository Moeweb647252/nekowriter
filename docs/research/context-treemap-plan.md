# Implementation Plan: The context view (a treemap of what fills a session's context)

> Milestone **M23** in `docs/PLAN.md`. Read `AGENTS.md` first. The spec sections listed in §7 are
> amended together with this plan, and the decision is recorded as **D-171**. The spec is
> authoritative; this plan explains the reasons, the order and the tests. M23 comes after M22. It
> has no functional dependency on M21 or M22, but it must cover their session kinds and tools,
> and the milestones run in order.

## 0. What the author asked for, and the answers

The author's request: "我想做一个上下文 token 组成的矩形树图" (a treemap of the tokens that make up
the context). The footer shows only cumulative usage, so nobody can see *what* fills a session's
context: the settings block, the memory, twenty `read_chapter` results, or the model's own
replayed output.

| Question | Binding answer |
|---|---|
| A product feature or a debug tool? | **A product feature in the agent panel**, for every session ("智能体面板里，每个会话可看") |
| Which request does it show? | **The current context by default**, i.e. what the next request would send, **with every earlier request selectable** ("默认下一次请求，可回看每一步") |
| How is the first level grouped? | **By content type** ("按内容类型"): system prompt, tool definitions, kickoff, the author's input, runtime notices, model output and tool results. The second level splits each group, for example kickoff blocks or tool results by tool |

## 1. Decisions at a glance

| # | Decision |
|---|---|
| C1 | **One view per session**: a `ContextDialog` opened from a **Context** button. It is shown wherever a session's conversation is: the agent panel's `UsageFooter`, the conception studio's conversation header and the research task page's session section. It works for every session kind, including the book agent (M21), and for every status, including terminal ones. |
| C2 | **What it shows**: by default the **current context**, built from the persisted history as it stands. A **request strip** selects any earlier request. History is append-only and sealed results are immutable (spec/03 §1.3), so request *k* (the request that produced assistant message `seq = k`) is rebuilt exactly. It consists of the frozen system prompt, the frozen tools, every message with `seq < k`, and the sealed results of their calls. |
| C3 | **Counting**: each part is estimated with the existing heuristic (CJK characters × 1 + other characters ÷ 4), using **one function**, `nekowriter_llm::estimate_tokens`. The parts are then **scaled uniformly** so that a sent request totals the provider's prompt count for it (`input + cached + cache_write`). The current context uses the ratio of the latest measured request. No tokenizer is bundled: models differ, and the stack is fixed. Shares are exact proportions of the estimates; token counts are labeled as estimates unless they are a provider total. |
| C4 | **Groups, in a fixed order**: `system`, `tools`, `kickoff`, `author`, `runtime`, `model`, `results`. Each group splits into **parts** (a kickoff block, one tool's definition, one tool's results, …), and a part may list **items** (one result, one message). The full classification is in §3.2. |
| C5 | **Appendices leave the tool result**. A sealed result is the tool's text plus appendices (spec/03 §5.3). Each appendix is estimated on its own and counted where it came from: an author message, feedback or an edit report goes to `author`; an effect notice or a runtime notice goes to `runtime`. The tool keeps the rest. |
| C6 | **Reasoning counts only when it is replayed**: always for `anthropic` (its stored blocks are replayed verbatim), and for `openai` and `mock` only when `options.reasoning_replay = always` (spec/04 §2, §5). |
| C7 | **Kickoff blocks carry their prompt key.** `StoredBlock::Text` gains an optional `source`, which is never sent to a provider. It holds the block key (`block.settings`, …, or the task key such as `writer.kickoff.chapter`) on kickoff blocks. On the sections of an idle delivery it holds `edit_report`, `inbox.user_message` or `inbox.notice`. Rows written before M23 have none and fall back by message kind (§3.2). No migration is needed. Kickoff texts and request bytes do not change, so `kickoff_baseline.rs` stays as it is. |
| C8 | **One read-only endpoint**: `GET /sessions/:id/context?seq=`. Like every read, it uses the pool directly (spec/01 §7). It reads the messages first, then the calls, and ignores calls of messages it did not load, so a concurrent step cannot add half a turn. It never takes the write lock and never calls a provider. |
| C9 | **Bounded payload**: each part lists at most 40 items (largest first), and the rest are folded into `rest: {count, estimate}`. Excerpts are the first 80 characters with whitespace collapsed. The request list is complete, because each request costs a few numbers. |
| C10 | **Color by origin, identity by label**. A treemap puts any two groups side by side. Seven categorical hues fail the all-pairs checks (the worst normal-vision ΔE is 10.5 light and 9.8 dark; the worst CVD ΔE is 3.2 light and 1.6 dark), so groups are not told apart by color. Instead there are **three origin hues**: **setup** (system, tools, kickoff), **messages** (author, runtime) and **agent work** (model, results). These are the first three reference categorical slots, which pass all-pairs on the app's surfaces in both modes. Groups are named by in-fill labels and separated by a wider gap. Leaves keep areas proportional across groups, so no header band takes room. Every value is also in a list view. |
| C11 | **Live while open**: the current view refetches on that session's `session.message`, `session.tool_call` and `session.updated` events, at most once per second. A past request never changes, so its query is never refetched. The query key therefore sits outside `keys.sessions.detail(id)`, which every session event invalidates by prefix. |
| C12 | **Read-only and per session**: no trimming or editing of context (history is append-only), no book-wide aggregate, no cost, no jump from a cell to the conversation. |

## 2. Facts this plan rests on (code at `3f5d391`)

- `StoredBlock::Text` has `text` and an optional `cache` only (`crates/server/src/db/json.rs:893`).
  Kickoff blocks are rendered in key order, empty ones are dropped
  (`agent/context/mod.rs:443-473`), and the rollover kickoff has a second loop with the same rule
  (`agent/context/mod.rs:2180-2198`). The key is lost once a block is stored, and an empty block
  leaves no gap, so a stored kickoff cannot be mapped back to its keys.
- Idle deliveries build one message from sections (an edit report, then the inbox items) with
  `text_blocks` (`agent/runner.rs:2125`, callers at `:2102` and `:2284`). The message kind is
  `user` when any item is an author message and `notice` otherwise.
- `build_request` (`agent/context/mod.rs:1822`) sends the user blocks' text, the assistant
  `provider_payload`, and each call's sealed `result_text` in `call_index` order. It refuses a
  history with an unsealed call.
- Each assistant message stores its own `Usage` (`runner.rs:765`). Its `last_prompt_tokens` is
  `input + cached + cache_write` (`message_usage`, `runner.rs:2531`), so every sent request has a
  provider prompt total unless it is `estimated`.
- `seal::compose(raw_result_text, appendices)` (`agent/seal.rs:59`) is the pure composition of a
  delivered result, so an unsealed call's text can be previewed exactly as it would be sealed now.
- There are three copies of the heuristic:
  - `nekowriter_llm::openai::estimate_tokens` (`crates/llm/src/openai.rs:514`; its CJK set covers
    Han, kana, Hangul and full-width forms), with `estimate_usage` (`:529`), which skips assistant
    messages;
  - the Demo provider's own estimator (`crates/llm/src/demo.rs:1002`). The Demo marks its usage
    `estimated: true` (`demo.rs:248-255`), so a Demo session is never calibrated;
  - `kickoff_too_large` (`services/sessions.rs:550-578`), with a narrower private `is_cjk` that
    has no Hangul and no full-width forms.
- `ReasoningReplay` defaults to `Never` (`crates/llm/src/types.rs:562`). The Anthropic adapter
  replays stored blocks verbatim (spec/04 §5).
- `ScriptedProvider::requests()` (`crates/llm/src/mock.rs:138`) returns the requests it received.
  A test can therefore compare a rebuilt request with the one that was really sent.
- `kickoff_baseline.rs` pins kickoff **text** (`kickoff_text`, `:271`), not stored JSON, so a new
  optional field leaves every snapshot unchanged.
- Only `AgentPanel` renders `UsageFooter` (`AgentPanel.tsx:399`). `ConceptionStudio`
  (`:237` header) and `ResearchTaskPage` (`:135`) show an `ActivityStream` with no footer.
- The web has no chart library (`web/package.json`), and the stack is fixed (spec/01 §2). The
  shadcn chart tokens are grey steps (`web/src/index.css:81-85`). The surfaces are `--background`,
  `oklch(1 0 0)` light and `oklch(0.145 0 0)` dark (`#ffffff` and `#0a0a0a`).
- Query keys live in `web/src/api/keys.ts:97` (`sessions.list`, `sessions.detail`). Every
  `session.message`, `session.updated` and `session.tool_call` invalidates
  `keys.sessions.detail(id)` **by prefix** (`web/src/api/events.ts:470-500`), including the
  messages query under it. TanStack refetches an active invalidated query whatever its
  `staleTime`.
- Reads use the pool directly; no code path opens a read transaction (`db/mod.rs:83`, spec/01 §7).
- `rollover_needed` (`runner.rs:2406`) decides eligibility inline: `(ChapterWrite, Drafting)` or
  `(ChapterRevise, Revising)`. Its threshold is `projected > (window × 0.85) as i64`.
- A Radix `Dialog` closes on Escape unless `onEscapeKeyDown` prevents it.

## 3. Design

### 3.1 Data: `StoredBlock::Text.source`

```rust
Text {
    text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cache: Option<bool>,
    /// Where the runtime took this block from (D-171): a kickoff block's prompt key, or
    /// `edit_report` / `inbox.user_message` / `inbox.notice` for an idle delivery's sections.
    /// Display only; never sent to a provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<String>,
},
```

- Set by both kickoff loops (the block key) and by `text_blocks`, which now takes
  `(source, text)` pairs. The edit-report section gets `edit_report`, and each inbox item gets
  `inbox.<kind>`.
- Every other constructor passes `None`: assistant text, nudges, the re-review message, and the
  debug kickoffs.
- `build_request` and `MessageDto` ignore it. The TS binding gains `source?: string`.

### 3.2 The breakdown (`crates/server/src/agent/breakdown.rs`)

A pure function over loaded rows, `breakdown(session, messages, calls, seq) -> ContextBreakdown`,
and an async loader in `services/sessions.rs`. The loader reads the session, then its messages,
then its calls, through the pool (no transaction, C8). It drops calls whose message it did not
load.

**Which rows.** For `seq = k`, the rows are the messages with `seq < k` and the calls of those
messages. For the current context, they are every message and every call. In both cases the
frozen `system_prompt` and `tools` are included.

**Delivered text of a call.**
- A sealed call uses its `result_text`.
- In the current context, an unsealed call with a raw result or any appendix uses
  `seal::compose(raw_result_text, appendices)`.
- Any other call (pending, executing, held or awaiting input without a result) counts in
  `pending_calls` and adds nothing.

Unconsumed inbox rows are not counted; they are delivered at the next sealing.

**Classification.**

| Group | Part (`key`, `name`) | From | Items |
|---|---|---|---|
| `system` | `system_prompt` | `agent_sessions.system_prompt` | — |
| `tools` | `tool`, the tool name | each frozen `ToolDef`: name + description + `input_schema` as compact JSON | — |
| `kickoff` | `block`, the block's `source` (`null` on an old row) | each text block of the `kickoff` message, **in kickoff order** | — |
| `author` | `message` | text blocks by rule (a) below; `inbox` appendices whose `inbox_kind` is `user_message` or absent (D-165) | per block / appendix |
| `author` | `feedback` | `user_feedback` appendices | per appendix |
| `author` | `edit_report` | `edit_report` appendices; text blocks by rule (a) | per appendix / block |
| `runtime` | `notice` | text blocks by rule (a); `inbox` appendices whose `inbox_kind` is `notice` | per block / appendix |
| `runtime` | `effect_notice` | `effect_notice` appendices | per appendix |
| `runtime` | `nudge` | text blocks by rule (a) | per block |
| `model` | `text` | assistant text blocks | per message |
| `model` | `reasoning` | assistant reasoning blocks, only when replayed (C6) | per message |
| `model` | `call`, the tool name | tool name + `input_text` of each tool-call block | per call |
| `results` | `result`, the tool name | the call's delivered text, minus the estimate of each of its appendices, never below 0 | per call |

**(a) Text blocks of a non-kickoff user-side message** are classified by `source` first:
`edit_report` → `author.edit_report`, `inbox.user_message` → `author.message`, `inbox.notice` →
`runtime.notice`. A block without `source` is classified by its message kind: `user` →
`author.message` (an old row's edit report lands here too), `notice` → `runtime.notice`, `nudge` →
`runtime.nudge`.

Empty parts and groups are omitted. Within a group the parts are ordered largest first, except
`kickoff`, which keeps the kickoff order so the UI can number unlabeled blocks.

**Estimate and scale.**
- `estimate` is the sum of the leaves. Every leaf uses `nekowriter_llm::estimate_tokens`.
- For `seq = k`: when message *k* has a `usage` with `estimated = false` and a prompt total
  `p > 0`, then `measured = p`, `cached = usage.cached_input_tokens`, `scale = p / estimate` and
  `calibrated = true`. Otherwise `measured` and `cached` are null, `scale` is 1, and `calibrated`
  is false.
- For the current context: `measured` and `cached` are null, and `scale` is the ratio of the
  latest calibrated request of the session (with `calibrated = true`), else 1.

**Other fields.**
- `rollover_at = floor(0.85 × context_window) − max_output_tokens` when the session, in its
  current kind and phase, may roll over (spec/03 §9); `null` otherwise. Eligibility is extracted
  from `rollover_needed` into `rollover_eligible(session)`, and both use it.
- `requests` lists every assistant message:
  - `seq`, `prompt_tokens` (`usage.last_prompt_tokens`), `cached_tokens`, `estimated`,
    `created_at`. A message without `usage` reports 0 and `estimated`.
  - `estimate`: the heuristic total of that request, as its own breakdown would report it. It is
    computed with running sums in the same pass, so the list stays linear in the history.

  An estimated provider count, such as the Demo's, skips assistant messages (`estimate_usage`).
  It therefore disagrees with the breakdown, and the strip draws `estimate` for those requests
  (§3.4).
- Items: `{seq, call_index, estimate, excerpt}`, largest first, at most 40 per part, plus `rest`.

**One estimator.** `estimate_tokens` is re-exported as `nekowriter_llm::estimate_tokens`.
`kickoff_too_large` calls it and its private `is_cjk` is removed. As a result the large-kickoff
warning now also counts Hangul and full-width characters as one token each, which D-171
records. The Demo estimator stays as it is, because it models the Demo provider and not the app.

### 3.3 API

| Method | Path | Body / query | Returns |
|---|---|---|---|
| GET | `/sessions/:id/context` | `?seq` (an assistant message's `seq`; absent = the current context) | `ContextBreakdownDto`. An unknown session → `404 not_found`; a `seq` that is not an assistant message of the session → `400 validation.failed` (`details.fields.seq`) |

```ts
type ContextBreakdownDto = {
  session_id: number;
  seq: number | null;                 // the request that produced this assistant message; null = current
  context_window: number; max_output_tokens: number;
  rollover_at: number | null;         // prompt size at which this session renews (spec/03 §9)
  estimate: number;                   // heuristic tokens over every leaf
  measured: number | null;            // the provider's prompt total for this request
  cached: number | null;              // its cached input tokens
  scale: number;                      // provider tokens per estimated token; 1 when uncalibrated
  calibrated: boolean;
  pending_calls: number;              // current context: calls with no result yet (not counted)
  groups: ContextGroupDto[];          // fixed order (C4); empty groups omitted
  requests: ContextRequestDto[];      // every request of the session, oldest first
};
type ContextGroupKey = "system" | "tools" | "kickoff" | "author" | "runtime" | "model" | "results";
type ContextPartKey = "system_prompt" | "tool" | "block" | "message" | "feedback" | "edit_report"
  | "notice" | "effect_notice" | "nudge" | "text" | "reasoning" | "call" | "result";
type ContextGroupDto = { key: ContextGroupKey; estimate: number; parts: ContextPartDto[] };
type ContextPartDto = {
  key: ContextPartKey;
  name: string | null;                // tool name (tool, call, result) or block source (block)
  estimate: number;
  items: ContextItemDto[];            // largest first, at most 40
  rest: { count: number; estimate: number } | null;
};
type ContextItemDto = { seq: number; call_index: number | null; estimate: number; excerpt: string };
type ContextRequestDto = {
  seq: number; prompt_tokens: number; cached_tokens: number; estimated: boolean;
  estimate: number;                   // the heuristic total of that request (its breakdown's estimate)
  created_at: string;
};
```

### 3.4 Web (`web/src/features/agent/context/`)

**Entry points.** `ContextButton` (Lucide `LayoutGrid`, label **Context**) opens `ContextDialog`
for a session id. It appears in:
- `UsageFooter`, at its end. Once the session has sent a request, the button shows
  `usage.last_prompt_tokens` as a share of `SessionDto.context_window`, a new field copied from
  `model_snapshot`;
- the conception studio's conversation header;
- the research task page's session section.

**`ContextDialog`** (shadcn `Dialog`, `max-w-5xl`), from top to bottom:
1. **Header line**:
   - "≈ 42,180 tokens · 33% of 128,000". A provider total drops the "≈".
   - The source: **Provider count** for a calibrated past request; **Estimate, scaled to the
     provider's last count** for a calibrated current view; **Estimate** otherwise.
   - "Cached 30,000" for a past request that has it.
   - "2 tool calls have no result yet and are not counted" when `pending_calls > 0`.
2. **`RequestStrip`**:
   - One column per request, then a **Current** column, which is outlined because it is an
     estimate.
   - Column height is `prompt_tokens` for a measured request and `estimate` for an estimated
     one, so a column always matches the header of its own view. The **Current** column is
     `round(estimate × scale)`. The scale tops at `max(context_window, the largest column)`. A
     hairline marks the window and a dashed line marks `rollover_at` (**Renews here**).
   - Columns have 4 px rounded tops anchored to the baseline and 1 px gaps. They are at least
     3 px wide; past that, the strip scrolls inside its own box and keeps the selected column in
     view.
   - Unselected columns use the muted ink, and the selected column the foreground ink.
   - Hover tooltip: "Step 12 · 42,180 tokens · cached 30,000 · 14:02".
   - Keyboard: one tab stop with `role="slider"`. Left, Right, Home and End move the selection,
     and `aria-valuetext` says "Step 12 of 40, 42,180 tokens".
3. **Breadcrumb** ("Context › Tool results › read_chapter") and a **Treemap | List** toggle (the
   last choice is remembered in the UI store).
4. **`ContextTreemap`** (SVG, 420 px tall):
   - Layout:
     - groups are laid out first (squarified, `lib/treemap.ts`), then their leaves inside;
     - a leaf is an item (or the folded `rest`) when its part has items, otherwise the part;
     - groups are separated by a 4 px gap and leaves by a 2 px gap. The leaves fill their
       group's whole box: no header band takes room, so areas stay proportional across groups.
   - Labels (white, inside the fill, never clipped; measured with 6 px padding):
     - a group's name and share, in bold, at the top-left of its box, when they fit in the
       group's top-left leaf; that leaf's own label then goes on the next lines;
     - a leaf's name, then its share, when they fit;
     - anything that does not fit is left to the tooltip, the legend and the list view.
   - Measuring: text goes through an injectable function (canvas `measureText` by default), and
     the width comes from a `ResizeObserver`, so jsdom tests pass fixed values.
   - Colors: the group's origin token (§3.5).
   - Interaction:
     - hover or focus rings the cell (2 px, foreground) and shows a tooltip with the group, part,
       item ("Message 12 · call 2"), "≈ tokens", share and excerpt;
     - click zooms one level: a group, then a part with items;
     - **Escape** or the breadcrumb zooms out. While zoomed, the dialog's `onEscapeKeyDown`
       prevents closing and zooms out instead. At the top level, Escape closes the dialog.
   - Keyboard: one tab stop. Arrow keys move between leaves in size order, **Enter** zooms, and
     **Escape** zooms out.
   - When the request changes, the zoom path is kept if it still exists, otherwise the view
     returns to the top.
   - Under reduced motion, zooming does not animate.
5. **List view**: a table with Name, ≈ Tokens and Share. Group rows expand to parts, and parts to
   items (40 plus "n more"). Rows are sorted largest first; kickoff blocks keep the kickoff order.
6. **Legend**: three swatches with the groups each covers ("Setup: system prompt, tools,
   kickoff", …).

**Labels.**
- Groups and part keys come from `agent:context.group.<key>` and `agent:context.part.<key>`.
- A `tool`, `call` or `result` part shows its tool name as is.
- A `block` part maps the known `block.*` keys to `agent:context.block.<name>`, where `<name>` is
  the key without its `block.` prefix (i18next splits keys on dots). The names are settings,
  canon_systems, writing_skills, memory, recent_chapters, goals, chapter_draft, cast_index,
  chapter_diff, book_brief and book_index. `assistant.kickoff` reads **First message**, because
  the book agent's kickoff wraps the author's first message. Any other key reads **Task**, and an
  unlabeled block is "Block *n*" by its kickoff position.
- Display tokens are `round(estimate × scale)`, formatted with `formatNumber`. A share is always
  of the whole request (`estimate / total estimate`), also when zoomed.

**Data.**
- `keys.sessions.context(id, seq | "current")` = `["sessions", "context", id, seq | "current"]`.
  It is **not** under `keys.sessions.detail(id)`: the session handlers invalidate that prefix on
  every event (§2), which would refetch past views and bypass the coalescing.
- The strip always reads `requests` from the current query, so it stays fresh while a past
  request is shown.
- The current query is invalidated by that session's `session.message`, `session.tool_call` and
  `session.updated` events, coalesced to one refetch per second, and only while the dialog is
  open (an inactive query is not refetched).
- Past queries have `staleTime: Infinity`.

### 3.5 Colors

The three origin tokens are defined in `index.css` for both themes and exposed through
`@theme inline`. They are the reference palette's first three categorical slots, checked with the
dataviz validator using `--pairs all` against the app's surfaces:

| Token | Light | Dark | Groups |
|---|---|---|---|
| `--context-setup` | `#2a78d6` | `#3987e5` | system, tools, kickoff |
| `--context-messages` | `#eb6834` | `#d95926` | author, runtime |
| `--context-work` | `#1baf7a` | `#199e70` | model, results |

- **Light** (`#ffffff`): every check passes. The worst CVD ΔE is 9.2 and the worst normal-vision
  ΔE is 24.0. The aqua is 2.82:1 against the surface, so the relief rule applies: visible labels
  and the list view.
- **Dark** (`#0a0a0a`): every check passes. The worst CVD ΔE is 9.4 and the worst normal-vision
  ΔE is 20.9.
- Labels inside a fill are `#ffffff` in both modes. Other text uses text tokens, never a group
  color.

## 4. Tasks

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 23.1 | **Docs**: done with the plan (D-171). Re-read the amended sections; fix any a code fact contradicts and note it in D-171. | plan §7 | `check.sh` green (docs only) |
| 23.2 | **Block sources and one estimator**: `StoredBlock::Text.source`; both kickoff loops and `text_blocks` set it; every other constructor passes `None`; binding. Re-export `estimate_tokens`; `kickoff_too_large` uses it and drops its `is_cjk`. | 02 §4; 03 §10; 04 §2 | plan §5.1; `kickoff_baseline` unchanged |
| 23.3 | **Breakdown and API**: `agent/breakdown.rs` (pure), the loader (ordered pool reads, C8), `rollover_eligible`, `GET /sessions/:id/context`, the DTOs, `SessionDto.context_window`, bindings. | 03 §13.1; 10 §2.7, §2.8 | plan §5.2 `context_breakdown.rs` |
| 23.4 | **Web**: `lib/treemap.ts`; `ContextButton`, `ContextDialog`, `RequestStrip`, `ContextTreemap`, the list view; the three entry points; the origin tokens; the query key and the SSE invalidation; both locales. | 11 §4.2, §5.3, §5.4, §7, §9; PROJECT §8.6 | plan §5.3 Vitest; `i18n:check` |
| 23.5 | **Smoke**: smoke item 18 with the Demo provider. | PLAN §M11.5 | the checklist passes |

## 5. Tests

### 5.1 Block sources (in `agent_creation.rs` and `agent_write_rollover.rs`)
- For every session kind, each kickoff block's `source` is its key, in kickoff order, with empty
  blocks dropped. The rollover child's blocks carry theirs too.
- An idle delivery with an edit report, an author message and a notice stores three blocks with
  `edit_report`, `inbox.user_message` and `inbox.notice`.
- The serialized request has no `source` (assert on the `ScriptedProvider` request).
- A stored block without `source` still deserializes.
- `kickoff_too_large` counts a Hangul text like the provider estimate.

### 5.2 `crates/server/tests/context_breakdown.rs`
- **Shape**: a scripted writer session covers:
  - kickoff blocks;
  - `read_chapter` and `query_entity` calls;
  - an author message delivered as an `inbox` appendix;
  - an edit report appendix, an effect notice and a nudge;
  - an idle author message.

  Every group and part of §3.2 appears with its name. Parts sum to their group, and groups sum to
  `estimate`.
- **Rebuild**: for the request that produced each assistant message *k*, the breakdown's leaf
  texts (exposed by the pure function before estimation) match the request the
  `ScriptedProvider` received:
  - the same system prompt and tool names;
  - the same user-block texts and tool-result texts, in order;
  - assistant parts from exactly the assistant messages before *k*.

  A mock replay payload cannot be estimated like a provider's, so the test compares texts and
  not totals.
- **Request list**: each `requests[i].estimate` equals the `estimate` of its own `seq` view.
- **Scale**:
  - a request with measured usage gives `measured` (its prompt total), `scale = measured /
    estimate` and `calibrated`;
  - the current context uses the latest measured ratio;
  - a session with only estimated usage (Demo) gives scale 1, not calibrated.
- **Pending**: a held call without a result counts in `pending_calls` and adds nothing. A call
  with a raw result that is not sealed yet counts with `seal::compose`. Neither changes a past
  request.
- **Appendices**: a sealed result's appendices move to `author` / `runtime`, and the tool keeps
  the rest, never below 0.
- **Replay rule**: reasoning is counted for `anthropic`, for `openai` with `always`, and not for
  `openai` with `never`.
- **Old rows**: a kickoff without `source` gives `block` parts named `null` in order. `user` and
  `notice` messages without `source` are classified by kind.
- **Caps**: 45 results of one tool give 40 items and `rest {5, sum}`. Excerpts are at most 80
  characters with whitespace collapsed.
- **Rollover point**: set for a drafting writer, `null` for a revising writer still in
  `revision_planning`, for a reviewer and for the book agent.
- **Reads**: a call inserted after the messages were read is ignored.
- **API**:
  - `?seq` that is a user message or does not exist → `400 validation.failed` with `fields.seq`;
  - an unknown session → 404;
  - a JSON snapshot (insta) of one response;
  - the bindings exist.

### 5.3 Vitest
- `lib/treemap.ts`:
  - areas proportional to weights (within 1 px²);
  - rects inside the bounds and not overlapping;
  - zero weights skipped;
  - deterministic;
  - the reference case of Bruls et al. (weights 6, 6, 4, 3, 2, 2, 1 in a 6 × 4 box) yields the
    paper's layout.
- `ContextDialog`:
  - group labels render where they fit;
  - the header line for a provider count, a scaled estimate and a plain estimate;
  - the pending note;
  - selecting a strip column fetches `?seq=` and the strip keeps reading the current query;
  - keyboard on the strip;
  - zoom in and out (click, Enter, Escape, breadcrumb), and the zoom path kept across requests;
  - Escape while zoomed zooms out without closing; at the top it closes;
  - an estimated request's column uses `estimate`;
  - the list view toggles, sorts, and folds "n more";
  - "Block 2" for an unlabeled block, **First message** for `assistant.kickoff`, and **Task** for
    another task key;
  - a group label appears only when it fits, and shares stay of the whole when zoomed.
- Entry points: the footer button with the window share, the conception header and the research
  page.
- Invalidation: a `session.message` for the session invalidates its current query, and one for
  another session does not. A past view's query is not refetched by any session event.
- Every `ContextGroupKey`, `ContextPartKey` and known block key has keys in both locales (dynamic
  keys, invisible to `check-i18n`).

## 6. Smoke item 18 (PLAN §M11.5)

- Demo, a book with an approved chapter 1: outline chapter 2, approve the outline, and start
  writing it. Open **Context** from the footer. The setup groups fill most of the view: the
  system prompt, the tools, and a kickoff with **Setting materials**, **Memory**, **Recent
  chapters**, **Chapter draft** and **Task**. The header says **Estimate**.
- After a few steps: **Model output** and **Tool results** grow, and the strip gains columns.
  Select an early column; the treemap shows less, and the current column stays outlined.
- Zoom into **Tool results**, then into a tool; Escape returns, and Escape at the top closes the
  dialog. Switch to **List**: the same numbers appear.
- Send a steering message while the agent runs; after its delivery it appears under **From the
  author**.
- Open **Context** in the conception studio and on a research task page.

## 7. Amended documents (with this plan)

- `docs/PROJECT.md`:
  - §6.19 Context view (new);
  - §8.6 Footer;
  - §8.7 (the studio and research-page entries);
  - §9.2 (out of scope);
  - §10 #36;
  - §11 row.
- `docs/spec/01-architecture.md` §3.1: `agent/breakdown.rs`.
- `docs/spec/02-data-model.md` §4: `StoredBlock.source`.
- `docs/spec/03-agent-runtime.md`: §10 (kickoff blocks store their key), §13.1 (new), §15 (tests).
- `docs/spec/04-llm.md` §2: the exported estimator.
- `docs/spec/10-api.md`: §2.7 (the endpoint), §2.8 (the DTOs and `SessionDto.context_window`).
- `docs/spec/11-frontend.md`:
  - §2 (`lib/treemap.ts`);
  - §4.2 (invalidation);
  - §5.3 (the footer entry, the context view and its three entry points);
  - §5.4 (the studio entry);
  - §7 (origin tokens);
  - §9 (Vitest).
- `docs/PLAN.md`: M23 and smoke item 18. `docs/README.md`: M0–M23.
- `docs/DECISIONS.md`: D-171.

## 8. Risks

- **Estimate drift by content type.** JSON schemas and Chinese prose tokenize differently, so a
  uniform scale gets the total right but can misstate one part by a few percent. The UI calls the
  numbers estimates, and the list view shows shares, which do not depend on the scale.
- **Provider differences.** Some models drop earlier reasoning from the context even though
  NekoWriter sends it. C6 counts what NekoWriter sends, and the scale absorbs the difference in
  the total.
- **Large sessions.** A long writer session can have hundreds of calls. The breakdown is linear
  in the history, items are capped, and the current query is coalesced. If a session is slow to
  load, the strip and the treemap keep their old data while the query refetches.
- **Old rows.** Kickoffs from before M23 show "Block *n*". This is accepted; no backfill.

## 9. Out of scope

- Editing, trimming or summarizing context from the view.
- A book-wide aggregate, a cross-session comparison, and cost in currency (PROJECT §9.2).
- Exact per-part counts from a bundled tokenizer or a provider token-count call.
- Jumping from a cell to its message in the conversation, and exporting the view.
