/**
 * The composer's disabled state (PROJECT §8.6, spec/11 §5.3).
 *
 * The author reported that the "continue the session to write again" hint appeared twice side by
 * side in the conception studio: it was the textarea's `placeholder` **and** the hint line under
 * it. The reason has to reach the author (and a screen reader) exactly once, and only the hint line
 * may change with the session state — a control whose accessible name changes while it is focused
 * is announced again by some screen readers.
 */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { describe, expect, it, vi } from "vitest"

import type { SessionDto } from "@/api/gen/SessionDto"
import { Composer, composerMode, disabledReasonKey } from "@/features/agent/Composer"

/** A minimal session of the given status. */
function session(overrides: Partial<SessionDto>): SessionDto {
  return {
    id: 1,
    book_id: 1,
    role: "conception",
    kind: "conception",
    chapter_id: null,
    track_id: null,
    research_task_id: null,
    chapter_number: null,
    research_topic: null,
    model_label: "Demo model",
    status: "running",
    phase: "planning",
    granularity: null,
    auto_pause: false,
    pause_after_outline: false,
    pause_requested: false,
    held_tool_call_id: null,
    base_content_version: null,
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

function renderComposer(current: SessionDto) {
  const onSend = vi.fn()
  const onFeedback = vi.fn()
  render(<Composer session={current} busy={false} onSend={onSend} onFeedback={onFeedback} />)
  return { onSend, onFeedback }
}

describe("Composer", () => {
  it("shows the disabled reason once, in the hint line", () => {
    renderComposer(session({ status: "stopped" }))
    // One node with the sentence, and it is the live-region hint — not the textarea's value,
    // placeholder or name.
    const reason = "Continue the session to write again."
    const matches = screen.getAllByText(reason)
    expect(matches).toHaveLength(1)
    expect(matches[0].getAttribute("aria-live")).toBe("polite")

    const textarea = screen.getByRole("textbox", { name: "Message" })
    expect(textarea).toBeDisabled()
    expect(textarea.getAttribute("placeholder")).toBeNull()
    // The hint is the textarea's description, so the reason is still reachable from it.
    expect(textarea.getAttribute("aria-describedby")).toBe(matches[0].id)
    expect(textarea.textContent).not.toContain(reason)
  })

  it("keeps a stable accessible name while the state changes", () => {
    const { rerender } = render(
      <Composer
        session={session({ status: "running" })}
        busy={false}
        onSend={vi.fn()}
        onFeedback={vi.fn()}
      />,
    )
    expect(screen.getByRole("textbox", { name: "Message" })).toBeInTheDocument()
    // The label is the hint line; the textarea is named by its own stable label.
    expect(screen.getByText("Steer the agent")).toBeInTheDocument()

    rerender(
      <Composer
        session={session({ status: "stopped" })}
        busy={false}
        onSend={vi.fn()}
        onFeedback={vi.fn()}
      />,
    )
    // Same accessible name after the state changed; the reason appears exactly once.
    expect(screen.getByRole("textbox", { name: "Message" })).toBeInTheDocument()
    expect(screen.getAllByText("Continue the session to write again.")).toHaveLength(1)
  })

  it("offers a neutral placeholder while it accepts input", async () => {
    const { onSend } = renderComposer(session({ status: "running" }))
    const textarea = screen.getByRole("textbox", { name: "Message" })
    expect(textarea.getAttribute("placeholder")).toBe("Type a message…")
    await userEvent.type(textarea, "more dialogue{Enter}")
    expect(onSend).toHaveBeenCalledWith("more dialogue")
  })

  it("sends feedback instead of a steering message while the session is held", async () => {
    const { onFeedback, onSend } = renderComposer(session({ status: "held" }))
    expect(screen.getByText("Feedback & continue")).toBeInTheDocument()
    await userEvent.type(screen.getByRole("textbox", { name: "Message" }), "keep it{Enter}")
    expect(onFeedback).toHaveBeenCalledWith("keep it")
    expect(onSend).not.toHaveBeenCalled()
  })

  it("does not send while an IME composition is in progress", async () => {
    const { onSend } = renderComposer(session({ status: "running" }))
    const textarea = screen.getByRole("textbox", { name: "Message" })
    await userEvent.type(textarea, "测试")
    onSend.mockClear()
    // A Enter that belongs to the IME candidate window: `isComposing` is set, so it must be
    // ignored (AGENTS.md §6) rather than submitted.
    const { fireEvent } = await import("@testing-library/react")
    fireEvent.keyDown(textarea, { key: "Enter", isComposing: true })
    expect(onSend).not.toHaveBeenCalled()
  })

  it("points at the review block instead of offering a second reject path", () => {
    // While the panel's held-call review block owns the decision, the composer must not offer
    // "Feedback & continue" as well: the same call would have two differently-worded reject paths
    // (spec/11 §5.3, the studio's `awaiting_input` form has the same relationship with this
    // composer).
    render(
      <Composer
        session={session({ status: "held" })}
        busy={false}
        onSend={vi.fn()}
        onFeedback={vi.fn()}
        reviewingHeld
      />,
    )
    expect(screen.getByText("Decide on the review above to continue.")).toBeInTheDocument()
    expect(screen.queryByText("Feedback & continue")).not.toBeInTheDocument()
    expect(screen.getByRole("textbox", { name: "Message" })).toBeDisabled()
  })

  it("reports the review-block mode through the pure helpers", () => {
    expect(composerMode(session({ status: "held" }), true)).toBe("disabled")
    expect(disabledReasonKey(session({ status: "held" }), true)).toBe("agent:composer.disabled_review")
    // Any other status is unaffected by the flag: it only ever describes a held session.
    expect(composerMode(session({ status: "paused" }), true)).toBe("feedback")
    expect(composerMode(session({ status: "held" }), false)).toBe("feedback")
  })
})
