/**
 * The session controls (PROJECT §8.6, spec/11 §5.3): abandoning ends a session for good, so it is
 * confirmed like every destructive action (PROJECT §8.13). The dialog lives in this shared
 * component, which is why the studio and the agent panel behave the same.
 *
 * The `paused` status is where these cases live now: a `held` session that can name its tool call
 * hands the approve/request-changes decision to `HeldCallReview`, which `heldCallReview.test.tsx`
 * covers. What is asserted here is the part that did *not* change — and the degradation that keeps
 * a held session usable when its call cannot be resolved.
 */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { describe, expect, it, vi } from "vitest"

import type { SessionDto } from "@/api/gen/SessionDto"
import { TooltipProvider } from "@/components/ui/tooltip"
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
    context_window: 100000,
    status: "paused",
    phase: "drafting",
    granularity: "paragraph",
    auto_pause: false,
    pause_after_outline: false,
    pause_requested: false,
    pause_after_step: false,
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
  }
}

const noop = () => undefined

function renderControls(onAbandon: () => void, overrides: Partial<SessionDto> = {}) {
  return render(
    <TooltipProvider>
      <SessionControls
        session={session(overrides)}
        busy={false}
        onPause={noop}
        onStop={noop}
        onContinue={noop}
        onFeedback={noop}
        onAbandon={onAbandon}
      />
    </TooltipProvider>,
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

  it("keeps Continue and Feedback… for a paused session", () => {
    renderControls(noop, { status: "paused" })
    expect(screen.getByRole("button", { name: "Continue" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Feedback…" })).toBeInTheDocument()
  })

  it("keeps the generic pair for a held session whose call cannot be resolved", () => {
    // The panel passes no `heldCall` when the id is outside the snapshot window; the session stays
    // approvable and rejectable instead of showing a review block for an unknown call.
    renderControls(noop, { status: "held", held_tool_call_id: 999 })
    expect(screen.getByRole("button", { name: "Continue" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Feedback…" })).toBeInTheDocument()
    expect(screen.queryByTestId("held-review")).not.toBeInTheDocument()
  })

  it("offers only Continue for a stopped session, with no feedback path", () => {
    renderControls(noop, { status: "stopped" })
    expect(screen.getByRole("button", { name: "Continue" })).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Feedback…" })).not.toBeInTheDocument()
  })
})

it("labels a pending step pause as pausing after this step", () => {
  renderControls(noop, { status: "running", pause_requested: false, pause_after_step: true })
  expect(screen.getByRole("button", { name: "Pausing after this step…" })).toBeInTheDocument()
  expect(screen.queryByRole("button", { name: "Pausing…" })).not.toBeInTheDocument()
})
