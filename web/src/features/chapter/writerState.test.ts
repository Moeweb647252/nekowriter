/** The writer state the reading column derives (spec/11 §5.2, PLAN M4.6). */
import { describe, expect, it } from "vitest"

import type { ChapterDto } from "@/api/gen/ChapterDto"
import type { SessionDto } from "@/api/gen/SessionDto"
import type { ToolCallDto } from "@/api/gen/ToolCallDto"
import { heldPids, openWriterSession } from "@/features/chapter/useWriterState"

const session = (overrides: Partial<SessionDto> = {}): SessionDto => ({
  id: 1,
  book_id: 1,
  role: "writer",
  kind: "chapter_write",
  chapter_id: 7,
  track_id: 1,
  research_task_id: null,
  chapter_number: 1,
  research_topic: null,
  model_label: "Demo",
  context_window: 100000,
  status: "running",
  phase: "drafting",
  granularity: "paragraph",
  auto_pause: false,
  pause_after_outline: false,
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
})

const chapter = (sessions: SessionDto[]): ChapterDto =>
  ({ id: 7, open_sessions: sessions }) as unknown as ChapterDto

const call = (overrides: Partial<ToolCallDto> = {}): ToolCallDto =>
  ({
    id: 10,
    message_id: 1,
    seq: 0,
    call_index: 0,
    name: "insert_text",
    input: null,
    input_text: "",
    status: "held",
    is_error: false,
    result_text: null,
    appendices: [],
    ui: null,
    sealed: false,
    created_at: "2026-01-01T00:00:00Z",
    finished_at: null,
    ...overrides,
  }) as ToolCallDto

describe("openWriterSession", () => {
  it("finds the chapter's writer session among its open sessions", () => {
    const reviewer = session({ id: 2, kind: "chapter_review", role: "reviewer" })
    const writer = session({ id: 1 })
    expect(openWriterSession(chapter([reviewer, writer]))?.id).toBe(1)
  })

  it("treats a revision session as a writer session", () => {
    const revise = session({ id: 3, kind: "chapter_revise" })
    expect(openWriterSession(chapter([revise]))?.id).toBe(3)
  })

  it("treats an outline/planner session as the chapter's owner too", () => {
    const planner = session({
      id: 4,
      role: "planner",
      kind: "chapter_outline",
      phase: "none",
    })
    expect(openWriterSession(chapter([planner]))?.id).toBe(4)
  })

  it("returns undefined when the chapter has no writer session", () => {
    expect(openWriterSession(chapter([]))).toBeUndefined()
    expect(openWriterSession(undefined)).toBeUndefined()
  })
})

describe("heldPids", () => {
  it("reads the target pids of a held text call", () => {
    const held = call({ ui: { inserted: ["p12", "p13"], appended_to: "p11" } })
    expect(heldPids(held)).toEqual(["p12", "p13"])
  })

  it("reads the pid of a replace or rewrite call", () => {
    expect(heldPids(call({ name: "replace_text", ui: { pid: "p4", before: "a", after: "b" } }))).toEqual([
      "p4",
    ])
  })

  it("reads the deleted pids", () => {
    expect(heldPids(call({ name: "delete_paragraphs", ui: { deleted: ["p8", "p9"] } }))).toEqual([
      "p8",
      "p9",
    ])
  })

  it("deduplicates a pid that appears twice", () => {
    expect(heldPids(call({ ui: { pid: "p1", inserted: ["p1"] } }))).toEqual(["p1"])
  })

  it("is empty without a ui payload", () => {
    expect(heldPids(undefined)).toEqual([])
    expect(heldPids(call({ ui: null }))).toEqual([])
  })
})
