# NekoWriter

A local-first studio for writing long-form fiction with AI agents. The author directs and
approves; agents plan, research, draft, review, and keep an explicit canon (tiered memory,
entity graph, timeline, setting materials) so long stories stay consistent.

**Status:** design complete, implementation not started. Implementation follows
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

## Development (once implemented)
```bash
pnpm --dir web install
cargo run -p nekowriter          # API on http://127.0.0.1:7878
pnpm --dir web dev               # UI on http://localhost:5173
scripts/check.sh                 # all checks
```
