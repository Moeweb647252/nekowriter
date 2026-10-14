# End-to-end test plan

A manual or browser-automated pass over the release build of NekoWriter, checking the MVP against `docs/product-spec.md`. Each case gives the steps and what must happen. Section numbers (§) refer to the spec.

## Setup

1. Build the release binary with the web UI embedded:
   ```sh
   pnpm --dir webui install
   pnpm --dir webui build
   cargo build --release -p nekowriter-app
   ```
2. Start it on a fresh data directory and keep the path; logs are in `<data-dir>/logs/`:
   ```sh
   DATA=$(mktemp -d)
   ./target/release/nekowriter --data-dir "$DATA" --port 47821 --no-browser
   ```
   Open `http://127.0.0.1:47821/`.
3. Drive a real browser (for example Playwright), 1440×900 unless a case says otherwise.
4. Services:
   - Part A, B, C and most of H and I need no model.
   - The other parts need an OpenAI-compatible Chat Completions service with tool calling and streaming: its API root including `/v1`, a key, a model ID and its context window. Ask the user for them; never write a key into a file or a report.
   - Cases marked **[emb]** also need an OpenAI-compatible `embeddings` endpoint and the model's vector size.
5. Languages: run every case in English. Run the cases marked **[both]** again with the interface in Simplified Chinese (Settings → Interface language).

### Checks on every screen

- No raw message keys such as `workspace.chapter.start`.
- In Simplified Chinese, no English interface text. Model IDs, URLs, user content and the novel's own text are exempt.
- `<html lang>` matches the interface language; numbers and dates are formatted for it.
- No errors in the browser console, and no failed requests except the 4xx answers a case expects.
- At 390px width, no horizontal page scroll.

## A. Runtime (no model)

| ID | Steps | Expected |
|---|---|---|
| A1 | Fresh start. `curl http://127.0.0.1:47821/api/health`. Open Settings → "This installation". | Health shows `uiEmbedded: true`. The Works page loads. Settings shows the data and log directories of `$DATA`. |
| A2 | Start a second instance with the same `--data-dir` and another port. | It exits with an error naming the data directory. The first keeps running. |
| A3 | Start an instance with another data directory on port 47821. | It exits with an error naming the address and suggesting `--port`. |
| A4 | Settings → "Exit NekoWriter" → confirm. Restart on the same data directory. | The page shows "NekoWriter has stopped". The process exits with status 0. After the restart, all data is still there. |
| A5 | Send Ctrl+C (SIGINT) to the process. | It stops cleanly; the log ends with "NekoWriter stopped". |
| A6 | `curl -X POST http://127.0.0.1:47821/api/projects -H 'Origin: https://evil.example' -H 'content-type: application/json' -d '{}'`, then `curl -H 'Host: example.com' http://127.0.0.1:47821/api/health`. | Both get 403 (`UNTRUSTED_ORIGIN`, `UNTRUSTED_HOST`). |

## B. Interface language and theme [both]

| ID | Steps | Expected |
|---|---|---|
| B1 | Open the app with browser locale `zh-CN`, then with `en-US`, each in a new profile. | Chinese, then English. |
| B2 | Switch the language in Settings, then reload. | The choice survives the reload; `<html lang>` follows it. |
| B3 | Switch the theme between light, dark and system. | Every page stays readable; nothing is invisible in dark mode. |

## C. Works library [both]

| ID | Steps | Expected |
|---|---|---|
| C1 | "New work": name, novel title, writing language Simplified Chinese. Then a second work in English. | Both appear. Opening one shows its workspace, with preparation not started. |
| C2 | Create with an empty name, then with a name over 100 characters. | Localized field errors next to the name. |
| C3 | Card actions → Rename. | The new name shows everywhere. |
| C4 | Card actions → Delete → confirm. Show "Deleted" → Restore. | The work moves to Deleted and comes back. |

## D. Settings

| ID | Steps | Expected |
|---|---|---|
| D1 | Model service: API root, key and a default model, then Save. Reload. Inspect the `/api/settings` response. | Saved. The key field is empty with "A key is stored. Enter a new one to replace it." No response contains the key. |
| D2 | "Test connection" with correct settings, then with a wrong API root path, then with a wrong key. | Success with the elapsed seconds; then localized errors naming HTTP 404 and 401. |
| D3 | "Remove the stored key" → Save. | The hint says no key is stored. |
| D4 [emb] | Embeddings card: API root, key, model ID and the correct dimensions → "Test embeddings" → Save. Then test with wrong dimensions. | Success names the dimensions. The wrong size gives "The vectors have N dimensions, but the settings say M…". |
| D5 | "Research and accumulation" card. | Both say "No engine available". |
| D6 | Workspace header → Work settings: change the writing language, the own model, the thinking level and the auxiliary tool budget. Then set the budget to 1001. Then turn thinking on for a model without thinking support. | Saving works. 1001 gives a field error. The thinking warning appears. |

## E. Preparation (model) — once in a Chinese work, once in an English work

| ID | Steps | Expected |
|---|---|---|
| E1 | Type a brief. Reload. "Save brief". | "Draft saved in this browser" while typing; the draft survives the reload; "Start preparation" stays disabled until the brief is saved. |
| E2 | "Start preparation". | The dock shows the run's progress and a live preview that streams. Then four tabs (Background, Research, Characters, Outline) with review statuses; the outline shows its chapter plan. The agents write in the work's writing language. |
| E3 | Try to edit the brief. | Not possible once preparation has started (§6). |
| E4 | On one tab, try "Request changes" with empty feedback. Then reject the outline with feedback and approve the other three. | Empty feedback cannot be sent. After the decisions the version revises; a new outline revision comes back for review. |
| E5 | In a round, reject Research with feedback and approve the rest. | After the revision, the reviews of Background, Characters and Outline become stale and are resubmitted (§6). |
| E6 | Select a passage in an artifact → "Cite selected text". Remove the chip, cite again, then reject with the citation and no text. Then try to select across two documents and cite. | The chip shows the quote. A rejection with only a citation is accepted. The cross-document selection gives "A citation stays within one text…". |
| E7 | On an open review, add notes → "Let the review agent decide". Then delegate another review and click "Cancel and review myself". | "The review agent is reviewing this." Its verdict applies by itself and shows that the review agent decided, with its confidence. The cancelled one returns to open. |
| E8 | Approve all four. | "The preparation is approved." Chapters appear in the chapter list. |

## F. Writing chapters (model)

| ID | Steps | Expected |
|---|---|---|
| F1 | Chapter 1: guidance, then "Start outline". Reject the outline with feedback, then "Approve outline". | The revised outline comes back; approval moves to drafting. |
| F2 | Watch the body being written; scroll up while text arrives. | Text being written is shown apart from committed text ("Being written"). The word count updates. "New content below" appears and scrolls down when clicked. |
| F3 | "Send feedback" while the agent writes. | The queue item goes "Waiting for the next result" → "Goes with the next request" → "Sent". |
| F4 | "Pause"; then "Continue". Pause again and send feedback while paused ("Send feedback and continue"). Pause, then "Cancel pause" before it takes effect. | "The agent pauses before its next change", then a paused state naming the held change. Continuing applies it. Cancelling the pause lets writing go on. |
| F5 | Stop the run, then continue it. | The stop is explained; continuing resumes writing without losing committed text. |
| F6 | Turn on "Review each edit before it is applied". | The next edit is a proposal: added text marked + and green, removed text struck through, marked - and red. Empty feedback → "Accept edit"; any feedback or citation → "Reject with feedback". Citing only proposed text rejects. Delegating a proposal review works. Switching back to automatic leaves a pending proposal pending. |
| F7 | "Approve chapter" → "Next chapter" … after the last plan chapter → "Confirm the novel complete". | Each chapter is approved in turn; the version ends completed. |
| F8 | "Add extra chapter" with a title; write and approve it. Then click the trash icon next to its title and confirm. | The extra chapter follows the same steps, and completion stays. The icon appears only on extra chapters and only while nothing runs. After deletion the chapter is gone from the reader and the chapter list. |
| F9 | While a request is in flight, kill the process (SIGKILL). Restart. | The run shows as interrupted with an explanation. "Continue" resumes, with no duplicated or lost text. |
| F10 | Restart while a run is paused, while a proposal waits, and while a question waits. | Each state comes back exactly as it was. |
| F11 | Click a chapter in the chapter list. Resize to 1000px, then to 390px. | The reader scrolls to the chapter. Below 1100px the chapter list collapses; below 768px it becomes a drawer (§18.2). |
| F12 | Use the prompt override from H6 to make the writer ask a question. | "The agent asks" dock with the questions. A required question blocks "Submit answer". Choices accept your own words. Answering continues the run. |

## G. Versions (model for G4)

| ID | Steps | Expected |
|---|---|---|
| G1 | Version menu → "New version…" → "Create version". | An empty version opens (preparation not started), with its own brief. |
| G2 | Switch versions in the menu. | Each version's own content shows. |
| G3 | "Delete versions…": delete another version with its confirmation. | Only versions other than the shown one are listed. The deleted version disappears from the menu. |
| G4 | Start preparation in version 2 while version 1 has an active run. Then try to delete version 1 from version 2. | Both are refused with localized conflict messages. |

## H. Prompts and templates [both]

| ID | Steps | Expected |
|---|---|---|
| H1 | Open Prompts. Pick a prompt; switch between the system and user prompt tabs. | The built-in Chinese text shows with "In effect: the built-in text" and a list of placeholders with help and examples. |
| H2 | Edit the system prompt for "Preparation" under "All works" → Save → reload → "Reset to the built-in text". | "In effect: the system's prompt" after saving and reloading; the reset restores the built-in text. |
| H3 | Remove `{{writing_language}}` from a system prompt → Save. Add `{{foo}}` → Save. | "The placeholder “writing_language” is required here."; then "The placeholder “foo” is not available here." |
| H4 | "Preview with examples". | The text renders with the example values. |
| H5 | Choose a work under "Applies to", override the same prompt, then reset it. | "In effect: this work's prompt" while it exists; the reset falls back to the system's prompt. |
| H6 | Override the preparation user prompt of one work to tell the agent to ask the user one question first, then run preparation. | The agent asks (used by F12), which shows the override reaches the model. |
| H7 | "Your templates" tab: create a template with a placeholder and its description; then save a text using an undeclared placeholder; edit; delete with confirmation. | Creating and editing work. The undeclared placeholder is refused with a message. The deleted template disappears. |
| H8 | In chapter guidance and in a feedback form: "Insert template", choose it, fill the placeholder → "Insert". | The filled text is appended to the draft after a blank line. |

## I. Knowledge base [both]

| ID | Steps | Expected |
|---|---|---|
| I1 | Spaces: create, rename, create a second with the same name, delete one that holds entries, delete an empty one. | The duplicate name and the non-empty delete are refused with messages; the rest works. |
| I2 | Entries: create one with the keywords fuel, lamp, sea and oil, separated by an ASCII comma, a full-width comma (U+FF0C) and an ideographic comma (U+3001); filter by title and by a keyword in another case; edit; move it to another space; delete it. | Four keywords; filtering ignores case; every change shows at once. |
| I3 | Without an embedding model: look at an entry, then search. | The entry says it is not searchable; the search says it needs an embedding model. |
| I4 [emb] | With embeddings set: create several entries on different topics; wait; search with a query that matches one topic. | Entries go "Waiting to be indexed" → "Searchable". The matching entry ranks first, with its space and relevance; clicking a result opens the entry. Searching with a space selected stays within it. |
| I5 [emb] | Change the embedding model or dimensions, then search right away; search again once indexing is done. | First, a message that the knowledge base is being indexed, with the entries remaining; then normal results. |
| I6 [emb] | Save embedding settings whose dimensions differ from what the model returns, then add an entry. | The entry shows "Indexing failed" with the reason and "Index again". |
| I7 [emb] | With searchable entries, run preparation in a new work whose brief needs that knowledge. Then set the work's auxiliary tool budget to 0 and run preparation in another new work. | The dashboard's embeddings count grows during the first run, and the research artifact uses the entries' content. The second run makes no knowledge searches. |

## J. Dashboard [both]

| ID | Steps | Expected |
|---|---|---|
| J1 | Switch between Today, Last 7 days and Last 30 days, and between all works and one. | Every figure changes consistently. |
| J2 | With a run working, and with a stopped run. | The working run is listed under "Working or waiting" with a link to its version; the stopped one under "Needs attention" with its reason. |
| J3 | Compare "Words in all chapters now" with the reader's word counts. | They match; today's bar shows the growth. |
| J4 | Look at token usage. | Rows for Writing, Review agents, Connection tests and Embeddings. Requests without reported usage are counted under "Unknown" and explained, never counted as zero. |

## K. Export (model for content)

| ID | Steps | Expected |
|---|---|---|
| K1 | Header → Export → Markdown, then Plain text, on a work titled `Rain: Letters?` and on one with a Chinese title. | The downloads are named after the sanitized title; the Chinese title survives in the file name. |
| K2 | Open the files of a Chinese and of an English work with plan chapters, an extra chapter, an unapproved chapter and an unwritten one. | The header and labels are in the writing language. Plan chapters come first, then extras. The unapproved chapter is marked as a draft; the unwritten one says so. No proposals, notes or prompts appear. |

## L. Robustness

| ID | Steps | Expected |
|---|---|---|
| L1 | Open Settings in two tabs and save new model settings in one. Then rename a work twice through the API with the same `expectedRevision`: `POST /api/projects/{id}/rename` with `{"commandId": "<new UUID>", "expectedRevision": <revision from GET /api/projects>, "name": "..."}`, a new command ID each time. | The other tab shows the saved values without a reload: an open settings form follows the latest saved values, so it never holds a stale revision. The second rename answers 409 with `code` `REVISION_CONFLICT`, and the name stays as the first rename set it. |
| L2 | Stop the service with Ctrl+C while a page is open; start it again. | The page shows "Reconnecting to NekoWriter…", then recovers and shows current data. |
| L3 | Double-click submit buttons (create work, approve, send feedback). | Each action is applied once. |

## Report

For each case report: ID, result (pass, fail or blocked), the language it ran in, and a short note. For every failure add:
- the steps taken;
- what was expected and what happened;
- a screenshot;
- the server log lines around it (error messages in the UI carry a request ID that appears in the log).

Mark cases blocked when a service they need is missing; do not skip them silently.
