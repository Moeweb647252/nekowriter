# NekoWriter

NekoWriter is a local AI workbench for writing novels. You give it a brief. Agents then prepare the background, research, characters and a chapter plan, and write the novel chapter by chapter. You review every step: you approve or reject each submission, give feedback while the agents write, answer their questions, and can review each edit before it is applied. The novel, your knowledge base and your settings stay in a SQLite database on your machine. The model service is any OpenAI-compatible Chat Completions API you configure.

The behavior is specified in [`docs/product-spec.md`](docs/product-spec.md); the design is explained in [`docs/architecture.md`](docs/architecture.md).

## Prerequisites

- Rust 1.99 (pinned in `rust-toolchain.toml`; `rustup` installs it on first use).
- Node.js 24 (as in CI) and pnpm 12 (pinned in `webui/package.json`).
- A Chat Completions-compatible model service with tool calling and streaming. Semantic knowledge search also needs an OpenAI-compatible `embeddings` endpoint.

Install the web UI's dependencies once:

```sh
pnpm --dir webui install
```

## Run

Build the web UI, then run the application, which serves it:

```sh
pnpm --dir webui build
cargo run --release -p nekowriter-app
```

`nekowriter` listens on `127.0.0.1:47821` and opens the UI in the default browser. Options:

| Option | Meaning |
|---|---|
| `--data-dir <path>` | Where the database and logs live. Default: `NekoWriter` in the OS local application-data directory. |
| `--port <number>` | The port on 127.0.0.1. A busy port is an error. |
| `--no-browser` | Do not open the browser. |

Only one process can use a data directory at a time. Stop the service with Ctrl+C, or with "Exit NekoWriter" on the Settings page. A database created by another version with a different schema is refused without being read or changed. There are no migrations.

## Develop

Run the service and the Vite dev server side by side. Vite proxies `/api` to the service:

```sh
cargo run -p nekowriter-app -- --no-browser
pnpm --dir webui dev    # http://localhost:5173
```

The TypeScript types in `webui/src/api/generated/` are generated from the Rust types by `ts-rs` when the Rust tests run. Never edit them by hand; run `cargo test --workspace` after changing a type that crosses the API.

## Test

One run checks everything, in dependency order (the Rust tests regenerate the API types the web UI checks against, and the release build embeds the web UI):

```sh
rm -rf webui/src/api/generated
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git status --porcelain -- webui/src/api/generated   # must show nothing
pnpm --dir webui exec prettier --check src
pnpm --dir webui lint
pnpm --dir webui typecheck
pnpm --dir webui test
pnpm --dir webui build
cargo build --release -p nekowriter-app
```

CI (`.github/workflows/ci.yml`) runs the same steps.

## Release

A release is the single `nekowriter` binary with the web UI embedded:

```sh
pnpm --dir webui build
cargo build --release -p nekowriter-app   # target/release/nekowriter
```

A release build fails if `webui/dist/index.html` is missing, so a binary cannot ship without its UI.

## Repository layout

| Path | What it holds |
|---|---|
| `crates/nekowriter-core` | Domain types and pure rules: workflow stages, runs, submissions, reviews, citations, templates, knowledge and usage. No I/O. |
| `crates/nekowriter-store` | Persistence over SQLite with SeaORM: idempotent commands, events, and every state change. |
| `crates/nekowriter-agent` | The model protocol: Chat Completions and embeddings clients, stage contracts, tools, context assembly, and the model-facing text in `resources/`. |
| `crates/nekowriter-app` | The `nekowriter` binary: HTTP API, event stream, dispatcher, indexer, export and the embedded web UI. |
| `webui` | The React web UI. User-facing text lives in `webui/src/locales/`. |
| `docs` | The product specification and the architecture. |

## Languages

The interface is in English and Simplified Chinese and follows the browser, with a switch in Settings. Each work has a writing language, Simplified Chinese or English, in which the agents write the novel. The built-in prompts are Chinese. Code, comments, logs and documents are English. The `english_only` test enforces this outside the resource directories that hold translated text.
