# Implementation Plan: Knowledge First

> Milestone **M15** in `docs/PLAN.md`. Read `AGENTS.md` and `NOTES.md` first. The spec sections
> listed in §7 were amended together with this plan, and the decision is recorded as **D-158**.
> The spec is authoritative; this plan explains the reasons, the order and the tests.

## 0. What the author asked for, and the answers

The author's request, paraphrased:

1. **Knowledge accumulation comes before every other stage.** An agent first checks whether the
   existing knowledge base can support the conception of this book, then fills the gaps, so that
   conception starts with enough information. Research should not have to wait until conception
   is finished.
2. **A book's agents see every knowledge scope by default.** The author can uncheck scopes per
   book.
3. **The New book dialog gets a checkbox for starting conception automatically.**

Answers the author gave while this plan was written (binding):

| Question | Answer |
|---|---|
| Default of the auto-start checkbox | **On** |
| Existing books after the visibility change | **Every existing book sees every scope.** The pre-ship report lists, per book, the scopes that became visible |
| May the conception agent delete research tasks the author created? | **No.** `set_research_topics` today deletes every `draft` task of the book, including the author's. That must stop (K9) |

## 1. Decisions at a glance

| # | Decision |
|---|---|
| K1 | A book created with the conception agent first runs a **knowledge check**. This is a researcher session that inventories the scopes visible to the book, judges whether they can support conceiving *this* idea, fills the gaps from the web, and reports what it found. Conception starts after the check. |
| K2 | The knowledge check is a `research_tasks` row with **`kind = 'preparation'`**, run as an ordinary `research` session with the researcher role. There is no new session kind (no `agent_sessions` rebuild), no new role, and no new model setting. |
| K3 | The check chooses where to write: an existing scope, or a new broad one it creates with a description, through one new tool, **`set_target_scope`**. Scopes the author excluded for the book are refused. Every other researcher tool keeps its schema. |
| K4 | The New book dialog gets **"Start conception automatically after the knowledge check"**. It is shown only when "Start with the conception agent" is on, and it defaults to on. When it is on, completing the check starts the conception session once. When it is off, the book waits in `conceiving` and the studio offers **Start conception**. |
| K5 | Visibility becomes **opt-out**. `book_scopes` (attached scopes) is replaced by `book_scope_exclusions` (unchecked scopes). A new scope is visible to every book at once. |
| K6 | The **conception agent reads the knowledge base**. Its role gains `search_knowledge` and `read_knowledge_entry`. Its kickoff lists the visible scopes and carries the check's report. Its research topics become "what is still missing". |
| K7 | Agent actions never override an author's exclusion (`set_target_scope` refuses an excluded scope). Two author actions do clear an exclusion for their target scope, which is what attaching did before: **Run** on a book-linked research task, and **Finalize**. |
| K8 | The API stays backward compatible. `BookInput.knowledge_check` defaults to `false` and `auto_start_conception` defaults to `true`, so an old request behaves as it does today. The web client always sends `knowledge_check: true`. |
| K9 | **The conception agent never deletes the author's research tasks.** A new column `research_tasks.origin` (`author` \| `conception`) records who owns a task. `set_research_topics` replaces only `conception`-origin drafts. A task the author creates is `author`, and so is a conception draft once the author edits it: anything the author touched is the author's. The conception agent sees the tasks it cannot replace, so it does not duplicate them. |

## 2. Facts this plan rests on (current code, verified)

- **Visibility.** `services/knowledge.rs` `visible_scope_ids()` (≈ line 637) reads `book_scopes`.
  `agent/tools/search.rs` `visible_scopes()` (≈ line 533) gates `search_knowledge` and
  `read_knowledge_entry` with it, and gives research sessions their target scope only.
  `agent/context/mod.rs` `settings_context()` (≈ line 814) lists the attached scopes in
  `block.settings`. Rows are added by conception finalize (`services/conception.rs` ≈ 429) and by
  research `run` (`services/research.rs` `attach_scope`, ≈ 511). The only web reader is
  `BookKnowledgePage.tsx` (`useBookScopes`/`usePutBookScopes`).
- **Conception.** `CONCEPTION_TOOLS` (`agent/tools/registry.rs` ≈ 167, 10 tools) has no knowledge
  tool. The kickoff list is `["conception.kickoff"]` (`context/mod.rs` ≈ 49), and
  `vars.conception` holds title, idea, genre and writing language only. `books::create`
  (`services/books.rs` ≈ 42–59) starts conception right after the book commits, through
  `conception::start_for_new_book`. `POST /books/:id/conception/start` starts it later, and the
  studio already shows a **Start** button when there is no session (`ConceptionStudio.tsx` ≈ 273).
- **Research topics.** A task created for a `conceiving` book is `draft` (`research.rs` ≈ 158),
  is a setting material, and is locked while conception runs (`ensure_editable`).
  `set_research_topics` deletes **every** `draft` task of the book (`agent/tools/conception.rs`
  ≈ 935), whoever created it. An author who creates a task during conception and has not run it
  yet therefore loses it the next time the agent sets topics; the tasks that were run survive
  because they are no longer drafts. The conception kickoff does not list existing tasks. Finalize
  turns drafts `ready`. The studio's topics tab shows `status === "draft"` rows.
- **Research sessions.** The kickoff is `["block.book_brief", "researcher.kickoff"]`
  (`context/mod.rs` ≈ 58, `research_context` ≈ 1532). Writes are restricted to the task's
  target scope (`agent/tools/research.rs` `target_scope`, ≈ 84). The budget is
  `MAX_TOOL_CALLS = 40` and `MAX_ENTRIES = 25` (research.rs:25). `finish_research` requires the
  target scope (≈ 427). Task status follows the session through `research::on_session_status`,
  called inside `agent::session::transition` (session.rs ≈ 248). `research::finish` writes
  `done`, the report and `entries_created`.
- **Kind-specific tools within one role** already exist. `tools_for_session` (registry.rs ≈ 241)
  freezes `propose_goal_change` only for maintenance, and `is_permitted` restricts `canon_seed`.
- **Starting a session inside a transaction.** `sessions::create_inner(state, tx, CreateSession)`
  does this. `maintenance::start_after_approval` is the pattern to copy: it maps
  `model.missing_role` to a warning.
- **Migrations.** `0002` rebuilds `research_tasks` and `0004` rebuilds `agent_sessions` with
  explicit column lists. That is why `0006` can `ALTER TABLE agent_sessions ADD COLUMN
  canon_systems` on a fresh database even though `0001` also has the column. `0002` copies
  `research_tasks` with an explicit column list (`0002_research_deferred.sql` ≈ line 54, verified),
  so the same mechanism covers `research_tasks.kind` and `.origin` (§5.1).
- **Session titles.** The web composes them from `kind` plus `research_topic`
  (`SessionHeader.tsx` ≈ 55). A preparation task has an empty topic, so the DTO must say what the
  session is (§4.6).

## 3. Design — knowledge visibility (K5, K7)

### 3.1 Rule
`visible(book) = every knowledge scope − excluded(book)`. A scope is visible to every book from
the moment it is created. Deleting a scope deletes its exclusion rows through `ON DELETE CASCADE`.

### 3.2 Schema
```sql
CREATE TABLE book_scope_exclusions (
  book_id   INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  scope_id  INTEGER NOT NULL REFERENCES knowledge_scopes(id) ON DELETE CASCADE,
  PRIMARY KEY (book_id, scope_id)
);
CREATE INDEX book_scope_exclusions_scope ON book_scope_exclusions(scope_id);
```
`book_scopes` is dropped (§5.1).

### 3.3 Services
- `knowledge::visible_scope_ids(conn, book_id)` keeps its signature and order (by id). It returns
  every scope id that has no exclusion row for the book, so its callers do not change.
- `GET /books/:id/scopes` keeps its shape and meaning: the `ScopeDto[]` **visible** to the book,
  sorted by name.
- `PUT /books/:id/scopes` takes **`{ excluded_scope_ids: number[] }`** (rename the field of
  `BookScopesInput`). It replaces the book's exclusion set, refuses an unknown id with `404` as it
  does today, and returns the visible scopes. It publishes `book.updated` as it does today.
  Sending exclusions rather than the visible set is deliberate: a scope created after the page
  loaded is in neither list, so it stays visible instead of being excluded by a stale save.
- Add a private helper `knowledge::include_scope(tx, book_id, scope_id)` that deletes the
  exclusion row if there is one. It replaces `research::attach_scope` and finalize's attach step
  (K7).
- `settings_context()` lists `visible_scope_ids()` instead of the `book_scopes` rows. The code
  path is otherwise unchanged.

### 3.4 Cost note
Every visible scope is one more line in `block.settings` and one more KNN query for a
`search_knowledge` without `scope` (spec/07 §3). Under opt-out this grows with the global
library. That is accepted for the MVP: the author unchecks unrelated scopes. Record the risk in
D-158 and add no cap.

## 4. Design — the knowledge check (K1–K4)

### 4.1 The task row
- New column `research_tasks.kind TEXT NOT NULL DEFAULT 'topic' CHECK (kind IN
  ('topic','preparation'))` and a `ResearchTaskKind { Topic, Preparation }` enum (`string_enum!`
  in `db/json.rs`). `ResearchTaskDto` gains `kind`.
- A preparation task:
  - always has a `book_id`;
  - has `topic = ''`, and the UI shows the localized name "Knowledge check";
  - keeps the author's optional focus in `requirements`;
  - has `scope_name = ''` and `scope_id = NULL` until `set_target_scope` sets both;
  - uses the book's writing language as `language`;
  - is **never `draft`**. It is created and run in one transaction, so it is never left `ready`
    either.
- **At most one open preparation task per book**: a second check is refused while one is
  `running` or `failed` (`409 research.invalid_state`, `details.session_id`).
- A preparation task is not a setting material:
  - `ensure_topics_editable` and the `settings.locked` rule do not apply to it;
  - it never bumps `ConceptionState.topics_revision`;
  - the conception view snapshot and `notice.setting_edits` ignore it. Check how the snapshot
    selects research topics, and filter it to `kind = 'topic'`;
  - `set_research_topics` deletes only `kind = 'topic'`, `origin = 'conception'` drafts (§4.8),
    and finalize's draft → ready only touches `kind = 'topic'` rows. Filter explicitly, even
    though a preparation task is never a draft.
  - Its `origin` is `author`.
- `PATCH` of a preparation task accepts `requirements` only. `topic` or `scope_name` returns
  `422 validation.invalid_fields`. `DELETE`, `run` (re-run), `defer` and `resume` follow the
  existing table unchanged.
- `run` and `prepare_scope` skip scope resolution for a preparation task. The agent chooses the
  target, and a re-run keeps the previous target until the agent switches it.

### 4.2 Starting a check
**`POST /api/books/:id/knowledge-check`**, body `{ requirements?: string }`, returns
`201 { task: ResearchTaskDto, session_id: number }`.

Preconditions, in this order:
1. The book exists and is `conceiving` or `active`; otherwise `409 book.invalid_state`.
2. The book has no open preparation task; otherwise `409 research.invalid_state` with
   `details.session_id`.
3. The researcher model resolves; otherwise the same error `run` gives today. The task row and
   the session are created in **one** transaction with `sessions::create_inner`, so a failure
   leaves no row behind.

The same service function, `research::start_knowledge_check(state, tx, book_id, requirements)`,
serves book creation (§4.5). There it maps `model.missing_role` to a warning.

### 4.3 The session
- Kind `research`, role researcher, `research_task_id` = the preparation task.
- Kickoff list: `["block.book_brief", "researcher.kickoff.prep"]` when the task is a preparation
  task, otherwise unchanged.
- **New prompt key `researcher.kickoff.prep`** (kind `task`, agent `researcher`, vars `["prep"]`).
  New `vars.prep` in `variables.toml`:
  - `requirements`: the author's focus (may be empty);
  - `language_name`;
  - `scopes[]`: `{ name, description, entry_count, titles[], more }` for every scope **visible**
    to the book, in name order. `titles` lists at most 50 entry titles per scope and 300 in total
    (constants next to `MAX_EXISTING_TITLES`), and `more` counts the titles left out;
  - `excluded_names[]`: the names of the scopes the author excluded for this book;
  - `max_tool_calls`, `max_entries`: the research constants (no new budget).
- **What the kickoff must tell the model** (the executor writes it in Chinese, in the pack's
  style, using the `{% if %}` branches the samples cover):
  1. Judge, from the title, idea and genre, what knowledge conceiving this book needs: real-world
     facts (era, geography, institutions, professions, technology) and genre knowledge
     (conventions, power systems, reader expectations, overused tropes).
  2. Check the inventory and use `search_knowledge` / `read_knowledge_entry` to verify coverage
     and depth. Do not trust titles alone.
  3. Fill only the gaps. First `set_target_scope`: prefer an existing broad scope on the same
     subject, otherwise create one broad scope named after the book's subject or era, following
     the one-to-two scope budget (D-145). Then `web_search`/`web_fetch` and
     `save_knowledge_entry`, or `update_knowledge_entry` for a thin entry in the target scope.
  4. Never duplicate an existing entry. If the base is already sufficient, add nothing.
  5. Finish with `finish_research`. The report covers what was already sufficient, what was added
     (titles), and what is still missing or needs the author's decision. **The report is handed to
     the conception agent.**
  6. An empty inventory renders its own branch ("the knowledge base is empty"), and the model
     then skips the searches.
- `researcher.system` stays static. Change it only where a sentence contradicts a check (for
  example, one that says every task must produce entries), and keep the edit minimal.

### 4.4 Tools in a check
- `RESEARCHER_TOOLS` becomes `["web_search", "web_fetch", "search_knowledge",
  "read_knowledge_entry", "set_target_scope", "save_knowledge_entry", "update_knowledge_entry",
  "finish_research"]`.
- `tools_for_session` gains a `preparation: bool` argument. It freezes `set_target_scope` only
  for a preparation task. A topic research session therefore freezes exactly the old 7 tools in
  the old order, and its baseline snapshot does not change.
- **`set_target_scope { name, description? }`** goes in `tools.toml` (description, examples,
  schema), with a handler in `agent/tools/research.rs` and a new result template
  `result.target_scope` (`scope`, `created`, `entry_count`, spec/09 §2):
  - trims `name` and validates it with the same rules as scope creation in
    `services/knowledge.rs`;
  - for an existing scope that the author **excluded** for this book, returns a new tool error
    kind `scope_excluded` (a `result.error` branch: the scope was excluded by the author for this
    book, choose another);
  - for an existing visible scope, sets `task.scope_id` and `task.scope_name`, and **ignores**
    `description` (an agent never rewrites a shared description);
  - for a new name, requires `description` (otherwise the existing `validation` error), creates
    the scope through the knowledge service so `knowledge.changed` is published, and sets the
    task;
  - may be called again to switch targets. Entries already saved stay where they are;
  - returns a result with the name, whether the scope was created, and its entry count. Copy
    `save_knowledge_entry`'s `ToolKind`.
- `target_scope()`: for a preparation task with no target, it returns a new tool error kind
  `target_scope_unset` (a `result.error` branch: call `set_target_scope` first) instead of
  `no_knowledge_scope`.
- `search.rs` `visible_scopes()`: a research session whose task is a preparation task searches
  the **book's visible scopes**. A topic task keeps its target scope only.
- `finish_research`: for a preparation task, a missing target scope is allowed, and the task
  becomes `done` with `entries_created` as today (possibly 0). Read the existing count and
  short-report checks (`m8_review_finish_count.rs`) and keep them.
- `scripts/validate_prompts.py`: update the researcher list (8) and the conception list (12, see
  §4.7). Add the new key's samples, and the result and error branches for both new tool error
  kinds.
- **Demo provider** (`crates/llm/demo`): add a check arm that calls `search_knowledge` →
  `set_target_scope` (new scope with a description) → `save_knowledge_entry` → `finish_research`.
  Pick the arm the same way the existing research arm is picked; read that code first.

### 4.5 Book creation (K4, K8)
- `BookInput` gains `knowledge_check: bool` (`#[serde(default)]`, false) and
  `auto_start_conception: bool` (serde default **true**). Both are ignored when
  `start_conception` is false.
- `CreateBookResponse` gains `research_task_id: Option<i32>`. `session_id` stays "the session
  started for the book": the check's session, or the conception session when no check ran.
- `ConceptionState` gains `start_after_knowledge_check: bool` (`#[serde(default)]`, so old rows
  read false). Export it with ts-rs.
- `books::create`, after the book commits, for a `conceiving` book:

  | `knowledge_check` | `auto_start_conception` | Effect |
  |---|---|---|
  | true | any | `start_knowledge_check`. If it started, set `start_after_knowledge_check = auto_start_conception` in the same transaction. If it could not start (`model.missing_role`), push the warning, then behave like the `false` rows below |
  | false | true | today's path: `conception::start_for_new_book` |
  | false | false | nothing starts: the book is `conceiving` with no session |

### 4.6 Auto-start after the check
- **Trigger**: a preparation task's session reaches `completed`.
- **Conditions**, all re-checked in the write that starts the session:
  1. the task is a preparation task of book B;
  2. B is `conceiving`;
  3. `B.conception.start_after_knowledge_check` is true;
  4. B has **no** conception session row in any status. Auto-start happens once: it never
     second-guesses an author who started conception by hand or abandoned it.
- **Effect**: `conception::start_after_knowledge_check(state, tx, book_id)`, modeled on
  `maintenance::start_after_approval`. It calls `sessions::create_inner` for a `Conception`
  session and sets `start_after_knowledge_check = false` in the same transaction.
  `model.missing_role` means no session and no error. The studio then shows Start. Any other
  error is logged at `warn` and must not fail the check's completion.
- **Where**: in the transaction that commits the check's `completed` status, if that path has
  `AppState`. Otherwise run it as a post-commit step of the runner's finalize, in its own
  `Db::write`. Never hold a transaction across a session spawn, and publish the new session's
  events after the commit.
- A check that ends `failed`, `cancelled` or abandoned starts nothing.
- `POST /conception/start` stays allowed while a check is open. That is the author's explicit
  choice, the studio asks first (§4.9), and condition 4 then prevents a second session.

### 4.7 Conception reads knowledge (K6)
- `CONCEPTION_TOOLS` (12): `["ask_user", "search_knowledge", "read_knowledge_entry",
  "web_search", "web_fetch", "write_setting_doc", "edit_setting_doc", "upsert_character",
  "remove_character", "reorder_characters", "set_research_topics", "request_review"]`. The book's
  visible scopes gate both knowledge tools. Verify that `book_id_of(ctx)` resolves for a
  conception session.
- `vars.conception` gains:
  - `scopes[]`: `{ name, description, entry_count }` of the visible scopes, in name order;
  - `knowledge_report`: the report of the book's latest `done` preparation task, otherwise
    empty.

  Update the Rust context struct, `sample_contexts` (one sample per branch: no scopes, scopes
  without a report, scopes with a report), `variables.toml` and the validator samples.
- `conception.kickoff` adds a knowledge section. It lists the scopes (or says the base is empty),
  includes the check's report when there is one, and tells the model to consult it.
- `conception.system` changes in two places:
  - **Working method**: consult the knowledge base with `search_knowledge` /
    `read_knowledge_entry` first, and use `web_search` / `web_fetch` only for what it lacks.
  - **Deliverable 5**: list as research topics only what the knowledge base still lacks, never
    repeat a topic its entries already cover, and prefer the name of an existing scope.

### 4.8 The author's research tasks are protected (K9)
- **Column**: `research_tasks.origin TEXT NOT NULL DEFAULT 'author' CHECK (origin IN
  ('author','conception'))`, enum `ResearchTaskOrigin { Author, Conception }`, and
  `ResearchTaskDto.origin`.
- **Who sets it**:
  - `POST /api/research-tasks` (every UI: Book › Knowledge, global `/research`, the studio's
    topics editor) → `author`.
  - `POST /books/:id/knowledge-check` → `author`.
  - `set_research_topics` → `conception`.
  - An author `PATCH` of a `conception` task sets it to `author` in the same write, because the
    author adopted it. The agent has no tool that edits a single task, so nothing ever turns an
    `author` task back into `conception`.
- **`set_research_topics`** deletes only the book's `draft` tasks with `kind = 'topic'` and
  `origin = 'conception'`, then inserts its topics as `conception` drafts. It never deletes,
  edits or re-orders an `author` task in any status.
- **The agent knows what it cannot replace.**
  - `vars.conception` gains `research_tasks[]`: `{ topic, requirements, scope_name, status }`
    for every `kind = 'topic'` task of the book that `set_research_topics` would **not** delete
    (author tasks in any status, and conception tasks that were run), in id order.
  - `conception.kickoff` lists them under a "research topics that already exist" heading and
    tells the model not to repeat them in `set_research_topics`.
  - The `set_research_topics` result (`result.conception_write`) gains `kept[]` (the topics of the
    author's drafts it left in place), so the model sees them again after every call.
  - Update `tools.toml`'s description of `set_research_topics`. It currently says it replaces
    every not-yet-run topic; it should say it replaces only the topics the agent set itself, and
    that the author's topics stay.
- **Mid-session author edits** are already reported through `notice.setting_edits` as topic
  changes. Check that a protected author draft added while the agent is idle shows up there, and
  that the snapshot keys rows by id so adoption by `PATCH` is reported as an edit, not as a
  delete plus an insert.
- **Finalize** is unchanged. It turns every `kind = 'topic'` draft, of either origin, `ready`.
- **UI**: the studio's topics tab marks author rows with a small "Added by you" label (en and
  zh-CN). Nothing else changes.

### 4.9 Web
- **New book dialog**: under "Start with the conception agent", an indented checkbox "Start
  conception automatically after the knowledge check" (default on, rendered only while
  conception is on). The conception description now says that a knowledge check runs first. The
  payload sends `knowledge_check: true` and `auto_start_conception`. **Landing**: a `conceiving`
  book opens `/books/:id/conception`, otherwise `/books/:id`. This replaces the `session_id` test.
- **Conception studio when the book has no conception session.** It picks the latest
  preparation task from the research tasks it already loads.
  - **Check open**: a "Knowledge check" header with the session status and the check's
    `ActivityStream`, a link to the research task page (steering, pause and stop live there), and
    **Start conception now**. That button asks for confirmation ("the check is still running; it
    keeps running").
  - **Check done**: the report (Markdown), the number of entries created, a link to
    Book › Knowledge, and **Start conception**, which is primary when auto-start was off.
  - **Check failed or cancelled**: the status, a link to the task, and **Start conception**.
  - **No check**: today's "no session" text with **Start conception**, plus **Check knowledge
    first**, which calls the endpoint.
  - When a conception session exists, the studio is unchanged. The page reacts to
    `session.updated` / `research.updated` without a reload, so auto-start swaps the pane.
- **Book › Knowledge**:
  - New hint text: agents in this book read every checked scope, and new scopes are checked
    automatically.
  - Checked = visible. Save sends the unchecked ids.
  - A **Check knowledge** button opens a small dialog with an optional focus and calls the
    endpoint, then navigates to the task page. It is disabled with a reason while a check is open.
- **Research list, task page and global `/research`**: a preparation task shows "Knowledge check"
  as its title, the target scope or "—", and no topic or scope editing.
- **Session title**: `SessionDto` gains `research_task_kind: Option<ResearchTaskKind>`, and
  `SessionHeader` uses `agent:session_title.knowledge_check` for a preparation task.
- **Global Knowledge page**: the scope-creation dialog gets a one-line hint that a new scope is
  visible to every book unless a book unchecks it.
- Every new string goes in `en` and `zh-CN`.

## 5. Data, API and type changes

### 5.1 Migration `0008_knowledge_first`
Run it in a Rust-managed transaction like `m0006` (the `ALTER` is not idempotent), with
`use_transaction() = Some(false)` and no down migration.
```sql
CREATE TABLE IF NOT EXISTS book_scope_exclusions (...);           -- §3.2
CREATE INDEX IF NOT EXISTS book_scope_exclusions_scope ON book_scope_exclusions(scope_id);
DROP TABLE IF EXISTS book_scopes;                                 -- author: all scopes visible
ALTER TABLE research_tasks ADD COLUMN kind TEXT NOT NULL DEFAULT 'topic'
  CHECK (kind IN ('topic','preparation'));
ALTER TABLE research_tasks ADD COLUMN origin TEXT NOT NULL DEFAULT 'author'
  CHECK (origin IN ('author','conception'));
-- Back-fill: who created an existing draft is not recorded. Drafts are treated as the agent's, so
-- in-flight conceptions keep today's replace behavior; every other row is the author's (only
-- drafts can ever be replaced, so the choice for them changes nothing).
UPDATE research_tasks SET origin = 'conception' WHERE status = 'draft';
```
- Also edit `0001_init.sql` to the final schema: remove `book_scopes`, add the exclusions table,
  and add `research_tasks.kind` and `research_tasks.origin`. spec/02 §2 already shows that
  schema. The two `ALTER`s still succeed on a fresh database, because `0002` rebuilds
  `research_tasks` from an explicit column list and so drops `0001`'s new columns (verified). The
  migration test proves both paths end in the same schema.
- Add a guard in the 0006 style: the `research_tasks` row count is equal before and after.
- The pre-ship report (15.7) also counts the drafts back-filled as `conception` per conceiving
  book. Those are the tasks the author could still lose to the next `set_research_topics`, and
  editing one adopts it.
- Register `m0008` in `crates/migration/src/lib.rs`.

### 5.2 Entities and types
- Delete the `book_scopes` entity. Add `book_scope_exclusions` (composite primary key, the two
  `belongs_to` relations). Replace the `has_many … via = "book_scopes"` relations on `books` and
  `knowledge_scopes`; dropping the M-N relation is fine if nothing uses it. Update
  `tests/entity_schema.rs` (table list, fixture inserts).
- `research_tasks.kind: ResearchTaskKind`, `research_tasks.origin: ResearchTaskOrigin`.
  `ResearchTaskDto.kind`, `ResearchTaskDto.origin`. `SessionDto.research_task_kind`.
- `BookInput.knowledge_check`, `BookInput.auto_start_conception`,
  `CreateBookResponse.research_task_id`, `ConceptionState.start_after_knowledge_check`,
  `BookScopesInput.excluded_scope_ids`. New `KnowledgeCheckInput { requirements }` and
  `KnowledgeCheckResponse { task, session_id }`.
- New tool error kinds `scope_excluded` and `target_scope_unset`. There are no new HTTP error
  codes.
- Regenerate the bindings (`cargo test -p nekowriter export_bindings`), and commit
  `web/src/api/gen`.

## 6. Tasks — milestone M15

Execute them in order. One commit per task, `scripts/check.sh` green before each commit.

| # | Task | Tests / acceptance |
|---|---|---|
| 15.1 | **Docs** — done with this plan (D-158, §7). Re-read the amended sections before 15.2. If code facts contradict them, fix the section and note it in D-158. | `check.sh` green (docs only) |
| 15.2 | **Schema and types** (§5): 0008 plus the 0001 edit, entities, enums, JSON fields, DTOs, bindings. | `knowledge_first_migration.rs`: a pre-0008 fixture (two books, three scopes, attachments for one book, research tasks in every status, one research session) upgrades with every research task intact and `kind = 'topic'`; drafts get `origin = 'conception'` and every other row `author`; `book_scopes` is gone and the exclusions table is empty; a fresh database and the upgraded one pass `entity_schema.rs`. JSON: `ConceptionState` without the field → false; `BookInput` without the new fields → `knowledge_check = false`, `auto_start_conception = true`. |
| 15.3 | **Opt-out visibility** (§3): services, `PUT` body, `include_scope` in run and finalize, `settings_context`. Rewrite the tests that assumed "not attached ⇒ invisible" as "excluded ⇒ invisible" (start with `knowledge_api.rs::book_scopes_attach_and_detach_visibility`, `m7_review_tools.rs`, `search_tools.rs`, `m7_review_context.rs`, and `grep -rn book_scopes crates/server/tests`). | `knowledge_visibility.rs`: a scope created after two books exist is visible to both with no action (AC1); excluding it removes it from `GET /books/:id/scopes`, `block.settings`, and `search_knowledge` / `read_knowledge_entry` results (`no_knowledge_scope` or not found, as today); `PUT` with an unknown id → 404 and no change; running a book-linked task whose target is excluded makes it visible again; finalize does the same for a topic's scope; deleting a scope removes its exclusion rows. |
| 15.4 | **Knowledge check backend** (§4.1–§4.4): task kind rules, endpoint, `prepare_scope` skip, `researcher.kickoff.prep` with context, variables, samples and validator, `set_target_scope`, the two error kinds, `tools_for_session(preparation)`, prep visibility in `search.rs`, `finish_research` without a target, `SessionDto.research_task_kind`, Demo arm. | `knowledge_check.rs` with `ScriptedProvider`: the kickoff lists visible scopes with capped titles and `more`, and the excluded names, never an excluded scope (AC4); `search_knowledge` spans the book's visible scopes; save before a target → `target_scope_unset`; existing target → entry saved there and the description untouched; new name without a description → `validation`; new with one → scope created and visible to another book; excluded name → `scope_excluded`; switching targets keeps earlier entries in place; finishing with no target and no entries → `done`, `entries_created = 0`; a second check while one is open → 409 with `session_id`; archived book → 409; no researcher model → error and **no** row; PATCH `topic` → 422; creating and deleting the check while conception runs is not `settings.locked`. A topic research session freezes exactly the old 7 tools (AC8). `prompts_registry.rs` snapshots for the new key's samples. |
| 15.5 | **Creation, auto-start, conception knowledge, protected tasks** (§4.5–§4.8): `BookInput` handling, `start_after_knowledge_check`, the hook, the conception tool list, the kickoff variables (`scopes`, `knowledge_report`, `research_tasks`) and prompts, `origin` on create/PATCH/`set_research_topics`, the `kept[]` result, the topics-tab label. | `research_topics_ownership.rs`: an author draft created before conception and one created while it is idle both survive `set_research_topics`, and both are listed in the next kickoff or in the tool's `kept[]` (AC11); an agent draft is replaced; an agent draft the author PATCHed becomes `author` and survives; a run agent task survives (unchanged rule); finalize turns drafts of both origins `ready`; `notice.setting_edits` reports an adoption as an edit. Then: | `knowledge_first_flow.rs`: create with `{start_conception, knowledge_check, auto_start_conception: true}` → the check's session, no conception session (AC3); the scripted check completes → exactly one conception session starts, its kickoff contains the scopes and the report, and its frozen tools include both knowledge tools (AC5, AC6); `auto_start_conception: false` → nothing starts after completion, and `conception/start` works; a check abandoned or failed → nothing starts; the author starts conception during the check → completion starts none (one conception session in total); re-running the check after conception was abandoned → nothing starts; no researcher model → warning, with conception started at once when auto-start is on and nothing when it is off; no conception model at completion → the check is `done`, no session, no error; conception `search_knowledge` sees visible scopes and not excluded ones; `set_research_topics` and finalize leave the preparation task untouched (AC7). The old request shape (no new fields) still starts conception immediately (AC10). Update `kickoff_baseline` **only** for the conception kickoff and the conception tool list, and justify any other diff in D-158. |
| 15.6 | **Web** (§4.9), both locales. | Vitest: the dialog hides the auto-start box when conception is off, sends both fields, and lands on the studio for a `conceiving` book; Book › Knowledge starts with all scopes checked, unchecking sends `excluded_scope_ids`, and a scope missing from the loaded list is not sent; studio states (check open → stream, link and confirmed Start; done → report and Start; failed → Start; no check → Check knowledge first) and the swap to the conception session on `session.updated`; the research list and session header show "Knowledge check"; `i18n:check` (AC9). |
| 15.7 | **Smoke and pre-ship.** Run smoke item 12 (§8) with the Demo provider. Run 0008 on a **copy** of the author's database. Before migrating, report per book the scopes that become newly visible (all scopes minus that book's `book_scopes` rows) and the number of research tasks. After migrating, confirm that the counts match. Put the results in D-158's consequences and in the final message. | the checklist passes; the report is in the final message |

## 7. Documents amended with this plan

These sections already describe the M15 behavior. The code catches up task by task.

- **PROJECT**:
  - §2: the knowledge base row says every book sees every scope unless the author unchecks it.
  - §4: Knowledge base; a new term **Knowledge check**.
  - §5: the Researcher also runs the knowledge check; Conception's typical tools gain
    `search_knowledge`.
  - §6.1: opt-out visibility.
  - §6.2: the knowledge check comes before conception.
  - §6.11: the knowledge check, then conception.
  - F2: the dialog's new checkbox, the check first in the studio, and auto-start.
  - F3: a research task can also be the knowledge check.
  - F10: unchecking, and **Check knowledge**.
  - §8.7: the studio states.
  - §8.10: labels and the scope hint.
  - §10: a new interpretation entry quoting §0.
  - §11: traceability.
- **spec/02**: §2 DDL; §3 `ResearchTaskKind`, `ResearchTaskOrigin`;
  §4 `ConceptionState.start_after_knowledge_check`; §5 invariants (a preparation task has a book,
  at most one is open per book, and it is never `draft`; agents replace only `conception`-origin
  drafts).
- **spec/03**: §10 kickoff table (preparation research, conception variables); §15 new test
  numbers for AC3–AC6.
- **spec/05**: §2 role lists (researcher 8 with `set_target_scope` frozen for preparation only;
  conception 12); §3 `search_knowledge` visibility; §7 `set_research_topics` (only its own
  drafts, `kept[]`); §8 `set_target_scope` and `finish_research` in a check; the two error kinds.
- **spec/07**: §1 visibility (opt-out, `include_scope`); §8 a new subsection "Knowledge check"
  (§4.1–§4.4, §4.6).
- **spec/08**: §5 the create table, auto-start, and Start conception while a check is open.
- **spec/09**: §2 the key `researcher.kickoff.prep`.
- **spec/10**: §2.2 `BookInput`, `CreateBookResponse`; §2.5 the `PUT /books/:id/scopes` body,
  `POST /books/:id/knowledge-check`, `ResearchTaskDto.kind` and `.origin`, PATCH rules; §2.7
  `SessionDto.research_task_kind`.
- **spec/11**: §5 the New book dialog, the studio states, Book › Knowledge, the session title.

## 8. Acceptance criteria and smoke item 12

| AC | Statement | Proof |
|---|---|---|
| AC1 | A new scope is visible to every book at once; excluding it hides it from every agent surface | 15.3 |
| AC2 | After 0008 every existing book sees every scope, and no research task is lost | 15.2, 15.7 |
| AC3 | A book created in the UI with conception runs the check before any conception session | 15.5 |
| AC4 | The check inventories the visible scopes, writes only into a scope it chose (never an excluded one), and may finish without adding anything | 15.4 |
| AC5 | With auto-start on, completing the check starts conception exactly once; with it off, nothing starts | 15.5 |
| AC6 | Conception sees the knowledge: both tools, the scope list, and the check's report | 15.5 |
| AC7 | `set_research_topics` and finalize never touch the check task | 15.5 |
| AC8 | Topic research sessions are unchanged (tools and kickoff). The only baseline snapshots that change are the conception kickoff and the conception tool list | 15.4, 15.5 |
| AC9 | The UI states of §4.9, in both locales | 15.6 |
| AC10 | An API request without the new fields behaves as today | 15.5 |
| AC11 | The conception agent never deletes or edits a research task the author created or edited, and it sees them so it does not duplicate them | 15.5 |

**Smoke item 12 (append to PLAN §M11.5):**
1. On the global Knowledge page, create a scope with one entry. Create a book with conception and
   auto-start on. The studio shows the knowledge check, whose kickoff (session info popover)
   lists the scope. The check sets a target scope and saves an entry, and when it finishes,
   conception starts by itself. The conception kickoff shows the scopes and the report, and its
   tool list has `search_knowledge`.
2. In Book › Knowledge every scope is checked. Uncheck one and save. The next session's
   `block.settings` no longer lists it.
3. Create a second book with auto-start off. After the check, the studio shows the report and
   **Start conception**, and nothing starts by itself. Start it.
4. Create another scope on the global page. Both books see it without any action.
5. In the second book, before starting conception, add a research topic in the studio's
   Research topics tab. Start conception and let the agent set its topics. Your topic is still
   there, marked "Added by you".

## 9. Risks

| Risk | Mitigation |
|---|---|
| Opt-out makes `block.settings` and keyword-less `search_knowledge` grow with the global library | Authors uncheck unrelated scopes. Revisit with a cap if it hurts (D-158 consequence) |
| Knowledge from another book's subject leaks into an unrelated book | Scope descriptions are shown to agents, the prompts say to use relevant knowledge only, and the author unchecks |
| Auto-start chains two paid sessions without a click | The checkbox. The check has the research budget |
| The author starts conception while the check runs | The confirmation dialog. Condition 4 prevents a second session |
| The `ALTER` on a fresh database collides with the edited `0001` | `0002`'s explicit column list drops them first (verified, §5.1). The migration test covers both paths |

## 10. Out of scope (do not build)
- Running conception's research topics automatically, or a queue that runs several research
  tasks.
- A knowledge check for side tracks, or one triggered automatically for books created without
  conception (the **Check knowledge** button covers them).
- Per-session scope selection, scope caps, or moving entries between scopes.
- Feeding the setting materials into the check's kickoff (book brief only).
- Reordering the navigator, or changing the overview's next-action rules (a conceiving book
  still shows `continue_conception`, which opens the studio and its new states).
