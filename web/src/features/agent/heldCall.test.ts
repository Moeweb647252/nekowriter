/**
 * The held-call decision as data — shared by the agent panel and the draft pane's dock (D-137,
 * D-141).
 *
 * The two mutation payloads are the whole point of this module, and the blank-feedback case is the
 * one that silently does the opposite of what the author asked: `services/sessions.rs`
 * `resume_inner` filters blank feedback to `None` and then **finalizes** the held call, which is the
 * approve path. So `requestChangesInput` must return `undefined` rather than an empty payload, and
 * `approveHeldInput` must carry no `feedback` key at all.
 *
 * Fail-before: before D-141 these payloads were built inline in two places (`AgentPanel`'s
 * `approveHeld` and `HeldCallReview`'s submit), so nothing asserted the shape once, and nothing
 * decided *where* a write hold is reviewed — `SessionControls` gave every resolvable hold to
 * `HeldCallReview`, including a held `insert_text`.
 */
import { describe, expect, it } from "vitest"

import type { ChapterContent } from "@/api/gen/ChapterContent"
import type { SessionDto } from "@/api/gen/SessionDto"
import type { ToolCallDto } from "@/api/gen/ToolCallDto"
import {
  approveHeldInput,
  findHeldCall,
  heldDecisionSurface,
  heldReviewCase,
  heldTargetPids,
  isChapterWriteTool,
  requestChangesInput,
  stagedContent,
} from "@/features/agent/heldCall"

function content(paragraphs: [string, string][]): ChapterContent {
  return {
    paragraphs: paragraphs.map(([pid, text]) => ({ pid, text })),
    next_pid: paragraphs.length + 1,
    tail_open: false,
  }
}

/** A held writer session, as `SessionDto` shapes it (only the fields under test are meaningful). */
function session(overrides: Partial<SessionDto> = {}): SessionDto {
  return {
    id: 9,
    book_id: 3,
    role: "writer",
    kind: "chapter_write",
    chapter_id: 2,
    track_id: 1,
    research_task_id: null,
    chapter_number: 1,
    research_topic: null,
    model_label: "Demo model",
    context_window: 100000,
    status: "held",
    phase: "drafting",
    granularity: "paragraph",
    auto_pause: false,
    pause_after_outline: false,
    pause_requested: false,
    held_tool_call_id: 283,
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
      research_budget_noticed: false,
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

function call(overrides: Partial<ToolCallDto> = {}): ToolCallDto {
  return {
    id: 283,
    message_id: 124,
    seq: 3,
    call_index: 0,
    name: "insert_text",
    input: { text: "雨点敲在船篷上。" },
    input_text: "",
    status: "held",
    is_error: false,
    result_text: null,
    appendices: [],
    ui: { inserted: [], appended_to: "p2", tail_open: false, word_count: 8 },
    sealed: false,
    pending_content: { content: content([["p3", "雨点敲在船篷上。"]]), expected_version: 1 },
    created_at: "2026-01-01T00:00:00Z",
    finished_at: null,
    ...overrides,
  }
}

describe("approveHeldInput", () => {
  it("sends the session id and no feedback key at all", () => {
    // Not `{ sessionId, feedback: undefined }` and not `""`: the resume hook omits the body key
    // only for `undefined`, and the server filters `""` to `None` — the same approve path, but the
    // payload has to say what it means.
    expect(approveHeldInput(9)).toEqual({ sessionId: 9 })
    expect(Object.keys(approveHeldInput(9))).toEqual(["sessionId"])
  })
})

describe("requestChangesInput", () => {
  it("sends exactly the trimmed feedback", () => {
    expect(requestChangesInput(9, "  再冷一点。 ")).toEqual({ sessionId: 9, feedback: "再冷一点。" })
  })

  it("refuses blank and whitespace-only feedback instead of approving the call", () => {
    // `resume_inner` filters blank feedback to `None`, which finalizes (approves) the held call.
    expect(requestChangesInput(9, "")).toBeUndefined()
    expect(requestChangesInput(9, "   ")).toBeUndefined()
    expect(requestChangesInput(9, "\n\t ")).toBeUndefined()
  })
})

describe("heldDecisionSurface", () => {
  it("gives a staged chapter write to the draft pane's dock", () => {
    // D-141: the text is the draft, so the decision belongs next to the merged document.
    for (const name of [
      "insert_text",
      "insert_paragraphs",
      "replace_text",
      "rewrite_paragraph",
      "delete_paragraphs",
      "finish_draft",
    ]) {
      expect(heldDecisionSurface(call({ name })), name).toBe("dock")
    }
  })

  it("keeps every other hold in the agent panel", () => {
    // The two designed review holds, and a non-write tool held by `auto_pause` (spec/03 §5.2).
    expect(heldDecisionSurface(call({ name: "submit_outline", pending_content: null }))).toBe(
      "panel",
    )
    expect(heldDecisionSurface(call({ name: "submit_revision_plan", pending_content: null }))).toBe(
      "panel",
    )
    expect(heldDecisionSurface(call({ name: "search_canon", pending_content: null }))).toBe("panel")
    expect(heldDecisionSurface(undefined)).toBe("panel")
  })

  it("keeps a write hold in the panel when it staged nothing", () => {
    // A call from before D-139 carries no `pending_content`, so the dock would have no text to
    // show; the panel's status-driven controls keep that decision rather than pointing at nothing.
    expect(heldDecisionSurface(call({ pending_content: null }))).toBe("panel")
  })
})

describe("isChapterWriteTool", () => {
  it("covers the write tools that stage content", () => {
    expect(isChapterWriteTool("insert_text")).toBe(true)
    expect(isChapterWriteTool("finish_draft")).toBe(true)
    expect(isChapterWriteTool("submit_outline")).toBe(false)
    expect(isChapterWriteTool("read_draft")).toBe(false)
  })
})

describe("stagedContent", () => {
  it("is the call's pending content, and nothing for a call without it", () => {
    expect(stagedContent(call())?.expected_version).toBe(1)
    expect(stagedContent(call({ pending_content: null }))).toBeUndefined()
    expect(stagedContent(undefined)).toBeUndefined()
  })
})

describe("heldTargetPids", () => {
  it("collects the pid, the inserted pids and the deleted pids without duplicates", () => {
    expect(heldTargetPids(call({ ui: { pid: "p4" } }))).toEqual(["p4"])
    expect(heldTargetPids(call({ ui: { inserted: ["p5", "p6"] } }))).toEqual(["p5", "p6"])
    expect(heldTargetPids(call({ ui: { deleted: ["p1"] } }))).toEqual(["p1"])
    expect(heldTargetPids(call({ ui: { pid: "p1", inserted: ["p1", "p2"], deleted: ["p1"] } }))).toEqual(
      ["p1", "p2"],
    )
  })

  it("is empty for a payload that names none, or no payload at all", () => {
    expect(heldTargetPids(call({ ui: null }))).toEqual([])
    expect(heldTargetPids(call({ ui: { word_count: 3 } }))).toEqual([])
    expect(heldTargetPids(undefined)).toEqual([])
  })
})

describe("heldReviewCase and findHeldCall stay the panel's contract", () => {
  it("keeps the two review wordings and the generic fallback", () => {
    expect(heldReviewCase("submit_outline")).toBe("outline")
    expect(heldReviewCase("submit_revision_plan")).toBe("revision_plan")
    expect(heldReviewCase("insert_text")).toBe("generic")
  })

  it("resolves only a held session's own call", () => {
    const held = call()
    expect(findHeldCall(session({ held_tool_call_id: 283 }), [held], [])).toBe(held)
    // `held_tool_call_id` is cleared when the call is finalized, so a stale id on a non-held row
    // must not resurrect a review block for a decision that was already made.
    expect(
      findHeldCall(session({ status: "running", held_tool_call_id: 283 }), [held], []),
    ).toBeUndefined()
    expect(findHeldCall(session({ held_tool_call_id: 999 }), [held], [])).toBeUndefined()
    expect(findHeldCall(session({ held_tool_call_id: null }), [held], [])).toBeUndefined()
  })
})
