# 11 — Frontend

## 1. Scaffold (exact commands, verified with shadcn CLI 4.22.1 and pnpm 12)

Run from the repository root:

```bash
pnpm dlx shadcn@4.22.1 init -t vite -n web --preset nova --base radix --no-monorepo
rm -rf web/.git                      # the Vite template initializes a nested repository; remove it
cd web
pnpm dlx shadcn@4.22.1 add alert alert-dialog badge breadcrumb button card checkbox collapsible \
  command dialog dropdown-menu empty input label popover progress radio-group resizable \
  scroll-area select skeleton sonner spinner switch table tabs textarea toggle \
  toggle-group tooltip
pnpm add motion zustand @tanstack/react-query react-router i18next react-i18next \
  react-markdown remark-gfm diff @tanstack/react-virtual \
  @dnd-kit/core @dnd-kit/sortable @dnd-kit/utilities
pnpm add -D vitest @testing-library/react @testing-library/user-event @testing-library/jest-dom jsdom
```

The scaffold provides `components.json` with `"style": "radix-nova"`, `"iconLibrary": "lucide"`,
Tailwind 4 (`@tailwindcss/vite`), the `@/` alias, a `ThemeProvider` (light/dark/system),
Geist Variable, ESLint and Prettier. Its components import `cn` from the `cn` package
(shadcn's replacement for clsx + tailwind-merge; `src/lib/utils.ts` re-exports it). Keep all of
these. If a component name above does not exist in the registry, skip it and note it in
`docs/DECISIONS.md`.

`package.json` scripts (add or adjust):
```json
{
  "dev": "vite",
  "build": "tsc -b && vite build",
  "typecheck": "tsc -b",
  "lint": "eslint .",
  "format": "prettier --write \"src/**/*.{ts,tsx,css,json}\"",
  "test": "vitest",
  "i18n:check": "node scripts/check-i18n.ts"
}
```
The scaffold's own `"typecheck": "tsc --noEmit"` checks nothing, because the root
`tsconfig.json` is a solution file (`"files": []` plus references). `tsc -b` checks every
referenced project, and those already set `noEmit`. Node ≥ 22.18 runs `.ts` scripts with
built-in type stripping (older Node needs `--experimental-strip-types`).

`vite.config.ts`: keep the scaffold. Add
`server: { port: 5173, proxy: { "/api": { target: "http://127.0.0.1:7878", changeOrigin: false } } }`
and a Vitest `test: { environment: "jsdom", setupFiles: ["src/test/setup.ts"] }`.

## 2. Directory structure

```
web/src/
├── main.tsx                     # i18n init, providers, router
├── app/
│   ├── providers.tsx            # ThemeProvider → QueryClientProvider → MotionConfig → TooltipProvider → Toaster
│   ├── router.tsx               # createBrowserRouter (react-router 8, data mode), lazy route modules
│   └── shell/                   # AppShell, TopBar, LanguageSwitch, ThemeToggle, QuickSwitch (⌘K)
├── api/
│   ├── gen/                     # ts-rs output (generated; committed; never edited by hand)
│   ├── client.ts                # fetch wrapper → ApiError {status, code, message, details}
│   ├── keys.ts                  # query-key factory
│   ├── endpoints/               # typed functions per resource (books.ts, chapters.ts, sessions.ts, …)
│   ├── hooks/                   # useQuery/useMutation hooks per resource
│   └── events.ts                # SSE client: dispatch to stores + query invalidation
├── stores/
│   ├── ui.ts                    # persisted UI prefs (zustand/persist → localStorage)
│   ├── live.ts                  # streaming buffers, tool previews, SSE connection state
│   └── chapterDoc.ts            # open chapter content + ops application + highlights
├── features/
│   ├── library/  conception/  workspace/  overview/  chapter/  agent/  canon/
│   ├── settings-materials/  proposals/  knowledge/  research/  prompts/  skills/  settings/
├── components/
│   ├── ui/                      # shadcn (generated)
│   └── common/                  # Markdown, StatusBadge, EmptyState, ConfirmDialog, WordCount, Hotkey, DiffView
├── i18n/
│   ├── index.ts                 # i18next init, language detection, formatters
│   └── locales/{en,zh-CN}/{common,library,book,chapter,agent,canon,knowledge,prompts,skills,settings,errors}.json
├── lib/                         # utils (cn), wordcount.ts, format.ts, text.ts (pid helpers), hotkeys.ts
│                                # and treemap.ts (squarified layout, D-171)
├── styles/                      # prose.css (reading column), status tokens
└── test/setup.ts
```

## 3. Routing

| Path | Module | Notes |
|---|---|---|
| `/` | `library/LibraryPage` | every status: `active` and `conceiving` always, `archived` when "Show archived" is on; a conceiving card opens `books/:bookId/conception`, every other card `books/:bookId` (D-129) |
| `/knowledge`, `/knowledge/scopes/:scopeId`, `/knowledge/scopes/:scopeId/entries/:entryId` | `knowledge/*` | |
| `/research/:taskId` | `research/ResearchTaskPage` | |
| `/prompts` | `prompts/PromptsPage` | global |
| `/skills` | `skills/SkillsPage` | global writing-skill catalogue (design §8) |
| `/settings/:section?` | `settings/SettingsPage` | providers, models, roles, search, embeddings, appearance |
| `/books/:bookId` | `workspace/BookLayout` (3 panels) | child routes below |
| `  index` | `overview/OverviewPage` | |
| `  conception` | `conception/ConceptionStudio` | full width (the agent panel is collapsed by default) |
| `  chapters/:chapterId` | `chapter/ChapterPage` | `?tab=` one of `text`, `outline`, `review`, `history` |
| `  settings/:doc` | `settings-materials/*` | `outline`, `world`, `protagonist`, `characters` |
| `  canon/entities/:entityId?`, `canon/timeline`, `canon/memory`, `canon/foreshadowing`, `canon/goals`, `canon/changes` | `canon/*` | `canon/goals?filter=ready&node=<id>` (spec/06 §11). A disabled system's page stays routable (spec/06 §10.3) |
| `  proposals` · `knowledge` · `prompts` · `tracks/:trackId` · `book-settings` | | |

Use `createBrowserRouter` from `react-router` and `RouterProvider` from `react-router/dom`. Route
modules are lazy (`lazy: () => import(...)`). An error element per layout shows a friendly
message and a retry.

## 4. Data layer

### 4.1 REST
- `client.ts`: `request<T>(method, path, body?)`, base `/api`. Non-2xx responses throw `ApiError`
  parsed from `{error: {code, message, details}}`. Network failures throw
  `ApiError{code: "network"}`.
- Query keys: `keys.books.all`, `keys.books.detail(id)`, `keys.chapters.list(trackId)`,
  `keys.chapters.detail(id)`, `keys.sessions.list(bookId, filters)`, `keys.sessions.detail(id)`,
  `keys.canon.entities(bookId, filters)`, `keys.canon.goals(bookId, filters)`,
  `keys.canon.counts(bookId)`, … (one factory, no ad-hoc arrays). The goal keys belong to the
  canon family on purpose: `canon.changed` already refreshes readiness.
- QueryClient defaults: `staleTime: 30_000`, `refetchOnWindowFocus: false` (SSE keeps data
  fresh), queries `retry: 1`, mutations `retry: 0`. Mutations invalidate their resource keys on
  success. Errors show a toast through `errors.json` (`t("errors:" + code, {defaultValue: message})`).

### 4.2 SSE (`api/events.ts`)
- One `EventSource("/api/events?book_id=…")` while a book is open (otherwise without a filter),
  re-created when the book changes.
- Dispatch table by event type (spec/10 §3): update stores, upsert into query caches with
  `queryClient.setQueryData` where the payload is complete (sessions, tool calls, chapter list
  items), and invalidate where it is a signal (canon, settings, knowledge).
- `session.updated` invalidates the chapter that embeds the session as well as the session itself:
  `keys.chapters.detail(session.chapter_id)` when the payload's `SessionDto` has a chapter, and no
  chapter when it does not (conception, research, canon seed). `ChapterDto.open_sessions` is the
  chapter pane's only handle on its writer session and the chapter query is the only place it comes
  from, so without this an **agent**-driven hold (`auto_pause`, `pause_after_outline`) left the
  cached session `running`, `useWriterState` never subscribed to the snapshot (it does so only for a
  `held` session), and the dock plus the merged preview were missing until a reload — while the
  agent panel, which resolves the hold from its own snapshot, correctly said to decide in the dock.
  Author-driven changes already refreshed the chapter (`invalidateSession` in
  `api/hooks/sessions.ts`); this is the same invalidation for the event path (D-144). The scope is
  one chapter key: no blanket invalidation, so a research or conception event cannot refetch the
  open chapter.
- `chapter.updated` also invalidates `keys.canon.goals(book_id)` and `keys.books.overview(book_id)`,
  because an approval can make a `chapter_at_least` condition true (spec/06 §11.3). No goal page
  ever needs a manual refresh.
- `session.message`, `session.tool_call` and `session.updated` invalidate
  `keys.sessions.context(session_id, "current")`, coalesced to one refetch per second. Only an open
  context view has that query active, so a closed one costs nothing. A past request's query
  (`keys.sessions.context(id, seq)`) never changes and has `staleTime: Infinity` (D-171).
  - The key is `["sessions", "context", id, seq | "current"]`, deliberately **outside**
    `keys.sessions.detail(id)`. Those handlers invalidate the detail prefix on every event, and
    TanStack refetches an active invalidated query whatever its `staleTime`, so a key under it
    would refetch past views and skip the coalescing.
- Deltas and previews are coalesced with `requestAnimationFrame` before they reach the store.
- After a reconnect (`onopen` following an `onerror`), invalidate all active queries
  (`queryClient.invalidateQueries()`) and reload the open chapter document.
- Connection state (`connecting | open | reconnecting`) is shown as a small dot in the top bar.

### 4.3 Zustand stores
- `ui.ts` (persisted): `editorFont: "serif" | "sans"`, `editorSize: "s" | "m" | "l"`,
  `navigatorCollapsed`, `agentPanelCollapsed`, `pinnedSession: Record<bookId, sessionId | null>`,
  `pinnedSessionManual: Record<bookId, boolean>` (D-167), `language`. Theme is owned by the
  scaffold's `ThemeProvider`.
- `live.ts`: `streams: Record<sessionId, { seq, text: Record<index, string>, reasoning: Record<index,
  string> }>`, `previews: Record<"sessionId:seq:callIndex", { name, field, text }>`. Actions
  `applyDelta`, `applyPreview`, `clearSeq(sessionId, seq)` (called when `session.message` for that
  seq arrives).
- `chapterDoc.ts`: `{ chapterId, version, paragraphs: {pid, text}[], tailOpen, highlights:
  Record<pid, {kind: "inserted" | "updated", at}> }`. Actions `load(dto)` and
  `applyOps(version, ops, actor) → "applied" | "gap"`. A gap triggers a refetch of the chapter.
  The store applies its own mutation responses and ignores duplicate SSE versions.

## 5. Key components

### 5.0 Library
- `LibraryPage` fetches `GET /books` **without** a `status` filter and hides `archived` rows in the
  browser unless "Show archived" is on. The server's filter is exact, so asking for `active` also
  excluded books still in conception and made them unreachable from the library between creation
  and finalize (D-129).
- `BookCard` shows a `conceiving` badge plus a one-line explanation and a "Continue conception"
  link to `/books/:bookId/conception`. The chapter/word counts and the latest-chapter line stay
  visible for a conceiving book — it may already own a chapter created before D-130 gated chapter
  creation on finalize — so the conceiving block is added, not substituted. Clicking its title or
  its "Open" menu item goes to the same studio; every other status keeps opening the overview.
- `NewBookDialog` (D-158): under "Start with the conception agent", an indented checkbox "Start
  conception automatically when the research before it finishes" (D-168 wording: the check and its
  topics; default on, rendered only while conception is on). With conception on, the request sends
  `start_conception: true`, `knowledge_check: true` and `auto_start_conception`. Landing: a
  `conceiving` book opens `/books/:id/conception`, any other book `/books/:id`. The landing no
  longer depends on `session_id`.

### 5.1 Workspace
- `BookLayout`: `ResizablePanelGroup` (shadcn wrapper over react-resizable-panels v4
  `Group`/`Panel`/`Separator`) with ids `nav`, `main`, `agent`. Sizes: nav `defaultSize="240px"`
  `minSize="200px"` `collapsible`; agent `defaultSize="380px"` `minSize="320px"` `collapsible`; main
  takes the rest. Persist with `const { defaultLayout, onLayoutChanged } = useDefaultLayout({ id:
  "book-workspace", storage: localStorage })`, passing both to the group (v4 typings: numbers are
  pixels, unitless strings are percentages, `"240px"` is explicit). Collapse and expand through
  `usePanelRef()`.
- `Navigator`: sections (Overview, **Book agent**, tracks with chapters, Settings materials, Canon,
  Knowledge, Proposals with badge, Prompts, Book settings). **Book agent** (Lucide `Bot`, D-169)
  opens the book agent in the agent panel (§5.3); it is disabled in a `conceiving` book with
  `agent:book_agent.conceiving_hint`. The chapter rows of each track are a dnd-kit
  `SortableContext` (official rows are fixed: `disabled`). "+ Next chapter" per track. Side
  stories collapse under one group.
  - The Canon group lists Entities, Timeline, Memory, Foreshadowing and Goals, then Changes.
  - Entries of systems that are off in `book.settings.canon_systems` are hidden. Changes and
    Proposals always show. QuickSwitch omits the same pages.
  - Goals carries a badge: its nodes that are `ready` or `closable`, from the goals list query,
    fetched only while Goals is on.

### 5.2 Chapter
- `ChapterPage`: header (label, inline-editable title, `StatusBadge`, `WordCount` with target),
  status actions (PROJECT §8.5), tabs.
- `ChapterText`: renders `chapterDoc.paragraphs` with `AnimatePresence`. Each `Paragraph` is
  memoized on `(pid, text, highlight)`. The reading column uses `styles/prose.css`: `.prose-cjk`
  (`text-indent: 2em; margin: 0; line-height: 1.9`) or `.prose-latin` (`margin-bottom: .8em`),
  chosen by the book's writing language.
- `GhostText`: reads `live.previews` for this chapter's running writer session. An
  `insert_text` preview is muted text with a blinking caret, appended to the open tail paragraph
  or as a new block; `insert_paragraphs` and `rewrite_paragraph` previews appear under their
  target pid (spec/03 §12).
- `HoldBar`: shown under the paragraphs touched by the held call (from `ToolCallDto.ui`). Buttons:
  Continue (⌘⏎), Feedback… (inline textarea → "Send & continue"). For `submit_outline` it renders
  on the Outline tab with "Approve outline" and "Request changes"; the outline and reference
  notes are editable while held, and unsaved edits are saved before the resume call. It never
  renders for a held **chapter write**: that hold belongs to the dock below (D-141).
- Held chapter write (D-139, D-141). A write tool never touches `chapters.content`; it stages a
  complete `ChapterContent` on `ToolCallDto.pending_content` (`expected_version` = the
  `content_version` it read). The pane shows the draft the author is being asked for, and the dock
  under it takes the decision:
  - `PendingText` renders the **merged** document — `pending_content.content` itself, with each
    paragraph marked `confirmed` / `changed` / `added` against the draft on screen, the latter two
    tinted with `--color-status-waiting` and a note saying the text is a preview. The merged view is
    **not** editable (a click would save a paragraph the author cannot see, and the save would bump
    the version the staged write is compared against), so `ParagraphEditor` and `FullTextEditor` are
    closed while it is up.
  - When `pending_content.expected_version` no longer equals the draft's `content_version`,
    `runner::apply_pending_content` **drops** the write (the author's edit wins). The pane then
    keeps the confirmed draft, the dock says the text will not be applied and names both versions,
    and its approval reads **Continue without it**.
- **`WriterDock`** (D-173; it replaces `PendingWriteDock`, `features/chapter/WriterDock.tsx`, state
  in `writerDock.ts`):
  - **Placement**: the Text tab panel is a column: the scrolling text, then the dock as a
    non-scrolling footer (`shrink-0`, top border). Inside the footer, `mx-auto max-w-[42em]` lines
    it up with the reading column, and the text never scrolls under it.
  - **Visibility**: shown while the chapter's open writer session (`chapter_write` or
    `chapter_revise`) is in `drafting` or `revising` and is `running`, `held` or `paused`.
  - **Shape**: a plain input, i.e. an auto-growing textarea (1–6 lines) with one round button
    (`size-8 rounded-full`) at its bottom-right:
    - Lucide `Check` for **Approve** (labeled "Apply and continue", or "Continue without it" for
      a superseded write);
    - Lucide `ArrowUp` for **Send**;
    - a spinner while a dock request runs.

    There is no title or button row. The state is told by the placeholder
    (`chapter:dock.placeholder.<state>`), the button, and an `aria-describedby` sentence; the
    superseded sentence names both versions. `data-testid="writer-dock"` carries `data-state`.
  - **Keys**: Enter or ⌘/Ctrl+Enter submits, Shift+Enter is a newline, and both are ignored during
    IME composition. Escape blurs. Feedback is the trimmed text, never blank.
  - **Behavior**. Empty means the trimmed input is empty. "Dock-owned" means `heldDecisionSurface`
    gives the call to the dock. `ownPause` is the dock's own step pause, pending or landed as
    `paused` without an error. "Leave" means leaving the draft region (below).

    | Session state | Auto-pause on, empty | On, text | Auto-pause off, empty | Off, text |
    |---|---|---|---|---|
    | `running`, no pause pending | Approve, disabled | Send → `message` | Send disabled; **focus** → `pause {after_step: true}` (sets `ownPause`) | Send → `resume {feedback}` after the queued pause |
    | `running`, the dock's step pause pending | Approve, disabled | Send → `message` | Send disabled; **leave** → `resume` (cancels) | Send → `resume {feedback}` (cancels, queues the note) |
    | `running`, another pause pending | Approve, disabled | Send → `message` | Send disabled; leave does nothing | Send → `message` |
    | `paused`, `ownPause` | Approve → `resume` | Send → `resume {feedback}` | Send disabled; **leave** → `resume` | Send → `resume {feedback}` |
    | `paused`, other (a panel pause, or an `error`) | Approve → `resume` | Send → `resume {feedback}` | Send disabled; leave does nothing | Send → `resume {feedback}` |
    | `held`, dock-owned (current or superseded) | Approve → `resume` | Send → `resume {feedback}` (discards the write) | Approve → `resume` | Send → `resume {feedback, keep_write: true}` (keeps it) |
    | `held`, panel-owned | disabled ("Decide the held step in the agent panel") | disabled | disabled | disabled |

    - With auto-pause off, focusing the dock asks for a **step pause** (spec/03 §6). The current
      step's write lands in the text, and the writer pauses before its next request, so the text is
      editable.
    - A hold is always decided with the button, never by leaving. With auto-pause off, a hold comes
      only from the panel's Pause or from a completing step. `ToolCallDto.completes` selects the
      `held_completes` placeholder.
    - **The draft region**: one `data-draft-region` container holds the reading column (paragraphs,
      `ParagraphEditor`, `FullTextEditor` and their toggles) and the dock. Moving from the dock into
      the draft text keeps the pause, so the author can edit while paused. Moving to the header,
      the tabs, the navigator, the agent panel or the background leaves it.
      - `useDraftRegionFocus`, active only while `ownPause`, reads `focusout`'s `relatedTarget`, or
        the last `pointerdown` target when that is null.
      - Before sending `resume`, it waits for the chapter's content saves, so the edit lands before
        the writer runs and is reported to it (spec/03 §7). The `useContentMutations` mutations
        share the `mutationKey` `keys.chapters.contentWrite(chapterId)`. The hook checks one task
        after the leaving event, so it sees a save the leaving editor starts on blur, and waits for
        `queryClient.isMutating({ mutationKey })` to reach 0.
    - Leaving acts only when the dock's input is empty, so unsent text keeps the pause. It also
      acts only while `document.hasFocus()` is true, so switching windows or tabs is not leaving.
    - `ownPause` is cleared by any resume, send or leave, and when the session reaches `held`,
      `completed` or another status. A reload forgets it.
    - After a send, the input clears and blurs, and that blur never acts. After an approval, focus
      stays.
    - Settings are read at each event.
    - The dock's requests are serialized:
      - a `409 session.invalid_state` on a focus pause or a leave resume refetches silently;
      - a send answered `409` because the session now runs with no pause pending is sent once more
        as `message`;
      - any other error is a toast, and the text is kept.
  - `pendingDraftView` and `PendingText` are unchanged: the highlighted text above the dock is what
    Approve applies and what a keep-write send keeps.
  - `pendingDraftView` (`features/chapter/pendingWrite.ts`) is the single derivation of that view,
    shared by `ChapterText` and `ChapterPage`, so the text and the dock cannot disagree.
- Word count while a write is staged: the header keeps the server's confirmed `word_count`
  (§8) and adds the projection as a labelled second number (waiting token, `→`); the body's line
  names both (`{{staged}} words once applied ({{confirmed}} now)`). One rule for both panes: the
  stored count is never replaced by a count of text the chapter does not hold.
- `ParagraphEditor`: click to edit (when not locked). Saves via `PATCH content` with
  `base_version`. Enter at the end of a paragraph inserts a new paragraph after it. Backspace in
  an empty paragraph deletes it. Both are ignored while an IME composition is active
  (`event.isComposing`), because Enter confirms Chinese and Japanese input.
- `FullTextEditor`: one textarea; saves via `PUT content/text`.
- `ReviewTab`: `ReportCard` (verdict, summary, issues with "Show" that scrolls and highlights pids,
  editable feedback), `SendBackDialog`, `DiscussDialog`, history list (reports and send-backs).
- `HistoryTab`: version list plus `DiffView` (`diffWordsWithSpace` for spaced languages,
  `diffChars` for unspaced ones, both from `diff`).
- `OutlineDialog`: guidance, the "Approve each chapter outline before it takes effect" switch
  (defaulting to the book setting) and Start. It shares the `previous_not_ready` block and
  **Write anyway** with the Write dialog through one extracted component.
- `WriteDialog`: guidance, granularity (`ToggleGroup`), auto-pause and Start; the description says
  it drafts from the accepted outline. The pause-after-outline switch is gone (it belongs to the
  Outline dialog). On `409 chapter.previous_not_ready` it shows the reason and a **Write anyway**
  button that resends with `force`; on `409 chapter.outline_missing` (a race) it shows the reason
  and a **Write the outline** button.
- Header actions follow the table of PROJECT §8.5. An **open session** (planner or writer) hides
  every start action, because the agent panel owns the session; **Redo the outline** is offered
  on any non-official chapter with no open session, including `in_review` and `revising`.
- `outlineState(chapter, calls)` (`features/chapter/outlineState.ts`) is the single derivation —
  `missing | in_progress | awaiting_approval | accepted` — shared by the header and the Outline
  tab, the `pendingDraftView` pattern of D-141 applied to the outline, so they cannot disagree.
  The Outline tab renders one state per the states in PROJECT §8.5 (the pre-split held-outline
  path was removed with the legacy branch, D-159). `useWriterState` keeps filtering `chapter_write | chapter_revise` for the dock; a
  sibling `openOutlineSession(chapter)` serves the Outline tab.

### 5.3 Agent panel (`features/agent`)
- `AgentPanel`: chooses the session by this precedence: a **manual** pin from the session switcher
  always wins (even a terminal one); otherwise an open pin wins; otherwise the newest open session
  **by id** (the book list is id-desc, a chapter's `open_sessions` is id-asc, so array order is not
  trusted); otherwise a terminal pin stays readable; otherwise the newest session. It fetches
  `SessionSnapshotDto` and renders:
  - `SessionHeader` (role icon: Sparkles / Search / ListTree (planner) / PenLine / ClipboardCheck /
    Archive / Bot (book agent); title; status; model, followed by " · {effort}" when
    `SessionDto.effort` is set — `settings:value.effort.<v>` with the raw value as the fallback
    (D-162); a session switcher dropdown whose pick is a manual pin, D-167);
  - `SessionControls` (by status, PROJECT §8.6), `GranularityControl`, `AutoPauseSwitch`
    (writer only, PATCH session);
  - `HeldCallReview` (a `held` session only): resolves `SessionDto.held_tool_call_id` against the
    snapshot's `tool_calls` and shows **what** is held through the same renderer registry the tool
    cards use (`agent:tools.<name>.summary`, so there is no second naming scheme and no raw
    identifier), the submitted text itself (the tool's body field: `outline`, `plan`, or the prose
    of a held write tool), and the two decisions in the domain's words — **Approve** (`resume` with
    **no** feedback, which finalizes the call with its deferred effects) and **Request changes**
    (`resume` with exactly the typed feedback, which revokes them and appends the revoke notice per
    spec/03 §5.4). `submit_outline` and `submit_revision_plan` get their own wording; every other
    hold (`pause_requested`, `auto_pause`) gets a generic one. The text is rendered inline and
    bounded rather than linked to the tool card, because the stream is virtualized, the card is
    collapsed by default and it shows raw JSON. When the id resolves to no call (the call is
    outside the snapshot's 200-message window) the block is absent and the status-driven
    Continue / Feedback… pair remains. `Abandon` stays in the overflow menu in both cases.
    A held **chapter write** is not reviewed here: its text is the draft, so `SessionControls`
    renders `HeldWriteNotice` instead — a sentence saying the agent's paragraphs are in the draft
    pane and a link to that chapter — and offers neither decision, because the draft pane's dock
    owns them (D-141). One decision surface per held call: the panel offers no second wording of
    the same `resume`. A write hold that staged nothing (a call from before D-139) stays in the
    panel, since the dock would have no text to show;
  - `ActivityStream` (virtualized with `@tanstack/react-virtual`; sticks to the bottom unless the
    author scrolled up): messages, live text and reasoning buffers, tool cards, and appendices. An
    `inbox` appendix whose `inbox_kind` is `user_message` (or absent on an old row) is lifted into
    its own author row in sequence position; a runtime `notice` stays in the card, and every other
    appendix kind is shown under its tool card as a `NoticeItem` (D-165). Text is shown **as delivered** to the model, so an author
    message carries its 【用户留言】 header once consumed (D-160), while its queued row shows the
    raw text.
    **Scrollback** (D-161) lives here, so the agent panel, the conception studio and the research
    task page all get it:
    - the snapshot's `next_cursor` feeds an infinite query over `GET /sessions/:id/messages`;
    - a **Load older messages** row sits at the top while a cursor remains, and scrolling to within
      one row of the top loads the next page by itself;
    - pages are prepended, de-duplicated by message id, and anchored so the first visible row
      does not move;
    - the stream shows loading, retry and "start of the conversation" states;
    - switching sessions drops the pages;
  - `Composer` (state-dependent label and endpoint: `message` / `resume` with feedback / disabled;
    disabled with `agent:composer.disabled_review` while `HeldCallReview` owns the decision, so a
    held call never has two differently-worded reject paths; for a write hold it is disabled with
    `agent:composer.disabled_dock`, which names the draft pane instead of "the review above".
    While a writer runs, both this composer and the draft pane's `WriterDock` can steer; only the
    dock pauses on focus, with auto-pause off, D-173. `SessionControls` shows "Pausing after this
    step…" instead of "Pausing…" while `pause_after_step` is set);
  - `UsageFooter`, ending with `ContextButton` (the context view, below).
- **Book agent** (D-169, spec/08 §8). The navigator item and a `Bot` button in the panel header
  open it:
  - with an open `book_agent` session, they pin it **manually** (D-167), so auto-follow never
    leaves it, even after it starts other sessions;
  - with none, they show `BookAgentStart`: a short intro, the write-mode control and a composer
    ("Ask the book agent…"). The first message posts `POST /books/:id/agent {message,
    write_mode}` and pins the new session. The initial mode is the last one chosen in this
    browser (UI store, `review` at first).

  Both entries are disabled in a `conceiving` book. For a `book_agent` session:
  - `SessionHeader` uses `Bot` and the title `agent:session_title.book_agent`.
  - `SessionControls` adds `WriteModeControl`, a segmented **Review changes** / **Full access**
    control (PATCH `write_mode`, allowed while running), with a hint per mode. The full-access
    hint names what cannot be reverted: research tasks and knowledge entries.
  - The overflow menu adds **End conversation** (`POST /sessions/:id/end`) and **New
    conversation** (end, then `BookAgentStart`).
  - A `provider.context_overflow` failure shows **New conversation** as its primary action.
  - **`HeldCallReview`** renders a call whose `effects.approval` is set from `ui.preview`:
    - `diff` through `DiffView`;
    - `fields` as a two-column table, with field names from
      `agent:book_agent.field.<name>`, and `shared` shown as a warning line;
    - `action` as text.

    Its decisions are **Apply this change** (`resume` without feedback) and **Request changes**
    (`resume` with the feedback). The composer is disabled with `agent:composer.disabled_review`,
    as for other reviewed holds.
  - Tool cards: every new tool (spec/05 §10) has a renderer and a summary key. The
    `start_session` card links to the started session, and the link is a manual pin.
- `ToolCallCard` uses a renderer registry `toolRenderers: Record<string, { icon: LucideIcon;
  summary: (ui, t) => string; Detail?: Component }>`. Each tool has a summary i18n key
  `agent:tools.<name>.summary`. Unknown tools fall back to the name with raw JSON details.
- Interactive cards: `AskUserForm` (radio or checkbox per question, "Other" text; submit →
  `answer`), `ReviewRequestForm` (per document: a tri-state verdict — `unset` / approved / needs
  changes — with `aria-pressed` toggles and a feedback box that appears only for *needs changes*,
  plus an "Approve all" shortcut and a general comment; submit is disabled with a visible count
  while any item is unset, so the form never answers the review on the author's behalf — D-134).
  These are the **`awaiting_input`** counterpart of `HeldCallReview` (D-137): one surface per
  waiting status, each rendered by the session screen that owns the conversation, both above the
  composer, and in both the composer steps aside while the form or the review holds the decision.
  The vocabulary matches too — approve / needs changes there, approve / request changes here.
- **Context view** (D-171, `features/agent/context/`, data from spec/03 §13.1):
  - **Entry points**: `ContextButton` (Lucide `LayoutGrid`, **Context**) opens `ContextDialog`
    for one session. It sits in three places:
    - at the end of `UsageFooter`, labeled with `usage.last_prompt_tokens` as a share of
      `SessionDto.context_window` once a request was sent;
    - in the conception studio's conversation header;
    - in the research task page's session section.

    Every session kind and status gets it.
  - **`ContextDialog`** (`Dialog`, `max-w-5xl`), top to bottom:
    1. **Header line**:
       - "≈ 42,180 tokens · 33% of 128,000" (no "≈" for a provider total);
       - the source: **Provider count** (a calibrated past request), **Estimate, scaled to the
         provider's last count** (a calibrated current view) or **Estimate**;
       - **Cached n** when known;
       - "n tool calls have no result yet and are not counted" when `pending_calls > 0`.
    2. **`RequestStrip`**:
       - one column per request, then an outlined **Current** column;
       - a column's height is `prompt_tokens` for a measured request and `estimate` for an
         estimated one, so it matches its own view's header. **Current** is
         `round(estimate × scale)`. The scale tops at `max(context_window, the largest column)`,
         with a hairline at the window and a dashed **Renews here** line at `rollover_at`;
       - columns have 4 px rounded tops on the baseline, 1 px gaps and a minimum width of 3 px.
         Past that the strip scrolls in its own box and keeps the selection in view;
       - muted ink, with the selected column in foreground ink;
       - a hover tooltip ("Step 12 · 42,180 tokens · cached 30,000 · 14:02");
       - one tab stop, `role="slider"`: Left, Right, Home and End move the selection, and
         `aria-valuetext` reads "Step 12 of 40, 42,180 tokens". The strip always reads `requests`
         from the current query.
    3. **Breadcrumb** ("Context › Tool results › read_chapter") and a **Treemap | List** toggle,
       remembered in the UI store.
    4. **`ContextTreemap`** (SVG, 420 px tall, layout from `lib/treemap.ts`):
       - groups are laid out first, then their leaves inside. A leaf is an item (and the folded
         `rest`) when its part has items, otherwise the part;
       - gaps are 4 px between groups and 2 px between leaves. Leaves fill the group's whole box,
         with no header band, so areas stay proportional across groups;
       - white in-fill labels, measured with 6 px padding and never clipped:
         - the group's name and share, in bold, at its top-left, when they fit in its top-left
           leaf (whose own label then moves down);
         - a leaf's name, then its share, when they fit;
         - the rest is left to the tooltip, the legend and the list;
       - text is measured through an injectable function (canvas `measureText` by default) and the
         width comes from a `ResizeObserver`, so tests pass fixed values;
       - fills use the group's origin token (§7);
       - hover or focus draws a 2 px foreground ring and a tooltip: group, part, item ("Message 12
         · call 2"), "≈ tokens", share, excerpt;
       - click zooms one level (a group, then a part with items), and **Escape** or the
         breadcrumb zooms out. While zoomed, `DialogContent`'s `onEscapeKeyDown` prevents closing
         and zooms out instead; at the top level Escape closes the dialog;
       - keyboard: one tab stop; arrows move between leaves in size order, **Enter** zooms,
         **Escape** zooms out;
       - a request change keeps the zoom path when it still exists;
       - no zoom animation under reduced motion.
    5. **List view**: a table (Name, ≈ Tokens, Share). Group rows expand to parts and parts to
       items, 40 plus "n more". Rows are sorted largest first, but kickoff blocks keep their order.
    6. **Legend**: three origin swatches, each naming its groups.
  - **Labels**:
    - `agent:context.group.<key>` and `agent:context.part.<key>`;
    - `tool`, `call` and `result` parts show the tool name;
    - a `block` part maps the known `block.*` keys to `agent:context.block.<name>`, where `<name>`
      is the key without `block.`, because i18next splits keys on dots. The names are `settings`,
      `canon_systems`, `writing_skills`, `memory`, `recent_chapters`, `goals`, `chapter_draft`,
      `cast_index`, `chapter_diff`, `book_brief` and `book_index`. `assistant.kickoff` reads
      **First message** (the book agent's kickoff wraps the author's first message). Any other key
      reads **Task**, and a block without `source` reads "Block *n*" by kickoff position.
  - **Numbers**: displayed tokens are `round(estimate × scale)` through `formatNumber`. A share is
    always of the whole request (`estimate / total`), also when zoomed.

### 5.4 Conception studio
Two panes: conversation (the same `ActivityStream` and `Composer` components) and documents
(tabs Outline / Characters / World / Protagonist / Research topics). Documents update live from
`write_setting_doc` previews and `settings.changed` events. They are editable when the session is
not running. `CharacterRankList` uses dnd-kit sortable (reorder → `POST characters/reorder`).
"Finalize book" lives in the documents pane footer. The conversation header carries
`ContextButton` for the conception session (§5.3, D-171).

**Before conception** (D-158, D-168). While the book has no conception session, the conversation
pane shows the research before conception (`PreConceptionResearch`). It is built from the
research tasks the studio already loads:
- **C** is the latest knowledge check (`kind === "preparation"`);
- **T** is C's topics (`check_id === C.id`), in id order.

| State | Pane |
|---|---|
| C open (any status except done, failed or cancelled): planning | A "Knowledge check" header with the session status, C's `ActivityStream` in a bounded flex pane, and a link to the research task page (steering, pause and stop live there). Below it, the topic list, filling in live as the check writes it. **Start conception now** asks first ("the check and its research topics keep running") |
| C done; some topic in T is open or `ready` | A "Research before conception" header with progress ("2 of 5 done"). C's report (`Markdown`), collapsed. The topic list. The open topic's `ActivityStream` with its session controls; a failed topic shows its error with **Continue** (D-163) and **Abandon** (skips it). When no topic is open but one waits (a stall), **Run next** runs the first waiting topic. **Start conception now** asks first. A hint: "Conception starts when the research finishes" when `start_after_knowledge_check` is set, otherwise "When the research finishes, start conception here" |
| C done; T settled (none `ready` or open) | The report, the topic list, the entries created by T in total, a link to Book › Knowledge, and **Start conception** (primary when auto-start was off) |
| C failed and the task still has a session | The status, a link to the task, **Continue** (zh-CN 继续), which continues that task on its existing research session and keeps the material it already fetched, and **Start conception** (different from continuing the check). Then the topic list |
| C cancelled, or failed without a session | The status, a link to the task, **Start conception**, and the topic list. Its waiting topics run only by **Run** on their task page |
| No check | Today's "no session" text with **Start conception**, plus **Check knowledge first** (`POST /books/:id/knowledge-check`) |

**The topic list** has one row per topic in T: a status badge, the topic and the scope, with the
requirements expandable.
- Rows by status:

  | Status | Row |
  |---|---|
  | `ready` (waiting) | Edit in place (topic, requirements, scope; PATCH), **Skip for now** (defer) and delete |
  | `deferred` | **Resume** |
  | `done` | Read-only, with its entry count and a link to the task |
  | Open (running, or `failed` with a session) | A link to the task; its controls are in the stream |

- Rows with `origin === "author"` are labelled "Added by you".
- **Add topic** at the end (topic, requirements, scope) posts with `book_id` and `check_id` and is
  shown in every state except "No check".

The pane follows `session.updated` and `research.updated` without a reload. So the stream moves
from the check to each topic in turn, and auto-start swaps the pane to the conception session.

With a conception session the studio is unchanged. The Research topics tab shows the supplementary
topics: `kind === "topic"` drafts (check topics are never drafts). It labels `origin === "author"`
rows "Added by you".

### 5.5 Canon, settings materials, knowledge, prompts, settings
Per PROJECT §8.8–§8.12. Shared pieces: `Markdown` (react-markdown + remark-gfm, no raw HTML),
`features/canon/EntityPicker` (the goal dialog's list pickers, D-155), `KeyValueEditor` (attributes),
`TimelineList` (dnd-kit sortable with `restrictToVerticalAxis`-style constraint and keyboard
sensor), `PromptEditor` (monospace textarea, variable docs side panel, "diff vs default" via
`DiffView`). The Goals dialog's pickers are native selects over the full list for the MVP, not
searchable comboboxes (D-155); a shared searchable picker is a follow-up that must keep the
dialog's scope.

**Manual canon editing** (D-161, PROJECT §8.8):
- `EntityCreateDialog` opens from the Entities filter bar's **+ Entity**. Its fields are kind*,
  name*, aliases, summary, importance, status, storyline (shared, or an **isolated** side track
  → `track_id`) and a change note. On success it opens the new entity.
- `EntityMergeDialog` opens from the detail header's **⋯ › Merge another entity into this one…**.
  It picks B with `features/canon/EntityPicker` (A excluded) and lists what moves (spec/06 §9). It
  warns that a merge cannot be reverted as a whole and explains the shared-into-isolated refusal.
  Its destructive confirm names both entities, and the route moves to A if it showed B.
- **Relation rows edit in place**: kind, inverse kind, description, status
  (`canon:relation_status.*`), since/until chapter and a change note. The labels follow the
  stored source → target, and only changed fields are sent.
- **Full history**: when the detail shows `DETAIL_CHANGE_LIMIT` (20) recent changes, **Show full
  history** loads `GET /entities/:id/history` (latest 100) into the same `ChangeList`.
- `TimelineEventDialog` opens from a row's **Edit**. It holds the insert form's fields (no
  position: drag and drop and Move reorder), sends only changed fields, and contains the
  **Referenced by** section. Since D-170 it also has a **parent** select (the entry's scope only,
  excluding the entry and its subtree; "Top level" for `null`) and a **status** control that
  offers only the allowed moves (create: Happened or Planned; a planned entry: Happened, with an
  optional chapter, or Abandoned; otherwise read-only).
- **Timeline tree** (D-170, spec/06 §1.4). `TimelineList` renders the flat story-order list as a
  tree by `parent_id`: indentation, expand/collapse per entry, siblings in story order.
  - Planned rows are dashed with a **Planned** badge. Abandoned rows are greyed and struck
    through, behind a **Show abandoned** toggle (on by default). The root never exists in the
    client.
  - The row menu has **Edit**, **Add sub-entry**, **Mark as happened** (planned only),
    **Abandon** (planned only) and **Delete**. Abandon's confirmation names the planned
    sub-entries that are abandoned with it. Delete is disabled, with the reason, while
    `child_count > 0`.
  - Drag and drop moves an entry among its siblings: the move endpoint with sibling neighbours.
    It never changes the parent; the dialog does.
  - `409 timeline.status_conflict` and `409 timeline.has_children` render their reason in the
    dialog or a toast.
- **Foreshadowing anchors** (D-170). A row shows "Pays off after «event»" (`before` / `at` /
  `after`), with a Planned or Abandoned badge from `payoff_event_status`, then the direction
  (`planned_payoff`). An unanchored row shows only the direction. The edit dialog has an anchor
  picker: an event combobox over non-abandoned entries and a position select, clearable as a pair.

**Book › Knowledge** (`BookKnowledgePage`, D-158) lists every scope (`useScopes`) with a checkbox
that is checked when the scope is in the book's visible list (`useBookScopes`). Save sends the
**unchecked** ids as `excluded_scope_ids`. A scope created after the page loaded is in neither
list, so it is not sent and stays visible. The hint says that agents in this book read every
checked scope and that new scopes are checked automatically. A **Check knowledge** button opens a
small dialog (optional focus) and calls `POST /books/:id/knowledge-check`, then navigates to the
task page. It is disabled, with the reason, while a check is open. The global scope-creation
dialog has a one-line hint that a new scope is visible to every book unless a book unchecks it.

**Knowledge checks in research lists**: `ResearchTaskList`, the task page and `/research` show a
`preparation` task as "Knowledge check" with its target scope or "—", and without topic or scope
editing. A task with a `check_id` carries the label "From the knowledge check". A check's task page
lists its topics (status, link to each task) under "Research topics" (D-168). `SessionHeader` titles
a research session whose `research_task_kind` is `preparation` with
`agent:session_title.knowledge_check`.

**Bulk defer of research tasks** (D-140) lives in one shared place,
`features/research/useBulkDefer.ts` (`useDeferReadyResearchTasks`), used by `ResearchTaskList`
("Defer all (n)", hidden when nothing is runnable) and by the overview's `run_research` card
(a secondary `ghost` button under the primary `Go` link, which stays primary). It defers the
`ready` rows of its book-scoped list query one request at a time, reusing `useDeferResearchTask`,
and reports a partial batch through `BulkDeferReport` (inline) and a toast: how many moved and
which topics did not.

**Memory** names the chapters injected in full (`MemoryViewDto.recent_chapters`) and states
which `block.memory` variant the page renders (spec/06 §4.5). It leaves out the sections of the
systems that are off in `MemoryViewDto.systems`, as the rendered text does (spec/06 §4.6).

**Goals** (`features/canon/GoalsPage.tsx`, spec/06 §11, D-150) uses the grammar of the other canon
pages.
- **Toolbar:** a track filter (shared nodes by default; an isolated track adds its private nodes,
  badged), a **Ready** filter (`ready` or `closable` nodes with their path) and **+ Goal**.
- **The tree** (goal → plan → task) shows every non-dropped node. Each row has:
  - the level icon (`Target`, `Route`, `Flag`);
  - the title and the **purpose** clamped to two lines, so it is readable without opening the
    node;
  - the status badge;
  - the range chip (overdue tinted);
  - linked canon chips;
  - for a `waiting` node, each condition as a sentence composed from `kind` and `target_label`
    ("Waiting for «玉佩之谜» to be resolved") with a satisfied or open mark, and broken
    conditions flagged;
  - the `ready` badge, or the `closable` prompt with **Mark done**;
  - a row menu: edit, add child, activate, mark done, drop, delete.
- `done` rows stay in the tree, dimmed. A node under a closed ancestor is marked "not sent to
  agents". `dropped` rows live in a collapsed **Dropped (n)** section.
- Pending goal proposals show inline: a status proposal on its node, a new task at the top of its
  parent, each with Accept and Reject through the proposal hooks.
- **`GoalNodeDialog`** has: level (fixed when adding a child), a parent select limited to the
  node's scope, title, purpose (required; hint "Why this exists and what it is for"), status,
  chapter range, a conditions editor (kind plus a target picker: a chapter number, a foreshadowing
  combobox, `EntityPicker` with a status select, or for **Event happened** (`event_occurred`,
  D-170) an event combobox with planned entries first), link pickers over the existing canon
  (`EntityPicker`, an event combobox, a foreshadowing combobox; never typed ids) and the change
  note. `409 goal.open_children` becomes a confirmation dialog that names the open descendants
  and says they stop reaching agents; its confirm resends with `confirm_open_children`.
- **Referenced by:** the entity detail pane and the timeline and foreshadowing edit dialogs list
  the nodes from `GET /books/:id/goals?entity_id=` (or `event_id`, `foreshadowing_id`) as links
  to `canon/goals?node=`. The section is hidden while Goals is off.
- **Proposals page:** `ProposalCard` gains a renderer for `goal_node` and `new_goal_task`
  targets (from `goal_target` and the edits).
- **Overview:** `AttentionList` renders the `ready_goals` row, and `NextActionCard` renders
  `ready_goals` as a secondary line under the three write kinds. Both link to
  `canon/goals?filter=ready`.

**Canon systems** (Book settings, spec/06 §10, D-150) is a section above "Memory windows".
- Five switches in the form draft, saved with the page's **Save** (no reload: the save invalidates
  the book and the navigator re-renders). Each has a one-line effect hint.
- A switch the draft turns **off** shows the retained rows from `GET /books/:id/canon/counts`
  ("42 entities, 30 relations and 120 appearances stay in the book. Agents stop reading and
  maintaining them.") and offers nothing destructive.
- A disabled system shows **View data** (a link to its page).
- The footnote: nothing is deleted, running sessions keep what they started with, and chapters
  approved while a system is off are not recorded into it later.
- Warning (non-blocking) when Memory is off and `recent_chapter_count` is 0: the writer then sees
  no earlier text.
- "Recent summaries" is disabled while Memory is off, and "Mid-term window" while Entities is off,
  each with a hint. "Recent chapters in full" stays: it is a length setting, not a system.
- A disabled system's page, reached through View data or a URL, renders under a banner ("This
  system is off for this book: agents neither read nor maintain it.") with **Turn on**, which
  PATCHes that one flag.

**Writing skills** (design §8). `/skills` is the global catalogue: it lists the active skills, lets
the author create, edit and delete user rows, and previews built-in rows (badge, no edit
affordance). Selection lives in Book settings: a checkbox list with a running
`n selected · x characters` total (above 8,000 characters warn, never block), an "applies to new
sessions" note, and the `recent_chapter_count` input. Deleting a user skill names the books that
use it from `used_by_books` before confirming.

**Settings › Models is protocol-driven** (spec/04 §5, D-132). The option form renders its fields
from `GET /provider-presets/options` (the `ModelOptionSpecDto` table generated from the adapters)
filtered by the selected provider's `protocol`, so a field the protocol cannot send is never
offered. Each field is labelled and explained through `settings:option.<key>` /
`settings:option.<key>_hint` (closed values through `settings:value.<key>.<value>`); no raw field
identifier is shown as a label. Switching the provider to one with another protocol removes the
now-stranded values and shows an "Options removed" notice (`settings:option_cleared_*`) instead of
dropping them silently; a stored mismatch (written before the rule existed) is flagged in the table
and in the dialog, with a one-click removal that PATCHes exactly those fields away.

**Thinking effort** (spec/04 §5.1, D-162) is not one of those options. The served table no longer
lists `reasoning_effort` or `effort`, and the dialog has its own **Thinking effort** section
after the protocol options.
- **Levels** follow the protocol:
  - `anthropic`: five checkboxes in canonical order (`settings:value.effort.*`);
  - `openai`: a chip input. Enter or **Add** appends a level, × removes one, and the order is the
    entry order. Client checks mirror the server (pattern, no duplicates, at most 8);
  - `mock`: a note replaces the section.
- **Default level**: a select with "None — the provider decides" plus the levels. It resets to
  None when its level is removed.
- **Protocol switch**: switching the provider to another protocol removes the levels the new
  protocol cannot send, and the "Options removed" notice names them.
- **Stored mismatch**: `ignored_effort_levels` is flagged in the table and in the dialog like
  `ignored_options`. Its fix PATCH removes those levels, and clears the default if it was one of
  them.
- **Whole profile**: every PATCH sends the whole profile, the levels and default included.
- **Table**: a **Thinking effort** column shows the levels joined with " · " and marks the default
  ("high (default)"); "—" means none.

**Settings › Agent roles** has one row per role (`AGENT_ROLES`): a model select and an effort
select.
- **Entries**: the effort select lists the role's resolved model's levels.
- **Unset label**: "Same as Writer" when the planner or the book agent (`assistant`, D-169)
  follows the writer's model; otherwise "Model
  default ({level})" when the model has a default; otherwise "Provider default".
- **Disabled**: it reads "No effort levels" when the model has none or no model resolves.
- **Stale value**: a stored effort that is not a level of the resolved model stays selected as
  "{level} (not supported)", and a hint names what sessions use instead.
- **Model change**: changing a role's model keeps its effort. **Clear all roles** clears both.
- **Patch**: `roles_defaults` is PATCHed whole.

**Book settings › Model overrides per role** gives each row an effort select whose unset entry is
"Global setting ({effective level})". The model and effort selects are enabled while the book is
`active` or `conceiving`; a conceiving book states that changes start with the **next session**
(a running session keeps its frozen model), while an `archived` book stays locked with its reason.
Both selects save with the form as part of the whole `model_overrides` object (D-166).

Both pages derive the labels from `resolveRoleEffort` in `features/settings/roleModels.ts`, which
mirrors spec/03 §4 step 1. The `model.effort_unsupported` creation warning shows as a toast.

## 6. i18n

- `i18n/index.ts`: i18next with bundled resources (all namespaces imported statically),
  `fallbackLng: "en"`, `supportedLngs: ["en", "zh-CN"]`, `interpolation.escapeValue: false`.
  Language: `ui.language` from the store; on first run, `navigator.language` starting with `zh`
  → `zh-CN`, otherwise `en`. Set `document.documentElement.lang`.
- Key naming: `namespace:area.item` (e.g. `agent:controls.pause`, `chapter:status.in_review`,
  `errors:chapter.locked`). Plurals use i18next suffixes (`_one`, `_other`).
- Formatting helpers (`lib/format.ts`): `formatNumber`, `formatRelativeTime`
  (`Intl.RelativeTimeFormat`), `formatDateTime`, `formatWords(n)` → "2,350 words" / "2,350 字".
- `scripts/check-i18n.ts`: fails when `en` and `zh-CN` key sets differ per namespace, or when a
  string-literal key in `t("…")` / `i18nKey="…"` calls under `src/` is missing from `en`.
- Never concatenate translated fragments. Use interpolation.
- One UI control is named by the prompt pack: the conception agent tells the author to click
  “定稿” (Finalize book). The zh-CN label of the Finalize action is therefore “定稿”.
- Chapter labels and session titles are composed in the UI; the API sends only data.
  `formatChapterLabel` builds a label from the chapter `number` and its track (`chapter:label`,
  or `chapter:side_label` with the track title). A session's title is
  `agent:session_title.<kind>` with that label or the research topic (`SessionDto`
  `chapter_number`, `research_topic`). `chapter_outline` renders "Outlining · Chapter N" /
  「细纲 · 第N章」 and uses the planner's `ListTree` icon.

## 7. Visual details

- **Status tokens** (in `index.css`, both themes): `--status-running`, `--status-waiting`
  (amber), `--status-done` (green), `--status-failed` (red), `--status-idle` (muted). Exposed to
  Tailwind via `@theme inline` as `bg-status-running` etc. `StatusBadge` maps every chapter,
  canon, session, and research status to a token, a Lucide icon, and an i18n label. The research
  status `deferred` uses the `idle` token (nothing is running and nothing is waiting on the agent)
  with its own label, `chapter:status.deferred` (D-138).
- **Context origin tokens** (D-171, `index.css`, both themes, exposed as `bg-context-setup` etc.):

  | Token | Light | Dark | Groups |
  |---|---|---|---|
  | `--context-setup` | `#2a78d6` | `#3987e5` | system, tools, kickoff |
  | `--context-messages` | `#eb6834` | `#d95926` | author, runtime |
  | `--context-work` | `#1baf7a` | `#199e70` | model, results |

  - Any two treemap groups can touch, so the hues were checked on **all pairs**. Seven hues fail
    that check, so the three origins carry color and the group labels carry identity. With the
    dataviz validator on `#ffffff` / `#0a0a0a`, the worst all-pairs values are CVD ΔE 9.2 / 9.4
    and normal-vision ΔE 24.0 / 20.9.
  - The light aqua is 2.82:1 against the surface, so labels stay visible and the list view
    exists.
  - In-fill labels are `#ffffff` in both themes. Other chart text uses text tokens.
- **Fonts**: UI uses the scaffold's Geist with the fallback `"PingFang SC", "Microsoft YaHei",
  "Noto Sans SC", sans-serif`. Prose serif stack: `"Noto Serif SC", "Source Han Serif SC",
  "Songti SC", "SimSun", serif`. Fonts are not bundled.
- **Motion** (`motion/react`): root `MotionConfig reducedMotion="user"`. Paragraph enter
  `{opacity: 0, y: 4} → {opacity: 1, y: 0}` 180 ms; exit height collapse 150 ms; highlight fade
  1.2 s; held ring pulse (2 s loop, subtle); list reorder via `layout`. No animation on large
  re-renders (initial load).
- **Keyboard** (`lib/hotkeys.ts`, a small custom hook; no library): ⌘/Ctrl+⏎, Alt+P, Esc,
  ⌘/Ctrl+K (PROJECT §8.13). Shortcuts are ignored while an IME composition is active
  (`event.isComposing`), which matters for Chinese input.
- **Accessibility**: Radix primitives handle focus. Session status changes are announced in an
  `aria-live="polite"` region. dnd-kit lists use the keyboard sensor.

## 8. Word count (live display)

`lib/wordcount.ts` approximates the server's UAX #29 count (the server value is authoritative and
arrives with every content event). Count each Han ideograph and each Hiragana character as one
word, each run of Katakana (incl. `ー`) as one, and each run of letters, marks, and digits in
other scripts as one. Inside a run, an apostrophe (`'`, `’`) or `.` between two letters, and a
`.` or `,` between two digits, keep the run together ("don't", "e.g", "3.14", "1,000"); hyphens
split words, as in UAX #29 ("well-known" is two). Shared vectors live in
`crates/server/src/text/wordcount_vectors.json`. Rust tests and Vitest both assert them (the web
test imports the file by relative path).

## 9. Tests (Vitest)
- `chapterDoc.applyOps`: insert at start, middle, and end; update; delete; tail; version gap.
- `live` buffers: delta coalescing and clearing on message persistence.
- `wordcount` vectors.
- `ToolCallCard` renderers: every tool name in the spec has a renderer and an i18n summary key.
- Every value of the generated `ErrorCode` union, every session error code, and every warning
  code (spec/10 §1, §4) has an `errors:` key, and every `SessionKind` an
  `agent:session_title.<kind>` key, in both locales (these keys are built dynamically, so
  `check-i18n` cannot see them).
- `check-i18n` runs in `scripts/check.sh`.
- Writer dock (D-173):
  - `writerDock.ts`: every cell of the §5.2 table, the window-blur and needs-attention exceptions,
    and the panel-owned hold;
  - `WriterDock`:
    - the dock sits outside the scroll region, is present while running, held and paused, and is
      absent in `revision_planning`, `stopped` and `completed`;
    - keys, including IME composition and Escape;
    - focus stays after an approval; the input clears and blurs after a send, with no second
      `resume`;
    - auto-pause off: one `pause {after_step: true}` on focus; on leaving the draft region empty,
      `resume` for the dock's own pause and nothing for another; nothing on leaving with text or on
      a window blur; a held write shows Approve, and a note sends `keep_write: true`;
    - the `ownPause` lifecycle, and the `held_completes` placeholder from `ToolCallDto.completes`;
    - the draft region: dock → paragraph sends nothing; then paragraph → header sends `resume`
      only after the save resolved; dock → agent panel sends `resume`;
    - "Pausing after this step…" in `SessionControls` while `pause_after_step` is set;
    - the serial queue, the silent 409, and the `message` fallback;
    - placeholders and labels per state;
  - both locales have every `chapter:dock.*` key, and the removed `chapter:pending_*` keys are gone
    from both.
- Context view (D-171):
  - `lib/treemap.ts`:
    - areas proportional to the weights;
    - rects inside the bounds and not overlapping;
    - zero weights skipped;
    - deterministic;
    - the Bruls et al. reference case (6, 6, 4, 3, 2, 2, 1 in 6 × 4) yields the paper's layout;
  - `ContextDialog`:
    - group labels;
    - the three header-line sources and the pending note;
    - a strip column fetches `?seq=`, the strip keeps reading the current query, and the strip
      keyboard works;
    - zoom by click, Enter, Escape and the breadcrumb, with the path kept across requests;
    - Escape while zoomed does not close the dialog, and at the top it does;
    - an estimated request's column uses `estimate`;
    - a group label only when it fits, and shares of the whole when zoomed;
    - the list view sorts and folds "n more";
    - "Block 2", **First message** and **Task** labels;
  - the footer, studio and research-page entry points;
  - a `session.message` invalidates only its own session's current query, and no session event
    refetches a past view;
  - every `ContextGroupKey`, `ContextPartKey` and known block key has keys in both locales.
- Canon systems (D-150): toggles save with the form; the navigator and QuickSwitch hide and show
  entries; retained counts and the Memory-off warning render; a disabled page shows its banner and
  **Turn on** works.
- Goals (D-150):
  - the tree renders purposes, statuses, ranges, chips and condition sentences with their marks;
  - the Ready filter and the navigator badge;
  - the dialog refuses an empty purpose;
  - the open-children confirmation resends with the flag;
  - Referenced-by lists;
  - `chapter.updated` invalidates the goal keys and the overview;
  - every `ErrorCode` (`goal.*`) and the `propose_goal_change` tool renderer have keys in both
    locales.
- Knowledge first (D-158):
  - `NewBookDialog` hides the auto-start box while conception is off, sends both fields, and
    lands on the studio for a `conceiving` book;
  - `BookKnowledgePage` starts with every visible scope checked, sends the unchecked ids as
    `excluded_scope_ids`, and does not send a scope missing from the loaded list;
  - the studio's four pre-conception states, the confirmation of **Start conception now**, and
    the swap to the conception session on `session.updated`;
  - the "Added by you" label on author topics;
  - the "Knowledge check" title in the research list and in `SessionHeader`;
  - every new string has keys in both locales.
- Research before conception (D-168):
  - `PreConceptionResearch` states:
    - planning, with the topic list filling in live;
    - the queue running, with the open topic's stream;
    - a failed topic with **Continue** and **Abandon**;
    - a stall with **Run next**;
    - settled, with **Start conception** primary when auto-start was off;
    - the swap to the conception session;
  - topic rows:
    - editing a waiting row sends a PATCH;
    - **Skip for now** sends defer, and **Resume** sends resume;
    - **Add topic** sends `check_id`;
    - a done row is read-only;
  - the "From the knowledge check" label, the check page's topic list, and the new checkbox
    wording; both locales.
- Book agent (D-169):
  - **Entry points.**
    - The navigator item and the header button pin the open session.
    - `BookAgentStart` posts with the chosen mode and remembers it.
    - Both are disabled in a conceiving book.
  - **Controls.**
    - `WriteModeControl` sends PATCH while running, and shows both hints.
    - **End conversation** and **New conversation** work.
    - A context overflow offers **New conversation**.
  - **`HeldCallReview`.**
    - The `diff`, `fields` (with `shared`) and `action` previews.
    - **Apply this change** sends `resume` without feedback, and **Request changes** sends the
      feedback.
  - **Tool cards.** Summaries for the new tools; the `start_session` link pins.
  - **Roles.** The seventh row on Agent roles and Book settings, with "Same as Writer".
  - **Both locales.**
- Feature gaps (D-161):
  - `ActivityStream` scrollback: the top row appears with a cursor, pages prepend in order without
    duplicates, and the start, error and session-switch states;
  - `EntityCreateDialog`: required fields, isolated tracks only, navigation;
  - `EntityMergeDialog`: A excluded, warning, refusal explanation, route replacement;
  - relation inline edit: changed fields only, note, ended label, cancel;
  - full-history toggle at 20 rows and its reset;
  - `TimelineEventDialog`: pre-fill, changed fields only, no position fields, Referenced-by gated
    by Goals.
- Timeline tree (D-170):
  - nesting, expand/collapse, the Planned badge, the abandoned strike-through and its toggle;
  - Add sub-entry; Delete disabled with sub-entries; Abandon's confirmation names the planned
    sub-entries; Mark as happened only on planned rows;
  - drag and drop never changes the parent;
  - the dialog's parent select excludes the subtree, the status control offers only allowed
    moves, and a `409` shows its reason;
  - foreshadowing: anchored rows with badges, unanchored rows with the direction only, the anchor
    picker;
  - the goal dialog's **Event happened** condition.
- Thinking effort (D-162):
  - the Models dialog:
    - the Anthropic checkboxes, the OpenAI chips (a duplicate and a pattern violation are
      refused) and the mock note;
    - the default resets when its level is removed;
    - the protocol-switch notice;
    - the mismatch fix carries the remaining levels;
    - neither old effort option is offered;
  - Agent roles:
    - an effort select per row and the three unset labels;
    - the disabled state and the stale marker;
    - Clear all clears the efforts;
    - the PATCH carries the whole object;
  - Book settings: an effort select per row, the "Global setting ({level})" label, saving with
    `model_overrides`;
  - `resolveRoleEffort`: the resolution cases of the plan, mirrored from the server test;
  - `SessionHeader` shows the effort;
  - `model.effort_unsupported` has an `errors:` key in both locales.

