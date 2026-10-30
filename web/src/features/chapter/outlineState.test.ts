/** The derived outline state shared by the header and the Outline tab (D-146). */
import { describe, expect, it } from "vitest"

import type { ChapterDto } from "@/api/gen/ChapterDto"
import { outlineState } from "@/features/chapter/outlineState"

function chapter(overrides: Partial<ChapterDto> = {}): ChapterDto {
  return {
    id: 1,
    book_id: 1,
    track_id: 1,
    number: 1,
    title: "",
    status: "planned",
    canon_status: "none",
    word_count: 0,
    has_outline: false,
    updated_at: "2026-01-01T00:00:00Z",
    guidance: "",
    outline: "",
    reference_notes: "",
    summary: "",
    content: { paragraphs: [], next_pid: 1, tail_open: false },
    content_version: 0,
    official_version_id: null,
    open_sessions: [],
    ...overrides,
  }
}

function outlineSession(status: string) {
  return {
    id: 9,
    book_id: 1,
    role: "planner",
    kind: "chapter_outline",
    chapter_id: 1,
    track_id: 1,
    research_task_id: null,
    chapter_number: 1,
    research_topic: null,
    model_profile_id: null,
    model_label: "Demo",
    status,
    phase: "none",
    granularity: null,
    auto_pause: false,
    pause_after_outline: true,
    pause_requested: false,
    held_tool_call_id: status === "held" ? 5 : null,
    inbox: [],
    inbox_pending: 0,
    error: null,
    retrying: null,
    warnings: [],
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    finished_at: null,
  } as unknown as ChapterDto["open_sessions"][number]
}

describe("outlineState", () => {
  it("is missing for an empty outline and no session", () => {
    expect(outlineState(chapter())).toBe("missing")
    expect(outlineState(undefined)).toBe("missing")
  })

  it("is in progress while an outline session is open", () => {
    expect(outlineState(chapter({ open_sessions: [outlineSession("running")] }))).toBe(
      "in_progress",
    )
    expect(outlineState(chapter({ open_sessions: [outlineSession("paused")] }))).toBe(
      "in_progress",
    )
    // A held retrieval call is still work in progress, not an approval.
    const held = outlineSession("held")
    expect(
      outlineState(chapter({ open_sessions: [held] }), [
        { id: 5, name: "get_entity" } as never,
      ]),
    ).toBe("in_progress")
    // A held call outside the snapshot window has no proposal to show; do not open an empty
    // approval UI (R3 suspicion 2).
    expect(outlineState(chapter({ open_sessions: [held] }), [])).toBe("in_progress")
  })

  it("is awaiting approval when the held call is submit_outline", () => {
    expect(outlineState(chapter({ open_sessions: [outlineSession("held")] }))).toBe(
      "awaiting_approval",
    )
    expect(
      outlineState(chapter({ open_sessions: [outlineSession("held")] }), [
        { id: 5, name: "submit_outline" } as never,
      ]),
    ).toBe("awaiting_approval")
  })

  it("is accepted for a non-empty outline and no session", () => {
    expect(outlineState(chapter({ outline: "## 场景一" }))).toBe("accepted")
    expect(outlineState(chapter({ outline: "   " }))).toBe("missing")
  })
})
