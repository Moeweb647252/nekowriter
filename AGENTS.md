# AGENTS.md — Guide for the executing agent

You are building the **NekoWriter MVP** from the documents in this repository. Nothing has been
implemented yet; the repository contains the design (`docs/`) and the shipped prompt pack
(`prompts/`).

## 1. Read first (in this order)
1. `docs/PROJECT.md`: what the product is, its user flows, UI, scope, and how ambiguous
   requirements were resolved (§10).
2. `docs/spec/01-architecture.md` … `docs/spec/11-frontend.md`: the technical spec. Read at least
   the sections referenced by your current task, plus 02 (data model) and 03 (agent runtime)
   once in full.
3. `docs/PLAN.md`: milestones and tasks. Execute them **in order**.
4. `docs/DECISIONS.md`: design decisions with their reasons. Append to it, never rewrite it.

## 2. Non-negotiable constraints
- **Language**: English only in code, identifiers, comments, commit messages, and docs.
  Model-facing text lives only in `prompts/` (Chinese). UI text lives only in the i18n resource
  files (`en` and `zh-CN`, always updated together). Other non-English text appears only as
  data: test fixtures (string literals in tests, fixture files, snapshots), the Demo provider's
  corpus file, and the embedding model table (`crates/embed/models.toml`); never in identifiers,
  comments, logs, or error messages.
- **Scope**: build what PROJECT §6–§8 describe and nothing more. PROJECT §9.2 lists what not to
  build. If something seems missing, choose the minimal option consistent with the spec and log it
  in `docs/DECISIONS.md`.
- **Stack** is fixed (spec/01 §2): Rust + axum + SeaORM 2 + SQLite (sqlite-vec, FTS5) +
  fastembed; React + shadcn/ui (Nova, Radix) + Lucide + Motion + react-resizable-panels + dnd-kit
  + Zustand. No other vector database. No `xtask` crate.
- **Fresh build**: do not look at, copy from, or port from other git branches.
- **Local single user**: bind to loopback, enforce the Host/Origin guard, no authentication.
- **Append-only agent history** (spec/03 §1.3): never modify persisted messages or sealed tool
  results.
- **Word count**: Unicode UAX #29 (`unicode-segmentation`). CJK ideographs count one each.

## 3. Repository map (target)
```
crates/server      nekowriter (lib + bin): api, services, agent runtime, tools, prompts, search, web
crates/llm         provider-neutral LLM client (OpenAI-compatible, Anthropic, mock)
crates/embed       fastembed wrapper + FakeEmbedder
crates/migration   sea-orm-migration running SQL files (schema = spec/02 §2)
prompts/           shipped prompt pack (Chinese, TOML); embedded at compile time
web/               Vite + React SPA (pnpm)
scripts/           check.sh, validate_prompts.py
docs/              PROJECT.md, PLAN.md, DECISIONS.md, spec/
```

## 4. Commands
| Purpose | Command |
|---|---|
| Install web deps | `pnpm --dir web install` |
| Run the backend | `cargo run -p nekowriter` (http://127.0.0.1:7878) |
| Run the frontend | `pnpm --dir web dev` (http://localhost:5173, proxies `/api`) |
| Generate TS types | `cargo test -p nekowriter export_bindings` |
| Validate prompts | `python3 -I scripts/validate_prompts.py .` (add a key name to print its rendering) |
| All checks | `scripts/check.sh` (must pass before every commit) |

`npm` and `npx` are not installed in this environment. Use `pnpm` and `pnpm dlx`. Use the Demo
provider (`mock://demo`, debug builds) for manual testing without API keys.

## 5. Workflow per task
1. Take the next task in `docs/PLAN.md` and read every spec section it references.
2. Implement it with the tests the task names. Tests need no network: use `wiremock`, the
   `ScriptedProvider`, and the `FakeEmbedder`.
3. Run `scripts/check.sh` and fix everything. Do not silence clippy without a justification
   comment.
4. If the implementation must deviate from the spec, update the spec section in the same commit
   and append a `docs/DECISIONS.md` entry (context, decision, consequences).
5. Commit with Conventional Commits (`feat(server): …`, `feat(web): …`, `fix: …`, `test: …`,
   `docs: …`), one commit per task when practical.

## 6. Conventions

**Rust**
- Edition 2024, rustfmt `max_width = 100`. `thiserror` in libraries; `AppError` at the HTTP
  boundary; `anyhow` only in `main` and tests. No `unwrap`/`expect` outside tests and startup
  invariants.
- Handlers stay thin, services own rules and transactions, DTOs are separate from entities
  (`ts-rs` exported). IDs are `i32`; timestamps `DateTime<Utc>`; enums are string-backed
  `DeriveActiveEnum`s.
- Every write transaction goes through `Db::write` (one process-wide write lock, spec/01 §7).
  Keep transactions short and never hold one across an LLM call, a web request, or an embedding.
- `tracing` for logs. Never log API keys, Authorization headers, or full prompts above `trace`.

**TypeScript / React**
- Strict TypeScript, no `any` (use `unknown` and narrow). API types come only from
  `@/api/gen`.
- Feature folders under `src/features`. Zustand stores in `src/stores` with selectors.
  TanStack Query keys only from `api/keys.ts`.
- Every user-visible string goes through `t()`. Never concatenate translated fragments.
- Generated shadcn components (`components/ui`) are edited only when needed; prefer composition.
  Lucide icons only. Motion for animation, respecting reduced motion.
- Keyboard shortcuts ignore IME composition (`event.isComposing`).

**Prompts**
- Edit only the TOML files in `prompts/`. System prompts stay static (no variables).
- Adding a variable means updating `variables.toml`, the Rust context struct and its sample
  contexts, and `scripts/validate_prompts.py` samples. Then run the validator.

## 7. Definition of done
A task is done when its spec is implemented, its tests exist and pass, `scripts/check.sh` is
green, both locales contain every new UI string, deviations are documented, and the work is
committed.

## 8. When in doubt
- The spec is authoritative. PROJECT §10 records how the original requirements were interpreted.
- If two spec documents conflict, 02 (data model) and 03 (agent runtime) win. Fix the other
  document and log the conflict.
- Ask the author only about decisions that change product behavior. Otherwise choose the simplest
  option consistent with the spec, implement it, and log it.
