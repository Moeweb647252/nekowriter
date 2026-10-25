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
