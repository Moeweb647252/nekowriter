# NekoWriter — Project Description

> Status: authoritative product description for the MVP. Technical details live in `docs/spec/`.
> The implementation order lives in `docs/PLAN.md`. Executors start with `AGENTS.md`.

---

## 1. Summary

NekoWriter is a local-first, single-user studio for writing long-form fiction (web novels,
serials, multi-volume sagas) together with AI agents. The author sets direction, watches the
agents work in real time, can interrupt at any sentence, and approves every chapter before it
becomes official text. The agents plan, research, draft, review, and maintain an explicit
**canon** (tiered memory, entity graph, timeline, setting materials), so a story stays consistent
across hundreds of chapters.

The MVP runs as one local Rust server on `127.0.0.1` that serves a React single-page app. It has
no accounts and no cloud component. It talks to LLM providers the author configures and runs a
small embedding model locally.

## 2. Problems we solve

| Problem | Our answer |
|---|---|
| An LLM's world knowledge is stale, generic, or wrong for a niche setting | A **scoped knowledge base** filled by a **research agent** from the web; books choose which scopes their agents may read |
| Long stories drift: forgotten characters, contradicted facts, broken chronology | An explicit **canon**: tiered memory, an entity graph, a chronological timeline, and setting materials. Agents look things up before writing and record what changed after |
| One-shot chapter generation is shallow, because the model cannot think through every sentence | **Granular drafting**: the agent writes one sentence, paragraph, free-sized chunk or chapter per round. The author trades cost for quality |
| Authors lose control of AI output | **Live writing view, pause/feedback at any step, auto-pause, mandatory author review, and a review agent that advises** |
| Prompt quality decides output quality, but prompts are locked in code | **Every model-facing text is a customizable template**, overridable globally and per book |

## 3. Design principles

1. **The author is the final authority.** No chapter becomes official without the author's
   approval. Once a book is finalized, agent changes to setting materials are proposals that the
   author accepts (during conception the agent drafts them under the author's review rounds).
2. **Look before writing, record after writing.** Writing is preceded by retrieval (planning
   phase) and followed by maintenance (canon updates). During drafting the agent only writes.
3. **Write only what has been thought through.** The drafting granularity decides how much
   text one model round may produce.
4. **Lossless interaction.** Pausing never loses work or wastes a request. A paused tool call
   simply has not returned yet. Resuming continues exactly where the agent stopped. Edits the
   author makes while the agent is paused are reported back to the agent, so its view of the
   text never goes stale.
5. **Canon is data, not vibes.** Memory, entities, relations, timeline events, foreshadowing and
   facts are structured records the author can inspect, edit, and revert.
6. **Transparent agents.** Every agent action (searches, lookups, insertions, edits, canon
   updates) is visible as it happens, with its inputs and results.
7. **Cost-aware by construction.** Prompts are assembled stable-first so provider prompt caching
   works. Agents never edit conversation history (append-only), and per-session token usage is
   always visible.
8. **Local-first and private.** Data lives in one SQLite file in the user's data directory. The
   server binds only to loopback.

## 4. Glossary

| Term | Meaning |
|---|---|
| **Book** | A writing project. It has setting materials, canon, tracks, chapters, a writing language, and book-level prompt overrides |
| **Writing language** | The language the novel is written in (BCP-47, e.g. `zh-CN`, `en`). Built-in prompts are Chinese; the writing language tells models which language to write in |
| **Track** | An ordered sequence of chapters. Every book has one **main track** (the main story). **Side stories** are additional tracks |
| **Canon mode** (side tracks) | `canon`: the side story's events become part of the shared canon. `isolated`: it can read canon but its changes stay inside the track |
| **Anchor** (side tracks) | Where a side story sits in story time: after a given main chapter, before the story, or after its end |
| **Chapter** | The unit of writing and review. Status: planned → drafting → in review → (revising ↔ in review) → official |
| **Paragraph id (pid)** | Stable short id of a paragraph inside a chapter (`p1`, `p2`, …). Agents address text by pid |
| **Setting materials** | The four author-approved foundation documents: book outline, character designs (ranked by importance), world setting, and the detailed protagonist description. They are injected verbatim into every writer, reviewer, and archivist context as user-role content |
| **Canon** | The structured story state: tiered memory, entity graph, timeline, memory notes (facts, world rules), and foreshadowing |
| **"The three things"** | Memory, entity graph, and timeline. Writer and reviewer both work from these plus the setting materials |
| **Tiered memory** | Short-term (current chapter outline, recent chapter summaries, the protagonist's immediate state, the previous chapter's ending), mid-term (recent state and relationship changes, character appearance records), long-term (character states, story facts, world rules, timeline, open foreshadowing) |
| **Entity** | A thing that exists in the story world: character, organization (sect, nation…), location, item, or other. It has a title (name), a description, a current state, and attributes |
| **Relation** | A directed, described link between two entities with a free-text kind ("senior brother", "classmate", "sworn enemy") |
| **Timeline event** | A story event kept in chronological story order ("Lin Fan obtains the Nine-Turn Sword"), linked to entities and its source chapter |
| **Foreshadowing** | A planted hook with a status (open, resolved, abandoned) |
| **Knowledge base** | External reference knowledge in **scopes** (global spaces such as "Tang dynasty officialdom" or "Xianxia cultivation tropes"). Each book selects which scopes its agents may read |
| **Research task** | A topic plus requirements for the research agent, which collects information from the web into a target scope |
| **Agent role** | Conception, Researcher, Writer, Reviewer, Archivist (see §5) |
| **Session** | One persisted agent conversation working on one task (e.g. "write chapter 12"). It has a status, a phase, and a full transcript |
| **Phase** | A stage inside a session that decides which tools are allowed (e.g. writer `planning` → `drafting`) |
| **Granularity** | How much text one `insert_text` call may add: `sentence`, `paragraph`, `free` (any size, with `paragraph_end`/`chapter_end` flags), or `chapter`. At sentence and paragraph level it also limits `insert_paragraphs` and `rewrite_paragraph` to one paragraph |
| **Hold** | A tool call that has been executed but whose result is withheld until the author resumes. The model request has already finished, so a hold costs nothing and survives restarts |
| **Pause** | An author request to hold at the next tool boundary |
| **Auto-pause** | Hold automatically after every text-changing tool call (and, if enabled, after the chapter outline) |
| **Feedback** | Author text appended to the withheld tool result when resuming |
| **Steering message** | Author text sent while the agent runs. It is appended to the next tool result |
| **Review report** | The review agent's verdict (pass or revise), issues anchored to pids, and consolidated feedback |
| **Send back** | The author rejects a chapter with feedback. The writer then revises the text in place or redrafts it |
| **Maintenance** | After approval, the archivist updates canon from the official text and may propose setting changes |
| **Proposal** | A suggested edit to setting materials made by an agent, waiting for the author's decision |
| **Change log** | Every agent or author change to canon or settings, with before/after snapshots, notes, and revert |

## 5. Agent roles

All built-in prompts are Chinese. Every role can use a different model, configured globally and
overridable per book.

| Role | Chinese name | Responsibility | Typical tools | Runs when |
|---|---|---|---|---|
| **Conception** | 构思 | Turns a title and a rough idea into the setting materials and research topics through questions, web research, and review rounds | `ask_user`, `web_search`, `web_fetch`, setting-doc and character tools, `set_research_topics`, `request_review` | Creating a book |
| **Researcher** | 资料员 | Collects knowledge for a topic per the requirements and saves well-sourced entries to a scope | `web_search`, `web_fetch`, `search_knowledge`, `save_knowledge_entry`, `finish_research` | Research tasks |
| **Writer** | 写手 | Plans each chapter (retrieval, then a detailed outline), drafts it with granular writing tools, and revises it after feedback | Retrieval tools (planning only), `submit_outline`, `insert_text` and edit tools (drafting only), `finish_draft`, `finish_revision` | Writing and revising chapters |
| **Reviewer** | 审稿 | Reviews a finished chapter against the setting materials, memory, entity graph, and timeline; reports issues and feedback | Retrieval tools, `submit_review` | The author delegates a review |
| **Archivist** | 档案员 | Maintains canon after a chapter becomes official: appearances, entities, relations, timeline, notes, foreshadowing, summary, protagonist state, and setting proposals. Also seeds canon from the settings | Canon read/write tools, `set_chapter_summary`, `propose_setting_change`, `finish_maintenance` | After approval; after conception |

## 6. Feature set (MVP)

### 6.1 Knowledge base
- Global **scopes** with a name and a description. **Entries** have a title, Markdown content,
  source URLs, and tags.
- Each book selects its visible scopes. Agents can only search or read entries in those scopes,
  and the server enforces this.
- Hybrid retrieval (keyword BM25 plus vector similarity) using a small local embedding model.
  Keyword search keeps working while the model downloads or if it fails.
- The author can create, edit, and delete entries by hand.

### 6.2 Knowledge accumulation (research)
- A research task has a topic, requirements, and a target scope (existing or new), and is
  optionally linked to a book. Entries are written in the book's writing language, or, for a
  task without a book, in a language chosen when creating it.
- The researcher plans sub-questions, searches the web, reads pages, cross-checks, and saves
  concise sourced entries without duplicates. The author watches live and can steer, pause, or
  stop.
- Conception produces research tasks automatically. The author reviews them and runs them.

### 6.3 Writing mode
- The writer edits a chapter through **add, delete, modify, and query** tools: `insert_text`,
  `insert_paragraphs`, `replace_text`, `rewrite_paragraph`, `delete_paragraphs`, `read_draft`,
  and `finish_draft`.
- **Granularity** decides how much `insert_text` may write per call: one sentence, one paragraph,
  any amount (`free`, with `paragraph_end`/`chapter_end`), or the whole chapter. At sentence and
  paragraph granularity, `insert_paragraphs` and `rewrite_paragraph` also write one paragraph at a
  time. It is set per book and can change mid-chapter; the change applies from the next call.
- Each round is one model request, so finer granularity costs more but lets the model think
  before every step. The UI says this next to the selector and shows token usage live.

### 6.4 Author participation
- Insertions and edits appear live. The text the agent is still generating streams in as ghost
  text before the tool call completes.
- **Pause**: the next tool call is executed (the author sees it) and its result is withheld.
  **Resume** returns the result unchanged, so the agent continues losslessly.
  **Feedback** returns the result with the author's text appended.
- **Auto-pause**: hold after every insertion or edit. The author approves with one click or
  gives feedback. Optional "pause after outline".
- **Steering**: a message sent while the agent runs is appended to the next tool result.
- **Direct edits while paused**: the author may edit the text. On resume the agent receives an
  exact report of what changed.
- **Stop** aborts the current model request immediately. **Continue** resumes later, even after
  a server restart.

### 6.5 Tiered memory
- **Short-term** (always injected): current chapter outline, the last N chapter summaries of the
  track (default 5), the protagonist's immediate state, and the ending of the previous chapter.
- **Mid-term** (always injected): structured recent evidence from the last M chapters
  (default 10): entity state changes and relationship changes with notes, character appearance
  records, and important characters absent for a long time.
- **Long-term** (retrieved by tools, with a compact index injected): character states and
  attributes, story facts, world rules, the timeline, and open foreshadowing.
- The Memory page shows exactly what the next chapter's writer will receive.

### 6.6 Entity graph
- Entities have a kind (character, organization, location, item, other), a name and aliases, a
  summary, a description, a current state, attributes (key/value), importance, status, and first
  and last appearance.
- Relations are directed and described, with a free-text kind and an optional inverse kind
  ("senior brother" / "junior brother"), and a status (active or ended).
- Agents query an entity's card (attributes, relations, recent changes, appearances), list
  entities, query relations, and search semantically.

### 6.7 Timeline
- Events are kept in **chronological story order**, which can differ from chapter order
  (flashbacks). New events are inserted at an explicit position (after or before another event).
- Each event has a story-time label, a title, a description, its source chapter, and involved
  entities. The author can reorder events by drag and drop.

### 6.8 Workflow discipline
- **Before writing**: the writer's planning phase retrieves from memory, graph, timeline,
  knowledge, and past chapters, then submits a detailed outline with reference notes.
- **While drafting**: retrieval tools are disabled. Only writing and draft-query tools work.
- **After a chapter becomes official**: the archivist maintains memory, graph, timeline, and
  notes, and proposes setting changes when needed.
- The reviewer reviews against the same memory, graph, timeline, and setting materials.

### 6.9 Review mode
- A finished draft is **in review**. Only the author can make it **official**.
- The author can: approve; send back with feedback (revise in place or redraft from scratch); or
  delegate to the **review agent**.
- A review report has a verdict, a summary, issues (severity, category, pids, quote, problem,
  suggestion), and consolidated feedback. The author can edit the feedback and send the chapter
  back with it.
- The author can comment on a report to have the reviewer **re-review**. This produces a new
  report in the same review session (or, if the text changed meanwhile, in a fresh session that
  receives the earlier report and the comment). The review of a report is never delegated
  further: the author judges the reviewer.

### 6.10 Prompt management
- Every text sent to a model is a template in the **prompt registry**: system prompts, task
  instructions, context blocks, runtime notices, and tool descriptions.
- Resolution: book override → global override → shipped default (Chinese). Edit, validate, reset.
- Templates use Jinja syntax (MiniJinja) with documented variables, and are validated when saved.

### 6.11 Book conception
- The author gives a title, a rough idea, an optional genre, and the writing language.
- The conception agent asks structured questions (multiple choice plus free text), researches the
  web, drafts the four setting materials and research topics, and requests reviews. The author
  approves each document or gives feedback, and the agent revises.
- On **Finalize** the book becomes active. The archivist seeds canon from the settings
  (characters → entities, factions, places, world rules, backstory relations), and research
  tasks become ready to run.

### 6.12 Writing chapters
- The author clicks **Write next chapter**, with optional guidance for the chapter.
- Planning: retrieval, then a detailed outline with reference notes. With "pause after outline"
  on, the author approves the outline or requests changes.
- Drafting: granular writing, with all participation features.
- Finish: the draft enters review (§6.9). Approval triggers maintenance (a book setting, on by
  default).

### 6.13 Setting materials evolve
- The author edits setting materials any time. Each change is recorded in the change log.
- Maintenance creates proposals (find/replace edits, an appended passage, or a new character
  design, each with a rationale). Accepting one applies it, and the next sessions see the new
  text.

### 6.14 Side stories
- Create a side track with a title, a premise, a canon mode, an anchor, and an optional focus
  character whose state replaces the protagonist's in short-term memory.
- Its chapters use the same workflow. Its context uses its own recent chapters, the main story up
  to the anchor, and an anchor notice ("events after main chapter X have not happened yet").
- Isolated side stories cannot change shared canon. They record track-local facts, events, and
  entities instead.

### 6.15 Software
- i18n for the whole UI (English and Simplified Chinese).
- Single user, no auth, local run. Data is one SQLite database (with sqlite-vec and FTS5 inside).
- Frontend: shadcn/ui (Nova preset, Radix base), Lucide icons, Motion, react-resizable-panels,
  dnd-kit, Zustand.
- Backend: Rust, axum, SeaORM 2 on SQLite, sqlite-vec, fastembed (local ONNX embedding model).

---

## 7. User flows

Each flow lists what the author does, what the system does, and where it happens in the UI
(screens in §8).

### F1 — First run and configuration
1. The author opens `http://127.0.0.1:7878`. The **Library** is empty and a setup banner says
   "Connect a model provider to start".
2. **Settings › Providers › Add provider**: pick a preset (Anthropic, OpenAI, DeepSeek, OpenRouter,
   Qwen/DashScope, Moonshot, Zhipu, Ollama, LM Studio, Custom OpenAI-compatible). The preset fills
   the base URL and protocol. Enter an API key (or `env:VAR_NAME`). **Test** shows success or the
   exact error.
3. **Settings › Models › Add model**: choose a provider and either type the model id or pick from
   **Fetch models** (when the provider lists models). Set the context window and max output
   tokens; presets pre-fill known values. Protocol options (reasoning replay, effort, cache TTL)
   have sensible defaults.
4. **Settings › Agent roles**: assign a default model to each role. One model may serve all roles.
5. **Settings › Web search**: choose Tavily (API key) or SearXNG (base URL), or none. Conception and
   research need search; without it their search tool returns a clear error.
6. **Settings › Embeddings**: shows the active local model and its download state. The first use
   downloads it into the data directory. A failure is shown here, and keyword search still works.
7. **Settings › Appearance**: UI language (English or 简体中文), theme (light, dark, system),
   editor font (serif or sans) and size.

### F2 — Create a book through conception
1. **Library › New book**. Dialog: title*, rough idea* (multi-line), genre (free text with
   suggestions), writing language* (default from the UI language), and "Start with the conception
   agent" (on by default; off creates an empty active book).
2. The **Conception studio** opens. The agent's first turn usually asks 2–4 structured questions
   (radio or checkbox options plus "Other"). The author answers in the inline form.
3. The agent may search and read web pages (visible as tool cards with titles and URLs), then
   writes drafts. The right side of the studio shows tabs for **Outline, Characters, World,
   Protagonist, Research topics**, updating live as the agent writes (streamed).
4. When drafts are ready the agent calls `request_review`. Each document tab gets an
   **Approve / Needs changes** control and a feedback box. A general comment box is available. The
   author submits, and the agent revises what was not approved.
5. The author may type a message at any time (steering while running, a reply when the agent is
   idle), except while a question form or review request waits for an answer: the form is the
   reply then. Whenever the agent is not running, the author may also edit a document directly;
   direct edits are reported to the agent.
6. When satisfied, the author clicks **Finalize book**. It is enabled when all four documents are
   non-empty, and warns if some are not approved. Then:
   - The book becomes active. The main track exists.
   - The archivist **seeds canon** from the settings (live in the agent panel).
   - Research topics become research tasks (status *ready*). Missing target scopes are created
     and attached to the book.
   - The UI lands on the **Book overview**, which suggests "Run research tasks" and "Write
     chapter 1".

### F3 — Research
1. **Book › Knowledge** (or the global **Knowledge** page) lists research tasks. Each has a topic,
   requirements, a target scope, a status, and the number of entries created.
2. **Run** starts a researcher session. The agent panel shows searches, fetched pages, and saved
   entries. Each entry links to the knowledge entry view.
3. The author can steer ("focus on clothing of the Song dynasty, not Tang"), pause, or stop.
4. The session ends with `finish_research` and a report. Entries are immediately searchable by
   keyword, and by vector once embedded.
5. The author can edit or delete any entry, and add entries by hand.

### F4 — Write a chapter (planning → drafting) with live participation
1. In the navigator, **+ Next chapter** (main or side track) creates a *planned* chapter and opens
   it. In the **Write** dialog the author may add guidance ("the duel ends in a draw; introduce
   the masked girl"), choose the granularity (default from book settings), and toggle auto-pause
   and pause-after-outline. **Start**. (If the previous chapter is not official with synced
   canon yet, the dialog says so and offers **Write anyway**.)
2. **Planning phase**: the agent panel shows retrieval calls (entity cards, timeline queries,
   knowledge hits). The agent submits the outline. The chapter's **Outline** tab fills (streamed).
   - With *pause after outline*: the session holds. The hold bar offers **Approve outline** or
     **Request changes** (feedback). Requesting changes keeps the agent in planning; it revises
     and resubmits. The author may also edit the outline directly and then approve; the agent
     drafts from the edited version and is told what changed.
3. **Drafting phase** (retrieval disabled): the **Text** tab shows paragraphs as they are inserted.
   While a call is being generated, its text streams as ghost text at the cursor position.
   - **Pause** (button or shortcut): the agent finishes the current call; the inserted text is
     highlighted with a hold bar: **Continue** / **Feedback…**.
   - **Feedback**: the author writes "too much exposition, show it through the fight" → the
     agent receives it appended to the result of the held call, and may revise earlier
     paragraphs with edit tools before continuing.
   - **Direct edit**: while held, the author clicks a paragraph and edits it. On Continue the
     agent gets "the author edited p7: …".
   - **Auto-pause** on: every insertion holds. Approving is one keystroke.
   - **Granularity** can change at any time from the agent panel (e.g. sentence-level for a
     crucial scene). The agent is told from its next call.
   - **Steering** while running: typed messages reach the agent with the next tool result.
   - **Stop**: aborts the in-flight request. The chapter keeps all applied text. **Continue**
     later picks up exactly there.
4. The agent finishes with `insert_text(chapter_end=true)` or `finish_draft`. (A chapter far
   below the target length is not finished on the first attempt: the agent is asked to keep
   writing or to confirm.) A *draft* version snapshot is saved, the chapter becomes **In
   review**, and a toast says "Chapter 12 is ready for your review".

### F5 — Review, send back, approve
1. **Review tab** of an in-review chapter: **Approve**, **Send back…**, **Ask the reviewer**.
2. **Ask the reviewer**: a reviewer session runs (visible in the agent panel). The report appears:
   verdict badge, summary, issue list (clicking an issue highlights its paragraphs), and
   editable consolidated feedback.
3. The author can:
   - **Send back with this feedback**: opens the send-back dialog prefilled with the reviewer's
     feedback (and optionally selected issues). The author edits it, chooses **Revise** (edit in
     place, default) or **Redraft** (re-plan and rewrite from scratch, with the old outline for
     reference), and confirms.
   - **Discuss / re-review**: the author comments on the report ("issue 2 is wrong: the token was
     returned in chapter 30"). The same reviewer continues and produces report #2 (if the text
     changed in between, a fresh reviewer session receives report #1 and the comment).
   - **Approve** regardless of the verdict.
4. **Send back** → status *revising*. A writer revision session plans (retrieval allowed),
   revises with edit tools (retrieval disabled), and finishes. A *revision* version is saved, the
   chapter returns to *in review*, and the **History** tab can diff any two versions.
5. **Approve** → status *official* (an *official* version is saved). Maintenance starts
   automatically (book setting, default on).

### F6 — Maintenance and canon review
1. The archivist session records appearances, creates or updates entities and relations (with
   change notes), inserts timeline events in chronological position, adds or retires facts and
   world rules, plants or resolves foreshadowing, writes the chapter summary, updates the
   protagonist's state, and may propose setting changes. It ends with a report.
2. The chapter's canon badge goes *pending → syncing → synced*. The **Changes** page (filterable
   by chapter) lists every change with before/after. **Revert** undoes one (the newest change of
   a target is reverted first).
3. **Proposals** (badge in the navigator) show a diff of each suggested setting edit with its
   rationale: **Accept** or **Reject**. A proposal whose original text (the part it replaces) no
   longer exists is marked *conflict*; the author edits the setting by hand and dismisses it.
4. If the author later edits an official chapter whose canon is synced, its canon badge becomes
   *stale*. **Re-sync canon** runs the archivist in delta mode, using the diff between the synced
   text and the current text.

### F7 — Manage canon by hand
- **Entities**: list with kind filters and search, plus a detail pane (edit fields, aliases,
  attributes, state, relations, appearances, history). Create, merge by hand (rename and move
  relations), delete. Edit forms have an optional *change note*; with a note, the change appears
  in the entity's history that agents read (`get_entity`). Mid-term memory lists chapter-bound
  changes only, so it shows what maintenance recorded.
- **Timeline**: vertical chronological list, drag and drop reorder, add or edit events, filter by
  entity or track.
- **Memory**: three-tier view for a chosen chapter context, and the facts and world rules list
  (add, edit, retire).
- **Foreshadowing**: open, resolved, and abandoned lists, with an edit form.
- Every manual change is in the change log and can be reverted.

### F8 — Customize prompts
1. **Prompts** (global) or **Book › Prompts** (book overrides): the left list is grouped by agent
   and category (System, Tasks, Context blocks, Notices, Tools). Each item shows its source
   badge: Default, Global, or Book.
2. Selecting an item shows the effective text in an editor, the available variables with
   descriptions, and the shipped default (read-only, with a diff toggle).
3. **Save** validates the template (syntax and unknown variables) and stores an override for that
   level. **Reset** deletes the override at that level.
4. New sessions use the new text. Running sessions keep the prompts they started with
   (append-only history).

### F9 — Side stories
1. Navigator › Side stories › **New side story**: title, premise, canon mode (*isolated* by
   default), anchor (after main chapter N — default the latest official, or before the story
   while no chapter is official — or before the story, or after the end), and an optional focus
   character (the side story's protagonist; the book's protagonist otherwise).
2. Chapters in the side track are written, reviewed, approved, and maintained like main chapters.
3. Isolated side stories show a badge, and their canon records appear only in that track's view.

### F10 — Knowledge base management
- **Knowledge** page: scopes (left), entries (middle), entry view or editor (right), and a search
  box with hybrid results.
- **Book › Knowledge**: check the scopes visible to this book's agents. The book's research tasks
  are listed below.

### F11 — Recovering from problems
- A session that fails (provider error after retries, refusal, or repeated replies without a tool
  call) shows a red status with the message and **Continue** (retry) or **Abandon**. Five tool
  errors in a row pause the session instead, so the author can read them and give feedback.
- After a server restart, sessions that were mid-request are *interrupted*: **Continue** re-sends
  the request. Held or awaiting sessions stay held with nothing lost.
- If the chapter is locked by an agent, the editor says so and offers **Pause** or **Stop**.

### F12 — Day-to-day overview
- **Book overview** shows the next action (e.g. "Chapter 43 awaits your review", "Write
  chapter 44"). It also shows attention items (held or awaiting sessions, chapters in review,
  pending proposals, failed sessions, stale canon), progress (chapters, words), and recent changes.

---

## 8. User interface

### 8.1 Information architecture

```
App shell (top bar: logo · Library · Knowledge · Prompts · Settings · theme · language)
├── Library                      /                       books grid, New book dialog
├── Book workspace               /books/:bookId/...      3 resizable panels
│   ├── Overview                 (index)
│   ├── Conception studio        conception              (while conceiving; read-only later)
│   ├── Chapter                  chapters/:chapterId     tabs: Text · Outline · Review · History
│   ├── Setting materials        settings/:doc           outline · world · protagonist · characters
│   ├── Canon                    canon/entities[/:id] · canon/timeline · canon/memory ·
│   │                            canon/foreshadowing · canon/changes
│   ├── Proposals                proposals
│   ├── Book knowledge           knowledge               visible scopes + research tasks
│   ├── Book prompts             prompts                 overrides
│   ├── Side story               tracks/:trackId         premise, anchor, mode, chapters
│   └── Book settings            book-settings
├── Knowledge base               /knowledge[/scopes/:id[/entries/:id]]
├── Research task                /research/:taskId       (also opens inside the book workspace)
├── Prompts (global)             /prompts
└── Settings                     /settings/{providers,models,roles,search,embeddings,appearance}
```

### 8.2 Global shell
- Top bar (h-12): app mark and name; primary nav (Library, Knowledge, Prompts, Settings); a theme
  toggle (sun/moon/monitor); a language switch (EN / 中文). Inside a book: a breadcrumb with a
  book switcher dropdown.
- Toasts (sonner) bottom-right for session events (held, awaiting input, completed, failed) and
  errors.
- The visual style is compact (shadcn Nova): small paddings, dense lists, 13–14 px UI text, and a
  roomy reading column for prose.

### 8.3 Library

```
┌────────────────────────────────────────────────────────────────────────────┐
│ ◆ NekoWriter    Library  Knowledge  Prompts  Settings              ◐  EN|中 │
├────────────────────────────────────────────────────────────────────────────┤
│ Library                                       [Search books…]  [+ New book] │
│ ┌──────────────────┐ ┌──────────────────┐ ┌──────────────────┐              │
│ │ 青云劫            │ │ Night Shift       │ │                  │              │
│ │ Xianxia · zh-CN   │ │ Thriller · en     │ │    +  New book   │              │
│ │ 42 ch · 153k words│ │ Conceiving        │ │                  │              │
│ │ ● Ch 43 in review │ │ ● Awaiting answer │ │                  │              │
│ │ 2 hours ago   ⋯   │ │ yesterday     ⋯   │ │                  │              │
│ └──────────────────┘ └──────────────────┘ └──────────────────┘              │
└────────────────────────────────────────────────────────────────────────────┘
```
- The card's "⋯" menu has: Open, Rename, Archive (or Unarchive), Delete (with typed-name
  confirmation). Archived books are hidden unless the "Show archived" filter is on.
- Empty state: illustration-free card, "Create your first book" with the New book button and a
  link to provider setup when none is configured.

### 8.4 Book workspace layout

```
┌ ◆ NekoWriter / 青云劫 ▾ ─────────────────────────────────────────────── ◐ EN|中 ┐
├───────────────┬──────────────────────────────────────────┬────────────────────────┤
│ NAVIGATOR     │ MAIN                                     │ AGENT PANEL            │
│ ⌂ Overview    │ Ch 43 · The Sword Tomb    ● Drafting  ⋯  │ ✒ Writer · Ch 43   ▾   │
│ ▾ Main story  │ [Text] [Outline] [Review] [History]      │ ● Running   ⏸  ■       │
│   41  ✓ ◉     │                                          │ Granularity S [P] F C  │
│   42  ✓ ◉     │  p1  林凡站在剑冢之外，……                 │ Auto-pause  ○          │
│ ▸ 43  ✎       │  p2  ……                                  │ ────────────────────── │
│   + Next      │  p3  ……………▍ (ghost text, streaming)      │ 💭 Considering pacing… │
│ ▸ Side stories│                                          │ 🔍 get_entity 林凡      │
│ ▾ Settings    │                                          │ 🕒 query_timeline       │
│   Outline     │                                          │ 📋 Outline submitted ✓ │
│   Characters  │                                          │ ✍ insert_text → p1     │
│   World       │                                          │ ✍ insert_text → p2     │
│   Protagonist │                                          │ ✍ insert_text …        │
│ ▾ Canon       │                                          │ ────────────────────── │
│   Entities    │                                          │ ┌────────────────────┐ │
│   Timeline    │                                          │ │ Message the agent… │ │
│   Memory      │                                          │ └────────────────────┘ │
│   Foreshadow  │                                          │   Send ⌘⏎               │
│   Changes     │                                          │ in 41.2k (38.9k cached)│
│ ▸ Knowledge   │                                          │ out 3.1k               │
│   Proposals ②│                                          │                        │
│ ⚙ Book settings│                                         │                        │
└───────────────┴──────────────────────────────────────────┴────────────────────────┘
```
- Three `react-resizable-panels` panels. The navigator (min 200 px, default 240 px, collapsible)
  and the agent panel (min 320 px, default 380 px, collapsible) remember their sizes per browser.
- Navigator chapter rows: number, title (truncated), and a status icon (planned ○, drafting ✎,
  in review ◐, revising ↺, official ✓) plus a canon badge for official chapters (synced ◉,
  pending/syncing spinner, stale ⚠, failed ✕). Rows of non-official chapters can be reordered
  by drag and drop.
- The agent panel follows the context: on a chapter it shows that chapter's latest session; on
  other pages, the most recently active session of the book. A dropdown lists all sessions
  (filter by kind or status) to pin any of them.

### 8.5 Chapter view

**Header**: chapter label and editable title, status badge, word count ("2,350 / 3,000–4,000"),
and actions by status:

| Status | Primary action | Secondary |
|---|---|---|
| planned | **Write** (opens the write dialog) | Write manually (focus the editor) |
| drafting | — (session controls live in the agent panel) | Submit for review (manual drafts) |
| in review | **Approve** · **Send back…** · **Ask the reviewer** | — |
| revising | — (session controls live in the agent panel) | Submit for review (when no writer session is running, e.g. after abandoning it) |
| official | — | Maintain / Retry maintenance (if canon is pending or failed) · Re-sync canon (if stale) · Copy text |

**Text tab**
- A reading column with max width ~42em and line height 1.9. For CJK writing languages: a
  first-line indent of 2em and no extra paragraph gap. For others: no indent and a 0.8em gap.
  The font follows the appearance setting.
- Paragraphs show their pid in the gutter on hover. Clicking a paragraph edits it in place
  (auto-sizing textarea; Esc cancels; blur or ⌘⏎ saves). **Full-text edit** switches to one
  large textarea (paragraphs separated by blank lines). Saving aligns paragraphs and keeps pids.
- Editing is disabled while the chapter's writer session is *running*. The banner says "The
  writer is working — Pause to edit".
- Live agent effects:
  - Inserted paragraphs fade and slide in (Motion, 180 ms). Updated paragraphs flash a subtle
    highlight. Deleted paragraphs collapse out.
  - Ghost text: while `insert_text` is being generated, its text streams in muted color with a
    blinking caret, appended to the open paragraph or as a new paragraph.
  - Held insertion: the affected paragraphs get a ring, and an inline **hold bar** appears below
    them:
    ```
    ╭──────────────────────────────────────────────────────────────╮
    │ ⏸ Paused after this insertion   [✓ Continue  ⌘⏎] [✎ Feedback…] │
    ╰──────────────────────────────────────────────────────────────╯
    ```
    Feedback… expands a textarea inside the bar ("Send & continue"). For a held outline the
    buttons read **Approve outline** and **Request changes**.

**Outline tab**: the chapter outline and the reference notes (Markdown, editable unless the
writer session is running, so also while the outline is held), plus the author's guidance.

**Review tab**:
```
Chapter 43 is waiting for your review.  [✓ Approve] [↩ Send back…] [✦ Ask the reviewer]
─ Report #2 · Needs revision · reviewer · 3 min ago ─────────────────────────────
Summary  The duel's outcome contradicts the timeline …
Issues   ● Critical · Continuity · p12–p14  “他再次拔出青冥剑”  The sword was destroyed in Ch 38 …  [Show]
         ● Minor    · Prose      · p30      Repeated “不禁” ×3 …                                   [Show]
Feedback for the writer (editable)
┌──────────────────────────────────────────────────────────────────────────────┐
│ …                                                                            │
└──────────────────────────────────────────────────────────────────────────────┘
[Send back with this feedback]   [Discuss with the reviewer…]
─ Earlier: Report #1 · Needs revision (collapsed) · Send-back #1 by author (collapsed) ─
```

**Send-back dialog**: a feedback textarea (prefilled when it comes from a report), checkboxes to
include specific issues, a mode radio (**Revise in place** / **Redraft from scratch**), and a
note that the current text is saved as a version.

**History tab**: the version list (kind, time, words, note) and a two-version diff (inline
word-level diff: insertions underlined green, deletions struck red). **Restore** creates a backup
version first.

### 8.6 Agent panel
- **Header**: role icon (Conception ✦ Sparkles, Researcher 🔎 Search, Writer ✒ PenLine,
  Reviewer ✓ ClipboardCheck, Archivist 🗃 Archive), session title, status badge, model name, and
  an info popover (Lucide `Info`) showing the session's frozen system prompt and tool list.
- **Controls** (by status):
  - running: **Pause** (shows "Pausing…" until held), **Stop** (shows "Stopping…" for the moment
    the runner needs to stop).
  - held: a **review block** naming what is held (its tool's label and one-line summary from the
    same registry the tool cards use) with the submitted text readable in it, and two decisions in
    the domain's words — **Approve** (the outline / the revision plan) and **Request changes**
    (reveals a feedback box). Approve agrees to the held call; Request changes sends the feedback
    back so the agent revises. **Abandon** stays in the menu. If the held call cannot be
    identified, the block is not shown and the status-driven pair below applies.
  - paused: **Continue**, **Feedback…**, **Abandon** (nothing is in flight, so there is nothing to
    stop).
  - awaiting input: the form in the stream is highlighted. **Abandon**.
  - idle (conception): the composer replies. **Abandon** in the menu.
  - stopped / failed / interrupted: **Continue**, **Abandon**.
  - completed: no controls (a reviewer session offers **Discuss** from the report).
- Writer sessions also have a **Granularity** segmented control (Sentence · Paragraph · Free ·
  Chapter, with a tooltip on the cost/quality trade-off) and an **Auto-pause** switch.
- **Activity stream** (virtualized; auto-scroll unless the author scrolled up):
  - Assistant text (Markdown) and reasoning (collapsed "Thinking…" with a muted body).
  - Tool call cards: icon, a one-line summary ("Looked up 林凡", "Inserted p12 (paragraph
    open)", "Searched: 宋代服饰 · 5 results"), status (spinner, held, error), and expandable
    input and result. Long-content tools (outline, setting docs, entries) stream a preview.
  - Author items: feedback, steering messages, edit reports, notices, all with timestamps.
  - Interactive cards: **ask_user** forms and **request_review** forms (conception).
- **Composer**: multiline. Its behavior is labeled by state: "Steer the agent" (running),
  "Feedback & continue" (held or paused), "Reply" (idle conception), disabled with a reason
  otherwise (for example while a form waits for an answer, or while the held review block above
  owns the decision).
- **Footer**: cumulative tokens (input, cached input, output) for the session.

### 8.7 Conception studio

```
┌────────────────────────────────────┬─────────────────────────────────────────────────┐
│ CONVERSATION                       │ [Outline] [Characters] [World] [Protagonist]   │
│ ✦ I have a few questions:          │ [Research topics]          ✓ Outline approved   │
│ ┌ Q1 Tone? ○ Dark ● Epic ○ Light ┐ │ ┌─────────────────────────────────────────────┐ │
│ │ Q2 Ending? ○ Tragic ● Open …   │ │ │ # 青云劫 · 故事大纲                          │ │
│ │ [Submit answers]               │ │ │ ## 核心冲突 …                                │ │
│ └────────────────────────────────┘ │ │ (live-updating, editable when not running)  │ │
│ 🔍 web_search 仙侠 修炼体系 设定    │ └─────────────────────────────────────────────┘ │
│ 🌐 web_fetch zhihu.com/…           │ Review: ○ Approve ● Needs changes               │
│ 📄 Drafted Outline, World          │ Feedback: [ …                                ]  │
│ ✦ Ready for your review →          │                                                 │
│ [Type a message…]          Send ⌘⏎ │                [Submit review]   [Finalize book] │
└────────────────────────────────────┴─────────────────────────────────────────────────┘
```
- **Characters tab**: ranked cards (rank, name, role label, summary, expandable profile). Drag and
  drop reorders the ranking, and the agent is told.
- **Research topics tab**: rows of topic, requirements, and target scope (editable; add or remove).

### 8.8 Canon pages
- **Entities**: a filter bar (kind chips, status, track, search) above a dense table (name,
  kind, summary, importance, last seen), and a detail pane on the right: header (name, kind,
  status, importance stars), aliases (tags), summary, description (Markdown), current state
  (highlighted box), attributes (key/value grid), relations (with an "add relation" combobox),
  appearances (chapter chips), and history (change log entries).
- **Timeline**: a vertical list with a rail. Each row has story time (left), title and
  description, entity chips, chapter link, and a track badge. Drag handles reorder rows. The
  "+ Add" between rows inserts at that position.
- **Memory**: a context selector ("Next chapter of: Main story" or a specific chapter) and three
  columns: Short-term, Mid-term, Long-term index, rendered exactly as the writer receives them.
  Below: the facts and world rules table.
- **Foreshadowing**: three tabs (Open, Resolved, Abandoned) with planted and resolved chapter
  links and the planned payoff.
- **Changes**: a filterable log (chapter, session, target type, actor) with a before/after diff
  per row and **Revert**.

### 8.9 Setting materials pages
- Outline, World, Protagonist: a Markdown editor (textarea plus a preview toggle), revision
  number, last-change info, **History** (change log filtered to this document), and pending
  proposals for this document shown inline above the editor.
- Characters: ranked list with drag and drop (dnd-kit), inline edit, add, delete. The link to the
  matching entity is shown when present.

### 8.10 Knowledge pages
- Three columns: scopes (with entry counts; create, rename, describe), entries (search,
  sort), and the entry (view or edit; sources as links; tags).
- The search box has a scope multi-select and shows hybrid results with the matched snippet
  highlighted.
- Research task view: header (topic, scope, status, Run, Stop), requirements, the live agent
  stream (the same component as the agent panel), and the report and created entries when done.
- **New research task** dialog: topic, requirements, target scope (pick one or type a new name),
  book (optional; preset inside a book workspace), and the entry language when no book is set
  (default: the UI language).

### 8.11 Prompts page
```
┌──────────────────────────┬──────────────────────────────────────────────────────────┐
│ [Search prompts…]        │ writer.system                       Source: Global ●    │
│ ▾ Writer                 │ Variables: (none)                                        │
│   System            G    │ ┌──────────────────────────────────────────────────────┐ │
│   Task: chapter     D    │ │ 你是一位经验丰富的职业长篇小说作者……                   │ │
│   Task: revise      D    │ │ …                                                    │ │
│   Notice: outline…  D    │ └──────────────────────────────────────────────────────┘ │
│ ▸ Reviewer               │ [Show default] [Diff vs default]   [Reset] [Save]        │
│ ▸ Archivist              │ ✓ Template valid                                         │
│ ▸ Conception             │                                                          │
│ ▸ Researcher             │                                                          │
│ ▸ Context blocks         │                                                          │
│ ▸ Notices                │                                                          │
│ ▸ Tools                  │                                                          │
└──────────────────────────┴──────────────────────────────────────────────────────────┘
```
Source badges: **D** default, **G** global override, **B** book override.

### 8.12 Settings pages
- **Providers**: a table and a form (preset, name, base URL, API key, extra headers, Test). The
  key is write-only: a stored key is shown masked (`sk-…abcd`) and is replaced only by typing a
  new one.
- **Models**: a table and a form (provider, model id with Fetch, display name, context window,
  max output tokens, protocol options).
- **Agent roles**: five selects (Conception, Researcher, Writer, Reviewer, Archivist).
- **Web search**: provider radio and its fields, and a **Test query** button.
- **Embeddings**: model select (three supported), status (not downloaded, downloading, ready,
  failed + message), the model cache path, index stats (documents, embedded, pending),
  **Rebuild index**.
- **Appearance**: language, theme, editor font, editor size.
- **Book settings** (inside a book): writing language, genre, default granularity, auto-pause,
  pause after outline, target chapter length (min/max), memory windows (recent summaries N,
  mid-term window M), auto-maintenance, model overrides per role, and visible knowledge scopes
  (shortcut).

### 8.13 Interaction details
- **Keyboard**: ⌘/Ctrl+⏎ = primary action in context (Continue when held, Send in the
  composer, Save in editors); Alt+P = Pause/Continue the focused session; Esc = cancel edit or
  close dialog; ⌘/Ctrl+K = quick switch (chapters and pages of the current book).
- **Status colors** (semantic tokens): running = primary, held/paused/awaiting = amber,
  completed/official = green, failed = red, interrupted/stopped = muted.
- **Motion**: subtle and fast (120–220 ms ease-out), layout animations for reorderable lists,
  `MotionConfig reducedMotion="user"` at the root.
- **Confirmations**: deleting books, tracks, or chapters; abandoning sessions; redraft (it
  replaces the text; the old text is kept as a version).
- **Empty states**: every list says what it is and how to fill it ("No timeline events yet —
  they are added by maintenance after a chapter becomes official, or add one by hand").
- **Errors**: inline field errors; API errors become toasts whose text is localized from the
  error code.

### 8.14 Visual language and i18n
- shadcn/ui **Nova** preset on Radix, neutral base color, Geist UI font with CJK system
  fallbacks. The prose font stack is serif ("Noto Serif SC", "Source Han Serif SC", "Songti SC",
  serif) or sans per setting.
- Lucide icons only.
- UI strings live in `en` and `zh-CN` resource files. Dates, numbers and relative times use
  `Intl` with the active locale. Word counts are labeled "words" (en) or "字" (zh-CN).

---

## 9. Scope

### 9.1 In scope (MVP)
Everything in §6 and the flows in §7, for one local user on a desktop browser (≥1280 px wide is
the design target; ≥1024 px must remain usable).

### 9.2 Out of scope (do not build)
- Accounts, authentication, multiple users, sharing, collaboration, cloud sync.
- Mobile layouts and touch-first interactions.
- Export to EPUB, DOCX or PDF, and publishing integrations (the MVP has "Copy text" per chapter).
- Graph visualization of entities (list and detail views only).
- Prompt lab or playground, prompt preview with live data, A/B testing.
- Cost in currency or price tables (token counts only).
- Volumes or arcs as data objects, chapter branching, alternative versions as branches.
- Multi-chapter autopilot (each chapter needs the author's approval anyway).
- Provider-hosted web tools (we run our own `web_search` and `web_fetch`).
- Image generation, text-to-speech, plugins, desktop notifications, telemetry.

## 10. Requirement interpretations

These decisions resolve ambiguities in the original requirements. Executors must follow them.
The author can revise them later.

1. **Scopes are global.** Knowledge scopes are shared spaces; a book selects which ones its
   agents may read. Research writes into exactly one target scope.
2. **One insertion tool, four granularities.** `insert_text(text, paragraph_end, chapter_end)`
   is the only append tool, and its validation follows the active granularity. Switching
   granularity mid-chapter keeps the tool list identical, which preserves prompt caching.
   `finish_draft` also exists, so the agent can finish after edits without inserting text.
3. **"Query" during drafting means reading the draft** (`read_draft`). All canon, knowledge and
   past-chapter retrieval is rejected during drafting and revising. The tools stay in the tool
   list (for caching) but the server refuses them in those phases.
4. **Pause acts at the next tool boundary.** The tool call that is being generated is executed
   and shown, then its result is withheld. **Stop** is the immediate abort and discards a
   half-generated response.
5. **Feedback is appended to the withheld tool result**, under a notice header from the prompt
   pack (`【用户反馈】`). Steering messages sent while the agent runs are appended to the next
   tool result.
6. **Edits made while held are reported** to the agent in the next tool result, computed as a
   paragraph-level diff against the text the agent last saw (plus the full outline or reference
   notes when the author changed them).
7. **Auto-pause covers text-changing tools** (`insert_text`, `insert_paragraphs`, `replace_text`,
   `rewrite_paragraph`, `delete_paragraphs`). "Pause after outline" is a separate switch (default
   on), because the outline is the cheapest place to correct course.
8. **Maintenance happens after approval**, so canon contains only official text, and the
   reviewer checks a chapter against canon as it was before that chapter.
9. **Review delegation is one level deep.** The reviewer advises and the author decides. The
   author's comments on a report make the same reviewer re-review (a fresh reviewer session, given
   the earlier report, when the text changed meanwhile). No agent ever reviews the reviewer.
10. **Setting materials are user-role content.** They are placed verbatim (each document in its
    own tag) at the top of the first user message of every writer, reviewer, and archivist
    session, after the static system prompt. Conception sessions build them instead, and
    research sessions get a short book brief.
11. **Agents propose setting changes; authors apply them.** After finalize, agent edits to setting
    materials are proposals (the conception agent writes drafts directly, reviewed by the
    author). Author edits apply immediately. Both are recorded in the change log.
12. **Tiered memory mapping**: short-term and mid-term are injected; long-term is retrieved by
    tools, with a compact index (cast list, open foreshadowing, counts) injected.
13. **Side stories are tracks** with a canon mode (`canon` or `isolated`) and an anchor.
14. **Prompt customization covers every model-facing text**, tool descriptions included. Tool
    parameter structures are fixed in code.
15. **Conception output** is the four setting materials plus research topics. Finalizing is the
    author's action, and it seeds canon.
16. **One tool call per round for the writer.** Parallel tool calls are disabled for writer
    sessions, so "each round triggers one request" holds literally. (If a provider ignores the
    flag, the extra calls simply run in order.)
17. **Word count** uses Unicode UAX #29 word segmentation: every CJK ideograph counts as one
    word, and space-delimited scripts count words.

## 11. Requirement traceability

| Requirement area | Product sections | Spec |
|---|---|---|
| Knowledge base and scopes | §6.1, F10 | `spec/07-knowledge-search.md` |
| Knowledge accumulation | §6.2, F3 | `spec/07`, `spec/05` (researcher tools) |
| Writing mode and granularity | §6.3, F4 | `spec/03` §Granularity, `spec/05` (drafting tools) |
| User participation | §6.4, F4 | `spec/03` §5.2–§5.4 (holds), §6 (control operations), §7 (edit reports) |
| Tiered memory | §6.5 | `spec/06-canon-memory.md` §Memory |
| Entity graph | §6.6, F7 | `spec/06` §Entities, §Relations |
| Timeline | §6.7, F7 | `spec/06` §Timeline |
| Workflow mandate | §6.8 | `spec/08-workflows.md` |
| Review mode | §6.9, F5 | `spec/08` §Review, `spec/05` (reviewer tools) |
| Prompt management | §6.10, F8 | `spec/09-prompts.md` |
| Book conception | §6.11, F2 | `spec/08` §Conception, `spec/05` (conception tools) |
| Writing chapters | §6.12, F4 | `spec/08` §Chapter lifecycle |
| Evolving settings | §6.13, F6 | `spec/06` §Proposals |
| Side stories | §6.14, F9 | `spec/06` §Visibility, `spec/08` §Side stories |
| Software stack and i18n | §6.15 | `spec/01-architecture.md`, `spec/11-frontend.md` |
