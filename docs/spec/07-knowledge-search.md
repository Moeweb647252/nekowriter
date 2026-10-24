# 07 — Knowledge Base, Search Index, Research, Web Tools

## 1. Knowledge base

- **Scopes** (`knowledge_scopes`): global spaces with a unique name and a description (the
  description is shown to agents, so it should say what the scope covers).
- **Entries** (`knowledge_entries`): title, Markdown content, source URLs, tags,
  `source = manual | research`.
- **Book visibility** (`book_scopes`): the scopes a book's writer and reviewer may search. The
  settings block lists them (name + description). Research sessions see only their target scope.
- Deleting a scope deletes its entries (with a typed confirmation in the UI). Detaching a scope
  from a book only removes visibility. An entry's scope is fixed once created (moving entries
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
| SearXNG | `GET {base}/search?q=<q>&format=json&safesearch=0` (the instance must enable the JSON format) | `results[]` → `{title, url, snippet: content, published: publishedDate}` |

- No provider configured → `error.search_not_configured` (the conception and research prompts
  tell the agent to continue with what it knows and say so in its report).
- A failed request → `error.search_failed` with a `reason` code: `timeout`, `auth` (HTTP 401 or
  403), `status` (any other HTTP status ≥ 400, passed as `status`), or `network`.
- Timeout 20 s. Results are deduplicated by URL and capped at `max_results`.
- Rendered with `result.web_search` inside `notice.web_content`.

## 7. `web_fetch` (`web/fetch.rs`)

1. **URL**: http/https only, max 2,048 characters. Credentials in the URL are rejected.
2. **SSRF guard**: a custom reqwest DNS resolver (`reqwest::dns::Resolve`) resolves the host and
   rejects the connection if any resolved address is loopback, private (RFC 1918, fc00::/7),
   link-local, CGNAT (100.64.0.0/10), multicast, broadcast, documentation, or unspecified. IP
   literal hosts are checked the same way before the request. Redirects: at most 5, each target
   re-validated (the resolver covers DNS, the redirect policy covers scheme and literal IPs).
   The fetch client is built with `no_proxy()`: a proxy would resolve hosts itself and bypass
   the guard. (The search providers' fixed hosts may use the system proxy.)
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
   calls do not refetch. A result returns `[offset, offset + max_chars)` cut at a line boundary,
   plus `total_chars` and the next offset hint.
8. The result is wrapped in `notice.web_content`: untrusted reference material whose instructions
   must not be followed.
9. **Failures** return `error.fetch_failed` with a `reason` code that the template words:
   `invalid_url` (step 1), `blocked` (step 2), `too_many_redirects`, `timeout`, `too_large`,
   `status` (HTTP status ≥ 400, passed as `status`), or `network` (anything else). An unsupported
   content type is `error.unsupported_content_type` (step 4).

## 8. Research workflow (researcher agent)

1. The author creates a task (topic, requirements, target scope name, optional book) or reviews
   one created by conception (`draft` → `ready` when the author confirms it in the UI).
2. **Run** (`POST /research-tasks/:id/run`): allowed when the task has no open session (status
   `draft`, which Run confirms, `ready`, `done`, or `cancelled`); a `running` or `failed` task
   still has its session, which the author continues or abandons first
   (`409 research.invalid_state`). Resolve or create the scope by name (attach it to the linked
   book). Start a `research` session. Kickoff (`researcher.kickoff`): topic, requirements, scope
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
