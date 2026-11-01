# Implementation Plan: Codebase Cleanup

> Milestone **M16** in `docs/PLAN.md`, decision **D-159**. Read `AGENTS.md` and `NOTES.md` first.
> The spec sections listed in §8 were amended together with this plan.

## 0. Goal and rules

The goal is to remove what nothing uses: unused dependencies, functions nobody calls, the finished
compatibility branch for pre-split planning sessions, prompt-pack and error-code leftovers, unused
web files and exports, and template leftovers. The codebase should get smaller without any
change to product behavior.

Rules for every task:

1. **No behavior change.** These stay byte for byte:
   - the kickoff baseline snapshots (`kickoff_baseline.rs`);
   - every prompt that is kept;
   - every API response, except that the error code `embedding.unavailable` disappears (it is
     never returned);
   - the database schema. There is no migration.
2. **Prove before deleting.** The inventory in §2 is a snapshot taken at commit `2880a77` plus
   the M15 docs. Re-run the finders in §4 when the task starts, and delete only what they still
   report. Anything new that they find, the task cleans as well, and its commit message names it.
3. **Historical rows keep decoding and rendering.** An enum value or a stored shape that old rows
   carry stays, even when no runtime path produces it any more (§3.4).
4. **A gap is not dead code.** A function or string that a spec'd feature should use, but whose UI
   was never built, stays. It is listed in §3, not deleted.
5. One commit per task (`refactor: …`, `chore: …`, `docs: …`), with `scripts/check.sh` green
   before each commit.
6. **Order**: after M15, following the PLAN order. If M15 has not started, M16 may run first, but
   never interleave the two milestones.

## 1. Decisions at a glance

| # | Decision |
|---|---|
| C1 | Remove the ten unused dependency entries (§2.1) and the workspace entries no crate uses any more. Remove the 20 redundant `[[test]]` sections of `crates/server/Cargo.toml`, which Cargo's auto-discovery already covers. |
| C2 | Delete the 33 Rust functions that have no reference anywhere in `crates/` (§2.3). |
| C3 | Delete the legacy planning branch (D-146). Its removal condition (the §2.12 query of the session-split plan returns 0 on the author's database) was met at task 12.7 and is re-checked first (§5, 16.4). `Phase::Planning` stays as a historical enum value. |
| C4 | Delete the prompt variable `previous_outline` (documented and sampled, used by no template), and the error code `embedding.unavailable` (never returned, because embedding failures degrade to keyword search, spec/07 §4). |
| C5 | Web: delete four unused files, the dead exports, `src/assets/react.svg` and the Vite template `web/README.md`, plus the i18n keys proven unused. Keep exports used inside their own file or by tests. |
| C6 | Fix the spec facts the survey found stale: the dependency table (spec/01 §2), the `EntityPicker` reference (spec/11 §5.5), and the description of `writer.notice.retrieval_budget` as legacy (spec/09 §2). |
| C7 | **Do not delete; these are planned elsewhere**: the six unused hooks that back missing features are M18 (`docs/research/feature-gaps-plan.md`, D-161), and the unrendered `notice.user_message` is M17 (`docs/research/steering-header-fix-plan.md`, D-160) (§3). |

## 2. Inventory (snapshot)

### 2.1 Rust dependencies with no use

`grep` finds no `use`, path or macro reference to these:

| Crate | Section | Dependency |
|---|---|---|
| `nekowriter` (server) | `[dependencies]` | `tower-http`, `tokio-stream`, `eventsource-stream`, `ipnet` |
| `nekowriter` | `[dev-dependencies]` | `pretty_assertions` |
| `nekowriter-llm` | `[dependencies]` | `tokio-util`, `async-trait` |
| `nekowriter-llm` | `[dev-dependencies]` | `tokio-util`, `tempfile` |
| `nekowriter-embed` | `[dependencies]` | `serde_json` |
| `nekowriter-migration` | `[dependencies]` | `tokio` |
| `nekowriter-migration` | `[dev-dependencies]` | `tempfile` |

- After those removals, delete the `[workspace.dependencies]` entries that no member uses any
  more. Expected: `tower-http`, `tokio-stream` and `ipnet`. `eventsource-stream`, `tokio-util`,
  `async-trait`, `pretty_assertions` and `tempfile` are still used elsewhere.
- `nekowriter-migration` declares `async-trait = "0.1"` directly. Switch it to
  `async-trait.workspace = true`.
- **Keep** `libsqlite3-sys` (`db/mod.rs` calls `sqlite3_auto_extension` through it) and
  `@types/node` (named in `tsconfig.node.json`).
- The 20 `[[test]]` sections (`api_health` … `research_tasks`) name the same file and the same
  name auto-discovery gives them; `autotests` is not disabled.

### 2.2 Spec facts made stale by these dependencies (already amended)
- spec/01 §2 listed `tower-http` and `tokio-stream` (`BroadcastStream`): the SSE endpoint uses
  `futures::stream::unfold`.
- It listed `ipnet`: the SSRF guard (`web/fetch.rs` `is_blocked_ip`) uses `std::net`.
- It said `rust-embed` embeds the prompts: the pack uses `include_str!` (`prompts/pack.rs`).

### 2.3 Rust functions with no reference

These are 33 functions whose name occurs exactly once in `crates/` (their definition), in source
or tests, excluding trait impls (21 rows in the table below):

| File | Functions |
|---|---|
| `agent/context/mod.rs` | `completion_version_kind`, `freezes_tool`, `is_author_kind`, `latest_version_kind`, `setting_docs_map`, `unresolved_calls`, `version_kind_label` (a cluster "re-exported for the milestones") |
| `agent/manager.rs` | `clear_session_error` |
| `agent/report.rs` | `paragraph_contexts` |
| `agent/seal.rs` | `push_appendix` |
| `agent/session.rs` | `clear_error` |
| `agent/tools/conception.rs` | `open_conception_session` |
| `agent/tools/registry.rs` | `is_writer_granularity` |
| `agent/tools/resolve.rs` | `render_error` |
| `agent/tools/writer.rs` | `granularity_error`, `resolver_for` |
| `api/dto/canon.rs` | `…::change_actor` (inherent method) |
| `db/json.rs` | `CanonSystems::all_on` |
| `db/mod.rs` | `Db::from_connection` |
| `services/canon/entity.rs` | `_join_tables` (an "unused-import guard" under `#[allow(dead_code)]`; delete it **and** the import it guards if nothing else uses it) |
| `services/canon/mod.rs` | `chapter_label_bare`, `prompts_for` |
| `services/change_log.rs` | `changed_at`, `snapshot_json` |
| `services/chapters.rs` | `paragraph_texts`, `record_summary_change`, `track_is_main`, `version_content` |
| `services/conception.rs` | `has_main_track` |
| `services/providers.rs` | `model_count` |
| `web/fetch.rs` | `poll_resolving` (`#[allow(dead_code)]`, "used only by the resolver unit test", which does not call it) |
| `web/mod.rs` | `…::search_reason` |
| `web/search.rs` | `hits_from_value` |

After deleting a function, delete the imports, types and constants that only it used. The
compiler's `unused` warnings point at them, and clippy `-D warnings` keeps them from staying.

### 2.4 The legacy planning branch (D-146)

**What it was for.** Before the outline split, a `chapter_write` session started in `planning`.
D-146 kept a marked branch so that sessions *open* at the upgrade could finish. It was to be
deleted "with its keys and its test" once this query returns 0 on the author's database:

```sql
SELECT count(*) FROM agent_sessions
WHERE kind = 'chapter_write' AND phase = 'planning'
  AND status NOT IN ('completed', 'superseded', 'abandoned');
```

Task 12.7 recorded **0**. New code never creates such a session, so the count cannot have grown
on that database.

**Delete:**

| Area | What |
|---|---|
| `agent/tools/registry.rs` | `LEGACY_WRITER_TOOLS`; the `Phase::Planning` arm of `is_permitted`, which joins `Phase::None => false`; the registry unit tests that use `legacy_writer_frozen()` |
| `services/sessions.rs` | the `CreateSession` fields `legacy_planning_fixture`, `legacy_system_prompt`, `legacy_kickoff_text` and every branch reading them (≈ 195, 328–344, 406, 497); the `false, None, None` triple at every call site (`chapters.rs`, `conception.rs`, `research.rs`, `sessions.rs`, and any M15 call site); the `chapter_write` branch of `rejected_notice` |
| `agent/tools/writer.rs` | the legacy branch of `submit_outline` (≈ 346: immediate write plus `phase_to: drafting` for `chapter_write`) |
| `agent/runner.rs` | the `"submit_outline"` arm of `phase_notice`, keeping `submit_revision_plan`; the `(ChapterWrite, Planning)` arm of `nudge_key` |
| `agent/seal.rs` | the doc-comment example naming `writer.notice.outline_accepted` |
| `agent/tools/test_tools.rs` | the `"planning"` phase option, if only legacy tests use it |
| Demo provider (`crates/llm/src/demo.rs` ≈ 325–330) | the `chapter_write`/`planning` dispatch. Keep the outline generator `chapter_outline` uses, renamed `chapter_outline_plan` |
| Prompt pack | `writer.notice.outline_accepted`, `writer.notice.outline_rejected`, `writer.notice.nudge_planning` (in `notices.toml`, `scripts/validate_prompts.py` lists and samples, `prompts_registry.rs` key lists and tests). **Keep `writer.notice.retrieval_budget`**: `runner.rs` ≈ 1481 renders it for every non-planner session over the retrieval budget |
| Web | the `legacyHeld` / `continueLegacy` path of `features/chapter/OutlineTab.tsx` and its test "keeps the legacy path…", plus the i18n keys only it used (§4.3 finds them) |
| Tests | `legacy_planning_session.rs`; `tests/fixtures/legacy_writer_kickoff.txt`, `legacy_writer_system.txt`; `start_legacy_writer`, `start_legacy_writer_on` and the `legacy` flag of `create_writer` in `agent_write_harness/mod.rs`; their uses in `agent_write_holds.rs` (5), `agent_write_tools.rs` (4), `agent_write_rollover.rs` (2), `agent_review_granularity.rs` (2), and the `Phase::Planning` / `LEGACY_WRITER_TOOLS` cases in `agent_tools.rs`, `search_tools.rs`, `canon_tools.rs` (`let _ = Phase::Planning;`) |

**Each legacy test case gets one decision**, recorded in the commit message:
- **delete** it when its subject is the legacy branch itself (an immediate outline write, the
  writer acceptance or rejection notice, the planning nudge, the 20-tool list);
- **port** it to an outline or drafting session when it uses a legacy session only as a vehicle
  for a general mechanism (holds, auto-pause, granularity, rollover, edit reports, the
  open-tail notice) and no existing test covers that mechanism. The candidates for porting are
  `outline_session.rs`, `write_session_split.rs` and the drafting harness.

  spec/03 §15 test 25 keeps its rollover half ("a plain drafting rollover has no redraft
  sentence; a redraft rollover keeps its chain's feedback"). If only a legacy test proved that,
  port it.

**Keep** (§3.4): `Phase::Planning` and its CHECK value, the `chapter_write` kind, the
`submit_outline` tool renderer in the web (historical transcripts), and the `phase_to` effect
field.

**Overrides of the removed keys.** D-146 made overrides of the retired `writer.kickoff.redraft`
inert. Confirm the registry treats an override of any unknown key the same way: it loads, it is
not rendered, and startup does not fail. Pin that with a test that inserts an override for
`writer.notice.outline_accepted` and starts the app. The Prompts page must not crash on such a row
either: hide it, or list it as inert, whichever the current code does for `writer.kickoff.redraft`.

### 2.5 Prompt pack and error code
- `previous_outline`: `variables.toml` documents it (`[vars.previous_outline]`),
  `prompts/samples.json` and `validate_prompts.py` sample it, and no template's `vars` lists it.
  It is a leftover of the retired `writer.kickoff.redraft`. Delete all three.
- `embedding.unavailable`:
  - remove it from `ErrorCode` (`error.rs`: the variant, `as_str`, the status mapping);
  - remove it from both `errors.json` locales and `web/src/i18n/locales.test.ts`'s list;
  - remove it from spec/10 §4 (already amended);
  - regenerate the bindings. The `ErrorCode` union loses one member, and that is the only expected
    `web/src/api/gen` diff of M16.

### 2.6 Web
- **Files no module imports**:
  - `components/common/EntityPicker.tsx`, superseded by `features/canon/EntityPicker.tsx`, which
    the goal dialog uses;
  - `components/ui/kbd.tsx`, `components/ui/separator.tsx`, `components/ui/sheet.tsx` (generated
    shadcn components that nothing composes; the CLI can add them again);
  - `src/assets/react.svg` (Vite template);
  - `web/README.md` (Vite and shadcn template text that tells the reader to run `npx`, which is
    not installed here). Replace it with three lines that point to the root `README.md`, or delete
    it.
- **Exports with no use at all**, not even inside their own file:
  - `api/endpoints/canon.ts`: `chapterAppearances`, `putAppearances`;
  - `api/endpoints/sessions.ts`: `isOpenToolCall`;
  - `api/events.ts`: type `EventPayload`;
  - `api/hooks/books.ts`: `apiError`;
  - `api/keys.ts`: type `QueryKeys`;
  - `lib/languages.ts`: type `WritingLanguage`.

  The server endpoints behind the two appearance functions stay; they are API surface (spec/10).
- **i18n keys without a literal or dynamic-prefix use** (23):
  - `book:agent_panel`;
  - `canon:event_ref`, `canon:foreshadowing_ref`,
    `canon:proposal.goal_node`, `canon:proposal.new_goal_task`,
    `canon:proposal.goal_status_change`, `canon:proposal.new_goal_task_parent`;
  - `common:back`, `common:add`, `common:more`;
  - `knowledge:chunks`, `knowledge:embed_state.pending`, `knowledge:embed_state.done`,
    `knowledge:embed_state.failed`, `knowledge:delete_scope_confirm`,
    `knowledge:research.run_running`, `knowledge:research.defer_blocked`,
    `knowledge:research.status`, `knowledge:research.language_required`;
  - `library:chapters_count`;
  - `settings:appearance_theme`, `settings:search_test_results_plural`.

  `canon:relation_status.active` was also reported, but **stays**: M18's relation edit (G4) uses
  it. The `canon:proposal.goal_*` keys are duplicates of the `book:proposal.*` keys that
  `ProposalCard` uses. `knowledge:delete_scope_confirm` was superseded by
  `delete_scope_typed_confirm`.

  For each key, search for a dynamic use the finder cannot see (a lookup table, a `keyPrefix`,
  string concatenation). Then:
  - delete it from **both** locales when it is unused;
  - keep it when a spec'd but unbuilt feature needs it, and add it to §3.

  Run the finder again after 16.4, because removing the legacy Outline-tab path frees more keys.
- **Not touched**: exports used only inside their own file (mostly `…Props` types and helper
  constants), exports used only by tests, the `cn` / `@/lib/utils` import split in generated
  components, and the favicon `public/vite.svg`, which `index.html` uses. Replacing it is
  branding, not cleanup.

## 3. Not cleanup — keep, and report to the author

### 3.1 Unused hooks that back missing features (→ M18)
These are unused because the UI that should call them was never built. Deleting them would hide
the gap. All six were verified and are closed by M18 (`docs/research/feature-gaps-plan.md`):

| Hook (`api/hooks`) | Gap | M18 |
|---|---|---|
| `useCreateEntity` | F7 "Create": `EntitiesPage` has no create action | G1 |
| `useMergeEntity` | F7 "merge by hand": no merge UI | G2 |
| `usePatchTimelineEvent` | F7 "edit events": `TimelinePage` cannot edit an event | G3 |
| `usePatchRelation` | F7 edits relations: `RelationsEditor` adds and deletes only | G4 |
| `useEntityHistory` | §8.8 history: the detail shows only the latest 20 changes | G5 |
| `useOlderMessages` | spec/11 §5.3 scrollback: `AgentPanel`'s "Load older" button has no handler | G6 |

16.6 deletes none of them. If M16 runs after M18, they are in use anyway.

### 3.2 Bug: steering messages lose their header (→ M17)
- `notice.user_message` (【用户留言】 + text) is never rendered. Inbox `user_message` rows are
  appended raw:
  - into a tool result's appendices (`runner.rs` ≈ 1946, `seal::inbox_appendix(item.content)`);
  - or into a new user message (≈ 1976).
- But spec/03 §5.3 item 4 says steering messages are appended as `notice.user_message`, and
  `writer.system` tells the model that author messages arrive under that header. When the text is
  appended to a tool result, the model cannot tell it from tool output.
- This is a behavior fix, not cleanup. **Do not delete the key.** M17
  (`docs/research/steering-header-fix-plan.md`, D-160) renders it when the row is consumed.

### 3.3 Kept on purpose
- **The research and requirements documents** in `docs/research/` are referenced by D-entries and
  PLAN rows as the record of a decision.
- **PLAN's finished milestone tables** stay as history.
- **Domain terms that look like cleanup markers**: retired memory notes, retired writing skills,
  `notice.*` keys rendered inside `result.error`, system keys built as `format!("{role}.system")`.
- **Test-only debug tools** (`echo`, `test_write`, …) and the debug session endpoint (debug builds
  only).

### 3.4 Historical shapes that stay
- `Phase::Planning`, the value `planning` in the `agent_sessions.phase` CHECK, and the
  `chapter_write` kind, because historical rows decode strictly (D-151).
- The `submit_outline` tool renderer, because old transcripts render it.
- `ToolEffects.phase_to`, which `submit_revision_plan` uses.

## 4. Finders (re-run at the start of each task)

Run them from the repository root, write output only under the scratchpad or `target/tmp`, and
add no script to the repository.

### 4.1 Rust dependencies
For each `[dependencies]` / `[dev-dependencies]` entry of a crate, search
`crates/<crate>/src` and `tests` for its Rust identifier (`-` → `_`), as a path, a `use` or an
attribute macro. Zero hits means the entry is a candidate. Proof: `cargo build --workspace
--all-targets` and `cargo build -p nekowriter --release --features embed-web` both pass after the
removal.

### 4.2 Rust functions
```python
# python3 -I, from the repository root: names defined with `fn` in crates/*/src that occur once
import os, re, collections
files = [os.path.join(r, f) for r, _, fs in os.walk("crates") for f in fs if f.endswith(".rs")]
text = {p: open(p, encoding="utf-8").read() for p in files}
count = collections.Counter(re.findall(r"\b[A-Za-z_]\w*\b", "\n".join(text.values())))
fn = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:const\s+)?fn\s+([A-Za-z_]\w*)", re.M)
for p, t in sorted(text.items()):
    if "/tests/" in p:
        continue
    for m in fn.finditer(t):
        if count[m.group(1)] == 1:
            print(p, m.group(1))
```
Ignore `#[test]` functions and trait-impl methods (`array_type`, `column_type`, …) in the output.

### 4.3 Web files, exports, i18n keys
- **Files**: resolve every `import` / `export … from` / `import()` specifier (`@/` → `src/`,
  relative paths, `.ts`/`.tsx`/`index`), and list the non-test, non-`gen` files that no non-test
  file imports.
- **Exports**: for each `export (function|const|type|interface|class)` outside `gen/` and
  `components/ui/`, list the names that occur in no other non-test file **and** only once in their
  own file.
- **i18n keys**: flatten `locales/en/*.json`, strip plural suffixes (`_one`, `_other`, …), and
  list the keys that occur neither as `ns:key` nor as a bare `"key"` in `src/**/*.ts(x)` and that
  match no dynamic prefix (`` `ns:prefix.${ `` or `"ns:prefix." +`).

## 5. Tasks — milestone M16

| # | Task | Tests / acceptance |
|---|---|---|
| 16.1 | **Docs** — done with this plan (D-159, §8). Re-read the amended sections before 16.2. | `check.sh` green (docs only) |
| 16.2 | **Dependencies and manifests** (§2.1). | §4.1 finds nothing; both builds of §4.1 pass; `cargo tree -d` shows no new duplicate; `check.sh` green |
| 16.3 | **Rust functions** (§2.3) and whatever 16.2 left unused. | §4.2 finds no non-test, non-trait function; no `#[allow(dead_code)]` is left in `crates/*/src`; `check.sh` green; `kickoff_baseline` unchanged |
| 16.4 | **The legacy planning branch** (§2.4). **First**, on a **copy** of the author's database (`~/.local/share/nekowriter/nekowriter.db`, as in 12.7), run the §2.4 query and count `prompt_overrides` rows for the three removed keys. If the query is **not 0**, stop: skip 16.4, revert this plan's legacy amendments in spec/02, 03, 04, 05, 09, 11 and PROJECT (§8), and report. | `grep -rn 'legacy_planning\|LEGACY_WRITER\|Phase::Planning\|nudge_planning\|outline_accepted\|writer.notice.outline_rejected\|legacyHeld' crates web/src scripts prompts` matches only the kept historical decode paths (§3.4) and D-146-era migration tests; every legacy test case is deleted or ported (listed in the commit message); the unknown-key override test passes; `prompts_registry` snapshot differs only by the three removed keys; `kickoff_baseline` unchanged; Vitest `outlineTab.test.tsx` passes without the legacy case; `check.sh` green |
| 16.5 | **Prompt variable and error code** (§2.5). | `validate_prompts.py` passes; the `web/src/api/gen` diff is the one `ErrorCode` member; `locales.test.ts` passes; `check.sh` green |
| 16.6 | **Web** (§2.6): files, exports, template leftovers, and i18n keys (after 16.4). The §3.1 hooks stay. | §4.3 finds only what §2.6 and §3 keep; `pnpm --dir web typecheck`, `lint`, `test --run`, `i18n:check`, `build` pass; the production bundle is not larger |
| 16.7 | **Sweep and report**: grep `NOTES.md`, `README.md`, `docs/` and `AGENTS.md` for every removed name and fix the stale mentions; re-run all finders; append the counts (dependencies, functions, files, keys, test cases deleted or ported) to D-159's consequences. | the final message carries the counts |

## 6. Acceptance (whole milestone)
- `scripts/check.sh` is green after every task.
- `kickoff_baseline` snapshots are unchanged.
- The `prompts_registry` snapshot loses only the three legacy keys.
- The `web/src/api/gen` diff is one `ErrorCode` member.
- No migration file is added. A historical `chapter_write`/`planning` row (fixture) still loads
  through `GET /api/sessions/:id` and its transcript renders.
- Smoke items 3–5 (PLAN §M11.5) pass with the Demo provider, since the outline, write and review
  loop is what 16.4 touches.

## 7. Risks

| Risk | Mitigation |
|---|---|
| A legacy test was the only proof of a general mechanism | The delete-or-port rule in §2.4, with each decision in the commit message |
| An author override of a removed key breaks startup | The unknown-key test in §2.4; the pre-check counts such rows |
| Removing a dependency drops a feature another crate relied on through unification | Both builds in §4.1, including the release build with `embed-web` |
| A "dead" i18n key is used dynamically | Look for a dynamic use per key before deleting it; `i18n:check` and Vitest catch a missing one |
| A "dead" hook is a missing feature | §3.1 keeps all six (M18 builds their features) |

## 8. Documents amended with this plan
- **spec/01 §2**: the dependency table (no `tower-http`, `tokio-stream` or `ipnet`;
  `eventsource-stream` is `nekowriter-llm` only; `rust-embed` is web dist only).
- **spec/02 §3**: `Phase` `planning` is historical only; §4 the `phase_to` comment.
- **spec/03**:
  - §1.1: no legacy row in the table; `planning` is historical;
  - §1.2: the historical 20-tool list;
  - §4: initial fields;
  - §5.4: revoke notices;
  - §5.6: nudge keys;
  - §15: test 2 and test 25.
- **spec/04 §7.2**: no `chapter_write`/`planning` Demo row.
- **spec/05**: §1.5 (`staged`); §2 the planning row and the `submit_outline` note.
- **spec/09 §2**: the three keys removed; `writer.notice.retrieval_budget` described as in use;
  overrides of removed keys are inert.
- **spec/10 §4**: no `embedding.unavailable`.
- **spec/11**: §5.2 the Outline tab without the legacy path; §5.5 the `EntityPicker` reference.
- **PROJECT**: §4 Phase; §8.5 no legacy Outline-tab bullet.
- **DECISIONS**: D-159. **PLAN**: the M16 table.

## 9. Out of scope
- Restructuring the test suite (about 120 integration-test binaries, or merging the review-round
  files `m6_*` … `m11_*`, `r11_*`). That would be a build-time project of its own.
- Fixing §3.2 (M17) or building the §3.1 features (M18).
- Narrowing `pub` to `pub(crate)` across the server crate, or dropping `export` from symbols used
  only in their own file.
- Local disk cleanup (`target/`, `target/tmp`). That is the author's machine, not the repository.
- Changing the API surface. Endpoints that the web does not call stay, because they are
  documented in spec/10.
