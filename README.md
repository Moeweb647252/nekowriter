# NekoWriter

A local-first studio for writing long-form fiction with AI agents. The author directs and
approves; agents plan, research, draft, review, and keep an explicit canon (tiered memory,
entity graph, timeline, setting materials) so long stories stay consistent.

**Status:** the MVP is complete (milestones M0–M11 in [`docs/PLAN.md`](docs/PLAN.md)). The
backend and the SPA build into one binary; `scripts/check.sh` is the single gate.

## Highlights
- **Granular drafting**: the writer adds text one sentence, paragraph, free-sized chunk, or
  chapter per round. You trade cost for quality and can switch mid-chapter.
- **Real-time participation**: watch insertions stream in, pause at any step, give feedback,
  edit the text yourself while the agent waits, or enable auto-pause. Nothing is lost when you
  pause, and a held or stopped session survives a server restart.
- **Look before writing, record after**: retrieval before each chapter, canon maintenance after
  each approved chapter, with a change log you can inspect and revert.
- **Review mode**: every chapter needs your approval; a review agent can advise you, and you can
  discuss its report before sending the chapter back.
- **Scoped knowledge base** filled by a research agent, with hybrid search (FTS5 + sqlite-vec)
  and a local embedding model.
- **Every prompt is customizable**, globally or per book. The shipped prompts are Chinese, and
  each book sets its writing language.
- **Side stories** as separate tracks, canon or isolated.
- **Deep links and a quick switch**: every page, scope, chapter and entry has a URL; ⌘/Ctrl+K
  jumps anywhere in the book.

## Install and run

Requirements: a Rust toolchain (edition 2024, see `rust-version` in `Cargo.toml`), Node.js with
`pnpm`, and `python3` (for the prompt validator). `npm`/`npx` are not used.

```bash
pnpm --dir web install           # once
cp .env.example .env             # optional: NW_DATA_DIR, RUST_LOG
cargo run -p nekowriter          # API on http://127.0.0.1:7878
pnpm --dir web dev               # UI on http://localhost:5173 (proxies /api)
scripts/check.sh                 # every check, in the spec/01 §12 order
```

Open the studio at <http://localhost:5173> while developing. The server binds `127.0.0.1:7878`
by default and refuses a non-loopback address unless `--allow-remote` is passed; it enforces a
Host/Origin guard and has no authentication.

### One binary (release)

`cargo run` serves the API only; the Vite dev server serves the SPA in development. A release
build embeds the built SPA and serves everything from one process:

```bash
pnpm --dir web build
cargo build --release -p nekowriter --features embed-web
./target/release/nekowriter --data-dir ~/.local/share/NekoWriter
```

Then open <http://127.0.0.1:7878>. Without the `embed-web` feature, `--web-dir <path>` serves a
build from disk, and `/` otherwise returns a hint page pointing at the Vite dev server. Embedded
assets are only as fresh as the last `pnpm --dir web build`.

## First run and configuration

The data directory (`--data-dir`, `NW_DATA_DIR`, or the OS data directory) holds
`nekowriter.db`, `models/` (the fastembed cache, unless `HF_HOME` is set) and `logs/`. It is safe
to delete for a fresh start; it holds everything the app knows.

1. **Settings › Providers**: add a provider. With a real key: choose the OpenAI-compatible or
   Anthropic protocol, enter the base URL and the API key (stored locally; `env:NAME` references
   an environment variable instead). Debug builds also offer the **Demo** provider
   (`mock://demo`), which drives every workflow deterministically without network or keys.
2. **Settings › Models**: add a model profile (context window and max output tokens included)
   and assign it to the agent roles in **Settings › Agent roles**.
3. **Library › New book**: a title and a writing language are enough. With a conception model
   the studio opens and the agent drafts the setting materials; review them, request changes,
   then **Finalize book** to start the canon.
4. **Settings › Embeddings**: keyword search works immediately; vectors need the local embedding
   model (downloaded on first use, then cached in `models/`). The page shows the model, the
   cache path and the index state.
5. **Settings › Web search**: optional. A Tavily or SearXNG endpoint enables `web_search` for the
   conception and research agents; `web_fetch` needs no configuration, because it fetches the URL
   the agent names (behind the SSRF guard of spec/07 §7).

Environment variables include `NW_BIND`, `NW_DATA_DIR`, `NW_WEB_DIR`, `NW_ENABLE_DEMO_PROVIDER`
(force the Demo provider in a release build), `HF_HOME` and `RUST_LOG`.

## Everyday use

- **Navigator**: the book's overview, tracks and chapters, canon pages, knowledge scopes,
  proposals, prompts and book settings.
- **Agent panel**: every session's activity stream, controls and token usage. Pause, stop,
  continue, give feedback or abandon; held sessions keep their tool call and resume from it.
- **Chapter page**: paragraph editing (Enter starts a paragraph, Backspace removes an empty one,
  both IME-safe), full-text editing, versions and diffs, the review report and send-back.
- **Keyboard**: ⌘/Ctrl+K quick switch, ⌘/Ctrl+Enter saves/sends, Alt+P pauses or continues the
  focused session, Esc cancels. Shortcuts are ignored while an IME composition is active.

## Documentation
- [`docs/PROJECT.md`](docs/PROJECT.md): product description, user flows, UI.
- [`docs/spec/`](docs/spec): technical specification (architecture, data model, agent runtime,
  LLM layer, tools, canon and memory, knowledge and search, workflows, prompts, API, frontend).
- [`docs/PLAN.md`](docs/PLAN.md): implementation plan and the M11 smoke checklist.
- [`docs/DECISIONS.md`](docs/DECISIONS.md): design decisions and their reasons.
- [`AGENTS.md`](AGENTS.md): guide for the implementing agent.
- [`prompts/`](prompts): the shipped prompt pack.

## Development

| Purpose | Command |
|---|---|
| Go backend | `cargo run -p nekowriter` (`--bind`, `--data-dir`, `--web-dir`, `--allow-remote`) |
| Go frontend | `pnpm --dir web dev` |
| Generate TS types | `cargo test -p nekowriter export_bindings` |
| Validate the prompt pack | `python3 -I scripts/validate_prompts.py .` |
| All checks | `scripts/check.sh` |
| Rust tests only | `cargo test --workspace` |

`git commit -- <paths>` per task; temp files belong in `target/tmp` (`TMPDIR` is configured for
the test processes).
