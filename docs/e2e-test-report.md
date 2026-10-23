# End-to-end test report

Run of `docs/e2e-test-plan.md` against the release build of NekoWriter, using the
`agent-browser` skill (real Chromium, headless). Date: 2026-10-14.

## What was under test

| | |
|---|---|
| Build | `target/release/nekowriter`, built 2026-10-14 16:06 (+0800), web UI embedded (`uiEmbedded: true`) |
| Data directory | `/tmp/nw-e2e/data` (fresh) |
| Address | `http://127.0.0.1:47821/` |
| Browser | agent-browser 0.39.0, Chrome 155, headless, `--no-sandbox`, 1440x900 unless a case says otherwise |
| Model service | local shim `http://127.0.0.1:47830/v1` forwarding to `https://opencode.ai/inference/go/v1` |
| Model | `longcat-2.5-preview-free` (context window 128000, 8192 reserved for the answer) |
| Languages | every case in English; the `[both]` cases (B1, B2, B3) also with the interface in Simplified Chinese |

Notes on the model service, because they shape several results:

* Only `longcat-2.5-preview-free` and `step-5-preview-free` may be used. `step-5-preview-free`
  answered `server_error: Endpoint is unavailable` / HTTP 429 during the run, so
  `longcat-2.5-preview-free` was used for all model work.
* The shim used for this run is kept as `docs/e2e-model-shim.py`. The gateway streams a non-standard `reasoning_content` delta and requires an
  `x-opencode-session` header that NekoWriter cannot send. The shim adds that header and
  strips `reasoning_content` (an extension NekoWriter does not know), so the app sees a plain
  Chat Completions stream. Everything else - including tool calls and SSE - passes through
  unchanged. The shim is a test fixture; it is not part of the product.
* NekoWriter accepts at most 2 MiB per answer. `longcat-2.5-preview-free` writes the
  preparation documents as tool-call arguments and can produce very large single answers; two
  early attempts died with `the response exceeded 2097152 bytes` before the runs were retried.

Status codes used below: **pass**, **fail**, **blocked** (a service the case needs is
missing), **not verified** (the case was started but the expected behaviour was not observed
in the available time - it is not a pass).

## Results

| ID | Result | Language | Note |
|---|---|---|---|
| A1 | pass | en | `/api/health` has `uiEmbedded: true`; Works loads; Settings > This installation shows `/tmp/nw-e2e/data` and `/tmp/nw-e2e/data/logs`; `html lang="en"`; console clean |
| A2 | pass | en | Second instance on the same data dir exits 1: "Another NekoWriter process is using the data directory /tmp/nw-e2e/data"; first keeps serving |
| A3 | pass | en | Other data dir on port 47821 exits 1: "... could not listen on 127.0.0.1:47821: Address already in use (os error 98) ... start NekoWriter with --port <number>" |
| A4 | pass | en | Exit via Settings: page shows "NekoWriter has stopped", process exit status 0, log "shutdown requested through the web UI" then "NekoWriter stopped"; after restart both works are still there |
| A5 | pass | en | SIGINT: process gone, log ends with "NekoWriter stopped" |
| A6 | pass | en | `Origin: https://evil.example` -> 403 `UNTRUSTED_ORIGIN`; `Host: example.com` -> 403 `UNTRUSTED_HOST` |
| B1 | pass | en + zh-CN | New profile with locale zh-CN -> Chinese UI, `html lang="zh-CN"`, heading 作品; new profile en-US -> English, `html lang="en"` |
| B2 | pass | en + zh-CN | Interface language switch applies at once, survives reload, `html lang` follows it, in both directions |
| B3 | pass | en + zh-CN | Light/dark/system. Text/background contrast computed on every page: dark min 5.86 (en) / 6.21 (zh), light min 4.35; no element below 2.0. System theme follows `prefers-color-scheme` |
| C1 | pass | en | "Chinese Work" (novel 雨天来信, Simplified Chinese) and "English Work"/"Rain Letters" created; opening one shows its workspace with preparation not started ("Start preparation" disabled, "Save the brief before starting.") |
| C2 | pass | en | Empty name -> "Work name is required."; 101 characters -> "Work name can be at most 100 characters."; both next to the field |
| C3 | pass | en | Rename shows the new name in the card, the toast and `/api/projects` |
| C4 | pass | en | Delete -> confirmation -> only in the Deleted tab ("Deleted Oct 14, 2026, 4:58 PM"); Restore brings it back |
| D1 | pass | en | API root, key and default model saved; after reload the key field is empty with "A key is stored. Enter a new one to replace it."; `/api/settings` returns `apiKeySet: true` and no key value |
| D2 | pass | en | "Connected. The test answer took 2.1 s."; wrong root -> "The test failed: the service answered with HTTP status 404."; wrong key -> HTTP status 401; both logged with request IDs |
| D3 | pass | en | "Remove the stored key" -> "The stored key is removed when you save." -> Save -> "Leave empty for services that need no key.", `apiKeySet: false` |
| D4 | blocked | - | `[emb]`: the gateway has no embeddings endpoint (`POST /v1/embeddings` -> 404; no embedding model in `/v1/models`) |
| D5 | pass | en | Research and accumulation: Web research = "No engine available", Knowledge accumulation = "No engine available" |
| D6 | pass | en | Writing language, own model, thinking level and auxiliary budget saved; 1001 -> "Auxiliary tool budget must be between 0 and 1000."; thinking on for a model without thinking support -> warning "The model this work uses does not support thinking." |
| E1 | pass | en | "Draft saved in this browser" while typing; the draft survives a reload; "Start preparation" stays disabled until the brief is saved |
| E2 | pass | en | Run dock shows "Preparing"/"Writing", a live preview that streams the background document, and the request counter; the result is four tabs (Background, Research, Characters, Outline) each "To review", with a three-chapter plan in the outline, written in the work's writing language |
| E3 | pass | en | After the start the brief panel shows the saved brief as read-only text (no textarea) |
| E4 | pass | en | Empty feedback leaves "Request changes" disabled; rejecting the outline marks it "Changes requested" while the other three stay "To review"; after all four are decided the run revises and "Outline Revision 2 To review" comes back following the feedback, the other three staying Approved |
| E5 | not verified | - | The English work was completed in one round before this round could be staged (see E8). Needs a fresh preparation round |
| E6 | not run | - | Needs a preparation review round; not reached in the available time |
| E7 | not run | - | Needs a preparation review round; not reached in the available time |
| E8 | pass | en | All four approved -> preparation complete, chapters "Salt on the Envelope", "What the Light Keeps", "Answers" appear, all Not started |
| F1 | pass | en | Chapter guidance -> "Start outline"; rejecting the outline with feedback produced a revised outline; "Approve outline" moved to drafting |
| F2 | not verified | en | The dock word count updates while the agent writes (0 -> 517 -> 733 -> 1,729 -> 2,576) and the reader holds the committed chapter text, but the "Being written" block and the "New content below" notice were not observed |
| F3 | pass (partial) | en | "Send feedback" while writing queues the item: "Waiting for the next result" -> "Sent; waiting for the answer"; the applied feedback edited the chapter (2,576 -> 2,009 words). The middle state "Goes with the next request" was not observed |
| F4 | pass | en | Pause -> "The agent pauses before its next change." with "Cancel pause" and "Send feedback and continue"; the pause then took effect as "Paused before this change: Add text to the end of the chapter."; Continue applied the held change (0 -> 1,019 words); Pause + Cancel pause let writing go on |
| F5 | pass | en | A stopped run is explained ("You stopped the run. Its text and your feedback are kept; continue when you are ready.") and Continue resumed writing with the committed text intact |
| F6 | pass (partial) | en | With "Review each edit before it is applied" on, the next edit came as a proposal: "The agent proposes adding text to the end of the chapter.", the added text rendered as a green `<ins>` marked "Added:", an empty feedback box leaves "Accept edit", any feedback turns it into "Reject with feedback", and switching back to automatic left the proposal pending. Accepting applied it (1,019 -> 1,560 words). No removal occurred, so the struck-through/-/red case and citing only proposed text are unverified |
| F7 | pass (partial) | en | Chapter 1 approved -> "Chapter 1: Salt on the Envelope is approved." + "Next chapter"; Chapter 2 went through outline and drafting the same way, then its run failed with the 2 MiB limit and was retried. The final chapters and "Confirm the novel complete" were not reached |
| F8 | not run | - | Needs a completed novel (an extra chapter can only be added after completion) |
| F9 | not run | - | Not reached in the available time |
| F10 | not run | - | Not reached in the available time |
| F11 | pass | en | Clicking a chapter scrolls the reader to it; at 1440px the chapter list is a 240px column, below 1100px it collapses behind "Show contents", at 390px that opens it as a drawer |
| F12 | pass (partial) | en | "The agent asks" appeared with the question, a "Required" marker, "Your answer" and "Submit answer"; a free-text answer continued the run. The empty-answer case did not block "Submit answer" (see below) |
| G1 | pass | en | "New version..." -> "Create version" opened an empty version with its own title and brief, preparation not started |
| G2 | pass | en | Switching versions shows each version's own title, brief and stage |
| G3 | pass | en | "Delete versions..." lists only the versions other than the shown one; "Delete version 1" removed it ("There is no other version to delete.") and the menu offers only Version 2 afterwards |
| G4 | not run | - | Needs two versions with an active run; not reached in the available time |
| H1 | pass | en | Prompt tabs show the built-in Chinese text, "In effect: the built-in text.", and the placeholder list with Required/Optional, help and examples |
| H2 | pass | en | Editing the Preparation system prompt for All works -> "In effect: the system's prompt."; survives reload; "Reset to the built-in text" restores it |
| H3 | pass | en | Removing `{{writing_language}}` -> "The placeholder “writing_language” is required here."; adding `{{foo}}` -> "The placeholder “foo” is not available here." |
| H4 | pass | en | "Preview with examples" renders the prompt with the example values |
| H5 | pass | en | Work-scoped override -> "In effect: this work's prompt."; its reset (labelled "Reset to the system's prompt") falls back to the system prompt, and the system override resets to the built-in text |
| H6 | not run | - | Not reached in the available time |
| H7 | pass | en | Template created with two placeholders and descriptions; an undeclared placeholder is refused ("The placeholder “foo” is not available here."); edit works; delete asks "Delete “Greeting edited”?" and removes it |
| H8 | **fail** | en | "Insert template" -> "Insert" does not append the filled text to the draft; it submits the surrounding feedback form instead (details below) |
| I1 | pass | en | Spaces: create, rename, duplicate name refused ("Another space already has this name."), non-empty delete refused ("The space still has entries. Move or delete them first."), empty space deleted |
| I2 | pass | en | Keywords split on ASCII comma, U+FF0C and U+3001 -> four keywords; filtering by title and by keyword in another case matches, unknown text does not; edit and move show at once; delete works |
| I3 | pass | en | Entry says "Not searchable: no embedding model is set"; the search says "Search by meaning needs an embedding model. Set one in Settings." |
| I4-I7 | blocked | - | `[emb]`: no embeddings service is available |
| J1 | pass | en | Today / Last 7 days / Last 30 days and All works / one work switch consistently |
| J2 | pass | en | A running run is listed under "Working or waiting" with a link to its version; a stopped run under "Needs attention" with "Stopped / Preparation / You stopped the run. Its text and your feedback are kept; continue when you are ready." |
| J3 | pass | en | "Words in all chapters now" = 2,843 equals the reader's chapter counts (1,824 approved + 1,019 in progress); today's bar shows +2,843 |
| J4 | pass | en | Rows for Writing, Review agents, Connection tests and Embeddings, with Unknown counts and "Requests with unknown usage: ... never counted as zero" |
| K1 | pass | en | `Rain: Letters?` downloads as `Rain Letters.md`/`.txt`; the Chinese title survives as `filename*=UTF-8''雨天来信.md`/`.txt` with an ASCII fallback |
| K2 | pass (partial) | en | The English export has the header, "Exported <date>. 0 of 3 planned chapters passed their whole-chapter review.", plan chapters in order, "*Draft: not approved yet.*" for the unapproved chapter and "*Not written yet.*" for the unwritten ones, with no feedback, notes or prompts. The Chinese export header/labels are Chinese; a Chinese work with chapters was not produced |
| L1 | not verified | en | Could not present a stale settings form: the open form is kept in sync with the server, so every cross-tab save was accepted (see below) |
| L2 | not run | - | Requires stopping the service while a page is open; not done because it would have aborted the running model work |
| L3 | pass (partial) | en | Double-clicking "Create work" created exactly one work, and double-clicking submit buttons did not duplicate anything observed. Approve/send-feedback double-clicks were not exercised |

### Cross-cutting checks

| Check | Result | Note |
|---|---|---|
| No raw message keys | pass | No key-like strings (`workspace.chapter.start`) appeared on any screen visited |
| No English text in the Chinese interface | pass | Pages visited in Chinese (Works, Dashboard, Prompts, Knowledge, Settings, workspace) showed no English interface text; model IDs, URLs and the novel's own text are exempt |
| `<html lang>` follows the interface language | pass | `en` / `zh-CN`, and numbers and dates use the corresponding format |
| No console errors or failed requests | pass | Console and page errors were empty on every page checked; the only 4xx answers were the ones D2/A6 expect |
| No horizontal page scroll at 390px | **fail** | All five top-level pages scroll horizontally (details below); the workspace page itself does not |

## Failures

### H8 - inserting a template submits the feedback form

Steps: open a feedback form (the writer's "Feedback for the writer", and the chapter review's
"What should change?"), type some draft text, click "Insert template", choose "Motif reminder",
fill `motif` with `lamp`, click "Insert".

Expected: the filled text is appended to the draft after a blank line and nothing else happens.

Happened, twice, in both forms: the dialog closed and the surrounding feedback form was
submitted. The dock then showed `Your feedback / <the draft text> / Sent; waiting for the
answer` (writer form) and the app left the review state (review form), while no draft ever
contained `Keep the lamp motif in the background.`. The "Insert" button behaves as the form's
submit button.

Evidence: screenshot `docs/e2e-screenshots/h8-insert-dialog.png`. No server log line - the
request itself is valid; the bug is in the dialog.

### 390px: horizontal page scroll

Steps: set the viewport to 390x844 and open Works, Dashboard, Prompts, Knowledge, Settings and a
workspace.

Expected (checks on every screen): no horizontal page scroll.

Happened: on all five top-level pages `document.documentElement.scrollWidth` is 551 while
`window.innerWidth` is 390. The cause is the main navigation (`nav.flex.gap-1`, 424px wide, its
right edge at 551px), which neither wraps nor collapses. The workspace page itself does not
overflow (scrollWidth 390).

Evidence: `docs/e2e-screenshots/works-390.png`, `docs/e2e-screenshots/workspace-390.png`.

### F12 - a required question does not block "Submit answer"

The dock appeared as expected ("The agent asks", the question, "Required", "Your answer",
"Submit answer"), free text was accepted and submitting it continued the run. However, with an
empty answer `Submit answer` was enabled (`agent-browser is enabled` returned true). The plan
expects a required question to block submission. I did not submit an empty answer, so the
server-side behaviour for an empty required answer is unknown.

Evidence: `docs/e2e-screenshots/f12-question.png`.

## Not verified / not run

* **E5, E6, E7, H6** need a fresh preparation round with several open reviews. The English
  work's preparation was finished in one round (E4 -> E8) before those rounds could be staged;
  the Chinese work's preparation was not reached.
* **F2** - the "Being written" block and the "New content below" notice were not observed, so
  their presence and behaviour are unverified even though the word count and reader content
  behaved as expected.
* **F6** is verified for additions, both button labels, the delegation button's presence and the
  switch-back-to-automatic case; the removal case (struck through, `-`, red), citing only
  proposed text and delegating a proposal review were not exercised.
* **F8** needs a completed novel, **F9/F10** need process kills and restarts mid-run, **G4**
  needs two versions with active runs; none was reached in the available time.
* **K2** - the Chinese half needs a Chinese work with plan chapters, an extra chapter and
  unapproved/unwritten chapters. Only its header/labels could be checked in Chinese.
* **L1** - the plan's stale-form scenario could not be staged: the open settings form followed
  the server (a value typed in one tab was replaced by the other tab's saved value, and the save
  that followed carried the newer `expectedRevision`), so every cross-tab save was accepted.
  Blocking `/api/events` did not stop the refresh. The API takes
  `{commandId, expectedRevision, ...}` and returned 200 for an up-to-date revision.
* **L2** - not attempted while a model run was in flight; A4 covers the stop/restart part
  without the "Reconnecting to NekoWriter..." banner.
* **H6** was not run; **I7** is blocked on embeddings (see above).

## Blocked

* **D4, I4, I5, I6, I7** (`[emb]`): no embeddings service. `POST /v1/embeddings` on the gateway
  answers 404 for every model tried and `/v1/models` lists no embedding model. The knowledge
  base correctly degrades to "Not searchable: no embedding model is set" (I3).

## Language coverage

Every case above ran with the interface in English. B1, B2 and B3 were run again with the
interface in Simplified Chinese as the plan requires. C, D, H, I, J and K step through English
only; their screens were also seen in Chinese during the theme and language passes, without
repeating the steps, so the `[both]` requirement is only partly met for those sections.

## Model-service failures observed

These are recorded because they were the reason several runs had to be retried, not because
they are NekoWriter defects:

| Log line | Meaning |
|---|---|
| `the model request failed ... elapsed_ms=278394 error=the response exceeded 2097152 bytes` | the model's single streamed answer passed the 2 MiB limit |
| `the model request failed ... error=the model service sent an invalid response: {"message":"Streaming response failed: [server_error] request failed",...}` | the gateway dropped a stream mid-answer |
| `the model request failed ... error=the model service sent an invalid response: Transport error: error decoding response body` | the gateway stream ended malformed |

Two attempts with `deepseek-v4.1-flash` and `glm-5.3` (used before the model restriction was
given) failed the same way; the final model work used `longcat-2.5-preview-free` with the
reasoning-stripping shim.

### The 2 MiB answer limit and streaming overhead

NekoWriter stops a request when the answer passes 2 MiB, and it sends no `max_tokens` (the shim
logged `max_tokens=None` for every app request). The gateway emits one SSE event per token, and
an event is roughly 200 bytes even when its delta carries a few characters. Measured with the
shim in place, one app request held 2,180,915 bytes of stream for 121,006 characters of
reasoning and no content or tool arguments at all - the bytes were almost entirely per-event
envelope. So the cap is reached after roughly ten thousand streamed tokens, including
`reasoning_content` that NekoWriter neither asks for nor uses. On a real OpenAI-compatible
service with per-token events this makes the 2 MiB limit easy to hit for long answers; whether
that is intended is a product question, not something this plan tests.

## Reproducing the run

```sh
# app
./target/release/nekowriter --data-dir /tmp/nw-e2e/data --port 47821 --no-browser

# model shim (adds x-opencode-session, strips reasoning_content)
python3 docs/e2e-model-shim.py 47830
# Settings > Model service: API root http://127.0.0.1:47830/v1, key, model longcat-2.5-preview-free,
# context window 128000, 8192 reserved for the answer

# browser
export AGENT_BROWSER_SESSION=nekowriter-e2e
export AGENT_BROWSER_ARGS="--no-sandbox,--disable-dev-shm-usage"
agent-browser open http://127.0.0.1:47821/
```

Screenshots for the failures and for the theme work are in `docs/e2e-screenshots/`. The app's
log for this run is `/tmp/nw-e2e/data/logs/nekowriter.2026-10-14.log`; error answers in the UI
carry a request ID that also appears there.
