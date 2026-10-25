/**
 * The session controls (PROJECT §8.6, spec/11 §5.3): abandoning ends a session for good, so it is
 * confirmed like every destructive action (PROJECT §8.13). The dialog lives in this shared
 * component, which is why the studio and the agent panel behave the same.
 */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { describe, expect, it, vi } from "vitest"

import type { SessionDto } from "@/api/gen/SessionDto"
import { SessionControls } from "@/features/agent/SessionControls"

/** A minimal session in the given status. */
function session(overrides: Partial<SessionDto>): SessionDto {
  return {
    id: 4,
    book_id: 1,
    role: "writer",
    kind: "chapter_write",
    chapter_id: 2,
    track_id: 1,
    research_task_id: null,
    chapter_number: 1,
    research_topic: null,
    model_label: "Demo model",
    status: "paused",
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

const noop = () => undefined

function renderControls(onAbandon: () => void) {
  return render(
    <SessionControls
      session={session({})}
      busy={false}
      onPause={noop}
      onStop={noop}
      onContinue={noop}
      onFeedback={noop}
      onAbandon={onAbandon}
    />,
  )
}

describe("SessionControls", () => {
  it("asks before abandoning a session", async () => {
    const onAbandon = vi.fn()
    renderControls(onAbandon)
    await userEvent.click(screen.getByRole("button", { name: "More" }))
    await userEvent.click(await screen.findByRole("menuitem", { name: "Abandon" }))
    // Nothing happens until the author confirms.
    expect(onAbandon).not.toHaveBeenCalled()
    expect(await screen.findByText("Abandon this session?")).toBeInTheDocument()
    await userEvent.click(screen.getByRole("button", { name: "Abandon session" }))
    expect(onAbandon).toHaveBeenCalledTimes(1)
  })

  it("does not abandon when the dialog is cancelled", async () => {
    const onAbandon = vi.fn()
    renderControls(onAbandon)
    await userEvent.click(screen.getByRole("button", { name: "More" }))
    await userEvent.click(await screen.findByRole("menuitem", { name: "Abandon" }))
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }))
    expect(onAbandon).not.toHaveBeenCalled()
  })
})
