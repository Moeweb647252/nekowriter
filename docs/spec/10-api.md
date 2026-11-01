# 10 — HTTP API and Events

## 1. Conventions

- Base path `/api`. JSON in and out (`application/json`; RFC 8259 fixes the encoding to UTF-8,
  and axum does not append a `charset` parameter). Field names are `snake_case`.
- IDs are integers. Timestamps are RFC 3339 strings.
- Every DTO is a Rust struct with `#[derive(Serialize, Deserialize, TS)] #[ts(export)]`, exported
  to `web/src/api/gen/` (`cargo test -p nekowriter export_bindings`). The frontend imports types
  only from there.
- **Lists**: most collections return full arrays (filters narrow them). Entities and the
  timeline are returned whole, because the UI sorts entities client-side and reorders the
  timeline by drag and drop in story order. The append-mostly, id-ordered lists (`changes`,
  `messages`, `knowledge entries`) return `{ items, next_cursor }` with `?cursor=&limit=`
  (default 100, max 500). The cursor is an opaque string (id-based).
- **Optimistic concurrency** for text: content writes carry `base_version`; a mismatch returns
  `409 conflict.version` with the current version.
- **Warnings**: mutation responses and `SessionDto.warnings` may include `warnings: string[]`
  (codes the UI localizes: `model.missing_role`, `context.large_kickoff`,
  `prompt.override_failed`).
- **Errors**: `{ "error": { "code": "chapter.locked", "message": "…", "details": {…} } }` (§4).

## 2. Endpoints

### 2.1 System and settings
| Method | Path | Body / query | Response |
|---|---|---|---|
| GET | `/health` | | `{ ok, version, vec_version, embedding: IndexStatusDto }` |
| GET / PATCH | `/app-settings` | `AppSettingsPatch` (roles.defaults, book_defaults, search, embedding) | `AppSettingsDto` (secrets masked) |
| POST | `/app-settings/search/test` | `{ query }` | `{ hits: SearchHitDto[] }` |
| GET / POST | `/providers` | `ProviderInput {name, protocol, base_url, api_key?, extra_headers}` | `ProviderDto[]` / `ProviderDto` (key masked) |
| PATCH / DELETE | `/providers/:id` | partial `ProviderInput` (omitted `api_key` = unchanged) | `ProviderDto` / 204 |
| POST | `/providers/:id/test` | `{ model? }` | `{ ok, latency_ms, text?, error? }` |
| GET | `/providers/:id/remote-models` | | `RemoteModelDto[]` |
| GET | `/provider-presets` | | `ProviderPresetDto[]` (spec/04 §8; Demo only when enabled) |
| GET | `/provider-presets/options` | | `ModelOptionSpecDto[]` — the `options` field → protocol table the settings form renders (spec/04 §5, D-132) |
| GET / POST | `/model-profiles` | `ModelProfileInput`; an option the provider's protocol cannot send is `400 validation.failed` (spec/04 §5) | `ModelProfileDto[]` / `ModelProfileDto`, including `ignored_options` for a stored mismatch (reported, never a failed read) |
| PATCH / DELETE | `/model-profiles/:id` | | |
| GET | `/embedding/status` | | `IndexStatusDto` |
| POST | `/embedding/rebuild` | | 202 |

### 2.2 Books, tracks, chapters
| Method | Path | Body / query | Response |
|---|---|---|---|
| GET | `/books` | `?status=` — `conceiving`/`active`/`archived` filters exactly; omitted, empty or `all` returns every status | `BookSummaryDto[]` (counts, words, latest activity, attention count). The library calls it unfiltered and hides archived rows itself (D-129) |
| POST | `/books` | `{ title, idea, genre, writing_language, start_conception, knowledge_check?, auto_start_conception? }` — `knowledge_check` defaults to `false` and `auto_start_conception` to `true`; both are ignored without `start_conception` (spec/08 §5, D-158) | `{ book: BookDto, session_id, research_task_id, warnings }` (`session_id`: the knowledge check's session, or the conception session when no check ran, or `null`; `research_task_id`: the check's task or `null`) |
| GET / PATCH / DELETE | `/books/:id` | PATCH: title, genre, writing_language, settings, model_overrides, status (archive) | `BookDto` |
| GET | `/books/:id/overview` | | `BookOverviewDto` (spec/08 §7) |
| POST | `/books/:id/conception/start` | | `{ session_id }` (the open session or a new one; allowed while a knowledge check is open; spec/08 §5) |
| POST | `/books/:id/knowledge-check` | `{ requirements? }` | `201 { task: ResearchTaskDto, session_id }` (spec/07 §8.1). `409 book.invalid_state` unless the book is `conceiving` or `active`; `409 research.invalid_state` with `details.session_id` while a check is open; no researcher model → the same error as `run`, and no row is written |
| POST | `/books/:id/conception/finalize` | | `{ book, seed_session_id?, warnings }` |
| POST | `/books/:id/canon/seed` | | `{ session_id }` (the open session or a new one; spec/08 §5) |
| GET / POST | `/books/:id/tracks` | `TrackInput` | `TrackDto[]` / `TrackDto` |
| PATCH / DELETE | `/tracks/:id` | | |
| GET | `/tracks/:id/chapters` | | `ChapterListItemDto[]` (no content) |
| POST | `/tracks/:id/chapters` | `{ title?, guidance? }` | `ChapterDto` |
| POST | `/tracks/:id/chapters/reorder` | `{ ids }` | `ChapterListItemDto[]` |
| GET | `/chapters/:id` | | `ChapterDto` (content, outline, notes, summary, versions meta, open sessions) |
| PATCH | `/chapters/:id` | `{ title?, guidance?, outline?, reference_notes?, summary? }` | `ChapterDto` |
| DELETE | `/chapters/:id` | | 204 |
| PATCH | `/chapters/:id/content` | `{ base_version, ops: ContentOpInput[] }` | `{ version, ops: ContentOp[], word_count }` |
| PUT | `/chapters/:id/content/text` | `{ base_version, text }` (paragraphs separated by blank lines) | same; pids kept by alignment (`similar` paragraph diff) |
| POST | `/chapters/:id/outline` | `ChapterOutlineInput {guidance?, pause_after_outline?, force?}` (spec/08 §1.1) | `{ session_id, warnings }` |
| POST | `/chapters/:id/write` | `ChapterWriteInput {guidance?, granularity?, auto_pause?, force?}` (spec/08 §1.1) | `{ session_id, warnings }` |
| POST | `/chapters/:id/submit` | | `ChapterDto` |
| POST | `/chapters/:id/review` | | `{ session_id }` |
| POST | `/chapters/:id/send-back` | `{ feedback, mode, review_report_id?, issue_indexes? }` | `{ session_id }` |
| POST | `/chapters/:id/approve` | | `{ chapter, maintenance_session_id?, warnings }` |
| POST | `/chapters/:id/maintain` | | `{ session_id }` |
| POST | `/chapters/:id/resync` | | `{ session_id }` |
| GET | `/chapters/:id/versions` | | `ChapterVersionMetaDto[]` |
| POST | `/chapters/:id/versions` | `{ note }` | manual snapshot |
| GET | `/chapter-versions/:id` | | `ChapterVersionDto` (with content) |
| POST | `/chapter-versions/:id/restore` | | `ChapterDto` |
| GET | `/chapters/:id/reviews` | | `{ reports: ReviewReportDto[], feedback: ChapterFeedbackDto[] }` |
| POST | `/review-reports/:id/rereview` | `{ comment }` | `{ session_id }` |

### 2.3 Setting materials and proposals
| Method | Path | Body | Response |
|---|---|---|---|
| GET | `/books/:id/setting-docs` | | `SettingDocDto[]` (3) |
| PUT | `/books/:id/setting-docs/:kind` | `{ content, base_revision }` | `SettingDocDto` |
| GET / POST | `/books/:id/characters` | `CharacterInput` | `CharacterDto[]` / `CharacterDto` |
| PATCH / DELETE | `/characters/:id` | | |
| POST | `/books/:id/characters/reorder` | `{ ids }` | `CharacterDto[]` |
| GET | `/books/:id/proposals` | `?status=pending` | `ProposalDto[]` (with the computed preview diff) |
| POST | `/proposals/:id/accept` / `/proposals/:id/reject` | accept: optional `{ confirm_open_children?: boolean }` (goal status proposals only, spec/06 §5) | `ProposalDto` (an accept whose `find` no longer matches, or whose goal target is gone or closed, returns 200 with `status: "conflict"`, nothing applied; a goal status change with open descendants is `409 goal.open_children` unless confirmed. Accept needs `pending`; reject needs `pending` or `conflict` (it dismisses a conflict); otherwise `409 proposal.invalid_state`) |

### 2.4 Canon
| Method | Path | Body / query | Response |
|---|---|---|---|
| GET | `/books/:id/entities` | `?kind&status&q&track_id` | `EntityListItemDto[]` |
| POST | `/books/:id/entities` | `EntityInput` | `EntityDto` |
| GET / PATCH / DELETE | `/entities/:id` | PATCH: partial `EntityInput` + optional `note` (the change note, spec/06 §3; also accepted by every canon PATCH below) | `EntityDto` (relations, appearances, recent changes, design link) |
| POST | `/entities/:id/merge` | `{ from_id }` | `EntityDto` |
| POST | `/entities/:id/protagonist` | | `EntityDto` |
| GET / POST | `/books/:id/relations` | `?entity_id` / `RelationInput` | `RelationDto[]` / `RelationDto` |
| PATCH / DELETE | `/relations/:id` | | |
| GET / POST | `/books/:id/timeline` | `?track_id&entity_id` / `TimelineEventInput {…, after_id?, before_id?}` | `TimelineEventDto[]` (story order) / `TimelineEventDto` |
| PATCH / DELETE | `/timeline-events/:id` | | |
| POST | `/timeline-events/:id/move` | `{ after_id? , before_id? }` | `TimelineEventDto` |
| GET / POST | `/books/:id/memory-notes` | `?kind&status` | `MemoryNoteDto[]` |
| PATCH / DELETE | `/memory-notes/:id` | | |
| GET / POST | `/books/:id/foreshadowing` | `?status` | `ForeshadowingDto[]` |
| PATCH / DELETE | `/foreshadowing/:id` | | |
| GET | `/books/:id/memory` | `?track_id&chapter_id` (default: the next chapter of the main track) | `MemorySnapshotDto` plus `rendered` (the exact `block.memory` text), `recent_chapters` (the chapters `block.recent_chapters` injects in full: id, number, label, title, characters, partial) and `systems` (the book's `CanonSystems` the text was rendered with) |
| GET | `/books/:id/changes` | `?chapter_id&session_id&target_type&actor&cursor` | page of `ChangeDto` |
| POST | `/changes/:id/revert` | | `ChangeDto` (the revert change); reverting the `create` of a goal node that has children is `409 goal.has_children` |
| GET | `/books/:id/goals` | `?track_id&status&entity_id&event_id&foreshadowing_id` (the last three return the nodes linking to or conditioned on that row: "Referenced by") | `GoalNodeDto[]` with the derived `ready`, `closable` and per-condition state (spec/06 §11.3) |
| POST | `/books/:id/goals` | `GoalNodeInput` | `GoalNodeDto` |
| PATCH / DELETE | `/goals/:id` | PATCH: partial `GoalNodeInput` (`level` and `track_id` cannot change), `note?`, `confirm_open_children?` | `GoalNodeDto` / 204 (`409 goal.open_children`, `409 goal.has_children`, spec/06 §11.4) |
| GET | `/books/:id/canon/counts` | | `CanonCountsDto`: per system, the rows kept in every scope and status (Book settings' retained-data line, spec/06 §10.3) |

### 2.5 Knowledge and research
| Method | Path | Body / query | Response |
|---|---|---|---|
| GET / POST | `/knowledge/scopes` | `{ name, description }` | `ScopeDto[]` (with entry counts) / `ScopeDto` |
| PATCH / DELETE | `/knowledge/scopes/:id` | | |
| GET / POST | `/knowledge/scopes/:id/entries` | `?q&cursor` / `EntryInput` | page / `EntryDto` |
| GET / PATCH / DELETE | `/knowledge/entries/:id` | | `EntryDto` |
| GET | `/knowledge/search` | `?q&scope_ids=1,2&limit` | `KnowledgeHitDto[]` |
| GET / PUT | `/books/:id/scopes` | PUT: `{ excluded_scope_ids }` replaces the book's exclusion set (unknown id → 404, nothing changes) | `ScopeDto[]`: the scopes **visible** to the book, by name (every scope minus its exclusions, spec/07 §1, D-158) |
| GET / POST | `/research-tasks` | `?book_id&status` / `{ book_id?, scope_name, topic, requirements, language? }` (created as `draft` while the book is conceiving, otherwise `ready`, always with `origin: author` and `kind: topic`; `language` is required without a book and ignored with one) | `ResearchTaskDto[]` / `ResearchTaskDto` |
| GET / PATCH / DELETE | `/research-tasks/:id` | PATCH: topic, requirements, scope_name, `status: ready` (confirms a draft); not while its session is open. A PATCH of a `conception` task sets `origin: author`. A knowledge check accepts `requirements` only (`topic` or `scope_name` → `422 validation.invalid_fields`) | `ResearchTaskDto` (with `kind`, `origin`, and the report and the created entries when done) |
| POST | `/research-tasks/:id/run` | | `{ session_id }` |
| POST | `/research-tasks/:id/defer` | `ready` → `deferred`; idempotent, a repeat call returns the current row (spec/07 §8, D-138) | `ResearchTaskDto` |
| POST | `/research-tasks/:id/resume` | `deferred` → `ready`; idempotent in the same way | `ResearchTaskDto` |

`defer` and `resume` are dedicated transitions rather than a generic PATCH because the author is
moving a task between named states with rules of its own, and because a PATCH of `status` already
means "confirm a draft". The refused edges are `409 research.invalid_state`, whose `details`
carries `{ status, operation }`; a task with an open session reports
`{ session_id }` instead, like every other write to it (spec/07 §8).

### 2.6 Prompts
| Method | Path | Body / query | Response |
|---|---|---|---|
| GET | `/prompts` | `?book_id` | `PromptEntryDto[]` |
| PUT | `/prompts/:key` | `{ scope: "global" \| "book", book_id?, content }` | `PromptEntryDto` or `400 prompt.template_invalid {line, column, message}` |
| DELETE | `/prompts/:key` | `?scope&book_id` | `PromptEntryDto` |

### 2.7 Sessions
| Method | Path | Body / query | Response |
|---|---|---|---|
| GET | `/books/:id/sessions` | `?kind&status&chapter_id` | `SessionDto[]` |
| GET | `/sessions/:id` | | `SessionSnapshotDto { session, messages (last 200), tool_calls (of those messages), inbox (unconsumed author messages, oldest first), inbox_pending (count of every unconsumed inbox row, including notices), system_prompt, tool_names, next_cursor }` (the frozen prompt and tool list are shown in the session info popover) |
| GET | `/sessions/:id/messages` | `?cursor&limit` (the cursor from the snapshot or the previous page; older first) | `{ items: { message, tool_calls }[], next_cursor }` for scrollback |
| POST | `/sessions/:id/pause` · `/resume` `{feedback?}` · `/stop` · `/continue` · `/abandon` | | `SessionDto` (validated against spec/03 §3; e.g. `stop` only while `running`) |
| POST | `/sessions/:id/message` | `{ text }` | `SessionDto` |
| POST | `/sessions/:id/answer` | `{ tool_call_id, payload }` | `SessionDto` |
| PATCH | `/sessions/:id` | `{ granularity?, auto_pause?, pause_after_outline? }` — per kind: an outline session accepts `pause_after_outline` only; a write or revise session accepts the three fields; other combinations are `409 session.invalid_state` | `SessionDto` |
| POST | `/debug/sessions/echo` | `{ book_id }` (debug builds only; a test session with test-only tools, used by PLAN M3.5) | `{ session_id }` |

### 2.8 Key DTOs (abridged; the Rust definitions are authoritative)

```ts
type SessionDto = {
  id: number; book_id: number | null; role: AgentRole; kind: SessionKind;
  chapter_id: number | null; track_id: number | null; research_task_id: number | null;
  chapter_number: number | null; research_topic: string | null;   // data for the UI-composed title
  research_task_kind: ResearchTaskKind | null;   // "preparation" → the "Knowledge check" title (D-158)
  model_label: string; status: SessionStatus; phase: Phase; granularity: Granularity | null;
  auto_pause: boolean; pause_after_outline: boolean; pause_requested: boolean;
  held_tool_call_id: number | null; base_content_version: number | null;
  error: SessionError | null; usage: Usage;
  parent_session_id: number | null; child_session_id: number | null;
  retrying: { attempt: number; wait_seconds: number; reason: string } | null;
  warnings: string[]; created_at: string; updated_at: string; finished_at: string | null;
};
type MessageDto = {
  id: number; seq: number; role: "user" | "assistant"; kind: MessageKind;
  blocks: { type: "text" | "reasoning"; text: string }[];   // tool_call blocks are delivered as ToolCallDto
  stop_reason: string | null; usage: Usage | null; created_at: string;
};
type ToolCallDto = {
  id: number; message_id: number; seq: number; call_index: number; name: string;
  input: unknown | null;            // parsed when valid JSON
  input_text: string;               // raw (truncated to 20k for display)
  status: ToolCallStatus; is_error: boolean;
  result_text: string | null;       // raw_result_text before sealing, result_text after
  appendices: Appendix[]; ui: unknown | null; sealed: boolean;
  // The staged chapter write of a deferred text-changing call (spec/03 §5.2, D-139): what
  // approval will commit. Null once the call is finalized, because the text is then in the chapter.
  pending_content: { content: ChapterContent; expected_version: number } | null;
  // The staged outline of a deferred submit_outline in an outline session (spec/03 §5.2): what
  // approval will write. Null once the call is finalized.
  pending_outline: { outline: string; reference_notes: string; title: string | null } | null;
  created_at: string; finished_at: string | null;
};
type ChapterDto = ChapterListItemDto & {
  guidance: string; outline: string; reference_notes: string; summary: string;
  content: ChapterContent; content_version: number;
  open_sessions: SessionDto[];      // open planner/writer/reviewer/archivist sessions of this chapter
};
type ChapterListItemDto = {
  id: number; book_id: number; track_id: number; number: number /* = ord */;
  title: string; status: ChapterStatus; canon_status: CanonStatus; word_count: number;
  has_outline: boolean;             // !chapters.outline.trim().is_empty() (the navigator's icon)
  updated_at: string;
};
type GoalNodeDto = {
  id: number; book_id: number; track_id: number | null; level: GoalLevel; parent_id: number | null;
  title: string; purpose: string; status: GoalStatus;
  chapter_from: number | null; chapter_to: number | null;
  conditions: { condition: GoalCondition; satisfied: boolean; broken: boolean;
                target_label: string | null /* stored name or title, for the readable sentence */ }[];
  entities: { id: number; name: string }[]; events: { id: number; title: string }[];
  foreshadowing: { id: number; title: string; status: ForeshadowingStatus }[];
  ready: boolean; closable: boolean;             // derived on read (spec/06 §11.3)
  created_at: string; updated_at: string;
};
type GoalNodeInput = {
  track_id?: number | null; level: GoalLevel; parent_id?: number | null;
  title: string; purpose: string; status?: GoalStatus;
  chapter_from?: number | null; chapter_to?: number | null; conditions?: GoalCondition[];
  entity_ids?: number[]; event_ids?: number[]; foreshadowing_ids?: number[];
};
type CanonCountsDto = {
  entities: { entities: number; relations: number; appearances: number };
  timeline: { events: number };
  memory: { notes: number; summaries: number };
  foreshadowing: { items: number };
  goals: { nodes: number };
};
// ProposalDto gains `goal_target: { id: number; level: GoalLevel; title: string; status: GoalStatus } | null`
// (the node of a goal_node proposal, or the parent plan of a new_goal_task proposal).
// BookOverviewDto: AttentionKind gains `ready_goals` (one row with its count); NextActionDto gains
// `ready_goals: number | null` (set on write_outline, write_chapter and write_next; spec/08 §7).
```

### 2.9 Writing skills (design §8)

| Method | Path | Body / query | Response |
|---|---|---|---|
| GET | `/writing-skills` | `?book_id=` | `WritingSkillDto[]` (active catalogue; `selected` and `used_by_books` are per view) |
| POST | `/writing-skills` | `WritingSkillInput {name, category?, description?, body?}` | `WritingSkillDto` (201; `author` is forced to `user`, `key` stays `null`) |
| PATCH / DELETE | `/writing-skills/:id` | `WritingSkillPatch` (partial) / — | `WritingSkillDto` / `WritingSkillDeleteResponse {used_by_books}` — **user rows only**; a built-in row is `409 writing_skill.builtin` |
| PUT | `/books/:id/writing-skills` | `BookWritingSkillsInput {skill_ids}` | `WritingSkillDto[]` (the replaced selection; every id must exist and not be retired) |

Deleting a user skill cascades away every book's selection of it, so `WritingSkillDeleteResponse`
(and every catalogue row's `used_by_books`) names the affected books for the confirmation. There
is no API shape that creates, edits or deletes a built-in row.

```ts
type WritingSkillDto = {
  id: number; key: string | null; name: string; category: string;
  author: "builtin" | "user"; description: string; body: string;
  enabled_by_default: boolean; retired: boolean;
  selected: boolean; used_by_books: { id: number; title: string }[];
  created_at: string; updated_at: string;
};
```

## 3. Server-sent events

`GET /api/events?book_id=<id>` sends events of that book plus global events (`book_id: null`).
Without `book_id` it sends everything.

- SSE framing: `id: <seq>`, `event: <type>`, `data: <JSON ServerEvent>`. A `: keep-alive` comment
  every 15 s.
- `seq` is a process-wide monotonic counter. On reconnect, the client refetches what it shows,
  because events are not replayed. A lagging subscriber (`broadcast` lag) receives `resync`.
- Events are emitted **after** the database commit.

| `type` | Payload | Frontend reaction |
|---|---|---|
| `session.updated` | `{ session: SessionDto }` | update the live store; invalidate `["sessions", …]`, and the chapter's detail key (`["chapters", "detail", session.chapter_id]`) when the session has a chapter — `ChapterDto.open_sessions` is what the chapter pane locks on and where it finds a held write (D-144); a session with no chapter (conception, research, canon seed) invalidates no chapter |
| `session.message` | `{ session_id, message: MessageDto }` | append to the activity stream; drop the stream buffer for that seq |
| `session.delta` | `{ session_id, seq, kind: "text" \| "reasoning", index, text }` | append to the live buffer |
| `session.tool_call` | `{ session_id, tool_call: ToolCallDto }` | upsert the card; no chapter invalidation of its own — `ToolCallDto` carries no chapter, and every tool call is published beside a `session.updated` (D-144) |
| `session.tool_preview` | `{ session_id, seq, call_index, name, field, delta }` | append to the preview buffer (ghost text) |
| `book.updated` | `{ book_id, op: "create" \| "update" \| "delete" }` (book row or its tracks: create, edit, finalize, archive) | invalidate the library, the book, and its tracks |
| `chapter.updated` | `{ chapter: ChapterListItemDto, outline_changed: boolean }` | update lists; invalidate the chapter query when the outline changed; invalidate the book's goal nodes and overview (an approval can make `chapter_at_least` true, spec/06 §11.3) |
| `chapter.content` | `{ chapter_id, version, ops: ContentOp[], word_count, tail_open, actor: "agent" \| "user" }` | apply the ops if `version = local + 1`, else refetch |
| `canon.changed` | `{ book_id, target_type, target_id, op, change_id }` (goal-node writes use `goal`, `plan`, `task`) | invalidate canon queries (the goal-node keys are in the canon family, so readiness refreshes with every canon write) and the memory snapshot |
| `settings.changed` | `{ book_id, kind: "setting_doc" \| "characters" \| "research_topics", doc_kind?, revision? }` (setting materials; a change of `books.settings` emits `book.updated`) | invalidate the matching queries |
| `proposal.changed` | `{ book_id, proposal_id, status }` | invalidate proposals and the overview |
| `knowledge.changed` | `{ scope_id, entry_id, op }` | invalidate knowledge queries |
| `research.updated` | `{ task: ResearchTaskDto }` | upsert |
| `index.status` | `IndexStatusDto { model, model_cache_dir, state, message?, pending, failed, done }` (`model_cache_dir` is the resolved fastembed cache: `$HF_HOME` or `<data_dir>/models`, spec/01 §6; PROJECT §8.12 shows it read-only) | settings page, health badge |
| `resync` | `{}` | invalidate everything visible |

```ts
type ContentOp =
  | { op: "insert"; after: string | null; pid: string; text: string }   // after = null → at the start
  | { op: "update"; pid: string; text: string }
  | { op: "delete"; pid: string }
  | { op: "tail"; open: boolean };
type ContentOpInput =                                                    // author edits
  | { op: "insert"; after: string | null; text: string }
  | { op: "update"; pid: string; text: string }
  | { op: "delete"; pid: string };
```

## 4. Error codes

| Code | HTTP | Meaning |
|---|---|---|
| `validation.failed` | 400 | invalid input; `details.fields` |
| `validation.invalid_fields` | 422 | invalid field for this resource; `details.fields`; a knowledge-check PATCH accepts `requirements` only |
| `not_found` | 404 | |
| `host.forbidden` | 403 | Host or Origin guard (spec/01 §10) |
| `conflict.version` | 409 | stale `base_version` / `base_revision` |
| `conflict.exists` | 409 | a unique name is taken (provider, model profile, scope, character, entity, entry), or a `POST` would create a second **active** relation for an ordered entity pair (`details.id` names the existing relation; edit it instead — spec/06 §1.2); `details.id` |
| `book.invalid_state` | 409 | e.g. finalize on a book that is not `conceiving`; `details.status` |
| `research.invalid_state` | 409 | the task already has an open session (`details.session_id`), or the transition is not allowed from its status (`details.status`, `details.operation`) |
| `settings.locked` | 423 | setting materials while the conception session runs; `details.session_id` |
| `chapter.invalid_state` | 409 | operation not allowed in this status; `details.status` |
| `chapter.session_exists` | 409 | `details.session_id` |
| `chapter.previous_not_ready` | 409 | `details.chapter_id`, `details.reason` |
| `chapter.is_anchor` | 409 | the chapter anchors side tracks; `details.track_ids` |
| `chapter.outline_missing` | 409 | a write session needs a non-empty `chapters.outline` (write and redraft); no details |
| `chapter.locked` | 423 | a planner or writer session of the chapter is running; `details.session_id` |
| `session.invalid_state` | 409 | `details.status`, `details.operation` |
| `model.missing_role` | 409 | no model for a role; `details.role` |
| `conception.incomplete` | 409 | `details.missing` |
| `track.mode_locked` | 409 | |
| `change.not_latest` | 409 | `details.later_change_ids` |
| `proposal.invalid_state` | 409 | the proposal's status does not allow the operation (accept: `pending`; reject: `pending` or `conflict`); `details.status` |
| `prompt.template_invalid` | 400 | `details.line`, `details.column`, `details.message` |
| `writing_skill.builtin` | 409 | PATCH/DELETE on a built-in writing skill; `details.id` |
| `goal.open_children` | 409 | setting `done`/`dropped` on a goal node (or accepting such a proposal) while it has `active` or `waiting` descendants, without `confirm_open_children`; `details.children: [{id, level, title, status}]` |
| `goal.has_children` | 409 | deleting a goal node with children, or reverting its `create`; `details.children_count` |
| `provider.unavailable` · `provider.auth` · `provider.bad_request` | 502 | provider tests and remote models (the connection test reports the same codes inside its 200 `error` field, spec/04 §8); the web search test maps a rejected key to `provider.auth` and other failures to `provider.unavailable` |
| `search.not_configured` | 409 | search test without a provider |
| `internal` | 500 | |

Session error codes (inside `SessionDto.error`, not HTTP): `provider.unavailable`,
`provider.auth`, `provider.bad_request`, `provider.context_overflow`, `provider.refusal`,
`agent.tool_error_streak`, `agent.no_tool_call`, `model.missing_provider`, `internal`.
