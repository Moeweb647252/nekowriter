# Implementation Plan: Research Before Conception

> Milestone **M20** in `docs/PLAN.md`. Read `AGENTS.md` first. The spec sections listed in §7 were
> amended together with this plan, and the decision is recorded as **D-168**. The spec is
> authoritative; this plan explains the reasons, the order and the tests. It builds on M15
> (`knowledge-first-plan.md`, D-158) and corrects part of it.

## 0. What the author asked for, and the answers

The author's question, paraphrased: *"Why are research topics still submitted during conception?
I said research comes first."*

When M15 was planned, the author explained the request like this: an agent first checks whether
the knowledge base can support conceiving the book, then fills the gaps, "so that conception has
enough information, **instead of running research after conception**". M15 added a knowledge check
before conception, but it kept the research topics as a conception deliverable. Those topics
become `ready` at finalize, and the overview then suggests running them. So most of the research
still runs after conception. The code does what the M15 plan said; the plan was wrong.

Answers the author gave while this plan was written (binding):

| Question | Answer |
|---|---|
| What should the gap filling before conception look like? | **The check lists topics, they run one by one, then conception starts.** The knowledge check judges what is missing and writes it as research topics. The studio shows them, and the author can add, edit and delete them. Each topic runs as its own research session, in order, with its own budget. Conception starts when all of them have finished, still governed by the auto-start checkbox |
| May the conception agent still propose research topics? | **Yes, as supplementary topics** (the author's words: "可以提作为补充的资料主题") |

## 1. Decisions at a glance

| # | Decision |
|---|---|
| R1 | **The knowledge check plans instead of collecting.** It inventories and verifies the knowledge visible to the book, then writes what is missing as research topics (at most 8) and finishes with a report. Its tools are `search_knowledge`, `read_knowledge_entry`, `set_research_topics` and `finish_research`. It has no web tools and saves no entries. |
| R2 | **A check's topics are ordinary topic tasks linked to the check.** A new column `research_tasks.check_id` points at the preparation task. A check topic is `ready` as soon as it exists, whether the book is `conceiving` or `active`; it is never `draft`. The check writes its topics with `origin = 'conception'` (the agent-owned value); the author adds topics with `origin = 'author'`. |
| R3 | **The topic queue.** Once the check is `done`, its `ready` topics run **one at a time**, in id order. Each runs as a normal research session with the frozen research budget (D-164). A topic that fails, is stopped or is interrupted holds the queue until the author continues or abandons it. Abandoning a topic skips it, and deferring a waiting topic takes it out of the queue. The queue has no state of its own: it is derived from the rows. |
| R4 | **Conception auto-starts when the check's research has settled.** That means the check is `done` and none of its topics is `ready` or open. It no longer happens when the check's session completes. With no topics, both moments are the same, as today. The conditions of spec/08 §5 (conceiving, flag, no conception session) are unchanged. |
| R5 | **The author edits the list.** Waiting topics can be edited, deleted and deferred. New topics are added at the end with `POST /research-tasks` and a `check_id`, and adding one to an idle queue starts it. Check topics are not setting materials: `settings.locked`, `topics_revision` and the conception view snapshot ignore them. |
| R6 | **The conception agent's topics are supplementary.** They use the same tool and the same draft and finalize mechanics. The prompts now say: only knowledge that came up during conception and that the knowledge base still lacks; often none. The author can run one at once from its task page, which already runs a draft. |
| R7 | **One tool, two owners.** `set_research_topics` keeps its name, schema and description, which is already generic ("replaces only the topics you set yourself that have not run"). In a conception session it replaces the agent's own drafts, as today. In a knowledge check it replaces the check's own waiting topics. The behavior depends on the session. |
| R8 | **`set_target_scope` is retired.** No new session freezes it. Its handler, its `tools.toml` entry, its result template and its two error kinds stay, because a check session frozen before this milestone may still call them (D-159: historical shapes stay). |
| R9 | **Exclusions.** The check's `set_research_topics` refuses a scope name the author excluded for the book (`error.scope_excluded`). A queued topic runs the way **Run** does, so a run can re-include its scope (spec/07 §8 step 2). The author has seen the list and can delete or defer the topic. |
| R10 | **A new scope gets a description.** When running a check topic creates a scope, the scope's description is the topics of that check's tasks with the same scope name, joined with ` / `. This is the rule finalize uses (spec/08 §5 step 4). An agent never writes a shared description. |

## 2. Facts this plan rests on (current code, verified at `48db009`)

- **M15's wording.**
  - PROJECT §6.2 says "Conception still produces research tasks, now only for what is still
    missing. The author reviews them and runs them", and §10 #29 says the same.
  - In the prompts:
    - deliverable 5 of `conception.system` (`prompts/system.toml` ≈ line 157) asks for research
      topics;
    - `conception.kickoff` (`prompts/tasks.toml` ≈ line 31) tells the agent to draft them.
  - In the services:
    - finalize turns `draft` topics `ready` (`services/conception.rs` `finalize_inner`);
    - the overview's rule 3 then suggests running them (spec/08 §7).
- **The check today.**
  - `research::start_knowledge_check` (`services/research.rs` ≈ 251) creates the preparation task
    and its session in one transaction.
  - A preparation session freezes all eight `RESEARCHER_TOOLS` (`agent/tools/registry.rs` ≈ 154).
    `tools_for_session` (≈ 215) filters `set_target_scope` out of topic sessions only.
  - `researcher.kickoff.prep` (`prompts/tasks.toml` ≈ 58) tells the check to pick a target scope,
    search the web and save entries.
  - The Demo arm `prep_plan` (`crates/llm/src/demo.rs` ≈ 636) recognises a check by
    `set_target_scope` in `hints.tools`.
- **Auto-start today.**
  - After a `finish_research` commit, the runner calls
    `conception::maybe_start_after_knowledge_check` (`agent/runner.rs` ≈ 1166).
  - `start_after_knowledge_check` (`services/conception.rs` ≈ 446) checks the session, the task,
    the book, the flag and that no conception session exists. It creates the conception session in
    its own write, and the session is spawned after the commit.
- **Abandon.** `sessions::abandon` (`services/sessions.rs` ≈ 1607) moves the session to
  `abandoned` in one write and then publishes. `research::on_session_status` (≈ 771) maps a
  session status to a task status:

  | Session | Task |
  |---|---|
  | `abandoned` | `cancelled` |
  | `failed`, `interrupted` | `failed` |
  | `running` | `running` |

  `finish_research` writes `done` itself.
- **Run.** `research::run` (≈ 573) has two paths.
  - A `failed` task with a session continues on that session (D-163).
  - Otherwise `prepare_scope` (≈ 621) resolves or creates the scope (`resolve_scope` ≈ 673 creates
    it with an empty description) and re-includes it for the book, in one write. Then
    `sessions::create` runs in another write.

  `is_open` is true for `running`, and for `failed` with a session. A `draft` is runnable, and
  running it confirms it.
- **Create.** `research::create_inner` (≈ 114) calls `setting_docs::ensure_editable` for every task
  of a book. It makes a task `draft` when the book is `conceiving`.
- **Schema.**
  - `research_tasks` comes from 0001, is rebuilt by 0002 from an explicit column list, and 0008
    adds `kind` and `origin`.
  - `origin` has `CHECK (origin IN ('author','conception'))`. A third value would need a table
    rebuild, so R2 reuses `conception` as the value for an agent-owned task.
- **Frozen definitions.** `agent_sessions.tools` stores each tool's full definition: name,
  description and schema. A check session frozen before this milestone therefore keeps
  `set_target_scope` and its old kickoff.
- **The author's database** (read-only, 2026-11-03), which is at migration 0010:
  - one `conceiving` book (#1);
  - a `done` old-style check (#1, 19 entries);
  - conception session #4, `awaiting_input`;
  - five `conception` drafts (#2–#6);
  - no open check.

  0011 changes no row in it. The five drafts simply count as supplementary topics now.

## 3. Design

### 3.1 Schema: `0011_check_topics.sql`

```sql
ALTER TABLE research_tasks ADD COLUMN check_id INTEGER
  REFERENCES research_tasks(id) ON DELETE SET NULL;   -- the knowledge check that listed this topic
CREATE INDEX research_tasks_check ON research_tasks(check_id);
```

- It runs in one Rust-managed transaction (`use_transaction() = Some(false)`), like 0008–0010,
  because the `ALTER` is not idempotent. A row-count guard in the 0008 style requires the
  `research_tasks` count to be equal before and after.
- **0001 is not edited.** 0002's rebuild would drop the column again on a fresh database, so an
  edit would not break anything, but 0011 stays the only source of the column. That is the rule of
  0009 and 0010.
- Existing rows get `NULL`, because no check has listed topics before this milestone.
- The entity gets `check_id: Option<i32>` with a self-referencing `belongs_to` (the
  `agent_sessions.parent_session_id` pattern from M14).

Invariants (spec/02 §5):

- A task with a `check_id` has `kind = 'topic'` and the check's `book_id`, and is never `draft`.
- Its check has `kind = 'preparation'`.
- Deleting a check sets its topics' `check_id` to `NULL`. They become ordinary topic tasks, and
  nothing runs them automatically any more.

### 3.2 The knowledge check plans

**Tools.**
- `RESEARCHER_TOOLS` becomes: `web_search`, `web_fetch`, `search_knowledge`,
  `read_knowledge_entry`, `set_research_topics`, `save_knowledge_entry`, `update_knowledge_entry`,
  `finish_research`.
- `tools_for_session` for the researcher:

  | Session | Frozen tools, in order |
  |---|---|
  | Knowledge check (`preparation`) | `search_knowledge`, `read_knowledge_entry`, `set_research_topics`, `finish_research` |
  | Topic | the old seven, unchanged |

- `set_target_scope` leaves the role list but stays registered (R8). `is_permitted` already keys
  on the frozen list, so an old check can still call it.

**Kickoff: `researcher.kickoff.prep`.** It is rewritten (in Chinese, in `prompts/tasks.toml`).
The book brief block before it is unchanged. The template contains:

- the writing language, and the author's focus (unchanged);
- the inventory of visible scopes with capped titles, and the excluded names (unchanged);
- **new:** the book's existing topic tasks (`prep.research_tasks[]`: `topic`, `requirements`,
  `scope_name`, `status`). These are every `kind = 'topic'` task of the book that this check's
  `set_research_topics` would not delete, the same rule as `conception.research_tasks[]`;
- the instructions:
  1. Judge what conceiving this title, idea and genre needs: real-world facts, and genre
     knowledge.
  2. Verify coverage with `search_knowledge` / `read_knowledge_entry`. Do not judge by titles
     alone.
  3. Write **only the gaps** as research topics with `set_research_topics`. For each topic, say
     what to collect and what it is for, and name a scope. Prefer an existing scope's name, and
     follow the one-to-two scope budget (spec/07 §1).
  4. Do not repeat an existing entry or an existing topic task. List at most
     `{{ prep.max_topics }}` topics, and none when the knowledge is enough.
  5. Finish with `finish_research`. The report covers what is already sufficient, which topics
     were listed and why, and what needs the author's decision. It is shown to the author and
     passed to the conception agent.
  6. This task collects nothing itself. The listed topics run as research tasks afterwards.
- the budget: `prep.max_tool_calls` only.

**Variables.** `prep.research_tasks[]` and `prep.max_topics` (the constant 8) are new, and
`prep.max_entries` is removed. Update together:
- `variables.toml`;
- `PrepContext` and its sample contexts;
- the samples in `scripts/validate_prompts.py`.

Then run the validator. `researcher.system` stays as it is: the kickoff states the check's
narrower job, as it did in M15.

**Report.** `finish_research` works as before. A new check's `entries_created` is 0. An old check
that saved entries still counts them.

### 3.3 `set_research_topics` in a knowledge check

The tool input is the same, `topics[] {topic, requirements, scope}`. In a research session whose
task is `preparation`:

1. Validate the input as the conception path does, plus two checks.
   - **More than 8 topics** → `error.validation`, and nothing is written. The message names the
     limit.
   - **A scope name that matches a scope excluded for the book** → `error.scope_excluded`
     (`name`), and nothing is written.
2. Delete this check's **waiting agent topics**: rows where
   - `check_id` is this task;
   - `kind = 'topic'`;
   - `origin = 'conception'`;
   - `status = 'ready'`;
   - `session_id IS NULL`.

   The delete never touches a topic of another check, a topic the author added or adopted, or a
   deferred, running or run topic.
3. Insert each topic with these values:

   | Field | Value |
   |---|---|
   | `kind` | `topic` |
   | `origin` | `conception` |
   | `status` | `ready` |
   | `check_id` | this task |
   | `book_id` | the check's book |
   | `language` | the book's writing language |
   | `scope_name` | the trimmed `scope` |
   | `scope_id` | `NULL` (resolved when the topic runs) |

4. Emit `research.updated` for every inserted row, and the same event for deletions that the
   conception path emits. It bumps no `topics_revision`, writes no `change_log` row, and is not
   subject to `settings.locked`.
5. The result is the existing `result.conception_write`, with `action = set_research_topics`.
   - `count` is the number inserted.
   - `kept[]` lists the topics of this check that it left in place. The `ui` payload is
     `{count, kept}`, as today.

In a conception session the tool is unchanged, except that its delete adds `check_id IS NULL`.
Check topics are never `draft`, so this filter is defensive: it states the ownership rule in the
query.

### 3.4 Check topics

| Operation | Rule |
|---|---|
| Create (author) | `POST /research-tasks` with `book_id` and `check_id`. The `check_id` must name a `preparation` task of the same book, and the book must be `conceiving` or `active`. Otherwise `422 validation.invalid_fields` (`check_id`), or `409 book.invalid_state` for the book's status. The task is `ready`, `origin: author`, `kind: topic`. **No** `setting_docs::ensure_editable`. After the commit, run `advance_check` (§3.5). A check that is still running may receive topics; they wait until it is `done` |
| PATCH | topic, requirements and scope_name, while the task is not open (unchanged). The setting-material lock (`ensure_topics_editable`) applies only to tasks without a `check_id`. A PATCH of a `conception` row sets `author`, as today. `check_id` cannot be changed |
| DELETE | Unchanged (refused while open), with no setting-material lock for a check topic |
| Defer / resume | Unchanged rules (spec/07 §8 step 6). After a resume of a check topic, run `advance_check` |
| Run | Unchanged. A manual Run of a waiting check topic is allowed, even while another topic of the check runs, and its completion advances the queue like any other. A run that creates the topic's scope uses R10 |

### 3.5 The queue: `research::advance_check`

`advance_check(state, check_id)` does its work in one `Db::write`, re-reading every row:

```
check := research_tasks[check_id]
stop unless check exists, kind = preparation, book_id set
stop unless book.status ∈ {conceiving, active}
stop unless check.status = done                       -- planning finished (not running/failed/cancelled)
topics := tasks with check_id = check.id and kind = topic, by id
stop if any topic is open (is_open)                   -- one at a time; a held topic holds the queue
next := first topic with status = ready
if next:
    resolve or create next's scope (R10) and re-include it for the book (as prepare_scope does)
    create its research session in this transaction (sessions::create_inner, kickoff_extra scope_id)
        on model.missing_role: no session, no error, log at info; the topic stays ready
    spawn the session after the commit
else:
    start conception if the spec/08 §5 conditions hold (conceiving, flag, no conception session)
```

- The second branch is the body of today's `start_after_knowledge_check`, refactored to take the
  book instead of a completed session. The flag is cleared in the same write, as today.
- `advance_check` runs **after a commit**. A failure is logged at `warn` and never fails the action
  that triggered it.

**Triggers.** Each of these calls `research::advance_after_session(state, session_id)`, which maps
a session to its task and then to a check id: the task itself when it is a check, its `check_id`
when it is a topic. With no check, nothing happens.

| Trigger | Where | Replaces |
|---|---|---|
| A research session completed with `finish_research` | `agent/runner.rs`, after the commit | the current `maybe_start_after_knowledge_check` call |
| A research session abandoned | `sessions::abandon`, after the publish | — |
| A check topic created by the author | `research::create`, after the commit | — |
| A check topic resumed | `research::transition` (resume), after the commit | — |

A failed, stopped or interrupted topic triggers nothing, so it holds the queue. When the author
continues it and it completes, the queue moves on.

**Races.** Every evaluation runs under the single write lock and re-reads the rows. The session it
creates sets the topic `running` in the same transaction. Two triggers can therefore never start
two topics: the second one sees an open topic and stops.

**Stalls.** The queue can stop with waiting topics and nothing open. That happens when no
researcher model is configured, or when starting a session fails. The studio then offers **Run
next** (§3.8), which runs the first waiting topic, and its completion restarts the chain.

### 3.6 Conception: supplementary topics

- **`conception.system` deliverable 5** is rewritten as *supplementary research topics
  (optional)*:
  - Research before conception already ran the knowledge check's topics.
  - List only knowledge that came up during conception, that the knowledge base still lacks, and
    that the writing needs.
  - The scope rules are unchanged.
  - Never repeat an existing topic, including the ones researched before conception.
  - Set none when nothing is missing.
- **The process bullet** about tools keeps `set_research_topics`, worded as setting supplementary
  topics.
- **`conception.kickoff`**:
  - The last line drafts the four documents, "and supplementary research topics only if needed".
  - The "existing research topics" list renders each task's status and its `entries_created`.
- **`conception.research_tasks[]`** gains `entries_created`. Update `variables.toml`, the context
  struct, the samples and the validator.
- **Unchanged:**
  - drafts, `origin`, the `kept[]` result, review of the research-topics item, finalize (drafts →
    `ready`), and overview rule 3;
  - the Research topics tab, which still lists `kind === "topic"` drafts. Check topics are never
    drafts, so they do not appear there.

### 3.7 API

- `ResearchTaskDto.check_id: number | null`. Regenerate the bindings.
- `ResearchTaskCreate.check_id?: number | null`, as in §3.4.
- `GET /research-tasks` keeps its filters. The web filters by `check_id` on the client, from the
  book's list it already loads.
- `POST /books/:id/knowledge-check` is unchanged. Its session now plans.
- Conception's `start`, `finalize` and `BookInput` are unchanged. `auto_start_conception` keeps its
  name; it now means "after the research before conception".

### 3.8 Web

**New book dialog.** The checkbox wording becomes "Start conception automatically when the
research before it finishes" (en and zh-CN). The key, the default and the payload are unchanged.

**Studio, before conception** (spec/11 §5.4; `BeforeConception`, split into a
`PreConceptionResearch` component). The latest check is C, and its topics are T: the book's tasks
with `check_id === C.id`.

| State | Pane |
|---|---|
| No check | Unchanged |
| C open (planning) | The "Knowledge check" header and status, and C's `ActivityStream`, as today. The topic list (below) fills in live as the check writes it. **Start conception now** asks first: "the check and its research topics keep running" |
| C done; some topic in T is open or ready | The header "Research before conception", with progress ("2 of 5 done"). The check's report, collapsed. The topic list. The open topic's `ActivityStream` with its session controls; a failed topic shows its error, **Continue** and **Abandon** (skip). When nothing is open but a topic waits (a stall): **Run next**. **Start conception now** (same confirmation). A hint: "Conception starts when the research finishes" when the flag is set, otherwise "When the research finishes, start conception here" |
| C done; T settled | The report. The topic list. The entries created by T in total, and a link to Book › Knowledge. **Start conception** (primary when auto-start was off) |
| C failed with a session | Unchanged (**Continue**, **Start conception**), plus the topic list |
| C cancelled, or failed without a session | Unchanged, plus the topic list. Its waiting topics run only by **Run** |

**The topic list.**
- Each row shows a status badge, the topic and the scope, with the requirements expandable.
  - **Waiting** rows (`ready`) can be edited inline (topic, requirements, scope), skipped
    (**Skip for now** → defer) and deleted.
  - **Deferred** rows offer **Resume**.
  - **Done** rows show their entry count and link to the task.
  - Author rows are labelled "Added by you".
- **Add topic** at the end sends `check_id`. It is available in every state except "No check".

The pane follows `research.updated` and `session.updated`, so the stream moves from topic to topic
and finally to the conception session without a reload.

**Research lists and the check page.**
- `ResearchTaskList` and `/research` label a task with a `check_id` "From the knowledge check".
- A check's task page lists its topics (status, link) under "Research topics".

**Both locales.** Every new string goes into `en` and `zh-CN`, and `i18n:check` must pass.

### 3.9 Demo provider

- `prep_plan` recognises a check by `set_research_topics` in `hints.tools` for a `research`
  session.
- Steps:
  1. `search_knowledge`;
  2. `set_research_topics` with the corpus's new `prep_topics` (two per language, each topic's
     own corpus `scope`: `Background` and `Customs` for the shipped English corpus);
  3. `finish_research` with the new `prep_report`.
- `prep_scope_description` is removed from `corpus.toml` and from the corpus struct.
- Topic sessions keep `research_plan`: save twice, then finish.
- Conception keeps setting `research_topics`, which are now its supplementary drafts.

The smoke run therefore shows: check → two topics → two research sessions in turn → conception.

## 4. Tasks

Execute them in order. One commit per task, with `scripts/check.sh` green before each commit.

| # | Task | Acceptance |
|---|---|---|
| 20.1 | **Docs**: done with this plan (D-168, §7). Re-read the amended sections. If a code fact contradicts them, fix the section and note it in D-168 | `check.sh` green (docs only) |
| 20.2 | **Schema and types** (§3.1, §3.7): 0011 and its module, the entity field and self relation, `ResearchTaskDto.check_id`, `ResearchTaskCreate.check_id`, bindings | §5.1; `entity_schema.rs` on fresh and upgraded databases |
| 20.3 | **The check plans** (§3.2, §3.3, §3.9): role list and `tools_for_session`, the check branch of `set_research_topics`, `researcher.kickoff.prep` with its context, variables, samples and validator, the Demo arm and corpus | §5.2; `prompts_registry` snapshots for the prep samples |
| 20.4 | **Check topics and the queue** (§3.4, §3.5): create with `check_id`, the lock exemptions, `advance_check`, `advance_after_session`, the four triggers, R10, auto-start moved to settle | §5.3 |
| 20.5 | **Supplementary conception topics** (§3.6): `conception.system`, `conception.kickoff`, `research_tasks[].entries_created`, the `check_id IS NULL` filter | §5.4; `kickoff_baseline` changes only where §5.4 says |
| 20.6 | **Web** (§3.8), both locales | §5.5; `i18n:check` |
| 20.7 | **Smoke and pre-ship**: smoke items 12 (as amended) and 15 with the Demo provider. Run 0011 on a **copy** of the author's database. Report the `research_tasks` count before and after, that every `check_id` is `NULL`, and that book #1's five drafts and conception session #4 are unchanged | the checklist passes; the report is in the final message and in D-168's consequences |

## 5. Tests

No test needs the network. Use the `ScriptedProvider`, the `FakeEmbedder` and the Demo provider.

### 5.1 `check_topics_migration.rs`
- A pre-0011 fixture upgrades with every row intact and `check_id IS NULL`. The fixture has a
  conceiving book, a done check, three drafts and two run topics.
- A fresh database and an upgraded one both pass `entity_schema.rs`.
- Deleting a check sets its topics' `check_id` to `NULL`.

### 5.2 `knowledge_check.rs` (rewritten for M20)
- **Frozen tools.**
  - A check freezes exactly `search_knowledge`, `read_knowledge_entry`, `set_research_topics` and
    `finish_research`.
  - A topic session freezes the old seven, unchanged.
  - No new session freezes `set_target_scope`.
  - A fixture check session whose frozen list contains `set_target_scope` can still call it.
- **Kickoff.**
  - It lists the visible scopes (capped titles, `more`, excluded names), the book's existing topic
    tasks and `max_topics`.
  - It has no entry budget.
- **`set_research_topics` in a check.**
  - It inserts `ready` topics with `check_id`, `origin = 'conception'` and the book's language.
  - A second call replaces only this check's waiting agent topics. These survive and are listed in
    `kept[]`:
    - an author-added topic;
    - a topic the author PATCHed;
    - a deferred one;
    - a running one;
    - a done one.
  - Nine topics → `validation`, and nothing is written.
  - An excluded scope name → `scope_excluded`, and nothing is written.
  - Both an existing scope name and a new one are accepted, and no scope is created yet.
  - `topics_revision` is unchanged.
  - The call works while a conception session runs.
- **Existing rules, kept.**
  - A second check while one is open → 409 with `session_id`.
  - An archived book → 409.
  - No researcher model → an error and **no** row.
  - A PATCH of `topic` on the check → 422.

### 5.3 `check_topic_queue.rs` (`ScriptedProvider`)
1. A book is created with `knowledge_check` and auto-start on, and the check lists three topics.
   - Topic 1 runs; when it completes, topic 2 starts, and so on. Only one research session is
     open at any time.
   - After the third topic, exactly one conception session starts and the flag is cleared.
   - The conception kickoff lists the three topics as `done`, with their `entries_created`.
2. The check lists no topics → conception starts right after the check, as before M20.
3. A topic fails → no new session, and the queue holds. **Continue** → it completes → the next
   topic starts.
4. The running topic is abandoned → it becomes `cancelled`, and the next topic starts.
5. A waiting topic is deferred → it is skipped, and settling ignores it. With auto-start off, once
   the research has settled, resuming it runs it at once.
6. The author adds a topic (`check_id`) in each of these situations:

   | Situation | Result |
   |---|---|
   | While a topic runs | It runs after the earlier topics |
   | While the queue is idle | It runs at once |
   | While the check is still planning | It waits for the check, and the check's `kept[]` lists it |
   | While conception runs | It is not refused |

   In every case it is created `ready` in a conceiving book.
7. Invalid `check_id` on create → 422 in each case:
   - a topic task's id;
   - another book's check;
   - a `check_id` without a `book_id`.
8. Auto-start off → nothing starts after the research settles, and `conception/start` works.
9. The author starts conception while topics remain → the queue continues, and settling starts no
   second conception session.
10. No researcher model when the queue advances → no session, no error, and the topic stays
    `ready`. A manual Run of it completes, and the next topic starts.
11. A topic names a scope that does not exist → its run creates the scope with the description
    "topic A / topic B", the topics of this check with that name. An existing scope's description
    is never changed.
12. The check is re-run while a topic runs → the queue waits until the check is `done` again,
    then continues with the check's current list.
13. A check is abandoned → its topics never run automatically. A check of an `active` book → its
    queue runs, and nothing about conception changes.

### 5.4 Conception and baselines
- Additions to `research_topics_ownership.rs`:
  - conception's `set_research_topics` never deletes a check topic, not even a `ready` one with no
    session;
  - the five-draft fixture behaves as before M20.
- `kickoff_baseline` may change **only** for these:
  - the prep kickoff and the prep tool list;
  - `conception.system`, the conception kickoff and its variable.

  Justify any other diff in D-168.

### 5.5 Vitest
- **Studio states** (§3.8):
  - the check planning, with the topic list filling in live;
  - the queue running, showing the open topic's stream;
  - a failed topic, with Continue and Abandon;
  - a stall, with Run next;
  - settled, with Start conception primary when auto-start was off;
  - the swap to the conception session on `session.updated`.
- **Editing.**
  - Editing a waiting row sends a PATCH.
  - Skip sends defer, and Resume sends resume.
  - Add topic sends `check_id`.
  - A done row is read-only.
- **Start conception now** confirms first.
- **Lists and labels.**
  - The "From the knowledge check" label.
  - The check page's topic list.
  - The new checkbox wording.
- `i18n:check`.

## 6. Smoke item 15 (PLAN §M11.5)

15. Research before conception (M20, plan §6):
    - Demo, auto-start on: create a book with conception.
      - The studio shows the knowledge check. It lists two research topics and finishes.
      - The topics run one after another, with one research session at a time, and each saves
        entries.
      - When the second finishes, conception starts by itself. Its kickoff lists both topics as
        done.
    - A second book, auto-start off:
      - While the first topic runs, edit the second topic's requirements and add a third topic.
      - Both run in order. After the last one, nothing starts, and **Start conception** starts
        conception.
    - A third book:
      - Abandon the running topic: the next one starts.
      - Skip the last waiting topic: when the running one finishes, the research is settled.
    - In conception, the agent sets supplementary topics in the Research topics tab. Finalize: they
      become `ready`, and the topics researched before conception stay `done`.

## 7. Amended documents (done with this plan)

- **PROJECT.md**:
  - glossary: Knowledge check;
  - §5: the Conception and Researcher rows;
  - §6.2, §6.11;
  - F2 steps 1–3 and 6;
  - F3 step 1;
  - §8.7: the Research topics tab and before conception;
  - §8.10: the research task view;
  - §10: #29 rewritten and #33 added.
- **spec/02**:
  - §1: the 0011 paragraph;
  - §2: the header list and `research_tasks.check_id`;
  - §4: the `ConceptionState` comment;
  - §5: invariants 18 and 19, and new invariant 21.
- **spec/03**:
  - §10: the kickoff table rows;
  - §15: test 32 amended and test 34 added.
- **spec/05**:
  - §2: the researcher list and the freezing rule;
  - §7: `set_research_topics` (both owners) and the knowledge note;
  - §8: the researcher tools, `set_target_scope` retired, the budget note;
  - §9: the Knowledge test bullet.
- **spec/07**:
  - §8 step 1;
  - §8.1 rewritten (planning, check topics, the queue).
- **spec/08 §5**:
  - the step table;
  - auto-start on settle;
  - Start conception.
- **spec/09 §2**:
  - the Tasks row;
  - the Results row.
- **spec/10**:
  - §2.2: the knowledge-check row;
  - §2.5: the research-task rows, and a check-topics paragraph.
- **spec/11**:
  - §5.0: the dialog checkbox;
  - §5.4: before conception;
  - §5.5: research lists;
  - §9: Vitest bullets.
- **spec/04 §7.2**: the Demo research row.
- **DECISIONS.md**: D-168.
- **PLAN.md**: M20, smoke item 12 amended, and smoke item 15.
- **docs/README.md**: M0–M20.
- **`knowledge-first-plan.md`**: a superseded-in-part note at the top.

## 8. Risks

| Risk | Mitigation |
|---|---|
| Up to 8 paid research sessions run without a click | The cap of 8. Each topic has the research budget. The author edits, skips or deletes waiting topics while earlier ones run, and abandoning the running one skips it |
| A gateway timeout (D-163) fails a long research turn | The queue holds instead of skipping. **Continue** resumes the same session |
| The check judges without the web and lists a poor topic | The topic sessions do the web work, the requirements are editable, and the author sees the list before most topics run |
| `origin = 'conception'` now also marks a check's topics | spec/02 documents it as "agent-owned". The two owners are told apart by `check_id`, and every delete filters on both |
| A queued topic re-includes a scope the author excluded after it was listed | It runs as **Run** does (R9). The author can skip or delete it. The check itself cannot propose an excluded scope |
| The author starts conception early, so research and conception run at the same time | This is the author's choice, behind a confirmation. Conception's `search_knowledge` sees new entries as they land |
| Overview rule 3 counts waiting topics of an active book's running queue | Harmless (the card opens the list), and rare. Left unchanged |

## 9. Out of scope (do not build)

- Running topics in parallel, reordering the queue, or a pause for the whole queue. The session
  controls and Skip cover these needs.
- Making the topic cap a setting.
- Running the conception agent's supplementary topics automatically, or a Run button in the
  studio's Research topics tab (the task page runs a draft).
- Putting the topic sessions' reports into the conception kickoff. Conception searches the
  knowledge base.
- Web tools for the check.
- Changing overview rule 3 or the research list's filters.
