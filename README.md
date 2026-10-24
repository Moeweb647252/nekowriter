# NekoWriter

A local-first studio for writing long-form fiction with AI agents. The author directs and
approves; agents plan, research, draft, review, and keep an explicit canon (tiered memory,
entity graph, timeline, setting materials) so long stories stay consistent.

**Status:** the foundations are implemented (workspace, configuration, database and full schema,
ts-rs bindings, web shell, `scripts/check.sh`). Feature work proceeds through the milestones in
[`docs/PLAN.md`](docs/PLAN.md).

## Highlights
- **Granular drafting**: the writer adds text one sentence, paragraph, free-sized chunk, or
  chapter per round. You trade cost for quality and can switch mid-chapter.
- **Real-time participation**: watch insertions stream in, pause at any step, give feedback,
  edit the text yourself while the agent waits, or enable auto-pause. Nothing is lost when you
  pause.
- **Look before writing, record after**: retrieval before each chapter, maintenance of canon
  after each approved chapter.
- **Review mode**: every chapter needs your approval; a review agent can advise you.
- **Scoped knowledge base** filled by a research agent, with hybrid search (FTS5 + sqlite-vec)
  and a local embedding model.
- **Every prompt is customizable**, globally or per book. The shipped prompts are Chinese, and
  each book sets its writing language.
- **Side stories** as separate tracks, canon or isolated.

## Documentation
- [`docs/PROJECT.md`](docs/PROJECT.md): product description, user flows, UI.
- [`docs/spec/`](docs/spec): technical specification (architecture, data model, agent runtime,
  LLM layer, tools, canon and memory, knowledge and search, workflows, prompts, API, frontend).
- [`docs/PLAN.md`](docs/PLAN.md): implementation plan.
- [`docs/DECISIONS.md`](docs/DECISIONS.md): design decisions and their reasons.
- [`AGENTS.md`](AGENTS.md): guide for the implementing agent.
- [`prompts/`](prompts): the shipped prompt pack.

## Development

Requirements: a Rust toolchain (edition 2024, see `rust-version` in `Cargo.toml`), Node.js with
`pnpm`, and `python3` (for the prompt validator). `npm`/`npx` are not used.

```bash
pnpm --dir web install           # once
cp .env.example .env             # optional: NW_DATA_DIR, RUST_LOG
cargo run -p nekowriter          # API on http://127.0.0.1:7878
pnpm --dir web dev               # UI on http://localhost:5173 (proxies /api)
scripts/check.sh                 # every check, in the spec/01 §12 order
```

The server binds `127.0.0.1:7878` by default and refuses a non-loopback address unless
`--allow-remote` is passed. Its data directory (`NW_DATA_DIR`, default `./.data` via
`.env.example`, otherwise the OS data dir) holds `nekowriter.db`, `models/` and `logs/`. Debug
builds enable the Demo provider, so `mock://demo` works without API keys.

Regenerate the TypeScript API types after changing a DTO:

```bash
cargo test -p nekowriter export_bindings
```

Project-specific commands:

| Purpose | Command |
|---|---|
| Go backend | `cargo run -p nekowriter` (`--bind`, `--data-dir`, `--web-dir`, `--allow-remote`) |
| Go frontend | `pnpm --dir web dev` |
| Generate TS types | `cargo test -p nekowriter export_bindings` |
| Validate the prompt pack | `python3 -I scripts/validate_prompts.py .` |
| All checks | `scripts/check.sh` |
