# 01 — Architecture

## 1. System context

```
┌──────────────────────────── Browser (single user) ─────────────────────────────┐
│ React SPA (Vite build) — REST (fetch) + one SSE stream (/api/events)            │
└───────────────────────────────▲──────────────────────────────┬────────────────┘
                                │ http://127.0.0.1:7878         │
┌───────────────────────────────┴──────────────────────────────▼────────────────┐
│ nekowriter (single Rust binary, tokio + axum)                                  │
│  api ── services ── agent runtime ── tools ── context builder ── prompts       │
│   │         │              │                                                   │
│   │         │              ├── nekowriter-llm ──► LLM providers (HTTPS)         │
│   │         │              │     (OpenAI-compatible · Anthropic · mock)        │
│   │         │              └── web tools ──────► search API · web pages         │
│   │         └── search index (FTS5 + jieba, sqlite-vec) ◄── indexer worker      │
│   │                                          ▲                                 │
│   │                                          └── nekowriter-embed (fastembed,   │
│   │                                               local ONNX model)            │
│   └── SeaORM 2 / sqlx ──► SQLite file (WAL) incl. vec0 + FTS5 virtual tables    │
└────────────────────────────────────────────────────────────────────────────────┘
```

Everything runs in one process. There is no external database, queue, or vector service.

## 2. Technology stack (pinned)

Use these major/minor versions; patch upgrades are fine. Verified against crates.io and npm on
2026-10-24. If a version below does not resolve, use the newest release of the same major and
note it in `docs/DECISIONS.md`.

### 2.1 Backend (Rust, edition 2024, stable toolchain ≥ 1.94)

| Crate | Version | Features / notes |
|---|---|---|
| tokio | 1.54 | `full` |
| axum | 0.8.9 | defaults + `macros` |
| tower | 0.5 | |
| tower-http | 0.7 | `trace`, `fs`, `set-header` |
| sea-orm | 2.0.4 | `default-features = false`, features `macros`, `with-json`, `with-chrono`, `sqlx-sqlite`, `runtime-tokio`, `sqlite-use-returning-for-3_35` |
| sea-orm-migration | 2.0.4 | `default-features = false` (the default `cli` feature is not needed), features `sqlx-sqlite`, `runtime-tokio` |
| libsqlite3-sys | same version that `sqlx-sqlite` resolves (currently 0.37.x; check `cargo tree -i libsqlite3-sys`) | no features; only used for `sqlite3_auto_extension` |
| sqlite-vec | 0.1.9 | statically linked extension |
| fastembed | 7.5 | `default-features = false`, features `onnx`, `ort-download-binaries-rustls-tls`, `hf-hub-rustls-tls` |
| reqwest | 0.13 | defaults (rustls) + `json`, `stream` |
| eventsource-stream | 0.2 | parse provider SSE streams |
| serde / serde_json | 1 | |
| toml | 1.1 | parse the prompt pack |
| minijinja | 3.0 | `serde`, `json`, `loop_controls` |
| ts-rs | 12 | `chrono-impl`, `serde-json-impl` |
| chrono | 0.4 | `serde` |
| thiserror | 2 | library errors |
| anyhow | 1 | binaries and tests only |
| tracing / tracing-subscriber | 0.1 / 0.3 | `env-filter`, `fmt`, `json` |
| futures / tokio-util / tokio-stream | 0.3 / 0.7 / 0.1 | `CancellationToken` (tokio-util, no features); `BroadcastStream` (tokio-stream feature `sync`) |
| clap | 4 | `derive`, `env` |
| directories | 6 | data dir |
| dotenvy | 0.15 | dev `.env` |
| rust-embed | 8 | prompts (always) and web dist (feature `embed-web`) |
| jieba-rs | 0.11 | default dict |
| unicode-segmentation | 1.13 | UAX #29 word count |
| similar | 3 | paragraph alignment, text diffs |
| jsonschema | 0.60 | `default-features = false` (no remote `$ref` resolution, so no second HTTP/TLS stack); validate tool inputs against `prompts/tools.toml` schemas |
| async-trait | 0.1 | `ToolHandler` trait objects (or hand-written boxed futures) |
| blake3 | 1.8 | content hashes |
| dom_smoothie | 0.18 | Readability-style main content extraction |
| htmd | 0.5 | HTML → Markdown |
| encoding_rs | 0.8 | non-UTF-8 pages (GBK, Big5, Shift_JIS…) |
| url / ipnet | 2 / 2 | URL handling, SSRF guard |
| Dev: tempfile, insta, pretty_assertions, wiremock | 3 / 1.49 / 1.4 / 0.6 | tests |

### 2.2 Frontend (Node ≥ 22.22, pnpm; npm/npx are not installed here, so use `pnpm dlx`)

Scaffolded by `shadcn@4.22` (see `spec/11-frontend.md` §1). The scaffold pins React 19.3,
Vite 8, TypeScript 6.0, Tailwind CSS 4, ESLint 10, Prettier 3, `radix-ui` 1.8, `lucide-react`.

| Package | Version | Use |
|---|---|---|
| motion | 14.6 | animation (`import { motion } from "motion/react"`) |
| react-resizable-panels | 4.14 | via the shadcn `resizable` component (v4 API: `Group`/`Panel`/`Separator`) |
| @dnd-kit/core, @dnd-kit/sortable, @dnd-kit/utilities | 6.3 / 10.0 / 3.2 | sortable lists |
| zustand | 5.0 | client and live-stream state |
| @tanstack/react-query | 5.104 | server state cache (REST resources) |
| react-router | 8.5 | routing (data mode; `react-router-dom` no longer exists) |
| i18next / react-i18next | 26 / 17 | i18n |
| react-markdown / remark-gfm | 10 / 4 | Markdown rendering |
| diff | 9 | version diffs in the UI |
| @tanstack/react-virtual | 3.14 | long lists (activity stream, chapter list, entities) |
| sonner | 2 | toasts (via shadcn) |
| vitest / @testing-library/react / jsdom | 5 / 16 / 30 | unit tests |

Zustand is the state-management library. TanStack Query caches REST data and is not a
second state store. Rule: REST resources → Query; UI state and live streaming state → Zustand.

## 3. Repository layout

```
nekowriter/
├── AGENTS.md                     # executor guide (read first)
├── README.md
├── Cargo.toml                    # [workspace] members = ["crates/*"], resolver = "3"
├── Cargo.lock
├── .cargo/config.toml            # [env] TS_RS_EXPORT_DIR = { value = "web/src/api/gen", relative = true }
├── rustfmt.toml                  # edition = "2024", max_width = 100
├── .env.example                  # NW_DATA_DIR=./.data  RUST_LOG=info,nekowriter=debug
├── crates/
│   ├── server/                   # package `nekowriter` (lib + bin `nekowriter`)
│   ├── llm/                      # package `nekowriter-llm`: provider-neutral LLM client
│   ├── embed/                    # package `nekowriter-embed`: fastembed wrapper + fake embedder
│   └── migration/                # package `nekowriter-migration`: sea-orm-migration, SQL files
├── prompts/                      # shipped default prompt pack (Chinese), embedded at compile time
├── web/                          # Vite + React SPA (pnpm)
├── scripts/
│   ├── check.sh                  # all checks, used before every commit
│   └── validate_prompts.py       # prompt pack validator (part of check.sh)
└── docs/
    ├── PROJECT.md · PLAN.md · DECISIONS.md (append-only)
    └── spec/01 … 11
```

There is **no** `xtask` crate. Orchestration is `scripts/check.sh` plus documented commands.

### 3.1 `crates/server` module tree

```
src/
├── main.rs              # CLI parsing, tracing init, calls lib::run
├── lib.rs               # pub mod …; pub async fn run(config) ; pub fn build_app(state) -> Router (tests)
├── config.rs            # Config (clap + env), data dir resolution
├── state.rs             # AppState (Arc) — db, events, prompts, llm registry, embed, indexer, runtime
├── error.rs             # AppError, ErrorCode, IntoResponse
├── db/
│   ├── mod.rs           # connect(): sqlite-vec registration, pragmas, pool, migrations; Db::write (§7)
│   ├── entities/        # SeaORM entities (one file per table)
│   └── json.rs          # typed JSON column wrappers (ChapterContent, BookSettings, …)
├── api/                 # axum routers per resource; thin handlers; DTOs (ts-rs) in api/dto/
│   ├── mod.rs           # router assembly, host/origin guard, error mapping
│   ├── events.rs        # GET /api/events (SSE)
│   └── …                # books.rs chapters.rs canon.rs knowledge.rs prompts.rs sessions.rs settings.rs …
├── services/            # domain logic; own transactions; emit events; enqueue indexing
│   ├── books.rs  tracks.rs  chapters.rs  versions.rs  settings_docs.rs  characters.rs
│   ├── proposals.rs  canon/{entities,relations,appearances,timeline,notes,foreshadowing,changes}.rs
│   ├── memory.rs        # tiered memory snapshot builder
│   ├── reviews.rs  knowledge.rs  research.rs  app_settings.rs  providers.rs
├── text/                # paragraphs (ChapterContent ops), granularity validation, word count,
│                        # language helpers, paragraph alignment, CJK-aware joining
├── prompts/             # registry: load pack, overrides, resolution, rendering, validation
├── agent/
│   ├── runtime.rs       # SessionManager: spawn/drive/cancel runners, recovery
│   ├── runner.rs        # the per-session loop (spec/03 §5)
│   ├── session.rs       # status/phase rules, control operations
│   ├── context/         # kickoff builders per session kind, block renderers
│   ├── request.rs       # neutral history → nekowriter_llm::ChatRequest
│   ├── preview.rs       # streaming partial-JSON string field extractor
│   ├── roles.rs         # role → tool list, permitted tools per phase
│   └── tools/           # tool trait, registry, implementations grouped by area
├── search/              # index schema mgmt, fts (jieba), vec (sqlite-vec), hybrid query, indexer
├── web/                 # web_search providers (tavily, searxng), web_fetch pipeline, SSRF guard
├── testing.rs           # TestApp and fixtures (§11; cfg(test) or feature `testing`)
└── events.rs            # EventBus (broadcast), ServerEvent enum (ts-rs)
```

### 3.2 `crates/llm` (no dependency on the server or the database)

```
crates/llm/
├── src/
│   ├── lib.rs        # pub use types::*; ProviderClient (enum dispatch, spec/04 §1); ProviderConfig
│   ├── types.rs      # ChatRequest, Message, TextBlock, ToolDef, StreamEvent, AssistantTurn, Usage, StopReason
│   ├── openai.rs     # OpenAI-compatible Chat Completions adapter
│   ├── anthropic.rs  # Anthropic Messages adapter
│   ├── mock.rs       # ScriptedProvider (tests) + DemoProvider (dev, feature-gated at runtime)
│   ├── sse.rs        # shared SSE decoding helpers
│   └── retry.rs      # error classification, backoff
├── demo/corpus.toml  # what the DemoProvider writes, zh and en (data, embedded with include_str!)
└── tests/            # stream parsing tests; recorded SSE transcripts in tests/fixtures/*.sse
```

### 3.3 `crates/embed`

```
src/lib.rs   # trait Embedder { fn info(); async fn embed(texts, kind) }, EmbedKind::{Query, Passage}
             # FastEmbedder (fastembed TextEmbedding behind std::sync::Mutex + spawn_blocking)
             # FakeEmbedder (deterministic hash-based vectors for tests)
             # SUPPORTED_MODELS: key, fastembed enum, dimension, query/passage prefixes,
             #   parsed from ../models.toml (data file, embedded with include_str!)
```

### 3.4 `crates/migration`

`sea-orm-migration` crate whose migrations execute embedded SQL files
(`src/sql/NNNN_name.sql`) via `execute_unprepared`. The SQL is the source of truth for the
schema (see `spec/02-data-model.md`). SeaORM entities in `crates/server/src/db/entities/` are
hand-written to match it. Schema sync (`schema-sync` feature) is **not** used.

## 4. Layering rules

1. **Handlers** (`api/`) parse input, call one service function, and map the result to a DTO.
   They contain no SQL and no business rules.
2. **Services** own business rules and transactions. Every write that changes canon or settings
   goes through a service, which records a `change_log` row, emits events after commit, and
   enqueues re-indexing. Each write operation has an outer function (opens `Db::write`,
   commits, then emits events; called by handlers) and an inner function that takes the
   transaction and returns the events to emit.
3. **Agent tools** call the inner service functions with the transaction in their `ToolCtx`
   (spec/05 §1), which also carries the session, the book, and visibility rules, and collects
   the events the runner emits after its commit.
4. **Entities** are only used inside `services/` and `db/`. DTOs (`api/dto/`) are separate structs
   with `#[derive(Serialize, Deserialize, TS)]` and `#[ts(export)]`.
5. The `llm` and `embed` crates know nothing about books or chapters.

## 5. Startup sequence (`lib::run`)

1. Parse config (clap + env + `.env` via dotenvy in debug builds). Init tracing
   (`RUST_LOG`, default `info,nekowriter=debug`).
2. Resolve and create the data directory (`--data-dir` / `NW_DATA_DIR`, default
   `directories::ProjectDirs::from("", "", "NekoWriter").data_dir()`). Create `models/` and
   `logs/` inside it.
3. Register sqlite-vec once per process **before any connection is opened**:
   ```rust
   unsafe {
       libsqlite3_sys::sqlite3_auto_extension(Some(std::mem::transmute(
           sqlite_vec::sqlite3_vec_init as *const (),
       )));
   }
   ```
4. Connect: `sqlite://<data_dir>/nekowriter.db?mode=rwc`. Every pooled connection gets
   `journal_mode=WAL`, `synchronous=NORMAL`, `foreign_keys=ON`, `busy_timeout=5000`,
   `temp_store=MEMORY`. Use SeaORM's sqlx connect-option hooks to set pragmas per connection, or
   build a `sqlx::SqlitePool` with `SqliteConnectOptions::pragma(..)` and wrap it with
   `SqlxSqliteConnector::from_sqlx_sqlite_pool`. Pool size: max 8. Verify with
   `select vec_version()` and log the result.
5. Run migrations (`Migrator::up`).
6. Load the prompt pack (embedded) and validate every default template. A failure aborts startup
   with a clear message, because it is a build defect.
7. Build the `AppState`: EventBus, PromptRegistry, LLM provider registry (lazy, from DB rows),
   embedder handle (lazy model load in the background), search index manager, indexer worker,
   and SessionManager.
8. **Recover sessions**: every session in status `running` → `interrupted`. Every tool call in
   status `executing` → `pending`. This is safe because tool effects are transactional (spec/03 §5.4).
9. Start background workers: the indexer (spec/07 §4) and the embedder warm-up.
10. Serve on `--bind` / `NW_BIND` (default `127.0.0.1:7878`). Refuse to bind to non-loopback
    addresses unless `--allow-remote` is passed, and print a warning when it is.

**Graceful shutdown** (Ctrl+C / SIGTERM): stop accepting requests, cancel every runner with the
reason `shutdown` (runners then transition to `interrupted`, not `stopped`, exactly as startup
recovery would), wait up to 10 s for them to exit, close SSE streams, and close the pool.

## 6. Configuration

| Flag | Env | Default | Meaning |
|---|---|---|---|
| `--bind` | `NW_BIND` | `127.0.0.1:7878` | listen address |
| `--data-dir` | `NW_DATA_DIR` | OS data dir | database, models, logs |
| `--web-dir` | `NW_WEB_DIR` | unset | serve SPA files from a directory (instead of embedded) |
| `--enable-demo-provider` | `NW_ENABLE_DEMO_PROVIDER` | false (true in debug builds) | allow the `mock` protocol for providers |
| `--allow-remote` | — | false | allow non-loopback bind |
| — | `HF_ENDPOINT` | unset | Hugging Face mirror for model downloads (read by fastembed) |
| — | `HF_HOME` | unset | when set, fastembed caches models there instead of `<data_dir>/models` (Settings › Embeddings shows the path) |

All user-facing settings (providers, models, roles, search, embedding) live in the database.

Data directory layout:
```
<data_dir>/nekowriter.db (+ -wal, -shm)
<data_dir>/models/            # fastembed cache
<data_dir>/logs/              # rolling daily log files (json), 7 days
```

## 7. Concurrency model

- One tokio runtime. All HTTP handlers are async.
- **Agent runners**: one tokio task per running session (spec/03). The `SessionManager` holds a
  `Mutex<HashMap<SessionId, RunnerHandle>>` (cancel token and join handle), which ensures at
  most one runner per session.
- **SQLite writes**: SQLite allows one writer at a time, and a deferred transaction that reads
  first and then writes fails at once with `SQLITE_BUSY_SNAPSHOT` (extended result code 517, which
  no `busy_timeout` retries) when another connection committed in between. Therefore every write
  transaction goes through one helper, `Db::write(|tx| …)`, which holds a process-wide
  `tokio::sync::Mutex` for the transaction's duration and opens the transaction with
  `BEGIN IMMEDIATE`, so SQLite's write lock is taken before the closure runs its first statement;
  read-only queries use the pool directly and run concurrently (WAL). A write that bypassed the
  helper would wait for that reserved lock and then be serialized behind it (an ordinary
  `SQLITE_BUSY`, code 5, retried by the 5 s timeout) instead of colliding at commit time. The
  integration-test fixtures that write rows directly use a test-only `Db::write_unprepared`, which
  takes the same lock, so the tests run under the production discipline (D-109). Keep
  transactions short: tool effects plus their result rows. Never hold a transaction across an LLM
  call, a web request, or an embedding; tools do such work in their `prepare` phase
  (spec/05 §1.2), so the write lock is never held while waiting on I/O. The lock is not
  reentrant, so only the outer service functions (§4), the runner, and the indexer open
  `Db::write`; inner functions and tools always use the transaction they are given.
- **Embedder**: `fastembed::TextEmbedding::embed(&mut self, …)` is CPU-bound. Run it via
  `spawn_blocking` behind a `std::sync::Mutex`. The indexer worker is its only bulk user. Query
  embeddings for searches take the same mutex (batches are small, so latency stays low).
- **jieba** segmentation is fast; call it inline. Hold the `Jieba` instance in a `LazyLock`.
- **Event bus**: `tokio::sync::broadcast` with capacity 4096 (spec/10 §3).

## 8. Error model

- `AppError { code: ErrorCode, message: String, details: Option<serde_json::Value> }`, where
  `ErrorCode` is a closed enum serialized as dotted strings (`chapter.locked`,
  `validation.failed`, …; full list in spec/10 §4).
- HTTP mapping: validation → 400, not found → 404, conflict/version/invalid state → 409,
  locked → 423, provider errors → 502, internal → 500.
- The JSON error body is `{ "error": { "code", "message", "details" } }`. The frontend localizes by
  `code` and falls back to `message`.
- No `unwrap()`/`expect()` outside tests and startup invariants. `?` everywhere with context.
- **Tool errors are not HTTP errors**: a failing tool returns a tool result with `is_error=true`
  and a model-readable message (spec/05 §1.5).

## 9. Logging and observability

- `tracing` spans per HTTP request (method, path, status, latency) and per session step
  (session id, kind, phase, step number).
- LLM calls log model, latency, stop reason, and usage at `info`. Request bodies are logged only
  at `trace`, with API keys and Authorization headers never logged.
- Logs go to stderr (pretty in debug, JSON in release) and to `<data_dir>/logs/`.

## 10. Security (single user, local, no auth)

1. **Loopback only** by default (§5 step 10).
2. **DNS-rebinding and CSRF guard**: a middleware rejects requests whose `Host` header is not
   `localhost`, `127.0.0.1` or `[::1]` (any port) with 403. For non-GET methods, an `Origin`
   header, when present, must have one of those hosts. This blocks malicious websites from
   driving the local API.
3. **Secrets**: API keys are stored in SQLite. The `env:NAME` form reads the key from the
   environment at call time, so users can avoid storing it. Keys are never returned by the API in
   clear text (masked `sk-…abcd`) and never logged.
4. **SSRF guard for `web_fetch`** (spec/07 §7): http(s) only; resolve DNS and reject loopback,
   private, link-local, CGNAT, multicast, and unspecified addresses for every redirect hop; at
   most 5 redirects; 20 s timeout; 5 MB body cap. LLM provider calls are not subject to this
   guard (local providers such as Ollama are legitimate).
5. **Prompt-injection hygiene**: web content in tool results is wrapped in a notice saying it is
   untrusted reference material (`notice.web_content`). The researcher's write tools can only
   touch its target scope. No tool can reach outside the current book (or target scope), the
   file system, or the network except through `web_search`/`web_fetch`.
6. Static files are served with `X-Content-Type-Options: nosniff` and a CSP that allows only self.

## 11. Testing strategy

| Layer | What | How |
|---|---|---|
| `text` | paragraph ops, granularity validation, joining, word count, alignment | pure unit tests with table-driven cases (incl. CJK and English) |
| `llm` | stream parsing for OpenAI-compatible and Anthropic, tool-arg accumulation, reasoning capture, usage, stop reasons, error classification | recorded SSE fixtures in `crates/llm/tests/fixtures/*.sse` and `wiremock` |
| `prompts` | every default template renders with each of its sample contexts; overrides resolve correctly | unit tests and `insta` snapshots of rendered kickoffs |
| `search` | jieba FTS, vec insert and query, RRF fusion, visibility filters | temp-file DB + `FakeEmbedder` |
| services | business rules, change log, revert, proposals | temp-file DB integration tests |
| agent runtime | full sessions with `ScriptedProvider`: hold/resume/feedback, steering, edit reports, stop/continue, interrupted recovery, phase gating, granularity switching, nudges, completion transitions | `crates/server/tests/agent_*.rs` |
| API | happy paths and error codes per resource; SSE event emission | `tower::ServiceExt::oneshot` against `build_app` |
| frontend | stores (chapter doc ops, live buffers), utils (word count parity, partial text), key components | vitest + Testing Library |

Test helpers live in `crates/server/src/testing.rs` (behind `#[cfg(any(test, feature = "testing"))]`):
`TestApp::new()` (temp data dir, migrated DB, fake embedder, scripted provider registry) and
fixtures (`book_with_chapters`, `canon_fixture`). The package lists itself as a dev-dependency
with `features = ["testing"]`, so `cargo test --workspace` builds the integration tests in
`crates/server/tests/` with these helpers (a `required-features` entry would skip them silently).

## 12. Build and run

| Task | Command |
|---|---|
| Install web deps | `pnpm --dir web install` |
| Run backend (dev) | `cargo run -p nekowriter` (serves the API; `/` returns a hint page if no web dir) |
| Run frontend (dev) | `pnpm --dir web dev` (Vite on 5173, proxies `/api` to 7878) |
| Generate TS types | `cargo test -p nekowriter export_bindings` (ts-rs writes to `web/src/api/gen`) |
| All checks | `scripts/check.sh` |
| Release build | `pnpm --dir web build && cargo build --release -p nekowriter --features embed-web` |

`scripts/check.sh` runs, in order, and fails fast:
`python3 -I scripts/validate_prompts.py .` → `cargo fmt --all --check` →
`cargo clippy --workspace --all-targets -- -D warnings` →
`cargo test --workspace` → `pnpm --dir web typecheck` → `pnpm --dir web lint` →
`pnpm --dir web test --run` → `pnpm --dir web i18n:check` → `pnpm --dir web build`.

With the `embed-web` feature, the binary embeds `web/dist` via rust-embed and serves it with SPA
fallback (unknown non-`/api` paths → `index.html`). Without it, `--web-dir` serves from disk, and
otherwise `/` returns a plain page saying "run the Vite dev server".
