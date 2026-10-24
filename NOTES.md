# Notes for implementing agents

Working notes on the conventions and shared foundation of this repository. `AGENTS.md` and
`docs/` are authoritative; this file only saves you from re-discovering the shared plumbing.

## Commands

| Purpose | Command |
|---|---|
| All checks (must be green before every commit) | `scripts/check.sh` |
| Rust format | `cargo fmt --all` |
| Rust lint | `cargo clippy --workspace --all-targets -- -D warnings` |
| Rust tests | `cargo test --workspace` |
| Generate TS types | `cargo test -p nekowriter export_bindings` |
| Validate prompts | `python3 -I scripts/validate_prompts.py .` |
| Web typecheck / lint / test / build | `pnpm --dir web typecheck` · `lint` · `test --run` · `build` |
| i18n parity | `pnpm --dir web i18n:check` |

`scripts/check.sh` runs all of these in the order fixed by `docs/spec/01-architecture.md` §12.
Never bypass a failing step.

## Shared Rust foundation (already in place)

- `crates/server/src/config.rs` — `Config` (clap + env + `.env` in debug builds).
- `crates/server/src/error.rs` — `AppError`, `ErrorCode::as_str()` dotted codes, `IntoResponse`,
  HTTP status mapping. Add a code to `ErrorCode` only if spec/10 §4 lists it.
- `crates/server/src/state.rs` — `AppState` (`db`, `config`, `vec_version`). Milestones add
  fields here (`Mutex`/`Arc` handles); keep it cheap to clone.
- `crates/server/src/db/mod.rs` — `Db::connect`, `Db::write` (the only write path),
  `Db::conn()` for reads, `Db::vec_version()`.
- `crates/server/src/db/entities/` — hand-written SeaORM entities matching
  `crates/migration/src/sql/0001_init.sql`. Composite-key join tables use
  `#[sea_orm(primary_key, auto_increment = false)]` on each key column.
- `crates/server/src/db/json.rs` — every typed JSON column and every string-backed enum from
  spec/02 §3–§4 (`BookSettings`, `ChapterContent`, `ViewSnapshot`, `StoredBlock`, `Usage`, …).
  Add new JSON shapes here and export them with `#[ts(export)]`, never inline `Value`.
- `crates/server/src/api/mod.rs` — `build_app(state)` (what tests call), the Host/Origin guard,
  `security_headers`.
- `crates/server/src/api/dto/` — DTOs. `ContentOp` / `ContentOpInput` already exist.
- `crates/migration/` — `nekowriter_migration::up(&conn)` applies the schema. `0001_init.sql` may
  still be edited; dev databases are recreated.
- `crates/llm/` — neutral types, OpenAI-compatible adapter, Anthropic adapter, error/retry
  classification, `ScriptedProvider`, `DemoProvider` with `demo/corpus.toml`.
- `crates/embed/` — `Embedder` trait, `FakeEmbedder`, `FastEmbedder`, `models.toml`.

## Shared web foundation (already in place)

- `web/src/i18n/` — one JSON file per namespace per locale, statically imported in
  `i18n/index.ts`. **Adding a namespace means editing `NAMESPACES`, `resources`, and both locale
  directories.** `scripts/check-i18n.ts` fails when key sets differ.
- `web/src/api/client.ts` — `api.get/post/patch/put/delete`, `ApiError {status, code, message,
  details}`.
- `web/src/api/keys.ts` — the only place query keys are built.
- `web/src/api/events.ts` — the SSE client (`subscribe`, `connectEvents`, `disconnectEvents`).
  Add new event types to its `types` array and to the dispatch table.
- `web/src/api/gen/` — ts-rs output. Generated; never edited by hand; committed.
- `web/src/stores/` — `ui` (persisted prefs), `live` (streams/previews/connection),
  `chapterDoc` (open chapter document + `applyOps`).
- `web/src/app/` — `providers.tsx` (Theme → Query → MotionConfig → Tooltip → Toaster),
  `router.tsx` (lazy routes), `shell/` (AppShell, ConnectionDot, LanguageSwitch, ThemeToggle).
- `web/src/components/ui/` — generated shadcn components; compose them, do not fork them.
- `web/src/lib/format.ts` (`formatNumber`, `formatDateTime`, `formatRelativeTime`,
  `formatWords`), `lib/wordcount.ts`, `styles/prose.css`.

## Conventions that bite

- **English everywhere except**: `prompts/` (model-facing), `web/src/i18n/locales/*` (UI),
  test fixtures, `crates/llm/demo/corpus.toml`, `crates/embed/models.toml`.
- **Every write transaction goes through `Db::write`** and must not await I/O inside it.
- **DTOs are separate from entities** and derive `Serialize, Deserialize, TS` with
  `#[ts(export)]`, then `cargo test -p nekowriter export_bindings`.
- **Both locales** get every new UI string in the same commit.
- **Append-only history**: never modify or delete persisted `agent_messages` rows or a sealed
  tool call's `result_text`.
- **No `unwrap`/`expect`** outside tests and startup invariants.
- Rust edition 2024, `rustfmt` `max_width = 100`. Clippy runs with `-D warnings`, so keep it clean
  as you go rather than at the end.
- Vitest config lives in `vite.config.ts` (`test.environment = "jsdom"`, setup
  `src/test/setup.ts`).

## Working with teammates

- One owner per file area. Do not edit files another task's `write_scopes` names.
- Commit only your own task's files (`git add <paths>`, never `git add -A`), with Conventional
  Commits. Another agent may have uncommitted work in the tree.
- Re-read a file before editing it; the filesystem may have changed under you.
- If the spec is ambiguous, choose the simplest option consistent with it and append a
  `docs/DECISIONS.md` entry. Never rewrite existing entries.
