# Plan: Thinking Effort Moves From the Model to the Role

> Milestone **M19** in `docs/PLAN.md`, decision **D-162**. The spec sections listed in §6 were
> amended together with this plan, so the spec is already the target. M19 depends on nothing
> after M14 and may run before or after M15–M18.

## 0. The author's decisions (2026-11-02)

| # | Question | Answer |
|---|---|---|
| Q1 | What does a role that chooses no effort use? | The **model's default level**. A model profile lists its levels and may mark one as the default. |
| Q2 | Can a book override the effort per role, as it overrides the model? | **Yes**. Each book override row has an effort select that follows the global setting by default. |
| Q3 | What level list do existing model profiles get? | **Only the level they use today.** The author adds more levels by hand. |

## 1. Today (verified at `43a62a4`)

- `model_profiles.options` (`ModelOptions`, spec/04 §5) carries two effort fields:
  - `reasoning_effort` (OpenAI-compatible): a free-form string, sent as `reasoning_effort` only
    when set (`crates/llm/src/openai.rs` ≈ 270);
  - `effort` (Anthropic): the closed set `low…max`, sent as `output_config.effort` on **every**
    request. It defaults to `high` when unset (`crates/llm/src/anthropic.rs` ≈ 313).
- A role chooses only a model. `RoleModels` (`crates/server/src/db/json.rs` ≈ 589) maps a role to a
  profile id at two levels: globally (`app_settings["roles.defaults"]`) and per book
  (`books.model_overrides`). `resolve_model` (`services/sessions.rs` ≈ 627) resolves it, and a
  planner with no entry of its own follows the writer.
- Creating a session freezes the profile's options into `agent_sessions.model_snapshot.options`
  (`model_snapshot_with_ttl` ≈ 689). A rollover child copies its parent's snapshot
  (`agent/context/mod.rs` ≈ 2165).
- So every role on one profile shares its effort. To run the archivist at `low` and the writer at
  `max` on the same model, the author must create two profiles of that model.
- The author's database (read-only query, 2026-11-02):
  - profiles #3 and #4 are both on the OpenAI protocol with `reasoning_effort: "max"`;
  - #1 is the Demo profile, with `{}`;
  - all six roles use #4, and no book carries an override.

Facts about the Anthropic API that shape the design (the `claude-api` reference, October 2026):
- **Levels differ per model.** Opus 4.6 and Sonnet 4.6 have no `xhigh`, Opus 4.5 accepts only
  `low`/`medium`/`high`, and Sonnet 4.5 and Haiku 4.5 reject `effort` entirely. A fixed level set
  per protocol is therefore wrong: the model has to declare its levels.
- **An omitted effort is not `high` everywhere.** It defaults to `high` on most models but to
  `medium` on Opus 5.5 and Haiku 5.5. "Send nothing" would quietly change what is sent today.
- **Changing the top-level effort between requests invalidates the messages cache.** The level
  must stay frozen per session, which the snapshot already guarantees.

## 2. Design (D-162)

### 2.1 One concept, two wire fields
**Thinking effort** is the protocol-neutral setting. Its zh-CN label is the author's term,
"思考强度"; the existing "推理强度" strings belong to the two removed option fields. The protocol
decides the wire field:

| Protocol | Wire field | Allowed levels |
|---|---|---|
| `openai` | `reasoning_effort` | free-form (gateways differ), pattern `^[A-Za-z0-9_-]{1,32}$` |
| `anthropic` | `output_config.effort` | `low`, `medium`, `high`, `xhigh`, `max` |
| `mock` | none | none: the Demo provider ignores every option (D-132) |

`crates/llm`'s `ModelOptions` keeps both fields as the **wire carrier**: the snapshot's shape does
not change. What changes is where the value comes from. Profiles no longer store either field; the
session resolves a level per role and writes it into the snapshot.

### 2.2 The model profile declares its levels
New `model_profiles` columns (spec/02 §2):
- `effort_levels`: a JSON array of strings, default `[]`. These are the levels the model accepts,
  in display order (lowest first by convention). An empty list means the model takes no effort
  parameter, and nothing is sent.
- `default_effort`: `TEXT NULL`, one of `effort_levels`. It is the level a role gets when it
  chooses none. `NULL` means that no level is sent and the provider decides.

Validation on `POST` / `PATCH /model-profiles` returns `400 validation.failed` with
`details.fields.effort_levels` or `details.fields.default_effort`:
- at most 8 levels, each matching the pattern above, with no duplicates;
- on `anthropic`, every level must be in the closed set. They are stored in canonical order
  (`low` → `max`) whatever order they arrive in;
- on `mock`, the list must be empty;
- `default_effort` must be one of the levels;
- `options` can no longer carry `reasoning_effort` or `effort`. `ModelOptionsDto` drops both fields
  and gets `#[serde(deny_unknown_fields)]`, so a stale client gets a 400 through the extractor
  rejection envelope instead of a silent drop (the D-132 lesson);
- `options.extra_body` must not contain `reasoning_effort` (`details.fields.extra_body`). It is
  merged last (spec/04 §1), so it would override the role's level without saying so.

The same validation applies wherever the wire values appear:
- **Stored mismatch**: `ModelProfileDto.ignored_effort_levels` lists the stored levels that the
  provider's current protocol cannot send. A provider whose protocol changed is the only way to get
  one. On `anthropic` these are the levels outside the closed set; on `mock`, all of them; on
  `openai`, none. They are reported and never fail a read. The startup scan of D-132 logs them next
  to `ignored_options`, and the session resolution skips them (§2.4).
- **Presets** (`ProviderPresetDto` gains both fields, spec/04 §8):
  - Anthropic: `[low, medium, high, xhigh, max]`, default `high`, which is today's preset
    `effort: high` on the wire;
  - OpenAI: `[low, medium, high]` with no default, so a new profile sends nothing until a role
    chooses;
  - every other preset: none.

  The preset-table test of spec/04 §8 also checks the levels.
- **Test connection** sends the profile's `default_effort`, because it tests the saved
  configuration (spec/04 §8).

### 2.3 The role chooses a level
`RoleModels` gains `efforts: RoleEfforts`, with `#[serde(default)]` and one `Option<String>` per
role. It is stored in the same two places as the role's model:
- `app_settings["roles.defaults"].efforts` is the global choice;
- `books.model_overrides.efforts` is the per-book override (Q2).

Old rows load with `efforts = {}`, and nothing is rewritten. The type keeps its name to avoid
churn; its doc comment says that it holds both.

Save validation, on `PATCH /app-settings` `roles_defaults` and on `PATCH /books/:id`
`model_overrides`:
- each value is trimmed and must match the level pattern, or the request fails with
  `400 validation.failed` and `details.fields["efforts.<role>"]`;
- the value is **not** checked against a model. The role's model can change later, and the
  resolution handles stale values.

**Clear all roles** clears the efforts too.

### 2.4 Resolution at session creation (spec/03 §4 step 1)
```
model   book.models[role] → global.models[role]
        → planner only: book.models.writer → global.models.writer
        via_writer := the planner's model came from a writer entry

candidates (unset entries skipped, in order)
        book.efforts[role] → global.efforts[role]
        → via_writer only: book.efforts.writer → global.efforts.writer

effort  the first candidate that is one of profile.effort_levels and sendable by the protocol
        → else profile.default_effort, if sendable
        → else none
```
- **Stale choices.** A candidate that is set but skipped adds the warning
  `model.effort_unsupported` to the creation response and to `SessionDto.warnings`, once per
  session. It is skipped when it is not a level of the resolved model, or when the protocol cannot
  send it. The session still starts. The next candidate applies, so a book override that the book's
  model does not support falls through to the global choice.
- **Planners.** A planner with its own model does not inherit the writer's effort, because another
  model's level means nothing there. A planner that follows the writer's model also follows the
  writer's level unless it sets one of its own.
- **One resolver.** Every creation path uses one resolver: `create_inner` and the two debug
  sessions. `try_resolve_model`, the D-046 probe, is unchanged.

### 2.5 The wire
- **Snapshot.** The resolved level goes into `model_snapshot.options`:
  - `openai` → `reasoning_effort`;
  - `anthropic` → `effort`, parsed into `Effort`. Validation guarantees that it parses; a failure
    counts as unsupported;
  - `mock` → nothing.

  Nothing else in the snapshot changes.
- **Frozen per session.** A rollover child copies its parent's snapshot, so the level never changes
  within a session or its rollover chain. A later settings change affects only new sessions,
  because an effort change mid-session would invalidate the cache.
- **Anthropic adapter.** It sends `output_config` only when `options.effort` is set. Today it always
  sends it, defaulting to `high`. With this change an empty level list really means "no effort
  parameter", which a model that rejects it (Haiku 4.5) needs. The migration (§2.6) keeps today's
  wire for every existing profile and session.
- **Header.** `SessionDto.effort: string | null` is read from the snapshot's protocol field, for the
  session header.

### 2.6 Migration `0009_role_effort.sql`
It runs in one Rust-managed transaction like 0006 and 0008 (`use_transaction() = Some(false)`),
because both `ALTER`s are non-idempotent.
1. `ALTER TABLE model_profiles ADD COLUMN effort_levels TEXT NOT NULL DEFAULT '[]' CHECK
   (json_valid(effort_levels))` and `ALTER TABLE model_profiles ADD COLUMN default_effort TEXT`.
   **0001 is not edited.** No later file rebuilds `model_profiles`, so a column added to 0001 would
   make this `ALTER` fail on a fresh database. 0009 is the only source of the two columns, and
   spec/02 §1 says so.
2. **OpenAI-protocol profiles** whose `options.reasoning_effort` is a non-blank string get
   `effort_levels = [v]` and `default_effort = v` (Q3).
3. **Anthropic-protocol profiles** get `effort_levels = [coalesce(options.effort, 'high')]` and the
   same default, because the adapter sent `high` when the field was unset.
4. **Both keys leave `options`** (`json_remove`) on every profile. A key that its protocol could not
   send (D-132) never reached the wire, so dropping it changes no request. The pre-ship report lists
   each such key.
5. **Anthropic snapshots without an effort** get `$.options.effort = 'high'`. These are the rows
   where `json_extract(model_snapshot, '$.protocol') = 'anthropic'` and `$.options.effort` is null,
   and `high` is the value the adapter sent implicitly. Without this step, an in-flight session
   would change its effort, and lose its cache, at its first request after the upgrade. The snapshot
   is frozen configuration, not message history: spec/03 §1.3 protects messages and sealed results.
6. **Roles get no effort.** They resolve to the profile's default, which reproduces each profile's
   old wire value.
7. **Guards** run in the same transaction, with the temp-table `CHECK` pattern of 0008:
   - the `model_profiles` row count is unchanged;
   - no profile's `options` still holds either key;
   - every non-null `default_effort` is one of its `effort_levels`.

Outcome on the author's database:
- #3 and #4 get `[max]` with default `max`, and #1 gets `[]`;
- the roles are unchanged, and every request is byte-identical.

To run the reviewer at `high`, the author adds levels to #4 and picks one for the role.

### 2.7 API changes (spec/10)
- `ModelProfileInput` gains `effort_levels: string[]` and `default_effort: string | null`. Neither
  is optional in TypeScript, so the compiler finds every caller that builds a profile.
  `ModelProfileDto` gains the same two fields and `ignored_effort_levels: string[]`.
  `ProviderPresetDto` gains the two fields.
- `ModelOptionsDto` loses `reasoning_effort` and `effort` and denies unknown fields.
- `GET /provider-presets/options` no longer lists `reasoning_effort` or `effort`. `crates/llm` gets:
  - `ModelOptionKey::PROFILE`, which is `ALL` minus the two effort keys. `ALL` stays for the wire
    test;
  - `EffortRule` with the values `None` (mock), `Free` (openai) and `Closed(&[…])` (anthropic), and
    `EffortRule::for_protocol` and `EffortRule::wire_key`.
- `RoleModels.efforts` (`RoleEfforts`).
- `SessionDto.effort`.
- The warning code `model.effort_unsupported`.

### 2.8 UI (spec/11 §5.3, §5.5)

**Settings › Models dialog.** A **Thinking effort** section follows the protocol options.
- **Levels** depend on the protocol:
  - `anthropic`: five checkboxes in canonical order, labelled `settings:value.effort.*`;
  - `openai`: a chip input. The author types a level and presses Enter or **Add**, and × removes
    a chip. The order is the entry order. Client checks mirror the server's pattern, duplicate and
    count rules;
  - `mock`: the section is replaced by a note ("The Demo provider takes no effort levels.").
- **Default level**: a select with "None — the provider decides" plus the levels. Removing the
  default level resets it to None.
- **Hint**: "Agent roles choose one of these levels. A role that chooses none uses the default."
- **Provider switch**: switching to another protocol removes the levels it cannot send and names
  them in the existing "Options removed" notice.
- **Stored mismatch**: `ignored_effort_levels` is flagged in the table and in the dialog, like
  `ignored_options`. Its one-click fix PATCHes them away, and clears the default if it was one of
  them.
- **Table**: a **Thinking effort** column shows the levels joined with " · ", marks the default
  ("high (default)"), and shows "—" when the list is empty.
- **Old fields**: the two old option fields disappear, because the option table no longer serves
  them. Their keys `settings:option.reasoning_effort*` and `settings:option.effort*` are removed;
  `settings:value.effort.*` stays for labels.

**Settings › Agent roles.** Each row gains an effort select next to its model select.
- **Entries**: the unset entry plus the levels of the role's resolved model.
- **Unset label**:
  - "Same as Writer" when the planner follows the writer's model;
  - otherwise "Model default ({level})" when the model has a default;
  - otherwise "Provider default".
- **Disabled**: with "No effort levels" when the resolved model has none or no model resolves.
- **Stale value**: a stored value that is not a level of the resolved model stays selected as
  "{level} (not supported)". A hint names the level that sessions will use instead.
- **Model change**: changing a role's model keeps its effort, which may still apply; the stale
  marker covers a mismatch. **Clear all roles** clears both columns.
- **Shared rule**: `features/settings/roleModels.ts` gains `resolveRoleEffort(…)`, which mirrors
  §2.4. Both pages use it for the unset label and the stale hint.

**Book settings › Model overrides per role.** Each row gains an effort select. Its unset entry is
"Global setting ({effective level})". It is locked under the same conditions as the model select,
and saved with the page's **Save** as part of the whole `model_overrides` object.

**SessionHeader.** It shows "{model label} · {effort label}" when `effort` is set. The effort label
is `settings:value.effort.<v>`, falling back to the raw value. The creation response's
`model.effort_unsupported` warning shows as a toast, like `context.large_kickoff`.

## 3. Tasks — milestone M19

| # | Task | Spec | Tests / acceptance |
|---|---|---|---|
| 19.1 | **Docs**: done with the plan (D-162). | plan §6 | `check.sh` green (docs only) |
| 19.2 | **`crates/llm`**: `EffortRule`, `ModelOptionKey::PROFILE`; the Anthropic adapter sends `output_config` only when `effort` is set. | 04 §3, §5 | plan §4.1 |
| 19.3 | **Schema and types**: `0009_role_effort.sql` and its module; the entity's two fields; `RoleModels.efforts`; the DTO changes of §2.7; regenerate the bindings. | 02 §1, §2, §4; 10 §2.1, §2.8 | plan §4.2; `entity_schema.rs`; JSON round trips, including old `RoleModels` JSON |
| 19.4 | **Model profiles**: validation, presets, the served option table, `ignored_effort_levels`, the startup scan, test connection. | 04 §5, §5.1, §8; 10 §2.1 | plan §4.3 |
| 19.5 | **Roles and sessions**: save validation of `efforts`; the resolver; the snapshot; the warning; `SessionDto.effort`. | 03 §4; 04 §5.1; 10 §1, §2.8 | plan §4.4 |
| 19.6 | **Web: Settings › Models** (§2.8). | 11 §5.5 | plan §4.5 |
| 19.7 | **Web: Agent roles, Book settings, SessionHeader**; i18n in both locales. | 11 §5.3, §5.5; PROJECT §8.12 | plan §4.5; `i18n:check` |
| 19.8 | **Smoke and pre-ship**: run `0009` on a **copy** of the author's database and report, per profile, the options before and after, the new levels and default, and every dropped cross-protocol key; run smoke item 14. | PLAN §M11.5 | the report is in the final message and in D-162's consequences; the checklist passes |

## 4. Tests

### 4.1 `crates/llm`
- `model_options.rs`:
  - `PROFILE` is `ALL` minus `reasoning_effort` and `effort`;
  - the wire test still covers `ALL`;
  - `EffortRule::for_protocol` and `wire_key` hold for all three protocols.
- `anthropic_stream.rs` (≈ 234 asserts today's implicit `high`): with `effort: None` the body has
  **no** `output_config`, and with `Some(Xhigh)` it has `output_config.effort = "xhigh"`.

### 4.2 Migration (`crates/server/tests/role_effort_migration.rs`)
**Pre-0009 fixture**:
- an OpenAI profile with `reasoning_effort: "max"`;
- an OpenAI profile with a stranded `effort: "high"`;
- an OpenAI profile with `reasoning_effort: ""`;
- an Anthropic profile with `effort: "xhigh"`;
- an Anthropic profile with neither field;
- a mock profile with `{}`;
- an Anthropic session whose snapshot has `effort: null`;
- an OpenAI session snapshot.

**Expected**:
- the levels and defaults of §2.6 steps 2–3, with `[]` and `NULL` for the empty-string profile,
  the stranded one and the mock;
- no `options` holds either key, and every other option is byte-equal after decoding;
- only the Anthropic snapshot changes, and only `options.effort`;
- the guards pass;
- a fresh database (0001 → 0009) has each column exactly once;
- a broken copy, such as a pre-seeded `effort_levels` column that makes the `ALTER` fail, leaves the
  pre-0009 database unchanged and retryable.

### 4.3 Model profiles API
- **Validation table**:
  - Anthropic `ultra` is refused;
  - duplicates are refused, and so are 9 levels;
  - a pattern violation is refused;
  - a non-empty list on mock is refused;
  - a default that is not a level is refused;
  - `options.effort` and `options.reasoning_effort` give a 400 that names the field;
  - `options.extra_body.reasoning_effort` gives a 400;
  - Anthropic levels are stored in canonical order.
- **Presets**: every preset's levels are valid for its protocol (an extension of the D-132 preset
  test).
- **Option table**: `GET /provider-presets/options` lists neither effort key.
- **Stored mismatch**: `ignored_effort_levels` after a provider protocol switch, plus the startup
  scan's log line.
- **Test connection** sends `default_effort` (wiremock asserts the body for both protocols).

### 4.4 Resolution (`crates/server/tests/role_effort.rs`)
Each case creates a session and asserts `model_snapshot.options` and the warnings.

| # | Setup | Expected |
|---|---|---|
| 1 | no efforts set; the profile's default is `high` | `high`, no warning |
| 2 | global writer `low` | `low` |
| 3 | book writer `max`, global writer `low`, both levels | `max` |
| 4 | book writer `max` that is not a level, global `low` | `low` + `model.effort_unsupported` |
| 5 | every candidate unsupported, no default | no effort field + the warning |
| 6 | planner follows the writer's model, no planner effort, writer `low` | `low` |
| 7 | planner on its own model with default `medium`, writer `low` | `medium` (no inheritance) |
| 8 | planner follows the writer's model, planner `high` | `high` |
| 9 | the same level on `openai` / `anthropic` / `mock` | `reasoning_effort` / `effort` / nothing |
| 10 | a rollover after the setting changed | the child keeps the parent's level |
| 11 | the setting changes while a session runs | the running session's snapshot is unchanged |

Also:
- **Old JSON**: `roles.defaults` JSON without `efforts` loads, and so does `model_overrides` JSON
  without it;
- **Save validation**: the save errors of §2.3;
- **`SessionDto.effort`**, per protocol;
- **The wire**, end to end with wiremock: the OpenAI body carries the role's `reasoning_effort`,
  and the Anthropic body carries `output_config.effort`.

### 4.5 Web (Vitest)
- `modelsPage`:
  - the Anthropic checkboxes, the OpenAI chips (a duplicate and a pattern violation are refused)
    and the mock note;
  - the default resets when its level is removed;
  - the provider-switch removal notice;
  - the mismatch fix PATCH carries the remaining levels;
  - the two old option fields are gone.
- `rolesPage`:
  - an effort select per row and the three unset labels;
  - the disabled state and the stale marker;
  - **Clear all roles** clears the efforts;
  - the PATCH carries the whole object.
- `bookSettingsPage`: an effort select per row, the "Global setting ({level})" label, and saving
  with `model_overrides`.
- `resolveRoleEffort`: the cases 1–8 of §4.4, mirrored.
- `SessionHeader` shows the effort.
- `model.effort_unsupported` has an `errors:` key in both locales (spec/11 §9).

## 5. Smoke item 14 (append to PLAN §M11.5)
**With the Demo provider**: the Demo model's dialog shows the "no effort levels" note, and on the
Roles page an effort select whose model is the Demo is disabled.

**With a real OpenAI-compatible or Anthropic provider**, when a key is available:
1. Give a model the levels `low`, `high` and `max`, with default `high`.
2. Set the reviewer to `low` and the writer to `max`, then write a chapter and review it. The
   writer's session header shows "max" and the reviewer's shows "low".
3. Override the writer to `high` in Book settings. The next write session shows "high".
4. Remove `high` from the model. The next write session shows the unsupported toast and uses `max`,
   the global choice.

## 6. Documents amended with this plan
- **PROJECT**: §5 (intro), F1 steps 3–4, §8.6 (the header shows the effort), §8.12 (Models, Agent
  roles, Book settings).
- **spec/02**:
  - §1: the 0009 paragraph, and why 0001 is not edited;
  - §2: the `model_profiles` columns;
  - §4: `RoleModels.efforts`, and a `ModelSnapshot` comment.
- **spec/03 §4 step 1**: the effort resolution.
- **spec/04**:
  - §2: the OpenAI body comment;
  - §3: the Anthropic `output_config` rule;
  - §5: the table rows of the two effort keys;
  - a new **§5.1 Thinking effort**;
  - §8: the presets and test connection;
  - §9: the tests.
- **spec/10**:
  - §1: the warning codes;
  - §2.1: the model-profile and app-settings rows;
  - §2.8: `SessionDto.effort`, `ModelProfileDto`, `RoleModels`.
- **spec/11**: §5.3 (`SessionHeader`), §5.5 (Settings › Models, Agent roles, Book settings), §9.
- **DECISIONS**: D-162. **PLAN**: the M19 table and smoke item 14. **docs/README**: M0–M19.

## 7. Risks

| Risk | Mitigation |
|---|---|
| An in-flight Anthropic session loses its implicit `high` when the adapter stops defaulting | 0009 step 5 makes the value explicit in every such snapshot |
| A stale browser tab still sends `options.effort` | `deny_unknown_fields`: a 400 that names the field, never a silent drop |
| A one-click fix PATCH (full profile) drops the levels | the TypeScript input type makes both fields required; a Vitest asserts it |
| An OpenAI-compatible gateway rejects a typed level | the request fails as `provider.bad_request`, as a wrong `reasoning_effort` does today; the author fixes the list |
| A model that rejects `effort` (Haiku 4.5) | now possible to configure: an empty list sends no `output_config` (today it always fails) |

## 8. Out of scope
- A per-session effort picker when starting a chapter, or per-kind levels such as outline and write
  under one role. The role is the unit.
- Anthropic's per-message effort (beta) and any change of level mid-session.
- `thinking_display` and every other option: they stay on the model.
- Discovering levels from a provider's models API.
- Renaming `RoleModels`.
