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

## Open items (questions for the author)
1. Confirm the reading of "this time it cannot be delegated to a lower-level review agent" as
   "the author judges the reviewer's report; no further delegation" (PROJECT §10.9).
2. Confirm the defaults: pause after outline **on**; auto-pause **off**; granularity
   **paragraph**; auto-maintenance **on**; isolated mode for new side stories.
