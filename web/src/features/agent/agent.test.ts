/**
 * Vitest for the agent panel's pure logic (PLAN M3.5, spec/11 §5.3): the live buffers, the stream
 * row builder, the session picker, the composer state machine, the status toasts and the tool
 * renderer registry.
 *
 * The live-buffer tests are the ones the plan names: `applyDelta` accumulates per stream kind and
 * `clearSeq` drops the buffers once the persisted message arrives.
 */
import type { TFunction } from "i18next"
import { describe, expect, it } from "vitest"

import type { MessageWithCallsDto } from "@/api/gen/MessageWithCallsDto"
import type { SessionDto } from "@/api/gen/SessionDto"
import type { ToolCallDto } from "@/api/gen/ToolCallDto"
import { buildRows } from "@/features/agent/ActivityStream"
import { composerMode, disabledReasonKey } from "@/features/agent/Composer"
import { pickSession } from "@/features/agent/AgentPanel"
import { sessionTitle } from "@/features/agent/SessionHeader"
import { toolRenderer, registeredTools } from "@/features/agent/toolRenderers"
import { statusNotification } from "@/features/agent/useSessionToasts"
import { useLiveStore } from "@/stores/live"

/**
 * A stand-in for i18next's `t`: it echoes the key and its interpolation values, so the assertions
 * can check which key a renderer picked without loading the locale files. The cast is needed
 * because i18next brands its `TFunction` and this is exactly what it does at runtime.
 */
const t = ((key: string, options?: Record<string, unknown>) =>
  options === undefined ? key : `${key}(${JSON.stringify(options)})`) as unknown as TFunction

/** The same, for the pure helpers typed against a plain function. */
const tPlain = (key: string, options?: Record<string, unknown>) =>
  options === undefined ? key : `${key}(${JSON.stringify(options)})`

/** A minimal session of the given status. */
function session(overrides: Partial<SessionDto>): SessionDto {
  return {
    id: 1,
    book_id: 1,
    role: "writer",
    kind: "chapter_write",
    chapter_id: 1,
    track_id: 1,
    research_task_id: null,
    chapter_number: 1,
    research_topic: null,
    model_label: "Demo model",
    status: "running",
    phase: "drafting",
    granularity: "paragraph",
    auto_pause: false,
    pause_after_outline: true,
    pause_requested: false,
    held_tool_call_id: null,
    base_content_version: 0,
    error: null,
    usage: {
      requests: 0,
      input_tokens: 0,
      cached_input_tokens: 0,
      cache_write_tokens: 0,
      output_tokens: 0,
      reasoning_tokens: 0,
      last_prompt_tokens: 0,
      estimated: false,
    },
    counters: {
      steps: 0,
      tool_calls: 0,
      retrieval_calls: 0,
      consecutive_tool_errors: 0,
      consecutive_nudges: 0,
      finish_warned: false,
    },
    parent_session_id: null,
    child_session_id: null,
    retrying: null,
    warnings: [],
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    finished_at: null,
    ...overrides,
  }
}

/** A minimal tool call. */
function toolCall(overrides: Partial<ToolCallDto>): ToolCallDto {
  return {
    id: 1,
    message_id: 1,
    seq: 1,
    call_index: 0,
    name: "echo",
    input: { text: "hi" },
    input_text: '{"text":"hi"}',
    status: "done",
    is_error: false,
    result_text: "hi",
    appendices: [],
    ui: null,
    sealed: true,
    created_at: "2026-01-01T00:00:00Z",
    finished_at: "2026-01-01T00:00:00Z",
    ...overrides,
  }
}

describe("live buffers", () => {
  it("accumulates text deltas per message index", () => {
    const store = useLiveStore.getState()
    store.clearSeq(1, 0)
    store.applyDelta(1, 0, "text", 0, "Hel")
    store.applyDelta(1, 0, "text", 0, "lo")
    store.applyDelta(1, 0, "text", 1, " World")
    const stream = useLiveStore.getState().streams[1]
    expect(stream.seq).toBe(0)
    expect(stream.text[0]).toBe("Hello")
    expect(stream.text[1]).toBe(" World")
  })

  it("keeps reasoning separate from text", () => {
    useLiveStore.getState().clearSeq(2, 0)
    useLiveStore.getState().applyDelta(2, 0, "reasoning", 0, "thinking…")
    useLiveStore.getState().applyDelta(2, 0, "text", 0, "answer")
    const stream = useLiveStore.getState().streams[2]
    expect(stream.reasoning[0]).toBe("thinking…")
    expect(stream.text[0]).toBe("answer")
  })

  it("starts a fresh buffer when the sequence advances", () => {
    useLiveStore.getState().clearSeq(3, 0)
    useLiveStore.getState().applyDelta(3, 0, "text", 0, "old")
    useLiveStore.getState().applyDelta(3, 1, "text", 0, "new")
    const stream = useLiveStore.getState().streams[3]
    expect(stream.seq).toBe(1)
    expect(stream.text[0]).toBe("new")
  })

  it("drops the buffers of a superseded sequence", () => {
    useLiveStore.getState().clearSeq(4, 0)
    useLiveStore.getState().applyDelta(4, 5, "text", 0, "x")
    useLiveStore.getState().clearSeq(4, 4)
    expect(useLiveStore.getState().streams[4]).toBeDefined()
    useLiveStore.getState().clearSeq(4, 5)
    expect(useLiveStore.getState().streams[4]).toBeUndefined()
  })

  it("accumulates a tool-argument preview per call and clears it with its sequence", () => {
    useLiveStore.getState().applyPreview(5, 3, 0, "insert_text", "text", "Hello ")
    useLiveStore.getState().applyPreview(5, 3, 0, "insert_text", "text", "world")
    const preview = useLiveStore.getState().previews["5:3:0"]
    expect(preview.name).toBe("insert_text")
    expect(preview.field).toBe("text")
    expect(preview.text).toBe("Hello world")
    useLiveStore.getState().clearSeq(5, 3)
    expect(useLiveStore.getState().previews["5:3:0"]).toBeUndefined()
  })
})

describe("stream rows", () => {
  it("lists persisted messages in order", () => {
    const messages: MessageWithCallsDto[] = [
      {
        message: {
          id: 1,
          seq: 0,
          role: "user",
          kind: "kickoff",
          blocks: [{ type: "text", text: "task" }],
          stop_reason: null,
          usage: null,
          created_at: "2026-01-01T00:00:00Z",
        },
        tool_calls: [],
      },
      {
        message: {
          id: 2,
          seq: 1,
          role: "assistant",
          kind: "assistant",
          blocks: [{ type: "text", text: "reply" }],
          stop_reason: "tool_use",
          usage: null,
          created_at: "2026-01-01T00:00:00Z",
        },
        tool_calls: [toolCall({})],
      },
    ]
    const rows = buildRows(messages, undefined, tPlain)
    expect(rows).toHaveLength(2)
    expect(rows[0].kind).toBe("message")
    expect(rows[1].kind).toBe("message")
  })

  it("appends a live row while a step is streaming", () => {
    const rows = buildRows(
      [],
      { seq: 7, text: { 0: "streaming" }, reasoning: { 0: "hmm" } },
      tPlain,
    )
    expect(rows).toHaveLength(1)
    expect(rows[0].kind).toBe("live")
    if (rows[0].kind === "live") {
      expect(rows[0].text).toBe("streaming")
      expect(rows[0].reasoning).toBe("hmm")
    }
  })

  it("shows an empty divider when nothing happened yet", () => {
    const rows = buildRows([], undefined, tPlain)
    expect(rows).toHaveLength(1)
    expect(rows[0].kind).toBe("divider")
  })

  it("does not add a live row for empty buffers", () => {
    const rows = buildRows([], { seq: 0, text: {}, reasoning: {} }, tPlain)
    expect(rows[0].kind).toBe("divider")
  })
})

describe("session selection", () => {
  it("prefers the pinned session", () => {
    const pinned = session({ id: 9, status: "completed" })
    const chosen = pickSession([session({ id: 1 })], pinned, 9)
    expect(chosen?.id).toBe(9)
  })

  it("falls back to the newest open session, then the newest", () => {
    const open = pickSession(
      [session({ id: 1, status: "completed" }), session({ id: 2, status: "running" })],
      undefined,
      null,
    )
    expect(open?.id).toBe(2)
    const newest = pickSession([session({ id: 1, status: "completed" })], undefined, null)
    expect(newest?.id).toBe(1)
  })
})

describe("session title", () => {
  it("composes the kind with the chapter number through i18n", () => {
    const title = sessionTitle(session({ chapter_number: 12 }), tPlain)
    expect(title).toContain("agent:session_title.chapter_write")
    expect(title).toContain("agent:chapter_number")
  })

  it("uses the research topic when there is no chapter", () => {
    const title = sessionTitle(
      session({ kind: "research", chapter_number: null, research_topic: "Song markets" }),
      tPlain,
    )
    expect(title).toContain("Song markets")
  })

  it("falls back to the kind alone", () => {
    const title = sessionTitle(
      session({ kind: "conception", chapter_number: null, research_topic: null }),
      tPlain,
    )
    expect(title).toBe("agent:session_title.conception")
  })
})

describe("composer state machine", () => {
  it("steers a running session", () => {
    expect(composerMode(session({ status: "running" }))).toBe("steer")
  })

  it("gives feedback to a held or paused session", () => {
    expect(composerMode(session({ status: "held" }))).toBe("feedback")
    expect(composerMode(session({ status: "paused" }))).toBe("feedback")
  })

  it("replies to an idle conception session only", () => {
    expect(composerMode(session({ status: "idle", kind: "conception" }))).toBe("reply")
    expect(composerMode(session({ status: "idle", kind: "chapter_write" }))).toBe("disabled")
  })

  it("is disabled while a form waits, and after the session ends", () => {
    expect(composerMode(session({ status: "awaiting_input" }))).toBe("disabled")
    expect(disabledReasonKey(session({ status: "awaiting_input" }))).toBe(
      "agent:composer.disabled_form",
    )
    expect(disabledReasonKey(session({ status: "stopped" }))).toBe(
      "agent:composer.disabled_continue",
    )
    expect(disabledReasonKey(session({ status: "completed" }))).toBe(
      "agent:composer.disabled_done",
    )
  })
})

describe("status toasts", () => {
  it("notifies on a transition into a status that needs the author", () => {
    expect(statusNotification(session({ status: "running" }), session({ status: "held" }))).toBe(
      "held",
    )
    expect(
      statusNotification(session({ status: "running" }), session({ status: "failed" })),
    ).toBe("failed")
    expect(
      statusNotification(session({ status: "running" }), session({ status: "completed" })),
    ).toBe("completed")
  })

  it("does not re-notify while the status is unchanged", () => {
    expect(statusNotification(session({ status: "held" }), session({ status: "held" }))).toBeNull()
  })

  it("ignores transitions that need nothing", () => {
    expect(
      statusNotification(session({ status: "held" }), session({ status: "running" })),
    ).toBeNull()
  })

  it("has no notification for the first observation", () => {
    expect(statusNotification(undefined, session({ status: "held" }))).toBe("held")
    expect(statusNotification(undefined, undefined)).toBeNull()
  })
})

describe("tool renderer registry", () => {
  it("summarizes a known tool from its ui payload", () => {
    const renderer = toolRenderer("echo")
    expect(renderer.summary({ chars: 5 }, t)).toBe('agent:tools.echo.summary({"chars":5})')
  })

  it("falls back to the generic summary for an unknown tool", () => {
    const renderer = toolRenderer("no_such_tool")
    expect(renderer.summary({}, t)).toBe("agent:tools.unknown")
    expect(renderer.summary({ a: 1, b: 2 }, t)).toContain("agent:tools.unknown_with")
  })

  it("uses a named summary when the payload carries the data", () => {
    const renderer = toolRenderer("insert_text")
    expect(renderer.summary({ appended_to: "p3", tail_open: true }, t)).toBe(
      'agent:tools.insert_text.summary_appended_open({"pid":"p3"})',
    )
    expect(renderer.summary({ inserted: ["p4", "p5"], tail_open: false }, t)).toBe(
      'agent:tools.insert_text.summary_inserted({"pids":"p4, p5"})',
    )
  })

  it("registers a renderer for every tool of every role list", () => {
    const required = [
      "get_entity",
      "list_entities",
      "get_relations",
      "query_timeline",
      "list_foreshadowing",
      "search_canon",
      "search_knowledge",
      "read_knowledge_entry",
      "search_chapters",
      "read_chapter",
      "submit_outline",
      "submit_revision_plan",
      "read_draft",
      "insert_text",
      "insert_paragraphs",
      "replace_text",
      "rewrite_paragraph",
      "delete_paragraphs",
      "finish_draft",
      "finish_revision",
      "submit_review",
      "create_entity",
      "update_entity",
      "set_relation",
      "end_relation",
      "record_appearances",
      "add_timeline_event",
      "update_timeline_event",
      "add_memory_note",
      "retire_memory_note",
      "add_foreshadowing",
      "resolve_foreshadowing",
      "set_chapter_summary",
      "propose_setting_change",
      "finish_maintenance",
      "ask_user",
      "web_search",
      "web_fetch",
      "write_setting_doc",
      "edit_setting_doc",
      "upsert_character",
      "remove_character",
      "reorder_characters",
      "set_research_topics",
      "request_review",
      "save_knowledge_entry",
      "update_knowledge_entry",
      "finish_research",
    ]
    const registered = new Set(registeredTools())
    for (const name of required) {
      expect(registered.has(name), `no renderer for ${name}`).toBe(true)
    }
  })

  it("handles an empty or malformed ui payload", () => {
    const renderer = toolRenderer("list_entities")
    expect(renderer.summary(null, t)).toContain("agent:tools.list_entities.summary")
    expect(renderer.summary("not an object", t)).toContain("count")
  })
})
