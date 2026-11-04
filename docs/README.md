# Documentation index

| Document | What it covers |
|---|---|
| [PROJECT.md](PROJECT.md) | Product description: problems, principles, glossary, agent roles, features, user flows (F1–F12), UI screens, scope, requirement interpretations, traceability |
| [spec/01-architecture.md](spec/01-architecture.md) | System context, pinned stack, repository layout, module trees, startup, configuration, concurrency, errors, logging, security, testing, build |
| [spec/02-data-model.md](spec/02-data-model.md) | Full SQLite schema (DDL), enums, typed JSON columns, invariants |
| [spec/03-agent-runtime.md](spec/03-agent-runtime.md) | Sessions, state machine, runner loop, holds, sealing and appendices, control operations, edit reports, granularity, rollover, context assembly, previews, required tests |
| [spec/04-llm.md](spec/04-llm.md) | Provider-neutral types, OpenAI-compatible and Anthropic adapters, errors and retries, model options, mock providers, presets |
| [spec/05-tools.md](spec/05-tools.md) | Tool framework, role tool lists and phase permissions, every tool's behavior |
| [spec/06-canon-memory.md](spec/06-canon-memory.md) | Canon field semantics, side-story visibility, change log and revert, tiered memory, proposals, maintenance, seeding, re-sync |
| [spec/07-knowledge-search.md](spec/07-knowledge-search.md) | Knowledge base, chunking, search index (FTS5 + jieba, sqlite-vec), embedder, hybrid retrieval, web search and fetch, research workflow |
| [spec/08-workflows.md](spec/08-workflows.md) | Chapter lifecycle and operations, writing, review, conception, side stories, overview |
| [spec/09-prompts.md](spec/09-prompts.md) | Prompt registry: pack layout, keys, resolution, overrides, authoring rules |
| [spec/10-api.md](spec/10-api.md) | REST endpoints, DTOs, server-sent events, error codes |
| [spec/11-frontend.md](spec/11-frontend.md) | Scaffold commands, structure, routing, data layer, stores, components, i18n, visual details |
| [PLAN.md](PLAN.md) | Milestones M0–M23 with tasks, acceptance criteria, smoke checklist |
| [DECISIONS.md](DECISIONS.md) | Decision log |

Reading order for implementers: `../AGENTS.md` → PROJECT → spec 01–03 → the specs referenced by
the current PLAN task.
