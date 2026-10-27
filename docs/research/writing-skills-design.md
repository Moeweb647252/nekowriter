# Writing Skills and Recent-Chapter Context — Design (rev. 3)

Status: **locked for implementation** (revision 4).
Revision 4 records the author's final ruling on the last open decision: **option 2 — the prose
bullets are removed from `writer.system` now**, accepting that a book has no prose rules until the
author writes and selects their own skill. The upgrade concern is moot: there are no existing books
(two throwaway test books, zero prompt overrides). Built-in skills are **read-only** and their bodies
ship **empty**; the author creates their own.

---

## 1. What the product is

NekoWriter is a **local, single-user** web app for writing long Chinese web serials. One author, one
SQLite database. Rust (axum + SeaORM + SQLite) backend; React SPA frontend.

The author works with **agent sessions**: a tool-calling loop with a frozen prompt (system prompt,
kickoff messages and tool list fixed at start, so a later pack edit never changes a running or
historical session). Risky tool calls can be **held**; the author approves or rejects, and the call's
deferred effects apply only on approval.

Structural facts that matter here:

- **Canon** in SQLite: entities, relations, timeline, memory notes, foreshadowing, change log with
  revert. A per-chapter maintenance session keeps it current.
- **A knowledge base** with scopes and hybrid (vector + FTS) search.
- **A prompt pack** in `prompts/*.toml`, embedded at compile time: ~69 prompt entries, 48 tools.
  Model-facing text is Chinese. `scripts/validate_prompts.py` validates it; snapshot tests pin the
  rendered output.
- **Per-book and global prompt overrides** (`prompt_overrides`), so any prompt can be replaced
  without a code change.
- **Kickoff blocks**: a session's first user message is assembled from an ordered list of blocks.
  `assemble_kickoff` **drops any block whose render is blank**, so optional blocks cost nothing when
  empty.
- `BookSettings` is a JSON column on `books` deserialised with `#[serde(default)]`:
  `granularity`, `auto_pause`, `pause_after_outline`, `target_words_min`, `target_words_max`,
  `recent_summary_count`, `midterm_window`, `auto_maintenance`. **No style/voice field today.**

`writer.system` is **≈2.1 k characters (≈5.1 k UTF-8 bytes)** — the figure 5,138 sometimes quoted as
"characters" is the byte count. Its `# 正文写作要求` section has **nine bullets**: two workflow rules,
six prose rules, and a formatting bullet.

---

## 2. The requirement and the author's rulings

1. 「弄一个像 skills 那样注入上下文的 writing skills，与书库 知识库 提示词 设置并列」 — a writing
   skills surface, injected into context, as a top-level navigation item.
2. 「将最近 n 章正文注入上下文」 — inject the full text of the last N chapters.

Rulings (revision 3):

| topic | ruling |
|---|---|
| What a skill is | 「支撑整个写作能力的提示词」 — the prompt underpinning the writing capability |
| Selection | a book selects **0–n** skills |
| Fields | a **category** field and an **author** field; **no fixed categories** |
| Storage | **B** — bodies in the database; the prompt pack is the seed source for built-in rows |
| Injection | **full**, not on-demand |
| Built-in content | **bodies ship empty**; built-ins are **read-only** |
| User content | **the author can create their own skills** |
| Import/export | out of scope |
| System prompt | **Option A, applied now** — the six prose bullets leave `writer.system` in this change; the two workflow bullets and the two format bullets stay |

Confirmed by review: Option A is the only shape that fits the spec — system prompts must stay
**static and shared across books** (spec/09 §5.1), so anything a book selects has to live in a kickoff
block.

---

## 3. Goals and non-goals

**Goals**

- A per-book selection of writing skills, visible and editable by the author.
- The author can **write their own** skills and have them injected in full.
- A skill's full text reaches writer, reviser and reviewer on every session of that book.
- The writer sees the actual approved prose of the last N chapters.
- Both mechanisms are **model-agnostic**: nothing encodes a specific model's vocabulary tics.

**Non-goals**

- Import/export, version history, marketplaces, per-role skill selection.
- Built-in skills are read-only, including in the UI.
- Any deterministic "banned phrase" gate. The author rejected that approach explicitly.

---

## 4. Data model

```sql
CREATE TABLE writing_skills (
  id                 INTEGER PRIMARY KEY,
  key                TEXT UNIQUE,                -- built-in only; NULL for user rows
  name               TEXT NOT NULL,
  category           TEXT NOT NULL DEFAULT '',
  author             TEXT NOT NULL,              -- 'builtin' | 'user'; no default
  description        TEXT NOT NULL DEFAULT '',
  body               TEXT NOT NULL DEFAULT '',
  enabled_by_default INTEGER NOT NULL DEFAULT 0,
  retired            INTEGER NOT NULL DEFAULT 0,
  created_at         TEXT NOT NULL,
  updated_at         TEXT NOT NULL
);

CREATE TABLE book_writing_skills (
  book_id    INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  skill_id   INTEGER NOT NULL REFERENCES writing_skills(id) ON DELETE CASCADE,
  created_at TEXT NOT NULL,
  PRIMARY KEY (book_id, skill_id)
);
```

Notes, each from the review:

- **`key` is built-in-only and permanent.** `UNIQUE` on a nullable column lets user rows have `NULL`
  and makes it impossible for a user skill to collide with a later built-in key. Keys are never
  renamed or reused.
- **`author` has no default**, so a new row is never silently reconciler-owned.
- **`retired` instead of deletion for built-ins.** A built-in that disappears from the pack is marked
  `retired = 1` and stops being offered; existing selections keep working.
- **User rows are deletable**, and deleting one cascades away every book's selection of it. That is
  correct for a row the author created and explicitly deleted; the UI must confirm it and say which
  books use it.

### 4.1 Migration and upgrade path

The author confirms **there are no existing books** (only two throwaway test books, and zero
`prompt_overrides`). Therefore:

- The migration only creates the two tables. **No backfill**, no `writer.system` overrides to clean.
- New books select the skills whose `enabled_by_default = 1` at creation.
- If a developer's database happens to contain test books, they are expected to be discarded.

Revision 2's elaborate backfill (selecting the default skill for every book, stripping the section
out of stored `writer.system` overrides) is **not needed and is dropped**. It must not be implemented
"just in case" — it would be dead code.

### 4.2 Reconciler — deferred

**Do not build the startup reconciler in this change.** Built-in bodies are empty, so there is
nothing to reconcile; a startup mutation with no payoff is pure risk (it would run on every boot
against every database for no visible effect).

The `basic-writing` row is seeded **by the migration**, with an empty body. When a built-in body is
first shipped, that change must either seed it in its own migration or introduce the reconciler
described in the review (insert-if-missing, update only `author = 'builtin'` rows, `retired = 1` for
removed entries, never delete, never touch `book_writing_skills`, run inside `Db::write` after the
pack loads). Record this in the decision entry so the debt is visible.

---

## 5. The built-in skill

| field | value |
|---|---|
| `key` | `basic-writing` |
| `name` | 基础写作 / Basic Writing |
| `category` | 写作 / Writing |
| `author` | `builtin` |
| `body` | **empty** |
| `enabled_by_default` | `false` (it has no content, so selecting it would do nothing) |

### 5.1 What happens to `# 正文写作要求` in this change

Per the author's ruling (option 2), the six **style** bullets are deleted now:

- 场景化叙事
- 人物要有区分度
- 节奏张弛有度
- 视角统一
- 语言干净
- 用中文写作时…**以短段为主（通常一到四句）** — style, so it leaves the system prompt entirely

These **stay in `writer.system`** because they are workflow, not style — unticking a skill must
never stop the writer being told to follow the outline:

- 先想清楚再写… (a PROJECT design principle)
- 遵循章纲，可以为了叙事自然调整细节，但不得违背参考要点与既有设定。

These **stay in `writer.system`** because they are format, not style:

- the Chinese quotation marks / no Markdown / no headings part of the Chinese-writing bullet
- the no-metadata rule

**Accepted consequence:** until the author writes and selects a skill, a book has **no prose style
rules at all**. That is deliberate and must be stated in the decision entry, so it is a known
property rather than a surprise.

---

## 6. Context injection

### 6.1 Block order and what the cache actually does

```
block.settings          (setting documents — the only block with a cache breakpoint)
block.writing_skills    (NEW — full text of selected skills)
block.memory            (tiered memory digest)
block.recent_chapters   (NEW — full text of the last N chapters)
writer.kickoff.chapter  (the task)
```

**Only `block.settings` carries a cache breakpoint** — `block_is_cached` matches that one key, and
the Anthropic path puts `cache_control` on that block plus a top-level automatic breakpoint
(spec/04 §3 uses 3 of the 4 allowed breakpoints). A block after `block.settings` is **not** in the
cached prefix. There is no "paid once per book" effect; caches are keyed on content and never serve
stale content. The placement is right for **stable-first ordering** (spec/03 §10), not for caching.
Do not add a fourth breakpoint for a few thousand tokens.

### 6.2 Which sessions get what

| session | writing skills | recent chapters |
|---|---|---|
| chapter write — draft | yes | yes |
| chapter write — redraft | yes | yes |
| chapter write — rollover | yes | **no** |
| chapter revise | yes | **no** |
| chapter review | yes | no |
| archivist (maintenance / resync / seed), conception, research | no | no |

Rollover and revise already carry the chapter's own text and `previous_tail` covers the seam. For a
rollover, a larger kickoff also worsens an existing risk: `rollover_needed` has no guard against a
kickoff that alone crosses the context threshold. The plan must state the full block order for
**every** session kind, including the separate rollover list.

### 6.3 Rendering and empty cases

- Skill bodies and chapter texts are wrapped in **XML-style tags** (spec/09 §5.8):
  `<writing_skill name="…" category="…">…</writing_skill>`,
  `<chapter_text number="…" title="…">…</chapter_text>`.
- If zero skills are selected — the normal case while built-ins are empty — the block renders blank
  and `assemble_kickoff` drops it. Same for recent chapters when none are eligible.
- The skill block states that a body is **instruction text about how to write, not a document about
  the book**, so it is never mistaken for canon.
- **Keep the chapter summaries.** They are condensed accounts, not copies of the text; dropping them
  would leave `### 近期章节摘要` as an unconditional heading with nothing under it. Only
  `previous_tail` is deduplicated.

### 6.4 Budget

- Skills: the selection UI shows `n selected · x characters`; above 8,000 characters warn, do not
  block, do not truncate.
- Recent chapters: `BookSettings.recent_chapter_count` (default **2**, `0` disables). The cap is
  expressed **relative to the chapter target**, not as a fixed character count — a 6,000-character
  cap already binds on a chapter 25–30 % over the default target.
- **A chapter over the cap is neither skipped nor truncated at the end.** Keep the ending, cut from
  the **front** at a paragraph boundary, and fill newest-first under one total budget.

---

## 7. Recent N chapters

Selection follows **the memory rule** (spec/06 §4.1):

1. The last N **official** chapters before the target chapter, ordered by `ord`.
2. Side tracks: the track's own official chapters, topped up from main-track official chapters up to
   the anchor — the same visibility rule memory applies.
3. **No top-up from non-official text.** A draft, an in-review chapter or a rejected version is not
   approved prose and memory knows nothing about it.
4. Never include the target chapter (automatic).
5. Skip chapters with empty text.

**Deduplication:** `previous_tail` is a strict subset of the injected text when its chapter is inside
the N, so it is dropped then. **Summaries are kept.**

**The block tells the model three things:**

- These chapters are there for **continuity** (events, scene state, voice).
- The **writing method takes precedence**; phrases from these chapters must not be reused.
- **These chapters are already included in full** — do not call `read_chapter` for them, or every
  draft pays a redundant call.

**Memory page.** PROJECT §6.5 promises the Memory page shows exactly what the writer receives;
`GET /books/:id/memory` returns the rendered blocks. The change must state which variant the page
renders and name the chapters injected in full.

---

## 8. API and UI

**API**

- `GET /api/writing-skills?book_id=` → catalogue with `selected` per book.
- `PUT /api/books/:id/writing-skills` → replace the book's selection.
- `POST /api/writing-skills` → create a **user** skill (`author` forced to `user`; `key` stays
  `NULL`).
- `PATCH /api/writing-skills/:id`, `DELETE /api/writing-skills/:id` → **user rows only**; a built-in
  returns `409` / `403` (pick one and justify). `DELETE` requires confirmation naming the books that
  use it.
- Do **not** expose a way to create, edit or delete built-in rows.

**UI**

- A top-level page has no "current book", so follow the Knowledge and Prompts pattern: `/skills` is
  the **catalogue** (list, create, edit and delete user skills; preview built-ins); **selection
  lives in book settings**, where the row is the checkbox list.
- Built-in cards show a `Built-in` badge and no edit affordance.
- Book settings page: the skills checkbox list, an "applies to new sessions" note, and the
  `recent_chapter_count` input.

---

## 9. The two pre-existing prompt defects

1. **The contradiction.** `prompts/tasks.toml`, archivist maintenance step 2 lists 「描述发生变化」
   as a routine `update_entity` reason, one line above the rule that the description is stable.
   Remove 描述 and **strengthen the guard** (the D-142 guard only checks that 「120 字」 appears).
2. **The always-true condition — fix in Rust, not in the template.** `variables.toml` documents the
   rollover `feedback` variable as "the feedback the session addresses, **or none**", and the samples
   use `null`; the runtime passes a `{text, issues}` struct unconditionally, so `{% if feedback %}`
   is always truthy and an ordinary rollover prints a dangling heading. **Fix:** pass `null` unless
   the latest feedback row's `session_id` is in this session's chain. This also repairs existing
   `writer.kickoff.rollover` overrides, which a template fix would not. The test must build a real
   rollover session — the sample-driven test passes today, which is why CI missed it.

---

## 10. Cost

With the built-in body empty, the only cost in this change is the recent chapters, N = 2:

| item | characters |
|---|---|
| last 2 official chapters | +7,000 |
| removed: `previous_tail` | −800 |
| **net** | **≈ +6,200 characters ≈ 5,700 tokens** |

Summaries are kept (§6.3), so there is no offset from them. A skill body the author writes adds its
own length on top; the selection UI shows the running total.

---

## 11. Tests

- **Schema**: both tables exist; cascades work; a user row round-trips; a second row with the same
  `key` is rejected; a user row with `key = NULL` is allowed more than once.
- **Built-in protection**: `PATCH`/`DELETE` on a built-in is refused; the UI has no edit affordance.
- **Selection**: 0 skills is valid and renders no block; `enabled_by_default` applies to a new book;
  book A's selection never leaks into book B.
- **Rendering**: block order per session kind (including the rollover list); XML tags present; empty
  cases produce no heading; the skill block says it is instruction text.
- **Recent chapters**: matches the memory rule (official only, same-track plus main-track anchor);
  target excluded; front-cut at a paragraph boundary keeping the ending; newest-first under one
  budget; the block contains the three guidance lines; `previous_tail` dropped; summaries kept.
- **Defects**: each of §9 with a failure demonstrated before the fix; the rollover fix exercised
  through a **real session**.
- **Prompt pack**: both block keys added to spec/09 §2, `variables.toml`, the Rust context structs,
  samples and the validator; snapshot fails first, then updates.

---

## 12. Risks

| risk | mitigation |
|---|---|
| Context bloat once several skills exist | running character total; >8,000 warns; no truncation |
| Recent chapters crowd out the task | N configurable to 0; relative cap keeping the ending |
| A retired skill silently removed from a book | `retired` only hides it from the catalogue; selections keep working |
| Deleting a user skill removes it from books | confirmation names the books that use it |
| Rollover blow-up | recent chapters excluded from rollover; order stated per session kind |

---

## 13. Resolved from earlier reviews

1. **Skills block placement** — right after settings, for stable-first ordering, not caching.
2. **Removing the prose section** — deferred, because built-in bodies ship empty (§5, §14).
3. **Reviewer gets skills** — yes; otherwise it grades without the standard the writer had and sends
   contradictory feedback. The reviewer prompt must not grade checklist-style.
4. **Chapters over the cap** — keep the ending, cut from the front at a paragraph boundary.
5. **Database as truth** — the reconciler suits it; built-ins are read-only so there is no
   customization to preserve.
6. **Duplication** — `read_chapter` already returns prior prose, so the block's value is that it is
   unconditional and carries the voice signal; `previous_tail` is a strict subset; summaries are not
   duplicates.

---

## 14. Decisions resolved

**The prose section is removed now (author's option 2).** The six style bullets leave
`writer.system`; the two workflow bullets and the two format bullets stay (§5.1). The accepted
consequence — a book has no prose style rules until the author writes and selects a skill — must be
recorded in the decision entry.

**Built-in bodies are empty and read-only; user skills are author-created.** The reconciler is
deferred until a built-in body exists (§4.2).

This document is locked; implementation follows.
