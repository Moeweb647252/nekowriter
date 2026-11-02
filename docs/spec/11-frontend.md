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
- Deltas and previews are coalesced with `requestAnimationFrame` before they reach the store.
- After a reconnect (`onopen` following an `onerror`), invalidate all active queries
  (`queryClient.invalidateQueries()`) and reload the open chapter document.
- Connection state (`connecting | open | reconnecting`) is shown as a small dot in the top bar.

### 4.3 Zustand stores
- `ui.ts` (persisted): `editorFont: "serif" | "sans"`, `editorSize: "s" | "m" | "l"`,
  `navigatorCollapsed`, `agentPanelCollapsed`, `pinnedSession: Record<bookId, sessionId | null>`,
  `language`. Theme is owned by the scaffold's `ThemeProvider`.
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
  conception automatically after the knowledge check" (default on, rendered only while conception
  is on). With conception on, the request sends `start_conception: true`, `knowledge_check: true`
  and `auto_start_conception`. Landing: a `conceiving` book opens `/books/:id/conception`, any
  other book `/books/:id`. The landing no longer depends on `session_id`.

### 5.1 Workspace
- `BookLayout`: `ResizablePanelGroup` (shadcn wrapper over react-resizable-panels v4
  `Group`/`Panel`/`Separator`) with ids `nav`, `main`, `agent`. Sizes: nav `defaultSize="240px"`
  `minSize="200px"` `collapsible`; agent `defaultSize="380px"` `minSize="320px"` `collapsible`; main
  takes the rest. Persist with `const { defaultLayout, onLayoutChanged } = useDefaultLayout({ id:
  "book-workspace", storage: localStorage })`, passing both to the group (v4 typings: numbers are
  pixels, unitless strings are percentages, `"240px"` is explicit). Collapse and expand through
  `usePanelRef()`.
- `Navigator`: sections (Overview, tracks with chapters, Settings materials, Canon, Knowledge,
  Proposals with badge, Prompts, Book settings). The chapter rows of each track are a dnd-kit
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
  - `PendingWriteDock` is sticky at the bottom of the middle column: **Apply and continue**
    (`resume` with no feedback — the transaction that applies `pending_content`) and **Request
    changes** (`resume` with the trimmed feedback). When `pending_content.expected_version` no
    longer equals the draft's `content_version`, `runner::apply_pending_content` **drops** the write
    (the author's edit wins), so the pane keeps the confirmed draft, the dock says the text will not
    be applied and names both versions, and its primary action becomes **Continue without it**.
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
- `AgentPanel`: chooses the session (pinned, or the chapter's open session, or the book's most
  recently updated), fetches `SessionSnapshotDto`, and renders:
  - `SessionHeader` (role icon: Sparkles / Search / ListTree (planner) / PenLine / ClipboardCheck
    / Archive; title; status; model; a session switcher dropdown);
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
    author scrolled up): messages, live text and reasoning buffers, tool cards, appendices shown
    under their tool card as `NoticeItem`s. Text is shown **as delivered** to the model, so an
    author message carries its 【用户留言】 header once consumed (D-160), while its queued row
    shows the raw text.
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
    `agent:composer.disabled_dock`, which names the draft pane instead of "the review above");
  - `UsageFooter`.
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

### 5.4 Conception studio
Two panes: conversation (the same `ActivityStream` and `Composer` components) and documents
(tabs Outline / Characters / World / Protagonist / Research topics). Documents update live from
`write_setting_doc` previews and `settings.changed` events. They are editable when the session is
not running. `CharacterRankList` uses dnd-kit sortable (reorder → `POST characters/reorder`).
"Finalize book" lives in the documents pane footer.

**Before conception** (D-158). While the book has no conception session, the conversation pane
shows the latest knowledge check, chosen from the research tasks the studio already loads
(`kind === "preparation"`):

| State | Pane |
|---|---|
| Check open (any status except done, failed or cancelled) | A "Knowledge check" header with the session status; the check's `ActivityStream` in a bounded flex pane; a link to the research task page (steering, pause and stop live there); **Start conception now**, which asks first ("the check is still running and keeps running") |
| Check done | The report (`Markdown`), the number of entries created, a link to Book › Knowledge, and **Start conception** (primary when auto-start was off) |
| Check failed or cancelled | The status, a link to the task, and **Start conception** |
| No check | Today's "no session" text with **Start conception**, plus **Check knowledge first** (`POST /books/:id/knowledge-check`) |

The pane follows `session.updated` and `research.updated` without a reload, so auto-start swaps it
to the conception session. With a conception session the studio is unchanged. The Research topics
tab shows `kind === "topic"` drafts and labels `origin === "author"` rows "Added by you".

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
  **Referenced by** section.

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
editing. `SessionHeader` titles a research session whose `research_task_kind` is `preparation`
with `agent:session_title.knowledge_check`.

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
  combobox, `EntityPicker` with a status select), link pickers over the existing canon
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
- Feature gaps (D-161):
  - `ActivityStream` scrollback: the top row appears with a cursor, pages prepend in order without
    duplicates, and the start, error and session-switch states;
  - `EntityCreateDialog`: required fields, isolated tracks only, navigation;
  - `EntityMergeDialog`: A excluded, warning, refusal explanation, route replacement;
  - relation inline edit: changed fields only, note, ended label, cancel;
  - full-history toggle at 20 rows and its reset;
  - `TimelineEventDialog`: pre-fill, changed fields only, no position fields, Referenced-by gated
    by Goals.

