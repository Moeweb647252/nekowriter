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
10. Writing goals (M13):
    - Create a goal, a plan under it, and a task for the next chapter, each with a purpose. Add a
      second task waiting for a hook to be resolved.
    - Outline and write the chapter. The planner's and the write session's kickoffs (session info
      popover) show the three purposes and not the waiting task.
    - Approve and maintain. Accept the archivist's goal proposal.
    - Resolve the hook by hand. The waiting task turns ready without a reload; activate it.
    - Revert one goal change on the Changes page.
11. Canon system switches (M13):
    - Turn Foreshadowing off and save. Its navigator entry disappears and the page explains what is
      kept.
    - Outline the next chapter: no hooks in the kickoff, no foreshadowing tools in the session's tool
      list.
    - Approve: maintenance skips the step and the chapter syncs.
    - Turn Foreshadowing on again. The entry and the data are back.
12. Knowledge first (M15, plan §8):
    - On the global Knowledge page, create a scope with one entry. Create a book with conception
      and auto-start on. The studio shows the knowledge check, whose kickoff lists the scope. The
      check lists research topics, they run (smoke item 15), and when the last one finishes,
      conception starts by itself (D-168). The conception kickoff shows the scopes and the report,
      and its tool list has `search_knowledge`.
    - In Book › Knowledge every scope is checked. Uncheck one and save. The next session's
      `block.settings` no longer lists it.
    - Create a second book with auto-start off. After the check and its topics, the studio shows
      the report and **Start conception**, and conception does not start by itself.
    - In that book, add a research topic in the Research topics tab, then start conception and let
      the agent set its topics. Your topic is still there, marked "Added by you".
    - Create another scope. Both books see it without any action.
13. Feature gaps (M18, plan §4):
    - Create an entity "Test sword" (item, shared), then a second one "The sword". Merge "The
      sword" into "Test sword": its name becomes an alias, and the Changes page shows the merge
      notes.
    - Edit a relation's description and end it. The row shows "ended", and the Changes page shows
      the relation change with your note (the entity history lists entity changes only).
    - Edit a timeline event's story time and linked entities. Its position is unchanged.
    - On an entity with more than 20 changes, **Show full history** lists the older ones.
    - Open a long session and scroll to the top. Older messages load without the view jumping.
14. Thinking effort per role (M19, plan §5):
    - Demo: the Demo model's dialog shows the "no effort levels" note, and the Agent roles effort
      select of a role on the Demo model is disabled.
    - With a real provider (when a key is available): give the model the levels `low`, `high`,
      `max` with default `high`; set the reviewer to `low` and the writer to `max`. Write and review
      a chapter: the session headers show "max" and "low".
    - Override the writer to `high` in Book settings. The next write session shows "high".
    - Remove `high` from the model. The next write session shows the unsupported toast and uses
      `max`, the global choice.
15. Research before conception (M20, plan §6):
    - Demo, auto-start on: create a book with conception. The studio shows the knowledge check; it
      lists two research topics and finishes. The topics run one after another (one research
      session at a time), each saving entries. When the second finishes, conception starts by
      itself, and its kickoff lists both topics as done.
    - A second book, auto-start off: while the first topic runs, edit the second topic's
      requirements and add a third topic. Both run in order. After the last one nothing starts,
      and **Start conception** starts conception.
    - A third book: abandon the running topic; the next one starts. Skip the last waiting topic;
      when the running one finishes, the research is settled.
    - In conception the agent sets supplementary topics (Research topics tab). Finalize: they become
      `ready`, and the topics researched before conception stay `done`.
16. Book agent (M21, plan §6):
    - Demo, an active book: click **Book agent** and send a message in **Review changes**. The
      agent reads the overview; its new research task waits with a preview. **Request changes**:
      no task appears, and the agent replies.
    - Send another message and **Apply this change**: the task appears in Book › Knowledge.
    - Switch to **Full access** and send a message: the next task appears without a stop.
    - While the book agent is open, start a write session from a chapter: the panel stays on the
      book agent.
    - **End conversation**; **Book agent** shows the start view again.
    - A conceiving book: the Book agent entry is disabled with the studio hint.
17. Timeline as history and plan (M22, plan §6):
    - Demo, a fresh book with conception: the canon seed creates planned entries from the outline,
      nested under phase entries, and anchors a foreshadowing to one of them. No chapter number
      appears on the Timeline or Foreshadowing page.
    - Write and approve chapter 1. Maintenance promotes one planned entry to happened with
      chapter 1 and adds a sub-entry.
    - A goal task waiting on **Event happened** for that entry is badged ready afterwards.
    - Abandon a planned entry: its planned sub-entries go with it, it stays greyed on the page, and
      a session's `query_timeline` no longer returns it. **Abandon** is not offered on a happened
      entry.
    - **Delete** is disabled on an entry with sub-entries. Deleting a leaf and reverting it brings
      it back under its parent.
18. Context view (M23, plan §6):
    - Demo, a book with an approved chapter 1: outline chapter 2, approve the outline, and write
      it. Open **Context** from the footer. The setup groups fill most of the view: system prompt,
      tools, and a kickoff with **Setting materials**, **Memory**, **Recent chapters**, **Chapter
      draft** and **Task**. The header says **Estimate**.
    - After a few steps, **Model output** and **Tool results** grow and the strip gains columns.
      An early column shows less, and **Current** stays outlined.
    - Zoom into **Tool results**, then into a tool; Escape returns, and Escape at the top closes
      the dialog. **List** shows the same numbers.
    - A steering message sent while the agent runs appears under **From the author** once
      delivered.
    - **Context** also opens from the conception studio and from a research task page.

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

## M13 — Writing goals and canon system switches

Plan: `docs/research/goals-and-system-switches-plan.md` (D-150). The author answers its §12
before 13.3 starts. **13.3 starts after M14** (the SeaORM 2 persistence-layer refactor, D-151).

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 13.1 | **Docs** — done with the plan (D-150). Re-check them against the author's §12 answers. | plan §8 | `check.sh` green (docs only) |
| 13.2 | **Pin the baseline** before any code change: snapshot every chapter-kind kickoff (outline, write, revise, review, maintenance, seed, resync, rollover) and every role's frozen tool list for an all-on fixture book with canon data and no goal nodes. | 03 §10 | `kickoff_baseline.rs` and its `.snap` files, committed on unchanged code; every later task leaves them untouched (AC13) |
| 13.3 | **Schema and types**: migration `0006` plus the `0001` edit; goal tables and entities; `agent_sessions.canon_systems`; enums, JSON types, DTOs and error codes; regenerate the bindings. | 02 §1–§5; 10 §2.4, §2.8, §4 | `goals_migration.rs`: a pre-0006 fixture (changes with revert links, every proposal type, sessions) upgrades with every row and link intact; a broken copy fails loudly with no data lost; the new values are accepted; old sessions read NULL. JSON round trips: `BookSettings` without the field → all on; a partial `canon_systems` → the missing fields on |
| 13.4 | **Switches in the runtime**: freeze `canon_systems`; filter tool lists; `block.canon_systems`; `block.memory` fragments; system-gated blocks; the `search_canon` filter; `finish_maintenance` required steps; conditional archivist tasks; the seed gate and overview rule 2; rollover inheritance; the Demo provider skips unfrozen tools; `GET /canon/counts`; `MemoryViewDto.systems`; prompt pack, variables, samples, validator. | 03 §1.2, §4, §9, §10, §15; 05 §2–§3, §6; 06 §4.6, §6–§8, §10; 08 §5, §7; 09 §2 | `canon_switches.rs` (spec/03 §15 tests 27–29): each system off removes its data, tools and step and keeps its rows (AC9, AC10); Memory off keeps the outline, including the IR-01 fallback (AC11); off then on restores everything (AC12); a mid-session flip changes nothing in that session or its rollover child (AC14); maintenance with Memory, Entities or all five off completes `synced` (AC15). `kickoff_baseline` unchanged (AC13). Registry and validator: the archivist's 22-tool role list; filtered lists keep the order |
| 13.5 | **Goals service and API**: `services/goals.rs` (validation, scope, parents, status rules, open-children confirmation, delete refusal, change log, the evaluator); revert arms; the entity merge; endpoints. | 02 §5; 06 §3, §9, §11; 10 §2.4 | `goals_api.rs`: create, edit and link a tree, refused without a purpose (AC1); one parent, re-parenting moves, tasks have no children (AC2); every mutation is logged and revertible, a delete or a create revert with children is refused (AC7). `goals_readiness.rs`: a hook resolved → ready with no refresh (AC3); chapter *n* approved → `chapter_at_least(n)` ready (AC4); broken targets; `closable`; scope checks; the merge moves links and conditions |
| 13.6 | **Goals in agent context**: `block.goals` and the relevance rules; the kickoff lists; the archivist gate; `propose_goal_change` (`tools.toml`, handler, result branches, `goal_proposals`); goal proposals accept and reject; the Demo arm. | 03 §10; 05 §2, §6; 06 §5, §6, §11.5–§11.6; 09 §2; 10 §2.3 | `goals_context.rs` (spec/03 §15 tests 30–31): the outline and write kickoffs at *k* carry each relevant task's purpose with its plan's and goal's, and nothing from a later task or a waiting node (AC5); the review kickoff the same (AC6); closed ancestors; side-track scope and book-wide goals; no leakage; the cap; Entities off removes entity links. `goals_proposals.rs`: create, complete and drop; conflict; the duplicate rule; open-children on accept; the isolated rule; no relevant goals → 21 frozen tools |
| 13.7 | **Overview**: the `ready_goals` attention row and next-action hint, gated by Goals. | 08 §7 | `overview_actions.rs`: a ready or closable node gives the row and the hint on the write kinds only; Goals off gives neither; the library count includes the row |
| 13.8 | **Web — switches**: the Canon systems section (hints, retained counts, View data, warnings, disabled window inputs); navigator and QuickSwitch hiding; the disabled-page banner with Turn on; Memory page sections; both locales. | 11 §5.1, §5.5, §9; PROJECT §8.4, §8.12 | Vitest per spec/11 §9; `i18n:check` |
| 13.9 | **Web — goals**: the Goals page, `GoalNodeDialog` and pickers, condition sentences, the Ready filter and badge, inline and Proposals-page cards, Referenced-by sections, the overview row and hint, the `chapter.updated` invalidation, the tool renderer; both locales. | 11 §3, §4, §5.5, §9; PROJECT §8.8 | Vitest per spec/11 §9 (AC8); `i18n:check` |
| 13.10 | **Smoke and pre-ship**: run smoke items 10–11 with the Demo provider; run `0006` on a **copy** of the author's database and report the guard counts and any overrides of `block.memory` and the three archivist tasks. | PLAN §M11.5 | the checklist passes; the report is in the final message |

## M14 — SeaORM 2 persistence-layer shape

Plan: `docs/research/seaorm-2-migration-plan.md` (D-151). Every task leaves the SQL files,
prompts, `web/src/api/gen`, API responses, events and the 13.2 kickoff snapshots unchanged.

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 14.1 | **Docs first**: §4, D-151, the M14 table. | plan §4 | `check.sh` green (docs only) |
| 14.2 | **Guards on unchanged code**: `tests/entity_schema.rs` checks 1–2 and `decode_all_rows`. | 02 §1–§2 | The new test passes before any entity changes. Deliberately breaking one entity column in a scratch edit makes it fail. |
| 14.3 | **Entity format, relation graph, unique keys**: all 36 entities become `#[sea_orm::model]`, the empty `Relation` enums go, and the relation graph and unique keys of §2.1 come in. Enum, JSON and bool fields stay `String`/`i32` in this task. | 02 §2 | Checks 3–4 pass. Smoke tests: `find_related` on a required FK and a nullable FK; the `parent_session` self reference through `Relation::ParentSession.def()` plus its stored `parent_session_id`, with `ChildSessions` as the reverse (a self-ref `BelongsTo` has no `Related` impl, so `find_related` is not available there); `books::Entity::load().with(tracks::Entity).with(knowledge_scopes::Entity)`; `entities` → `timeline_events` via the junction; `chapters::Entity::find_by_track_ord`. The suite passes with no other test edits. |
| 14.4 | **Enum and boolean columns**: the `string_enum!` derive, the 40 enum fields and the 8 bool fields; delete `enum_of` and the column uses of `from_str_opt`/`as_str`; the §2.9 grep gate; compile-driven test edits (string literal → enum). | 02 §1, §3 | Check 5 passes. Existing tests pass, changed only from string to enum comparisons. No `web/src/api/gen` diff. The grep gate lists only genuine string columns. |
| 14.5 | **JSON columns**: `JsonText<T>` with its unit tests, the 30 fields, `json::ModelOptions`, and removal of every silent column fallback (§1.4). | 02 §1, §4 | `JsonText` tests: for every type in the existing round-trip test, the stored text equals `serde_json::to_string`; `Option` ↔ `NULL`; a shape mismatch names the column; the empty string fails the CHECK. Check 6 passes. `strict_decode.rs`: a chapter whose `content` is valid JSON of the wrong shape (set through `Db::write_unprepared`) makes `GET /api/chapters/:id` answer `500 internal`, and a write session refuses to start instead of seeing an empty chapter. |
| 14.6 | **Writes** (§2.6): `guard_update`, `transition`, the session patch, the `canon_status` compare-and-swap, the `error` clear, manager recovery, indexer `embed_state`, `raw_sql!` for static raw statements, and a reason comment on every remaining raw statement. | 03 §3, §14; 07 §4 | The race and transition suites (`review_races`, `outline_concurrency`, `m11_transition_atomicity`, the agent harness suites) and the M7 index suites pass unchanged. A unit test: `guard_update` with a stale expected status returns `session.invalid_state` and changes no column. A unit test per non-trivial converted statement compares `.build(DbBackend::Sqlite).to_string()` with the SQL it replaces, ignoring quoting. `grep -n 'format!' crates/server/src/services/indexer.rs` shows no SQL. |
| 14.7 | **Reads** (§2.7): loaders for the §1.7 loops, `is_in` batching, partial models for the `chapters`/`chapter_versions` list queries, `has_related`, generated finders. | 07, 08 §7 | Each converted function keeps its output, including order, as proven by its existing tests or a new one written **before** the change. `overview_actions.rs` and the agent-context snapshots are unchanged. |
| 14.8 | **`COLUMN` sweep** (§2.8): mechanical, `src` and `tests`. | — | No behavior change. `grep` shows `Column::` only where §2.8 allows it. |
| 14.9 | **Pre-ship**: run `decode_all_rows` on a **copy** of the author's database; run smoke items 1–4 and 8 with the Demo provider; compare `cargo build -p nekowriter` times before M14 and after it. | PLAN §M11.5 | Every row decodes (any failure is reported with its table, id and column, and is not "fixed" in code). The checklist passes. The build times and row counts are in the final message and in D-151's consequences. |

## M15 — Knowledge first

Plan: `docs/research/knowledge-first-plan.md` (D-158). The spec sections it lists in §7 are already
amended; the code catches up task by task. Execute in order, one commit per task.

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 15.1 | **Docs**: done with the plan (D-158). Re-read the amended sections before 15.2, and fix any that a code fact contradicts (note it in D-158). | plan §7 | `check.sh` green (docs only) |
| 15.2 | **Schema and types**: migration `0008` plus the `0001` edit (`book_scope_exclusions`, `book_scopes` dropped, `research_tasks.kind` and `.origin` with the draft back-fill); entities; `ResearchTaskKind`, `ResearchTaskOrigin`; `ConceptionState.start_after_knowledge_check`; the DTO fields; bindings. | 02 §1–§5; 10 §2.2, §2.5, §2.8 | `knowledge_first_migration.rs` per plan §6 (AC2); `entity_schema.rs` on fresh and upgraded databases; JSON defaults |
| 15.3 | **Opt-out visibility**: `visible_scope_ids`, `PUT /books/:id/scopes {excluded_scope_ids}`, `include_scope` in run and finalize, `settings_context`; rewrite the "not attached ⇒ invisible" tests. | 07 §1; 08 §5 | `knowledge_visibility.rs` per plan §6 (AC1) |
| 15.4 | **Knowledge check backend**: task rules, `POST /books/:id/knowledge-check`, `researcher.kickoff.prep` (context, variables, samples, validator), `set_target_scope`, `scope_excluded` and `target_scope_unset`, `tools_for_session(preparation)`, visibility in checks, `finish_research` without a target, `SessionDto.research_task_kind`, the Demo arm. | 07 §8.1; 05 §2, §8; 09 §2; 10 §2.2, §2.5 | `knowledge_check.rs` per plan §6 (AC4, AC8); prompt snapshots |
| 15.5 | **Creation, auto-start, conception knowledge, protected tasks**: `BookInput` handling; the auto-start hook; conception tools (12) and kickoff variables (`scopes`, `knowledge_report`, `research_tasks`); `conception.system` and `conception.kickoff`; `origin` on create, PATCH and `set_research_topics`; `kept[]`. | 08 §5; 05 §7; 03 §10, §15 (32–33) | `knowledge_first_flow.rs` and `research_topics_ownership.rs` per plan §6 (AC3, AC5–AC7, AC10, AC11); `kickoff_baseline` changes only for the conception kickoff and tool list |
| 15.6 | **Web**: the New book dialog checkbox and landing; the studio's pre-conception states; Book › Knowledge (unchecking, Check knowledge); "Knowledge check" labels and session title; "Added by you"; the scope hint; both locales. | 11 §5.0, §5.4, §5.5, §9; PROJECT §8.7, §8.10 | Vitest per spec/11 §9 (AC9); `i18n:check` |
| 15.7 | **Smoke and pre-ship**: smoke item 12 with the Demo provider. Run `0008` on a **copy** of the author's database, and report per book the scopes that become visible, the research task counts before and after, and the drafts back-filled as `conception` per conceiving book. | PLAN §M11.5 | the checklist passes; the report is in the final message and in D-158's consequences |

## M16 — Codebase cleanup

Plan: `docs/research/codebase-cleanup-plan.md` (D-159). The spec sections it lists in §8 are
already amended. It runs after M15; if M15 has not started, M16 may go first, but the two are
never interleaved. **No behavior change**: kickoff baselines, kept prompts, API (minus the
never-returned error code) and schema stay identical. Every deletion is re-proven by
the plan's §4 finders when its task starts.

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 16.1 | **Docs**: done with the plan (D-159). Re-read the amended sections before 16.2. | plan §8 | `check.sh` green (docs only) |
| 16.2 | **Dependencies and manifests**: the unused entries of plan §2.1, the workspace entries left unused, `async-trait` in the migration crate through the workspace, and the redundant `[[test]]` sections. | 01 §2 | the §4.1 finder is empty; `cargo build --workspace --all-targets` and the release build with `embed-web` pass |
| 16.3 | **Rust functions** of plan §2.3 and what they alone used. | — | the §4.2 finder is empty; no `#[allow(dead_code)]` in `crates/*/src`; `kickoff_baseline` unchanged |
| 16.4 | **The legacy planning branch** (plan §2.4). First run its query on a **copy** of the author's database; if it is not 0, stop and revert the legacy amendments. | 02 §3; 03 §1, §4, §5, §15; 04 §7.2; 05 §2; 09 §2; 11 §5.2 | the plan's grep gate; every legacy test deleted or ported (listed in the commit); the unknown-key override test; `prompts_registry` loses only the three keys; smoke items 3–5 with the Demo provider |
| 16.5 | **Unused prompt variable and error code**. | 09 §2; 10 §4 | the validator passes; the `gen` diff is one `ErrorCode` member |
| 16.6 | **Web**: unused files, dead exports, template leftovers, and proven-unused i18n keys (re-run after 16.4). The plan §3.1 hooks stay; M18 uses them. | 11 §5.5 | the §4.3 finders report only what the plan keeps; web typecheck, lint, test, i18n, build pass |
| 16.7 | **Sweep and report**: stale mentions in `NOTES.md`, `README.md`, `docs/`; the final counts in D-159. | — | the report is in the final message |

## M17 — Steering messages carry their header

Plan: `docs/research/steering-header-fix-plan.md` (D-160). It is independent of M15 and M16 and
may run before either.

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 17.1 | **Render `notice.user_message` when a `user_message` inbox row is consumed**: one helper for the three paths (sealing appendix, user message after an empty batch, text-only handling); `agent_inbox.content` stays raw; raw fallback on a render failure; the rollover test comment becomes an assertion. Write the tests first, and they must fail on the current code. | 03 §5.3, §5.6; 11 §5.3 | `steering_header.rs` (plan §4: paths A–C, paused feedback, notices untouched, overrides, rollover, several messages); `kickoff_baseline` and `prompts_registry` unchanged; Demo smoke: a message sent mid-write shows 【用户留言】 in the next tool card's appendix |

## M18 — Close the feature gaps (web)

Plan: `docs/research/feature-gaps-plan.md` (D-161). Web only: no server, schema or prompt change.

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 18.1 | **Docs**: done with the plan (D-161). | plan §5 | `check.sh` green (docs only) |
| 18.2 | **G6 scrollback** in `ActivityStream`; remove the dead `AgentPanel` button. | 11 §5.3 | Vitest per plan §3; manual: a session over 200 messages scrolls back to its kickoff |
| 18.3 | **G1 create entity** and **G2 merge**. | PROJECT §8.8; 11 §5.5; 06 §9 | Vitest per plan §3; manual merge with a shared chapter |
| 18.4 | **G4 relation edit** and **G5 full history**. | PROJECT §8.8; 11 §5.5 | Vitest per plan §3 |
| 18.5 | **G3 timeline event edit**. | PROJECT §8.8; 11 §5.5 | Vitest per plan §3 |
| 18.6 | **Wrap-up**: `i18n:check`; smoke item 13 with the Demo provider. | PLAN §M11.5 | the checklist passes |

## M19 — Thinking effort per role

Plan: `docs/research/role-effort-plan.md` (D-162). It depends on nothing after M14 and may run
before or after M15–M18.

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 19.1 | **Docs**: done with the plan (D-162). | plan §6 | `check.sh` green (docs only) |
| 19.2 | **`crates/llm`**: `EffortRule` (`for_protocol`, `wire_key`), `ModelOptionKey::PROFILE`; the Anthropic adapter sends `output_config` only when `effort` is set. | 04 §3, §5, §5.1 | plan §4.1: `model_options.rs`, `anthropic_stream.rs` |
| 19.3 | **Schema and types**: `0009_role_effort.sql` and its module (Rust-managed transaction, guards); `model_profiles.effort_levels` / `default_effort` on the entity; `RoleModels.efforts`; `ModelOptionsDto` without the two keys and with `deny_unknown_fields`; `ModelProfileInput` / `ModelProfileDto` / `ProviderPresetDto` / `SessionDto` fields; regenerate the bindings. | 02 §1, §2, §4; 10 §2.1, §2.8 | plan §4.2 `role_effort_migration.rs`; `entity_schema.rs`; JSON round trips, including old `RoleModels` JSON without `efforts` |
| 19.4 | **Model profiles**: level validation per `EffortRule`; `options` and `extra_body` refusals; presets; the served option table; `ignored_effort_levels` and the startup scan; test connection with `default_effort`. | 04 §5, §5.1, §8; 10 §2.1 | plan §4.3 |
| 19.5 | **Roles and sessions**: save validation of `efforts` (app settings and book PATCH); the one resolver for every creation path; the snapshot field per protocol; `model.effort_unsupported`; `SessionDto.effort`. | 03 §4; 04 §5.1; 10 §1, §2.8 | plan §4.4 `role_effort.rs` (cases 1–11, old JSON, wiremock wire checks) |
| 19.6 | **Web: Settings › Models**: the Thinking effort section, the table column, the protocol-switch and mismatch handling; drop the two old option fields and their i18n keys. | 11 §5.5 | plan §4.5 |
| 19.7 | **Web: Agent roles, Book settings, SessionHeader**: the effort selects, `resolveRoleEffort`, the header label, the warning toast; both locales. | 11 §5.3, §5.5; PROJECT §8.6, §8.12 | plan §4.5; `i18n:check` |
| 19.8 | **Smoke and pre-ship**: run `0009` on a **copy** of the author's database; report per profile the options before and after, the new levels and default, and every dropped cross-protocol key; run smoke item 14. | PLAN §M11.5 | the checklist passes; the report is in the final message and in D-162's consequences |

## M20 — Research before conception

Plan: `docs/research/research-before-conception-plan.md` (D-168). It builds on M15 and depends on
nothing after it. Execute in order, one commit per task.

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 20.1 | **Docs**: done with the plan (D-168). Re-read the amended sections; fix any a code fact contradicts and note it in D-168. | plan §7 | `check.sh` green (docs only) |
| 20.2 | **Schema and types**: `0011_check_topics.sql` and its module (Rust-managed transaction, row-count guard; 0001 not edited); `research_tasks.check_id` with its self relation; `ResearchTaskDto.check_id`, `ResearchTaskCreate.check_id`; bindings. | 02 §1, §2, §5; 10 §2.5 | plan §5.1 `check_topics_migration.rs`; `entity_schema.rs` on fresh and upgraded databases |
| 20.3 | **The check plans**: `RESEARCHER_TOOLS` and `tools_for_session` (a check freezes four tools; `set_target_scope` retired but registered); the check branch of `set_research_topics` (cap 8, `scope_excluded`, own waiting topics only, `kept[]`); `researcher.kickoff.prep` with `prep.research_tasks[]` and `prep.max_topics`, without `prep.max_entries` (variables, samples, validator); the Demo arm and corpus (`prep_topics`, `prep_report`). | 05 §2, §7, §8; 07 §8.1; 09 §2; 04 §7.2 | plan §5.2 `knowledge_check.rs`; `prompts_registry` snapshots |
| 20.4 | **Check topics and the queue**: create with `check_id` (validation, `ready`, no setting-material lock), PATCH and DELETE without the lock; `research::advance_check` and `advance_after_session`; the four triggers (runner after `finish_research`, `sessions::abandon`, create, resume); the scope-description rule; conception auto-start moved to settle (refactor `start_after_knowledge_check` to take the book). | 07 §8.1; 08 §5; 10 §2.5 | plan §5.3 `check_topic_queue.rs` (cases 1–13) |
| 20.5 | **Supplementary conception topics**: `conception.system` deliverable 5 and process bullet, `conception.kickoff`, `conception.research_tasks[].entries_created`, the `check_id IS NULL` filter in conception's `set_research_topics`. | 05 §7; 08 §5; 09 §2 | plan §5.4; `kickoff_baseline` changes only for the prep kickoff and tools and the conception kickoff and system prompt |
| 20.6 | **Web**: `PreConceptionResearch` (states, topic list, Run next, Start conception now), the "From the knowledge check" label, the check page's topic list, the New book checkbox wording; both locales. | 11 §5.0, §5.4, §5.5; PROJECT §8.7, §8.10 | plan §5.5 Vitest; `i18n:check` |
| 20.7 | **Smoke and pre-ship**: smoke items 12 and 15 with the Demo provider. Run `0011` on a **copy** of the author's database; report the `research_tasks` count before and after, that every `check_id` is `NULL`, and that book #1's five drafts and conception session #4 are unchanged. | PLAN §M11.5 | the checklist passes; the report is in the final message and in D-168's consequences |

## M21 — Book agent

Plan: `docs/research/book-agent-plan.md` (D-169). It depends on nothing after M20. Execute in order,
one commit per task.

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 21.1 | **Docs**: done with the plan (D-169). Re-read the amended sections; fix any a code fact contradicts and note it in D-169. | plan §7 | `check.sh` green (docs only) |
| 21.2 | **Schema and types**: `0012_book_agent.sql` (the 0004 rebuild procedure: `role` + `assistant`, `kind` + `book_agent`, `write_mode`; reference counts and `foreign_key_check` guards; 0001 not edited); entity, `AgentRole::Assistant`, `SessionKind::BookAgent`, `WriteMode`, `RoleModels.assistant`, `SessionDto.write_mode`, `BookAgentStart`, the PATCH field; bindings. | 02 §1–§5; 10 §2.8 | plan §5.1 `book_agent_migration.rs`; `entity_schema.rs` fresh and upgraded |
| 21.3 | **Runtime**: `POST /books/:id/agent`, the assistant → writer model fallback, idle on text, `end`, `write_mode` PATCH while running, the approval hold before `execute`, `ToolOutcome.spawn`, the kickoff (`block.book_index`, `assistant.kickoff`, `assistant.system`, variables, samples, validator). | 03 §3–§6, §10, §14; 08 §8; 09 §2; 10 §2.2, §2.7 | plan §5.2 `book_agent_session.rs` |
| 21.4 | **Read tools and reused tools**: `ASSISTANT_TOOLS` and the freeze rules; the new read tools; main-story canon visibility; reused setting, canon and knowledge tools with `NULL` chapters; their previews. | 05 §2, §10 | plan §5.3 `book_agent_tools_read.rs` |
| 21.5 | **New action tools**: chapter, canon delete, goal, research and knowledge tools; in-transaction services; author-path locks and edit reports; backups; previews; error kinds and results. | 05 §10; 08 §1.4 | plan §5.4 `book_agent_tools_write.rs` |
| 21.6 | **`start_session`**: in-transaction service variants for the five actions (outline, maintain, resync, research, knowledge check; no write or review), `session_conflict`, `start_refused`, post-commit spawn in both commit paths. | 05 §10; 08 §1.1, §8 | plan §5.5 `book_agent_start_session.rs` |
| 21.7 | **Web and Demo**: the navigator item and panel button (manual pin), `BookAgentStart`, `WriteModeControl`, End/New conversation, `HeldCallReview` previews, tool renderers, the seventh role row; the Demo `book_agent` arm and corpus; both locales. | 11 §5.1, §5.3, §5.5; 04 §7.2; PROJECT §6.18, §8.6, §8.12 | plan §5.6 Vitest; `i18n:check` |
| 21.8 | **Smoke and pre-ship**: smoke item 16 with the Demo provider. Run `0012` on a **copy** of the author's database; report the guard counts and `foreign_key_check`, and that every session row decodes with `write_mode` `NULL`. | PLAN §M11.5 | the checklist passes; the report is in the final message and in D-169's consequences |

## M22 — Timeline as history and plan

Plan: `docs/research/timeline-tree-plan.md` (D-170). It depends on M21: it changes tools the book
agent reuses and adds `update_foreshadowing` to its list. Execute in order, one commit per task.

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 22.1 | **Docs**: done with the plan (D-170). Re-read the amended sections; fix any a code fact contradicts and note it in D-170. | plan §7 | `check.sh` green (docs only) |
| 22.2 | **Schema and types**: `0013_timeline_tree.sql` and its module (Rust-managed transaction; guards: entry count unchanged, one root per book, no entry without a parent; 0001 not edited); the root insert at book creation; entity, `TimelineKind`, `TimelineStatus`, `PayoffPosition`, snapshots with serde defaults; the DTO, input and patch fields; bindings. | 02 §1–§5; 10 §2.4 | plan §5.1 `timeline_tree_migration.rs`; `entity_schema.rs` fresh and upgraded |
| 22.3 | **Tree rules**: `services/canon/timeline_tree.rs` (parent, scope, consistency, abandon, subtree moves); `kind = 'entry'` in every position query and reader; delete refused with children and clearing anchors; revert rules; foreshadowing anchors; the API errors. | 06 §1.4, §1.5, §3, §9; 02 §5 | plan §5.2 `timeline_tree.rs` |
| 22.4 | **Tools**: the `add_timeline_event`, `update_timeline_event`, `query_timeline`, `list_foreshadowing`, `add_foreshadowing` and `resolve_foreshadowing` changes; `update_foreshadowing` (registry, freeze map, archivist and assistant lists, validator map); `timeline_conflict`, `timeline_has_children`, `system_off` for anchors; agent visibility; search docs (no root, abandoned removed, planned flag). | 05 §2, §3, §6, §10; 07 §2 | plan §5.3 `timeline_tree_tools.rs` |
| 22.5 | **Goals, context and prompts**: `event_occurred` in the evaluator and validation; `memory.counts.event_planned`; the planned and anchor markers in results and `block.goals`; `block.canon_systems`; `conception.system`, `archivist.system`, the seed, maintenance and re-sync tasks, `tools.toml`; variables, samples, validator. | 06 §4.4, §6–§8, §10.3, §11.3; 08 §5; 09 §2 | plan §5.4; `prompts_registry` snapshots; each `kickoff_baseline` change justified |
| 22.6 | **Web**: the timeline tree, row menu and dialog fields; foreshadowing anchors and picker; the **Event happened** condition; both locales. | 11 §5.5, §9; PROJECT §8.8 | plan §5.5 Vitest; `i18n:check` |
| 22.7 | **Smoke and pre-ship**: smoke item 17 with the Demo provider. Run `0013` on a **copy** of the author's database; report the guard counts, the root id, that the 3 rows are top level and `occurred`, and that the 12 foreshadowing rows are unanchored with their text unchanged. | PLAN §M11.5 | the checklist passes; the report is in the final message and in D-170's consequences |

## M23 — Context view

Plan: `docs/research/context-treemap-plan.md` (D-171). It follows M22 and covers the session kinds
and tools of M21 and M22. Execute in order, one commit per task.

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 23.1 | **Docs**: done with the plan (D-171). Re-read the amended sections; fix any a code fact contradicts and note it in D-171. | plan §7 | `check.sh` green (docs only) |
| 23.2 | **Block sources and one estimator**: `StoredBlock::Text.source`, set by both kickoff loops (block key) and by `text_blocks` (`edit_report`, `inbox.<kind>`); `None` everywhere else; never sent; binding. Re-export `nekowriter_llm::estimate_tokens`; `kickoff_too_large` uses it and drops its private `is_cjk`. | 02 §4; 03 §10; 04 §2 | plan §5.1; `kickoff_baseline` unchanged |
| 23.3 | **Breakdown and API**: `agent/breakdown.rs` (pure: rebuild, classification, appendices, replay rule, scale, caps, `rollover_at`), the loader (ordered pool reads, no transaction), `rollover_eligible`, `GET /sessions/:id/context`, the DTOs, `SessionDto.context_window`, bindings. | 03 §13.1; 10 §2.7, §2.8 | plan §5.2 `context_breakdown.rs`; spec/03 §15 #36 |
| 23.4 | **Web**: `lib/treemap.ts` (squarified); `ContextButton`, `ContextDialog`, `RequestStrip`, `ContextTreemap`, the list view; the footer, studio and research-page entries; the origin tokens; `keys.sessions.context` and the SSE invalidation; both locales. | 11 §2, §4.2, §5.3, §5.4, §7, §9; PROJECT §6.19, §8.6 | plan §5.3 Vitest; `i18n:check` |
| 23.5 | **Smoke**: smoke item 18 with the Demo provider. | PLAN §M11.5 | the checklist passes |

## M24 — Context subdivisions

Plan: `docs/research/context-subdivisions-plan.md` (D-172). It follows M23 and only subdivides the
kickoff; tool results stay one part per tool.

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 24.1 | **Docs**: done with the plan (D-172). | plan §3 | `check.sh` green (docs only) |
| 24.2 | **Prompt pack and assembly**: the eight sub-block keys, `kickoff_blocks`, validator and samples. The concatenation of the sub-blocks equals today's parent rendering byte for byte. | 03 §10; 09 §2 | plan §2; `kickoff_baseline` diff per kind |
| 24.3 | **Context labels**: `agent:context.block.*` for the eight keys, both locales; the treemap shows them. | 11 §5.3 | Vitest; `i18n:check` |
| 24.4 | **Smoke**: smoke item 19 on a write session, compared with the sidebar. | PLAN §M11.5 | screenshots; D-172 consequences |
