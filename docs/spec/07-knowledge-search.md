# 07 — Knowledge Base, Search Index, Research, Web Tools

## 1. Knowledge base

- **Scopes** (`knowledge_scopes`): global spaces with a unique name and a description (the
  description is shown to agents, so it should say what the scope covers). **A book normally uses
  one or two scopes**: one broad scope named for the book's subject or era, and a second only when
  the material genuinely splits into two unrelated domains a writer would search separately. The
  shipped conception prompt states this budget (D-145) and forbids one scope per subtopic, chapter,
  character, or document type; it is prompt guidance, not a schema or API constraint. Fewer, larger
  scopes keep a `search_knowledge` call without a scope useful — every visible scope is a separate
  KNN query (§3) and a separate name the author has to choose among.
- **Entries** (`knowledge_entries`): title, Markdown content, source URLs, tags,
  `source = manual | research`.
- **Book visibility is opt-out** (`book_scope_exclusions`, D-158): a book's agents (planner,
  writer, reviewer, conception, and the book's knowledge check) see **every** scope except those
  the author unchecked for the book. A scope is visible to every book from the moment it is
  created; nothing has to be attached. `knowledge::visible_scope_ids(book)` is the one rule, and
  the settings block, the conception kickoff, the knowledge check's inventory, `search_knowledge`
  and `read_knowledge_entry` all use it, so a model never learns a scope it cannot search. A topic
  research session sees only its target scope (§8); a knowledge check sees the book's visible
  scopes (§8.1).
- **Exclusions belong to the author.** An agent never clears one: `set_target_scope` refuses an
  excluded scope. Two author actions clear the exclusion of their own target scope, because they
  ask for knowledge in that scope for this book: **Run** on a book-linked research task (§8 step 2)
  and **Finalize** (spec/08 §5).
- Deleting a scope deletes its entries (with a typed confirmation in the UI) and its exclusion
  rows. Unchecking a scope for a book only removes visibility. An entry's scope is fixed once created (moving entries
  between scopes is not part of the MVP), so `knowledge_chunks.scope_id` and the vector
  partition never change.

### 1.1 Chunking (`search/chunk.rs`)
Entries are chunked on every content change:
1. Parse Markdown headings (`#`…`######`). Each chunk records its heading path
   ("宋代服饰 > 男子常服") in `heading`.
2. Within a section, accumulate paragraphs (split on blank lines) until adding the next one
   would exceed **700 characters**. A single paragraph over 1,000 characters is split at sentence
   ends (`。！？!?.;；` followed by an optional closing quote), or else hard-split at 1,000.
3. No overlap. The embedding input is `heading + "\n" + text` (the title when there is no
   heading), so each chunk carries its context.
4. `content_hash = blake3(heading + text)`. Re-chunking diffs by hash (matched one to one, so
   repeated identical chunks are counted): an old chunk whose hash still occurs keeps its row,
   id, FTS row, vector, and `embed_state` (only `ord` changes, renumbered in two phases like
   chapter `ord`); the other old chunks are deleted (the triggers of spec/02 remove their index
   rows), and new chunks are inserted as `pending`.

## 2. Book corpus (`search_docs`)

Services keep one search doc per indexable item, upserted in the same transaction as the
source change (FTS row too). An upsert that changes `content_hash` sets `embed_state = pending`
(the old vector stays until the new one replaces it). Embedding happens asynchronously (§4).

| `kind` | Source | `title` | `body` | `track_id` |
|---|---|---|---|---|
| `entity` | entities | name | summary · aliases · state · attributes (`k: v`) · description (first 1,500 chars) | entity's |
| `note` | memory_notes (active) | first 40 characters of the content | content | note's |
| `foreshadowing` | foreshadowing | title | description · planned payoff · resolution | row's |
| `event` | timeline_events | title | story_time · description | row's |
| `summary` | chapters (official) | chapter title | summary | NULL for main/canon tracks, T for isolated |
| `passage` | official chapter text | chapter title | ~700-character chunks of consecutive paragraphs; `meta.pids` = covered pids | as `summary` |

Index titles hold no rendered labels. Chapter labels and kind labels are added at query time
(through `shared.chapter_label` / `shared.label`) when results are rendered. Retired notes and
deleted rows remove their docs (the delete triggers of spec/02 clean `fts_book` and `vec_book`).
Passages are rebuilt whenever an official chapter's content changes (approval, author edits,
restore) and on re-sync, inside the same transaction as the change, with the same hash diff as
knowledge chunks (§1.1; `ref_part` plays the role of `ord`), so unchanged passages keep their
vectors. `search_chapters` therefore always searches the current official text.

## 3. Index manager (`search/index.rs`)

- On startup and on embedding-model change, read `index_meta` (`embedding_model`,
  `embedding_dim`). If they differ from the active model (or the vec tables are missing), in one
  transaction: drop `vec_book` and `vec_knowledge`, recreate them and their delete triggers with
  the new dimension (DDL in spec/02), set every `search_docs.embed_state` and
  `knowledge_chunks.embed_state` to `pending`, then write the new `index_meta`. FTS tables are
  unaffected.
- **Writes**: vectors are bound as little-endian `f32` BLOBs
  (`v.iter().flat_map(|f| f.to_le_bytes())`). Replace = `DELETE` the rowid then `INSERT`.
  `vec_book.corpus` is `passage` for passages and `canon` for every other kind
  (`search_chapters` queries `passage`, `search_canon` queries `canon`).
- **KNN**:
  ```sql
  SELECT doc_id, distance FROM vec_book
  WHERE embedding MATCH :q AND k = :k AND book_id = :book_id AND corpus = :corpus;
  SELECT chunk_id, distance FROM vec_knowledge
  WHERE embedding MATCH :q AND k = :k AND scope_id = :scope_id;      -- once per visible scope, then merge
  ```
  The `corpus` filter keeps the many passages from crowding canon hits out of `search_canon`
  (and the reverse). Only equality constraints are used on partition and metadata columns, so
  multiple scopes mean one query per scope. Kind subsets and track visibility are applied after
  KNN by joining `search_docs` (over-fetch `k = max(30, limit × 6)`, keep the best 30).

## 4. Indexer worker and embedder

- **Embedder** (`crates/embed`): `FastEmbedder::new(model_key, cache_dir)` loads lazily in
  `spawn_blocking`. Status: `not_loaded → loading (downloading) → ready | failed(message)`,
  published as `index.status` events and shown in Settings › Embeddings. Downloads are logged when
  they start and end (fastembed's own progress bar is off: `with_show_download_progress(false)`),
  and the UI shows an indeterminate state.
- **Supported models**:

  | key | fastembed variant | dim | query prefix | passage prefix |
  |---|---|---|---|---|
  | `multilingual-e5-small` (default) | `EmbeddingModel::MultilingualE5Small` | 384 | `query: ` | `passage: ` |
  | `bge-small-zh-v1.5` | `EmbeddingModel::BGESmallZHV15` | 512 | `为这个句子生成表示以用于检索相关文章：` | (none) |
  | `embeddinggemma-300m-q` | `EmbeddingModel::EmbeddingGemma300MQ` | 768 | `task: search result \| query: ` | `title: none \| text: ` |

  Read the dimension from `TextEmbedding::get_model_info(&model).dim` and assert it against the
  table. Vectors are L2-normalized before storage (cosine distance in vec0).
  The prefixes are model-input conventions, not prompts (not customizable), and one of them is
  Chinese, so the table lives in a data file, `crates/embed/models.toml` (key, variant name,
  dimension, prefixes), embedded with `include_str!` and parsed into `SUPPORTED_MODELS`.
- **Worker loop**: wake on a `Notify` (signaled by services after commits) or every 5 s. Take up
  to 32 pending items (book docs and knowledge chunks, alternating) with their `content_hash`
  and embed them as passages outside any transaction. Then, in one transaction, write the vector
  and `embed_state = done` only for items whose row still exists with the same `content_hash`
  while the same embedding model is active; the others are skipped (a changed item is `pending`
  again and comes back in a later batch, and a deleted one must not leave a vector behind). If
  the embedder is not ready, sleep until it is. On an embedding error, mark the batch `failed`
  and log it. Failed items are retried when the model reloads or on **Rebuild index**.
- **FakeEmbedder** (tests): deterministic 32-dim vectors from blake3 hashes of character
  trigrams, so similar texts get similar vectors.

## 5. Hybrid retrieval (`search/query.rs`)

```text
hybrid(query, query_vector?, corpus, filters, limit):
  fts_hits ← FTS5 query (≤ 30, ranked by bm25)   # empty if no usable tokens
  vec_hits ← KNN (≤ 30 per §3)                   # empty if no query vector
  fused    ← RRF: score(d) = Σ 1 / (60 + rank_in_list(d))
  return top `limit` by fused score, each with a snippet
```

The query vector is computed in the tool's `prepare` phase (spec/05 §1.2), outside the
transaction; it is absent when the embedder is not ready, and the result then says that only
keyword matching ran. The FTS query is one SQL statement that joins `search_docs` (or
`knowledge_chunks`) and applies the filters before `ORDER BY rank LIMIT 30`, so other books'
documents never take result slots.

- **Segmentation** (`search/fts.rs`): `jieba.cut_for_search(text, true)`, lowercase, drop
  tokens without letters or digits, join with spaces. The same function runs at index time
  (title and body) and query time.
- **FTS query string**: unique query tokens (at most 16), each wrapped in double quotes (inner
  quotes doubled), joined with ` OR `. Example: `"林凡" OR "剑冢" OR "第三层"`.
- **Filters**: book corpus joins `search_docs` (`book_id`, `kind`, track visibility,
  `kind <> 'passage'` for `search_canon`, `kind = 'passage'` for `search_chapters`). Knowledge
  joins `knowledge_chunks` with the visible scope ids.
- **Snippets** are computed from the original text, not the segmented FTS text: find the first
  occurrence of the longest matching query token and take ±150 characters, snapped to sentence
  boundaries when possible. For vector-only hits, use the first 300 characters.
- Results are grouped per source item (a knowledge entry or chapter appears once, with its best
  chunk), then cut to `limit`.

## 6. `web_search` (`web/search.rs`)

```rust
pub trait SearchProvider { async fn search(&self, q: &str, n: usize) -> Result<Vec<SearchHit>, WebError>; }
pub struct SearchHit { pub title: String, pub url: String, pub snippet: String, pub published: Option<String> }
```

| Provider | Request | Mapping |
|---|---|---|
| Tavily | `POST https://api.tavily.com/search`, header `Authorization: Bearer <key>`, body `{"query", "max_results": n, "search_depth": "basic", "include_answer": false}` | `results[]` → `{title, url, snippet: content, published: published_date}` |
| DeepSeek | `POST {base}/v1/tools/search` (base `https://api.deepseek.com`), header `Authorization: Bearer <key>`, body `{"query", "count": n, "need_content": false}` | `results[]` → `{title, url, snippet: summary, published: date}` |
| SearXNG | `GET {base}/search?q=<q>&format=json&safesearch=0` (the instance must enable the JSON format) | `results[]` → `{title, url, snippet: content, published: publishedDate}` |

- DeepSeek returns both a short `snippet` (~100–200 characters, documented for front-end display)
  and a longer `summary` (~600–800 characters, documented for a model to read). `SearchHit.snippet`
  is rendered into the **model prompt** by `result.web_search`, so the provider maps `summary` and
  leaves `snippet`/`content` unused; `need_content: false` keeps the discarded body out of the
  response. `date` is already RFC 3339 UTC and stays absent when null or empty (D-127).
- No provider configured → `error.search_not_configured` (the conception and research prompts
  tell the agent to continue with what it knows and say so in its report).
- A failed request → `error.search_failed` with a `reason` code: `timeout`, `auth` (HTTP 401 or
  403), `status` (any other HTTP status ≥ 400, passed as `status`), or `network`.
- Timeout 20 s. Results are deduplicated by URL and capped at `max_results`.
- Rendered with `result.web_search` inside `notice.web_content`.

## 7. `web_fetch` (`web/fetch.rs`)

`web_fetch` has two backends behind one tool entry point and one page cache, chosen by
`app_settings["search"].fetch_via` (spec/02 §4.1): `direct` (the default) and `deepseek` (§7.1).
Steps 1–6 describe the direct backend; the tool never knows which backend ran.

1. **URL**: http/https only, max 2,048 characters. Credentials in the URL are rejected.
2. **SSRF guard**: a custom reqwest DNS resolver (`reqwest::dns::Resolve`) resolves the host and
   rejects the connection if any resolved address is loopback, private (RFC 1918, fc00::/7),
   link-local, CGNAT (100.64.0.0/10), multicast, broadcast, documentation, or unspecified. IP
   literal hosts are checked the same way before the request. Redirects: at most 5, each target
   re-validated (the resolver covers DNS, the redirect policy covers scheme and literal IPs).
   The direct fetch client is built with `no_proxy()`: a proxy would resolve hosts itself and
   bypass the guard. (The search providers' fixed hosts and the DeepSeek fetch host may use the
   system proxy: connecting to them cannot be redirected into a private address by the URL.)
3. **Limits**: 20 s total timeout; 5 MB body cap (stream and abort); `User-Agent:
   NekoWriter/<version> (+local research assistant)`; `Accept: text/html,text/plain;q=0.9,*/*;q=0.1`.
4. **Content types**: `text/html`, `application/xhtml+xml` → HTML pipeline; `text/plain`,
   `text/markdown` → as is; others → `error.unsupported_content_type` (PDF is not supported in
   the MVP).
5. **Decoding**: charset from `Content-Type`, then BOM, then `<meta charset>` /
   `http-equiv` sniffing in the first 4 KB, then UTF-8 (lossy). Decode with `encoding_rs`, which
   covers GBK, GB18030, Big5, Shift_JIS and the other WHATWG encodings.
6. **Extraction**: `dom_smoothie` Readability with the URL as document URL. On success, use the
   article title and its content HTML. Otherwise use the `<body>`. Then convert HTML → Markdown
   with `htmd` (skip `script`, `style`, `nav`, `footer`, `form`, `iframe`). Collapse 3+ blank
   lines, trim trailing spaces.
7. **Paging**: the converted Markdown is cached per session (LRU, 32 pages, 30 min), so `offset`
   calls do not refetch. Both backends key the cache on the requested URL and store the returned
   Markdown, so `offset` semantics are identical whichever backend ran. A result returns
   `[offset, offset + max_chars)` cut at a line boundary, plus `total_chars` and the next offset
   hint.
8. The result is wrapped in `notice.web_content`: untrusted reference material whose instructions
   must not be followed.
9. **Failures** return `error.fetch_failed` with a `reason` code that the template words:
   `invalid_url` (step 1), `blocked` (step 2), `too_many_redirects`, `timeout`, `too_large`,
   `status` (HTTP status ≥ 400, passed as `status`), or `network` (anything else). An unsupported
   content type is `error.unsupported_content_type` (step 4). The DeepSeek backend uses the same
   vocabulary with the mapping of §7.1.

### 7.1 DeepSeek fetch backend

Selecting `fetch_via: "deepseek"` sends `POST {base}/v1/tools/fetch` (base
`https://api.deepseek.com`) with `Authorization: Bearer <key>`, `Content-Type: application/json`
and body `{"url": "<url>"}`. The response is `{"title", "content"}`, where `content` is extracted
Markdown. The key is `deepseek_api_key`, the same secret the DeepSeek search provider uses
(§6, D-127); selecting the backend without one is a validation error.

- **No SSRF surface on this path — deliberate exception**: our process only ever connects to the
  fixed trusted host `api.deepseek.com`; the requested URL travels as data and DeepSeek's servers
  fetch it. Step 2's private-address check is therefore **not** applied here: it would forbid pages
  that DeepSeek can legitimately reach, while the attack it prevents (our resolver/socket being
  steered into loopback, RFC 1918 or metadata) does not exist on this path. Step 1's URL *shape*
  rules still run locally, with the endpoint's 8,192-character cap, so a malformed URL spends no
  request.
- **Mapping**: `title` → `title`, `content` → `markdown`; `url` is the requested URL because
  DeepSeek reports neither the final URL nor its redirects.
- **Statuses**: 408 and 504 → `timeout`; 413 → `too_large`; 401, 402, 403, 404, 400, 429 and 5xx →
  `status` with the number (403 is a policy refusal, 402 insufficient balance). A transport failure
  is `network`, and a 200 with empty `content` is also `network` because there is no HTTP status to
  carry. The request is not retried; the reason reaches the model, which decides.
- **Paging**: the step 7 cache is shared with the direct backend, so `offset` reads and LRU
  behaviour are unchanged.

## 8. Research workflow (researcher agent)

1. The author creates a task (topic, requirements, target scope name, optional book) or reviews
   one created by conception (`draft` → `ready` when the author confirms it in the UI). A task the
   author creates or edits has `origin = 'author'`, and the conception agent never deletes it
   (spec/05 §7, D-158). This section describes `kind = 'topic'` tasks. The knowledge check is
   §8.1.
2. **Run** (`POST /research-tasks/:id/run`): allowed when the task has no open session (status
   `draft`, which Run confirms, `ready`, `deferred`, `done`, or `cancelled`); a `running` or
   `failed` task still has its session, which the author continues or abandons first
   (`409 research.invalid_state`). Resolve or create the scope by name, and make it visible to the
   linked book (an exclusion of that scope for the book is removed, §1). Start a `research`
   session. A `deferred` task runs directly: "not now" is not a lock, and the run moves it to
   `running` in step 5, so the state never outlives the first run. Kickoff
   (`researcher.kickoff`): topic, requirements, scope
   name and description, the titles of existing entries in the scope (≤ 200), the budget, and
   the linked book's brief (`block.book_brief`: title, genre, idea, writing language), so
   research serves the story.
3. The researcher decomposes the topic into questions, searches, reads authoritative pages,
   cross-checks, and saves entries:
   - one concept per entry, 300–1,500 characters, a structure that suits writers (definition,
     key facts, sensory and period details, terminology, common misconceptions, and how it can be
     used in fiction);
   - sources listed, uncertainty stated;
   - written in the task's `language`: the linked book's writing language, or, for a task
     without a book, the language chosen when creating it (default: the UI language).
4. `finish_research(report)`: what was covered, what was not found, suggested follow-ups. The
   task becomes `done` with `entries_created`.
5. Task status follows its session: `running` while the session is open (including held,
   paused, or stopped, which the session badge shows), `failed` while it is failed (Continue
   sets it back to `running`), `done` on completion, `cancelled` when the session is abandoned.
6. **Defer and resume** (D-138): `POST /research-tasks/:id/defer` postpones a `ready` task and
   `POST /research-tasks/:id/resume` returns it to `ready`. Deferring is the middle way between
   running a task (costs a model call and time) and deleting it (loses the intent), and it is
   what lets a book with pending research be written anyway: `next_action` and `attention` count
   `ready` tasks only, so a deferred task is skipped until the author resumes it (spec/08 §7).
   The table is:

   | From | `defer` | `resume` |
   |---|---|---|
   | `draft` | `409 research.invalid_state` — unconfirmed topic, confirmed with PATCH to `ready` | same |
   | `ready` | → `deferred` | no-op, the row is returned |
   | `deferred` | no-op, the row is returned | → `ready` |
   | `running`, `failed` | `409` naming the open session (finish or stop it first) | same |
   | `done`, `cancelled` | `409 research.invalid_state` — settled work stays in the record | same |

   Both endpoints name a **target status**, which makes a repeat call idempotent rather than an
   error: a double-click returns the current row, writes nothing, and publishes no event. The
   `409` carries `details.status` (the task's current status) and `details.operation`
   (`defer`/`resume`); the open-session case carries `details.session_id` instead, as it does for
   PATCH, DELETE and run.

   **Bulk defer** (D-140): the list's "Defer all (n)" and the overview's `run_research` card both
   postpone every `ready` task of the surface's scope — the book for a book page and for the card,
   every listed task on `/research` — through one client-side loop of the per-task endpoint, one
   request at a time. There is no batch endpoint: the calls are idempotent, and a loop keeps a
   mid-way failure attributable to one task. A partial batch is reported (how many moved, which
   tasks did not), never swallowed; the card's "Go" link stays its primary action.

### 8.1 Knowledge check (D-158)

Knowledge comes before conception. A **knowledge check** (`research_tasks.kind = 'preparation'`)
asks the researcher whether the knowledge visible to a book can support conceiving it, and has
it fill the gaps. A book created with the conception agent runs one first (spec/08 §5), and its
report is handed to the conception agent. The author can also start one from Book › Knowledge at
any time while the book is `conceiving` or `active`.

1. **The task.** It always has a `book_id`, `topic = ''` (the UI shows the localized name
   "Knowledge check"), the author's optional focus in `requirements`, the book's writing language,
   `origin = 'author'`, and `scope_name = ''` / `scope_id = NULL` until the agent picks a target.
   It is never `draft`: `POST /books/:id/knowledge-check` creates the row and starts its session in
   one transaction, so a failure (no researcher model) leaves no row. At most one check per book
   is open (`running`, or `failed` with its session); a second one is refused with
   `409 research.invalid_state` and `details.session_id`. A check is not a setting material: the
   `settings.locked` rule, `topics_revision` and the conception view snapshot ignore it, and
   `set_research_topics` and finalize never touch it. PATCH accepts `requirements` only
   (`topic` or `scope_name` → `422 validation.invalid_fields`). DELETE, run (re-run), defer and
   resume follow §8 unchanged. A re-run keeps the last target until the agent switches it.
2. **The session.** It is an ordinary `research` session with the researcher role and its budget
   (40 tool calls, 25 entries). The kickoff is `block.book_brief` plus
   `researcher.kickoff.prep`, which contains:
   - the author's focus;
   - the inventory of every scope visible to the book (name, description, entry count, and up to
     50 entry titles per scope and 300 in total, with the number left out);
   - the names of the scopes the author excluded;
   - the budget.
3. **The work.**
   - Judge what conceiving this title, idea and genre needs: real-world facts and genre
     knowledge.
   - Verify coverage with `search_knowledge` / `read_knowledge_entry`, which span the book's
     visible scopes, not just the target.
   - Fill only the gaps. `set_target_scope` picks an existing broad scope on the subject, or
     creates one with a description following the one-to-two scope budget (§1). Then search, fetch
     and save, or update a thin entry of the target scope.
   - Never duplicate an entry. A sufficient base may get no new entry at all.
4. **The report.** `finish_research(report)` covers what was already sufficient, what was added,
   and what is still missing or needs the author's decision. A check may finish without a target
   scope. The task becomes `done` with `entries_created` as in §8 step 4. The conception kickoff
   carries the report of the book's latest `done` check (spec/05 §7).
5. **What happens next** is decided by `ConceptionState.start_after_knowledge_check`
   (spec/08 §5). The check itself never starts or changes anything else.
