/**
 * One decision surface per held call (D-141).
 *
 * The author reported the split: 「正文编写要在中间的正文草稿区进行，正文编辑时的意见提出在草稿区
 * 下方的 dock 栏里进行」. A held chapter write therefore belongs to the draft pane's dock, and the
 * agent panel must not keep a competing block — nor a bare Continue, which is the same `resume`
 * with no feedback under a second wording.
 *
 * Fail-before: before this change `SessionControls` handed *every* resolvable held call to
 * `HeldCallReview`, so a held `insert_text` rendered the panel's Approve/Request changes block over
 * the tool's input text while the draft pane showed the same write as a ghost. Two surfaces, two
 * wordings, one mutation.
 */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { MemoryRouter } from "react-router"
import { describe, expect, it, vi } from "vitest"

import type { ChapterContent } from "@/api/gen/ChapterContent"
import type { SessionDto } from "@/api/gen/SessionDto"
import type { ToolCallDto } from "@/api/gen/ToolCallDto"
import { TooltipProvider } from "@/components/ui/tooltip"
import { SessionControls } from "@/features/agent/SessionControls"
import { composerMode, disabledReasonKey } from "@/features/agent/Composer"

function content(paragraphs: [string, string][]): ChapterContent {
  return {
    paragraphs: paragraphs.map(([pid, text]) => ({ pid, text })),
    next_pid: paragraphs.length + 1,
    tail_open: false,
  }
}

/** A held write call that staged content (so the draft pane can show it). */
function writeCall(overrides: Partial<ToolCallDto> = {}): ToolCallDto {
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
    completes: false,
    pending_content: { content: content([["p3", "雨点敲在船篷上。"]]), expected_version: 1 },
    created_at: "2026-01-01T00:00:00Z",
    finished_at: null,
    ...overrides,
  }
}

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
    pause_after_step: false,
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

const noop = () => undefined

/**
 * The panel's `SessionControls` as `AgentPanel` wires it: the call and both handlers always arrive,
 * and the component itself decides which surface owns it (D-141).
 */
function renderDockOwned(chapterHref?: string) {
  const onApproveHeld = vi.fn()
  const onRequestChanges = vi.fn()
  const onContinue = vi.fn()
  render(
    <MemoryRouter>
      <TooltipProvider>
        <SessionControls
          session={session()}
          busy={false}
          onPause={noop}
          onStop={noop}
          onContinue={onContinue}
          onFeedback={noop}
          onAbandon={noop}
          heldCall={writeCall()}
          onApproveHeld={onApproveHeld}
          onRequestChanges={onRequestChanges}
          chapterHref={chapterHref ?? "/books/3/chapters/2"}
        />
      </TooltipProvider>
    </MemoryRouter>,
  )
  return { onApproveHeld, onRequestChanges, onContinue }
}

describe("a held chapter write is decided in the draft pane's dock", () => {
  it("renders no review block and no approve pair in the agent panel", () => {
    renderDockOwned()
    // The block that would decide it, and the pair that would decide it, are both absent; only the
    // pointer to the draft remains.
    expect(screen.queryByTestId("held-review")).not.toBeInTheDocument()
    expect(screen.getByTestId("held-write-notice")).toBeInTheDocument()
    // Neither wording of the approve decision is offered here.
    expect(screen.queryByRole("button", { name: "Approve and continue" })).not.toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Continue" })).not.toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Request changes" })).not.toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Feedback…" })).not.toBeInTheDocument()
  })

  it("explains where the decision is and links to the draft", () => {
    // `AgentPanel` passes the session's own chapter, so a pinned session of another chapter still
    // links to the right page.
    render(
      <MemoryRouter>
        <TooltipProvider>
          <SessionControls
            session={session()}
            busy={false}
            onPause={noop}
            onStop={noop}
            onContinue={noop}
            onFeedback={noop}
            onAbandon={noop}
            heldCall={writeCall()}
            onApproveHeld={noop}
            onRequestChanges={noop}
            chapterHref="/books/3/chapters/2"
          />
        </TooltipProvider>
      </MemoryRouter>,
    )
    expect(screen.getByTestId("held-write-notice")).toBeInTheDocument()
    expect(screen.getByText("The agent's text is waiting in the draft")).toBeInTheDocument()
    expect(screen.getByRole("link", { name: /Open the draft/ })).toHaveAttribute(
      "href",
      "/books/3/chapters/2",
    )
  })

  it("still leaves Abandon reachable, because it is a session operation", async () => {
    // Abandoning is not a decision about this write, so the panel keeps it (behind the same
    // confirmation every destructive action needs).
    const onAbandon = vi.fn()
    render(
      <MemoryRouter>
        <TooltipProvider>
          <SessionControls
            session={session()}
            busy={false}
            onPause={noop}
            onStop={noop}
            onContinue={noop}
            onFeedback={noop}
            onAbandon={onAbandon}
            heldCall={writeCall()}
            onApproveHeld={noop}
            onRequestChanges={noop}
            chapterHref="/books/3/chapters/2"
          />
        </TooltipProvider>
      </MemoryRouter>,
    )
    await userEvent.click(screen.getByRole("button", { name: "More" }))
    await userEvent.click(await screen.findByRole("menuitem", { name: "Abandon" }))
    await userEvent.click(screen.getByRole("button", { name: "Abandon session" }))
    expect(onAbandon).toHaveBeenCalledTimes(1)
  })
})

describe("the composer's reason names the surface that owns the decision", () => {
  it("points at the draft pane for a dock-owned hold", () => {
    expect(disabledReasonKey(session({ status: "held" }), true, true)).toBe(
      "agent:composer.disabled_dock",
    )
  })

  it("keeps pointing at the block above it for every other hold", () => {
    expect(disabledReasonKey(session({ status: "held" }), true, false)).toBe(
      "agent:composer.disabled_review",
    )
  })

  it("stays disabled in both cases, so a held session has one decision", () => {
    expect(composerMode(session({ status: "held" }), true)).toBe("disabled")
  })
})

describe("SessionControls keeps the review block for a non-write hold", () => {
  it("gives a held submit_outline to HeldCallReview, unchanged", () => {
    const onApproveHeld = vi.fn()
    render(
      <MemoryRouter>
        <TooltipProvider>
          <SessionControls
            session={session({ held_tool_call_id: 282 })}
            busy={false}
            onPause={noop}
            onStop={noop}
            onContinue={noop}
            onFeedback={noop}
            onAbandon={noop}
            heldCall={writeCall({
              id: 282,
              name: "submit_outline",
              input: { outline: "## 本章定位\n\n先确认时代。", reference_notes: "" },
              ui: { outline_chars: 12, notes_chars: 0 },
              pending_content: null,
            })}
            onApproveHeld={onApproveHeld}
            onRequestChanges={noop}
          />
        </TooltipProvider>
      </MemoryRouter>,
    )
    expect(screen.getByTestId("held-review")).toHaveAttribute("data-held-case", "outline")
    expect(screen.queryByTestId("held-write-notice")).not.toBeInTheDocument()
    // The panel's own wording for an outline hold is untouched.
    expect(screen.getByRole("button", { name: "Approve outline" })).toBeInTheDocument()
  })

  it("keeps the generic pair when the held call cannot be resolved", () => {
    render(
      <MemoryRouter>
        <TooltipProvider>
          <SessionControls
            session={session({ held_tool_call_id: 999 })}
            busy={false}
            onPause={noop}
            onStop={noop}
            onContinue={noop}
            onFeedback={noop}
            onAbandon={noop}
          />
        </TooltipProvider>
      </MemoryRouter>,
    )
    expect(screen.queryByTestId("held-review")).not.toBeInTheDocument()
    expect(screen.queryByTestId("held-write-notice")).not.toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Continue" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Feedback…" })).toBeInTheDocument()
  })
})
