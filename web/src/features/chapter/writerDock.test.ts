import { describe, expect, it } from "vitest"

import type { SessionDto } from "@/api/gen/SessionDto"
import type { ToolCallDto } from "@/api/gen/ToolCallDto"
import { writerDockState, type WriterDockInput } from "./writerDock"

function session(overrides: Partial<SessionDto> = {}): SessionDto {
  return {
    id: 1, book_id: 1, role: "writer", kind: "chapter_write", chapter_id: 1, track_id: 1,
    research_task_id: null, chapter_number: 1, research_topic: null, research_task_kind: null,
    model_label: "Demo", context_window: 1000, effort: null, status: "running", phase: "drafting",
    granularity: "paragraph", auto_pause: true, pause_after_outline: false, write_mode: null,
    pause_requested: false, pause_after_step: false, held_tool_call_id: null,
    base_content_version: 0, error: null, usage: {} as SessionDto["usage"],
    counters: {} as SessionDto["counters"], parent_session_id: null, child_session_id: null,
    retrying: null, warnings: [], created_at: "", updated_at: "", finished_at: null,
    ...overrides,
  }
}

function call(overrides: Partial<ToolCallDto> = {}): ToolCallDto {
  return {
    id: 7, message_id: 1, seq: 1, call_index: 0, name: "insert_text", input: { text: "hi" },
    input_text: "", status: "held", is_error: false, result_text: null, appendices: [], ui: null,
    sealed: false, pending_content: null, pending_outline: null, completes: false,
    created_at: "", finished_at: null,
    ...overrides,
  }
}

function state(overrides: Partial<WriterDockInput>): ReturnType<typeof writerDockState> {
  return writerDockState({
    session: session(), held: undefined, surface: undefined, standing: undefined,
    ownPause: false, text: "", ...overrides,
  })
}

describe("writerDockState", () => {
  it("is hidden outside an active writer phase", () => {
    expect(state({ session: session({ phase: "revision_planning" }) }).visible).toBe(false)
    expect(state({ session: session({ status: "completed" }) }).visible).toBe(false)
    expect(state({ session: undefined }).visible).toBe(false)
  })

  it("auto-pause on: running steers, held approves or requests changes", () => {
    expect(state({})).toMatchObject({ button: "approve", buttonEnabled: false, submit: null })
    expect(state({ text: "more" })).toMatchObject({ button: "send", submit: "message" })
    const held = state({ held: call(), surface: "dock", standing: "current" })
    expect(held).toMatchObject({ button: "approve", submit: "approve", state: "held" })
    expect(state({ held: call(), surface: "dock", standing: "current", text: "x" }).submit).toBe("feedback")
  })

  it("auto-pause off: focus requests a step pause, text sends feedback", () => {
    const off = session({ auto_pause: false })
    expect(state({ session: off })).toMatchObject({
      button: "send",
      buttonEnabled: false,
      onFocus: "pause_after_step",
      submit: null,
    })
    expect(state({ session: off, text: "note" })).toMatchObject({
      button: "send",
      buttonEnabled: true,
      submit: "feedback",
    })
    expect(state({ session: off, ownPause: true })).toMatchObject({
      button: "send",
      buttonEnabled: false,
      onLeave: "resume",
    })
    expect(state({ session: off, ownPause: true, text: "note" })).toMatchObject({
      buttonEnabled: true,
      submit: "feedback",
    })
  })

  it("paused sessions: auto-pause on approves; off leaves Send disabled unless text is present", () => {
    const paused = session({ status: "paused" })
    expect(state({ session: paused })).toMatchObject({
      button: "approve",
      buttonEnabled: true,
      submit: "resume",
    })
    expect(state({ session: paused, text: "note" })).toMatchObject({ submit: "feedback" })
    const off = { ...paused, auto_pause: false }
    expect(state({ session: off, ownPause: true })).toMatchObject({
      button: "send",
      buttonEnabled: false,
      submit: null,
      onLeave: "resume",
    })
    expect(state({ session: off, ownPause: false })).toMatchObject({
      button: "send",
      buttonEnabled: false,
      submit: null,
      onLeave: null,
    })
    expect(state({ session: off, ownPause: true, text: "note" })).toMatchObject({
      buttonEnabled: true,
      submit: "feedback",
      onLeave: null,
    })
  })

  it("uses the server step flag when a reload loses the local ownPause bit", () => {
    const off = session({ auto_pause: false, pause_requested: true, pause_after_step: true })
    expect(state({ session: off, ownPause: false })).toMatchObject({
      state: "off_pausing",
      submit: null,
      onLeave: "resume",
    })
  })

  it("marks a superseded held write and keeps only the keep-write action with text", () => {
    const off = session({ auto_pause: false })
    expect(
      state({ session: off, held: call(), surface: "dock", standing: "superseded", text: "note" }),
    ).toMatchObject({ state: "superseded", submit: "feedback_keep", buttonEnabled: true })
    expect(
      state({ session: off, held: call(), surface: "dock", standing: "superseded" }),
    ).toMatchObject({ state: "superseded", submit: "approve" })
  })

  it("panel-owned holds disable the dock and auto-pause-off holds keep the write", () => {
    expect(state({ held: call(), surface: "panel" })).toMatchObject({ disabled: true, state: "panel_hold" })
    expect(
      state({ session: session({ auto_pause: false }), held: call({ completes: true }), surface: "dock", text: "keep" }),
    ).toMatchObject({ submit: "feedback_keep", state: "held_keep" })
  })

  it("changes the placeholder for a completing held call", () => {
    expect(state({ held: call({ completes: true }), surface: "dock" }).placeholder).toContain("held_completes")
  })
})
