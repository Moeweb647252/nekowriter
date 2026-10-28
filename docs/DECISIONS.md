# Decision log

Append-only. Each entry: context → decision → consequences. Entries D-001 to D-022 were made
during the design phase and D-023 onward in the design reviews (all 2026-10-24).
Product-level interpretations of the requirements are in PROJECT §10; this log records
technical decisions.

### D-001 Vectors live in SQLite (sqlite-vec)
- **Context**: the requirements ask for a lightweight vector database; earlier guidance rules
  out LanceDB and wants exact cosine search inside SQLite.
- **Decision**: `sqlite-vec` 0.1.9 `vec0` tables (cosine distance, exact KNN) in the main
  database, registered process-wide with `sqlite3_auto_extension` before the pool opens.
- **Consequences**: one file and one transaction model for all data. Partition keys support
  only `=`, so multi-scope knowledge search issues one KNN per scope. Vec tables are created at
  runtime because the dimension depends on the model.

### D-002 Local embeddings with fastembed
- **Decision**: `fastembed` 7.5 (ONNX Runtime, rustls downloads). Default
  `multilingual-e5-small` (384-dim, multilingual, MIT). Alternatives: `bge-small-zh-v1.5`,
  `embeddinggemma-300m-q`. Query and passage prefixes are code constants.
- **Consequences**: the first run downloads the model; keyword search keeps working without it.
  Switching models rebuilds the vec tables in the background.

### D-003 Hybrid retrieval with jieba-segmented FTS5
- **Decision**: FTS5 (`unicode61`) over jieba `cut_for_search` tokens, fused with vector KNN by
  reciprocal rank fusion (k = 60).
- **Consequences**: exact names (common in fiction) are found reliably, even when a small
  embedding model handles proper nouns poorly.

### D-004 Hand-written LLM adapters on reqwest
- **Decision**: no SDKs or agent frameworks. Two protocol adapters (OpenAI-compatible Chat
  Completions, Anthropic Messages) plus mock.
- **Why**: exact replay of provider payloads (thinking signatures, `reasoning_content`), cache
  breakpoint control, eager tool-input streaming for live ghost text, and a persisted resumable
  loop.

### D-005 Anthropic API version 2026-10-21 rules
- **Decision**: send `anthropic-version: 2026-10-21`; never send `temperature`/`top_p`/`top_k`
  (400 on this version); never force `tool_choice` (rejected by current models); use
  `thinking: {type: "adaptive"}` + `output_config.effort`; `eager_input_streaming: true` on tools.

### D-006 Reasoning replay for OpenAI-compatible providers
- **Context**: DeepSeek thinking mode returns 400 when an assistant's `reasoning_content` is
  dropped from later requests in a tool loop.
- **Decision**: store reasoning in `provider_payload` and replay it when the model option
  `reasoning_replay = always` (default in the DeepSeek, Kimi, and OpenRouter presets).

### D-007 Fixed tool list per role, phase-gated permissions
- **Context**: prompt caching is a prefix match (tools → system → messages). Changing tools
  invalidates the whole cache.
- **Decision**: each role has one fixed tool list frozen per session. Phases only restrict which
  tools may run; forbidden calls get an error result with no effect.
- **Consequences**: "no retrieval during drafting" is enforced by the server, not by removing
  tools, and caches survive phase changes.

### D-008 One insertion tool for all granularities
- **Decision**: `insert_text(text, paragraph_end, chapter_end)` validated per granularity, plus
  `finish_draft`. Granularity switches are announced through tool-result notices.
- **Consequences**: switching mid-chapter costs nothing. Validation uses the granularity in
  effect when the call was generated (`request_meta`).

### D-009 Holds are persisted states; history is append-only
- **Decision**: a held or awaiting session has no live task. Resume respawns the runner. Tool
  results are composed at sealing from raw output plus appendices in a canonical order.
- **Consequences**: pauses survive restarts and cost no tokens. Feedback, edit reports, and
  steering messages reach the model exactly once.

### D-010 Edit reports from view snapshots
- **Decision**: store the chapter content when a writer session stops running, and diff it at
  the next sealing to tell the agent exactly what the author changed.

### D-011 Maintenance after approval
- **Decision**: canon updates run only on official text. The reviewer checks against canon from
  before the chapter.

### D-012 Review delegation is one level deep
- **Decision**: the reviewer advises and the author decides. A comment on a report re-runs the
  same reviewer. No agent reviews the reviewer.

### D-013 Agent setting edits are proposals
- **Decision**: maintenance proposes find/replace, append, or new-character edits. The author
  accepts or rejects them, and a stale `find` becomes a conflict.

### D-014 Side stories as tracks with private rows
- **Decision**: canon rows carry `track_id` (NULL = shared). Isolated tracks create private
  rows and cannot modify shared rows. There are no shadow copies or per-track overlays.
- **Consequences**: simple visibility rules (`track_id IS NULL OR track_id = T`). An isolated
  story records changes to shared characters as track-local notes.

### D-015 SQL-file migrations, no schema sync
- **Decision**: `sea-orm-migration` runs embedded SQL files. Entities are hand-written to match.
  SeaORM's `schema-sync` (meant for development) is not used.
- **Why**: virtual tables (FTS5, vec0), partial indexes, and CHECK constraints need raw SQL, and
  explicit migrations protect real books.

### D-016 Frontend foundations
- **Decision**: shadcn CLI 4.22, `--preset nova --base radix` (style `radix-nova`, Lucide).
  React Router 8 data mode. TanStack Query caches REST resources; Zustand holds UI state, live
  streams, and the open chapter document.
- **Why Radix over Base UI**: both are first-class in shadcn; Radix has the most mature ecosystem
  and the most familiar API for implementers.

### D-017 All model-facing text in the prompt pack
- **Decision**: tool results, errors, notices, enum labels (`shared.label`), and chapter labels
  (`shared.chapter_label`) are templates. Tool descriptions are overridable; parameter schemas
  are fixed contracts.
- **Consequences**: code stays English-only and every model-facing word is customizable.

### D-018 Identifier and time formats
- **Decision**: `INTEGER PRIMARY KEY` ↔ Rust `i32` ↔ TS `number`. Agents see `#id`.
  Timestamps are RFC 3339 UTC strings.

### D-019 Demo provider
- **Decision**: a deterministic `mock` protocol provider (debug builds or a flag) drives every
  workflow without API keys, for development and the smoke checklist.

### D-020 Word count
- **Decision**: the server computes UAX #29 counts (authoritative). The client approximates for
  live typing; shared test vectors keep them aligned.

### D-021 Context rollover instead of history editing
- **Decision**: when a drafting or revising session nears the context window, continue in a
  child session whose kickoff contains the current draft. Old sessions are never trimmed.

### D-022 Writer runs one tool call per round
- **Decision**: parallel tool calls are disabled for the writer, matching "each agent round
  triggers one request". Other roles may batch lookups.

### D-023 Stop applies only to running sessions
- **Context**: stopping a held or awaiting session left a call without a result, and Continue
  then had nothing to run.
- **Decision**: `stop` is valid only while `running`. Held, paused, awaiting, and idle sessions
  have nothing in flight; they offer Continue/Feedback (or the form, or a reply) and Abandon.
  Leaving `running` clears `pause_requested`, so a resume never pauses again immediately.

### D-024 Tools run in two phases
- **Context**: `web_fetch` can take 20 s, and query embeddings are CPU work; both ran inside the
  call's transaction, against the rule "never hold a transaction across a network request".
- **Decision**: `ToolHandler::prepare` (outside any transaction, cancellable by Stop) and
  `execute` (inside the transaction, database only). A stop during `prepare` returns the call
  to `pending`.

### D-025 Calls after a completing call are not executed
- **Decision**: when a call completes the session, the remaining calls of the same response end
  as `error.after_completion` and the whole batch is sealed.
- **Consequences**: completed sessions keep a valid history, which re-review relies on.

### D-026 Sessions record their base content version
- **Decision**: `agent_sessions.base_content_version` stores the chapter version a session
  started from. It decides whether an open reviewer session can be reused and whether a
  finished maintenance is `synced` or `stale`. Re-sync saves the official version at its start.

### D-027 Edit reports cover the outline and the conception documents
- **Context**: the most natural reaction to a held outline is to edit it and approve, but the
  agent was never told; conception edits had no stored baseline.
- **Decision**: `view_snapshot` is a tagged `ViewSnapshot` (chapter text + outline + reference
  notes, or conception documents + characters + topics). Setting materials are locked only
  while the conception session runs, as the studio already showed.

### D-028 Rollover carries the author's feedback
- **Decision**: the rollover kickoff lists the author's earlier feedback and steering messages
  from the parent chain (`author_notes`) and the accepted revision plan, so a new context does
  not silently drop instructions.

### D-029 Granularity also limits paragraph edits; short completions are confirmed
- **Decision**: under `sentence` and `paragraph`, `insert_paragraphs` and `rewrite_paragraph`
  take one paragraph, so granularity cannot be bypassed. In every mode an open tail is
  continued by the next `insert_text`. A completion below 60% of the target minimum is withheld
  once and must be confirmed with `finish_draft`.

### D-030 Index rows are cleaned by triggers; vectors carry a corpus
- **Decision**: AFTER DELETE triggers remove `fts_*` and `vec_*` rows (virtual tables are not
  covered by foreign keys, and cascades bypass service code). `vec_book.corpus` (`canon` /
  `passage`) is an equality filter in KNN, so passages never crowd canon hits out. Passages are
  rebuilt on every official content change.

### D-031 Cache breakpoints on the system prompt and the settings block
- **Decision**: explicit Anthropic breakpoints on the system block (shared by a role across
  books) and on the settings block (shared by a book), plus automatic tail caching, all with one
  TTL resolved at session creation (`1h` for conception and for writer sessions that hold for
  the author, otherwise `5m`).

### D-032 Change notes hold only written text
- **Decision**: `change_log.note` holds the agent's note, the author's optional change note, a
  proposal's rationale, or the language-neutral `#B → #A` for merges, and is otherwise empty. The
  UI derives localized labels from the row; model-facing renderings use non-empty notes only.
  No UI-language or code-generated wording reaches a model.

### D-033 A call is executed and finalized in one transaction
- **Context**: committing a tool's effects first and its final status later meant a crash in
  between left an `executing` call whose effects were already applied; recovery would run it
  twice (e.g. insert the same text twice).
- **Decision**: domain effects, result, counters, and the final status (`done`, `error`,
  `held`, `awaiting_input`) commit together. Deferred effects of a held call commit with the
  resume or answer that finalizes it.

### D-034 The runner is the only writer of a running session
- **Decision**: session rows are updated column-wise with an expected-status guard. The API
  writes only the pause flag, the writer switches, inbox rows, and the status changes of
  spec/03 §6; `stop` just cancels the token, and the runner performs the transition at its next
  await point.
- **Consequences**: no lost updates between a pause click and a runner commit, and a double
  click cannot spawn a second runner.

### D-035 Edits made while stopped or failed reach the model on Continue
- **Decision**: when the latest batch was already sealed by a request that did not finish,
  pending edit reports and messages become one new user message. Appendices are composed in the
  canonical order by kind, not by insertion time.

### D-036 Isolated side stories stay out of shared canon everywhere
- **Decision**: active-relation uniqueness is enforced separately for shared canon and per
  isolated track; appearances in isolated chapters do not change shared entities' first/last
  chapter; isolated tracks cannot propose setting changes.

### D-037 Smaller data-model completions
- **Decision**: `agent_sessions.pending_comment` carries a re-review comment into the next
  report; `research_tasks.language` fixes the entry language (book language, or chosen when
  there is no book); chapter `ord` is contiguous and is the chapter number; an anchor chapter
  cannot be deleted; the main track's title is empty and localized by the UI; deleting a book or
  track stops its sessions first; archiving only hides a book.

### D-038 Side-story memory fills both windows and flags post-anchor state
- **Decision**: a side track's prior chapters are topped up from the main story to
  max(N, M), and the protagonist's (and `get_entity`'s) state is marked when its newest change
  comes from after the anchor.

### D-039 Re-chunking keeps unchanged rows
- **Decision**: knowledge chunks and passages are diffed by content hash; unchanged rows keep
  their id, FTS row, and vector, and only the rest are deleted (the delete triggers then clean
  their index rows) or inserted.

### D-040 Dependency features trimmed
- **Decision**: `sea-orm-migration` and `jsonschema` without default features (no CLI, no
  remote schema resolution with its own TLS stack); `tokio-stream` with `sync`; `sea-orm` keeps
  `sqlite-use-returning-for-3_35`. All pinned versions were re-resolved on 2026-10-24.

### D-041 Non-English text is data, never code
- **Decision**: besides prompts and i18n files, non-English text may appear only as data: test
  fixtures, the Demo provider's `crates/llm/demo/corpus.toml`, and the embedding model table
  `crates/embed/models.toml` (the BGE query prefix is Chinese). The connection test's fixed
  `ping` is the only model-facing string in code. Retries allow 4 attempts (first try plus the
  three documented backoffs); HTTP 409 is retried as a transient conflict.

### D-042 Embedded documents are wrapped in tags
- **Context**: setting documents and outlines are Markdown with their own `#`/`##` headings,
  which collided with the headings of the blocks that embed them.
- **Decision**: embedded documents sit inside XML-style tags (spec/09 §5, rule 8); the block headings
  stay for orientation.

### D-043 Schema cascade fixes and side-story focus
- **Decision**: `tracks.anchor_chapter_id` cascades (a SET NULL would violate the anchor CHECK and
  abort a book delete); `change_log.track_id` cascades with an isolated track; a trigger removes
  summary and passage docs when a chapter row disappears through any cascade. Side tracks get an
  optional `focus_entity_id`, the side story's protagonist for short-term memory. Entities and
  the timeline are listed whole by the API (story order and client-side sorting).

### D-044 Overrides are validated per branch and can never stop an agent
- **Decision**: a saved override is rendered with every sample context of its key (one per
  branch of the shipped template). If an overridden template still fails at runtime, the shipped
  default renders instead and the session carries `prompt.override_failed`.
- Also decided in this review round: a conflicting proposal accept returns 200 with status
  `conflict` (Reject dismisses it); Retry after an abandoned re-sync starts a new re-sync;
  graceful shutdown marks running sessions `interrupted`.

### D-045 One in-process write lock; the fetch client ignores proxies
- **Context**: deferred SQLite transactions that read before writing fail with `SQLITE_BUSY`
  immediately (no busy wait) when another connection committed meanwhile; and a system proxy
  resolves hosts itself, which would bypass the `web_fetch` SSRF resolver.
- **Decision**: every write transaction runs through `Db::write`, holding a process-wide async
  mutex (reads stay concurrent under WAL; transactions never wait on I/O, so the lock is held
  briefly). The `web_fetch` client uses `no_proxy()`.

### D-046 Notes on earlier entries, and the last review round
- **Refinements** (the earlier entries stay as written): D-002's prefixes now live in
  `crates/embed/models.toml` (D-041); D-010's snapshot became the tagged `ViewSnapshot`
  (D-027); D-012's re-review uses a fresh reviewer session, given the earlier report, when the
  text changed since that report; D-034's API writes include resume, answer, and continue on
  sessions that are not running (spec/03 §14).
- **Decided in this round**: knowledge entry titles are unique per scope in the schema;
  `error.mutually_exclusive` for parameters that cannot be combined; entries are chunked in the
  same transaction as their change; re-sync reuses an identical official version; merges move
  the protagonist flag and side-track focus; creating a book without a conception model still
  succeeds with a warning; the validator rejects a tag line glued to the following text.

### D-047 Demo proposals, idempotent start endpoints, phase-free sessions
- **Decision**: the Demo maintenance proposes one world-setting append, so the manual loop
  and the smoke checklist can accept a proposal without an API key. `conception/start` and
  `canon/seed` return the open session when there is one (like Ask the reviewer). Sessions
  without phases are labeled 本任务 in `notice.tool_not_allowed`, whose wording no longer
  assumes a phase.

### D-048 Abandoned archivist sessions fail the chapter's canon
- **Context**: abandoning a stopped or paused maintenance session left `canon_status = syncing`
  with no session, and Retry (which needs `pending` or `failed`) could never run again.
- **Decision**: abandoning an open maintenance or re-sync session sets `canon_status = failed`.
  `model.missing_provider` fails a session whose provider row was deleted.

### D-049 Integration tests reach the test helpers through a self dev-dependency
- **Decision**: `crates/server` lists itself under `[dev-dependencies]` with the `testing`
  feature instead of marking tests with `required-features`, which `cargo test --workspace`
  would silently skip.

### D-050 The indexer re-checks content before storing vectors; the outline appears once
- **Decision**: embeddings are computed outside transactions, so the indexer stores a vector
  only if its row still exists with the same `content_hash` under the same model; changed
  items stay `pending`. `block.memory` leaves out the chapter outline whenever the kickoff
  also contains `block.chapter_draft` or is a redraft, and the redraft task tags the previous
  outline like every embedded document.

### D-051 Timeline position conflicts use `mutually_exclusive`
- **Context**: `position_conflict` named only `after_event` / `before_event`, and
  `update_timeline_event` did not say what `move_after` plus `move_before` does.
- **Decision**: both timeline tools return `error.mutually_exclusive` with the two field names;
  the `position_conflict` code is removed.

### D-052 Name collisions and unknown scopes get their own errors; re-sync reads the chapter
- **Context**: renames and new-character proposals could hit a unique index and surface as
  an internal error; `search_knowledge` had no answer for an unknown scope name; the re-sync
  task asked for a complete appearance list while its kickoff shows only the diff.
- **Decision**: handlers check unique names for creates and renames (`error.entity_exists`,
  `error.character_exists`, `error.entry_exists`); an unknown or invisible `scope` returns
  `error.scope_not_found` with the visible names; a proposal whose new character's name is
  taken by accept time becomes `conflict`; the re-sync task has the archivist read the chapter
  with `read_chapter` before re-recording appearances or rewriting the summary.

### D-053 Templates join lists themselves
- **Context**: aliases, field names, pids, tool names, and scope names reached several templates
  as strings joined by code with `、`, which put model-facing punctuation into code and out of
  reach of prompt overrides.
- **Decision**: lists are passed as arrays and joined in the templates; a passage's pid range is
  passed as its pids. `update_knowledge_entry` edits fail with `error.find_failed` like the
  other find/replace tools.

### D-054 Chapter labels and session titles are composed in the UI
- **Context**: `ChapterListItemDto.label` and `agent_sessions.title` had no defined source; the
  server does not know the UI language, and the prompt pack's labels are Chinese.
- **Decision**: the API sends data (`number`, `chapter_number`, `research_topic`, `kind`) and the
  UI composes labels and titles through i18n; `agent_sessions.title` is removed. A scope
  created at finalize joins its topics with ` / `. Tool `ui` payloads follow the same rule:
  `read_chapter` reports the chapter's track and number, and canon writes the target's stored
  name, never a rendered label.

### D-055 Every `*_label` is rendered by the prompt pack
- **Decision**: a failed find in `update_knowledge_entry` uses the `target_type` label of
  `knowledge_entry` as `doc_label` instead of the entry title, so the label rule of spec/09
  holds; a character design's `role_label` is the one stored-text exception.

### D-056 Every error code has a documented trigger; fetch failures are codes
- **Decision**: spec/05 §1.5 lists the triggers of the error codes its tables do not name
  (`not_found`, `pid_not_found`, `draft_empty`, `relation_not_found`, `reorder_mismatch`,
  `character_not_found`, `fetch_failed`), so each is tested. `error.fetch_failed` takes a
  `reason` code worded by the template instead of text written by code. `error.pid_not_found`
  points readers of official chapters back to `read_chapter`, since only writers have
  `read_draft`.

### D-057 Revision planning has its own nudge and revoke notice
- **Context**: a text-only reply in `revision_planning` had no fitting nudge (the planning
  nudge asks for an outline, the revising one for edits), and feedback on a held
  `submit_revision_plan` revoked the phase change without a notice that fit it.
- **Decision**: `writer.notice.nudge_revision_planning` and
  `writer.notice.revision_plan_rejected`; spec/03 §5.6 maps every writer phase to its nudge.

### D-058 Web search failures have their own code
- **Decision**: a failed `web_search` request returns `error.search_failed` with a `reason`
  code (`timeout`, `auth`, `status`, `network`) worded by the template, so it never surfaces
  as an internal error. The HTTP search test reports `provider.auth` or `provider.unavailable`.

### D-059 Appearance bounds and kickoff wording follow the chapter's real state
- **Decision**: an entity's `first_chapter_id` / `last_chapter_id` are recomputed whenever a
  chapter's appearance set changes, including reverts, merges, and chapter deletes, instead of
  being left `NULL` by the foreign key. `block.memory` tells a later chapter written before
  its predecessors are official (Write anyway) that no earlier text is official yet, instead of
  calling it the first chapter. The review and rollover kickoffs mention the outline only when
  the chapter has one (a chapter written by hand has none).

### D-060 Kickoffs carry every author instruction; isolated maintenance asks for no forbidden write
- **Context**: the revise and rollover kickoffs left out the chapter guidance, a rollover dropped
  the ticked issues and a redraft's feedback, the redraft kickoff pointed at pids of the archived
  text, isolated maintenance was told (and asked once) to update a shared protagonist it may not
  write, and the prompts named a Finalize button label that no spec defined.
- **Decision**: every writer kickoff states the guidance, and `writer.kickoff.revise` also the
  target length; the rollover task repeats the feedback being addressed with its ticked issues;
  redraft issues are listed by quote; in an isolated track a shared protagonist's changes become
  a track-local note and `finish_maintenance` skips the state confirmation. The zh-CN label of
  the Finalize action is “定稿”, the name the conception prompts use (with "Finalize book").

### D-064 Service errors cross `Db::write` as an encoded payload
- **Context**: `Db::write` speaks `sea_orm::DbErr`, but service functions must return
  `AppError` so handlers get the exact spec/10 §4 code and `details` (e.g. `chapter.locked`
  with `session_id`) instead of a generic 500.
- **Decision**: `services::write` wraps a service closure: an `AppError` is serialized into
  `DbErr::Custom` behind a marker prefix and decoded again at the transaction boundary. Genuine
  database errors still become `internal`. Every outer service function uses it, so no service
  re-implements the encoding.
- **Consequences**: the typed error survives a rollback, and `Db::write`'s signature stays the
  only write path (spec/01 §7). Plain-TEXT enum columns are read with `services::enum_of`, while
  JSON columns keep using `parse_json` — mixing the two was the bug this found.

### D-065 M1 defers what later milestones own
- **Context**: several M1 endpoints have preconditions that only later milestones can produce
  (writer sessions, canon maintenance, side stories, conception).
- **Decision**: the state rules are implemented and tested against the real rows now, and the
  endpoint that creates that state arrives with its milestone: `POST /books` with
  `start_conception` creates the book `active` and returns the warning `conception.unavailable`
  (M9 adds the session); a `side` track create is `validation.failed` (M10); `POST /tracks/:id/tracks`
  re-creating a main track is `conflict.exists`; `submit`/`approve` refuse a chapter whose
  writer session is running with `chapter.session_exists`, and an open non-running writer session
  must be abandoned by the caller (M3) before submit. Tests set the session and canon rows
  directly through `TestApp` fixtures.
- **Consequences**: no half-built flow is exposed, and the conditions are already enforced the
  moment the later milestone supplies the state.

### D-066 SSE events are pushed for every connection; `chapter.content` carries no book id
- **Context**: spec/10 §3 says `GET /api/events?book_id=` "sends events of that book plus global
  events", but the `chapter.content` payload has no `book_id` field, and `settings.changed` does.
- **Decision**: the filter drops events whose payload names a different book; events without a
  book id (`chapter.content`, `resync`) reach every connection and the client ignores what it does
  not show. `seq` is not part of the SSE `data:` payload — it is only the `id:` field, exactly as
  spec/10 §3 frames it.
- **Consequences**: an open chapter never misses its own text updates, and the client's "apply if
  version = local + 1, else refetch" rule stays the only gap handler.

### D-061 Responses declare `application/json`, not a charset parameter
- **Context**: spec/10 §1 promised `application/json; charset=utf-8`, but the framework's JSON
  response type emits `application/json`. RFC 8259 defines JSON text as UTF-8, so the parameter
  adds no information.
- **Decision**: responses send `application/json`; spec/10 §1 now says so. No middleware rewrites
  the header, because the value carries no semantics for the client.
- **Consequences**: the API matches what the framework emits, and the frontend keeps parsing the
  body as UTF-8 JSON.

### D-067 Prompt contexts and sample coverage come from one data file
- **Context**: spec/09 §2 asks for a typed context struct per key family and for a
  `sample_contexts(key)` table with one sample per branch, and `scripts/validate_prompts.py`
  already models both as Jinja2 dictionaries. Duplicating that table in Rust would make the
  validator and the server disagree the moment one is edited.
- **Decision**: `crates/server/src/prompts/samples.json` holds the base sample of every template
  variable and the per-branch variant table, generated from the validator and embedded at compile
  time; `samples.rs` turns it into `sample_contexts(key)`. `prompts/context.rs` holds the typed
  structs, and a test asserts that each struct's serialized field set matches the sample shape, so
  a renamed field fails the suite instead of failing at render time under strict undefined.
- **Consequences**: the non-English sample strings stay data (D-041), the validator and the server
  check the same branches, and a new template variable needs one edit in `samples.json` plus its
  struct (as AGENTS.md §6 already requires).

### D-068 The prompt registry caches per `AppState`, the pack once per process
- **Context**: spec/09 §3 wants the environment cached per book and invalidated on override
  changes. A process-wide cache would be shared by integration tests that each own a database, so
  a test's override could leak into another test's render.
- **Decision**: the parsed and validated `PromptPack` is a `OnceLock` (it is compile-time data),
  while the resolved `PromptSet` cache belongs to the `PromptRegistry` inside one `AppState`. A
  global override change clears every book's set; a book change clears that book's.
- **Consequences**: every database has its own view of the overrides, and rule validation still
  runs exactly once per process.

### D-069 Deleting a provider is allowed; a dangling role default reports a missing model
- **Context**: `DELETE /api/providers/:id` cascades its model profiles. Refusing the delete when a
  role default still pointed at one of them was the alternative, but spec/03 and D-048 define what
  happens instead: a session whose provider row disappeared fails with `model.missing_provider`,
  and starting one with an unknown profile surfaces `model.missing_role`.
- **Decision**: the delete succeeds; the stored `roles.defaults` entry keeps its (now dangling) id
  until the author changes it in Settings › Agent roles, and the settings page shows such a role as
  "Missing model".
- **Consequences**: no new error code is needed, and a role assignment is never silently rewritten.

### D-070 Connection-test failures are classified, not raised
- **Context**: spec/10 §2.1 returns `{ ok, latency_ms, text?, error? }` for
  `POST /providers/:id/test`, while the sibling `remote-models` call maps provider failures to
  HTTP 502 with `provider.auth` / `provider.unavailable`.
- **Decision**: the connection test always answers 200 with `ok: false` and a classified `error`
  (`provider.auth`, `provider.bad_request` or `provider.unavailable`, spec/10 §4), because a
  rejected key is expected input for that endpoint and not a server fault. `remote-models` keeps
  the 502 mapping of spec/10 §4.
- **Consequences**: the UI can render the result inline without treating a failed test as an error,
  and the remote-model list still reports a real API failure.

### D-071 Optional DTO fields are optional in the exported TypeScript
- **Context**: the settings and prompt DTOs keep `Option` fields omittable on the wire
  (`skip_serializing_if`), but ts-rs exported them as required `| null` properties, so the client
  had to send `api_key: null` where "unchanged" means "omit the field".
- **Decision**: those DTOs carry `#[ts(optional_fields = nullable)]`, so the generated types say
  `api_key?: string | null`. `ModelOptionsDto` and `RoleModels` are patch-shaped for the same
  reason.
- **Consequences**: the UI cannot accidentally clear a stored key by round-tripping a masked value,
  and `edit`/`patch` payloads compile without filler fields.

### D-072 The frozen tool list is the outer bound of what a session may run
- **Context**: spec/03 §1.2 freezes each role's ordered tool list into `agent_sessions.tools` and
  says phases "restrict which tools may run". The phase matrix of spec/05 §2 was the only gate in
  the first M3 implementation, which let a tool run whenever the *phase* allowed it even if the
  session had never frozen that tool. That is observable today with the test-only tools of the
  debug session (spec/10 §2.7), which are not part of any role list, and it would be observable
  later whenever a milestone adds a tool to a role's list.
- **Decision**: `is_permitted(kind, phase, tool, frozen)` short-circuits on the session's frozen
  list before applying the phase matrix; a tool absent from `agent_sessions.tools` cannot run in
  that session, whatever the phase or kind says. The phase matrix is a second, narrower gate
  inside the frozen list.
- **Consequences**: the frozen list is the outer bound, so a tool that M4–M10 later adds to a
  role's list cannot retroactively become runnable in a session created before the change — the
  session keeps exactly the tool set it started with, which is what makes the provider's cached
  prefix (tools → system → messages) stay valid. The test-only tools of the debug session are
  exempt from the phase matrix (they have no phase meaning) but still only run in a session that
  froze them, so this cannot widen a real session.

### D-073 Session warnings are events, not stored columns
- **Context**: spec/10 §1 says mutation responses and `SessionDto.warnings` "may include
  `warnings` (`model.missing_role`, `context.large_kickoff`, `prompt.override_failed`)", but
  `agent_sessions` (spec/02 §2) has no warnings column, and spec/02 and spec/03 win over the other
  documents (AGENTS.md §8).
- **Decision**: warnings are computed where they are known — at session creation, from the
  resolved prompts and the kickoff size — and travel on **both** the creation response's
  `SessionDto.warnings` and the `session.updated` event published for the just-created session, so
  a client that missed the HTTP response still receives them on the event stream. They are not
  persisted: a later `GET /api/sessions/:id` returns `[]`.
- **Consequences**: re-deriving a kickoff-size warning after the fact would be a lie about what
  happened at creation (the kickoff and the context window are not both available), and
  `prompt.override_failed` describes one render, not a property of the session. The UI therefore
  toasts on the event, and the schema stays exactly as spec/02 defines it.

### D-074 The agent panel's stream uses `@tanstack/react-virtual`, whose hook trips the React Compiler lint
- **Context**: spec/11 §5.3 requires the activity stream to be virtualized "with
  `@tanstack/react-virtual`". ESLint's React Compiler rule reports
  `react-hooks/incompatible-library` for `useVirtualizer()` because the API returns functions that
  the compiler cannot memoize safely.
- **Decision**: keep the library the spec mandates; the warning is the expected result of the two
  requirements meeting, not a defect. It is a `warning`, so `pnpm lint` still exits 0, and the rule
  cannot be silenced for this call without also hiding a real future finding.
- **Consequences**: the activity stream component is not memoized by the React Compiler; its rows
  are already memoized by `useMemo` on the message list, and the virtualizer owns the scroll
  behaviour. A reviewer sees exactly one lint warning, documented here.

### D-075 Backspace in the last remaining empty paragraph is a no-op
- **Context**: spec/11 §5.2 specifies "Backspace in an empty paragraph deletes it." Read
  literally, a chapter with one empty paragraph would lose its last paragraph and the author would
  be left in the paragraph view with no editable target at all. The full-text editor already
  exists as the way to replace a chapter's whole text, including emptying it.
- **Decision**: Backspace in an empty paragraph deletes it, except when it is the only paragraph
  of the chapter, where it does nothing. Enter at the end of a paragraph inserts a new one after
  it, as specified. Both keys are ignored while an IME composition is active, because Enter and
  Backspace belong to the composition then (AGENTS.md §6).
- **Consequences**: an author never reaches a paragraph view with nothing to click; to empty a
  chapter they use the full-text editor, which stores an empty document in one step. The rule is a
  pure function, `paragraphKeyAction` in `web/src/lib/paragraphEditing.ts`, pinned by
  `web/src/lib/paragraphEditing.test.ts` ("inserts a paragraph when Enter is pressed at the end of
  the text", "keeps Enter inside the text as a line break", "deletes the paragraph on Backspace in
  an empty one", "ignores every key while an IME composition is active").

### D-076 Tools never write `counters`; they return deferred counter flags on `ToolOutcome`
- **Context**: `finalize_inner` (spec/03 §5.2) writes the session's `counters` column wholesale
  from a `SessionCounters` snapshot taken from the session row **as it was loaded before
  `execute` ran** (`step_tool` clones that row into the tool transaction). A tool that wrote
  `counters` directly inside `execute` — through the transaction it is handed — would have its
  write silently discarded by the finalize write in the same transaction, with no error and no
  log line. This is a shape bug, not a tool bug: it is invisible to the tool author and it gets
  more likely as counters are added.
- **Decision**: tools never write `agent_sessions.counters`. A tool that must advance a counter
  reports it on the outcome (`ToolOutcome.finish_warned`, spec/03 §8's one-shot short-completion
  flag), and `finalize_inner` merges the flag into the snapshot it is about to write with `|=`,
  so a previously set flag is never cleared. The single write of the counters column stays in
  `finalize_inner`, which owns the transaction.
- **Consequences**: `counters` has exactly one writer, so the read-modify-write is not racy and a
  new counter only needs a field on `ToolOutcome` plus one merge line. The `|=` (rather than `=`)
  matters because `finish_warned` is monotonic by spec/03 §8: once a completion has been withheld
  it must stay withheld for the rest of the session. Precedent and implementation: the M4 writer
  tools' short-completion path (`crates/server/src/agent/tools/writer.rs`,
  `counters.finish_warned |= outcome.finish_warned` in `agent/runner.rs`). The alternative — a
  per-column `UPDATE` inside `finalize_inner` — was rejected because it would keep the hazard for
  every future counter that touches a column this function also writes.

### D-077 The Demo provider's revise flow stops short of `replace_text`
- **Context**: spec/04 §7.2 describes the Demo provider's `chapter_revise` behavior as
  "`submit_revision_plan`, then one `replace_text` on `p1`, then `finish_revision`". The Demo
  provider sees only the request it is handed; it has no access to the chapter's current text, so
  it cannot know what `p1` contains. `replace_text` requires `old` to occur exactly once in the
  paragraph (spec/05 §4), which makes any made-up `old` a guaranteed `error.text_not_found`. A
  corpus entry could only be made to match by depending on which granularity and which step wrote
  `p1`, which is exactly the kind of coupling that makes the manual check flaky rather than
  representative. spec/04 §7.2's table is a development aid for exercising the UI, not a normative
  product behavior, and the writer tools are already covered by their own tests
  (`crates/server/tests/agent_write_tools.rs`).
- **Decision**: the Demo's `chapter_revise` submits the revision plan in `revision_planning` and
  calls `finish_revision` in `revising`, using the plan text as the summary. It does not emit a
  `replace_text` step, and no `revision_old` sample is added to
  `crates/llm/demo/corpus.toml`. The `replace_text` path is deferred to M6, which owns the revise
  flow end to end and can pass a `block.chapter_draft`-backed Demo hint (or otherwise make the
  current draft visible to the provider) so the `old` text is guaranteed to occur exactly once.
- **Consequences**: the Demo provider alone does not exercise `replace_text` during a revision;
  the M6 manual check and M6's scripted runtime tests are what cover that path. If a later
  milestone gives the Demo provider a view of the draft, the deferred step can be added by
  extending `chapter_revise_plan` in `crates/llm/src/demo.rs` without touching any other decision.

### D-078 Canon reads and writes share one visibility helper
- **Context**: spec/06 §2 states the rule as `track_id IS NULL OR track_id = T`, and M5 has to
  apply it to seven canon tables from three callers (the author's API, the agent tools, the memory
  snapshot). Spreading the condition over each query is how one of them eventually forgets the
  `track_id` clause.
- **Decision**: `services/canon::Visibility` owns the rule (`visible_condition`, `sees`,
  `may_modify`, `new_row_track`), and `services/canon::CanonWrite` pairs a visibility with the
  actor, session and chapter a change belongs to. Every canon read that feeds a model goes through
  `Visibility`; every canon write goes through `CanonWrite`, which raises
  `error.isolated_readonly` for a shared row. The author's API uses `CanonWrite::author()`, whose
  visibility is `None` and therefore unrestricted.
- **Consequences**: one place to change if spec/06 §2 ever grows another clause, and the isolated
  rule is unit- and service-tested once instead of per table. `error.isolated_readonly` is a tool
  error code (spec/09 §2), so the canon services keep it as a distinct
  `CanonError::IsolatedReadonly` variant that survives the `Db::write` boundary; the HTTP layer
  maps it to `400 validation.failed`, which the author's UI can never trigger.

### D-079 A revert change carries the inverse operation, not a copy of the original
- **Context**: spec/06 §3 says a revert is itself a change with `reverts_id` and that the original
  gets `reverted_by_id`, but it does not say which `op`/`target_id` the revert row carries.
- **Decision**: the revert row records the operation it performed on the target: `delete` for the
  revert of a `create`, `update` for the revert of an `update`, and `create` for the revert of a
  `delete` — the last with the **new** `target_id` the recreated row got, so the row points at the
  entity that now exists (spec/02 §5.10). `before`/`after` describe that operation like any other
  change's, so the Changes page diffs a revert exactly as it diffs the original. **One revert
  writes exactly one change row**: the inverse operation's own change row *is* the revert —
  `CanonWrite::as_revert` stamps it with the `system` actor, the author's revert note and
  `reverts_id` — rather than the operation writing a row and the revert writing a second one.
- **Consequences**: `later_change_ids` of `change.not_latest` and the UI's Revert button work on
  one consistent notion of "the target's changes"; a reverted delete's change row points at the
  recreated id, which is what a reader needs to follow the entity. Reverting a revert is refused
  (`400 validation.failed`): the spec's "only the latest change" rule plus `reverts_id` already
  says the way back is to change the row again. Two regression tests pin the one-row rule for the
  create and delete reverts.

  The delete-revert path also exposed that SQLite reuses the rowid of a deleted maximum row, so a
  recreated entity could silently get the id of the row it replaced and the change log would then
  name two different rows with one target id. `canon::fresh_id` therefore allocates the id of
  every recreated row explicitly — one past the larger of the live table's max id and the change
  log's highest recorded target id for that type — and the five `recreate_inner` functions use
  it.

### D-080 Appearance bounds are recomputed from appearances, never patched
- **Context**: D-059 requires `first_chapter_id`/`last_chapter_id` to be recomputed whenever a
  chapter's appearance set changes (including reverts, merges and chapter deletes), and a shared
  entity counts only shared-canon chapters.
- **Decision**: one function, `canon::entity::recompute_bounds(book, entity_ids)`, reads every
  appearance of the affected entities and rewrites both columns from (track `ord`, chapter `ord`),
  skipping non-shared chapters for a shared entity. Callers pass the union of the old and new
  appearance sets, so an entity that *left* a chapter loses its bound too. Chapter delete reads the
  chapter's entity ids before the cascade and calls it; a merge calls it for the surviving entity.
- **Consequences**: bounds are always derivable from the appearances table, so a hand-edited
  database or a partial failure cannot leave a stale bound behind. The cost is one query per
  affected entity per appearance-set write, which is bounded by the chapter's cast (tens of rows).

### D-081 The memory snapshot's caps and the "important" threshold
- **Context**: spec/06 §4 gives explicit caps for most lists (40 changes, 30 appearance rows, 20
  open items, 15 key notes, 40 cast rows) but not for `absent_important`, and it says "important"
  without a number.
- **Decision**: `IMPORTANT = 4` (spec/06 §1.1's "major" and above) gates `absent_important`,
  `key_notes` and `open_foreshadowing`; `absent_important` is capped at 10. `key_notes` keeps only
  `fact` and `world_rule` rows (never `other`), as §4.4 names those two kinds, and it keeps the
  highest importance first. `open_foreshadowing` lists every open item regardless of importance
  (the spec's own wording is "unresolved foreshadowing", and §4.4 gives the ordering, not a
  threshold).
- **Consequences**: the caps are constants in `services/memory.rs` with the spec section in their
  doc comments, so a reviewer can check them against §4.3–§4.4 in one place; a test asserts each
  one at its boundary. If the author wants `other` notes in the key-notes list, it is one `is_in`
  clause.

### D-082 The Memory page renders every part, so its block.memory always shows the outline
- **Context**: spec/06 §4.2 says `block.memory` leaves the chapter outline out when the kickoff
  already shows it, and spec/10 §2.4 makes `GET /books/:id/memory` return the exact `block.memory`
  text. The Memory page has no `block.chapter_draft` beside it, so nothing else would render the
  outline there.
- **Decision**: the endpoint renders `block.memory` with the chapter's outline included; only a
  real kickoff blanks it, through `KickoffPlan::renders_outline_elsewhere`. `GET
  /books/:id/memory` is therefore "what the block would say for this position", not "what some
  particular session's kickoff says".
- **Consequences**: the Memory page shows the author the whole context, including the outline the
  writer will receive from its own task block. A test asserts both branches: a review kickoff (which
  has `block.chapter_draft`) does not repeat the outline, and a write kickoff of a chapter with no
  draft carries it.

### D-083 Retrieval tools read canon through the services, and the anchor marker is shared code
- **Context**: spec/05 §3's six retrieval tools could query the tables directly, and the
  "after the anchor" marker of spec/06 §2 is needed by four of them while `block.anchor` itself
  arrives with M10.
- **Decision**: every tool resolves entities through `EntityResolver` (spec/05 §1.7) and reads
  through `services/canon`, so the visibility rule is the same one the writer's memory uses. The
  anchor marker is one helper in `agent/tools/retrieval.rs` (`is_after_anchor`), computed from the
  session track's `anchor_kind`/`anchor_chapter_id` and a chapter's (track, ord).
- **Consequences**: a change to the visibility or label rules reaches the tools automatically, and
  the marker is ready for M10 to reuse from `block.anchor` by moving the helper rather than
  rewriting it. `list_entities` sorts in Rust after the query (the spec's "importance desc, last
  appearance desc" order) because SQLite's NULL ordering would put never-seen entities first.

### D-084 A preview ghost is placed by tool, not by target pid
- **Context**: spec/11 §5.2 asks for an `insert_text` preview "appended to the open tail paragraph
  or as a new paragraph", and for `insert_paragraphs` / `rewrite_paragraph` previews "under their
  target pid" (spec/03 §12). The `session.tool_preview` event carries only
  `{session_id, seq, call_index, name, field, delta}` (spec/10 §3): the target pid is an argument
  the model is still generating, and the only place it becomes trustworthy is the persisted call,
  which by definition exists after the preview is over.
- **Decision**: the ghost is placed by tool. `insert_text` streams into the open tail paragraph when
  the document's tail is open and as a new block otherwise, which matches where the call will
  actually write (spec/05 §4). `insert_paragraphs` and `rewrite_paragraph` render as a block of
  their own, and their tool card names the target pid, which is the same place the author reads
  the result a moment later.
- **Consequences**: no preview ever shows prose at a position the finished call does not write to,
  and the ghost needs no state beyond the preview buffer. A future preview event that carried a
  target pid (or a Demo-only hint) could place the paragraph tools' ghosts exactly; that is a
  protocol change, not a UI one, so it is not done here.

**M6 update (the deferral is closed)**: M6 passes the chapter's current paragraphs to the Demo
provider through `demo_hints.draft` (pid and text, built in `agent/context` next to the other
demo hints), so the `chapter_revise` revising step emits `replace_text` with one paragraph's
full text as `old` — a string that by construction occurs exactly once — and `revision_edit` as
`new`. A chapter with no known paragraph (an empty draft hint) falls back to `finish_revision`
as before. The Demo provider now exercises the path spec/04 §7.2 describes, and
`crates/llm/tests/demo.rs` pins it. The original entry above is left as written.

### D-085 M5 review refinements: revert notes, list defaults, private relations
- **Context**: the M5 review found places where the implementation read the spec more loosely
  than written. The earlier entries stay as written; this one records the resolutions.
- **Revert notes**: spec/06 §3 lists reverts under "other system changes: empty", while D-079
  mentioned "the author's revert note" and `RevertInput` accepted one. A revert now always
  records an empty note and the endpoint takes no note. The Changes page's localized label
  already came from `op`/`reverts_id`, not from `note`.
- **D-081's first sentence**: it said `IMPORTANT = 4` gates `open_foreshadowing`; the rest of the
  entry and spec/06 §4.4 say every open item is listed, ordered by importance — no threshold.
  The service lists every open item (unchanged behavior; the entry's contradiction is resolved
  in favour of the spec).
- **Private relations**: spec/06 §2's write rule is about the row a write creates, not about the
  endpoints it reads. An isolated track may create a **private** relation between shared entities
  (`track_id = T`); requiring the endpoints to be writable rejected every track-local relation
  about the main cast, and the per-track unique indexes of spec/06 §1.2 exist for exactly that
  case. Conversely a shared relation may only link shared entities, so the author's API (which
  has no track context) rejects a private endpoint.
- **List defaults**: spec/06 §2's "the UI's canon pages show shared rows by default" applies to
  the timeline, relations, notes and foreshadowing lists too, not only to entities; they now
  filter `track_id IS NULL`, and an entity's detail filters its relations to the entity's own
  canon. The M10 track filter adds one isolated track's private rows to these lists; until then
  the author reaches a private row by id (detail endpoints are unrestricted, D-078).
- **Consequences**: one visibility rule for every author-facing list and one write rule for
  relations, with less chance of an M10 track filter having to re-audit each service.

### D-086 A re-review session receives the previous report through `kickoff_extra`
- **Context**: spec/03 §6 says a new `chapter_review` session's kickoff carries the previous report
  and the author's comment (`reviewer.kickoff` with `prior`), but the schema has no column for "the
  report this session re-reviews"; adding one would be a migration for a value used exactly once,
  at creation.
- **Decision**: `CreateSession.kickoff_extra` (reserved since M3) carries
  `{"prior_report_id": id}`; the kickoff builder loads that report and renders `prior`, taking the
  comment from `pending_comment` (which the schema already has for this purpose). Continuing the
  completed session uses the same `pending_comment`; `submit_review` copies it into `user_comment`
  and clears it.
- **Consequences**: no schema change, and the persisted state still reconstructs the history (the
  report row, the comment on the session while it runs). The kickoff builder is the only reader of
  the transient value.

### D-087 Send-back stores the feedback row and starts the writer session in one transaction
- **Context**: the revision and redraft kickoffs render `chapter_feedback` (spec/08 §3), so the row
  must exist before the session is created; a failed creation (e.g. no writer model) must not leave
  a feedback row that later reads as "the author sent this back" without a session.
- **Decision**: `services::reviews::send_back` runs one `Db::write` transaction that inserts the
  feedback row, calls `sessions::create_inner` (made `pub(crate)`) with the revise or redraft
  `CreateSession`, then stamps `chapter_feedback.session_id`. The outer function publishes the
  accumulated events and spawns the runner only after the commit.
- **Consequences**: all-or-nothing send-back with no compensation path; `sessions::create_inner` is
  now the reusable in-transaction entry point for services that start a session as part of a larger
  operation.

### D-088 Canon `failed` is written by `transition`, not by each failure path
- **Context**: spec/06 §6 and D-048 require `canon_status = failed` when an archivist session fails
  or is abandoned. There are three failure paths in the runner plus the API abandon path, and a
  caller that forgets one leaves the chapter stuck in `syncing` with no session (the defect D-048
  fixed).
- **Decision**: `agent::session::transition` performs the side effect in the same transaction as the
  status change: when the target status is `failed` or `abandoned`, the kind is `chapter_maintenance`
  or `chapter_resync`, and the chapter's `canon_status` is `syncing`, it becomes `failed`. The
  `syncing` guard means a second session's successful completion is never clobbered.
- **Consequences**: every way a session can fail or be abandoned goes through one function, so the
  chapter can never stay `syncing` without a session. Recovery (`running` → `interrupted`) does not
  count as failure: Retry then continues the interrupted session.

### D-089 A proposal preview is the full before/after text, not an API-side diff
- **Context**: spec/10 §2.3 says the proposals list carries "the computed preview diff" without
  defining its shape, while the frontend already owns an inline diff renderer for chapter versions
  (`DiffView`) that splits words for spaced languages and characters for CJK.
- **Decision**: `ProposalPreviewDto { before, after }` returns the target's current text and the
  text after applying the proposal (a stale `find` leaves `after == before`, which is exactly the
  pre-accept conflict signal); a new-character proposal returns the empty string and the created
  summary+profile. The UI renders it with `DiffView`.
- **Consequences**: one preview shape for all three proposal kinds, no locale-dependent diff in the
  API, and the calculation is shared with accept (`apply_edits` is the same function).

### D-090 The Demo maintenance creates the character it records when the book has no canon yet
- **Context**: D-047 promises a Demo maintenance that records appearances and proposes one
  world-setting append so the manual loop works without keys. M6 ships before M9's canon seeding, so
  a book created without conception has no entities and `record_appearances` would fail; the manual
  loop must still run end to end.
- **Decision**: the Demo maintenance reads the protagonist hint; when there is none it first calls
  `create_entity` with the corpus' first character, then walks the normal sequence
  (appearances → protagonist state → timeline → summary → world proposal → `finish_maintenance`).
  An isolated track skips the proposal (spec/06 §6 step 9).
- **Consequences**: the M6.5 manual loop runs on a fresh book, and the extra `create_entity` step
  disappears on its own once the book has a protagonist (after M9 seeding).

### D-091 Search documents hash and embed their body; the title rides along as context
- **Context**: spec/07 §2 defines the indexed fields of `search_docs` but not the content hash or
  the embedding input, and §1.1's chunk hash explicitly covers only `heading + text` while the
  embedding input substitutes the title for an empty heading.
- **Decision**: a `search_docs` row's `content_hash` is `blake3(body)` and its embedding input is
  `title + "\n" + body`, mirroring the knowledge-chunk rule: a rename updates the FTS title and the
  snippet context without re-embedding unchanged text, while a body change resets
  `embed_state = pending`. A knowledge chunk's FTS body holds the heading path plus the text, so a
  section name is findable by keyword as well as by vector. An entry with empty text is marked
  `done` without a vector (nothing to embed); an official chapter's summary document exists only
  when its summary is non-empty (a stub summary has nothing to find).
- **Consequences**: one hash rule for both corpora and D-039's "unchanged rows keep their vector"
  holds across renames; the FTS title and the vector context can drift after a rename until the
  next body change, which no test or ranking path depends on.

### D-092 Scope-level knowledge changes reuse `knowledge.changed`; entry pages are oldest first
- **Context**: spec/10 §3 gives `knowledge.changed` the payload `{scope_id, entry_id, op}` but
  scopes also change (create, rename, delete), and spec/10 §1 does not fix the entry page order.
- **Decision**: a scope change emits `knowledge.changed` with `entry_id = 0`; entry pages are
  ordered by ascending id with an id cursor (the UI pages forward and sorts/filters client-side).
- **Consequences**: one event invalidates the knowledge pages for both scopes and entries; the
  `entry_id = 0` sentinel is documented here rather than growing a second event type.

### D-093 The indexer's empty-text items and failed loads
- **Context**: spec/07 §4 says failed items retry on a model reload or Rebuild, and that the worker
  waits while the embedder is not ready; it does not say what a document with no text does.
- **Decision**: the worker marks an item with an empty embedding input `done` without storing a
  vector (an empty entity or note is searchable by title through FTS only). A failed model load is
  not retried by the 5-second poll: the worker waits, and `POST /embedding/rebuild` (or a model
  change) reloads it, so a broken download cannot hammer the network. In the test build the worker
  is driven batch by batch ([`services::indexer::run_once`]) and only the production build spawns
  the loop; `AppState::new` installs the deterministic fake embedder at the model table's dimension.
- **Consequences**: deterministic integration tests without timing assumptions; the production
  startup spawns the loop after `search::initialize` created the runtime schema.

### D-094 The search tools label hits with `search_kind` and cap knowledge snippets
- **Context**: spec/05 §3 renders `search_canon` hits as `【类型】标题(#ref)`, and the `ui` payloads
  are `{query, count}`; `prompts/shared.toml` offers both `entity_kind` (人物) and `search_kind`
  (实体) labels.
- **Decision**: the bracket label is the `search_kind` label of the document kind (实体/笔记/伏笔/
  事件/章节摘要), because the model is looking at the archive kind, not the entity kind.
  Knowledge snippets are cut to 400 characters (§3's wording); a `search_chapters` hit carries the
  `shared.chapter_label` with its title and its covered pids from `meta.pids`. Main-story chapters
  after a side story's anchor are marked in `search_chapters` (the same anchor helper as the
  retrieval tools); `search_canon` items carry no anchor mark.
- **Consequences**: rendering matches the prompt templates' variables exactly (`after_anchor` is
  always present), and the M10 anchor work only needs to extend the canon-side marking.

### D-095 A manual submit ends the chapter's open writer session
- **Context**: spec/08 §1.1's Submit row says "an open, non-running one is abandoned after the UI
  confirms", but nothing abandoned it: the backend only enforced the `running` block, and D-065
  deferred the rest to the caller while the chapter UI has no session-abandon call. A chapter
  submitted from a held/paused/stopped writer session could never be approved (`approve` refuses
  an open writer session) — a dead end only the agent panel could resolve.
- **Decision**: `submit` abandons the chapter's writer sessions that are open and quiescent
  (`idle`, `held`, `awaiting_input`, `paused`, `stopped`, `failed`, `interrupted`) in its own
  transaction and publishes their `session.updated` events; a `running` writer still answers
  `409 chapter.session_exists` because ending one needs its runner cancelled and awaited
  (spec/03 §6). This implements spec/08 §1.1 as written and supersedes D-065's ruling that an
  open non-running session "must be abandoned by the caller", which the caller never did. The
  chapter header confirms before such a submit.
- **Consequences**: submit→approve works after any writer pause/hold/stop, for manual drafts and
  for the redraft and revision paths (both start writer-role sessions); the spec text needed no
  change. `crates/server/tests/submit_session.rs` proves the abandonment and the surviving
  `running` block; `web/src/features/chapter/submitConfirm.test.tsx` proves the confirmation.

### D-096 The `web_fetch` DNS override exists only for tests and is compiled out of production
- **Context**: spec/07 §7 steps 2–3 require the fetch client to refuse loopback, private,
  link-local, CGNAT, multicast and documentation addresses, resolved on every redirect hop. A
  `wiremock` server listens on loopback, so every positive `web_fetch` test (extraction, decoding,
  paging, the cap) would be refused by the guard. There has to be a seam that lets a local server
  stand in for a public host, and such a seam is exactly the shape of thing that becomes a
  production bypass by accident.
- **Decision**: `Fetcher::with_host_overrides(Vec<(name, SocketAddr)>)` replaces the DNS answer for
  **only the exact names passed in**; it is behind `#[cfg(any(test, feature = "testing"))]`, so no
  release build can call it, and it is used only by `crates/server/tests/`. Nothing else about the
  client changes: the guard, the redirect policy, the 20 s timeout, the 5 MB cap and the extractor
  are the production ones. This is a test seam, **not** a supported feature — there is no setting,
  environment variable or API that reaches it.
- **Consequences**: the guard itself stays the only path to a socket in production, and the
  override cannot be widened by configuration. Two regression tests exist specifically to prove the
  seam is not a bypass: `the_host_override_does_not_permit_any_other_name` (a non-overridden name
  and every literal private IP are still refused) and
  `a_redirect_to_a_second_host_is_re_resolved_and_refused` (a hop to a different name is resolved
  again by the production resolver and the test asserts the redirect target was never requested, so
  the refusal precedes any connection). Reviewed by the M8 reviewer as a security-relevant seam.

### D-098 The research task surface: statuses, the kickoff's scope block and where its UI strings live
- **Context**: spec/07 §8 and spec/10 §2.5 fix the research-task lifecycle but leave three
  questions to the implementation: (a) what a task's status is at creation, (b) what the
  researcher kickoff looks like for a task with no book, and (c) where the research pages' UI
  strings belong, since spec/11 §2 names a closed set of i18n namespaces and `research` is not one
  of them.
- **Decision**: (a) `POST /research-tasks` stores `draft` while the linked book is `conceiving`
  and `ready` otherwise; `PATCH` accepts only `status: ready` (confirming a draft) because the
  runtime and the researcher own every other status. (b) `block.book_brief` renders only when the
  task belongs to a book: a book-less task gets the researcher block alone, because an empty
  brief would read to the model as a blank book rather than "no book". (c) the research pages' and
  settings' strings live under `knowledge:research.*` and `settings:*` — the namespace set of
  spec/11 §2 stays closed, and PROJECT §8.10 already places the research task view on the
  knowledge pages. The task's scope description and existing titles (≤ 200) come from the resolved
  `knowledge_scopes` row, so `run` must resolve (or create) the scope **before** the session starts;
  a task that has no `scope_id` is therefore a shape no researcher session can reach.
- **Consequences**: the lifecycle is driveable by the UI without a hidden rule (draft → confirm →
  run → done/cancelled), a standalone research task reads naturally to the model, and the i18n
  contract of spec/11 §2 is unchanged. Tests: `crates/server/tests/research_tasks.rs` (creation
  rules, run/scope reuse, the kickoff description and titles, the 200 cap, the book-less shape,
  `research.invalid_state` with its `session_id`, abandon→cancelled) and
  `crates/server/tests/research_tools.rs` (the write tools, their error wording, the budget notice
  and the `knowledge.changed` events).

### D-099 The Demo researcher continues instead of duplicating
- **Context**: spec/04 §7.2 requires the Demo provider to save two knowledge entries and call
  `finish_research`, so a research run can be exercised without a provider key. A *second* run of
  the same task (which spec/07 §8 allows once the first finished) would hit `error.entry_exists`
  and, without a hint, could never reach `finish_research`.
- **Decision**: `DemoHints` gains `saved_entries`, the number of entries the task already created;
  the demo researcher skips that many corpus entries and finishes once both are saved. The corpus
  entries carry `source_urls`, so `save_knowledge_entry`'s citation rule is exercised offline.
- **Consequences**: repeated runs terminate and exercise the "continue existing material" path
  instead of a dead end; the hint is data on the request, so it cannot leak into a real provider.
  Test: `crates/llm/tests/demo.rs::the_researcher_saves_entries_then_finishes` and
  `crates/server/tests/research_tasks.rs::a_second_run_does_not_duplicate_the_entries`.

## Open items (questions for the author)
1. Confirm the reading of "this time it cannot be delegated to a lower-level review agent" as
   "the author judges the reviewer's report; no further delegation" (PROJECT §10.9).
2. Confirm the defaults: pause after outline **on**; auto-pause **off**; granularity
   **paragraph**; auto-maintenance **on**; isolated mode for new side stories.

### D-097 `index.status` carries the resolved model cache path
- **Context**: PROJECT §8.12 lists "the model cache path" among the Settings › Embeddings fields,
  while spec/10 §3 fixes `IndexStatusDto` field by field (`model, state, message?, pending, failed,
  done`) and spec/01 §6 says the cache is `<data_dir>/models` unless `HF_HOME` is set. The M7 review
  found that the page showed neither the path nor anything the frontend could derive it from: the
  DTO is the only payload `/api/embedding/status`, `/api/health` and the `index.status` event carry.
  (The review first filed this as a spec conflict and deferred it; the Lead ruled that the conflict
  must be resolved rather than left unimplemented.)
- **Decision**: add `model_cache_dir: String` to `IndexStatusDto` (and therefore to the
  `index.status` event payload) holding the directory the embedder actually uses: `$HF_HOME` when
  set and non-empty, otherwise `<data_dir>/models`. The value is resolved once at startup in
  `AppState::new` through `Config::model_cache_dir` (a pure function of `data_dir` and `HF_HOME`,
  so it is testable without mutating the process environment) and only ever read afterwards.
  Settings › Embeddings shows it read-only; it is not editable, because changing it means a
  different fastembed cache and a model download. spec/10 §3's `index.status` row was updated in
  the same commit.
- **Consequences**: the DTO the settings page, the health badge and the startup log describe is
  now complete; `HF_HOME` is honoured by the server as spec/01 §6 promised (it previously built
  `<data_dir>/models` unconditionally, so the documented override did nothing). Tests:
  `Config::model_cache_dir` covers both branches, and `m7_review_index.rs` proves the DTO and the
  `index.status` event carry the resolved path.

### D-100 Side stories become a real track endpoint (M10 replaces D-065's deferral)
- **Context**: D-065 deferred side-track creation to M10, where `POST /books/:id/tracks` answered
  `validation.failed`. spec/08 §6 now defines the real body (`title, synopsis, canon_mode,
  anchor_kind, anchor_chapter_id?, focus_entity_id?`), its defaults and its two validation rules,
  and the chapter, memory and context rules of a side track (spec/08 §6, spec/06 §2/§4.1,
  spec/03 §10) must be reachable through it rather than through fixtures.
- **Decision**: (a) a `POST /books/:id/tracks` without `kind` creates a **side track** — that is the
  body spec/08 §6 describes — while an explicit `kind: "main"` still restores a missing main track
  (the M1 path, unchanged). (b) The spec defaults are applied server-side because they depend on
  the book: `canon_mode = isolated` and an `after_chapter` anchor on the latest official main
  chapter, or `before_start` when none is official. (c) A side story needs a non-empty title
  (`details.fields`), an `after_chapter` anchor must be a main-track chapter of the same book, and a
  focus entity must be a visible character — shared, or one of the isolated track's own private
  characters when patching. (d) `canon_mode` changes are refused with `track.mode_locked` once the
  track has an official chapter; the anchor may change at any time and only affects sessions created
  afterwards. (e) `PATCH /tracks/:id` clears the focus character with an explicit
  `clear_focus_entity` flag: `Option<Option<i32>>` cannot distinguish "absent" from `null` under
  plain serde without an extension, and ts-rs warns about a custom `deserialize_with`, which the
  team's `-D warnings` lint would turn into a build failure. (f) Deleting a side track stops and
  abandons its open sessions before the cascade (spec/08 §1.3); the shared entities that appeared in
  its chapters get their `first_chapter_id`/`last_chapter_id` recomputed (D-059, D-080), and the
  existing anchor guard keeps an anchor chapter undeletable (`chapter.is_anchor`,
  `details.track_ids`).
- **Consequences**: the M1 test that asserted the deferral now asserts the created side track. The
  first chapter of an `after_chapter` side story treats the anchor chapter as its "previous chapter"
  in the write precondition (spec/08 §1.1, §6), and the write details name it with `"anchor": true`
  so the dialog can explain which chapter is not ready. Tests:
  `crates/server/tests/side_story_tracks.rs` (defaults and `ord`, every validation, the mode lock,
  anchor editing, deletion incl. an anchor-pinned chapter and an open session, the first-chapter
  write rule, `block.anchor` in writer/reviewer/archivist kickoffs, and the §4.1 memory window).

### D-101 Isolated writes are proven end to end, and the canon pages' track filter is scoped
- **Context**: spec/06 §2's visibility rules (an isolated side track reads shared canon and writes
  only private rows; its UI view adds the private rows, badged) were implemented with M5's canon
  services but exercised only through SQL fixtures: the tracks API could not create a side track
  before M10. spec/10 §2.4 gives only three canon list endpoints a `track_id` filter (entities,
  timeline, memory), while memory notes, foreshadowing and the change log have none.
- **Decision**: (a) the M10.2 runtime test creates the track through the real API and drives a real
  `chapter_maintenance` session in it, proving `error.isolated_readonly` on a shared row, a private
  entity/relation with `track_id = T`, a shared entity whose bounds do not move although the
  isolated chapter records its appearance, and the chapter ending `synced`. (b) The canon pages'
  track filter is offered on **Entities**, **Timeline** and **Memory** — the pages whose endpoint
  has a `track_id` filter — and only for isolated side tracks (a canon-mode side track writes shared
  rows, so it has no private rows to add, spec/08 §6). Private rows carry the existing
  `canon:private_badge`. Memory additionally selects a side track as the memory position (its
  snapshot is the §4.1 topped-up window) and renders the snapshot's anchor and an `after_anchor`
  badge for the protagonist, so the page agrees with `block.memory` (D-082).
- **Consequences**: notes, foreshadowing and the change log keep showing shared rows only, matching
  their API; no new endpoint parameters were invented beyond spec/10 §2.4. Tests:
  `crates/server/tests/side_story_isolated.rs` (isolated maintenance, the author's track filter,
  `get_entity`'s and `search_chapters`' post-anchor markers over API-created tracks) and
  `web/src/features/canon/memoryPage.test.tsx`.

### D-102 The side-story UI: one navigator group, a dialog and a track page
- **Context**: PROJECT §8.4/F9 and spec/11 §3 name a "Side stories" navigator group, a New side
  story dialog and a `tracks/:trackId` page; M1's navigator rendered only the main track.
- **Decision**: (a) the navigator shows a collapsible **Side stories** group under the main track
  with the count, a "+" that opens the dialog, and one chapter group per side track whose title
  links to the track page. (b) The dialog collects the spec/08 §6 fields and derives the spec
  defaults from the loaded chapter list (`isolated`; `after_chapter` on the latest official main
  chapter, `before_start` when there is none) without copying them into state by an effect, so a
  dialog opened while the lists load still settles on the right default and an explicit choice
  wins. (c) The track page edits the metadata (title, premise, canon mode, anchor, focus
  character), disables the canon mode once the track has an official chapter, lists the side
  track's chapters with a **Next chapter** action, and deletes the track only after a typed
  confirmation (the deletion cascades its chapters, spec/08 §1.3). The dialog and page strings live
  in the `book` namespace, whose key set stays inside spec/11 §2's closed namespace list.
- **Consequences**: a side story can be created, edited and deleted entirely from the UI, and the
  pages that show canon can be filtered by its track (D-101). Tests:
  `web/src/features/workspace/sideStoryPage.test.tsx` (the dialog's default anchor and submitted
  input, the canon-mode lock hint, the dynamic anchor/canon-mode locale keys in both locales).

### D-103 The answer and held paths apply the full recorded `ToolEffects`
- **Context**: the M3 runtime review found that the transactions finalizing a deferred call
  applied only part of its recorded `ToolEffects`: `answer_inner` applied `effects.complete`
  alone, so an awaiting call's `phase_to` and the completion's sibling/seal/transition half were
  dropped. Nothing could observe it before M9 because no tool set `await_input`; `ask_user` and
  `request_review` are the first ones, so the gap became live exactly here (and the M9.1 tests
  found the mirror bug: `ask_user`/`request_review` set the effect but not
  `ToolOutcome.await_input`, so the calls finalized as `done` and the session ran through).
- **Decision**: both API paths that finalize a deferred call (`answer`, `resume`) run the same
  sequence a tool's own finalize does (spec/03 §5.4 has no exception for them): the phase change
  with its acceptance appendix first, then the call row, then `run_completion_handler` — domain
  effects, `error.after_completion` on the remaining pending calls, sealing, and the transition
  to `completed` — instead of reopening a session that already finished. `assert`-free
  regression tests drive the path through two test-only fixtures (`test_ask`, and
  `test_write(complete: true)`), which only a debug session can freeze (D-072).
- **Consequences**: a deferred call's effects are applied exactly once, the appendix order stays
  canonical (effect notice → edit report → feedback → inbox), and a completed session is never
  left `running`. `crates/server/tests/conception_effects.rs` proves both paths, including the
  awaiting call's own `status = awaiting_input` and its recorded `await_input` effect.

### D-104 The setting-material services gain agent paths that skip the author lock
- **Context**: `setting_docs::ensure_editable` rejects a write with `settings.locked` while the
  book's conception session is running (spec/08 §1.4). The conception agent *is* that session, so
  routing its own `upsert_character`/`remove_character` writes through the author-facing service
  functions would make every character write fail with 423.
- **Decision**: the character service grows `create_from_agent` / `update_from_agent` /
  `delete_from_agent`, sharing the body with the author path but recording `actor: agent` and the
  session id; the document service already had `write_from_agent` for the same reason. The lock
  check remains exactly the author's, as spec/08 §1.4 words it.
- **Consequences**: the agent's writes work while it runs, the change log distinguishes them, and
  the author's direct edits stay blocked until the session stops running.

### D-105 Conception revision counters make an approval go stale
- **Context**: `ConceptionState` has `characters_revision`/`topics_revision` and their
  `*_approved_revision` counterparts, but spec/02 §4 only defines their shape, not who bumps them.
  The studio must warn before finalizing when an approved item changed since (PROJECT §8.7 step 6),
  and the agent must not rewrite a document the author approved at its current revision.
- **Decision**: every agent write to the characters (create/update/remove/reorder) bumps
  `characters_revision`; `set_research_topics` bumps `topics_revision`; a `request_review` answer
  records the current counter as `*_approved_revision`. Documents use their own
  `setting_docs.approved_revision`/`revision` pair, which the same write path bumps. Finalize does
  not block on unapproved items — it lists them to the author, who confirms (spec/08 §5).
- **Consequences**: approval is a revision reference, so a later write makes it stale without a
  second flag; the studio's Finalize button warns exactly while a revision is unapproved, and the
  Demo provider revises only the documents whose revision is not approved
  (`demo_hints.approved_docs`).

### D-106 The Demo provider needs the book's materials, so `DemoHints` carries them
- **Context**: spec/04 §7.2 asks the Demo to seed one entity per character design (rank-1 as the
  protagonist) and, in conception, to revise exactly the documents that were not approved. The
  Demo receives only `demo_hints` and the conversation history; the history carries the rendered
  review text (Chinese, model-facing) but not the designs, and parsing prompt-pack text in the
  provider would put model wording into code.
- **Decision**: `DemoHints` gains `characters` (`{name, role_label, summary, profile}`, rank order)
  and `approved_docs` (the setting-document kinds whose `revision` equals `approved_revision`),
  both filled by the server's `demo_hints()`. They are data-only fields, like the existing
  `protagonist`, `review_attempt` and `draft` hints.
- **Consequences**: the Demo behavior stays deterministic without reading rendered prompts, and
  the fields exist only for the mock protocol (`demo_hints` is `None` for real providers).

### D-107 `Continue` waits for the previous runner to exit; a test-only gate pins the dying-runner window
- **Context**: `SessionManager::spawn` is a no-op while a live handle exists (one runner per
  session, spec/03 §14). A runner that fails commits its terminal status and then keeps running
  briefly (publishing its events, removing its own handle). A `Continue` that lands in that window
  set the row back to `running` and was swallowed by `spawn`: the dying runner looked active, so
  no runner was started and the session stayed `running` with nothing to drive it. This surfaced
  as a rare `agent_write_holds` failure ("session did not finish; it is running") under parallel
  load, and the same sequence is reachable from the UI (click Continue while a failed runner is
  still exiting).
- **Decision**: `continue_session` waits (bounded, 10 s) for the previous runner to be gone,
  whatever ended it — not only for a **cancelled** one. Continue only admits `stopped`, `failed`
  and `interrupted`, and in all three the runner is exiting or already gone, so it never waits on
  a parked runner (`held`/`paused` are `resume`'s statuses, whose `wait_cancelled` is unchanged).
  A test-only gate, `SessionManager::test_hold_next_runner_exit` (compiled only for tests and the
  `testing` feature), holds the next exiting runner between its final write and its handle
  removal, so `m8_review_continue_race.rs` lands in that window deterministically instead of
  relying on scheduler luck.
- **Consequences**: the invariant "after `Continue`, either a runner is registered or the session
  is no longer `running`" is pinned by a regression test that fails without the fix. The more
  general shape is recorded for M11: `spawn` could instead chain onto a live, non-cancelled
  runner (await it, re-read the row, drive only if it is still `running`; safe because `drive`
  returns at once for a non-running row), which would also close the same window for `resume`
  from `failed`/`interrupted`.

### D-108 The single binary embeds `web/dist`; `--web-dir` wins when set
- **Context**: PLAN 11.4 and spec/01 §12 ask for a release build in which one binary serves the
  SPA: `rust-embed` of `web/dist` when the `embed-web` feature is on, `--web-dir` from disk
  otherwise, and a plain hint page when neither is configured. The cloud of small choices around
  that (header details, which requests may fall back, which MIME map) was undecided.
- **Decision**: (a) `--web-dir` takes precedence over the embedded copy when both exist, so a
  release binary can still be pointed at a fresh build during QA. (b) The SPA fallback answers
  `GET`/`HEAD` only for paths outside `/api`; a non-GET page request is `405` rather than the
  shell, and `/api` never reaches the fallback. (c) Unknown `/api` paths answer the standard
  error envelope with `not_found` (spec/10 §1) now that a catch-all sits on the API router.
  (d) The content type comes from an explicit extension table instead of rust-embed's optional
  `mime-guess` feature, so both the disk and embedded paths share one map and no build feature
  affects the headers. (e) Hashed `assets/*` get `Cache-Control: public, max-age=31536000,
  immutable`; everything else revalidates. The `X-Content-Type-Options`/CSP layer still covers
  every response, static files and the guard's 403 included (spec/01 §10.6).
- **Consequences**: `pnpm --dir web build` is a prerequisite of
  `cargo build --release --features embed-web` (rust-embed needs the folder at compile time,
  exactly as spec/01 §12's release command already ordered), while `cargo test --workspace` and
  the `embed-web`-gated unit test need no assets in the default build. Tests:
  `crates/server/src/api/mod.rs::guard_tests` (disk serving, deep-link fallback, traversal
  refusal, MIME and cache headers, `/api` envelope, hint page, and the feature-gated embedded
  shell), plus a manual release-binary run with no `--web-dir` against `/` and
  `/books/1/chapters/2` (reported with the M11 smoke results).

### D-109 Runner spawns chain instead of being swallowed; one open researcher session per task
- **Context**: M8's `Continue` fix (D-107) waited for the previous runner per caller. The
  general shape of the hazard is that `SessionManager::spawn` treats any live handle as an active
  runner, while a runner that committed a non-running status (held, paused, completed, failed,
  interrupted) is still alive for a moment afterwards. `resume` had the same window (`held` and
  `paused` runners exit exactly like that, and `wait_cancelled` returns at once for them), and
  every future control operation would have had to remember its own wait. Two more structural
  races were open: `research::run` checked "no open session" in a separate transaction from session
  creation (two concurrent `POST /run` could create two researcher sessions), and the test
  fixtures wrote rows through `Db::conn()` instead of the process-wide write lock.
- **Decision**: (a) `spawn` now **chains**: if a live, non-cancelled runner exists it subscribes to
  a per-handle `watch` channel, waits (bounded at 10 s) for that runner to leave the handle map,
  re-reads the row, and starts a runner only if the session is still `running`. Continue, Resume
  and any future control op are covered in one place; the per-caller waits stay as defence in
  depth. (b) Session creation re-checks a research task's open session inside the creation
  transaction, so the losing concurrent run gets the documented `409 research.invalid_state` with
  `details.session_id` (`research::run`'s own check can race; this one cannot, because the write
  lock serializes the create transactions). (c) `Db::write` opens with `BEGIN IMMEDIATE`, and the
  test filesystem grows `Db::write_unprepared`, which every `TestApp` fixture helper now uses.
  (d) The harness's default status waits scale with `available_parallelism` (`wait_budget()`)
  instead of a fixed 10 s, because a full parallel suite can schedule a runner late; explicit
  per-call timeouts are unchanged.
- **Evidence for (c)**: the failure mode was measured with two WAL connections: a deferred
  read-then-write transaction that loses the commit race fails with extended result code **517**
  (`SQLITE_BUSY_SNAPSHOT`) — `busy_timeout` does not help — while a writer that meets a held
  `BEGIN IMMEDIATE` waits and then fails with code **5** (`SQLITE_BUSY`), which the timeout
  retries. The 5 s `busy_timeout` is therefore kept (no 30 s+ values); the fix is the immediate
  BEGIN plus the one write path, not a longer timeout.
- **Consequences**: the invariant "after Resume/Continue, either a runner is registered or the
  session is no longer `running`" holds in both windows; tests: `m11_runner_resume_race.rs` (the
  exit gate from D-107 lands a Resume inside the dying-runner window; fails without the chaining),
  `m11_research_race.rs` (a second create is refused deterministically, concurrent runs produce one
  201 and one 409, one session row), and
  `db::tests::write_transactions_take_the_write_lock_immediately` (a second connection cannot take
  the write lock while a `Db::write` transaction is open before its first write; fails with a
  deferred BEGIN). Each was verified to fail with the fix reverted before being committed.

### D-110 Book delete tears down its sessions exactly like a track delete
- **Context**: spec/08 §1.3 says a book delete (like a track delete) stops the book's open
  sessions, awaits their runners and abandons them before the cascade, "so no runner writes to
  deleted rows. The confirmation dialog says so." The track delete implemented that rule; the book
  delete cascaded the rows directly and only the UI's own refetch hid the consequence. It was not
  reproducible as a user-visible bug (a runner writing into a cascade-deleted book fails and stops
  on its own), but the rule is explicit and the window is real.
- **Decision**: the open-session query moves to the session service
  (`sessions::open_ids_for_book` / `open_ids_for_track`, one status filter), and
  `books::delete` abandons every open session through `sessions::abandon` — which cancels a
  running session's token and waits for its runner — before its own delete transaction, mirroring
  `tracks::delete`. `sessions::abandon`'s `session.invalid_state` is tolerated: another caller
  finalizing the same session first is the state the loop wants. The Library's delete dialog now
  says an open agent session is stopped first (both locales), which is the sentence spec/08 §1.3
  requires the dialog to carry.
- **Consequences**: no transaction is held across the abandons, and the API shape is unchanged
  (`DELETE /api/books/:id` → 204). Test: `m11_book_delete_sessions.rs` deletes a book while a
  writer runner is inside a delayed model call and asserts the runner is gone when the delete
  returns and the cascaded session is a 404; it fails without the teardown loop (verified by
  reverting it).

### D-111 The conception studio reuses the agent panel's controls, and Abandon is confirmed there
- **Context**: spec/11 §5.4 places the author's work with the conception agent in the studio, but
  the studio rendered only the conversation, the documents and the composer: Pause, Stop,
  Continue, Feedback and Abandon were reachable only from the book workspace's agent panel.
  Separately, PROJECT §8.13 lists "abandoning sessions" among the actions that need a
  confirmation, and the shared `SessionControls` opened Abandon directly.
- **Decision**: the studio renders the existing `SessionControls` (no second implementation) with
  the same operation mapping the agent panel uses: Pause/Stop while running, Continue (resume for
  held/paused, continue for stopped/failed/interrupted), Feedback through `resume`, Abandon. The
  confirmation moves **into** `SessionControls`, so every route that uses it — agent panel and
  studio — asks before abandoning, and the new strings live in the `agent` namespace
  (`controls.abandon_title` / `_body` / `_confirm`) in both locales.
- **Consequences**: one control surface for every session route, and the destructive action asks
  once everywhere. Tests: `features/conception/conceptionStudio.test.tsx` (controls appear for a
  running/held conception session and call the right operation) and
  `features/agent/SessionControls.test.tsx` (Abandon asks first; cancelling does nothing).

### D-112 The overview's next action and attention list are data; the UI composes labels
- **Context**: spec/08 §7 defines `next_action` (a six-way first match) and `attention[]`
  (open sessions, chapters in review, unsynced canon, pending proposals, ready research) without
  fixing their wire shapes, and spec/11 §6 requires the UI to compose every chapter label and
  session title from data (D-054).
- **Decision**: (a) `NextActionDto { kind, chapter?, session_id?, track_id? }` with the six kinds
  as a snake_case enum, and `AttentionItemDto { kind, count, session_id?, session_kind?,
  session_status?, chapter?, track_id? }`. Sessions, chapters and canon are one row per target
  (`count = 1`) so the UI can list and link each; proposals and ready research are one row carrying
  their count, because a count is all the author needs to open the page. (b) The next action's
  `write_next` names its track and `continue_chapter` names the chapter's open writer session, so
  the card can act without another lookup. (c) `attention_count` on the library card is the same
  list's counts summed, instead of the in-review chapter count M1 used. (d) `usage` sums the
  `usage` JSON of every session of the book (requests and token totals, `last_prompt_tokens`
  recomputed) instead of the zero-filled placeholder.
- **Consequences**: one server-side definition of "what needs the author" serves the overview and
  the library badge, and the frontend never has to join sessions or tracks to render the list
  (`web/src/features/overview/AttentionList.tsx`). Tests:
  `crates/server/tests/overview_actions.rs` (every next-action branch in order, the seed-not-
  repeated rule, the attention kinds and their targets, the card's count, the usage sum) and
  `web/src/features/overview/overview.test.tsx` (localized next-action label and link, side-track
  chapter label, attention rows, empty state, usage line). The persistent quick-switch entry point
  (⌘/Ctrl+K, spec/11 §7) lives in `app/shell/AppShell.tsx` + `QuickSwitch.tsx`, and the shared
  `useHotkey` hook in `lib/hotkeys.ts` ignores IME composition for every shortcut it registers
  (Alt+P in the agent panel included); `lib/hotkeys.test.tsx` pins both the matching and the IME
  guard.

### D-113 Every write the agent runner makes goes through `Db::write`
- **Context**: `session::transition` writes more than one row: the session status, and an archivist
  session's chapter consequence (`chapters.canon_status = 'failed'`), plus a research task's
  status. Most callers passed a transaction, but the runner's failure, refusal, cancel and pause
  paths passed `state.db().conn()`, so each UPDATE auto-committed on its own. A reader could
  therefore observe the session `failed` while the chapter still said `syncing` — exactly the
  transient state the M9 review caught as a flake (session `failed`, `canon_status` not yet
  `failed`) — and it violated spec/01 §7 ("every write transaction goes through `Db::write`") and
  `transition`'s own contract that its writes are one transaction. The M9 review found it by
  reading the code after the flake; it did not reproduce again in 19 runs.
- **Decision**: the runner has two helpers, `transition_in_write` (load + transition inside
  `services::write`) and `fail_in_write` (store the error **and** perform the failing transition in
  one transaction), and every runner status change on a plain connection was replaced with them
  (drive's cancel and pause transitions, the cancelled-stream transition, the failed-step path, the
  refusal path and the deleted-provider path). A test-only pause
  (`agent::session::TEST_TRANSITION_PAUSE_MS`, zero in production) sits between the status write
  and the domain consequence: inside the transaction it changes nothing for readers, while with a
  plain connection it exposes the inconsistent pair immediately.
- **Consequences**: a session and its chapter/research consequence become visible together, so
  Retry and the UI never act on a half-applied transition. Test:
  `crates/server/tests/m11_transition_atomicity.rs` runs a failing maintenance session with the
  pause at 150 ms while a second connection polls both rows, and asserts the pair never disagrees
  (verified: it fails with the old plain-connection writes and passes with the transaction).

### D-114 A rejected review item loses its approval; the rejection is recorded on the item
- **Context**: D-105 says every agent write bumps `revision`/`*_revision` so an earlier approval
  becomes stale, but an explicit **rejection** in a review does not write to the item: the item is
  unchanged, so its `approved_revision`/`*_approved_revision` stayed at the earlier value and a
  second review of the same text treated the rejected item as approved. The M9 review found this
  in the `request_review` answer path.
- **Decision**: answering a `request_review` with a rejection clears that item's approval:
  `setting_docs.approved_revision` for a document, and `conception.characters_approved_revision` /
  `conception.topics_approved_revision` for the character designs and the research topics. Approving
  an item records the current revision (unchanged). Do not amend D-105: its "a later write makes
  approval stale" rule still holds for writes; this entry covers the explicit-rejection case, which
  is not a write to the item.
- **Consequences**: a rejected item can never be carried by an older review into finalize, and the
  studio's "unapproved items" warning covers rejections too.
  `crates/server/tests/m9_review_answers.rs::a_rejected_item_loses_its_approval` pins it.

### D-115 Session creation broadcasts its domain side effects
- **Context**: creating a session changes domain rows in the same transaction (spec/03 §4): a
  writer session moves its chapter to `drafting`/`revising`, a redraft saves a backup and clears
  the text, maintenance/re-sync set `canon_status = syncing`, and a research session makes its task
  `running`. Only the `session.updated`/`session.message` events were published, so a client that
  relies on SSE for chapter freshness could show a stale chapter or navigator row until it
  refetched (the M6 review raised this as a consistency gap; the callers' own cache invalidation
  hid it).
- **Decision**: the creation transaction returns the events its side effects imply and
  `sessions::create_inner` includes them in the list published after the commit, so every caller of
  the shared plumbing (write, send-back, redraft, maintenance, re-sync, research run,
  `conception/start`, `canon/seed`) emits them. Concretely: a chapter status or canon-status change
  emits `chapter.updated` with the re-read row and `outline_changed: false` (the outline never
  changes at creation); a redraft additionally emits `chapter.content` with the new version, a
  `delete` op for every paragraph of the text it cleared and a closed `tail`, because
  `chapter.content` carries `ops` and a client applying them must remove the paragraphs it shows;
  a research task emits `research.updated` with the task read back in the transaction (`running`,
  with its session id).
- **Consequences**: a client never needs a refetch to learn what a creation changed, and the event
  list stays exactly the set spec/10 §3 defines (no new event types). Tests:
  `crates/server/tests/m11_creation_events.rs` drives each kind through `sessions::create` (and
  `research::run`) against a subscribed event bus and asserts the event kind, the committed status,
  and for the redraft the exact delete-op list and version; a conception session (no chapter)
  publishes no chapter event.

### D-116 A failed session shows its stored error, and every session error code is localized
- **Context**: spec/11 §9 requires an `errors:` key for every value of `ErrorCode`, every session
  error code and every warning code, in both locales, because those keys are built from data and
  `check-i18n` cannot see them. The M11 audit found the five session-only codes
  (`provider.context_overflow`, `provider.refusal`, `agent.tool_error_streak`, `agent.no_tool_call`,
  `model.missing_provider`) had no keys, and that nothing rendered `SessionDto.error` at all: a
  failed session only raised "the session failed", so the author never read why.
- **Decision**: the five codes get author-actionable strings in both locales (what happened and
  what to do instead of restating the code), and a shared `SessionErrorNotice` renders
  `errors:<code>` under the session controls in both the agent panel and the studio, falling back
  to the stored English message for a code a newer server might send. `i18n/locales.test.ts` now
  asserts the session-code list in both raw tables and through the real i18n resolver;
  `SessionErrorNotice.test.tsx` pins the fallback.
- **Consequences**: the dynamic-key contract of spec/11 §9 is complete, and a failure's cause is
  visible where the author is (D-111 put the same controls in the studio).

### D-117 Streaming deltas are coalesced per animation frame; paragraphs are memoized
- **Context**: PLAN 11.3 asks a 300-paragraph chapter to stay smooth while the writer streams.
  The SSE client applied every `session.delta`/`session.tool_preview` frame as its own store
  write (one React render per fragment), and the doc comment already claimed requestAnimationFrame
  coalescing that did not exist. The reading column rendered every paragraph on every render and
  rebuilt its edit callbacks inline, so memoizing the paragraph alone would have changed nothing.
- **Decision**: (a) `api/events.ts` buffers deltas and previews and flushes them once per
  animation frame, merging the fragments of one buffer (session, seq, kind, index) or preview into
  one store write; `flushLiveDeltas()` is exported for teardown and tests, the stream disconnect
  and a persisted `session.message` flush/drop the buffer (a message supersedes its sequence).
  (b) `Paragraph` is a `memo` component whose edit callbacks take the pid and are `useCallback`ed
  with a stable `patch.mutate`, and `useErrorToast` returns a `useCallback` identity, so the
  memo actually holds; the streaming tail paragraph's `ghost` prop still changes per delta, which
  is exactly the one paragraph that must re-render.
- **Consequences**: a streaming turn costs at most one render per frame regardless of token rate,
  and only the tail paragraph re-renders while it streams. Tests:
  `api/events.test.ts` (two deltas before a flush leave the store empty, then merge into one
  write; a persisted message drops the buffer) and
  `features/chapter/chapterParagraph.test.tsx` (the component is a memo component and saves with
  its pid). Manual check: the smoke run streamed a held chapter through this path with the 465 MB
  real embedding model loaded and the UI remained responsive (no browser profiling available in
  this environment).

### D-118 The knowledge scope and entry are URL state; a deleted target says so
- **Context**: spec/11 §3 names `/knowledge/scopes/:scopeId` and
  `/knowledge/scopes/:scopeId/entries/:entryId`, but M7 kept the selection in `useState` only, so
  a scope or entry could not be linked to, bookmarked or restored on reload (the M7 review
  deferred this to M11). A deep link to a deleted scope silently showed the first scope.
- **Decision**: the page reads `scopeId`/`entryId` from the router params; selecting a scope,
  an entry, a search hit, or creating/deleting one navigates, so the URL is the single source of
  truth. A URL naming a scope that is not in the scope list, or an entry whose fetch failed,
  renders a not-found empty state (`knowledge:scope_not_found` / `entry_not_found`) instead of
  falling back. The three route shapes (`/knowledge`, `.../scopes/:scopeId`,
  `.../entries/:entryId`) all render the same page.
- **Consequences**: browser back/forward moves between scopes and entries, and reloading a deep
  link restores the same three-column view. Tests:
  `features/knowledge/knowledgeRoutes.test.tsx` (scope from the URL, scope+entry, both not-found
  states, and the entry click navigating to its URL); `knowledgeEntry.test.tsx` keeps its
  no-selection rendering under a router mock.

### D-119 The M11 smoke run: what the checklist exercised, and what this environment cannot
- **Context**: PLAN M11.5 is a real end-to-end run of all nine steps with the Demo provider
  (`mock://demo`), recording what was done and observed. It was driven over the HTTP API against
  a `target/debug/nekowriter` server (the SPA has no browser-automation harness in this
  environment), with the web-side behaviour of each step checked wherever it is observable
  through the API (statuses, events, report text, prompt snapshots).
- **Observed** (all nine steps; see the milestone report for the step-by-step log): conception
  with an answered question form and a review that requested one change, finalize and canon
  seeding; a research run whose two entries were found by keyword immediately and by a
  cross-lingual semantic query after indexing; paragraph drafting that held at the outline, took
  an edited outline, held mid-draft after a pause, took an edited paragraph, and showed the edit
  report in the resumed turn; reviewer report → discussion comment (re-review) → send-back with
  edited feedback → revision → approval → maintenance → revert of a canon change → accepted
  proposal; sentence drafting with three auto-pauses and a switch to free granularity; stop
  mid-stream → server restart → continue, and a held session surviving a restart; an isolated side
  story that wrote, approved and maintained a chapter while the shared canon stayed byte-identical
  and its private entities appeared only with the track filter; a book-level `writer.system`
  override that the next session froze while an earlier session kept its own prompt.
- **Environment findings recorded rather than hidden**: the Demo provider is the only provider
  that can be exercised (no real LLM key is available), and UI-only interactions (the step 9
  language/theme switch) are covered by the resource contract (`i18n:check`, `locales.test.ts`)
  rather than a browser. Contrary to the task brief, the real `multilingual-e5-small` model did
  download into the data directory (465 MB) and the vector path was therefore exercised for
  real, including a cross-lingual query that only the vectors could match.
- **Product behaviour the run confirmed as correct, not a defect**: `chapter.previous_not_ready`
  refuses to write a chapter whose predecessor is still in review (the run used the documented
  `force`/Write anyway path for the throwaway later chapters), auto-pause is an independent switch
  from granularity (switching to free keeps the holds until the author turns auto-pause off or
  keeps continuing), and the Demo's text-only conception reply leaves the session `idle` (it waits
  for the author, then finalize). No new product defect surfaced beyond the four already fixed in
  M11 (task-21 creation events, task-24 races and write discipline, the book-delete teardown and
  the studio controls).

### D-120 The web tools are exercised through a real session, and wait budgets are floors
- **Context**: the M8 review recorded two gaps in the M11 hardening list: `web_fetch`'s
  `prepare`/`execute` split was only tested through the tool object, never through a runner that
  freezes the tool list and seals the result; and `agent_harness::wait_status_within`'s deadline
  was a fixed ceiling that a caller could not raise (`wait_budget()` only covered the default
  waits), so a full parallel suite could still fail for scheduling load rather than for a defect.
- **Decision**: (a) `TestApp::set_web` exposes the D-096 fetcher seam to integration tests, and
  `m11_web_tool_session.rs` drives a real researcher session with a `ScriptedProvider` against a
  `wiremock` stand-in host: the guard, the fetch, the extraction, the prompt template and the
  sealing all run as in production, and the test asserts the sealed `web_fetch` call (status,
  `is_error`, `sealed`, the extracted body and title in `result_text`) and the task ending `done`.
  (b) `wait_status_within` treats its explicit timeout as a floor (`timeout.max(wait_budget())`), so
  every existing per-test deadline scales with the machine instead of assuming an idle one; a
  genuinely stuck session still fails at the budget.
- **Consequences**: the coverage gap is closed rather than deferred, and no test can be made green
  by scheduling luck alone. Test: `crates/server/tests/m11_web_tool_session.rs`. The three
  consecutive full-suite runs recorded for D-109's acceptance exercise the raised budgets.

### D-121 Session creation enforces invariant 5's chapter scope; the API half of it was racy
- **Context**: spec/02 §5 invariant 5 says a chapter has at most one open writer session, one open
  reviewer session and one open archivist session. Each of the three callers checked it before
  creating its session (`chapters::write`, `reviews::ask_reviewer`,
  `maintenance::maintain`/`resync`), but every one of those checks is a read in a transaction of its
  own. The insert happens later, in session creation, so two concurrent requests both passed the
  check and left the chapter with two open sessions. The final review reproduced it: ten
  `POST /chapters/:id/write` pairs with a ready predecessor created **two** writer sessions in
  10/10 rounds, and one of the ten reviewer pairs created two reviewer sessions. Two writer runners
  on one chapter write the same text under different `base_content_version`s, and the UI shows only
  one of them, so the other is unreachable. The same class of defect was fixed for the research task
  in D-109 and for the book-scoped kinds (`conception`, `canon_seed`) already in M9; the
  chapter-scoped kinds had been left to their callers.
- **Decision**: `sessions::create_inner` re-checks the chapter-scoped half of invariant 5 inside the
  creation transaction, under the process-wide write lock — the only place the check and the insert
  are atomic. The guard is keyed by **role**, not by chapter (`chapter_write` and `chapter_revise`
  are both `writer`), because invariant 5 allows one open session per chapter-scoped role: a reviewer
  and a writer session on one chapter is a supported state (spec/03 §14: reviewer sessions do not
  lock the chapter). The loser answers the documented `409 chapter.session_exists` with
  `details.session_id` (spec/10 §4). The three status lists that spelled "open" out by hand are now
  one `open_statuses()`, so the new guard cannot drift from the callers'. `reviews::ask_reviewer`
  treats the new 409 as the idempotent case spec/08 §1.1 already describes — a concurrent request
  created the chapter's session, and "ask the reviewer" is satisfied by it — and returns that
  session's id instead of surfacing the race to the author.
- **Consequences**: an open writer session is now unreachable-by-race rather than only
  unreachable-by-discipline, and a future caller cannot reintroduce the hole by forgetting a check.
  Two existing tests created a second writer session on one chapter and had only passed because the
  invariant was unenforced: `agent_runtime::the_writer_switches_come_from_the_book_settings` now
  abandons the first session before creating the override case, and
  `agent_write_context::the_first_chapter_kickoff_snapshot` drops a debug session it never used.
  Tests: `crates/server/tests/m11_open_session_invariant.rs` (concurrent writes → one session and a
  `chapter.session_exists` naming it; concurrent reviewer requests → one session; a direct second
  create refused; the guard is per role, so a reviewer session starts beside an open writer session
  and a second writer session is still refused). All four fail with the guard reverted.

### D-122 The web test setup provides a ResizeObserver, so the gate stops flaking
- **Context**: `scripts/check.sh` is the project's single gate, and its `web test` step failed
  intermittently with `ReferenceError: ResizeObserver is not defined`, attributed to
  `features/conception/conceptionStudio.test.tsx` ("offers Pause and Stop while the conception
  session runs") and reported as an unhandled error. The final review measured it: **3 of 8** full
  `pnpm --dir web test --run` runs were red, and none failed in isolation. jsdom implements no
  `ResizeObserver` (verified directly against the installed copy), while the Radix primitives behind
  shadcn's `Tooltip` and `DropdownMenu` construct one as soon as their content mounts. The throw
  therefore lands in a layout effect *after* the clicking test's own assertions, which is why the
  failure both moved between runs and named the wrong test.
- **Decision**: `web/src/test/setup.ts` installs a no-op `ResizeObserver` when the environment has
  none. It is test infrastructure only — no production code changes, and no assertion depends on
  measured sizes; the tests assert on rendered output, and browsers provide the real object.
- **Consequences**: the gate is deterministic for the tests that mount a tooltip. Verified by
  reverting the stub: 2 of 6 full runs failed again with the same error, and with the stub 10 of 10
  consecutive full runs passed (188/188). A future component that mounts a Radix popup in a test
  needs no per-file stub.

### D-123 A whole-document replacement keeps `next_pid` monotonic
- **Context**: spec/02 §5 invariant 3 says "pids are unique within a chapter and never reused
  (`next_pid` only grows)". Two operations replace the whole document with one that can carry
  *fewer* pids than the state it replaces, and both stored that document's own counter unchanged:
  a redraft clears the text (`ChapterContent::default()`, counter 1) and a version restore writes
  back an older, shorter document with its original counter. The final review measured it — a
  three-paragraph chapter whose counter was 4 went back to **1** after a redraft, and a restore put
  a counter of 6 back to 5. The next `insert_text` then handed out `p1` while the backup version
  and the archived text still held that pid, which is exactly what invariant 3 forbids, and it
  breaks the "a version is a snapshot of a chapter's text" assumption the History tab and the
  redraft's "ticked issues are listed by their quotes, because their pids belong to the archived
  text" rule (spec/08 §3.6) rest on.
- **Decision**: one helper, `text::keep_next_pid(content, previous)`, repairs the replacement's
  counter against its own pids and then raises it to the previous document's counter when that is
  higher. The redraft path (`sessions::apply_creation_side_effects`) and the restore path
  (`chapters::restore_inner`) call it. `repair_next_pid` keeps its meaning — "at least one above
  every pid present" — and stays the first step.
- **Consequences**: the counter is monotonic across every whole-document write, so a redraft's
  first paragraph never reuses an archived pid and a restored version cannot hand one out either.
  Tests: `text::content::tests::keep_next_pid_survives_a_whole_document_replacement` (redraft,
  restore and a hand-edited under-counted row),
  `crates/server/tests/m11_next_pid_invariant.rs::restoring_an_older_version_does_not_lower_the_pid_counter`
  and `...::a_redraft_keeps_the_pid_counter`; both integration tests fail with the helper call
  reverted.

### D-124 Archivist tool cards resolve their summaries; the tab title and the error boundary are localized
- **Context**: the final review found three user-visible strings that bypassed the i18n contract, and
  the test that should have caught the first one could not fail.
  (a) `archivistSummary` in `features/agent/toolRenderers.ts` built its key with `summaryKey(name)` —
  `tools.<name>.summary` — and passed it to the component's `t`, whose default namespace is `common`.
  The `agent:` namespace was missing, so i18next resolved nothing and all twelve archivist tool
  cards rendered the literal key: a maintenance session showed `tools.create_entity.summary` instead
  of "Created an entity (1 field)". Reproduced against the real en resources before fixing.
  (b) `i18n/locales.test.ts` guarded that renderer with `expect(summary).not.toContain("agent:tools.")`
  — and a key *without* the prefix renders as `tools.x.summary`, which that assertion accepts. It
  also passed `ui = {}`, so the `_target` branch was never exercised. A canonical "a test that
  proves nothing", in the same family the project already found twice.
  (c) The router's `RouteError` — the app's only `errorElement`, hence the one screen a failed lazy
  load can show — hardcoded "Something went wrong" and "Reload" while `common:error_title` and
  `common:retry` already existed, and `index.html` shipped the scaffold title `vite-app` with
  nothing ever setting `document.title`.
- **Decision**: (a) the archivist renderer qualifies both keys with `agent:` and falls back to the
  bare summary when a tool of the shape has no `summary_target` sibling (`record_appearances`
  reports a count, not a target), so the shared renderer is total over its inputs. (b) the
  assertion now compares the *resolved* string against the locale table itself
  (`t` and the table's `summary_target` with `{{target}}` substituted), renders both branches of
  every registered tool, and rejects any unresolved `(agent:)?tools.*` shape — it fails on the
  exact bug (`expected 'Created an entity (1 fields)' to be 'Created Probe'`). (c) `RouteError`
  renders `common:error_title`, `common:error_body` and `common:retry` through `t()`;
  `index.html` carries the product name as the pre-boot fallback, and `applyDocumentLanguage` sets
  `document.title` from `common:app_name` whenever the language changes.
- **Consequences**: no tool card and no error screen can show a raw key or unlocalized English, and
  the locale test now fails before the renderer does. Tests:
  `web/src/i18n/locales.test.ts` (both branches, resolved text compared to the table),
  `web/src/i18n/language.test.ts` (the title follows the locale) and
  `web/src/app/router.test.tsx` (the boundary renders zh-CN text and not the English literal; the
  file is `.tsx` because it now renders JSX). Each was verified to fail with its fix reverted.

### D-125 The review loop's closing batch: a prompt-freeze regression test, a bindings gate, honest docs
- **Context**: the final review's second pass re-read the whole tree against the whole spec and found
  a set of gaps that no single milestone review could see, because each is a claim the project makes
  about itself rather than a defect in one feature.
  (a) PLAN 2.3 and M11.5 smoke step 8 claim "the next session uses the override; sessions created
  before the change keep the old prompt", and **nothing tested the second half**: `prompt_overrides`
  appeared in `prompts_registry.rs` (rendering only) and `agent_runtime.rs` asserted only that
  `system_prompt` is non-empty, which a frozen prompt and a live one both satisfy.
  (b) PLAN 0.4 says "running the test twice yields no diff"; there was no such check. `export_bindings`
  is ts-rs's generated test name, so the *name* existed while the property did not, and nothing
  compared the committed `web/src/api/gen/` to what the DTOs produce.
  (c) spec/03 §15 test 15's `awaiting_input` half was asserted nowhere, its doc comment claimed all
  three non-running statuses while the body exercised only `held`, and a sibling test named
  `stop_on_a_paused_session_is_invalid_state` admitted in its own comment that "nothing pauses here".
  (d) `api_events.rs` still drained events with a fixed 50 × 2 ms loop and slept 50 ms before
  publishing, which is load-dependent — the pattern the project had already replaced in
  `m6_review_events` and `m11_creation_events`.
  (e) `agent_control`'s "one runner only" assertion was `active_count() <= 1`, which zero also
  satisfies — exactly the failure the D-107 race produced.
  (f) README said the search endpoint enables `web_fetch` (it does not; only `web_search` reads
  `app_settings["search"]`), NOTES.md's debug-tool list omitted `test_ask`, called `keys.ts` the only
  place a query key is built, and still told readers `0001_init.sql` "may still be edited" — true
  while dev databases were recreated by hand, false and silently ineffective now that
  `sea_orm_migration` only runs pending migrations.
- **Decision**: (a) `crates/server/tests/m11_prompt_freeze.rs` — a session started under one
  `writer.system` override, the override then replaced by a second, and the session's **next request**
  must still carry the first (verified on the wire body, so it distinguishes frozen from live);
  plus the before/after API assertion that a saved override never rewrites an existing session.
  (b) `scripts/check-bindings.sh` runs the export tests and fails when the tracked output changed;
  `scripts/check.sh` runs it after `cargo test` and spec/01 §12 lists it. (c) `test_15` now covers
  `held` only in its own test, a new `test_15_stop_is_invalid_on_an_awaiting_input_session` covers
  `awaiting_input` (and `pause`/`continue` on it), the completed-session test is renamed
  `control_operations_on_a_completed_session_are_invalid_state`. (d) `drain` drains to empty after
  one yield, and the two subscribe-then-publish tests use a bounded 10 s deadline instead of fixed
  sleeps. (e) the assertion is now `manager().is_running(session_id)`. (f) the three documents are
  corrected.
- **Consequences**: every M11.5 smoke claim and every PLAN acceptance line that names a test now has
  a test that fails without the behaviour, and the one gate that guards the generated types is part of
  `scripts/check.sh` instead of a convention. Verified fail-before for (a) by mutating
  `build_request` to return a fixed string (the wire assertion fails) and for (b) by hand-editing a
  generated file (the script exits 1); (c)–(e) are assertion strengthenings over code paths that
  already behaved correctly.

### D-126 `--allow-remote` reaches the Host/Origin guard instead of being defeated by it
- **Context**: spec/01 §5 step 10 said to "refuse to bind to non-loopback addresses unless
  `--allow-remote` is passed" and §10.2 said the middleware rejects every `Host` that is not
  `localhost`, `127.0.0.1` or `[::1]`. Both were implemented literally and therefore contradicted
  each other: `--allow-remote` opened the bind to `0.0.0.0`, and the guard then rejected every
  request arriving on the non-loopback interface. The author observed it on the release binary:
  `target/release/nekowriter --bind 0.0.0.0:8789 --allow-remote`, then `GET
  http://10.2.0.7:8789/` → `403 {"error":{"code":"host.forbidden","message":"host
  10.2.0.7:8789 is not allowed"}}`. The flag thus produced a server nothing could talk to, while
  `Config::check_bind_allowed` happily warned about the remote bind. The guard had no access to
  the config at all (`ALLOWED_HOSTS` was a fixed three-element array).
- **Decision**: make the flag's meaning coherent across both rules; it now means "allow a
  non-loopback bind **and** accept requests addressed to an IP literal". (a) The three loopback
  names stay always allowed. (b) With `allow_remote`, a `Host` whose host part parses as an IP
  literal (`IpAddr::from_str`) is allowed too — `10.2.0.7:8789` and, brackets stripped,
  `[2001:db8::1]:8789`. Hostnames stay forbidden in the remote case as well. (c) The guard takes
  `State(state): State<AppState>` (it was already registered with `from_fn_with_state`) and reads
  `state.config().allow_remote`. (d) For non-GET requests, the same flag additionally allows an
  `http`/`https` `Origin` whose authority is **exactly** the request's `Host`; a loopback `Origin`
  keeps being allowed without that equality because the Vite dev server legitimately sends
  `Origin: http://localhost:5173` to a `127.0.0.1` Host. `Origin: null` stays rejected in both
  modes, and with the flag off nothing changes at all.
- **Why not "allow any IP-literal origin"**: a page served from a *different* LAN address could
  then drive the API cross-origin, which is exactly the CSRF hole §10.2 exists to close. Requiring
  `Origin` authority == `Host` lets the server's own page mutate its API and nothing else.
- **Consequences**: remote access works over the network address (`http://10.2.0.7:8789`), which is
  what the flag was for. Hostname-based remote access does not work and would need an explicit
  hostname allowlist flag if it were ever wanted (deferred; it would reintroduce the rebinding
  surface, so it needs its own decision). The anti-rebinding property is untouched because a
  rebinding attack requires a hostname and hostnames remain forbidden. The API still has no
  authentication, so `--allow-remote` keeps warning (unchanged). §5 step 10, §6's `--allow-remote`
  row and §10.2 now state the remote case. Tests (verified to fail before the fix:
  `remote_ip_literal_hosts_are_accepted_with_allow_remote` and
  `remote_origin_matching_the_host_is_accepted_with_allow_remote` both got 403):
  `crates/server/src/api/mod.rs` › `guard_tests` —
  `remote_ip_literal_hosts_are_accepted_with_allow_remote`,
  `hostnames_stay_forbidden_with_allow_remote`,
  `ip_literal_hosts_are_forbidden_without_allow_remote` (the regression guard for the flag's
  default), `remote_origin_matching_the_host_is_accepted_with_allow_remote`,
  `a_foreign_lan_origin_is_forbidden_even_with_allow_remote`, `null_origin_stays_forbidden_with_allow_remote`
  and `remote_origin_is_forbidden_without_allow_remote`. End-to-end on the release binary:
  `Host: 10.2.0.7:8789` → 200, `Host: evil.com` → 403.

### D-127 DeepSeek becomes a `web_search` provider, keyed separately from Tavily
- **Context**: the author asked to add DeepSeek to the web-search providers ("联网搜索提供商补上
  deepseek"). DeepSeek exposes a raw JSON Search API (`POST /v1/tools/search`, bearer auth with the
  same API key as its chat endpoint) whose response is a natural fit for the existing
  `SearchProviderKind`/`SearchHit` abstraction. Its results carry **two** prose fields per hit:
  `snippet` (~100–200 characters, documented "适用于前端展现" — for front-end display) and `summary`
  (~600–800 characters, documented "适用于给模型阅读" — for a model to read).
- **Decision**: add `SearchProviderKind::Deepseek` / `SearchProvider::Deepseek` (wire value
  `"deepseek"`), a `deepseek_api_key` field, and a test-only `deepseek_base_url` seam exactly like
  Tavily's. The provider sends `{"query", "count": n, "need_content": false}` to
  `{base}/v1/tools/search` and maps `title` → `title`, `url` → `url`, **`summary` → `snippet`**, and
  `date` → `published` (an absent or empty date stays absent). Failure classification is the shared
  `checked_body`/`classify_request_error` path, so 401/403 become `auth`, other statuses become
  `status` with the number (402 insufficient balance and 429 rate limiting therefore reach the model
  as numbers), a timeout is `timeout`, and everything else is `network`. No retry logic is added.
- **Why `summary`, not `snippet`**: `SearchHit.snippet` is not display text in this project — it is
  rendered into the model prompt by `result.web_search` (`prompts/results.toml`). DeepSeek documents
  `summary` as the field intended for a model to read and `snippet` as one for a front-end. Mapping
  the display field into a prompt would silently degrade every DeepSeek search, so the semantic field
  wins. `need_content` stays `false` because we never read `content`, and a body we would discard
  must not be paid for. This is a deliberate mapping and should not be "corrected" to `snippet`
  without changing what `SearchHit.snippet` means.
- **Why a separate secret**: the DeepSeek key is a DeepSeek API key, not a Tavily key, so overloading
  `tavily_api_key` would force the author to retype/relabel secrets when switching providers. The new
  `deepseek_api_key` therefore gets full parity: literal or `env:NAME` resolved at call time, masked
  in responses, and a masked patch value means "unchanged". Fixing that parity exposed a real
  pre-existing defect: `app_settings::update` merged a patch over `app_settings::get`, which returns
  **masked** secrets, so any later settings save persisted the mask and destroyed the stored Tavily
  key as well. `update` now reads the raw `search` section through
  `search_settings::stored`, and both secrets share one `apply_secret`/`mask_search_secrets` path.
- **Scope**: search only. `web_fetch` is unchanged: the direct fetcher already implements the SSRF
  guard, Readability extraction and paging cache that spec/07 §7 requires, and DeepSeek's Fetch
  endpoint would be a second, billed extraction path. (The author subsequently requested that path
  too; it is D-128, which supersedes this scope note but not the search design above.)
- **Consequences**: the settings shape, validation (`deepseek` without a key is a 400
  `validation.failed`), the UI provider select, both locales, and the specs are extended. The
  DeepSeek base URL is overridable only under `cfg(any(test, feature = "testing"))`; production has
  no setting, environment variable or API field that can change it, and
  `the_deepseek_base_url_seam_is_test_only` pins the default. Tests: `web_search_providers.rs` —
  `deepseek_sends_the_documented_request_and_maps_its_results`,
  `a_rejected_deepseek_key_is_provider_auth`, `other_deepseek_failures_are_provider_unavailable`,
  `a_rejected_deepseek_request_is_status_402_and_429`, `deepseek_is_not_configured_without_a_key`,
  `a_deepseek_key_from_the_environment_is_read_at_call_time`,
  `deepseek_api_key_is_masked_and_a_round_tripped_mask_keeps_it`,
  `a_deepseek_provider_without_a_key_is_rejected`, `the_deepseek_base_url_seam_is_test_only`;
  `search_settings.rs` unit tests; `webSearchPage.test.tsx` for the option and its key field.
  Fail-before was real: before the implementation the provider tests did not compile (13 errors for
  the missing variant, config fields, constant and seam) and the Vitest case failed on a missing
  `option` named DeepSeek (Radix also needed jsdom pointer-capture stubs).

### D-128 `web_fetch` gains a DeepSeek backend; the guarded direct fetcher stays the default
- **Context**: after D-127 shipped DeepSeek search, the author explicitly asked for DeepSeek Fetch
  too. D-127's scope note ("search only") therefore no longer describes the product and is
  superseded here, while its search design stands. The direct fetcher (spec/07 §7) is free, local,
  and produces our own Readability+htmd extraction, but it cannot extract every page; a remote
  extractor is a useful fallback for those.
- **Corrected SSRF analysis**: an earlier review statement said routing fetch through DeepSeek
  "would remove the SSRF guards". That was imprecise and is not the reason for any choice here.
  With DeepSeek Fetch our server only ever opens a connection to the **fixed trusted host
  `api.deepseek.com`**; the target URL is a JSON field fetched by DeepSeek's network. The SSRF
  attack surface — our resolver/socket being steered into loopback, RFC 1918, link-local or cloud
  metadata — **does not exist on that path**, because our process never resolves or connects to the
  user-supplied host. The private-address check is therefore deliberately **not** applied to the
  DeepSeek backend: applying it would only forbid pages DeepSeek can legitimately reach. The
  exception is scoped by construction (only `DeepseekFetcher` skips it); the direct `Fetcher` keeps
  every step-2 check unchanged, and
  `the_private_address_exception_is_scoped_to_the_deepseek_backend` pins both halves — the direct
  backend still refuses `http://127.0.0.1/` while the DeepSeek backend sends that URL as data.
- **What is actually traded**: extraction control (their extractor instead of Readability+htmd),
  the fetch vantage point (their network location, not ours), and money (5 CNY per 1,000
  successful fetches; Search and Fetch are billed separately). URL *shape* validation is still
  ours: http/https, a host, no userinfo, and ≤8,192 characters — checked locally so a malformed URL
  spends no request. The endpoint's lowercase/uppercase request is the only traffic on this path.
- **Decision**: `app_settings["search"]` gains `fetch_via: "direct" | "deepseek"` (default
  `"direct"`), **separate from `provider`** because search and fetch are independent choices (a user
  may search with Tavily and fetch with DeepSeek). `fetch_via: "deepseek"` requires
  `deepseek_api_key` (the D-127 secret) and is otherwise rejected with a 400 field error naming
  `search.fetch_via`. Internally `FetchBackend` selects between the existing `Fetcher` and a new
  `DeepseekFetcher`; `WebService::fetch_document` is the single entry point and applies the shared
  `PageCache`, so the tool never branches on the backend.
- **DeepSeek fetch mapping**: `POST {base}/v1/tools/fetch`, bearer key, body `{"url"}`; response
  `{title, content}` → `FetchedDocument { title, markdown: content }`. `url` is the **requested**
  URL: DeepSeek reports neither the final URL nor redirects. Status mapping: 408/504 → `timeout`,
  413 → `too_large`, 401/402/403/404/400/429/5xx → `status` with the number (so 403 = policy refusal
  and 402 = insufficient balance are visible to the author), transport failure → `network`, and a
  200 with empty `content` → `network` (a successful answer with nothing usable; there is no status
  to carry). No retry logic: the reason reaches the model, which decides.
- **Why `direct` stays the default**: it is free, needs no key, makes no third-party request, and
  gives us known extraction output; DeepSeek is opt-in for pages our extractor cannot read. The
  cache is keyed by the requested URL and shared by both backends, so `offset` reads never
  re-request and paging behaves identically after switching.
- **Consequences**: the `search` settings shape, validation, Settings › Web search UI, both
  locales and spec/07 §7 (which now names both backends) are updated. The UI offers the backend
  Select and shows the DeepSeek key field whenever either independent choice needs it (search
  provider `deepseek` **or** `fetch_via: "deepseek"`), so the Tavily-search + DeepSeek-fetch
  combination is configurable without detouring through another provider. Tests:
  `crates/server/tests/web_fetch_backends.rs` —
  `deepseek_fetch_sends_the_documented_request_and_maps_the_page`, `deepseek_fetch_error_mapping`,
  `deepseek_fetch_validates_the_url_shape_before_sending`,
  `fetching_through_deepseek_still_pages_and_caches`, `the_direct_fetcher_remains_the_default`,
  `the_private_address_exception_is_scoped_to_the_deepseek_backend`, `fetch_via_defaults_to_direct`,
  `fetch_via_deepseek_requires_a_key`; `webSearchPage.test.tsx` for the fetch-backend select.
  Fail-before was real: before the implementation the backend tests did not compile (missing
  `FetchBackend`, `FetchConfig`, `WebService::fetch_document` and `SearchConfig::fetch_via`; 11
  errors) and the Vitest case could not find a combobox named "Page fetching".

### D-129 Conceiving books are part of the default library view, and their card opens the studio
- **Context**: the author reported that the library showed two books while their database held
  three. The third (`带着蓝色大肥鱼穿洪武`) was `conceiving`. `LibraryPage` fetched
  `?status=active` whenever "Show archived" was off, and `books::list` filters server-side on that
  exact value, so a book in conception never reached the browser — the page's own
  `status !== "archived"` filter had nothing to filter because the row was already gone. The
  book was therefore **invisible in the library from creation until finalize**. That is a
  reachability bug, not a missing card: a book created with "start with the conception agent" that
  the author closes the studio on — or whose first turn the provider refuses, which is the state
  in the author's own session — had no card, no way back in, and no indication it existed. The
  overview already treats `conceiving` as its own state with a Continue-conception button
  (`OverviewPage`), so the library was the only screen that disagreed.
- **Decision**: the library loads `GET /books` **unfiltered** and applies the "Show archived"
  checkbox in the browser, which is where the archive decision already lived
  (`list.filter(status !== "archived")`). The server's `?status=` contract is untouched: it stays
  exact for `conceiving`/`active`/`archived`, with `""`/`all` still meaning no filter, because the
  book switcher, the breadcrumb and the research dialog use the unfiltered form and other screens
  may yet want an exact one. The alternative — a new "not archived" value or a status list on the
  endpoint — was rejected: it adds API surface for a single page, and it would make `?status=`
  mean two different things depending on its value.
- **Consequences**: the default view is `conceiving` + `active`; "Show archived" adds `archived`
  exactly as before, and the checkbox still only changes what is displayed, never what is fetched.
  The request is now the full list every time, which is one author's books (three rows here) and
  the same payload the switcher already requested. `BookCard` renders a `conceiving` book
  differently: a `library:conceiving` badge next to the genre, a waiting-toned ring, and below the
  usual lines a `library:conceiving_hint` sentence with a "Continue conception" link to
  `/books/:id/conception`. The title button and the "⋯ → Open" item use the same route for a
  conceiving book and keep `/books/:id` for every other status, so nothing that worked before
  changed.
  **The chapter/word counts and the latest-chapter line stay visible for a conceiving book.** The
  first version of this card replaced them with the conceiving block, assuming a conceiving book
  has no chapters. Rendering the author's real data disproved it: book 3 is `conceiving` and owns a
  chapter already (`chapters` row `(2, track 3, book 3, ord 1, 'planned')`, created before D-130
  gated chapter creation on finalize), so the replacement would have hidden real data behind a
  friendly message. The card now always shows the counts and adds the conceiving block, and
  `libraryPage.test.tsx` has a case for a conceiving book that already has one chapter.
  New i18n keys (`library:conceiving`, `library:conceiving_hint`) exist in both locales;
  `i18n:check` covers the parity and `libraryPage.test.tsx` resolves them through the real i18n
  instance. Tests: `crates/server/tests/library_list.rs` —
  `the_unfiltered_list_contains_every_book_status` (three statuses in one list) and
  `the_status_filter_stays_exact` (`?status=active` → active only; `conceiving` and `archived`
  filter alike; `""`/`all` → all three; an unknown value is still a 400).
  `web/src/features/library/libraryPage.test.tsx` mocks `useBooks` the way the server behaves (a
  filter really does remove rows) and asserts the conceiving card renders with "Show archived"
  off, that its real chapter counts are not hidden, that an archived book stays hidden until the
  toggle and reappears with it, that the card's link and title route to `/books/3/conception`, and
  that an active book still opens the overview. Fail-before was real: with the old page, "renders
  a conceiving book with Show archived off" failed on a missing heading text and "takes a
  conceiving book straight into its studio" failed with no accessible link named "Continue
  conception"; the other two cases already passed, which is what pinned the regression to the
  status filter rather than to the card.

### D-130 Chapter creation is gated on the book being `active`
- **Context**: the author reported "构思未完成时仍可以创建章节" — chapters could be created while the
  conception was unfinished. The root cause was in the spec, not only the code: spec/08 §1.1's
  operation table listed `Create chapter` with `—` in **Preconditions**, so nothing told an
  implementer to check the book at all. `chapters::create_inner` loaded the track, computed `ord`
  and inserted with `status: planned`; no path from the endpoint to the insert ever read
  `books.status`. The UI compounded it: `Navigator`'s per-track "+ Next chapter" called
  `useCreateChapter(track.id)` unconditionally, even though the same component already receives
  `book: BookDto` with its `status`, and `SideStoryPage`/`NextActionCard` reproduced the button.
  The overview, by contrast, already treats `conceiving` as its own state (it shows Continue
  conception), so the two screens disagreed about what a conceiving book allows.
- **Why it matters**: `conception/finalize` is not bookkeeping — it is the step that activates the
  book *and* the step that makes the setting materials and canon exist, which is exactly what the
  writer's kickoff renders (`writer.kickoff.chapter` injects the settings block and canon). A
  chapter created before finalize enters a track whose book has no outline, world, protagonist or
  canon, so the first write session runs against an empty context and produces a chapter unrelated
  to the book's eventual premise; it also makes a `conceiving` book look started in the navigator
  and in the overview's progress counts, and it is awkward to undo (chapter deletion did not exist
  yet). Putting the gate in the UI alone was rejected: the API is the contract, and a UI disable
  is an addition on top of the rule, not the rule.

  This is not hypothetical: the author's own database has a `conceiving` book (`带着蓝色大肥鱼穿洪武`)
  with exactly one `planned`, zero-word chapter, created precisely *because* nothing refused it.
  So the fix has to handle a book that already has chapters, not only a pristine one: refuse new
  ones, leave the existing row intact, and let it become fully usable at finalize.
- **Decision**: creating a chapter on a book whose status is not `active` is refused **by the
  server** with `409 book.invalid_state` and `details.status` naming the current status
  (`ErrorCode::BookInvalidState`, matching how `conception/finalize` and `books::update` express
  the same envelope). `archived` is refused by the same rule — it is hidden from the default
  library view and must not accumulate new chapters either. The check lives *inside* the existing
  `create_inner` transaction (`chapters::ensure_book_active(tx, book_id)` right after the track is
  loaded), so no second transaction and no new lock were added, and the status cannot flip between
  the check and the insert. It is a precondition of the book, not of the track kind: a side track
  of a conceiving book is refused identically (§6 says side-story writing is the main-track path).
- **The reverse direction is the point**: `conception/finalize` sets `active`, so after it succeeds
  creation must work normally — and the test asserts that transition, not just the refusal, because
  a refusal-only test would let a regression where finalize forgets to activate the book pass
  silently. The same test covers the pre-existing chapter: after finalize it must be a first-class
  chapter, so a `POST /chapters/:id/write` on it starts a `chapter_write` session normally. "The
  chapter that already existed is stranded after finalize" would be a worse bug than the one being
  fixed, hence the explicit assertion. Also asserted: the refused attempts consume no chapter
  number (the chapter list stays empty and the next successful create is numbered 1/2, so the
  Navigator's numbering has no gap), and a book created with `start_conception: false` (which
  starts `active`) keeps working with no friction.
- **Consequences**: conceiving and archived books cannot accumulate chapters; `conception/finalize`
  is the gate to the writing workflow, which is a product statement spec/08 §1.1 now carries in its
  Preconditions column instead of `—`. Every existing fixture already created chapters on books
  made through `TestApp::create_book` (which passes `start_conception: false`, i.e. `active`), so
  **no fixture had to change** and the whole pre-existing suite still passes unmodified — the
  guard's unit of work is new-file-only. The UI half (disable the "+ Next chapter" affordances with
  a reason that points at the studio) is routed separately, since `Navigator.tsx` is owned by the
  chapter-deletion task; until it lands the server refuses and the error toast words
  `book.invalid_state`. Tests: `crates/server/tests/chapter_creation_gate.rs` —
  `a_conceiving_book_refuses_a_new_chapter` (409 + `details.status`; after three refused attempts
  the chapter table still holds zero rows for the track and `MIN`/`MAX(ord)` are both `NULL`,
  asserted with SQL rather than inferred from the list DTO, because `ord` is `last + 1` and a
  consumed number would surface only as a numbering gap after finalize),
  `an_archived_book_refuses_a_new_chapter` (409, the existing chapter is untouched, and
  unarchiving resumes creation at number 2),
  `a_conceiving_book_with_an_existing_chapter_refuses_only_the_second_one` (the real-data shape: a
  seeded `planned` chapter stays readable while a second create is refused),
  `finalize_is_what_makes_chapter_creation_possible` (three refusals → `conception/finalize`
  returns `status: active` → created, numbered 1, with `MIN(ord) = MAX(ord) = 1` and exactly one
  row, so the refusals cost no number),
  `a_chapter_that_predates_finalize_is_writable_once_the_book_is_active` (finalize refuses before
  the materials exist with `conception.incomplete`, then succeeds; the legacy chapter is unchanged,
  takes a `chapter_write` session, and the next create still numbers 2),
  `a_side_track_of_a_conceiving_book_gets_no_chapter`, and
  `an_active_book_still_accepts_chapters`. Fail-before was real: with the `ensure_book_active` call
  removed, five of the seven fail (four expect `409` and get `201`, and the transition test fails on
  its first refusal); the two controls that assert pre-existing behaviour still pass.

### D-131 The conception studio owns its height; the workspace panel stops scrolling on that route only
- **Context**: the author reported that the conception studio's middle column had no scrollbar of its
  own, that new content did not scroll into view, and that it shared one scrollbar with the
  Outline / Characters / World / Protagonist / Research-topics column. Three separate defects added
  up to that. (1) `BookLayout`'s main panel is react-resizable-panels' `overflow: auto` wrapper and
  its child was `min-h-full`, so the studio's `flex h-full min-h-0 flex-col` had no definite height
  to resolve against: both columns grew with their content, the *panel* scrolled, and the composer —
  a sibling of the conversation column, not a fixed element — scrolled out of view. (2)
  `ActivityStream`'s stick effect depended on `rows.length` only. While a reply streams the live row
  keeps growing and no row is added, so the view followed the first chunk and then stood still until
  the next message. (3) The composer used its state label as the textarea's `placeholder` **and**
  rendered it in the `aria-live` hint line, so a disabled reason appeared twice in a row.
- **Decision**: the height chain is route-aware. `BookLayout` decides by pathname
  (`isConceptionStudio`, exported and unit-tested): on `/books/:id/conception` the main panel gets
  an inline `overflow: hidden` and its child `relative h-full`; every other route keeps
  `min-h-full` and the panel's own scrolling. Only an inline style can win here, because
  react-resizable-panels writes `overflow: auto` inline on the panel's inner wrapper — a Tailwind
  class would lose. The studio root is `flex h-full min-h-0 flex-col overflow-hidden` with exactly
  two scroll containers, `data-scroll-pane="conversation"` (owned by `ActivityStream`, passed in as
  a new `paneName` prop) and `data-scroll-pane="document"`. Header, session controls, error notice,
  pending form, composer and the finalize bar are siblings of those containers, so they cannot
  scroll. A pending `ask_user` / `request_review` form is capped at `max-h-[45%]` with its own
  scrollbar: with two questions it is ~370px tall and had squeezed the conversation pane to 16px.
  `ActivityStream` now also keys its follow effect on `tailSignature(rows)` — the last row's key,
  its text and reasoning lengths, its block lengths and its tool-call states — so a row that grows in
  place is followed too. The decision is a pure function (`isNearBottom`, 80px) and the scroll is
  issued one animation frame later via `scrollToEnd()`, which targets the element's own max scroll
  offset instead of the last item's estimated end. The composer renders its state label once, in the
  live-region hint; the textarea gets a stable `aria-label` (`agent:composer.label`) and
  `aria-describedby` pointing at that hint, and a neutral `placeholder`
  (`agent:composer.placeholder`) only while it accepts input. Both new keys exist in `en` and
  `zh-CN`.
- **Consequences**: **the other workspace routes are untouched by construction** — the studio branch
  is a different `className`/`style` pair selected by the pathname, and `BookLayout.test.tsx`
  asserts both directions (studio: `h-full` + inline `overflow: hidden`; chapter/overview/canon/
  settings routes: `min-h-full` + the library's `overflow: auto`), so a regression fails a unit test
  rather than only a manual check. Verified in the running app: on the studio the panel's
  `scrollHeight === clientHeight` (no outer scrollbar), the conversation pane scrolls from 983 to 0
  while the document pane, the page and the panel stay at 0, and appending 1200px to the document
  column moves it to 400 while the conversation stays at 983. On `/books/1/chapters/3`,
  `/books/1/canon/entities` and `/books/1` the panel still scrolls the page (`scrollTop` 600 of a
  2416/2416/2772 range). Streaming evidence from a recorded `requestAnimationFrame` loop while the
  Demo provider streamed: content grew 1414 → 1462 → 1582 → 1510 and the viewport was out of
  position for exactly one frame per growth (48px, then 120px), returning to distance 0 both times;
  with the author scrolled to `scrollTop` 40 the content still grew to 1606 and the viewport did not
  move at all, and scrolling back to the end re-pinned it. The disabled hint appears once in both
  locales (one match for the reason string in the rendered page, in a disabled `Message`
  textarea with no placeholder). Tests: `agent.test.ts` (threshold table and
  tail-signature cases), `ActivityStream.test.tsx` (a growing live row is followed, a settled tool
  result is followed, scrolling up stops/no-ops, scrolling back re-pins),
  `conceptionStudio.test.tsx` (the two scroll panes are distinct and neither contains the composer,
  the finalize button or the tab list; the root is bounded), `BookLayout.test.tsx` (both route
  directions) and `Composer.test.tsx` (one hint, stable accessible name, neutral placeholder,
  feedback mode, IME guard).

### D-132 A model option the provider's protocol cannot send is refused on save, reported on read, and absent from the form
- **Context**: the author reported that book 3's conception session never triggered the model's
  reasoning. Book 3 resolves to model profile #4 ("DeepSeek V4 Pro"), whose provider "Magpie" is
  `protocol: "openai"`, but whose `options` held `{"effort":"high","thinking_display":"omitted",
  "cache_ttl":"1h"}` — **Anthropic** fields. `ModelOptions` has two separate reasoning knobs,
  `effort` (Anthropic) and `reasoning_effort` (OpenAI-compatible); the OpenAI adapter sends only the
  latter, so the request carried no reasoning parameter at all. The session ran, the turn
  succeeded, and nothing anywhere reported the mismatch, which is why the author could not tell
  what was wrong. Audit of the author's database (copied to `target/tmp`, never touched in place):
  profiles #2, #3 and #4 all had the same defect, because the settings form offered a **flat list
  of every option for every model**, including `effort` for OpenAI-protocol profiles — and never
  offered `reasoning_effort` at all. The same defect class as the M2 incident recorded in
  `providers.rs` (unvalidated options silently dropped `reasoning_replay: always`), which had never
  been generalised into a cross-protocol rule.
- **Decision**: the option → protocol classification is stated once, in `crates/llm`
  (`ModelOptionKey::reachable_by`), next to the adapters it describes, and **verified against them**
  by `crates/llm/tests/model_options.rs`: for every option, both request bodies are built and the
  option must reach the wire exactly for the protocols the table lists. The independent reading of
  the adapters **corrected two assumptions** the bug report itself carried: `reasoning_replay` is
  *both* (the Anthropic adapter replays its stored blocks unconditionally, which is `always`), and
  `extra_body` is **OpenAI-only** (only `openai::build_body` merges it; the Anthropic body has no
  such hook). `cache_ttl` is also *both*: the server resolves it into `CacheHints.ttl`, which only
  the Anthropic adapter renders as breakpoints — the OpenAI body has none by design (spec/04 §2).
  The Demo (`mock`) protocol is classified as sending **nothing**, because its in-process client
  ignores the request entirely. The rule is then applied in three places: (1) `POST`/`PATCH
  /model-profiles` returns `400 validation.failed` with one `details.fields.<option>` entry per
  stranded field, naming the field and the protocol, and pointing at the counterpart
  (`effort` → `reasoning_effort`) where there is one; (2) reads never fail — `ModelProfileDto`
  gains `ignored_options`, and startup logs one warning per mismatched profile, because the
  author's database already contains such rows and a failed read would lock them out of the page
  that has to fix them; (3) `GET /provider-presets/options` publishes the same table (with `kind`,
  closed `values` and the effective `default_value`), and Settings › Models renders **only** the
  options the selected provider's protocol can send, with human labels and a one-line explanation
  per option in both locales. The option form is protocol-driven: switching a profile to a provider
  with another protocol removes the now-stranded values *visibly* (an "Options removed" notice),
  and an already-stored mismatch shows a callout naming the fields with a one-click "Remove the
  unsupported options" that PATCHes exactly those fields away. A value equal to its option's
  default is not treated as set, since profiles are stored with every field serialized.
- **Consequences**: an option that cannot reach the wire is now unreachable through the write API,
  visible in the UI, and named in the log; the shipped presets were audited and **none was wrong**
  (Anthropic ships `effort`, DeepSeek/OpenRouter/Moonshot ship `reasoning_replay`, Ollama ships
  `send_parallel_tool_calls`, the Demo preset ships none), and that audit is now a test over the
  whole preset table so a future preset cannot regress it. `extra_body` was confirmed on the wire to
  merge last and win, which makes it the documented escape hatch for a gateway knob with no option.
  The client-side table is served by the API rather than copied into TypeScript for the same
  reason the bug happened: a second copy of a protocol rule is a second place to be wrong. The
  author's own row still needs one manual action (documented in the report): open Settings › Models
  and either replace `effort: "high"` with `reasoning_effort: "high"` (dropping `thinking_display`
  and `cache_ttl`, which the OpenAI protocol also cannot send) or click the offered removal. The
  Demo provider can no longer carry any model option, which is honest — it sends no request — and
  its shipped preset was already empty.

### D-133 The "use the default" choice in the models form is a real value, not an empty string
- **Context**: the author reported 「添加模型的模态框的表单默认值显示是空白的」 — in the Add-model
  dialog every option `Select` displayed nothing, although the form and the served option table both
  knew each option's default. The "use the default" choice was `<SelectItem value="">` with a bare
  `<SelectValue />`. In `@radix-ui/react-select@2.4.1`, `SelectValue` calls
  `shouldShowPlaceholder(value) { return value === "" || value === undefined }` and renders its
  `placeholder` branch — and no `placeholder` was passed, so that branch rendered **nothing**. The
  empty-string item was therefore never a selectable value that displays its own label; Radix
  treated it as "unset". `RolesPage` had the same `<SelectItem value="">` shape (it passed a
  `placeholder`, so its control was grey hint text rather than blank, but the "no model" state was
  still not a real value). The bug shipped with a green suite because every existing assertion asked
  whether a `Select` **existed**, never what it **displayed**.
- **Decision**: the unset/default choice gets an explicit sentinel value, `DEFAULT_OPTION_VALUE`
  (`"__default__"`) in `web/src/features/settings/modelOptions.ts`, with `toOptionSelection` /
  `fromOptionSelection` mapping it to and from `undefined`; the dialog deletes the key, so the
  request omits the option and the server applies its own default. The mapping also maps an empty
  string to unset, so a stored `""` can never recreate a blank trigger. Both Select-backed kinds
  (`choice` and `boolean`) use it, and `RolesPage` reuses the same constant for "no model assigned",
  which maps back to `null` (what the settings API expects). **Passing a `placeholder` is
  deliberately rejected as the fix**: the default is a real, selectable, persistent state that must
  display its actual text ("Default: High", "Not set"), and a placeholder is greyed hint text that
  vanishes on selection and is marked `data-placeholder`. The `number`/`text`/`json` inputs keep an
  empty value, which legitimately means "omit the field"; a published `default_value` is shown there
  as `option_default_named` placeholder text (none of the served numeric/text/json options has one
  today, so this is the uniform rule rather than a visible change). The sentinel is a fixed string,
  so a served value could in principle collide with it: a test asserts it stays outside every
  published `values` list.
- **Consequences**: a fresh Add-model dialog shows `Default: Never` / `Default: Automatic` /
  `Default: max_tokens` for the options that have one and `Not set` for the rest, in both locales;
  the same text is shown in the closed trigger after choosing the default item, and the trigger is
  no longer marked `data-placeholder`. The tests that would have caught the original defect are in
  `modelsPage.test.tsx` — per-option displayed text against the served table, a loop over every
  option of both protocols that fails on any blank trigger, the sentinel round-trip (the PATCH body
  omits the key, and never contains `""` or `__default__`), and the `boolean` kind — plus
  `rolesPage.test.tsx` for the role pickers. Reverting the page to the empty-string item fails seven
  of those tests, so the guard is verified rather than assumed.

### D-134 A review verdict starts unset, and every click on it is visible
- **Context**: the author reported 「通过点了没用」 with a screenshot of the conception review form
  showing all five rows (大纲 / 人物 / 世界观 / 主角 / 资料主题) already dark on 「通过」. The form
  initialised **every** item to `approved: true` (`ReviewRequestForm.tsx:64-66`) and expressed the
  verdict only through the button `variant` (`approved ? "default" : "outline"`). It therefore
  opened pre-answered: clicking 「通过」 on an untouched row produced the same state, the same
  variant and the same payload, i.e. nothing at all. 「需要修改」 *did* respond (it flipped the
  variant and revealed the feedback box), and that asymmetry is what made the dead button look like
  a dead form. The suite was green throughout because every existing review-form test asserted on
  `buildReviewPayload`'s output, never on what the form rendered or how it changed.
- **Decision**: the per-item verdict is **tri-state** — `unset` → `approved` / `needs_changes` —
  and starts `unset`. This is the domain's real shape: the payload's `approved` is a per-item bool
  the author is being asked to fill in (spec/05 §7), and pre-filling it is the form answering the
  review on the author's behalf. Each row now shows its verdict three ways: a labelled pill
  (`待审阅` / `已通过` / `待修改`), the filled variant plus a check/pencil icon on the selected
  toggle, and `aria-pressed` on both toggles, so the state is legible to a screen reader too. A
  selected toggle, when clicked again, **releases back to unset** — that is the ARIA toggle-button
  contract, and it is also the only way to undo `Approve all` per row; without it the second click
  on an already-selected verdict would be the original no-op again. Submit review is disabled while
  any present item is unset and a `role="status"` line says how many are left (a silently disabled
  button is its own usability defect). 「通过全部」 is an explicit control, so approving everything
  is a choice the author makes rather than one the form made for them; it is disabled once every
  item is approved, so it cannot become a no-op either. The payload is built from the live state at
  submit time, so approve → needs changes → approve sends the last click; an unset item is omitted
  (the payload's items are optional, spec/05 §7 — but the form never produces that state through
  its own UI); `feedback` is sent only for a rejection, because a draft typed while an item needed
  changes must not ride along with `approved: true` (the prompt renders an approved item's feedback
  too). Typed feedback survives a toggle, so a stray click does not destroy the author's text.
- **Consequences**: the author can no longer submit a review without looking at each item, which is
  what `request_review` is for; `AskUserForm` was audited for the same pattern and is clean —
  every radio and checkbox starts unchecked and its controls are native inputs whose checkedness
  the DOM exposes, so the first click always changes something (a radio group with a *real*
  pre-selection would also be fine, since the author can see and change it, but this form has none).
  The old `approved: true` default is gone, so reopening a form on a session that is still awaiting
  input shows `待审阅` rather than a fabricated approval. Regression guards: the DOM-level tests in
  `reviewRequestForm.test.tsx` assert the rendered `data-verdict`, both `aria-pressed` values, the
  button variant, the feedback box's presence and the submit button's enabled state after each
  click — reverting the initialiser to `approved: true` fails them immediately — plus the
  `conceptionStudio.test.tsx` case that reproduces the reported screen (five unset rows, one click,
  one row changes) and `conception.test.ts` for the pure transitions and payload shapes.

### D-135 The canon actions pin their session in the agent panel and say so
- **Context**: the author reported 「建立档案只有侧边栏模式」 — that canon seeding exists only in the
  sidebar mode. PROJECT §8.4 F2 step 6 says the archivist "seeds canon from the settings (live in
  the agent panel)" and spec/11 §5.3 puts a book's sessions in the panel, so a dedicated route would
  be unrequested scope and a second studio. The real defect was narrower: `NextActionCard` handles
  six kinds, and four of them lead somewhere — `continue_conception` is a `Link` to the studio,
  `review_chapter`/`continue_chapter` are `Link`s to the chapter, `write_next` navigates to the
  chapter it created. `seed_canon` and `sync_canon` were bare buttons: they called the mutation,
  spun while pending, and then showed nothing — no toast, no destination, no hint that a
  multi-minute archivist run had started and where it could be watched. Two of six behaving
  differently is what read as "seeding only exists in the sidebar".
- **Decision**: both archivist actions, on success, (a) **pin the returned session** in the
  persisted UI store (`useUiStore().setPinnedSession(bookId, sessionId)`) — which is exactly what
  `AgentPanel`'s `pickSession` prefers, so the panel switches to the seeding run without a route,
  (b) **reopen the agent panel** if the author had collapsed it (the panel is `collapsible`,
  spec/11 §5.1; a hint pointing at an invisible panel is no better than no hint), (c) raise a
  success toast and render a `role="status"` line naming the session, and (d) show a **hint before
  the click** saying the run happens in the agent panel. The hint has a second wording for a
  collapsed panel plus a **Show the agent panel** button, because the workspace has no other
  control for that panel (it is reopened only by dragging the handle or pressing Enter on it). The
  workspace hands `agentPanelCollapsed`/`expandAgentPanel` to routed pages through the outlet
  context (`features/workspace/shell.ts`); the collapsed flag comes from the panel's own
  `onResize` (0 px *is* collapsed for a collapsible panel), not from
  `ui.agentPanelCollapsed`, which nothing has ever written and which would drift from the screen.
- **Consequences**: the entry point now guides instead of swallowing the click, and the panel stays
  the location (no spec change; PROJECT §8.4 already says so). The panel is a generic session view —
  it shows the archivist's tool cards with their canon writes, its pause/stop controls, the usage
  footer and the session switcher, and it scrolls, so a run with dozens of canon writes is
  followable; a dedicated seeding surface would duplicate `ActivityStream`/`SessionControls` for one
  session kind, which is why it is not built. One residual gap stays open and is logged here rather
  than widened into this change: the overview's **attention list** renders the same open
  `canon_seed`/`chapter_resync` sessions as rows whose target is the page they are already on
  (`/books/:bookId` for a session without a chapter), i.e. clicking such a row does nothing
  visible — the same defect class on a second surface. The fix belongs in the attention list (its
  session rows should pin like the next action does) and is not part of D-135's scope. Regression
  guards in `overview.test.tsx`: the pin is asserted on the store for both kinds, the visible
  `status` line and the toast are asserted, the collapsed-panel wording and the reopen call are
  asserted, a failure pins nothing, the four hint keys are asserted in both locales, and a
  table-driven test over all six `NextActionKind`s asserts each one either navigates or calls a
  mutation whose success is visible. Reverting `seed_canon`'s `onSuccess` to a no-op fails five
  tests; dropping the pre-click hint fails five; removing the panel reopen fails one.

### D-136 Running research tasks is the overview's step between the conception and the chapters
- **Context**: the author reported 「为什么从构思到正文我没有看到知识积累这个步骤」 — why is there
  no knowledge-accumulation step between conception and drafting. Their own database confirmed the
  silent skip: conception completed, `canon_seed` completed with 43 entities, **8 research tasks
  all `ready`**, **0 `knowledge_entries`**, and chapter 1 already `drafting`. Nothing had told them
  a step was pending. The cause was two documents disagreeing: PROJECT §8.4 F2 step 6 says the
  overview after finalize "suggests **'Run research tasks'** and 'Write chapter 1'", but spec/08 §7
  listed six `next_action` rules with **no research branch**, so `NextActionKind` had no research
  variant and the implementation followed spec/08 faithfully into the gap. The tasks were surfaced
  only as a grey `ready_research` row in `attention[]` while the prominent card said "Write the
  next chapter". AGENTS.md §8 makes the conflict the executor's to resolve and log; PROJECT is the
  correct behaviour here, because the writer's kickoff and canon maintenance read knowledge
  (spec/07 §5) and with zero entries both improvise from canon alone. The author chose option (a):
  research tasks become a **required** step of the overview's next action.
- **Decision**: (a) new `NextActionKind::RunResearch` (wire `run_research`), inserted **between
  `seed_canon` and `review_chapter`** — i.e. PROJECT's sentence order, before every chapter rule.
  (b) It triggers on at least one research task of the book with status **`ready` only**, filtered
  through the typed `ResearchTaskStatus::Ready` constant. `draft` does not count: the author has
  not confirmed the topic and Run is refused in that state (spec/07 §8), so offering it would be a
  dead end; `running` has a session the author is already watching; `done`/`failed`/`cancelled`
  are not runnable. (c) `NextActionDto` gains `ready_research: Option<i32>` — the count of
  runnable tasks, so the card says "Run 8 research tasks" instead of an unspecific "Run research
  tasks" — and `research_task_id: Option<i32>`, set **only when exactly one task is runnable**.
  Neither field changes the meaning of `chapter`, `session_id` or `track_id`. (d) `NextActionCard`
  renders the action as a `Link`, not a mute button: to `/research/:taskId` when the server names a
  single task (that page's header carries its Run button, so the author lands where the click is),
  otherwise to `/research`, where the author has to choose a task — with several, naming any one of
  them would be an arbitrary pick, and only the list shows the choice. The count is in the label in
  both cases, so "several" is never ambiguous. (e) The `ReadyResearch` attention row is **kept**: the
  attention list is the full picture, `next_action` is the single suggestion. (f) spec/08 §7 is
  amended in this commit: the new rule is rule 3, the chapter rules renumber to 4–7, and the
  ordering consequence is written into the section.
- **Consequences**: research now **gates** writing. A book with any `ready` task keeps answering
  `run_research` and will **not** be prompted to write its next chapter until those tasks are run,
  failed, `cancelled` (or, from the follow-up task, deferred) — a chapter `in_review` or `drafting`
  waits behind it, though its `attention[]` row still shows it and stays clickable, so nothing is
  hidden. This is the trade-off the author accepted when choosing option (a): without the gate the
  empty knowledge base was invisible, and the writer improvised from canon alone. The escape
  hatches are running the tasks from `/research` or cancelling the ones they do not want; a
  non-destructive "defer" is a separate follow-up rather than part of this change, and the
  `Ready`-only filter is what lets that status drop out of the trigger without touching this rule.
  Books without research tasks are unchanged (the no-regression case asserts `write_next`), and
  so are rules 1 and 2 — a conceiving book still continues its conception and an unseeded book
  still seeds first, even with runnable tasks. Tests: `crates/server/tests/overview_actions.rs`
  (ready tasks are the next action with the count and, for one, the task id; three ready tasks name
  no single target; `draft`/`done`/`failed`/`cancelled` do not trigger it and do not count in
  attention; a book without tasks still writes; research outranks a chapter in review, with both
  attention rows intact; conception and seeding still outrank research; another book's tasks do not
  leak) and `web/src/features/overview/overview.test.tsx` (plural label 8/1, `/research` for several,
  `/research/:id` for one, list fallback when the id is absent, zh-CN rendering, both plural forms in
  both locales, and `run_research` added to `KIND_TABLE`, whose exhaustive-kind assertion fails if
  the table falls behind the union). Fail-before: removing rule 3 fails three server tests and
  dropping the fixture's constant from `KIND_TABLE` fails the coverage test.

### D-137 The agent panel reviews a held call where it is held, in the tool's own words
- **Context**: the author reported 「章节细纲等待我的审阅，但是侧边栏没有审阅能力」 — the chapter
  outline is waiting for my review, but the sidebar has no ability to review it. Their database was
  a literal reproduction: chapter 2 (`drafting`), session 9 (`chapter_write`, `held`,
  `held_tool_call_id` 282), call 282 `submit_outline` carrying 3,729 characters of outline. Nothing
  was broken on the server. `resume` with no feedback already *approves* a held call (it finalizes
  it with its deferred effects); `resume` with feedback already *rejects* it (it clears the effects,
  appends `writer.notice.outline_rejected` and the feedback, spec/03 §5.4). The gap was that the
  panel's controls were purely **status** driven, so a held session showed the same generic
  「继续」(Continue) and 「反馈…」(Feedback…): nothing named the tool, nothing showed what was
  submitted, and nothing said that Continue means "approve this outline". The spec calls this exact
  workflow its most common case (spec/03 §7: "The most common case is editing the outline while
  `submit_outline` is held, then approving it") and it had no affordance at all. This is the fourth
  report of the same class ("the UI does not let me do the thing"), so the fix had to name the
  decision, show its object, and keep the two surfaces of this panel coherent.
- **Decision**: (a) a new `HeldCallReview` block in `features/agent`, rendered by `SessionControls`
  under the control bar whenever the status is `held` **and** `held_tool_call_id` resolves to a
  call in the snapshot. It names the held tool through the registry the tool cards already use
  (`agent:tools.<name>.summary` plus the renderer's icon) — no second naming scheme, no raw
  identifier; it renders the submitted body inline (the tool's own field: `outline` for
  `submit_outline`, `plan` for `submit_revision_plan`, the prose field for a held write tool) as
  bounded scrolling Markdown, plus the reference notes for an outline. (b) The two decisions are
  labelled in the domain's words: **Approve** / **Request changes** for `submit_outline`
  (「通过章纲」/「提出修改」, reusing the chapter `HoldBar`'s wording, spec/11 §5.2), **Approve
  revision plan** / **Request changes** for `submit_revision_plan`, and a generic **Approve and
  continue** for every other hold (`pause_requested`, `auto_pause`). (c) The payloads are the
  documented contract and nothing else: approve calls `resume` with `{ sessionId }` and **no**
  feedback key; request changes calls it with `{ sessionId, feedback }` where `feedback` is exactly
  the trimmed text. The submit button is disabled while the box is blank or whitespace-only,
  because the server filters blank feedback to `None` and would then **approve** the call — a
  silent opposite outcome. (d) The block replaces the generic Continue / Feedback… pair for a
  resolvable held call instead of sitting beside it, and the composer is disabled there
  (`agent:composer.disabled_review`), so a held call has exactly one approve path and one reject
  path. Abandon stays in the overflow menu behind its confirmation in every status. (e) No server
  endpoint and **no DTO change**: `SessionSnapshotDto` already carries `session.held_tool_call_id`,
  `tool_calls[]` and each call's full parsed `input` (`ToolCallDto.input`), which is everything the
  block needs. The snapshot window (200 messages) is the only limit; see the degradation below.
- **Consequences**: the decision can be made where it is held — the author reads the outline in
  the panel and approves or rejects it without hunting for a collapsed tool card (the stream is
  virtualized, so a card scrolled out of view is not even in the DOM, and the card shows raw JSON).
  The mechanism is untouched: one endpoint, one pair of payloads, so the panel, the chapter
  `HoldBar` and the studio cannot drift apart in *behaviour*, only in wording — and the wording is
  now identical where it overlaps (`通过章纲` / `提出修改`). **Degradation, stated**: a `held`
  session whose call is not in the snapshot window (or whose snapshot is still loading) shows no
  review block and keeps the status-driven Continue / Feedback… pair; there is nothing to render
  and guessing would be worse than the old controls. The composer keeps its "Feedback & continue"
  mode in that case, so the generic reject path is still reachable. **Relationship to the
  conception studio (the consistency question)**: the studio's `AskUserForm` / `ReviewRequestForm`
  are the **`awaiting_input`** counterpart — one surface per waiting status, each rendered by the
  screen that owns the conversation, both above the composer, both disabling the composer while
  they hold the decision, and both using the same verbs (approve vs. request/needs changes). They
  are not unified into one component because their payloads differ (`answer` with a form payload
  vs. `resume` with optional free text) and their content is shaped differently (per-item verdicts
  vs. one submitted document); the rule that *is* shared is that the surface owning the decision
  also disables the composer, which the studio already did and the panel now does.
- **Regression guards**: `heldCallReview.test.tsx` asserts the exact mutation payloads —
  `{ sessionId: 9 }` for approve, `{ sessionId: 9, feedback: "…" }` for request changes — through
  the real `AgentPanel` with mocked hooks, plus the generic `auto_pause` case, the unresolved-call
  degradation, Abandon reachability, and that the rendered text is the outline Markdown rather than
  the raw input. Fail-before, verified three times: resolving no held call fails four tests,
  forcing `reviewHeld` to `false` fails four, and wiring approve to `onRequestChanges("")` fails
  six. `SessionControls.test.tsx` keeps the paused and degraded-held cases; `Composer.test.tsx`
  pins the review-block mode; `locales.test.ts` requires every new key in both locales.

### D-138 A deferred research status is the release valve for the required research step
- **Context**: D-136 made running research a *required* step of the overview's `next_action`
  (`run_research` sits between `seed_canon` and the chapter rules), so a book with any `ready`
  research task is never prompted to write. The author reported the consequence in their own
  database: book 3 has a `drafting` chapter, 0 knowledge entries, and 8 `ready` research tasks, so
  the overview keeps asking for research and the chapter cannot be advanced from the card. Without
  a third option the only ways out were to run a task (a model call and real time, per task) or to
  cancel it (loses the topic, the requirements and the intent). Deferring is the author's "I know
  about this, not now".
- **Decision**: a new `ResearchTaskStatus::Deferred` (wire `deferred`) with two dedicated
  endpoints, `POST /api/research-tasks/:id/defer` and `.../resume`. (a) *Transitions*: `ready` →
  `deferred` and `deferred` → `ready`; `run` also accepts a `deferred` task straight to `running`.
  Refused with `409 research.invalid_state`: `draft` → `deferred` (an unconfirmed topic cannot be
  postponed, it is confirmed or deleted; it never blocks writing anyway) and
  `running`/`failed`/`done`/`cancelled` → `deferred` (open sessions must finish or stop first, and
  settled work stays settled — deferring it would claim completed work is pending or resurrect a
  dropped intent). `deferred` → `running` is deliberately *not* an author endpoint: it happens
  through `run`, which already owns session creation. (b) *Idempotency*: each endpoint names a
  **target status**, not an action, so a repeat call returns the current row with `200`, writes
  nothing and emits no event; a double-click is harmless. (c) *Dedicated endpoints, not PATCH*: a
  PATCH of `status` already means "confirm a draft", and these are named state changes with their
  own rules. (d) *Overview*: `next_action` rule 3 and the `attention[]` count already filtered
  `ResearchTaskStatus::Ready` only, so `deferred` falls out with no logic change — asserted through
  the real `/overview` handler, not assumed. (e) *Schema*: `0001_init.sql`'s CHECK is edited **and**
  `0002_research_deferred.sql` ships. The edit is what a fresh database and every test fixture gets
  (spec/02 §1 permits it while no release is tagged, and `git tag` is empty). It cannot reach an
  existing database: `seaql_migrations` already records `m0001_init`, and only pending files run —
  so the author's own database would have failed every `deferred` write with a CHECK violation.
  The upgrade rebuilds `research_tasks` (SQLite cannot alter a CHECK in place) with
  `PRAGMA foreign_keys = OFF` around it: `Db::connect` sets the pragma ON and a `DROP TABLE` then
  fires `agent_sessions`' `ON DELETE CASCADE` (silently deleting the researcher sessions) and
  `knowledge_entries`' `ON DELETE SET NULL` (silently unlinking every entry). The pragma is a no-op
  inside a transaction, so the migration declares `use_transaction() = Some(false)` and opens its
  own; a row-count guard turns any such loss into a failed migration instead of a quiet one.
- **Consequences**: the author can postpone everything and get straight back to writing — the
  deferred tasks keep their topic, requirements, scope and language, and the card's count drops to
  the runnable ones. Deferral is *not* a lock and not a cancel: `run` accepts a deferred task,
  `resume` restores the prompt, and the row is otherwise untouched, so the state is recoverable in
  both directions and appears in the record (`deferred` is a real status, so the list filter and
  the badge show it). The UI offers Defer on `ready` rows in the list and on the task page, Resume
  on `deferred` ones, and neither while a session is open (where the server refuses anyway); the
  badge uses the `idle` token with its own label. One gap had to be closed with the feature: the
  SSE handler for `research.updated` invalidated only `keys.research.all`, so a defer made in
  another tab or by an agent left the overview offering `run_research` until a manual reload; it
  now invalidates that book's overview too, and the research mutations do the same for the same
  reason. **Relationship to D-136 (the ruling kept)**: D-136's ordering is untouched — research
  still outranks chapters *while it is runnable*. Deferral is not an escape from the research step;
  it is the explicit, recorded, reversible decision to do it later, which is what a single-author
  tool needs when the gate is otherwise absolute.
- **Regression guards**: `research_deferred.rs` pins the whole matrix in one table (14 edges: every
  allowed transition succeeds, every refused one is `409 research.invalid_state` naming the current
  status, every open-session edge names the session), plus idempotency (a repeat call writes
  nothing and publishes no event), the `research.updated` event, and a deferred task running
  directly. `overview_actions.rs` proves the flip through the real overview: only-deferred →
  `write_next`, deferred + `drafting` chapter → `continue_chapter`, one ready among deferred →
  `run_research` with count 1, resume → `run_research` again. `research_deferred_migration.rs`
  builds a pre-`deferred` database, upgrades it, and asserts the sessions and entry links survive
  (deleting the pragma from the script makes it fail with `CHECK constraint failed: lost = 0`).
  `researchDeferred.test.tsx` covers the buttons and the badge; `events.test.ts` covers the
  overview invalidation (removing it fails the test).

### D-139 A held write is staged, not committed: the chapter moves in the transaction that finalizes the call

- **Context**: every draft-writing tool (`insert_text`, `insert_paragraphs`, `replace_text`,
  `rewrite_paragraph`, `delete_paragraphs`, and `finish_draft` when it closes an open tail) wrote
  `chapters.content`, `content_version`, `word_count` and the `chapter.content` event **inside
  `execute`**. The hold decision is made after `execute` returns (spec/03 §5.2), so a call that held
  for approval had already changed the chapter, and only its *effects* (phase change, completion)
  were deferred. The data proved it: a chapter sat at `content_version` 1 with 1,823 words and an
  `updated_at` 5 ms after a `held` `insert_text` started, and the call's `input.text` was
  byte-identical to the stored paragraphs. Approve and reject changed nothing about the text; a
  rejected write stayed in the chapter, and the UI showed a word count in the header that the body
  denied ("本章还没有正文").
- **Decision**: a text-changing call **stages** its write and the transaction that finalizes the
  call applies it. `execute` computes the new `ChapterContent` exactly as before — granularity
  validation and `tail_open` stay there, so a rejected call still writes nothing — and returns it on
  the outcome. `finalize` applies it when the call is not deferred, so content, `content_version` + 1,
  `word_count` and one `chapter.content` event move together with the call's status; when the call is
  deferred the staged write is stored in `tc.effects.pending_content` and the chapter is left
  byte-identical. Storing it **in `ToolEffects`** rather than in a new column is the load-bearing
  choice: §5.4's "feedback on a held call revokes its deferred effects" is implemented by resetting
  `effects` to default, so a rejected write is discarded by the code that already exists, with no
  second code path to keep in sync, and no migration.
- **Consequences**: `pending_content` carries the `content_version` the tool read, because the author
  may edit the chapter while a call is held (spec/08 §1.4). Approval applies the staged content only
  while that version still matches; once the author has moved the chapter, the staged write is
  dropped instead of silently reverting their work — the author's text wins and the §7 edit report is
  what tells the model what the draft actually holds (the same mechanism that already covers every
  other tool, and the precedent spec/03 §15 test 18 sets for the outline). Applying an approved write
  also advances the stored `view_snapshot`'s content past it, but only when the snapshot still equals
  the pre-write chapter; without that, the agent's own accepted text would be reported back to it as
  an author edit on the next sealing. `answer_inner` applies the same invariant. The staged content is
  exposed on `ToolCallDto.pending_content` (a `PendingContent` of `content` + `expected_version`), so
  the session snapshot already carries a held write's text — the draft-preview UI needs no new
  endpoint. Ops and the word count are derived at apply time (`transform_ops`,
  `content_word_count`) instead of being stored, so they cannot fall out of sync with the state they
  are applied to. `submit_outline` deliberately keeps writing `chapters.outline` inside `execute`:
  spec/03 §7 makes "edit the outline while `submit_outline` is held, then approve it" the common
  author flow, which requires the outline to be visible and editable while it is held, so it is not
  the same defect (its phase change is deferred, as before).
- **Existing data**: no repair was attempted and nothing was written to the author's database. A
  chapter whose write was held under the old code keeps that text and its bumped `content_version` —
  it is a legitimate committed write as far as the schema knows. The held call's row still says
  `held` with no `pending_content`; approving it after the upgrade applies no staged write (there is
  none), so it finalizes the call and its effects without touching the text again. The author's
  remedy, if they want the rejected text gone, is to edit the chapter normally — a manual SQL repair
  is their decision, not this change's.
- **Regression guards**: `agent_write_tools.rs` proves a held write leaves the chapter empty and
  `word_count` 0, that approval applies it once with `content_version` + 1, and that a granularity
  rejection stages nothing; `agent_write_holds.rs` proves no `chapter.content` is published while
  held and exactly one on approval, that two approvals do not apply either write twice, that the
  snapshot exposes the pending text, and that feedback on a held completing write leaves the chapter
  without a single paragraph of the rejected 2,000-character text (its follow-up write is `p1`, so no
  pid was consumed either); the non-held path keeps one event and one version bump per write. The
  tests that had encoded the bug ("the call executed before it held", "both inserts were written
  before the second held") were corrected, and the author-edit-while-held tests now pre-seed the
  chapter, because a held write is no longer something the author can edit.

### D-140 Bulk defer: one shared batch defers a book's ready tasks from the list and from the overview card
- **Context**: D-138 added `deferred` and the per-row Defer button, which solved the dead end but
  left two gaps the author named. (a) Their database has 8 `ready` tasks across 4 scopes, so
  postponing all of them was eight clicks and the overview insisted on research the whole time.
  (b) The demand appears on the overview (`next_action.kind === "run_research"`, D-136) but the
  release valve did not: the author had to leave the card, find the research list and click again.
  The per-row action is the right grain for one task and the wrong grain for the reported
  situation, which is "all of it, not now".
- **Decision**: one batch operation, `useDeferReadyResearchTasks` in
  `web/src/features/research/useBulkDefer.ts`, used by **both** surfaces. (a) *One place*: the
  hook owns scoping, sequencing, reporting and invalidation, so `ResearchTaskList` and
  `NextActionCard` cannot drift; both render the same `BulkDeferReport` for the partial case. It
  reuses `useDeferResearchTask` rather than adding a code path, so the invalidation D-138 fixed
  (research queries **and** that book's overview) is the one the batch gets. (b) *Scoping*: the
  candidates are the rows of the same book-scoped `useResearchTasks({ bookId })` query the list
  renders, filtered to `status === "ready"` **and** `book_id === bookId`. The second filter is
  defence in depth — the server already filters by book — so a stale or wrongly shaped cache entry
  cannot postpone another book's work. The global `/research` page passes no `bookId` and defers
  every ready task it lists, which is the honest meaning of "all" there. (c) *Sequencing*: strictly
  one request at a time. Eight requests are microseconds apart on loopback, and bounded concurrency
  would cost the property that matters — a failure attributable to one task while the rest
  continue. The targets are snapshotted before the loop, because each success invalidates the list
  and a live array would shrink underneath it and skip rows. (d) *Partial failure*: the outcome
  carries `{ deferred: id[], failed: { taskId, topic, error }[] }`, the toast says "Deferred 7 of 8
  research tasks" with `Not deferred: <topic>` as its description (warning when some moved, error
  when none did), and `BulkDeferReport` renders the same two lines inline, because a toast is
  transient and the rows underneath are the evidence. Topics, never ids: ids are not shown to the
  author anywhere. (e) *No confirmation dialog*: deferring is reversible (Resume, D-138), so a
  blocking dialog would tax the common case; instead the control names its scope ("Defer all (8)")
  and the hint says it is postponing, not deleting. (f) *No batch endpoint*: eight idempotent
  requests from a client loop are the expected shape, the endpoints already name a target status so
  a retry is safe, and a server batch would add a transaction boundary, an error-reporting shape and
  a DTO for a call that costs eight round trips on loopback. (g) *Overview card*: the primary stays
  primary — the "Go" link is the only `default`-variant control and still opens `/research` (or
  `/research/:id` when `research_task_id` names exactly one task); the defer control is a `ghost`
  button in a muted row under it, with the consequence in a hint. The card's count comes from the
  list query the batch defers, falling back to `ready_research` only while that query is loading, so
  the label and the click cannot disagree. On success the overview refetches to the next action, so
  the card is replaced rather than left asking. (h) *The class-level guard*: `KIND_TABLE` still
  describes the **primary** affordance per kind, and now declares the secondary explicitly in an
  `Expectation.secondary` field (`run_research` → `"deferResearch"`), with a test that the table's
  declared secondaries match what the card really renders and another that the two are
  distinguishable (link + `default` vs button + `ghost`). The table no longer quietly describes less
  than the card does.
- **Consequences**: the author postpones the whole backlog in one click from either surface and
  gets straight back to writing; the deferred tasks keep everything (D-138) and Resume restores them
  one by one. A partial batch is visible twice (toast and inline) and names the tasks that did not
  move. Because the loop is sequential, a slow or hanging request delays the rest — acceptable at
  eight rows and preferable to an unattributable parallel failure. The count is per surface scope,
  so a book page never says "all" about another book's tasks.
- **Regression guards**: `useBulkDefer.test.tsx` pins the shared operation against a real query
  cache — exact ids and order, one request in flight at a time (peak concurrency 1), the
  `book_id` defence filter, the partial and all-failed reports, and that the list refetches while
  that book's overview is invalidated and another book's is not. `researchBulkDefer.test.tsx` pins
  the list: the count in the label, exact id set, nothing for `deferred`/`draft`/`done`, absence when
  nothing is runnable or the list is empty, the inline partial report, and the rows flipping to
  Deferred. `overview.test.tsx` pins the card: `Go` stays a `default` link and still targets
  `/research/:id` for the single task, the defer control is `ghost`, the exact ids, the overview
  invalidation, the partial report, and the `KIND_TABLE` honesty tests. Removing either control
  fails 9 of those tests; replacing the shared batch with a parallel, swallowing one fails 3 more.

### D-141 A held chapter write is reviewed in the draft pane, against the merged text
- **Context**: D-137 gave a `held` session one review surface (`HeldCallReview`) in the agent panel,
  because a bare Continue never said which tool was held or what it submitted. That fix was right
  for `submit_outline` and `submit_revision_plan` — the text to judge *is* the message — and wrong
  for a chapter write. The author reported the split in those words: 「正文编写要在中间的正文草稿区
  进行，正文编辑时的意见提出在草稿区下方的 dock 栏里进行」 — the body is written in the middle draft
  pane, so an opinion about it belongs in a dock under that pane. The panel's block showed the raw
  `input` text of the one call, while the draft pane (the place the author was looking) showed
  nothing at all: a held `insert_text` arrived as a live ghost, then vanished from the document,
  because D-139 keeps it out of `chapters.content`. On top of that, three smaller contradictions
  came with it: the header showed the server's `word_count` (which excludes the staged text) next to
  a body saying 「本章还没有正文」; a stale write could be approved from the panel while
  `apply_pending_content` silently dropped it; and `HoldBar` under the paragraphs offered a *third*
  wording of the same two decisions.
- **Decision**: the pane shows the draft the author is being asked for, and the dock under it takes
  the decision.
  - **Pending content is a full snapshot, not a delta** — this is what makes the preview possible and
    is confirmed in code: `crates/server/src/agent/tools/writer.rs` `stage_content` clones the whole
    resulting `ChapterContent` into `PendingContent.content`, and
    `crates/server/src/agent/runner.rs::apply_pending_content` stores `write.content` verbatim
    (`content: Set(stored)`) and derives `transform_ops` and `content_word_count` *from the state it
    is applied to*. So `pendingDraftView` renders `pending_content.content` as the document and
    consults the confirmed draft only to classify each paragraph as `confirmed` / `changed` /
    `added` — never to concatenate.
  - **Merged, highlighted**: `PendingText` renders that document with the waiting token on the
    `changed` and `added` paragraphs plus a note that the text is a preview. It is read-only:
    editing it would save a paragraph the author cannot see, and the save would bump the very
    `content_version` the staged write is compared against.
  - **The superseded case does not lie**: `apply_pending_content` drops a write whose
    `expected_version` is not the chapter's current `content_version`. So `PendingWriteDock` reads
    that comparison and, when it fails, keeps the confirmed draft on screen and says the staged text
    will not be applied, naming both versions; its primary action becomes **Continue without it**
    (the same `resume` — the write is discarded in the finalizing transaction, spec/03 §7's edit
    report tells the model what the draft now holds). There is no "apply" button that cannot apply.
  - **One decision surface per held call**: `heldDecisionSurface` gives a write tool that staged
    content to the dock, and everything else to the panel. `SessionControls` applies the rule
    itself, so a caller cannot forget it: a dock-owned call renders `HeldWriteNotice` (a sentence
    plus a link to the chapter) and neither decision, and the composer is disabled with
    `agent:composer.disabled_dock` instead of "the review above". A write hold that staged nothing
    (any call from before D-139) stays in the panel, because the dock would have no text to show.
  - **Word count, one rule**: the header keeps the server's confirmed `word_count` (spec/11 §8 is
    authoritative) and gains a labelled projection in the waiting token; the body's line names both
    (`{{staged}} words once applied ({{confirmed}} now)`). The stored count is never replaced by a
    count of text the chapter does not hold.
  - **Extraction, not copies**: `features/agent/heldCall.ts` owns which call is held, its body text,
    its wording case, its target pids, the surface rule and the **two mutation payloads**
    (`approveHeldInput` = `resume` with no `feedback` key; `requestChangesInput` = the trimmed
    feedback, or `undefined` when blank); `useHeldDecision` wires them to the resume mutation for
    both surfaces; `HeldFeedback` is the one request-changes box (trim, blank-disabled, ⌘⏎ with the
    IME guard). `HeldCallReview` re-exports the pure helpers so its existing call sites and tests
    keep one import site.
- **Consequences**: the author decides where they write, on the text they will actually get, and the
  panel never offers a second, differently-worded way to make the same decision. A held write that
  the author outran is now visibly dropped instead of failing silently. The dock is part of the
  chapter page (not the layout shell), so it inherits the page's data and a reload restores it; it
  is sticky inside the middle column so the decision cannot scroll away from the text. The cost is
  that the chapter page now reads `pending_content` and computes the merged document on every
  render where a write holds — bounded by the chapter's paragraph count, and skipped entirely when
  nothing is staged.
- **Regression guards**: `pendingWrite.test.ts` pins the snapshot semantics (a rewrite plus an added
  paragraph renders as the staged document, not as a concatenation; a deleted paragraph disappears;
  a superseded write is described but not applied) and both word counts. `heldCall.test.ts` pins the
  payloads (`{sessionId}` exactly, trimmed feedback, `undefined` for blank — the server reads blank
  feedback as an approval), the surface rule per tool name, and the pid extraction.
  `pendingWriteDock.test.tsx` pins both dock states, the exact payloads and the blank guard.
  `heldWriteSurface.test.tsx` pins that a write hold yields no `held-review` block, no
  Continue/Feedback pair and no approve button in the panel, that the notice links to the chapter, and
  that a `submit_outline` hold still renders `HeldCallReview` unchanged. `pendingPane.test.tsx` pins
  the merged rendering, the read-only pane, the two-count word line and the absence of `HoldBar` for
  a write hold. `wordCount.test.tsx` pins the confirmed-plus-projection rule.

### D-142 Entity `description` has a stated ceiling, and per-chapter developments no longer go into it
- **Context**: the author reported 「实体的详细描述过于详细了（比如林昭的），每章加一点以后还得了，改改预置提示词控制一下」. Their book (id 3) has 44 entities holding **8,290 characters** of `description` in total. The four text fields are meant to be a hierarchy — `summary` one-line identity, `state` the instant condition, `attributes` the discrete facts, `description` the stable reference text — and the archivist had collapsed all four into `description`: 林昭 has a 41-character `summary` and a **672**-character `description` (16×), 大肥鱼 49 vs **705** (14×); `description` is 61% of all four fields' stored text for the book and 65% of 林昭's own entry. The prompts invited exactly this: `tools.toml` called the field 「详细描述」 (detailed) with no size signal, `update_entity.description_append` was 「追加到详细描述末尾的新信息」 with no limit and no "only if durable" rule, and the **archivist system prompt literally instructed** 「补充新信息时用 description_append 追加，不要整段重写」 — so every chapter added a paragraph and nothing ever removed one. The accumulated text also costs context on every retrieval: `get_entity` renders `description` in full via `result.entity_card` (only the generic 12,000-character result cap applies, which a single card never reaches), and the archivist's maintenance step 2 listed 「描述发生变化」 among routine reasons to call `update_entity`.
- **Decision**: a stated ceiling of **about 120 characters (2–4 sentences)** on `description`, enforced by prompt text only — no schema constraint, no rejection path (the task asked for exactly that: 「改改预置提示词」).
  - **Why 120**: against the measured distribution, 120 is the median-ish anchor that separates "reference entry" from "biography" — the book's median `description` is 104 characters and 24 of 44 entities are already at or below 120, so the number describes the good rows rather than inventing a new standard. The 20 entities over 120 hold 6,368 of the 8,290 characters; at the ceiling the book's total would be **4,322 (−48%)**. It is also *estimable*: a character count is something a model can check while writing, unlike "be concise", which the old text effectively said.
  - **Where the rule lives**: `tools.toml` `create_entity.description` and `update_entity.description` state the ceiling **and what belongs elsewhere** (`state` current condition, `attributes` one fact per field, `summary` one-line identity); the two tool top-level descriptions repeat the short form; `description_append` is restricted to information that is still true 20 chapters later and permanently redefines the entity, repeats the ceiling, and says to **rewrite tighter with `description` rather than append** once the text is at or past it. The archivist system prompt's field-semantics block was rewritten (its old "always append, never rewrite" sentence was the direct cause). All three `archivist.kickoff.*` prompts that write entities (maintenance step 2, seed step 1–2, resync) now say `description` is stable reference and **not a chapter log**, that per-chapter developments belong to `state` + `add_memory_note` + `add_timeline_event`, that discrete facts belong to `attributes`, and that an over-long `description` is rewritten tighter.
  - **Condensing is encouraged, not silent**: maintenance step 2 says to compress an over-length `description` to the ceiling in the same pass that was already touching that entity (「本次本来就要更新这个实体时，顺手把过长的描述压缩到 120 字以内」), and the append field says to rewrite instead of append. This is deliberately scoped to entities the agent is already editing: a blanket "trim everything" instruction would make the archivist rewrite 44 rows it has no mandate to touch, and a maintenance session only knows the entities that appeared in its chapter.
- **Consequences**: the existing 8,290 characters are **not** auto-trimmed — no migration, no background job, no repository change (the measured database was only read). Old text stays until the author edits it or until the archivist next updates one of those entities, which is why condensing is written into the same step rather than left to a separate pass. Future chapters stop growing `description` and put their developments in `state`, `add_memory_note` and `add_timeline_event`, which is where the tiered memory reads them from (spec/06 §4). No tool contract changed: `description` is still a free `str` of any length, so old rows and author edits remain valid, and `get_entity` still renders whatever is stored — recommending, not truncating, is the deliberate choice, because a silent cut would hide canon the writer may need. The recommendation for the existing bloat is a manual pass over the 20 entities above the ceiling (the author's own edit forms, with `state`/`attributes` absorbing the chapter-specific sentences), not a script: the sentences that need to go are interleaved with the ones that must stay, and only the author knows which "生平" details are load-bearing.
- **Regression guards**: `prompts_registry.rs::the_entity_description_ceiling_is_stated_where_entities_are_written` asserts the ceiling text, the "not a chapter log" rule and the rewrite-instead-of-append rule in all three archivist kickoffs, the archivist system prompt, and both entity-write tool schemas — a future prompt edit cannot drop the rule silently. The rendered-pack snapshot (`prompts_registry__rendered_prompt_pack.snap`) changed in exactly the seven places the edits touched (archivist.system, the two maintenance branches, seed, the two resync branches, `tool.create_entity`, `tool.update_entity`), which is the proof the text actually landed. `scripts/validate_prompts.py` stays green; no variable was added (the guidance needs none).
- **Spec**: spec/06 §1.1 now states the ceiling and the hierarchy in the field table, adds a paragraph on why the ceiling is prompt-level rather than a schema constraint, and §6 step 2 no longer lists "new durable description details" as a routine reason to extend the field.

### D-143 One active relation per ordered pair: `kind` is data, not identity
- **Context**: the author reported 「关系的 create 应该做 title重复性检查」 with a screenshot of one
  entity's relation list showing several near-identical lines. Their book (id 3) turned out to hold
  63 relations of which **5 pairs carried more than one row**, all created by the archivist with
  drifting wording ("军中下马威者→被说服者" `ended` #36, "军中下马威者（后转为半服气）" `ended` #56,
  "下马威的军中前辈（后转为半服气）" **active** #58 on one pair; "亦师亦友亦兄弟（东宫属官）"
  **active** #28 vs "东宫属官（亦师亦友亦兄弟）" `ended` #54 on another). The check the author asked
  for already existed, for the shape they were not hitting: `ensure_no_active_duplicate` filtered on
  `(source, target, kind)`, so an exact re-create returned `409` — but **0 pairs carried two `active`
  relations of the same kind**, because the archivist never repeated the wording; it re-created the
  pair with the bond re-worded, and `kind` being part of the key made every re-wording a new legal
  active row. `ended` rows never blocked, so the old wordings piled up as history and the active one
  drifted. spec/06 §1.2 *licensed* this ("Several relations of different kinds may exist between the
  same pair (师兄 and 情敌). Only one active relation per `(source, target, kind)`").
- **Decision**: at most one **active** relation per **ordered `(source, target)` pair** per canon;
  `kind` becomes data about the bond rather than part of its identity. A pair that is written again
  with different wording **edits** its existing row.
  - **The pair is ordered, not unordered.** `(A, B)` and `(B, A)` each keep one active row. The
    author's canon requires this: `(3,4) 父亲` and `(4,3) 嫡长子`, `(1,3) 诤臣谋士` and `(3,1) 君主`,
    `(10,1)`/`(1,10) 爱人兼政治盟友` and `(32,10) 父亲`/`(10,32) 次女` are all **legitimately active in
    both directions** — 3 unordered pairs would collide under a symmetric rule. The direction carries
    meaning that `inverse_kind` cannot: the reverse of 父亲 is 嫡长子 only for a specific pair, and
    `inverse_kind` is a label, not a second row. A symmetric guard would reject canon this product
    considers correct, so it was rejected.
  - **Two enforcement points, deliberately.** The service returns `conflict.exists` with `details.id`
    naming the existing row plus its kind in the message (English in code, localised in the UI by
    code, model-readable through the prompt pack), so the author's `POST` and any future caller get a
    usable answer. The two partial unique indexes are re-keyed from `(source, target, kind)` to
    `(source, target)` to catch every path that does not go through the service — a merge, a revert,
    a hand-written statement.
  - **The tool upserts, the API conflicts.** This asymmetry is the core of the fix. `set_relation` is
    the archivist's *only* way to write a relation; making a re-worded kind an error would leave it no
    way to correct a wording it regrets — it would be forced to `end_relation` and re-create, which
    loses history and is exactly the process that produced the duplicates. So `set_relation_inner` now
    looks the pair up by `(source, target)` alone (`find_active_pair`) and patches `kind` on the row
    it finds, while `create_inner` — the author's `POST` — keeps the conflict. `find_active` (kind-keyed)
    remains for `end_relation`, whose contract genuinely names the wording being ended: after a
    re-word, ending the old wording is `relation_not_found` rather than silently ending a bond the
    caller did not name. That edge case is recoverable — the archivist reads the pair's current
    wording with `get_relations` — and is preferable to letting `kind` drift back into an identity.
  - **`ended` history is untouched.** An `ended` row is not active, so it never occupies the slot: a
    bond that lapses and resumes gets a **new** active row and the old one stays as history, which is
    how the author's already-resumed pairs look (id 1 `ended` + id 61 active, same kind). The guard
    only ever looks at `status = 'active'`.
  - **Track scoping preserved exactly.** The guard's `track_id` match keeps the two canons apart the
    way the indexes do — `Some(T)` matches only track `T`'s rows, `None` only shared rows — so an
    isolated track still records its own private relation over a pair that already has a shared one
    (spec/06 §2), and a shared write never sees a private row. Both directions are tested.
  - **Reverts cannot resurrect a duplicate.** `restore_inner` (revert of an update) and
    `recreate_inner` (revert of a delete) bypassed the guard entirely; both now check the pair when
    the restored snapshot is `active`, excluding the row being written, so reverting a status change
    on legacy data cannot produce a second active row. `update_inner` gained the same check for
    `status → active` (reactivating an `ended` row).
  - **Merges follow the same key.** `entity::move_relations`'s `taken` set dropped `kind` from its
    tuple, so a moved row that lands on a pair A already has is dropped even when the wording
    differs — A's existing row survives with A's wording, since a merge cannot invent a combined one.
- **Existing duplicates are not touched.** The 5 cluttered pairs stay exactly as they are: choosing a
  survivor and ending the loser rewrites the author's canon, which this project does not do silently
  (the same stance as D-142's untrimmed descriptions). The migration refuses to pick a survivor and
  reports the offending ids instead, so the cleanup remains the author's decision.
- **Consequences**: the archivist can no longer inflate a relation list by re-wording, which is the
  reported symptom. A pair that genuinely carries two simultaneous bonds (师兄 *and* 情敌) can no
  longer be expressed as two rows; the more important wording stays in the relation and the other
  belongs in `description` or a memory note — a real expressiveness loss, accepted because two rows
  were indistinguishable from the duplication bug in both the UI and `get_relations`. Existing
  databases are upgraded by `m0003_relation_pair_unique`, which replaces both indexes and **fails with
  the offending pairs listed** if the database already holds a duplicate; the author's database has 0
  such pairs (verified against a copy), so it upgrades cleanly. 0001_init.sql carries the new indexes
  too, so fresh databases and every test fixture are correct without the migration.
- **Regression guards**: `relation_pair_uniqueness.rs` (9 tests) pins the conflict + `details.id`, the
  same-kind case, both directions as separate pairs, `ended` never blocking, the revert-of-delete
  path, the two merge cases (same kind and re-worded), and track scoping both ways
  (a private relation over a shared pair is legal; the shared canon does not see it).
  `relation_pair_archivist.rs` (3 tests) drives a **real maintenance session** whose archivist sets one
  pair twice with different wording and asserts one row survives with the new wording, that both
  `ui.target_id`s name the same row, and that an explicit second `POST` still conflicts.
  `relation_pair_migration.rs` (4 tests) proves the fresh index refuses a re-worded duplicate at the
  SQLite level, that the index is partial on `active`, that the upgrade converts a pre-rule database
  and is repeatable, and that a database which already holds a duplicate is **refused with its ids
  named and its rows untouched**. All three were checked to fail before the fix (the pair-uniqueness
  and archivist suites were run against the old `kind`-keyed guard and index and failed 3/3 and 3/9
  respectively), and the suite's history/direction/track tests pass in both directions, which is what
  proves the guard is not over-broad.
- **Spec**: spec/06 §1.2 now states the pair-level rule, the ordered-pair reading and its reason, the
  "edit, don't re-create" contract, that `ended` never blocks, and what to do with two simultaneous
  bonds; spec/05 §6 `set_relation` documents the upsert-by-pair behaviour; spec/02 §2's SQL mirror and
  spec/10 §4's `conflict.exists` row carry the new key and its meaning.

### D-144 A session event refreshes the chapter that embeds the session

- **Context**: the author reported 「dock 栏在哪」 — "where is the dock?" — with a screenshot of a
  `held` writer session whose agent panel told them to decide in the dock, and no dock on the page.
  The dock was not broken. Loaded fresh, the chapter page rendered it (measured in a real browser:
  `getBoundingClientRect()` = `{top: 411, height: 122, bottom: 533}` in a 577 px viewport, with the
  merged preview). It was **unreachable on the live path**. `ChapterDto.open_sessions` is the
  client's only handle on a chapter's writer session and is cached under
  `keys.chapters.detail(chapterId)`; `useWriterState` subscribes to the session snapshot only when
  that session's `status` is `held` (D-141 needs `pending_content`, which only the snapshot carries).
  An author-driven change was already covered: `invalidateSession` (`api/hooks/sessions.ts`) drops
  the chapter key for pause, resume, stop and abandon, with a comment saying why. An **agent-driven**
  hold — a write tool hitting `auto_pause`, or `pause_after_outline` — has no client mutation, only
  a `session.updated` event, and that handler invalidated `keys.sessions.detail(sessionId)` alone.
  So the cached `open_sessions[0].status` stayed `running`, the snapshot was never fetched,
  `findHeldCall` returned `undefined`, `usePendingDraft().view` was `undefined`, and both the dock
  and the merged preview stayed absent until something else refetched the chapter. A reload "fixed"
  it, which is the signature of a stale cache; `pendingWriteDock.test.tsx` passed because it mounts
  a fixture whose `open_sessions` already says `held`, so the test never walked the live path.
- **Decision**: the `session.updated` / `session.tool_call` handler in `api/events.ts` now also
  invalidates `keys.chapters.detail(chapterId)`, derived from the event payload's `session.chapter_id`.
  The field is real, not invented: `SessionDto` has carried `chapter_id: Option<i32>` all along
  (`api/dto/sessions.rs`, exported to `web/src/api/gen/SessionDto.ts`), the `session.updated` variant
  carries the whole `SessionDto`, and every status transition goes through `session::transition`,
  which returns `SessionUpdated` with that DTO — the hold path included (`runner::finalize_call`
  pushes the held `SessionToolCall` and then the transition's `SessionUpdated` in the same batch).
  Confirmed on the wire against a copy of the author's database: the hold of session 12 published
  `SessionUpdated { ..., chapter_id: Some(3), status: Held, held_tool_call_id: Some(320) }`.
  The invalidation is **scoped to the chapter the payload names**: a session with no chapter
  (conception, research, canon seed) invalidates nothing extra, so a research or conception step
  never refetches the open chapter. The handler no longer returns early when it cannot read a
  session id, so a payload with a chapter but an unreadable id still refreshes the chapter.
- **Consequences**: an agent-driven hold now reaches the dock live, which is what makes D-141
  reachable at all on the user's own path. The cost is one extra chapter refetch per session event
  for chapter-owned sessions — a `session.updated` is published per step, so a long draft refetches
  the open chapter once per step while the author has that chapter's page open. That is the same
  request the author-driven path already made, it is bounded to a single chapter key, and a step
  is a model round trip, so it does not compete with typing. `session.tool_call` alone carries only
  `{session_id, tool_call}` and `ToolCallDto` has no `chapter_id`, so its new branch is a no-op; the
  server publishes a `session.updated` beside every tool call, and that event carries the chapter —
  the branch exists so the two event types keep one handler rather than silently diverging later.
- **Sibling audit** (`open_sessions` consumers, all invalidated by this same key): `useWriterState`
  (lock, held call, dock, merged preview, live preview) — fixed by this entry, and the only one that
  was visibly broken; `usePendingDraft` and `ChapterPage`'s dock — same chain, fixed with it;
  `ChapterText` and `OutlineTab` (inline `HoldBar`, outline approval) — they read the same hook, so
  an agent-driven `submit_outline` hold (the `pause_after_outline` case) was **also** invisible until
  reload, and is fixed by the same line; `AgentPanel.chapterOpenSessions` had a milder variant: the
  pinned session comes from `sessions.list` and the snapshot, so the panel showed the hold correctly,
  which is exactly why the author saw "decide in the dock" next to no dock; `ChapterHeader`'s
  "end the open writer session" warning is weaker as a result of this fix, because a held session is
  no longer cached as `running` there — its `status !== "running"` filter now sees the hold, which is
  the correct reading of spec/08 §1.1. `SideStoryPage` and `Navigator` read no `open_sessions`.
  No consumer keeps its own `running`-only subscription.
- **Regression guards**: `api/events.test.ts` pins that a `session.updated` with a chapter id
  invalidates that chapter's detail key and that one without a chapter id does not; both were
  checked to fail before the fix (`expected false to be true`). `sessionHoldDock.test.tsx` mounts the
  real `ChapterPage` over a query client with the chapter endpoint answering `running`, then
  `held` after the event, and asserts the dock appears **and** the merged preview renders — with the
  invalidation removed it fails with `Unable to find an element by: [data-testid="pending-write-dock"]`,
  which is the author's symptom in one line. A real browser run (copy of the author's database, own
  port, SPA built both ways) reproduced their sequence end to end: page open while the writer was
  running → agent hold → with the fix the dock and the merged preview appeared **without a reload**
  (one `navigate` entry, `performance.now()` past 55 s); with the old handler the same sequence left
  the pane empty under "The writer is working" while the panel said "Decide on the agent's text in
  the draft pane", and only a reload produced the dock.
- **Spec**: spec/10 §3's reaction for `session.updated` now names the chapter's `open_sessions`
  invalidation and its scope; spec/11 §4.2 records the same rule beside the other event-driven
  invalidations.

### D-145 A book keeps to one or two knowledge scopes

- **Context**: the author asked 「改提示词，一本书一般情况下用一到两个知识库 scope」. The pack's only
  guidance was 「同类主题使用同一个范围名」 in `conception.system` and `set_research_topics.scope` — a
  rule about *when to reuse a name*, never *how many names may exist*. A model classifying tidily
  creates one scope per domain (宋代官制、市井生活、军事、服饰、饮食…). Each scope is a separate
  vector partition: a `search_knowledge` call without a scope runs one KNN query per visible scope
  and merges the results (spec/07 §3), so many small scopes fragment retrieval and leave the author
  choosing among near-duplicate names. The author's database is empty — 2 books, 1 chapter, 0
  entities, 0 relations, and both `knowledge_scopes` and `research_tasks` have 0 rows — so this is
  a prophylactic prompt change, not a data repair.
- **Decision**: a book normally uses **one or two** knowledge scopes in total.
  - Prefer one broad scope named for the book's subject or era (for example 「宋代社会」); add a
    second only when the material genuinely splits into two unrelated domains that a writer would
    search separately (for example a historical setting plus a specialist technical field).
  - Never one scope per subtopic, chapter, or character; never split by document type.
  - A topic that does not fit goes into the existing broad scope; a genuinely unrelated domain is the
    case for the second (and last) scope, not a third.
  - **Where the budget lives**: `conception.system`'s research-topics item states the countable rule
    (「一般情况下，一本书总共只使用一到两个范围」, one broad scope preferred, the second only for
    unrelated domains, no third); `set_research_topics.scope` repeats it as the field contract; the
    tool's top-level description repeats the short form; its existing single-scope example is kept
    and a second example shows three topics sharing one scope name.
  - **No scope-creating tool exists**: there is no `create_knowledge_scope` or
    `update_knowledge_scope` in the tool pack or the registry (48 tools, none scope-related). Scopes
    are created implicitly by the `scope` names of `set_research_topics` when conception is finalized
    (spec/08 §5 step 4) and by a research task's target scope name on run, or by the author through
    the knowledge API/UI. The model-facing path that creates them is therefore `set_research_topics`
    alone, and the budget is stated there. No new variable was added; the rule needs none.
- **Consequences**: future conception runs produce one larger coherent corpus instead of a
  taxonomy, so a `search_knowledge` call without a scope stays useful and the author has at most two
  near-duplicate names to choose from; the per-scope KNN cost in spec/07 §3 also stays bounded. This
  is guidance, not enforcement: `set_research_topics` still accepts any number of distinct scope
  names, and the author can still create scopes by hand. Existing books are not migrated — there is
  nothing to migrate in the author's current database (0 scopes), and a migration could not know
  which of many small scopes the author would want merged.
- **Regression guards**: `prompts_registry.rs::the_knowledge_scope_budget_is_stated_where_research_topics_are_set`
  asserts the budget in the rendered `conception.system` prompt (1–2 total, no per-subtopic/chapter/
  character split, no third) and in the `set_research_topics` scope field description (1–2 total, no
  per-subtopic split). The rendered-pack snapshot changed in exactly the two places the prompt edits
  touched (`conception.system`, `tool.set_research_topics`), and it failed first before being
  updated — proof the text actually landed. `scripts/validate_prompts.py` stays green; the added
  tool example validates against the schema.
- **Spec**: spec/07 §1 now states the one-or-two-scope budget as prompt guidance, with the cost of
  extra scopes (one KNN query per visible scope, one more name in the author's list).

### D-146 Split chapter writing into an outline session and a drafting-only write session

- **Context**: the requirement (R1–R13 of `docs/research/session-split-requirements.md`) asks for
  "write the outline" and "write the chapter" as two visible steps. Today one `chapter_write`
  session runs the writer's `planning` phase (retrieval, then `submit_outline`) and then drafts in
  the same session and with the same frozen tool list and system prompt; redraft re-plans and
  rewrites in one go. Tool lists are per **role** (D-007), so a session that must not contain
  `submit_outline` cannot simply share the writer's list. `chapters.outline` is also written by
  `submit_outline`'s `execute` today — before the hold decision — so a held or rejected outline is
  already stored in the chapter.
- **Decision**:
  - A new session kind **`chapter_outline`** runs under a new sixth role **`planner`**, with its own
    system prompt, its own fixed tool list (the ten retrieval tools, `read_draft`, `submit_outline`)
    and its own model assignment. `chapter_write` keeps its name and becomes drafting-only: its
    frozen list is the writer list minus `submit_outline` (19 tools), and its initial phase is
    `drafting`.
  - **Staging**: the outline session stages its outline. `submit_outline` in a `chapter_outline`
    session writes nothing to the chapter; it returns `ToolEffects { complete: outline,
    pending_outline }`, and `apply_pending_outline` writes `chapters.outline`,
    `reference_notes` and (only while the title is still empty) the proposed title in the
    transaction that finalizes an accepted call — the non-held path or the approval that resumes a
    held one. New invariant: `chapters.outline` holds only an accepted or author-written outline.
    Feedback revokes the effects and the session keeps working; abandon discards the staged outline;
    edits made while the session is held are reported to the planner.
  - `submit_outline` is the outline session's terminal tool with the new `outline` completion kind.
    With `pause_after_outline` on it holds and the completion applies on approval; with it off the
    outline is accepted immediately and the session completes. There is no `finish_outline` tool and
    no auto-start of the write session after approval.
  - The write session starts in `drafting`, needs a non-empty `chapters.outline`
    (`409 chapter.outline_missing` otherwise), and its kickoff carries `block.chapter_draft` (the
    outline, reference notes and text) plus `writer.kickoff.chapter`, which now carries the drafting
    procedure including the granularity rule. `writer.kickoff.redraft` is retired; redraft is the
    feedback branch of `writer.kickoff.chapter`, fed by `kickoff_extra.feedback_id` because the
    `chapter_feedback` row is linked to the session only after creation. The rollover kickoff's
    `feedback` follows the session chain instead of always reading the chapter's latest row.
  - **No new chapter status**. An outline session never changes the status: a planned chapter stays
    `planned` through outlining and acceptance, and `planned → drafting` still happens only on
    write-session creation or an author text edit. The outline's state is derived: missing (empty
    outline), in progress (an open outline session), awaiting approval (that session holds
    `submit_outline`), accepted (non-empty outline and no open outline session).
  - **Concurrency**: the planner and writer roles form one lock group per chapter — at most one open
    session across both roles — so outline, write and revise sessions exclude each other on one
    chapter while chapters never block each other. Approve, manual submit, version restore and the
    edit lock all use the group.
  - **Model routing**: `RoleModels.planner` is optional with `#[serde(default)]`. The planner
    resolves `book.model_overrides.planner` → `roles.defaults.planner` →
    `book.model_overrides.writer` → `roles.defaults.writer`; when nothing resolves, the error is
    `409 model.missing_role` with `details.role = "planner"`. No stored setting is rewritten, so an
    unset planner reproduces today's behaviour at upgrade and for every later book.
  - **`pause_after_outline` is kept** (same `BookSettings` field, same column, same default `true`,
    same `hold_condition`); its meaning narrows to "hold the outline session's `submit_outline`".
    Book settings, `book_defaults` and existing session rows keep their values; the label changes,
    the Write dialog loses the switch and the new Outline dialog gains it.
  - **Next action**: a new `write_outline` for a planned chapter without an outline (and for a
    chapter with an open outline session), and `write_chapter` for a planned chapter that has one.
    An open outline session counts as work in progress (rule 5). A planned chapter is offered
    before `write_next` creates a new one (rule 7, after `sync_canon`).
  - **Prompt pack**: new `planner.system`, `planner.kickoff`, `planner.notice.outline_rejected`,
    `planner.notice.nudge` and `planner.notice.retrieval_budget`; `writer.system` loses the planning
    phase; `writer.kickoff.chapter` becomes the drafting kickoff; `writer.kickoff.redraft` is
    retired; the three writer notices that only legacy sessions use are marked legacy; the
    `submit_outline` tool description and `result.writer_control` gain the staged behaviour.
  - **Open legacy planning sessions** (`chapter_write` in `planning` at the upgrade) finish under
    their frozen prompt through a marked branch: the planning permission arm, an immediate write
    plus `phase_to: drafting` in `submit_outline`, the writer acceptance and rejection notices, the
    planning nudge key, the Demo provider's planning arm and the legacy Outline-tab path. Their
    rollover children inherit the frozen prompt. They are not migrated; a follow-up task deletes
    the branch when the removal query in the plan returns 0.
  - **Migration `0004_outline_sessions`** rebuilds `agent_sessions` (its CHECK constraints list the
    roles and kinds) with foreign keys off, row-count guards on all ten referencing tables and
    `PRAGMA foreign_key_check`; it also moves `prompt_overrides` rows of `writer.kickoff.chapter`
    to `planner.kickoff` where no planner override exists at the same scope. `0001_init.sql` is
    edited for fresh databases.
  - **Behaviour choices taken from the plan's §9** (the first option of each): role labels
    "Planner" / 「策划」 and session title 「细纲」; R6's first clause read as cross-chapter; redraft
    redrafts prose from the current outline while re-planning stays a separate step; an unset
    planner follows the writer's model; re-outlining is allowed on any non-official chapter
    (including in review); the navigator's `has_outline` icon is included; the planner may issue
    parallel tool calls.
- **Consequences**: the author approves two visible steps instead of one and the write session can
  no longer re-plan mid-draft — stop it, redo the outline, continue — which is the point of the
  split. Each step has its own history, usage and prompt cache prefix (planner and writer no longer
  share a cached tools+system prefix; the write and revise sessions of a chapter still do).
  Historical sessions render unchanged; sessions open in `planning` at the upgrade keep working
  through the marked branch. Overrides of `writer.kickoff.chapter` move to `planner.kickoff`
  because their text is planning instruction; overrides of the retired `writer.kickoff.redraft`
  become inert; overrides of `writer.system`, `tool.submit_outline` and the legacy notices are
  kept.
  - **Author-database dry run (task 12.7)**: on a copy of the author's database
    (`~/.local/share/nekowriter/nekowriter.db`: 2 books, 1 chapter, 6 sessions, 30 messages, 19 tool
    calls, 1 inbox row, 2 versions, and 0 proposals, change-log rows, research tasks, reports or
    feedback), `0004` ran after a copy that recorded `m0001`–`m0003`. Every table count was
    unchanged before and after (the ten session-referencing tables included), `PRAGMA
    foreign_key_check` returned no row, and `m0004_outline_sessions` was recorded once. The plan's
    removal query for open legacy planning sessions returned **0** before and after, so no session
    needs the legacy branch. `prompt_overrides` had **0** rows: none of the plan §2.10 keys has an
    override to move (`writer.kickoff.chapter`), leave inert (`writer.kickoff.redraft`) or keep
    (`writer.system`, `tool.submit_outline`, `writer.notice.outline_accepted`,
    `writer.notice.outline_rejected`, `writer.notice.nudge_planning`).
  - **Demo-provider smoke (items 3, 5, 8)**: with `--enable-demo-provider` on a fresh data dir, the
    new outline step held, accepted a feedback round, approved with the chapter still `planned`,
    and the write step then paused mid-draft, took a paragraph edit and feedback, and finished to
    `in_review`; chapter 1 approved and its canon synced; chapter 2 outlined and written at sentence
    granularity with auto-pause for three steps, then switched to free and finished; book overrides
    of `planner.system` and `writer.system` were frozen into the next outline and write sessions
    while a session created before the override kept the shipped prompt. The smoke checklist items
    3, 5 and 8 were rewritten for the two steps.
- **Spec**: PROJECT §4–§6, §10.7; spec/02 §1–§5; spec/03 §1, §4–§12, §14–§15; spec/04 §3, §7.2;
  spec/05 §1.5, §2, §4; spec/06 §4.2; spec/08 §1–§3, §7; spec/09 §1–§2; spec/10 §2.2, §2.7–§2.8,
  §4; spec/11 §5–§6.

### D-147 Enter at the end creates a client-side pending paragraph
- **Context**: Persisting an empty paragraph to make Enter-at-end work left a zero-height,
  unfocusable paragraph and leaked empty paragraphs into full-text round trips and the submit gate
  (R10-01–R10-04). The empty paragraph belongs to the editor, not the data model.
- **Decision**: an exactly-empty insert is rejected again. Enter at the end opens a focused
  client-side paragraph editor below the current paragraph; it persists as one normal `insert`
  with the typed text on blur, and sends nothing if dismissed empty. A draft containing line breaks
  is updated first and the pending paragraph is anchored after the last split paragraph.
- **Consequences**: submitted/approved chapters and `finish_draft`/`finish_revision` require a
  non-zero word count; full-text blank lines remain separators, never paragraph rows. The legacy
  `f94c2f9` empty-insert allowance is deliberately reverted.
### D-148 Writing skills and recent-chapter context

- **Context**: the author asked for a writing-skills surface injected like the knowledge base
  (「弄一个像 skills 那样注入上下文的 writing skills，与书库 知识库 提示词 设置并列」) and for the
  last n chapters' text in context (「将最近 n 章正文注入上下文」). The locked design is
  `docs/research/writing-skills-design.md` rev. 4, re-cut against the M12 outline/write split per
  `session-split-plan.md` §2.14. M12 changed the ground the design assumed in five places:
  `chapter_outline` is a new session kind with `block.settings · block.memory · planner.kickoff`;
  `chapter_write` is now drafting-only and always carries `block.chapter_draft`;
  `writer.kickoff.chapter` is the drafting kickoff and `writer.kickoff.redraft` is retired;
  `writer.system` is worded with 细纲 and no longer describes planning; and the rollover feedback
  truthiness defect (design §9.2) is already fixed in M12 (`kickoff_feedback` returns
  `Option<FeedbackContext>` and walks the session chain), so this change does not touch it.
- **Decision**:
  - **Storage B**: `writing_skills` holds the bodies and `book_writing_skills` the per-book
    selection (migration `0005`). The migration seeds one built-in, `basic-writing`, with
    **empty body**, `author = 'builtin'`, `enabled_by_default = 0`. Built-in rows are read-only
    through the API (`409 writing_skill.builtin`); the author creates user rows (`author = 'user'`,
    `key = NULL`). Deleting a user row cascades its selections away and returns the books that
    used it. No reconciler ships (design §4.2): when the first built-in body is actually shipped,
    that change adds **both** `prompts/skills.toml` and the reconciler together; until then a
    pack file would be unread dead weight, so the validator does not learn one.
  - **Injection is full**, not on-demand, into `block.writing_skills`; bodies are wrapped in
    `<writing_skill name=… category=…>` tags and the block says the text is instruction about how
    to write, **not a document about the book**. It sits right after `block.settings` for
    stable-first ordering; it earns **no cache breakpoint** because `block_is_cached` matches only
    `block.settings`.
  - **Session kinds**: `chapter_write` gets both skills and recent chapters; `chapter_outline`
    gets recent chapters but **no skills** (it writes an outline, not prose); `chapter_revise` and
    `chapter_review` get skills only; rollover gets skills only (its chapter text and
    `previous_tail` already cover the seam); archivist, conception and research get neither.
  - **`block.recent_chapters`** injects the last `recent_chapter_count` (default 2, `0` disables)
    **official** chapters before the target, oldest first, wrapped in `<chapter_text …>` tags. It
    follows memory's rule exactly: same-track official chapters, a side track topped up from
    main-track official chapters up to its anchor, no non-official text (drafts, in-review and
    rejected text are not approved prose), target excluded, empty text skipped. The per-chapter
    cap is `target_words_max × 1.5` (6,000 characters at the 4,000 default; 6,000 when the target
    is unset) and the total budget is `recent_chapter_count × cap`, filled newest-first; a chapter
    over its cap is front-cut at a paragraph boundary keeping the ending and is announced as
    `partial`. `previous_tail` is dropped when its chapter is injected in full; chapter summaries
    stay because they are condensed accounts, not copies.
  - **The prose style section leaves `writer.system` now** (design §5.1 option A): the six style
    bullets and the short-paragraph clause are deleted; 先想清楚再写, 遵循细纲…, the Chinese
    quotation marks/no-Markdown part and the no-metadata bullet stay. **Accepted consequence: a
    book has no prose style rules at all until the author writes and selects a skill.** That is
    deliberate, not a bug.
  - **API** (spec/10 §2.9): `GET /writing-skills?book_id=`, `POST /writing-skills`,
    `PATCH`/`DELETE /writing-skills/:id` (user rows only; delete returns `used_by_books`), and
    `PUT /books/:id/writing-skills` for the selection.
- **Consequences**: the built-in `basic-writing` row is selectable but injects nothing while its
  body is empty. The recent-chapters injection adds roughly two chapters of prose to every write
  and outline kickoff; the per-chapter cap and `recent_chapter_count = 0` are the controls. The
  skills block is sent in full on every write/revise/review session, so the selection UI shows a
  running character total and warns above 8,000 characters without blocking. Existing selections
  keep working when a built-in is later retired. The Memory page names the chapters injected in
  full (`MemoryViewDto.recent_chapters`) so PROJECT §6.5 still shows what the writer receives even
  though the full text travels in its own kickoff block. Two items of debt are explicit: the
  reconciler (with `prompts/skills.toml`) and built-in bodies are deferred together until a body
  exists.
- **Spec**: design §4–§8; spec/02 §1–§5; spec/03 §10; spec/06 §4.1; spec/09 §2, §5.8;
  spec/10 §2.9, §4; spec/11 §2–§3, §5.5, §6.

### D-149 Review round 1 fixes for writing skills and recent chapters

- **Context**: review of `25f32db` found that the recent-chapter block did not treat
  `recent_chapter_count` as a chapter limit, that a write-session rollover dropped `previous_tail`
  although it emits no recent-chapter block, and several smaller contract/test defects. Two
  suspicions needed a documented answer: a single-paragraph over-cap chapter had no cut rule
  beyond a raw character slice, and a retired but selected skill was dropped by the next
  selection toggle.
- **Decision**:
  - A single paragraph over the per-chapter cap is cut at a **sentence boundary** when one leaves
    a fitting non-empty suffix (the earliest such boundary, so as much ending as possible is
    kept); only when even the last sentence is longer than the limit does it keep the last
    `limit` characters. The paragraph-boundary rule still applies to multi-paragraph chapters.
  - `PUT /books/:id/writing-skills` preserves the book's **already-selected retired skills**
    when the replacement payload cannot name them: the active catalogue hides retired rows, so a
    toggle built from the visible list must not silently drop a selection that design §4/§12 says
    keeps working. An explicitly supplied retired id is still a `validation.failed`, because the
    request body contract (spec/10 §2.9) forbids selecting a retired skill.
  - The Memory page applies the same `previous_tail` dedupe as the writer's `block.memory`, so the
    endpoint's `rendered` text is the exact text the writer receives (PROJECT §6.5, spec/06 §4.5).
  - A kickoff's memory dedupe is derived from the **actual emitted block list**, not from
    `KickoffPlan::blocks()`: rollover builds its own list without `block.recent_chapters`.
- **Consequences**: with one over-cap paragraph the model still gets the chapter's ending, now
  starting at a sentence start whenever the text allows; a retired built-in selection survives
  unrelated toggles until the row is deleted (built-ins are never deleted) or the book is deleted.
- **Spec**: design §6.4, §7; spec/03 §9–§10; spec/06 §4.5; spec/10 §2.9.
