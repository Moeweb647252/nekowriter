# NekoWriter MVP Product Specification

This document is the behavior contract for the NekoWriter MVP. Code, tests and UI must agree with it. When they disagree, resolve the conflict here first; an implementation must never silently change product semantics.

"Must" marks a hard requirement. Section numbers are stable references for code comments and tests.

## 1. Product

NekoWriter is a local, single-user AI novel-writing workbench. A person sets a goal for a novel, reviews the agent's preparation work, guides and reviews the novel chapter by chapter, and finishes it, optionally adding extra chapters afterwards.

- The service and all data are local. Model and embedding calls go to services the user configures; their inputs leave the machine.
- One process: a Rust backend serving the web UI and the HTTP API on the loopback interface.
- Quality means: body text is never written incorrectly, feedback is never lost, reviews never apply to the wrong revision, interruptions are explainable, data can be exported, and the UI is comfortable for long reading sessions.
- The MVP is built for extensibility: business flows (stages, review kinds, tools, engines) are added, changed or removed by adding, changing or removing modules, not by editing central functions.

## 2. Glossary

| Term | Definition |
|---|---|
| Project | A container for settings and versions. The project name is not the novel title. |
| Version | An independent manuscript of a novel inside a project: its own novel title, brief, preparation artifacts, chapters, workflow and completion mark. |
| Document | Reviewable, citable content: novel brief, background, research, characters, rough outline, chapter guidance, chapter outline, chapter body. |
| Revision | One immutable commit of a document. Revisions are never overwritten. |
| Proposal | An `append_content` / `replace_content` edit computed against a specific body revision and not yet accepted. |
| Run | One agent execution for a stage (preparation, one chapter outline, one chapter body) or one delegated review. A run makes many model requests. |
| Model request | One actual request attempt with its settings, context, usage and outcome. |
| Tool call | A complete tool invocation from the model: tool name, call ID, arguments, status, result. |
| Review | A decision about one target revision or proposal, made by the user or by a delegated review agent. |
| Feedback | Free-form user input queued for delivery to the agent. It is neither a review decision nor an answer to a question. |
| Budget scope | The auxiliary-tool budget ledger of one preparation stage or one chapter outline. |
| Fold | The plain-text form of a stage's approved artifacts that replaces the stage's message history in later model context. |

## 3. Scope

### 3.1 In the MVP

| Module | Content |
|---|---|
| Dashboard | Token usage, running jobs, net word-count growth; time range and project filters (§15). |
| Projects | Create, rename, settings (model, thinking level, auxiliary tool budget, writing language), novel title, versions, soft delete and restore (§14). |
| Preparation | Background, research, characters, rough outline with an ordered chapter plan; ask the user; review all four after submission (§6). |
| Writing | Chapter guidance, outline and its review, body editing through tools, AutoApprove and ApproveRequired with switching, pause and the feedback queue, whole-chapter review, completion, extra chapters (§7, §9). |
| Review delegation | Every review can be delegated to a review agent with a per-review-kind prompt (§10). |
| Stage context folding | Approved stages fold to their artifacts' full text (§11). |
| Workspace | Chapter navigation, continuous reading, live updates, edit diff highlighting, citations, context dock (§18). |
| Prompts | Built-in system/user prompt pairs, system edits, project overrides, variable help, separate user prompt templates (§13). |
| Knowledge base | Spaces, entries, manual entry, vector search with an explicit embedding configuration, index status, a knowledge-accumulation interface with an empty registry (§12). |
| Capability infrastructure | A web-research interface with an empty registry; budget scopes (§12). |
| Internationalization | UI in English and Simplified Chinese; per-project writing language (§17). |
| Local use | Local start, persistence, explicit recovery of interrupted work, Markdown/TXT export, basic diagnostics, explicit shutdown (§16). |

### 3.2 Not in the MVP

Collaboration, accounts, cloud sync, real-time co-editing, a rich-text editor, branch merging, publishing, PDF/EPUB layout, a workflow editor, model protocols other than Chat Completions, concrete web-research engines, concrete knowledge-accumulation engines, direct manual editing of chapter bodies by the user, copying a version's approved content into a new version, full-manuscript search, a keyboard-shortcut help panel, and a purge (permanent delete) action.

## 4. Invariants

1. Only revisions committed by the backend are official content. Streaming previews and pending proposals are not.
2. A version has at most one active writing run; a project writes at most one version at a time.
3. A writing run has at most one unresolved tool call. A delegated review is a separate child run and never gains writing tools.
4. A tool call applies at most one edit; a review produces at most one final decision.
5. Accepting an edit re-validates that the proposal's base revision is still current.
6. An approval covers exactly the reviewed revision and its recorded dependencies; a new revision never inherits an old approval.
7. Every feedback item has a persistent ID, a sequence number and a delivery status. Clearing an input box never counts as delivery.
8. A sent model request and the message history it carried are never rewritten.
9. State changes, content changes, tool results and their persistent events commit in one transaction.
10. Model, embedding and research calls and waiting for the user never hold a database transaction.
11. Model output can never bypass the tool allowlist, budgets, version ownership of citations, or stage guards.
12. Soft-deleted objects and objects with a soft-deleted ancestor are excluded from normal lists, workflows and knowledge search.
13. Unreported token usage is unknown, not zero.
14. Errors, interruptions, exhausted budgets and missing capabilities are never treated as an approval or a completed stage.
15. Once every required artifact of a stage is approved, that stage's message history appears in later model context only as its fold. A stage is never folded early, and original records are never deleted.
16. Every assistant tool call in a run's message ledger has exactly one matching tool-result message later in the same run, on every path including failures.
17. Model and embedding API keys are stored only in local settings rows. They never appear in HTTP responses, events, command receipts, logs or request snapshots; views only say whether a key is set.

## 5. Workflow

### 5.1 Two state axes

The **workflow axis** says where the version is in the story:

```text
preparing.{not_started, generating, reviewing, revising}
writing.{chapter_ready, outline_generating, outline_reviewing, drafting, chapter_reviewing, chapter_done}
completed
extras.{the writing sub-states, for a chapter of kind extra}
```

The **execution axis** says what the active run is doing: `idle`, `queued`, `request_in_flight`, `tool_boundary`, `waiting_answer`, `waiting_review`, `paused`, `interrupted`, `failed`, `stopped`.

Control data lives next to them: `approval_mode` (`auto_approve` | `approve_required`) and `pause_requested`. A run's summary status (`queued`, `running`, `waiting`, `paused`, `interrupted`, `stopped`, `failed`, `succeeded`) is derived from the execution axis; the two are never edited independently.

`stopped` ends the current execution without deleting text and without completing the novel. Continuing reuses the same run, its context, tool calls, feedback and budget.

### 5.2 Runs

A top-level run exists per preparation stage, per chapter outline and per chapter body. Revisions after a rejected review belong to the same run. A run is `succeeded` only when its stage is approved and its fold is committed. A delegated review is a child run. Folding is done by the application without a model call.

### 5.3 Transitions

| Workflow state | Input | Guards and effects | Next state |
|---|---|---|---|
| (none) | Version created | Brief slot, the four artifact documents and the workflow exist; no run | preparing.not_started |
| preparing.not_started | Start preparation | The brief is non-empty and the model settings pass the capability check; create the preparation run and its budget scope | preparing.generating |
| preparing.generating / revising | Preparation submitted | First round: all four artifacts and a valid chapter plan; revision round: exactly the artifacts that need revision | preparing.reviewing |
| preparing.reviewing | Round decided | All four current revisions approved with valid dependencies; commit the preparation fold | writing.chapter_ready |
| preparing.reviewing | Round decided | Some artifact rejected or stale; persist feedback and the set to revise | preparing.revising |
| writing.chapter_ready | Start chapter | A plan item exists; guidance saved (may be empty); first chapter needs the preparation fold, later chapters need the previous chapter approved and folded | writing.outline_generating |
| writing.outline_generating | Outline submitted | Current chapter, outline revision and guidance agree | writing.outline_reviewing |
| writing.outline_reviewing | Approved | Target revision still current; commit the outline fold | writing.drafting |
| writing.outline_reviewing | Rejected | Keep feedback; reuse the chapter's budget scope | writing.outline_generating |
| writing.drafting | Chapter submitted | No unresolved tool call, no undelivered feedback, no pause request, non-empty body | writing.chapter_reviewing |
| writing.chapter_reviewing | Approved | Body and dependencies unchanged; lock the approved revision; commit the body fold | writing.chapter_done |
| writing.chapter_reviewing | Rejected | Feedback joins the body run's next delivery; body history kept | writing.drafting |
| writing.chapter_done | Next chapter | Unfinished plan items remain; stop at guidance, no model call | writing.chapter_ready |
| writing.chapter_done | Confirm completion | Every plan chapter approved; no open review, proposal or execution | completed |
| completed | Add extra chapter | Create an extra chapter with its title, its own guidance slot and budget scope; the completion time stays | extras.chapter_ready |
| extras.chapter_done | Finish extra chapter | The extra chapter is approved and nothing is pending | completed |

The writing and extras states are stored as one chapter stage (the chapter and its step); the chapter's kind tells them apart. Approving an outline creates the chapter's body run, which starts without another user action.

An invalid transition is refused with `STATE_CONFLICT` and a typed reason (for example `wrong_step`), and the client reloads the workspace. The UI never relies on hidden buttons in place of backend validation.

## 6. Preparation

- The novel brief must be non-empty before preparation starts. It can be edited only until preparation starts.
- The agent must submit four artifacts with stable keys `background`, `research`, `characters`, `outline`. Tabs are ordered background, research, characters, outline; this order does not constrain generation order.
- The rough outline includes a structured chapter plan: stable `plan_item_id`, order, title and goal summary. The plan has at least one chapter with unique order. It freezes when writing starts. Extra chapters never modify it.
- Each artifact is approved, or rejected with written feedback; a rejection without feedback is refused, because the agent needs to know what to change.
- After the first round, the four artifacts are reviewed in any order. When every open review in the round is decided, the version moves to revising or to writing as one batch. A revision round reviews only new revisions and stale items.
- Dependencies are fixed: research → background, characters, outline; background → characters, outline; characters → outline. Each dependency records the exact revision ID. A batch first creates all new revisions, then records dependencies, so order inside a batch cannot cause false staleness.
- When a source revision changes, reviews of downstream artifacts that reference the old source become `stale`. Stale is not approved. The MVP does not judge whether a change is semantically irrelevant; the downstream artifact must be resubmitted against the new dependency.
- The research artifact records sources honestly: user-provided material, knowledge-base entries, the model's own unverified knowledge, or gaps. It never invents links (D11).
- Once writing starts, preparation artifacts and the chapter plan are locked for the version. Changing them means creating a new, empty version and preparing again (D15).

## 7. Writing

- Each chapter starts at guidance input; empty guidance is allowed. The guidance is saved as a revision and becomes that chapter's run input.
- The outline is reviewed before the body starts (D01).
- The body is changed only through `append_content` and `replace_content` (§8.5). Completing a chapter is a structured submission, never a plain-text claim.
- Both approval modes keep outline and whole-chapter reviews mandatory.
- After the whole-chapter review approves, the chapter is final and the workspace stops at the next chapter's guidance (D05). When every plan chapter is approved, the user confirms completion.
- Extra chapters reuse the chapter flow with their own numbering; the completion mark stays and extra chapters can be added repeatedly (D13).

## 8. Agent protocol

### 8.1 Model adapter

- One protocol: Chat Completions-compatible HTTP. The configured API root ends with its real prefix (for example `/v1`); the adapter appends `chat/completions`. URLs are never guessed from failures.
- Requests declare tools and stream responses. Tool-call IDs, names and argument JSON are accumulated per tool index and handed over only when complete. `response_format` is not used.
- Parallel tool calls are disabled in the request. If a response still contains several tool calls, none runs; each gets an error result saying that parallel calls are not allowed, and the model is told to call tools one at a time.
- Timeouts distinguish connect, first byte and stream idle time. The response size limit is 2 MiB.

### 8.2 Rounds

```text
continue the workflow
→ wait for the global model permit
→ at the command boundary, read the latest settings and seal the request
→ stream the model response
→ persist the complete assistant message and usage
→ branch: business tool call / stage submission / protocol error
→ run the tool through gates and a transaction, or hold it for the user
→ when the next request is sealed, render tool results and queued feedback into it
→ next round, until a submission passes the stage guards
```

The global model permit lets one model request be in flight at a time, across all projects; queued runs are served oldest first. Each round checks the model settings again before sealing; settings that do not allow a request fail the run with the halt reason `model_not_ready`.

Review and future research agents use separate conversations; their messages never enter the writing agent's ledger.

### 8.3 Tool allowlist

| Run kind | Business tools | Auxiliary budget |
|---|---|---|
| Preparation writer | `ask_user`, `query_knowledge` (only when an embedding profile is ready), `research_web` (only when an engine is registered) | `ask_user` 0; the others 1 per call |
| Outline writer | same as preparation | same |
| Body writer | `ask_user`, `append_content`, `replace_content` | 0 |
| Review agent | none | not charged |

Each run kind additionally declares exactly one submission tool. Unavailable tools are not declared. A call to an undeclared or newly unavailable tool returns an explicit error; it is never redirected to another tool.

### 8.4 Submission tools

```text
submit_preparation      { artifacts: [...], chapter_plan: [...] }   chapter_plan required when the outline is in this round
submit_chapter_outline  { chapter_id, content }
submit_chapter          { chapter_id, summary }
submit_review           { passed, confidence, issues }
```

- The tool schema, the parser and the contract text in the system prompt come from one definition. The preparation artifact list contains exactly the artifacts required this round. Chapter-level `chapter_id` is a single-value enum fixed to the current chapter.
- A parse failure returns a paired error result with the exact reason; for value errors it states both the only valid value and the received value.
- A submission rejected by a business guard (empty body, missing artifact, invalid review result) is a **rejected round**: the model's output is kept, a paired error result and a correction are written, and the same run is re-queued. Rejected rounds do not count toward the consecutive protocol-error limit.
- A plain-text final answer without a submission call is a protocol error; the correction names the tool to call.
- A submission never overwrites a pending proposal and never replaces the whole-chapter review. The chapter summary is not body text.

### 8.5 Body editing

- `append_content({ content })` and `replace_content({ old_str, new_str })` take no project or chapter IDs; the target is the run's current chapter.
- Body text is UTF-8 plain text with LF line endings normalized on input, matched exactly afterwards (no Unicode normalization, trimming, case or whitespace folding).
- `content` must be non-empty and is appended verbatim. `old_str` must be non-empty and must match exactly once, counting overlapping matches over valid character boundaries (`aa` in `aaa` matches twice and is rejected). `new_str` may be empty (deletion).
- `old_str == new_str` returns `NO_CHANGE`: no revision, no word-count change, no approval needed.
- No match (`EDIT_NO_MATCH`), several matches (`EDIT_AMBIGUOUS_MATCH`), a stale base revision, a deleted chapter or a wrong stage never change the body.
- Edit flow: complete arguments → validate context and arguments → record the call and its base revision → compute the candidate and diff → pause/approval gate → re-validate the base on acceptance → commit revision, word-count delta, tool outcome and event in one transaction.
- Tool results are structured: `status` (`applied`, `unchanged` for `NO_CHANGE`, `rejected`, `error`), base and result revision IDs, data such as the word count, and a separate `feedback` array. Feedback is never mixed into error strings.

### 8.6 Asking the user

- `ask_user` carries a list of questions, each with a stable ID, title, kind (`single`, `multiple`, `text`), options (ID and label) and a required flag. Single and multiple choice always also accept custom text.
- Answers store `selected_option_ids` and `custom_text` separately. Single choice selects at most one option. A required question needs a selection or non-empty text.
- One call creates one question record and one pending tool call. Re-submitting the same command never answers twice.
- Answers, review decisions and feedback are separate commands; feedback during a pause never answers a question.

### 8.7 Limits and continuation

- Each run stores `request_count` and `request_allowance_end` (initially 0 and 100). A request is sent only while `request_count < request_allowance_end`; marking it dispatched and incrementing the count happen in one transaction before network I/O. Prepared-then-cancelled requests do not count; dispatched requests that failed or whose result is unknown do.
- Three consecutive protocol errors stop the run with the halt reason `protocol_error_limit`; an exhausted allowance stops it with `request_limit`.
- Explicitly continuing a stopped, failed or interrupted run sets `request_allowance_end = request_count + 100`, resets the protocol-error count and increments `resume_count`. Pausing, feedback, answers, accepted edits, reviews, reconnects and restarts never grant allowance.
- A delegated review child run gets an allowance of 3 (the protocol-error limit), because a review submits exactly one verdict but may be corrected.
- When a request fails terminally, any of its tool calls that never ran get paired error results, so the run can always continue.

## 9. Pause, feedback and approval modes

### 9.1 Sealing

Tool call states: `received → held | awaiting_review | awaiting_answer → resolved → included_in_request`. `resolved` means the outcome is persisted but not yet sent; `included_in_request` means it has been sealed into a request snapshot.

A tool's base outcome is immutable; feedback bindings are stored separately. When the next request is built, the outcome and its bound feedback are rendered into one tool message, stored in the ledger and in the request snapshot, and never rendered again (D09).

### 9.2 Feedback batches

- Free feedback and pauses apply to a chapter body that is being written (its run is queued, running or paused).
- Feedback states: `queued → bound → included`. `included` means it was sealed into a request; it promises nothing about the model's understanding.
- All `queued` and `bound` feedback of a run forms its single unsealed batch. The batch waits for or binds one tool result and keeps accepting new feedback until a request carrying that result is sealed. Sealing drains the whole batch in sequence order. Feedback arriving after sealing opens the next batch.
- Binding priority:
  1. If the batch is bound to an unsealed result, new feedback joins it.
  2. If the batch is unbound, the user submits feedback while a pause is requested but not yet held, and the latest result is still unsealed, the whole batch binds to that result and the pause request is cleared.
  3. Otherwise the first feedback records `after_tool_sequence` (the highest tool sequence with a complete result) and waits for a later result; a call already received but not finished (for example awaiting an answer) qualifies.
  4. Later feedback in an unbound batch inherits the batch anchor; when the target result completes, the whole batch binds to it.
- Example: A is bound to tool T, T is resolved but its request is not sealed; B arrives; A and B are both delivered with T. Only feedback arriving after A was sealed waits for the next batch.
- The dock shows queued and bound feedback, and separately the feedback included in the run's latest unanswered request with its delivery status (`sent`, `failed`, `unknown`, `cancelled`). After a successful response the dock clears it; records remain.

### 9.3 AutoApprove

| User action and situation | Behavior | Result |
|---|---|---|
| Pause while a request is in flight | Set `pause_requested`; keep receiving | The next complete new tool call is held before execution |
| Pause after a tool call started executing | Commands are serialized; the committed edit stands | The pause applies at the next new tool call |
| Pause while a resolved, unsealed result exists | Keep the result | Hold before the next new tool call |
| Feedback while a pause is requested but nothing is held | Queue it, clear the pause request, bind per §9.2 | No need to press continue |
| Feedback while a tool call is held | Queue it, clear the pause, resume the held call through the current gate | All queued feedback is attached to that call's result |
| First feedback with no unsealed batch | Open a batch that waits for the next new result | Never binds an older, already finished result |
| More feedback with an unsealed batch | Join the batch | The whole batch is delivered together |
| Continue while paused, without feedback | Clear the pause and resume | Queued feedback is delivered in order |
| Feedback while `ask_user` is shown | Queue only; the answer is still required | Delivered with the answer's result |

"Cancel pause" is not "approve a proposal", and feedback is not an answer.

### 9.4 No next tool call

- If the model submits the chapter while `pause_requested`, the submission is held at the submission boundary and the run pauses; continuing re-checks the completion guards.
- If the model submits the chapter while feedback is undelivered and no unsealed result exists to carry it, the submission is rejected as a rejected round (§8.4) whose result carries the feedback, and the model is told to act on it with tool calls before submitting again.
- On request failure, stop or interruption, feedback keeps its delivery state; bound feedback stays bound to its result.

### 9.5 ApproveRequired

- Every effective body change first becomes a proposal; the official body keeps its revision and the reader shows the candidate diff marked as pending. A run has at most one proposal.
- Decision rule: text that is empty after trimming Unicode whitespace and no citations means accept; any text or citation means reject with feedback. The primary button reads "Accept edit" or "Reject with feedback" accordingly.
- The free-feedback entry is different: it needs text, so empty feedback is refused as an empty field (`FIELD_INVALID`) and never approves an edit.
- Accepting re-validates the base revision and commits; rejecting leaves the body unchanged and returns the user's feedback in the tool result, and the agent proposes again.
- Per-edit approval never replaces the whole-chapter review.

### 9.6 Switching modes

| Switch | Behavior |
|---|---|
| Auto → Required, request in flight | The next complete edit uses the new mode; committed edits stand |
| Auto → Required, tool held by pause | The pause stays; after resuming, the edit goes to per-edit review |
| Required → Auto with a pending proposal | The switch does not approve it; it must still be accepted, rejected or delegated; later edits are auto-accepted (D14) |
| Any switch while `ask_user` waits | Does not answer it |
| Any switch while a review is delegated | Does not cancel the delegation or change its target |
| Switch and new call arrive together | The version's command order decides; the response returns the effective control revision |

## 10. Reviews and delegation

- Review kinds: `preparation.background`, `preparation.research`, `preparation.characters`, `preparation.outline`, `chapter.outline`, `chapter.edit`, `chapter.final`. Each review stores kind, target, target revision (or proposal), dependencies, status and decision source.
- Statuses: `open → delegated → approved | rejected`, plus `stale`. User decisions go from `open` to a final status. Cancelling or failing a delegation fails the child run and returns the review to `open`; a failure (including an interruption) is shown with its reason until the review is delegated again or decided. A review run is never continued: the user delegates the review again.
- Rejections: preparation follows the batch rule (§6); outline and whole-chapter rejections return to generation immediately; a proposal rejection rejects only that edit.
- A user's rejection of the whole chapter delivers its issues to the body run like feedback (§9.2).
- **Delegation** saves the current draft feedback and its verified citations as notes for the review agent, and creates a child run using the project's current model and the review kind's prompt. The run reviews the review's own target: an immutable revision, or the pending proposal. The review agent has no writing tools and submits exactly one structured verdict:
  - `passed` is a boolean, `confidence` a finite number in `[0, 1]`, `issues` an array of strings; all three are required and never auto-filled.
  - `passed` alone decides (D08); confidence is only displayed. With `passed = false` and no issues, the writer receives "the review failed and the review agent gave no specific issues" rather than invented issues.
  - A valid verdict applies automatically. Invalid submissions are corrected within the protocol-error limit; only transport failure, persistence failure or reaching the limit fails the delegation.
- Delegation does not consume the auxiliary budget, only model tokens, and covers only that review (D07).
- While delegated, the UI offers "cancel delegation and review myself". Cancelling invalidates the attempt; a late verdict is never applied. A verdict is applied only if the review is still `delegated`, the attempt is current and the target and dependencies still match.

## 11. Stage context folding

- **When:** after all required artifacts of the current stage are approved (by the user or a delegate). Preparation folds when all four are approved; a chapter outline folds when approved; a chapter body folds when its whole-chapter review approves; extra chapters follow the same rule. Accepting a single edit never folds.
- **Content:** the full text of the approved revisions, rendered deterministically without a model call, in fixed order (background, research, characters, outline with its chapter plan as readable text), separated by blank lines. The rendering used for folding is the same one shown for review. The fold contains no questions, tool calls, failed revisions, feedback, review reasons, flags, confidence, tokens or timestamps.
- **Transport:** the folds are sent together as one ordinary assistant text message without tool calls, right after the system prompt, never as a system instruction. New folds are appended, so earlier ones stay a stable prefix.
- **Transaction:** the approval, artifact lock, fold checkpoint, run success, next workflow state and events commit together, and only when nothing is pending (no request in flight, unresolved tool, proposal, unanswered question or undelivered feedback). Replays reuse the committed fold.
- **Assembly order:** current role and task constraints → stable tool definitions → folds of completed stages in story order → current stage input, current body and the stage's unfolded messages. A folded stage appears exactly once.
- **Limits:** when the context still exceeds the model's input budget (the context window minus the tokens reserved for the answer) after folding, the run fails with the halt reason `context_limit_exceeded`, naming the estimate and the limit. Artifacts are never summarized or truncated (D16).
- Settings and prompts are read fresh for every request; only stable blocks are kept stable, for provider cache friendliness.

## 12. Budget, knowledge and capabilities

### 12.1 Auxiliary budget

- `auxiliary_tool_budget` is a non-negative project setting (default 10). Each preparation stage and each chapter outline has its own scope that records only the used count; the limit is read from the latest settings before each charged call (D06).
- Revisions, pauses, continues and retries never create a new scope. Lowering the budget below usage leaves 0 remaining; raising it takes effect immediately.
- A charged tool is charged once per `(scope, tool_call)` when it passes validation and actually executes; no-result and service failures still count. Validation failures, missing engines and an empty budget are not charged.
- An empty budget returns `QUOTA_EXHAUSTED`; the agent may keep writing or ask the user.

### 12.2 Knowledge base

- Entries have a title, keywords, content, a space and a content revision; spaces can be created and renamed. Deleting a non-empty space is rejected. Spaces belong to no project, and live spaces have unique names because agents choose spaces by name.
- Saving an entry commits it and creates an index task. Index status per entry: `not_configured`, `pending`, `running`, `ready`, `failed` (with the reason and an explicit reindex action).
- A background indexer works through the tasks one at a time. An outcome for an entry or profile that changed while it was embedded is dropped, so a slow task never overwrites a newer one. A task in flight at shutdown is queued again at the next start. Deleting an entry removes its chunks.
- An explicit embedding configuration (API root, model, dimensions, key) with one Chat-Completions-family `embeddings` adapter is required for semantic search (D10). Without it, manual knowledge management still works.
- Chunking: paragraph-first, target 600 non-whitespace characters, hard limit 1000 characters, about 80 characters overlap, splitting long paragraphs on character boundaries.
- Vectors are stored per chunk and embedding profile (model + dimensions); changing the profile requires a new index, and search is unavailable until the new profile is ready. Old and new vectors are never mixed, and keyword filtering never stands in for semantic search.
- Search: compute the query vector once; exact cosine similarity over ready chunks of live entries in the selected spaces with the current profile; `top_k` (default 5, max 10) counts final valid results. Results include title, chunk text, entry/revision/chunk IDs, the entry's space and a relevance score (the cosine similarity, not a "confidence"). Search is refused with its reason while no embedding model is set or a newly chosen model is still indexing. Searching from the knowledge page never charges the writing budget; embedding usage is recorded.
- Title and keyword filtering is a separate browsing feature.

### 12.3 Engines

- Web research contract: question, user hints, language and run context in; findings, sources (URL, title, access time), limitations and usage out. It never writes novel text.
- Knowledge accumulation contract: user hints and target space in; candidate entries with sources out. Candidates are validated like manual entries and inserted in one transaction with their index tasks.
- Both registries are empty in the MVP. Capability queries return an empty list with a reason; invoking an unregistered engine returns `ENGINE_NOT_AVAILABLE`, creates no run and charges nothing. The UI shows "no engine available", never fake progress.

## 13. Prompts and settings

### 13.1 Built-in prompts

Ten keys, each with a system and a user prompt: `preparation.generate`, `chapter.outline`, `chapter.write`, `review.preparation.background`, `review.preparation.research`, `review.preparation.characters`, `review.preparation.outline`, `review.chapter.outline`, `review.chapter.edit`, `review.chapter.final`. Extra chapters reuse the chapter prompts with a `chapter_kind` variable.

- Built-in prompts are Chinese (§17.3). They are embedded at compile time.
- Effective prompt: project override → system override → built-in. Resetting deletes the override; an empty override is real empty content.
- Each prompt declares its variables and which of them it requires. Every variable is text; its help text and an example live in the UI catalogs. A preview renders the prompt text with those examples through the server's template engine; the agent adds the stage's material and tool instructions after the text.
- Overrides are saved per scope: system overrides quote the system settings' revision, a project's overrides quote the project's revision.

### 13.2 Template language

- Only `{{variable_name}}` text substitution: no expressions, loops, includes or code. Substitution happens once in scan order; `{{...}}` inside values is not expanded. `\{{...}}` produces a literal placeholder.
- Saving rejects unknown variables, missing required placeholders and incomplete braces; previews reject missing required runtime values.
- User prompt templates are separate reusable content (title, body, own variable descriptions) that can be inserted into guidance or feedback drafts. A template's body uses exactly the placeholders it declares. Inserting asks for a value for each, renders the template through the server's template engine and appends the text to the draft. They never override built-in prompts and never gain system-message authority. The built-in catalog and user templates share one template implementation.

### 13.3 Settings

- System: model service API root and key, default model ID with its capability profile, context limits, embedding configuration, built-in prompt overrides.
- Project: model reference (model ID with its own capability profile), thinking level, auxiliary budget, writing language, project prompt overrides. Delegated reviews use the project's model.
- Every request resolves the effective model (project, then system default), the effective prompts, and validates capabilities (tools and streaming are required for every stage; the thinking parameter must be supported if used) before sending. The same check runs before accepting HTTP commands.
- A request snapshot stores the settings revision, rendered prompts with hashes, model ID, capabilities, effective thinking parameter, tool definitions, context sources, the final message array and the provider request ID. Keys are read only right before sending.
- The connection test is an explicit real request with the model ID and capability profile under test; if thinking is declared, it really sends `reasoning_effort`. Static validation never shows "connected". The embedding test also checks output dimensions and finite values and names the exact failure.
- Saving a key: omitted keeps the stored key, `null` clears it, a string replaces it.

## 14. Projects, versions and deletion

- Creating a project creates its first version. A version has a label and a novel title. New versions are empty (D03, D15). The project's selected version is a display choice only; switching it never starts generation.
- Soft delete applies to projects, versions, knowledge spaces and entries, and prompt templates. Restore is offered for projects. Deleting a parent does not rewrite its children; queries check ancestors. Plan chapters cannot be deleted; extra chapters can be soft-deleted when nothing is active.
- Deleting a project or version with an active run or open review is refused with `STATE_CONFLICT` (`run_active`). The selected version cannot be deleted.
- Clearing a prompt override deletes the override row.

## 15. Dashboard

- **Token usage:** from actual requests, grouped by project, time and role (writing, review, embedding; connection tests, which belong to no project, are recorded and shown separately). Fields: input, output, cached input (a subset of input), reasoning (a subset of output), with `usage_status` (`reported`, `unknown`, `invalid`) and a typed unknown reason. Totals use known input + output; unknown requests are counted separately. Embedding input is listed separately, never mixed into conversation totals. No pricing.
- **Running jobs:** each version's current run when it is queued, running, waiting for the user, waiting for review or paused, each labeled; interrupted, stopped and failed runs, which wait to be continued, in a needs-attention list with the reason. Top-level runs are counted; a review agent's run is shown under its parent. A job links back to its version and names its chapter.
- **Word count growth:** word count is the number of UAX #29 words (`unicode_words`): each CJK ideograph counts as one word, other scripts count words, punctuation and whitespace are excluded (replaces D04). Each body revision records `delta = new - old` (may be negative); rejected edits and replays record 0. Titles, outlines, settings and model reasoning are excluded. The chart shows net growth; the current total of visible versions is shown separately.
- Ranges: today by the hour, and the last 7 (the default) or 30 days by the day. The client computes the bucket boundaries in the user's time zone and sends them as UTC instants; the query counts records in those half-open intervals. A project filter narrows every metric except connection tests. Only live projects and versions count. All metrics are computed from records, never from stored counters.

## 16. Local runtime

### 16.1 Process and data

- Data lives in the OS application-data directory under `NekoWriter`, or in `--data-dir` (resolved to an absolute path): the SQLite database, `logs/` and a process lock file. A second process cannot open the same data directory.
- The server binds `127.0.0.1` on port 47821 by default (`--port`); a busy port is an error. After binding, the default browser opens unless `--no-browser` is given; a failure to open is only logged.
- The schema is created from the current code on first start and stores its version. Any other version is refused with an actionable message naming the data directory, both versions, that no data was read or changed, and the next steps. No migrations exist. Every startup failure (directory, logs, lock, database, port) uses the same actionable reporting.
- SQLite runs with foreign keys, WAL, `synchronous=FULL` and a bounded busy timeout.
- Logs roll daily and keep 7 files; they record request, run, stage, tool, duration and error, but never full body text, prompts or keys.

### 16.2 Commands, events and safety

- Every mutation carries a `command_id`; state-dependent commands carry an expected revision. A replay of the same command returns the original result; the same ID with different content returns `IDEMPOTENCY_CONFLICT`. Receipts are kept permanently.
- Errors are `{code, params, requestId}` with 422 for invalid input, 409 for state or revision conflicts, 404 for missing or deleted resources, and 502/504 for external service failures.
- Events are persisted with a sequence number in the same transaction as the change and streamed over SSE. The client takes a snapshot with its cursor, subscribes after it, and resumes with `Last-Event-ID`. Persistent events are kept 7 days; a cursor outside the window gets `stream.reset_required`. Heartbeats every 15 seconds. Streaming previews are ephemeral, unsequenced and dropped for slow subscribers.
- Views name document revisions by ID, number and word count. A revision's content is read by its ID; revisions never change, so clients keep what they read.
- Write requests require a loopback `Host` and, when present, an allowed `Origin` (the app itself and the Vite dev server). GET requests are read-only. Markdown never renders raw HTML; external links open safely; export filenames are sanitized.

### 16.3 Interruption, stop and shutdown

- Checkpoints: request prepared, response persisted, tool persisted, proposal pending, waiting for an answer, tool outcome committed, feedback bound, request sealed, review decided, fold committed.
- On startup, stable waits (questions, proposals, pauses) are restored; automatically executing states become `interrupted` and nothing is sent until the user continues.
- Continuing: unsent request → rebuild from latest settings; uncertain request → mark `unknown` and create a new attempt; persisted but uncommitted tool → re-run through the current gate; committed edit not yet seen by the model → reuse the stored outcome; bound feedback → still bound; committed fold → build from the fold.
- Stop: no further requests, cancel the in-flight request, keep committed text and undelivered feedback, mark the run stopped; late callbacks are ignored. Pending proposals stay pending. Stop, pause, feedback and mode switches are control commands: they quote no workspace revision, because the agent advances the workspace while they are being made, and they apply in the version's command order.
- A failure inside NekoWriter while it carries out a run's step fails the run with the halt reason `internal` instead of leaving it queued; the log has the details and the user can continue the run.
- Shutdown (Ctrl+C, SIGTERM or the UI's "exit service" with confirmation): stop accepting connections, cancel in-flight requests, mark active runs interrupted and requests cancelled or unknown in one transaction, checkpoint the WAL, close the database. Startup recovery and shutdown share one implementation. The UI then shows "service stopped" with restart instructions.
- Diagnostics: the health endpoint and the settings page show the version, the UI address, the data directory and the log directory.

### 16.4 Export

- Markdown and TXT export of a version's committed bodies, in plan order followed by extra chapters, with the novel title and chapter labels, from one consistent read.
- A chapter is final only when its approved revision is the current one; otherwise it is labeled as a draft in text. Chapters without a body say so.
- The header line states the export time (UTC) and how many plan chapters passed their whole-chapter review.
- Export text (header, labels, draft markers) is in the version's writing language.
- Excluded: proposals, previews, prompts, token usage, keys, review notes and run records.
- Filename: the sanitized novel title (path separators, control and reserved characters removed, at most 80 characters, `nekowriter-export` when empty), with an ASCII fallback and an RFC 5987 UTF-8 filename in `Content-Disposition`.

## 17. Internationalization

### 17.1 UI language

- The UI ships in English (`en`) and Simplified Chinese (`zh-CN`). Any `zh*` browser language selects `zh-CN`; anything else selects `en`. The user can switch in settings; the choice is remembered in the browser.
- All user-facing text comes from the UI catalogs, including error messages (rendered from error codes and parameters), enum labels and prompt-variable help. The backend never sends user-facing prose.
- Numbers and dates use the active locale; `<html lang>` follows it.

### 17.2 Writing language

- Each project has a writing language: `zh-CN` or `en`. It is chosen at creation (defaulting to the UI language) and can be changed in project settings; each request uses the latest value.
- The writing agents must write all novel prose in the writing language. Review agents write their issues in the writing language.
- Export text follows the writing language.

### 17.3 Model-facing text

- Built-in prompts, tool descriptions, tool results, corrections and context labels are Chinese and live in resource files. The writing language reaches the model through the `writing_language` prompt variable, which is required in the system prompts of `preparation.generate`, `chapter.outline` and `chapter.write`.

### 17.4 Repository language

- Code, comments, logs, developer messages and documents are English. Non-ASCII text is allowed only in locale and model-text resource files.

## 18. UI and UX

### 18.1 Information architecture

- Top-level pages: Works (library), Dashboard, Knowledge, Templates, Settings. The workspace is opened from a work and has no global sidebar.
- Routes: `/works`, `/works/:projectId/versions/:versionId`, `/dashboard`, `/knowledge`, `/templates`, `/settings`; unknown paths redirect to `/works`.
- First use: configure and test the model → create a project → enter the novel brief → start preparation. Embedding is configured separately; research and accumulation show "no engine available".
- Work cards show the name, current version, stage, recent activity, main-chapter progress, extra-chapter count and word count.

### 18.2 Workspace layout

- Header: project and version switcher, novel title, stage progress, save status, project settings.
- Left: chapter list (preparation entry, then chapters with status). Clicking scrolls the single continuous reader to the chapter, including chapters not yet loaded.
- Center: the continuous reader of committed body text, pending diffs and streaming preview, each visually distinct.
- Bottom: the context dock with status, citations, the current input and one clear primary action. The dock is not a generic chat box.
- Desktop: chapter list 220–260px, text width 760–840px, dock 200–280px tall and resizable. Below 1100px the chapter list collapses; below 768px it becomes a drawer.
- Reading typography: a serif reading face at 18–20px with a line height around 1.85; light and dark themes from CSS tokens.

### 18.3 Dock states

| State | Content | Primary action |
|---|---|---|
| Agent question | Single/multiple choice with custom answers, or text | Submit answer |
| Preparation review | Four tabs (background, research, characters, outline), each with status and feedback | Approve or request changes; delegate |
| Next chapter | Chapter goal and guidance input | Start outline |
| Outline review | Outline preview, citations, feedback | Approve; delegate |
| AutoApprove writing | Feedback, queue state, mode switch | Submit feedback; pause |
| Pause requested | Pause state, feedback | Submit feedback and continue; cancel pause |
| Paused | Held tool summary, feedback | Continue; submit feedback and continue |
| Per-edit review | Diff summary, citations, feedback | Accept edit (empty) or reject with feedback; delegate |
| Whole-chapter review | Chapter summary, full text, feedback | Approve chapter; delegate |
| Execution problem | Reason, preserved content, undelivered feedback | Fix settings and continue, or stop |
| All chapters approved | Completion prompt | Confirm completion |

Button labels name the action; a generic "Send" never hides the difference between approving and rejecting.

### 18.4 Reading, previews and citations

- The reader follows new content only when the user is at the end and not selecting or reviewing a diff; otherwise it keeps the viewport and shows a "new content" button. Streaming updates are batched per frame.
- Free feedback, review rejections, proposal rejections and delegation notes can cite passages: at most 20 each, in addition to or, except for free feedback, instead of text.
- A citation stores its target, start and end offsets (Unicode scalar values, half-open) and the quote. The target is a revision of this version's documents, or the text a pending proposal would produce, named by tool call and candidate hash. Browser UTF-16 offsets are converted explicitly. A citation stays within one text; cross-chapter selections cannot be cited.
- The backend verifies every quote against its target when the command arrives and refuses the command otherwise (`CITATION_INVALID`). The writer and the review agent receive each quote labelled with where it comes from.
- Diffs use color, symbols and text labels together.

### 18.5 Drafts and accessibility

- Unsent inputs (brief, guidance, review feedback, free feedback) are saved in the browser per data directory, version, phase and target, and cleared only after the server confirms the command. A local draft that conflicts with a newer server revision is kept and flagged.
- "Saved" appears only after the server confirms; local drafts say "draft saved in this browser".
- All major actions work from the keyboard. Ctrl/Cmd+Enter submits the current form and never fires during IME composition. Escape closes only non-destructive layers. Variable help icons are keyboard-focusable. Dialogs manage focus.
- Failures (model, index, missing engines) show the real reason; nothing hides behind an endless skeleton.

## 19. MVP checklist

Each line maps to the milestone that delivers it (M0–M5 in the build plan).

| # | Requirement | Section | Milestone |
|---|---|---|---|
| 1 | Workspace, schema v1, version check, process lock, logging, loopback server, embedded UI | §16.1 | M0 |
| 2 | Commands with idempotency, events with SSE cursors, typed error codes | §16.2 | M0 |
| 3 | Project create, rename, list, soft delete, restore; first version | §14 | M0 |
| 4 | UI in en and zh-CN with switching; writing language chosen at creation | §17 | M0 |
| 5 | Diagnostics and explicit shutdown | §16.3 | M0 |
| 6 | System model settings, capability check, connection test; project model and thinking level | §13.3 | M1 |
| 7 | Versions (create empty, select) | §14 | M1 |
| 8 | Preparation: brief, four artifacts, chapter plan, review tabs, revise, dependencies, stale | §6 | M1 |
| 9 | Preparation fold | §11 | M1 |
| 10 | Model adapter, rounds, submission tools, protocol corrections, request allowance | §8 | M1 |
| 11 | Chapter guidance, outline and review, body tools, whole-chapter review, next chapter, completion, extras | §7 | M2 |
| 12 | Outline and body folds; context assembly; `context_limit_exceeded` | §11 | M2 |
| 13 | Workspace reader, chapter list, live preview | §18.2 | M2 |
| 14 | Pause and feedback batches; AutoApprove matrix | §9.1–§9.4 | M3 |
| 15 | ApproveRequired proposals and diffs; mode switching | §9.5–§9.6 | M3 |
| 16 | `ask_user` | §8.6 | M3 |
| 17 | Review delegation for every review kind | §10 | M3 |
| 18 | Stop, continue, interruption recovery | §16.3 | M3 |
| 19 | Citations | §18.4 | M3 |
| 20 | Knowledge spaces and entries, embedding settings, indexing, search, `query_knowledge`, budget scopes | §12 | M4 |
| 21 | Research and accumulation interfaces with empty registries | §12.3 | M4 |
| 22 | Prompt catalog, overrides, variable help, preview; user templates | §13 | M4 |
| 23 | Dashboard | §15 | M4 |
| 24 | Export | §16.4 | M4 |
| 25 | README and architecture document | — | M5 |
