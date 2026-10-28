# NekoWriter — Implementation Plan

Execute the milestones **in order**. Each milestone builds on the previous one and ends in a
working, demonstrable state. Read `AGENTS.md` first.

**Every task is done when:**
1. the code implements the referenced spec sections (the spec wins over intuition; deviations
   are recorded in `docs/DECISIONS.md` with a reason);
2. the tests named in the task exist and pass;
3. `scripts/check.sh` is green;
4. every new UI string exists in `en` and `zh-CN`;
5. the work is committed (Conventional Commits, e.g. `feat(server): chapter content ops`).

Use the **Demo provider** (`mock://demo`, spec/04 §7.2) for every manual check, so no API key is
needed. Real providers are tested when the author supplies keys.

---

## M0 — Foundations

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 0.1 | Cargo workspace (`resolver = "3"`, edition 2024), crates `server` (`nekowriter`, lib + bin), `llm`, `embed`, `migration`; `rustfmt.toml`, `.cargo/config.toml` (TS_RS_EXPORT_DIR), `.gitignore` (`target/`, `web/node_modules/`, `web/dist/`, `.data/`, `.env`), `.env.example` | 01 §2.1, §3 | `cargo build --workspace` |
| 0.2 | Config (clap + env + dotenvy in debug), tracing, data dir, `AppState`, `AppError`/`ErrorCode` + JSON error body, router with `/api/health`, Host/Origin guard, the "run the dev server" hint page, graceful shutdown | 01 §5–§10 | API tests: health 200; `Host: evil.com` → 403; non-loopback bind refused without `--allow-remote` |
| 0.3 | DB connect with sqlite-vec auto-extension and per-connection pragmas; `Db::write` (process-wide write lock, spec/01 §7); `0001_init.sql` (full schema, spec/02 §2) run by `nekowriter-migration`; SeaORM entities for every table; typed JSON columns and enums | 02, 01 §5 | migrate a temp DB; `select vec_version()`; create a 4-dim vec0 table, insert, KNN; FTS5 insert + MATCH; JSON round trip for each typed column; 50 concurrent `Db::write` read-then-write transactions all succeed (no `SQLITE_BUSY`); the delete triggers remove FTS rows |
| 0.4 | ts-rs export pipeline (`export_bindings` test), generated files committed under `web/src/api/gen/` | 10 §1 | running the test twice yields no diff |
| 0.5 | Web scaffold (exact commands), providers, router skeleton with lazy routes, i18n (both locales, all namespaces created), theme toggle, language switch, `AppShell`/`TopBar`, API client, SSE client (connection-state dot), Library placeholder, Vite proxy, Vitest setup, `check-i18n` | 11 §1–§4, §6 | `pnpm typecheck`, `lint`, `test`, `i18n:check`, `build` pass; manual: the shell renders in en/zh-CN, light/dark |
| 0.6 | `scripts/check.sh` (exact order in 01 §12), `README.md` (dev quickstart) | 01 §12 | the script passes from a clean checkout |

## M1 — Books, tracks, chapters, setting materials (no agents)

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 1.1 | `text` module: `ChapterContent` ops (insert/update/delete/tail), pid allocation, UAX #29 word count + `wordcount_vectors.json`, CJK-aware joining, paragraph alignment for full-text saves (`similar`), language helpers (unspaced languages, display names) | 03 §8, 02 §4, 11 §8 | table-driven unit tests incl. Chinese, Japanese, English; alignment keeps pids for unchanged and edited paragraphs |
| 1.2 | Services + API: app settings (`book_defaults`), books (create → main track + 3 setting docs; archive; delete), tracks (main only for now), chapters (CRUD, reorder rules, content PATCH/PUT with `base_version`, versions/restore, submit, approve → official version + `canon_status = pending`), setting docs (PUT with `base_revision`), character designs (CRUD, reorder), change log for setting docs and characters (`actor: user`) | 08 §1, 10 §2.2–2.3, 06 §3 | service and API tests for every state rule and error code in 08 §1.1 |
| 1.3 | EventBus + `GET /api/events` (filters, keep-alive, lag → `resync`); emit `book.updated`, `chapter.updated`, `chapter.content`, `settings.changed` after commit | 10 §3 | API test reads the SSE stream and asserts the events of a content PATCH |
| 1.4 | UI: Library (cards, New book dialog: creates an active book for now), `BookLayout` with persisted resizable panels, `Navigator` (track chapters with dnd-kit reorder for non-official), Overview (progress only), `ChapterPage` (Text: paragraph editing, full-text editor, word count; Outline: editable; History: versions, `DiffView`, restore), setting docs editors (textarea + preview), character rank list (dnd-kit), Book settings page, `StatusBadge` tokens | PROJECT §8.3–§8.5, §8.9, §8.12; 11 §5 | Vitest: `chapterDoc.applyOps`, `wordcount`. Manual: create book → chapters → write → submit → approve; diff and restore; reorder characters; two browser tabs stay in sync through SSE |

## M2 — LLM layer, providers and models, prompt registry

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 2.1 | `nekowriter-llm`: types, OpenAI-compatible adapter, Anthropic adapter, error classification, retry helper, `ScriptedProvider`, `DemoProvider` (skeleton with `crates/llm/demo/corpus.toml`; behaviors are completed per milestone) | 04 | all fixture and body-builder tests in 04 §9 |
| 2.2 | Providers and model profiles: services + API (CRUD, test, remote models, presets), `env:NAME` keys, masking; `roles.defaults`; book `model_overrides` | 04 §8, 10 §2.1 | API tests incl. masking; `wiremock`-backed test for "Test connection" and remote models |
| 2.3 | Prompt registry: load the embedded pack (`prompts/*.toml`), `PromptSet::for_book`, MiniJinja environment (strict, includes), context structs + `sample_contexts(key)` (one per branch) for every key, validation, API | 09 | render every key with each of its samples (insta snapshots); override resolution order; invalid template → `prompt.template_invalid` with line/column; a broken override branch at runtime falls back to the default with `prompt.override_failed` |
| 2.4 | UI: Settings › Providers, Models (Fetch models), Agent roles, Appearance; Prompts (global) and Book › Prompts (grouped list, editor, variable docs, diff vs default, reset, validation errors) | PROJECT §8.11–§8.12 | manual: add the Demo provider and a model, assign roles; edit and reset a prompt override |

## M3 — Agent runtime core

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 3.1 | Session services; `SessionManager` (spawn, cancel with a reason, one runner per session, startup recovery, graceful shutdown → `interrupted`); runner (`step_model`, `step_tool` with the prepare/execute split, sealing, finalize incl. `after_completion`, transition with view snapshots and pause clearing), inbox, counters, usage, retries, nudges, circuit breakers | 03 §1–§7 | runtime tests with test-only tools (`echo`, `test_write` in WRITE_TOOLS, `test_finish` with `complete`, `test_slow` with a cancellable `prepare`) covering spec/03 §15 items 4, 5, 7, 8, 11, 12, 14, 15, 16, 17, 20, 21 |
| 3.2 | Tool framework: `ToolHandler` (`prepare` outside the transaction, `execute` inside), registry, `tools.toml` loading, `jsonschema` validation, example tests, role tool lists and phase permissions, result rendering via templates, entity reference resolver (unit-tested on fixtures) | 05 §1–§2 | schema/struct consistency test; permission matrix test |
| 3.3 | Control API (pause, resume, stop, continue, abandon, message, answer, PATCH) and session SSE events (`updated`, `message`, `delta`, `tool_call`, `tool_preview`); `PreviewExtractor` | 03 §6, §12; 10 §2.7, §3 | extractor fed at every byte split; control state-table tests (invalid ops → 409) |
| 3.4 | Session creation: model resolution, `model_snapshot`, frozen `system_prompt`/`tools`, kickoff assembly framework (block list per kind) | 03 §4, §10–§11 | request-building test: identical prefixes across steps |
| 3.5 | Agent panel UI: header, controls, activity stream (virtualized, live buffers, tool cards with a generic renderer, appendices), composer by state, usage footer; toasts for held, awaiting, failed, completed; session switcher | PROJECT §8.6; 11 §5.3 | Vitest for live buffers; manual with a test-only session started by a debug endpoint `POST /api/debug/sessions/echo` (debug builds only) |

## M4 — Writing chapters (planning → drafting) with author participation

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 4.1 | Context: `block.settings`, `block.chapter_draft`, `block.anchor` (rendered only for side tracks, which arrive in M10), memory v1 (summaries, previous tail, protagonist when present; empty-state rendering), `writer.kickoff.chapter`, `writer.kickoff.redraft`, `writer.kickoff.rollover` | 03 §10; 06 §4.2 | insta snapshots of kickoffs for a first chapter and a later chapter |
| 4.2 | Writer tools: `submit_outline`, `read_draft`, `insert_text` (all granularities, open-tail rule), `insert_paragraphs`, `replace_text`, `rewrite_paragraph`, `delete_paragraphs` (single-paragraph limits under sentence/paragraph), `finish_draft`; short-completion confirmation; effects and completion handler (draft version, `in_review`) | 05 §4; 03 §8 | granularity matrix tests; each tool's errors; runtime test 19 |
| 4.3 | Holds: auto-pause, pause after outline (approve and request-changes paths), feedback appendices, deferred-effect revocation, edit reports (paragraphs full and compact, outline and reference notes), granularity PATCH with notice | 03 §5.2–§5.4, §6–§8 | runtime tests 1, 2, 3, 6, 9, 10, 18 |
| 4.4 | Rollover (incl. `author_notes` and the revision plan) | 03 §9 | runtime test 13 |
| 4.5 | `POST /chapters/:id/write` (preconditions, `force`), Demo provider writer behaviors | 08 §1.1, §2; 04 §7.2 | API tests for the preconditions |
| 4.6 | UI: `WriteDialog`, live `ChapterText` (insert, update, delete animations), `GhostText`, `HoldBar` (text and outline variants), lock banner, granularity control and auto-pause switch, writer tool renderers | PROJECT §8.5–§8.6; 11 §5.2 | manual with Demo: write a chapter at each granularity; pause mid-stream, edit while held, give feedback, switch granularity; stop and continue; restart the server while held, then continue |

## M5 — Canon, retrieval tools, tiered memory

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 5.1 | Canon services: entities (incl. protagonist flag, merge), relations, appearances, timeline (positions, midpoint insert, renormalize, move), memory notes, foreshadowing, change log for all, revert rules; visibility helper | 06 §1–§3, §9; 02 | service tests incl. revert of create/update/delete, `change.not_latest`, an isolated-track visibility fixture, merge edge cases (both entities in one chapter, a relation between them, private into shared, shared into private refused) |
| 5.2 | Canon API + UI: Entities (table + detail, relations editor, attributes editor, history), Timeline (dnd-kit reorder, insert between), Memory notes, Foreshadowing, Changes (diff + revert) | PROJECT §8.8; 10 §2.4 | manual: create, edit, revert; reorder the timeline |
| 5.3 | Memory snapshot (full: mid-term evidence, absences, key notes, open foreshadowing, cast, counts) + `block.memory` + `GET /books/:id/memory` + Memory page | 06 §4 | snapshot tests with a 12-chapter fixture (windows, caps, ordering, empty states) |
| 5.4 | Retrieval tools without semantic search: `get_entity`, `list_entities`, `get_relations`, `query_timeline`, `list_foreshadowing`, `read_chapter`; Demo planning uses `get_entity` | 05 §3 | per-tool tests incl. resolution errors and anchor markers |

## M6 — Review, revision, maintenance, proposals

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 6.1 | Reviewer: `chapter_review` session (kickoff), `submit_review`, reports API, re-review (same session, or a new one with `prior` when the text changed), stale reviewer sessions abandoned on approve/send back, Demo behavior | 08 §3; 05 §5; 03 §6 | runtime tests: review → report → rereview → report #2; edit the text → rereview starts a new session whose kickoff contains the previous report |
| 6.2 | Send back: `chapter_feedback` (with the ticked `issue_indexes` copied into `issues`), `chapter_revise` session (`writer.kickoff.revise`, `submit_revision_plan`, `finish_revision`, completion), redraft path | 08 §1.1, §3 | API and runtime tests for both modes; versions created |
| 6.3 | Archivist: `chapter_maintenance` (kickoff with cast index), all canon write tools, `set_chapter_summary`, `finish_maintenance` validation, completion handler (`synced`/`stale` via `base_content_version`, `failed`), auto-maintenance on approve, `maintain` (continue or start) and `resync` (official version first) endpoints, `block.chapter_diff` | 06 §6, §8; 05 §6 | runtime tests with scripted maintenance; an author edit during maintenance → `stale`; abandoning a stopped maintenance session → `failed`, and Retry starts a new one; isolated-track write rejection |
| 6.4 | Proposals: `propose_setting_change`, accept/reject/conflict, the proposals page and inline display on setting docs | 06 §5 | service tests for find uniqueness and conflict |
| 6.5 | UI: Review tab (reports, issues jump/highlight, editable feedback), Send-back dialog, Discuss dialog, maintenance report rendering in the agent panel, canon badges and Retry/Re-sync actions, proposals badge in the navigator | PROJECT §8.5 | manual full loop with Demo: write → review → send back → revise → approve → maintenance → inspect changes → revert one → accept a proposal |

## M7 — Search index and knowledge base

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 7.1 | `nekowriter-embed`: `FastEmbedder` (the three supported models from `crates/embed/models.toml`, prefixes, normalization, lazy load, status), `FakeEmbedder` | 07 §4 | unit tests with `FakeEmbedder`; an ignored test that loads the default model (run manually) |
| 7.2 | Index manager (vec tables with `corpus` and delete triggers per active dimension, model switch rebuild), FTS with jieba, `search_docs` maintenance hooks in canon and chapter services (passages on every official content change), knowledge chunking, indexer worker, `index.status` events | 07 §1.1–§4; 02 §5 (11) | tests: chunking, segmentation, upsert/delete flows, rebuild on dimension change, no orphan index rows after deleting a book and a scope, an item changed or deleted while its batch is embedded gets no stale vector |
| 7.3 | Hybrid query (RRF, filters, snippets, grouping) | 07 §5 | ranking tests on a fixture corpus (exact-name queries win via FTS; paraphrases via vectors) |
| 7.4 | Knowledge services/API (scopes, entries, search, book scopes) + UI (three-column page, search box, Book › Knowledge scope picker) + Settings › Embeddings | 07 §1; PROJECT §8.10 | manual: create scope and entries, search, attach to a book |
| 7.5 | Tools: `search_canon`, `search_knowledge`, `read_knowledge_entry`, `search_chapters` (writer and reviewer lists) | 05 §3 | visibility tests (scopes, tracks) |

## M8 — Web tools and research

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 8.1 | `web_search` providers (Tavily, SearXNG), Settings › Web search with Test | 07 §6 | wiremock tests for both providers |
| 8.2 | `web_fetch`: SSRF resolver, redirect policy, limits, decoding, Readability + htmd, paging cache, `notice.web_content` | 07 §7 | tests: private IPs rejected (literal and DNS), redirect to a private IP rejected, GBK page decoded, size cap, paging |
| 8.3 | Research tasks (services/API, task language, run preconditions), researcher session, `save_knowledge_entry`, `update_knowledge_entry`, `finish_research`, budget notice, Demo behavior; Research task page and the list in Book › Knowledge | 07 §8; 05 §8 | runtime test with scripted research; manual with Demo |

## M9 — Book conception

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 9.1 | Conception tools: `ask_user` (answer validation), `write_setting_doc`, `edit_setting_doc`, character tools, `set_research_topics`, `request_review` (approvals bookkeeping); conception view snapshots and `notice.setting_edits` reporting; `settings.locked` while running; idle-on-text-only behavior | 05 §7; 03 §5.6, §7; 08 §1.4 | runtime tests: ask → answer → write → review → revise; author edits a document and a profile while awaiting review → the answer's result carries the setting edits |
| 9.2 | Book creation with `start_conception`, `conception/start`, `conception/finalize` (preconditions, research tasks, scopes, seed) and `canon_seed` (`archivist.kickoff.seed`, design ↔ entity linking) | 08 §5; 06 §7 | API tests for finalize preconditions and effects |
| 9.3 | Conception studio UI (conversation, live documents, `AskUserForm`, `ReviewRequestForm`, character DnD, research topics editor, Finalize) and the New book dialog with conception on | PROJECT §8.7 | manual with Demo: the full conception → finalize → seed → overview flow |

## M10 — Side stories

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 10.1 | Side track creation and editing (canon mode lock, anchors, focus character), chapter rules for the first side chapter, side-track memory (§4.1 fill rule, focus character, post-anchor state flag), `block.anchor` variants, anchor markers in tools | 08 §6; 06 §2, §4.1 | memory snapshot tests for side tracks; anchor marker tests |
| 10.2 | Isolated writes end to end (maintenance in an isolated track), canon pages track filter and badges | 06 §2 | runtime test: isolated maintenance cannot modify shared rows; private rows are visible only in that track |
| 10.3 | UI: New side story dialog, Navigator group, track page | PROJECT §8.4 | manual with Demo: write and maintain a side-story chapter in both modes |

## M11 — Hardening and polish

| # | Task | Tests / acceptance |
|---|---|---|
| 11.1 | Overview: next action and attention list (08 §7); Quick switch (⌘K); keyboard shortcuts with IME guard | manual |
| 11.2 | Empty states, error states and localized error toasts for every error code; confirmation dialogs | `i18n:check`; manual pass in both languages |
| 11.3 | Performance: memoized paragraphs, rAF-coalesced deltas, virtualized long lists; a 300-paragraph chapter stays smooth while streaming | manual profiling |
| 11.4 | Release build with `embed-web` (single binary serving the SPA with fallback) | the binary works without the Vite server |
| 11.5 | End-to-end smoke script (below) with the Demo provider, then (if keys are available) with one real provider per protocol; fix everything found | the checklist passes |
| 11.6 | Documentation: README (install, run, configure), `docs/DECISIONS.md` complete | review |

### Smoke checklist (M11.5)
1. Fresh data dir → add the Demo provider, a model, roles → create a book with conception →
   answer questions → review (request changes once) → finalize → canon seeded.
2. Run a research task → entries searchable (keyword immediately, vector after indexing).
3. Write the outline of chapter 1 with pause-after-outline → request changes once → approve
   (the chapter stays planned) → Write the chapter at paragraph granularity → pause mid-draft →
   edit a paragraph → feedback → finish.
4. Ask the reviewer → discuss → send back with edited feedback (revise) → finish revision →
   approve → maintenance → inspect and revert one change → accept a proposal.
5. Approve chapter 1, then outline chapter 2 and write it at sentence granularity with
   auto-pause for three steps, then switch to free.
6. Stop mid-stream → restart the server → continue. A held session survives the restart.
7. Create an isolated side story anchored after chapter 2 → write, approve, and maintain one
   chapter → shared canon is unchanged.
8. Override `planner.system` and `writer.system` for this book → the next outline session uses
   the planner override and the next write session the writer override (check each session's info
   popover); sessions created before the change keep the old prompt.
9. Switch the UI language and theme. Every screen is fully translated.

## M12 — Outline and write sessions

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 12.1 | **Docs first.** Amend every section listed in the outline-split plan §5, append D-146 (or the next free number) recording its §0 and the §9 answers, and append this table to PLAN.md. | plan §5 | `check.sh` green (docs only) |
| 12.2 | **Schema and types.** Migration `0004` plus the `0001` edit; the enums and DTOs of plan §3; regenerate the bindings. | 02 §1–§5, 10 §2.8, §4 | `outline_split_migration.rs`: a pre-0004 fixture holding every kind, a held call, a parent chain, inbox rows, reports, feedback, proposals, versions, change log and research tasks upgrades with every row and reference intact; a deliberately broken copy fails loudly with no data lost; the override move and the `NOT EXISTS` guard; the new values are accepted. JSON round trips for `ToolEffects` and `RoleModels`. |
| 12.3 | **The outline session, additive.** Planner role and tool list (`registry.rs`); `planner.*` keys plus contexts, samples, `variables.toml` and the validator's role tool lists and samples; kickoff; staging plus `apply_pending_outline` in both finalize paths; `outline` completion; revoke-notice precedence; nudge and budget keys by role; view snapshot and edit lock for the kind; planner model fallback; `POST /chapters/:id/outline`; Demo `chapter_outline` arm. The old write flow is untouched. | 03 §1, §4–§7, §10, §14; 05 §2, §4; 04 §7.2 | `outline_session.rs`: hold → approve applies outline, notes and title, the session is `completed`, no further request, the status is still `planned`; feedback → rejection notice, edit report and feedback in order, no `completion_revoked`, resubmit works; flag off → completes immediately; abandon while held → previous outline intact; the chapter is byte-identical while held; author edits while held are reported. `planner_routing.rs`: the four-level precedence and `missing_role`. Registry permission matrix. `prompts_registry.rs`: snapshot plus planner guard tests. |
| 12.4 | **The write session becomes drafting-only.** Writer list without `submit_outline`; `initial_phase` → drafting; new `writer.system` and `writer.kickoff.chapter` (granularity rule included); `block.chapter_draft` in the write kickoff; `writer.kickoff.redraft` retired; redraft via `kickoff_extra.feedback_id`; rollover feedback chain rule; `outline_missing` on write and redraft; legacy branch kept and marked; legacy notice titles; `tool.submit_outline` and `result.writer_control` (`staged`). | 03 §1, §4, §9–§10, §15; 05 §2, §4; 08 §1.1, §2–§3; 09 §2 | `write_session_split.rs`: a fresh chapter → outline (approve) → write starts in drafting without `submit_outline` frozen, the kickoff carries the outline, notes and granularity rule, and a scripted `submit_outline` call is refused with no effect (AC1); re-outline after prose leaves text, versions and the write session's messages byte-identical (AC2); separate usage (AC3); redraft → drafting session with the feedback in the kickoff; a plain drafting rollover has no redraft sentence; `outline_missing` (empty and whitespace-only). `legacy_planning_session.rs`: a fixture `chapter_write`/`planning` session plans, holds, approves (notice), drafts and completes, and feedback gives the legacy rejection notice (AC5). Rewrite spec/03 §15 tests 1, 2 and 18 for the outline session; update the kickoff snapshots. |
| 12.5 | **Overview, locks, DTOs.** Next-action rules 5 and 7; `has_outline`; `CHAPTER_ROLES` + planner; lock group in creation, approve, submit, restore and the edit lock; PATCH per kind. | 08 §1, §7; 10 §2 | `overview_actions.rs`: one case per stage — no outline, outline in progress, held, approved, drafting, in review, official unsynced, official synced → `write_next`; a planned chapter is offered before `write_next` (AC6). `outline_concurrency.rs`: the R6 matrix and a concurrent-creation race (D-121 style); 423 while an outline session runs. |
| 12.6 | **Web.** Role lists and the follows-writer hint; Outline dialog; Write dialog; header action table; Outline tab states plus the legacy path; `outlineState`; next-action kinds; navigator `has_outline`; session titles and icons; send-back wording; Prompts group; both locales. | 11 §5–§6, §9; PROJECT §8 | Vitest: an `outlineState` table; header actions per state; Outline tab approve and request-changes calls `resume` with and without feedback, and the legacy held path still saves then resumes; the card's targets and labels; the Roles and Book settings pages render six rows and the planner hint; the Write dialog's `outline_missing` state; `i18n:check` (AC4, AC7). |
| 12.7 | **Smoke and pre-ship.** Update smoke items 3, 5 and 8 for the two steps and run them with the Demo provider. Run `0004` on a **copy** of the author's database and report open legacy planning sessions and the overrides of every key in plan §2.10 with their fate. Record the results in the D-entry's consequences. | PLAN §M11.5 | the checklist passes; the report is in the final message |
