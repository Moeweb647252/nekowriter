# Requirements: Split "Writer — Write Chapter" into Two Sessions

**Purpose of this document.** These are the requirements for a change to NekoWriter. An engineer or
an AI assistant should be able to read this and produce an implementation plan without further
questions about intent. It states the current behaviour, what must become true, the constraints the
plan must respect, and the questions the plan must answer.

Status: **proposal, not implemented.** No code has been written for this.

---

## 1. Product context

NekoWriter is a **local, single-user** web app for writing long Chinese web serials. One author, one
SQLite database. Rust (axum + SeaORM + SQLite) backend; React SPA frontend.

The author works with **agent sessions**. A session is a tool-calling loop with a frozen prompt
(system prompt, kickoff messages and tool list are fixed when it starts, so a later prompt-pack edit
never changes a running or historical session). Risky tool calls can be **held**: the author approves
or rejects them, and the call's deferred effects apply only on approval.

Relevant vocabulary:

- **Chapter status**: `planned`, `drafting`, `in_review`, `revising`, `official`.
- **Session kind**: `conception`, `research`, `chapter_write`, `chapter_revise`, `chapter_review`,
  `chapter_maintenance`, `canon_seed`, `chapter_resync`.
- **Phase** (per session): `none`, `planning`, `drafting`, `revision_planning`, `revising`.
- **Agent role** (used for model routing): `conception`, `researcher`, `writer`, `reviewer`,
  `archivist`. Each role can be assigned a model profile globally and per book.

---

## 2. Current behaviour (verified against the code)

Today a chapter is written **in one session**.

1. A chapter is created with `status = planned` and an empty `outline`.
2. The author starts a write session. `POST /chapters/:id/write` creates an `agent_sessions` row
   with `kind = chapter_write`, `phase = planning`, and a frozen prompt and tool list.
3. The kickoff is assembled from three blocks in order: `block.settings` (the book's setting
   documents — the cache breakpoint), `block.memory` (tiered memory digest), and
   `writer.kickoff.chapter`.
4. `writer.kickoff.chapter` — whose pack title is 「写手 · 撰写章节」 ("Writer — write a chapter") —
   contains **only planning instructions**: research the chapter's facts, then call `submit_outline`
   with `outline`, `reference_notes` and optionally `title`.
5. `submit_outline` writes `chapters.outline`, `chapters.reference_notes` and, if the chapter has no
   title, `chapters.title`. Its effect sets the session's phase to `drafting`. **The session
   continues in the same conversation** — there is no new session, no new history.
6. If the session's `pause_after_outline` flag is set (or `auto_pause` is on and the tool is a write
   tool), the `submit_outline` call is **held**. The author approves it (effects applied) or rejects
   it (effects revoked, feedback appended).
7. Once the phase is `drafting`, the actual writing instructions arrive as **notices**
   (`notice.*` — the granularity rules: sentence / paragraph / free / chapter). There is no second
   kickoff.
8. The writer calls `insert_text`, `insert_paragraphs`, `rewrite_paragraph`, `delete_paragraphs`,
   `replace_text`, `finish_draft`. With `auto_pause` on, each write is held and its content is
   **staged**, applied only on approval.
9. `finish_draft` completes the session.
10. **At most one open writer session per chapter** is allowed; starting another abandons or is
    refused by the existing `open_writer_session` rule.
11. The overview's next-action engine emits `WriteNext` when no chapter is in progress,
    `ContinueChapter` when a chapter is `drafting` or `revising`, and `ReviewChapter` when a chapter
    is `in_review`.

The mismatch the author noticed: the prompt entry titled 「写手 · 撰写章节」 is entirely about
producing the **outline**, while the instructions for writing the prose are a separate group of
notices. One session, one history, one model, two quite different jobs.

---

## 3. The requirement

> 「写手.撰写章节应该跟 写手.撰写细纲分开」

**Split the writer's two jobs into two independent sessions:** one that produces the chapter outline,
and one that writes the chapter from it.

The author chose the structural split (not merely renaming), because they want:
- the outline to be a step that can be **redone on its own** without touching the prose or its
  history;
- the two steps to be **independently routable to different models** (planning wants a strong model;
  drafting can be cheaper and faster);
- the overview to show the author the next concrete step (write the outline / write the chapter).

---

## 4. Required behaviour

**R1 — Two session kinds.** There is a session for producing an outline and a session for writing
the prose, as distinct as `chapter_review` is from `chapter_write` today. Their pack titles must
match what they do (「撰写细纲」 and 「撰写章节」, English equivalents in the pack).

**R2 — The outline session.**
- Starts from a chapter that has no outline (or whose outline the author wants to replace).
- Its tools include the research/retrieval tools and `submit_outline`, and **must not** include the
  prose-writing tools.
- `submit_outline` remains the terminal act: on acceptance the outline is stored and the session
  ends (or finalises as its own completion kind). It must not continue into drafting.
- The author can review the outline (the existing hold/approve mechanism is the natural fit) and
  reject it with feedback, which keeps the session able to revise the outline.

**R3 — The write session.**
- Can start only when the chapter has an accepted, non-empty outline. If it has none, the system
  must say so rather than silently planning again.
- Its tools include the prose-writing tools and **must not** include `submit_outline`.
- It begins in the drafting phase; no planning phase and no second `submit_outline`.
- `finish_draft` remains its terminal act.

**R4 — Independent histories and accounting.** Each session has its own append-only message history,
its own frozen prompt, its own token/usage figures. Re-running the outline session must not rewrite
or truncate the write session's history, and vice versa.

**R5 — Independent model routing.** The author must be able to assign different model profiles to
the outline step and the writing step. Today both are the `writer` role. The plan must say how this
is achieved (a new role, or another mechanism) and what it changes in the two existing model-setting
screens and their stored settings.

**R6 — Concurrency rule.** At most one open session **of each kind** per chapter. An open outline
session must not block starting a write session for a chapter that already has an outline, and a
write session must not be startable while an outline session is open for that chapter.

**R7 — `pause_after_outline`.** The setting exists today to hold the outline for approval inside the
single session. With the split, the outline session ends on its own, so the flag's meaning changes.
The plan must state what happens to it: kept with a defined meaning, repurposed, or removed with a
migration/compatibility story. Author-facing settings must not be left describing behaviour that no
longer exists.

**R8 — Next-action engine.** From a chapter with no outline, the overview must offer "write the
outline"; from a chapter with an outline but no draft, "write the chapter"; the existing
`ContinueChapter`, `ReviewChapter`, `WriteNext`, `SyncCanon`, `SeedCanon`, `RunResearch`,
`ContinueConception` behaviours must remain correct. The plan must enumerate the new and changed
next-action kinds, and say how the engine distinguishes "no outline yet" from "outline exists".

**R9 — UI.**
- The session header and any session lists show the two kinds with distinct, accurate labels.
- Starting each session is reachable from where the author already works (the chapter page and/or the
  overview card); the plan must name the exact entry points and any gating.
- The existing draft-pane decision dock (for held prose writes) and the held-call review for
  `submit_outline` must keep working, with the outline hold now belonging to the outline session.
- The chapter page must make clear whether the outline is missing, awaiting approval, or accepted.

**R10 — Backwards compatibility.**
- Historical `chapter_write` sessions must remain readable, and their stored payloads must not need
  rewriting.
- A `chapter_write` session that is **open in `planning` phase** at the moment of the upgrade must
  either keep working to completion under its frozen prompt, or be migrated with a stated rule. The
  plan must choose one and say how.
- Chapters that already have an outline must not be forced through the outline step again.

**R11 — Prompt pack.** `writer.kickoff.chapter` currently mixes the two jobs. The plan must specify
the resulting pack entries (an outline kickoff, a writing kickoff, and what remains a notice), their
titles in both languages, and whether the drafting-granularity notices stay as notices or move into
the writing kickoff.

**R12 — Chapter status.** Define exactly when a chapter moves `planned → drafting` (on outline
acceptance, on write-session start, or elsewhere) and whether any new status is needed (e.g. an
"outline pending" state). If a new status is needed, say so and include the migration.

**R13 — Existing invariants that must not regress.** Staged prose writes applied only on approval;
append-only histories; one process-wide write lock; prompt freeze per session; the reviewer reads the
chapter text; canon maintenance runs after a chapter is official.

---

## 5. Constraints the plan must respect

- **No network in tests.** Use `wiremock`, `ScriptedProvider`, `FakeEmbedder`.
- **Every write goes through the existing `Db::write` transaction wrapper.**
- **Do not modify persisted messages or sealed tool results** (append-only history).
- **Model-facing text lives only in `prompts/` and is Chinese**; UI strings live only in the i18n
  files and must be added in **both** `en` and `zh-CN` in the same change.
- **English only** in code, identifiers, comments, logs, errors and commit messages.
- **Spec is authoritative.** `docs/PROJECT.md` §9.2 lists what is out of scope. If this change
  requires a spec amendment, the plan must name every document that has to change
  (`docs/spec/02` data model, `03` agent runtime, `05` tools, `06` canon, `08` workflows,
  `09` prompts, `10` API, `11` frontend).
- **A `docs/DECISIONS.md` entry** is required, appended (never rewritten), with a unique number.
- **Migrations are append-only after the first tagged release** — check whether one is needed and
  justify it.
- **`scripts/check.sh` must pass**, including prompt validation and binding generation if a DTO
  changes.

---

## 6. Out of scope

- Any change to how the outline or the prose is *judged* (review criteria, quality gates).
- Writing skills, style guides or a voice anchor — a separate, pending change that also touches the
  kickoff block list, and therefore must be **sequenced** with this one rather than developed in
  parallel.
- Multi-chapter or batch generation.
- New providers or model plumbing.

---

## 7. Acceptance criteria

1. From a freshly created chapter, an author can produce an outline in one session, approve it, and
   then start a second session that writes prose from it without re-planning and without
   `submit_outline` being available.
2. Re-running the outline session after prose exists does not destroy or rewrite the prose history;
   the plan states what happens to the existing outline and draft.
3. The two sessions have separate histories and separate token accounting.
4. The outline and the writing step can be assigned different model profiles, and the two existing
   model-setting screens reflect that without breaking existing stored settings.
5. Historical sessions still render; an open planning-phase session is handled by a stated rule.
6. The overview offers the right next action at each stage (no outline / outline approved / drafting
   / in review / official).
7. `scripts/check.sh` passes; the prompt snapshot and bindings are updated as needed; every new UI
   string exists in both locales.

---

## 8. Questions the plan must answer

1. **Model routing:** a new `AgentRole` (six instead of five) or reuse `writer` for both? What happens
   to existing role→model settings and to `books.model_overrides`?
2. **Chapter status while the outline is being produced:** stay `planned`, or introduce a status?
   What does the chapter list show the author?
3. **What ends the outline session** — `submit_outline` alone, an explicit `finish_outline` tool, or
   the acceptance of the held call? What is its completion kind?
4. **Can the author write prose with a manually authored outline** (no outline session)? Can they
   skip the step?
5. **Regenerating an outline after prose exists:** allowed (and then what happens to the draft), or
   refused with a conflict?
6. **`pause_after_outline`:** keep, reinterpret or remove — and what existing settings/rows need
   handling.
7. **Ordering with the pending writing-skills change:** both touch `kickoff_blocks` and the writer
   prompt. Which lands first, and does the other need rebasing?
8. **Backwards compatibility for open sessions:** the precise rule and how a session started before
   the upgrade finishes.
