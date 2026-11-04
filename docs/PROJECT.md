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
| An LLM's world knowledge is stale, generic, or wrong for a niche setting | A **scoped knowledge base** filled by a **research agent** from the web, checked and completed **before conception**; a book's agents read every scope unless the author unchecks it |
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
| **Timeline event** | A story event kept in chronological story order ("Lin Fan obtains the Nine-Turn Sword"), linked to entities and its source chapter. Events form a tree of entries and sub-entries, and each is *happened*, *planned* or *abandoned* (§6.7) |
| **Foreshadowing** | A planted hook with a status (open, resolved, abandoned), a payoff direction, and optionally a payoff anchor: before, at or after a timeline entry, never a chapter number |
| **Writing goals** | The author's narrative intent in three levels: a **goal** (an arc or the whole book: what the reader should feel, understand or believe), its **plans** (the next tens of chapters: what has to change), and their **tasks** (the next few chapters: the scene or event where it changes). Every node carries a mandatory **purpose**, why it exists, which is what agents receive (§6.16) |
| **Ready** (goals) | A `waiting` node whose conditions (a chapter reached, a hook resolved, an entity's status) are all met in current canon; or an active node whose children are all closed. Derived, never stored |
| **Canon system** | One of Entities, Timeline, Memory (notes and chapter summaries), Foreshadowing and Goals. Each can be switched off per book (§6.17) |
| **Knowledge base** | External reference knowledge in **scopes** (global spaces such as "Tang dynasty officialdom" or "Xianxia cultivation tropes"). A book's agents read every scope except those the author unchecked for the book |
| **Research task** | A topic plus requirements for the research agent, which collects information from the web into a target scope. Tasks the author creates or edits are the author's, and no agent deletes them |
| **Knowledge check** | A research task of its own kind: the research agent checks whether the knowledge visible to a book can support conceiving it, and lists what is missing as research topics. The topics then run one by one. All of this happens before conception, and the check's report goes to the conception agent |
| **Agent role** | Planner, Conception, Researcher, Writer, Reviewer, Archivist, Book agent (see §5) |
| **Book agent** | A conversation with an agent that reads and changes everything in one book and can start the other agents. It works in **review** mode (each change waits for the author's approval) or **full access** (changes apply at once), chosen by the author (§6.18) |
| **Session** | One persisted agent conversation working on one task (e.g. "write chapter 12"). It has a status, a phase, and a full transcript |
| **Phase** | A stage inside a session that decides which tools are allowed. Planner sessions are phase-free; a write session drafts; a revise session runs `revision_planning` → `revising`. `planning` appears only in historical write sessions that ended before the outline split |
| **Granularity** | How much text one `insert_text` call may add: `sentence`, `paragraph`, `free` (any size, with `paragraph_end`/`chapter_end` flags), or `chapter`. At sentence and paragraph level it also limits `insert_paragraphs` and `rewrite_paragraph` to one paragraph |
| **Hold** | A tool call that has been executed but whose result is withheld until the author resumes. The model request has already finished, so a hold costs nothing and survives restarts |
| **Pause** | An author request to hold at the next tool boundary |
| **Auto-pause** | Hold automatically after every text-changing tool call. The separate **pause after outline** switch holds an outline session's `submit_outline` for approval |
| **Feedback** | Author text appended to the withheld tool result when resuming |
| **Steering message** | Author text sent while the agent runs. It is appended to the next tool result |
| **Review report** | The review agent's verdict (pass or revise), issues anchored to pids, and consolidated feedback |
| **Send back** | The author rejects a chapter with feedback. The writer then revises the text in place or redrafts it |
| **Maintenance** | After approval, the archivist updates canon from the official text and may propose setting changes |
| **Proposal** | A suggested edit to setting materials made by an agent, waiting for the author's decision |
| **Change log** | Every agent or author change to canon or settings, with before/after snapshots, notes, and revert |

## 5. Agent roles

All built-in prompts are Chinese. Every role can use a different model and a different thinking
effort (one of the levels its model lists), configured globally and overridable per book (D-162).

| Role | Chinese name | Responsibility | Typical tools | Runs when |
|---|---|---|---|---|
| **Conception** | 构思 | Turns a title and a rough idea into the setting materials, and optional supplementary research topics, through questions, the knowledge base, web research, and review rounds | `ask_user`, `search_knowledge`, `web_search`, `web_fetch`, setting-doc and character tools, `set_research_topics`, `request_review` | Creating a book, after the research before conception |
| **Researcher** | 资料员 | Collects knowledge for a topic per the requirements and saves well-sourced entries to a scope. In a knowledge check, it judges what the book lacks and lists it as research topics | `web_search`, `web_fetch`, `search_knowledge`, `set_research_topics` (knowledge check), `save_knowledge_entry`, `finish_research` | Research tasks; the knowledge check and its topics before conception |
| **Planner** | 策划 | Plans a chapter: retrieves what it needs, then submits a detailed outline with reference notes for the author to accept | Retrieval tools, `read_draft`, `submit_outline` | Writing a chapter outline |
| **Writer** | 写手 | Drafts a chapter from its accepted outline with granular writing tools, and revises or redrafts it after feedback | Retrieval tools (revision planning only), `insert_text` and edit tools (drafting/revising), `finish_draft`, `finish_revision` | Writing and revising chapters |
| **Reviewer** | 审稿 | Reviews a finished chapter against the setting materials, memory, entity graph, and timeline; reports issues and feedback | Retrieval tools, `submit_review` | The author delegates a review |
| **Archivist** | 档案员 | Maintains canon after a chapter becomes official: appearances, entities, relations, timeline, notes, foreshadowing, summary, protagonist state, and setting proposals. Proposes goal changes (a new task, a node done, a node dropped) for the author to decide. Also seeds canon from the settings. Maintains only the canon systems the book has on | Canon read/write tools, `set_chapter_summary`, `propose_goal_change`, `propose_setting_change`, `finish_maintenance` | After approval; after conception |
| **Book agent** | 书籍助手 | Talks with the author about the whole book. It reads and changes the setting materials, chapters (text, outline, notes), canon, goals, research tasks and knowledge entries, and starts detailed-outline, maintenance, re-sync and research sessions (not writing or review, for now). The author's decisions (approving outlines and chapters, proposals, reverts) stay with the author | Read tools for every content type, setting and chapter edit tools, canon and goal tools, research and knowledge tools, `start_session` | When the author opens it, in an active or archived book |

## 6. Feature set (MVP)

### 6.1 Knowledge base
- Global **scopes** with a name and a description. **Entries** have a title, Markdown content,
  source URLs, and tags.
- A book's agents see **every** scope by default, including scopes created later. The author
  unchecks the scopes a book should not use. Agents can only search or read entries in the
  visible scopes, and the server enforces this.
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
- **Research comes first.** Before conception, a **knowledge check** looks at what the knowledge
  base already holds for this book's idea and decides whether it is enough to conceive the book.
  It lists what is missing as **research topics**. The author can edit, skip, delete or add topics.
  The topics then run one by one, each as its own research task. Conception starts when they have
  finished, so it conceives from real material instead of research running afterwards. The author
  can run a check again from Book › Knowledge at any time; its topics run the same way.
- The conception agent may still propose research topics, but only as **supplements**: knowledge
  that came up during conception and is still missing. The author reviews them and runs them.
  Research tasks the author creates or edits are never deleted by an agent.

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
  gives feedback. Separately, **pause after outline** holds the planner's `submit_outline` so the
  author can approve the outline before any writing starts.
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
- Each part of tiered memory belongs to one canon system (or is session scaffolding, like the
  current chapter's label), so switching a system off removes exactly its parts (§6.17).

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
- **History and plan in one tree** (D-170). An entry may hold sub-entries, to any depth: a phase
  of the story holds its scenes. Each entry is *happened*, *planned* (expected, not yet written;
  it has no chapter) or *abandoned* (a plan given up). A plan becomes history when the text
  reaches it; a happened entry is never abandoned, only corrected or deleted. Agents never see
  abandoned entries.
- **Plans are relations, not chapter numbers.** Writing is incremental, so nobody can say in
  advance that a payoff lands in "chapter 50–53". Conception plans by phases and ordering
  relations; the archivist seeds planned entries from the outline; a foreshadowing's payoff is
  anchored before, at or after an entry, with a direction in words.

### 6.8 Workflow discipline
- **Before writing**: a separate **planner** session retrieves from memory, graph, timeline,
  knowledge, and past chapters, then submits a detailed outline with reference notes. The outline
  takes effect only when the author accepts it (or immediately, when pause-after-outline is off).
- **While drafting**: a **write** session starts from the accepted outline; retrieval tools are
  disabled. Only writing and draft-query tools work.
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
- The author gives a title, a rough idea, an optional genre, and the writing language, and
  chooses whether conception starts automatically when the research before it finishes (on by
  default).
- The knowledge check and its research topics run first (§6.2). Then the conception agent asks
  structured questions (multiple choice plus free text), researches the web, drafts the four
  setting materials and any supplementary research topics, and requests reviews. The author
  approves each document or gives feedback, and the agent revises.
- On **Finalize** the book becomes active. The archivist seeds canon from the settings
  (characters → entities, factions, places, world rules, backstory relations), and the
  supplementary research topics become ready to run.

### 6.12 Writing chapters
- The author clicks **Write the outline** for a planned chapter, with optional guidance.
- Outline: the planner retrieves, then submits a detailed outline with reference notes. With
  "pause after outline" on, the author approves the outline or requests changes; with it off the
  outline is accepted at once. The chapter stays planned throughout.
- The author then clicks **Write the chapter**: a write session drafts from the accepted outline
  with granular writing, with all participation features. **Redo the outline** is available on any
  non-official chapter that has no open session.
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

### 6.16 Writing goals
- A tree of **goals → plans → tasks**. Every node has a title, a mandatory **purpose** (why it
  exists, in the author's words), a status (active, waiting, done, dropped), an optional chapter
  range, and links to entities, timeline events and foreshadowing.
- A **waiting** node names what it waits for: a chapter reached, a hook resolved, an entity's
  status. Readiness is computed from current canon, so a node becomes ready the moment the canon
  does. Ready nodes are badged in the navigator and suggested on the overview.
- Agents receive the purposes of the **active** nodes relevant to the chapter. A task with its plan
  and goal reaches the planner, the writer and the reviewer, so a scene is written for its reason,
  not as a generic scene.
- The author creates and edits nodes. The archivist proposes new tasks, completions and drops
  after maintenance, and the author decides. Every change is in the change log and revertible.
- **Goals and planned timeline entries complement each other** (D-170). A planned entry says what
  the story is expected to go through; a goal says why. A node can link planned entries, and can
  wait for one to happen ("Waiting for «搬进 602» to happen").
- Not a project-management tool (no assignees, estimates or dependencies), not a replacement for
  chapter outlines, and not volumes or arcs as data objects (§9.2).

### 6.17 Canon system switches
- Per book, each of Entities, Timeline, Memory, Foreshadowing and Goals can be switched off.
- **Off** means: its content leaves the agents' context, maintenance skips it, its tools are not
  offered, and its page leaves the navigation.
- **Nothing is deleted:** switching it back on restores everything with the data intact. Sessions
  already running keep what they started with.
- Book settings shows what each switch does and how much data a disabled system keeps.

### 6.18 Book agent
- On every page of an active or archived book, the author can open the **book agent**: a
  conversation about the whole book in the agent panel.
- **What it can do.** It reads and changes:
  - the setting materials;
  - chapters: text, outline, reference notes and title, and creating chapters;
  - canon: entities, relations, timeline, memory, foreshadowing;
  - writing goals;
  - the book's research tasks;
  - knowledge entries in the scopes the book sees (shared with other books).

  It can also start detailed-outline, maintenance, re-sync, research and knowledge-check sessions
  for the author. Writing and reviewing chapters are not delegated for now: the author starts
  those sessions as today.
- **Write mode, chosen by the author** and switchable at any time:
  - **Review changes** (default): each change stops with a preview (a diff, or the fields that
    change). The author applies it or asks for something else.
  - **Full access**: changes apply at once.

  Either way, canon, setting and goal changes are on the Changes page and can be reverted.
  Chapter text gets a backup version before the agent first edits it.
- **What it cannot do.** It does not take the author's decisions: accepting outlines and revision
  plans, approving or sending back chapters, finalizing, proposals, reverts, merges, deleting
  chapters or books, and settings.
- One conversation per book is open at a time. **End conversation** closes it, and **New
  conversation** starts fresh.

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
   tokens; presets pre-fill known values. Protocol options (reasoning replay, cache TTL) have
   sensible defaults. **Thinking effort** lists the levels the model accepts (the Anthropic preset
   fills all five, `high` as the default) and marks one as the default.
4. **Settings › Agent roles**: assign a default model to each role, and optionally a thinking
   effort from that model's levels. One model may serve all roles at different efforts; a role
   without an effort uses the model's default level.
5. **Settings › Web search**: choose Tavily (API key) or SearXNG (base URL), or none. Conception and
   research need search; without it their search tool returns a clear error.
6. **Settings › Embeddings**: shows the active local model and its download state. The first use
   downloads it into the data directory. A failure is shown here, and keyword search still works.
7. **Settings › Appearance**: UI language (English or 简体中文), theme (light, dark, system),
   editor font (serif or sans) and size.

### F2 — Create a book through conception
1. **Library › New book**. Dialog: title*, rough idea* (multi-line), genre (free text with
   suggestions), writing language* (default from the UI language), and "Start with the conception
   agent" (on by default; off creates an empty active book). Under it, "Start conception
   automatically when the research before it finishes" (on by default).
2. The **Conception studio** opens on the **research before conception**.
   - **Knowledge check.** The research agent looks at what the knowledge base already has for
     this idea and searches it. Then it lists what is missing as **research topics**: what to
     collect, what it is for, and the scope. Its searches and the topic list are visible as they
     happen. It ends with a report: what is enough, which topics it listed and why, and what needs
     the author's decision.
   - **Research topics.** The topics run one by one, each as its own research session. The studio
     shows the list with each topic's status, and the live stream of the running one. The author
     can edit or skip a waiting topic, delete it, or add a topic of their own; an added topic runs
     in turn. A topic that fails waits for **Continue** or **Abandon**, and abandoning skips it.
   - With auto-start on, conception starts by itself when the last topic finishes, or right after
     the check when it lists none.
   - With it off, the studio shows the report, the finished list and **Start conception**. The
     author can first read, edit or add knowledge.
   - **Start conception now** is available at any time, after a confirmation. Research that is
     still running keeps running.

   The conception agent starts from the knowledge base, the check's report and the list of
   researched topics. Its first turn
   usually asks 2–4 structured questions (radio or checkbox options plus "Other"). The author
   answers in the inline form.
3. The agent may search the knowledge base and read web pages (visible as tool cards with titles
   and URLs), then
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
   - The supplementary research topics become research tasks (status *ready*). Missing target
     scopes are created; a target scope the author had unchecked for this book becomes visible
     again. The topics researched before conception are already done.
   - The UI lands on the **Book overview**, which suggests "Run research tasks" when supplementary
     topics are waiting, and "Write chapter 1".

### F3 — Research
1. **Book › Knowledge** (or the global **Knowledge** page) lists research tasks. Each has a topic,
   requirements, a target scope, a status, and the number of entries created. A knowledge check
   is listed as "Knowledge check", and the topics it listed are labelled "From the knowledge
   check".
2. **Run** starts a researcher session. The agent panel shows searches, fetched pages, and saved
   entries. Each entry links to the knowledge entry view.
3. The author can steer ("focus on clothing of the Song dynasty, not Tang"), pause, or stop.
4. The session ends with `finish_research` and a report. Entries are immediately searchable by
   keyword, and by vector once embedded.
5. The author can edit or delete any entry, and add entries by hand.

### F4 — Write the outline
1. In the navigator, **+ Next chapter** (main or side track) creates a *planned* chapter and opens
   it. The header offers **Write the outline**, which opens the **Outline** dialog: guidance
   ("the duel ends in a draw; introduce the masked girl"), the "approve the outline before it
   takes effect" switch (default from book settings) and **Start**. (If the previous chapter is
   not official with synced canon yet, the dialog says so and offers **Write anyway**.)
2. The agent panel shows the planner's retrieval calls (entity cards, timeline queries, knowledge
   hits). The planner submits the outline; the chapter's **Outline** tab fills (streamed), and
   the chapter stays *planned* — the stored outline does not change until the author accepts it.
   - With *pause after outline*: the session holds. The tab shows the proposal with **Approve
     outline** / **Request changes** (feedback), and the current accepted outline below it when
     there is one. Requesting changes keeps the planner working; it revises and resubmits. The
     author may also edit the chapter's fields directly while the session is not running; the
     planner is told what changed.
   - With the switch off: the outline is accepted as soon as it is submitted and the session
     completes. The chapter still stays *planned*.
3. The header then offers **Write the chapter** (and, until writing starts, **Redo the outline**).

### F4b — Write the chapter (drafting) with live participation
1. **Write the chapter** opens the **Write** dialog: guidance, the granularity (default from book
   settings) and auto-pause. **Start**. (The same previous-chapter rule and **Write anyway**
   apply. A chapter with no outline cannot start a write session; the dialog offers **Write the
   outline** instead.)
2. **Drafting** (retrieval disabled): the **Text** tab shows paragraphs as they are inserted.
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
3. The agent finishes with `insert_text(chapter_end=true)` or `finish_draft`. (A chapter far
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
     place, default) or **Redraft** (archive the text and draft again from the current outline,
     following the feedback), and confirms. To change the plan itself, the author redoes the
     outline first: **Redo the outline** is available on the in-review chapter (any non-official
     chapter without an open session), and the redraft then starts from the accepted result.
   - **Discuss / re-review**: the author comments on the report ("issue 2 is wrong: the token was
     returned in chapter 30"). The same reviewer continues and produces report #2 (if the text
     changed in between, a fresh reviewer session receives report #1 and the comment).
   - **Approve** regardless of the verdict.
4. **Send back (revise)** → status *revising*. A writer revision session plans (retrieval
   allowed), revises with edit tools (retrieval disabled), and finishes. A *revision* version is
   saved, the chapter returns to *in review*, and the **History** tab can diff any two versions.
   **Send back (redraft)** → the text is archived, the chapter returns to *drafting*, and a write
   session drafts again from the current outline with the feedback; without a non-empty outline
   the request is refused and the dialog offers **Revise in place** or writing an outline.
5. **Approve** → status *official* (an *official* version is saved). Maintenance starts
   automatically (book setting, default on).

### F6 — Maintenance and canon review
1. The archivist session records appearances, creates or updates entities and relations (with
   change notes), inserts timeline events in chronological position, adds or retires facts and
   world rules, plants or resolves foreshadowing, writes the chapter summary, updates the
   protagonist's state, and may propose setting changes. It ends with a report. It does only the
   steps of the canon systems the book has on. When the chapter has relevant writing goals, it may
   also propose goal changes: a new task the chapter made possible, a node whose purpose the
   chapter achieved, or a task the chapter made unreachable.
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
- **Timeline**: a tree in chronological order, drag and drop reorder among siblings, add or edit
  events and sub-entries, mark a planned entry as happened or abandon it, filter by entity or
  track.
- **Memory**: three-tier view for a chosen chapter context, and the facts and world rules list
  (add, edit, retire).
- **Foreshadowing**: open, resolved, and abandoned lists, with an edit form that also sets the
  payoff anchor.
- **Goals**: a tree of goals, plans and tasks with their purposes visible. The author:
  - adds a node (title, purpose, chapter range, linked canon picked from lists);
  - sets a node to wait for conditions, activates it when it becomes ready, and marks nodes done or
    dropped;
  - accepts or rejects the archivist's goal proposals inline.

  Entity, event and foreshadowing views list the goals that reference them.
- **Book settings › Canon systems**: switch a system off or on. The page says what stays in the
  book, and a disabled system's data stays reachable through **View data**.
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
- **Book › Knowledge**: every scope is checked unless the author unchecked it for this book;
  unchecking hides it from this book's agents, and new scopes arrive checked. **Check knowledge**
  runs a knowledge check now. The book's research tasks are listed below.

### F11 — Recovering from problems
- A session that fails (provider error after retries, refusal, or repeated replies without a tool
  call) shows a red status with the message and **Continue** (retry) or **Abandon**. Five tool
  errors in a row pause the session instead, so the author can read them and give feedback.
- After a server restart, sessions that were mid-request are *interrupted*: **Continue** re-sends
  the request. Held or awaiting sessions stay held with nothing lost.
- If the chapter is locked by an agent, the editor says so and offers **Pause** or **Stop**.

### F12 — Day-to-day overview
- **Book overview** shows the next action (e.g. "Chapter 43 awaits your review", "Outline
  Chapter 44", "Write Chapter 44"). A planned chapter is offered before **Write next chapter**
  creates a new one. When writing goals are ready to activate or to close, the chapter actions
  say so in a second line. It also shows attention items (held or awaiting sessions, chapters in
  review, pending proposals, failed sessions, stale canon, ready goals), progress (chapters,
  words), and recent changes.

---

### F13 — Ask the book agent
1. In a book, click **Book agent** (navigator or agent panel). Choose **Review changes** or **Full
   access**, and type a request ("from chapter 12 on, Lin Fan's sword is called Qingshuang: fix
   the settings and the written chapters").
2. The agent reads what it needs: the book overview, chapters, settings and canon. In review mode
   each change stops in the panel with its diff. **Apply this change** or **Request changes**
   (with a reason). In full access the changes appear at once, as tool cards.
3. It can start work for you ("write the outline of chapter 45"). The started session appears in
   the session list, and its own decisions (accepting the outline) are still yours.
4. Reply to continue, switch the mode at any time, or **End conversation**.

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
│   │                            canon/foreshadowing · canon/goals · canon/changes
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
│   Goals ①    │                                          │                        │
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
- The Canon group shows only the canon systems the book has on (§6.17); Changes always shows.
  Goals carries a badge counting nodes that are ready to activate or to close.
- The agent panel follows the context: on a chapter it shows that chapter's latest session; on
  other pages, the most recently active session of the book. A dropdown lists all sessions
  (filter by kind or status) to pin any of them. Opening the **book agent** (navigator, or the
  panel's `Bot` button) pins its conversation, so the panel stays on it while it starts other
  sessions.

### 8.5 Chapter view

**Header**: chapter label and editable title, status badge, word count ("2,350 / 3,000–4,000"),
and actions by status:

| Status / state | Primary action | Secondary |
|---|---|---|
| planned, no outline, no open session | **Write the outline** (opens the Outline dialog) | Write manually (focus the editor) |
| planned, outline accepted, no open session | **Write the chapter** (opens the Write dialog) | **Redo the outline** · Write manually |
| drafting, no open session | **Write the chapter** when it has an outline, otherwise **Write the outline** | Submit for review (manual drafts) · **Redo the outline** |
| an open planner or writer session | — (the agent panel owns the session) | — |
| in review | **Approve** · **Send back…** · **Ask the reviewer** | **Redo the outline** |
| revising | — (session controls live in the agent panel) | Submit for review (when no session is running, e.g. after abandoning one) · **Redo the outline** |
| official | — | Maintain / Retry maintenance (if canon is pending or failed) · Re-sync canon (if stale) · Copy text |

**Text tab**
- A reading column with max width ~42em and line height 1.9. For CJK writing languages: a
  first-line indent of 2em and no extra paragraph gap. For others: no indent and a 0.8em gap.
  The font follows the appearance setting.
- Paragraphs show their pid in the gutter on hover. Clicking a paragraph edits it in place
  (auto-sizing textarea; Esc cancels; blur or ⌘⏎ saves). **Full-text edit** switches to one
  large textarea (paragraphs separated by blank lines). Saving aligns paragraphs and keeps pids.
- Editing is disabled while the chapter's planner or writer session is *running*. The banner
  says "The writer is working — Pause to edit".
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

**Outline tab**: the chapter outline and the reference notes (Markdown), plus the author's
guidance. Its state is derived, never stored:
- **missing** (empty outline): an empty state with **Write the outline**; the fields stay editable,
  so the author can write the outline by hand.
- **in progress** (an open outline session): the fields are read-only, "The planner is working",
  with a link that focuses the session in the panel.
- **awaiting approval** (that session holds `submit_outline`): the proposal (outline, reference
  notes, proposed title) read from the held call's input, the hold bar with **Approve outline** /
  **Request changes**, and the current accepted outline below it marked "replaced when you
  approve".
- **accepted**: the editable fields as before, plus **Redo the outline** when the chapter is not
  official and has no open session.

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
note that the current text is saved as a version. The redraft description says the text is
archived and the writer drafts again from the current outline, following the feedback; to change
the plan, redo the outline first.

**History tab**: the version list (kind, time, words, note) and a two-version diff (inline
word-level diff: insertions underlined green, deletions struck red). **Restore** creates a backup
version first.

### 8.6 Agent panel
- **Header**: role icon (Conception ✦ Sparkles, Researcher 🔎 Search, Planner 🌳 ListTree, Writer ✒
  PenLine, Reviewer ✓ ClipboardCheck, Archivist 🗃 Archive, Book agent 🤖 Bot), session title, status
  badge, model name with the session's frozen thinking effort ("Opus 5.5 · High"), and an info
  popover (Lucide `Info`) showing the session's frozen system prompt and tool list.
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
- **Research topics tab**: the supplementary topics, as rows of topic, requirements, and target
  scope (editable; add or remove). Rows the author added or edited are labelled "Added by you";
  the agent keeps them.
- **Before conception starts**, the conversation pane shows the research before conception
  instead:
  - while the check runs: its live stream, a link to its task page, and the topic list as it
    fills in;
  - then: progress ("2 of 5 done"), the check's report, the topic list (edit, skip, delete or add
    waiting topics), and the running topic's live stream with its controls;
  - **Start conception**, which asks for confirmation while research still runs.

  With no check, it offers **Start conception** and **Check knowledge first**.

### 8.8 Canon pages
- **Entities**: a filter bar (kind chips, status, track, search, **+ Entity**) above a dense
  table (name, kind, summary, importance, last seen), and a detail pane on the right:
  - a header (name, kind, status, importance stars) whose **⋯** menu offers **Merge another entity
    into this one…** (with a warning that a merge cannot be reverted as a whole);
  - aliases (tags), summary, description (Markdown), current state (highlighted box), attributes
    (key/value grid);
  - relations, with an "add relation" combobox; each relation row edits in place (kind, inverse
    kind, description, active or ended);
  - appearances (chapter chips);
  - history: the latest change log entries, with **Show full history**.

  **+ Entity** asks for kind, name, aliases, summary, importance, status and the storyline (shared,
  or an isolated side story).
- **Timeline**: a vertical list with a rail. Each row has story time (left), title and
  description, entity chips, chapter link, a track badge, and **Edit** (a dialog with the same
  fields; position changes only by dragging). Drag handles reorder rows. The "+ Add" between rows
  inserts at that position. Since D-170 the list is a tree: sub-entries are indented under their
  entry and collapse with it. Planned rows are dashed with a **Planned** badge. Abandoned rows are
  greyed and struck through (a toggle hides them). The row menu adds **Add sub-entry**, **Mark
  as happened** and **Abandon** for planned rows. **Delete** waits until an entry has no
  sub-entries.
- **Memory**: a context selector ("Next chapter of: Main story" or a specific chapter) and three
  columns: Short-term, Mid-term, Long-term index, rendered exactly as the writer receives them.
  Below: the facts and world rules table.
- **Foreshadowing**: three tabs (Open, Resolved, Abandoned) with planted and resolved chapter
  links and the planned payoff: "Pays off after «event»" with a Planned badge when the anchor
  has not happened yet, then the direction.
- **Goals**: a toolbar (track filter, **Ready** filter, **+ Goal**) above a tree of goals, plans
  and tasks.
  - Each row shows the level icon, the title, the **purpose** (two lines, readable without
    opening), the status, the chapter range (overdue tinted) and linked canon chips.
  - A waiting node lists its conditions as sentences ("Waiting for «玉佩之谜» to be resolved")
    with a met or unmet mark.
  - Ready nodes carry a badge. A node whose children are all closed offers **Mark done**.
  - Pending archivist proposals sit on the node they concern, with Accept and Reject.
  - Done nodes stay dimmed in the tree. Dropped nodes collapse into a section at the bottom.
  - The node dialog: title, purpose (required), status, range, conditions, and canon links picked
    from lists. Closing a node with open children asks first and names them.
- **Changes**: a filterable log (chapter, session, target type, actor) with a before/after diff
  per row and **Revert**.
- A canon page whose system is off for the book is hidden from the navigator. Opened through
  Book settings › **View data**, it shows a banner with **Turn on**.

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
  A knowledge check shows "Knowledge check" instead of a topic, and lists the topics it wrote,
  with their statuses.
- Creating a scope says that every book sees it unless a book unchecks it.
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
  max output tokens, protocol options, **thinking effort**: the levels the model accepts, as five
  checkboxes for Anthropic or typed chips for OpenAI-compatible providers, and an optional default
  level; the Demo takes none).
- **Agent roles**: seven rows (Conception, Researcher, Planner, Writer, Reviewer, Archivist, Book
  agent), each with a model select and an effort select listing that model's levels. The planner's
  and the book agent's unset model reads "Same as Writer" and follows the writer's model, and then
  the unset effort follows the writer's too. Any other unset effort reads "Model default ({level})"
  or "Provider default". A stored effort the model does not list is marked "(not supported)" with
  what sessions use instead.
- **Web search**: provider radio and its fields, and a **Test query** button.
- **Embeddings**: model select (three supported), status (not downloaded, downloading, ready,
  failed + message), the model cache path, index stats (documents, embedded, pending),
  **Rebuild index**.
- **Appearance**: language, theme, editor font, editor size.
- **Book settings** (inside a book): writing language, genre, default granularity, auto-pause,
  **Approve each chapter outline before it takes effect** (the pause-after-outline switch, with
  the hint "When off, the planner's outline is accepted as soon as it is submitted"), target
  chapter length (min/max), **canon systems** (five switches with a one-line effect each; a switch
  turned off says how much data the book keeps and offers **View data**; a footnote says nothing is
  deleted and running sessions are unaffected; a warning when Memory is off and recent chapters
  are 0), memory windows (recent summaries N, mid-term window M; disabled while their system is
  off), auto-maintenance, model overrides per role (seven rows, each a model and an effort select
  whose unset entry follows the global setting; the planner's hint names its effective model —
  "Default: {model}", otherwise "Follows the Writer: {model}", or "none"), and
  visible knowledge scopes (shortcut).

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
- Volumes or arcs as data objects, chapter branching, alternative versions as branches. Writing
  goals (§6.16) are intent records with an optional chapter range, not containers of chapters.
  Timeline entries (§6.7) group story events in story time; they hold no chapters and no chapter
  range.
- Project-management features for goals (assignees, estimates, task dependencies, burndown),
  prose generated from a goal, and seeding goals from the setting materials.
- Per-chapter or per-session canon system switches, partial switches, and back-filling a
  re-enabled system.
- Multi-chapter autopilot (each chapter needs the author's approval anyway).
- Provider-hosted web tools (we run our own `web_search` and `web_fetch`).
- Image generation, text-to-speech, plugins, desktop notifications, telemetry.

## 10. Requirement interpretations

These decisions resolve ambiguities in the original requirements. Executors must follow them.
The author can revise them later.

1. **Scopes are global.** Knowledge scopes are shared spaces; a book's agents read all of them
   unless the author unchecks some (D-158). Research writes into exactly one target scope at a
   time.
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
   on) on the outline session's `submit_outline`: the outline is the cheapest place to correct
   course, and approving it is the outline step's own decision. The Write dialog does not carry
   the switch.
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
    author). Author edits apply immediately. Both are recorded in the change log. The **book
    agent** is the exception (D-169): the author asks it directly and chooses whether each change
    is reviewed first or applies at once. Its changes are logged as the agent's.
12. **Tiered memory mapping**: short-term and mid-term are injected; long-term is retrieved by
    tools, with a compact index (cast list, open foreshadowing, counts) injected.
13. **Side stories are tracks** with a canon mode (`canon` or `isolated`) and an anchor.
14. **Prompt customization covers every model-facing text**, tool descriptions included. Tool
    parameter structures are fixed in code.
15. **Conception output** is the four setting materials plus optional supplementary research
    topics (D-168). Finalizing is the author's action, and it seeds canon.
16. **One tool call per round for the writer.** Parallel tool calls are disabled for writer
    sessions, so "each round triggers one request" holds literally. (If a provider ignores the
    flag, the extra calls simply run in order.)
17. **Word count** uses Unicode UAX #29 word segmentation: every CJK ideograph counts as one
    word, and space-delimited scripts count words.
18. **Writing goals are intent, kept like canon.** They are not story facts, but they follow canon's
    conventions: `track_id` scope, change log, revert, a per-book switch. Their readiness is
    computed from canon (D-150).
19. **Only active goal nodes reach agents.** "Active and waiting nodes are injected" conflicts with
    "a waiting node is never injected", and the second wins. A node under a done or dropped
    ancestor is not injected either.
20. **No timeline condition.** A condition can only name an existing event, so "the event exists"
    would be true at once. Waits use a chapter reached, a hook resolved or an entity's status.
    Events remain linkable.
21. **The protagonist's state belongs to Entities.** With Entities off it leaves the prompt,
    because maintenance would stop keeping it current. Continuity rests on the previous chapter's
    ending, recent chapters in full, summaries and the protagonist document.
22. **The chapter outline is never Memory's.** The writer gets it with the chapter text. The echo
    in tiered memory, used by re-sync and as a fallback, is session scaffolding and survives
    Memory off.
23. **"Archive counts" under Memory off** means Memory's own counts (facts, world rules). Each
    system's counts leave with that system.
24. **Switches are independent of other settings.** Recent chapters in full is a length setting,
    not a system. Auto-maintenance decides when maintenance runs, not what it maintains. The
    memory windows apply only while their system is on.
25. **Ready goals do not take over the next action.** They appear as a second line on the chapter
    actions and as an attention row, because a node can stay ready as long as the author likes.
26. **"A task looks unreachable" is a drop proposal.** The archivist explains why. Accept drops
    the task, reject keeps it.
27. **Side stories get book-wide goals.** Shared goals without a chapter range reach every
    storyline. An isolated side story also gets its own private nodes. Main-story plans and tasks
    never reach a side story.
28. **Upgrading changes nothing for an existing book.** With all systems on and no goal nodes,
    every prompt and tool list is byte-identical to before.
29. **"Knowledge accumulation comes before every stage"** means an agent first checks whether the
    existing knowledge base can support conceiving the book. It lists the gaps as research topics,
    the topics run one by one, and only then does conception start, from that knowledge. The
    research is not left until after conception. The check is a research task of its own kind,
    not a new role (D-158). It plans and its topics collect (D-168).
30. **"Agents see every scope by default"** is opt-out: a new scope is visible to every book at
    once. For existing data, every book sees every scope after the upgrade (the author's choice).
    Agents never re-enable a scope the author unchecked. Running a research task for the book, or
    finalizing, does, because the author asked for that scope.
31. **The auto-start checkbox decides what happens when the research before conception
    finishes**: conception starts by itself (default), or the book waits for **Start conception**.
    It fires at most once per book. It does not hold back the check's topics, which run either way
    (D-168).
32. **The author's research tasks are the author's.** The conception agent replaces only the
    topics it set itself. A topic the author added or edited stays.
33. **Conception's research topics are supplements** (D-168). The research before conception covers
    what the knowledge check found missing. The conception agent may still propose topics for
    knowledge that came up while conceiving and is still missing. They follow the old path: drafts,
    review, ready at finalize. A topic the check listed runs without a click, as "research first"
    requires. The author can edit, skip or delete it first.
34. **"A book agent that can access and change everything"** (D-169) means every content type
    of the book, plus starting outline, maintenance and research agents. Starting chapter writing
    and review was left out on the author's review of the plan ("not for now"). It never takes a
    decision the workflow reserves for the author: approving outlines and chapters, finalizing,
    proposals, reverts, merges, deleting chapters or books, and settings. "Full access and a review
    mode, the author chooses" is a per-conversation write mode. Review stops each change before it
    applies; it does not turn changes into proposals.
35. **"Writing is incremental; precise global planning is impossible"** (D-170). The author found
    foreshadowing payoffs planned at chapter numbers before chapter 1 existed, and asked for a
    structural constraint tied to the timeline. Read as:
    - the timeline holds history and plan in one tree, with unlimited depth and one empty root
      per book;
    - plans are anchored by relation (before, at or after an entry), not by chapter;
    - planned entries are seeded from the outline with no cap;
    - agents never see abandoned entries;
    - resolving against a planned anchor is a reminder, not a refusal.

    On the review of the plan the author added two rulings. Planned entries and goals
    "complement each other" (a goal can wait for a planned entry to happen), and a happened entry
    may not be abandoned. Existing books are not converted automatically.

## 11. Requirement traceability

| Requirement area | Product sections | Spec |
|---|---|---|
| Knowledge base and scopes | §6.1, F10 | `spec/07-knowledge-search.md` |
| Knowledge accumulation | §6.2, F3 | `spec/07`, `spec/05` (researcher tools) |
| Writing mode and granularity | §6.3, F4b | `spec/03` §Granularity, `spec/05` (drafting tools) |
| Chapter outlining | §6.12, F4 | `spec/03` §5.2, `spec/05` (planner tools), `spec/08` §2 |
| User participation | §6.4, F4 | `spec/03` §5.2–§5.4 (holds), §6 (control operations), §7 (edit reports) |
| Tiered memory | §6.5 | `spec/06-canon-memory.md` §Memory |
| Entity graph | §6.6, F7 | `spec/06` §Entities, §Relations |
| Timeline | §6.7, F7 | `spec/06` §Timeline (§1.4 tree and plan, D-170), `spec/05` §3, §6, `spec/10` §2.4 |
| Workflow mandate | §6.8 | `spec/08-workflows.md` |
| Review mode | §6.9, F5 | `spec/08` §Review, `spec/05` (reviewer tools) |
| Prompt management | §6.10, F8 | `spec/09-prompts.md` |
| Book conception | §6.11, F2 | `spec/08` §Conception, `spec/05` (conception tools) |
| Knowledge first | §6.1, §6.2, §6.11, F2, F10 | `spec/07` §1, §8.1, `spec/08` §5, `spec/05` §2, §7, §8, D-158 |
| Writing chapters | §6.12, F4 | `spec/08` §Chapter lifecycle |
| Evolving settings | §6.13, F6 | `spec/06` §Proposals |
| Side stories | §6.14, F9 | `spec/06` §Visibility, `spec/08` §Side stories |
| Writing goals | §6.16, F6, F7, F12 | `spec/06` §11, `spec/05` (`propose_goal_change`), `spec/03` §10, `spec/10` §2.4 |
| Canon system switches | §6.17, F7 | `spec/06` §10, `spec/05` §2, `spec/03` §1.2, §4, §10 |
| Book agent | §6.18, F13 | `spec/08` §8, `spec/05` §10, `spec/03` §5.2 (approval hold), §6, `spec/10` §2.2, §2.7, `spec/11` §5.3, D-169 |
| Software stack and i18n | §6.15 | `spec/01-architecture.md`, `spec/11-frontend.md` |
