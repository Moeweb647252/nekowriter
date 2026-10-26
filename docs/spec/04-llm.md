# 04 — LLM Layer (`crates/llm`)

A provider-neutral, streaming-first client with two real protocols (OpenAI-compatible Chat
Completions and Anthropic Messages) and a mock protocol. It is hand-written on `reqwest`
because the runtime needs exact control over replay payloads, cache breakpoints, reasoning
replay, and partial tool-argument streaming. No SDK is used.

## 1. Neutral types (`types.rs`)

```rust
pub struct ChatRequest {
    pub model: String,
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDef>,             // fixed order, frozen per session
    pub max_output_tokens: u32,
    pub parallel_tool_calls: bool,       // false → at most one tool call per response
    pub cache: CacheHints,
    pub options: ModelOptions,
    pub demo_hints: Option<DemoHints>,   // only read by the demo provider (§7)
}

pub struct ToolDef { pub name: String, pub description: String, pub input_schema: serde_json::Value }

pub enum Message {
    User { blocks: Vec<TextBlock> },               // kickoff, author messages, notices, nudges
    Assistant { payload: ProviderPayload },        // exact replay of what the provider returned
    ToolResults { results: Vec<ToolResult> },      // always directly after an Assistant message
}
pub struct TextBlock { pub text: String, pub cache: bool }   // cache = breakpoint after this block
pub struct ToolResult { pub call_id: String, pub content: String, pub is_error: bool }
pub struct CacheHints { pub ttl: CacheTtl }                  // 5m | 1h (already resolved; §3)

pub enum StreamEvent {
    TextDelta { index: u32, text: String },        // index = provider content-block index (display grouping)
    ReasoningDelta { index: u32, text: String },
    ToolCallStart { call_index: u32, id: String, name: String },   // call_index = 0-based ordinal among
    ToolCallArgsDelta { call_index: u32, fragment: String },       // this turn's tool calls
    Usage(Usage),                                  // may arrive more than once; adapter merges
}

pub struct AssistantTurn {
    pub text: String,                              // visible text (concatenated)
    pub reasoning: String,                         // reasoning/thinking summary for display
    pub tool_calls: Vec<ToolCallOut>,              // { id, name, input_text } in order
    pub stop_reason: StopReason,                   // EndTurn | ToolUse | MaxTokens | Refusal{category, explanation} | PauseTurn | Other(String)
    pub usage: Usage,                              // input (uncached), cached_input, cache_write, output, reasoning
    pub payload: ProviderPayload,                  // replay material (spec/02 §4)
    pub request_id: Option<String>,                // provider request id header, for error reports
}

pub enum ProviderClient { OpenAi(OpenAiClient), Anthropic(AnthropicClient), Mock(MockClient) }
impl ProviderClient {
    pub async fn run(&self, req: &ChatRequest, on_event: &mut (dyn FnMut(StreamEvent) + Send))
        -> Result<AssistantTurn, LlmError>;
    pub async fn list_models(&self) -> Result<Vec<RemoteModel>, LlmError>;   // id, display_name, context, max_output
}
```

Enum dispatch (no `dyn` async traits). Dropping the `run` future cancels the HTTP request. The
runner wraps it in `tokio::select!` with the session's cancellation token.

`ProviderConfig { protocol, base_url, api_key: Option<String>, extra_headers }` is built by the
server from the `providers` row at call time. `env:NAME` keys are resolved there; a missing env
var gives `LlmError::Auth("environment variable NAME is not set")`.

## 2. OpenAI-compatible adapter (`openai.rs`)

**Endpoint**: `POST {base_url}/chat/completions`, with `Authorization: Bearer <key>` when a key is
set, plus the extra headers.

**Body**:
```json
{
  "model": "<model>",
  "stream": true,
  "stream_options": { "include_usage": true },          // when options.stream_usage
  "messages": [ { "role": "system", "content": "<system>" }, … ],
  "tools": [ { "type": "function", "function": { "name", "description", "parameters": <schema> } } ],
  "tool_choice": "auto",
  "parallel_tool_calls": <req.parallel_tool_calls>,     // sent only when options.send_parallel_tool_calls (default true)
  "max_tokens": <max_output_tokens>,                    // or "max_completion_tokens" per options.max_tokens_field
  "temperature": <t>,                                   // only when options.temperature is set
  "reasoning_effort": "<e>"                             // only when options.reasoning_effort is set
  /* …options.extra_body merged last (shallow) */
}
```

**Message mapping**:
- `User` → `{ "role": "user", "content": blocks joined with "\n\n" }`. Cache flags are ignored,
  because providers cache prefixes automatically.
- `Assistant` → `{ "role": "assistant", "content": payload.content (null when empty),
  "tool_calls": payload.tool_calls }`, plus `"reasoning_content": payload.reasoning_content` when
  `options.reasoning_replay = always` and it is non-empty. **DeepSeek thinking mode with tools
  rejects requests that omit it (HTTP 400)**, so the DeepSeek and Kimi presets use `always`.
- `ToolResults` → one `{ "role": "tool", "tool_call_id", "content" }` per result. This protocol
  has no error flag; error texts (`result.error`) state the failure themselves, so nothing is
  prefixed (a prefix would be model-facing text in code).

**Stream parsing** (`data:` lines until `data: [DONE]`):
- `choices[0].delta.content` → `TextDelta`.
- `choices[0].delta.reasoning_content` (DeepSeek, Qwen, Kimi, GLM) or `delta.reasoning`
  (OpenRouter) → `ReasoningDelta`, accumulated into `payload.reasoning_content`.
- `choices[0].delta.tool_calls[]` → accumulate by `index`. The first fragment carries `id` and
  `function.name` (emit `ToolCallStart`). Later fragments append `function.arguments` (emit
  `ToolCallArgsDelta`). Missing ids are synthesized as `call_<index>`.
- `finish_reason`: `stop` → EndTurn, `tool_calls` → ToolUse, `length` → MaxTokens,
  `content_filter` → Refusal, anything else → Other.
- Usage chunk (empty `choices`): `prompt_tokens`, `completion_tokens`,
  `prompt_tokens_details.cached_tokens` or DeepSeek `prompt_cache_hit_tokens`, and
  `completion_tokens_details.reasoning_tokens`. Mapping: `cached_input = cached`,
  `input = prompt_tokens − cached`, `output = completion_tokens`. When no usage arrives,
  estimate (spec/03 §10 heuristic) and mark it `estimated: true`.
- An `{"error": …}` object inside the stream → `LlmError::Provider`.

## 3. Anthropic adapter (`anthropic.rs`)

**Endpoint**: `POST {base_url}/v1/messages`, with headers `x-api-key`,
`anthropic-version: 2026-10-21`, `content-type: application/json`, and
`anthropic-beta: server-side-fallback-2026-07-01` only when `options.refusal_fallback` is set.

**Body**:
```json
{
  "model": "<model>",
  "max_tokens": <max_output_tokens>,
  "stream": true,
  "system": [ { "type": "text", "text": "<system>", "cache_control": { "type": "ephemeral", "ttl": "1h" } } ],   // same TTL rule as below
  "tools": [ { "name", "description", "input_schema": <schema>, "eager_input_streaming": true } ],
  "tool_choice": { "type": "auto", "disable_parallel_tool_use": <!req.parallel_tool_calls> },
  "thinking": { "type": "adaptive", "display": "<summarized|omitted>" },
  "output_config": { "effort": "<low|medium|high|xhigh|max>" },
  "cache_control": { "type": "ephemeral", "ttl": "1h" },       // top-level automatic caching of the growing tail; omit "ttl" for 5m
  "messages": [ … ],
  "fallbacks": "default"                                         // only with refusal_fallback
}
```

Rules from the current API (API version `2026-10-21`):
- **Never send** `temperature`, `top_p`, or `top_k`, which return 400 on this API version. The
  `temperature` option is ignored for Anthropic.
- **Never use forced tool choice** (`any` / `tool`). Current models (Opus 5.5, Sonnet 5.5,
  Fable 5.1) reject it. Use `auto` and steer through prompts and nudges (spec/03 §5.6).
- `thinking: {type: "adaptive"}` is valid on all current models. Do not send `budget_tokens` or
  `{type: "disabled"}`, which several current models reject. Effort is the depth control.
- `eager_input_streaming: true` on every tool, so long `insert_text` arguments stream
  immediately. The client must then validate the input itself, which the server already does
  (spec/03 §5.2). Set it false for proxies that reject the field (`options.eager_input_streaming`).

**Caching** (3 of the 4 allowed breakpoints):
- An explicit breakpoint `cache_control: {type: "ephemeral"[, "ttl": "1h"]}` on the system
  block: tools and system prompt are fixed per role, so every session of the role shares it,
  across books.
- An explicit breakpoint on every user text block with `cache: true` (the settings block,
  shared by the book's sessions).
- The top-level automatic breakpoint for the growing conversation tail.
- All three use the **same TTL**, because the API requires longer TTLs to come before shorter
  ones and rejects an explicit marker on the last block whose TTL differs from the top-level one.
- `options.cache_ttl` is `5m`, `1h`, or `auto` (default). The server resolves `auto` once, at
  session creation (spec/03 §4): `1h` for conception sessions and for writer sessions with
  `auto_pause` or `pause_after_outline` (the author reads the outline or a step, often for more
  than 5 minutes, and a miss re-writes the whole prefix), otherwise `5m`.

**Message mapping**:
- `User` → `{ role: "user", content: [ {type: "text", text, cache_control?}, … ] }`.
- `Assistant` → `{ role: "assistant", content: payload.content }`, the raw blocks exactly as
  received: `thinking` blocks with `signature`, `redacted_thinking`, `text`, `tool_use`, and any
  unknown block types (e.g. `fallback`). Never edit them.
- `ToolResults` → `{ role: "user", content: [ {type: "tool_result", tool_use_id, content: [ {type:
  "text", text} ], is_error} … ] }`. If the next neutral message is a `User`, its text blocks are
  appended to this same user message (after the `tool_result` blocks), so roles alternate.

**Stream parsing** (SSE `event:` + `data:`):

| Event | Handling |
|---|---|
| `message_start` | record `message.usage` (input_tokens, cache_creation_input_tokens, cache_read_input_tokens) |
| `content_block_start` | start block `index` with the given `content_block`: `text`, `thinking`, `redacted_thinking` (complete in start), `tool_use` (emit `ToolCallStart{call_index: next ordinal, id, name}`; later `input_json_delta`s of that block map to the same ordinal), or unknown (store raw) |
| `content_block_delta` | `text_delta` → TextDelta · `thinking_delta` → ReasoningDelta · `signature_delta` → append to the block's signature · `input_json_delta.partial_json` → ToolCallArgsDelta + buffer |
| `content_block_stop` | finalize the block. For `tool_use`, parse the buffered JSON into `input` for the payload; if invalid, keep `input: {}` in the payload and pass the raw text as `input_text` (the runtime reports the error, spec/03 §5.5) |
| `message_delta` | `delta.stop_reason`, `delta.stop_details` (refusal category/explanation), `usage.output_tokens` |
| `message_stop` | done |
| `ping` | ignore (resets the idle timeout) |
| `error` | `LlmError` from `error.type` (`overloaded_error` → Overloaded, `rate_limit_error` → RateLimited, …) |

Stop reasons: `end_turn`/`stop_sequence` → EndTurn, `tool_use` → ToolUse, `max_tokens` →
MaxTokens, `refusal` → Refusal, `pause_turn` → PauseTurn, anything else → Other. Usage:
`input = input_tokens`, `cached_input = cache_read_input_tokens`,
`cache_write = cache_creation_input_tokens`, `output = output_tokens`.

## 4. Errors and HTTP behavior

```rust
pub enum LlmError {
    Network(String), Timeout(&'static str), RateLimited { retry_after: Option<Duration> },
    Overloaded, Server(u16, String), Auth(String), BadRequest(String), NotFound(String),
    ContextOverflow(String), Provider(String), Protocol(String), Cancelled,
}
impl LlmError { pub fn retryable(&self) -> bool }   // Network, Timeout, RateLimited, Overloaded, Server
```

- Status mapping: 401/403 → Auth; 404 → NotFound; 408 → Timeout; 409 → Server(409) (transient
  conflict, retried like the official SDKs do); 429 → RateLimited (parse `retry-after`, both
  seconds and HTTP dates); 529 → Overloaded; 5xx → Server; 400/413 → ContextOverflow when the
  message mentions context or prompt length ("prompt is too long", "maximum context length",
  "context_length_exceeded"), else BadRequest.
- Timeouts: connect 15 s; idle gap between stream events 300 s (reasoning models can be silent
  for a long time; Anthropic sends pings).
- Capture `request-id` / `x-request-id` response headers into errors and turns.
- Never log API keys or Authorization headers.

## 5. Model options (`model_profiles.options`)

Every field is sent by **exactly one** adapter, or by both in the two cases marked *both* below.
The classification is not documentation guesswork: `crates/llm/tests/model_options.rs` builds both
request bodies with each option set and asserts that it reaches the wire exactly for the protocols
listed here, so a field added to one adapter without updating this table (or the reverse) fails a
test.

| Option | Protocol(s) that send it | Where it lands | Default |
|---|---|---|---|
| `reasoning_replay` | **both** | `openai.rs`: echoes `reasoning_content` on replayed assistant turns when `always`; `anthropic.rs`: replays the stored content blocks verbatim regardless | `never` |
| `cache_ttl` | **both** | resolved by the server into `CacheHints.ttl`, then used for the Anthropic `cache_control` breakpoints; OpenAI relies on automatic prefix caching and sends nothing for it | `auto` (§3 Caching) |
| `reasoning_effort` | openai | `reasoning_effort` in the body — free-form string because gateways accept different values | — |
| `max_tokens_field` | openai | name of the field that carries the output cap: `max_tokens` or `max_completion_tokens` | `max_tokens` |
| `send_parallel_tool_calls` | openai | whether `parallel_tool_calls` is sent at all | `true` |
| `stream_usage` | openai | whether `stream_options.include_usage` is sent | `true` |
| `temperature` | openai | `temperature` in the body. The Anthropic adapter must **never** send it (§3) | — |
| `extra_body` | openai | raw JSON merged into the body **last (shallow)**, so it wins over every field above (§1) | — |
| `effort` | anthropic | `output_config.effort` | `high` |
| `thinking_display` | anthropic | `thinking.display` | `summarized` |
| `refusal_fallback` | anthropic | the `anthropic-beta` header plus `fallbacks: "default"` | `false` |
| `eager_input_streaming` | anthropic | `eager_input_streaming` on every tool | `true` |

`reasoning_replay` is listed as *both* on purpose even though only `openai.rs` reads the flag: the
Anthropic adapter always replays its stored blocks, which is exactly what `always` asks for. The
option therefore carries a meaning on both sides rather than being inert on one.

The Demo (`mock`) protocol sends **no** model options: its client is the in-process provider, which
ignores the request it is handed. Every option is inert there, so none is accepted (§8).

**Cross-protocol options are refused on save.** `ModelOptions` is one struct for both protocols, so
a field belonging to the other protocol deserializes happily and then reaches no request body. That
is how a profile stored `effort: "high"` on an OpenAI-protocol provider, the OpenAI adapter sent
only `reasoning_effort`, and the request carried **no reasoning parameter at all** while the turn
succeeded (D-132). Therefore:

- `POST`/`PATCH /model-profiles` returns `400 validation.failed` with one `details.fields.<option>`
  entry per stranded field, naming the field and the provider's protocol. Values are validated by
  round-tripping through `ModelOptions` first, as before.
- Reads **never fail** on a stored mismatch. `ModelProfileDto.ignored_options` lists the fields the
  provider's protocol cannot send, and the server logs one warning per mismatched profile at
  startup, so the author can find and fix the row from Settings › Models.
- `GET /provider-presets/options` publishes this table (`key`, `protocols`, `kind`, `values`,
  `default_value`). The settings form renders its option list from that response instead of a
  TypeScript copy, so the UI cannot offer a field the wire cannot carry.
- A value equal to the option's default is not "set" and is neither refused nor reported; profiles
  are stored with every field serialized, so treating defaults as deviations would flag every row.

## 6. Retry policy (used by the runtime)

- Retry only before the turn is complete, and only for `retryable()` errors.
- At most 4 attempts in total (the first try plus 3 retries), with backoff 2 s → 8 s → 30 s
  plus ±20% jitter. `retry_after` overrides the backoff when it is longer (capped at 120 s).
- Mid-stream failures discard the partial output and retry the whole request (the history is
  unchanged, so the request is identical).
- `session.updated` carries `retrying: {attempt, wait_seconds, reason}` so the UI can show
  "Provider overloaded — retrying in 8 s".

## 7. Mock protocol (`mock.rs`)

### 7.1 `ScriptedProvider` (tests)
- Built from a queue of `ScriptStep { events: Vec<StreamEvent>, stop: StopReason, usage: Usage,
  delay_per_event: Duration }`. Helpers: `step().text("…").tool("insert_text", json!({…})).stop(ToolUse)`.
- Records every `ChatRequest` (`requests()`) for assertions, including byte-exact JSON
  serialization through the OpenAI adapter's body builder, to test append-only prefixes.
- Can inject errors (`fail_with(LlmError::Overloaded)` for N attempts) and long delays (for
  stop/cancel tests).

### 7.2 `DemoProvider` (manual testing without API keys)
Available when the server runs with `--enable-demo-provider` (default in debug builds). It is a
deterministic fake agent that drives every workflow so the UI can be exercised end to end. It
decides from `req.demo_hints = {kind, phase, granularity, step_in_phase, language}` and the
history:

| Kind / phase | Behavior |
|---|---|
| chapter_write / planning | `get_entity` on the protagonist (if any), then `submit_outline` with a three-scene outline |
| chapter_write / drafting | `insert_text` with text from a small built-in Chinese and English corpus, matching the granularity (sentence, paragraph, free chunk, or chapter); `chapter_end` after ~8 paragraphs, and `finish_draft` when that completion is withheld as short |
| chapter_revise | `submit_revision_plan`, then one `replace_text` on the first current paragraph (the pid and the exact `old` text come from the draft the request carries, D-077), then `finish_revision` |
| chapter_review | `submit_review` (verdict `revise` on attempt 1, `pass` on later attempts) with one issue on `p2` |
| maintenance / resync | `record_appearances` (protagonist), `update_entity` of the protagonist's state with a change note (an isolated track refuses it for a shared protagonist, and the demo moves on), `add_timeline_event`, `set_chapter_summary`, in maintenance (not resync) one `propose_setting_change` (an `append` to the world setting; isolated tracks refuse it, and the demo moves on), `finish_maintenance` (again if it asks for confirmation). A book with no protagonist entity yet (no canon seed) gets one `create_entity` first, so the required steps exist without an API key (D-090) |
| seed | `create_entity` per character design (the rank-1 one with `is_protagonist`), then `finish_maintenance` |
| conception | `ask_user` (2 questions) → after the answers: `write_setting_doc` ×3, `upsert_character` ×2, `set_research_topics` → `request_review` → revises any document that was not approved → a closing text message |
| research | `save_knowledge_entry` ×2 (no network) → `finish_research` |

It streams text in 20–40 character chunks with 30 ms delays, so streaming and ghost text are
visible. `list_models` returns `demo-model` (context 200k, output 16k). Everything the demo
"writes" (sample sentences, the outline, setting documents, character names, the closing
message, in Chinese and English) lives in `crates/llm/demo/corpus.toml`, embedded with
`include_str!`, so the source stays English-only; `demo_hints.language` (the book's writing
language) picks the variant.

## 8. Provider presets (UI helper; values are editable)

| Preset | Protocol | Base URL | Defaults |
|---|---|---|---|
| Anthropic | anthropic | `https://api.anthropic.com` | Suggested models: `claude-opus-5-5`, `claude-sonnet-5-5`, `claude-haiku-5-5` (context 1,000,000; max output 128,000; options `effort: high`). `refusal_fallback` stays off for Haiku 5.5, which has no server-side fallback |
| OpenAI | openai | `https://api.openai.com/v1` | `max_tokens_field: max_completion_tokens` |
| DeepSeek | openai | `https://api.deepseek.com` | `reasoning_replay: always` |
| OpenRouter | openai | `https://openrouter.ai/api/v1` | `reasoning_replay: always` |
| Qwen (DashScope) | openai | `https://dashscope.aliyuncs.com/compatible-mode/v1` | |
| Moonshot (Kimi) | openai | `https://api.moonshot.cn/v1` | `reasoning_replay: always` |
| Zhipu (GLM) | openai | `https://open.bigmodel.cn/api/paas/v4` | |
| Ollama | openai | `http://127.0.0.1:11434/v1` | no key; `send_parallel_tool_calls: false` |
| LM Studio | openai | `http://127.0.0.1:1234/v1` | no key |
| Custom | openai | (empty) | |
| Demo (dev only) | mock | `mock://demo` | no key |

Every preset's `options` must be sendable by that preset's own protocol: a preset that shipped a
field its protocol cannot send would be a default defect, inherited by every install. That is
pinned by a test over the whole preset table, not by inspection (§5, D-132).

**Fetch models**: OpenAI-compatible `GET {base}/models` → `data[].id`. Anthropic
`GET {base}/v1/models` → `data[]` with `id`, `display_name`, `max_input_tokens`, `max_tokens`,
which pre-fill the context window and max output. When a provider does not support listing, the
UI shows "type the model id".

**Test connection**: one streamed request (`max_output_tokens: 32`, user text "ping", no tools)
that reports success with latency and the response text, or the classified error with the
provider's message.

## 9. Required tests (`crates/llm/tests`)

- OpenAI-compatible fixtures: plain text; tool call split across many fragments; two parallel tool
  calls; DeepSeek `reasoning_content` + tool call + usage with `prompt_cache_hit_tokens`; an
  in-stream error object; `length` finish. Assert events, `AssistantTurn`, and that replay emits
  `reasoning_content` only when `always`.
- Anthropic fixtures: thinking + signature + text + tool_use with `input_json_delta` fragments;
  `redacted_thinking`; unknown block type preserved verbatim; `refusal` with `stop_details`;
  usage merge; `overloaded_error` event. Assert the replay payload is byte-identical to the
  received blocks, and that the request body never contains `temperature`/`top_p`/`top_k`.
- Body builders: tool-result plus user-text merging (Anthropic); `parallel_tool_calls` and
  `max_tokens_field` handling (OpenAI); cache_control placement (system block, `cache: true`
  blocks, top-level) and TTL consistency; tool-call ordinals when thinking and text blocks
  precede the `tool_use` blocks.
- The option table (§5): for every `ModelOptionKey`, set it on a request and build both bodies;
  assert it reaches the wire exactly for the protocols the table lists. Also: `extra_body` merges
  last and overrides built-in fields; the OpenAI body never carries `output_config`/`fallbacks`
  and the Anthropic body never carries `reasoning_effort`/`temperature`.
- Error classification table tests, including `retry-after` parsing.
