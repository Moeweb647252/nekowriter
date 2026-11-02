/**
 * The held-call review block (PROJECT §8.6, spec/11 §5.3, spec/03 §5.2/§5.4).
 *
 * What this covers, and why it is behavioural rather than presentational: the author reported
 * 「章节细纲等待我的审阅，但是侧边栏没有审阅能力」 — a session held on `submit_outline` offered
 * only the generic Continue / Feedback… pair, so nothing named the tool, showed the outline, or
 * said that Continue means "approve this text". These tests assert the two **mutation payloads**
 * the panel produces (the whole point of the change):
 *
 * - Approve calls `resume` with **no** feedback — the documented approve path, which finalizes
 *   the held call with its deferred effects (`services/sessions.rs` `resume_inner`);
 * - Request changes calls it with **exactly** the typed feedback, which is what revokes those
 *   effects and appends the revoke notice plus the feedback (spec/03 §5.4).
 *
 * The last block drives the panel itself, because the wiring (which call is resolved from the
 * snapshot, and what the click payload is) is what shipped broken, not the block.
 */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeEach, describe, expect, it, vi } from "vitest"

import type { SessionDto } from "@/api/gen/SessionDto"
import type { ToolCallDto } from "@/api/gen/ToolCallDto"
import type { MessageWithCallsDto } from "@/api/gen/MessageWithCallsDto"

/** The resume mutation the panel calls, with its exact variables. */
const resumeMutate = vi.hoisted(() => vi.fn())

/**
 * What the mocked session hooks answer. `session` is the row the panel starts from,
 * `snapshot` the `SessionSnapshotDto` it fetches; the panel tests set both.
 */
const hooks = vi.hoisted(() => ({
  session: undefined as unknown,
  snapshot: { messages: [] as unknown[], tool_calls: [] as unknown[] },
}))

vi.mock("@/api/hooks/sessions", () => ({
  useBookSessions: () => ({ data: [hooks.session], isPending: false }),
  useSession: () => ({
    data: {
      session: hooks.session,
      messages: hooks.snapshot.messages,
      tool_calls: hooks.snapshot.tool_calls,
      next_cursor: null,
    },
    isPending: false,
  }),
  useResumeSession: () => ({ mutate: resumeMutate, isPending: false }),
  useSendSessionMessage: () => ({ mutate: vi.fn(), isPending: false }),
  useSessionControl: () => ({ mutate: vi.fn(), isPending: false }),
  usePatchSession: () => ({ mutate: vi.fn(), isPending: false }),
  useStartDebugSession: () => ({ mutate: vi.fn(), isPending: false }),
  useOlderMessages: () => ({
    data: undefined,
    hasNextPage: false,
    isFetchingNextPage: false,
    isFetchNextPageError: false,
    error: null,
    fetchNextPage: vi.fn(),
  }),
}))

vi.mock("@/api/hooks/chapters", () => ({
  useChapter: () => ({ data: undefined, isPending: false }),
}))

vi.mock("@/components/common/ErrorToast", () => ({
  useErrorToast: () => vi.fn(),
  useWarningToasts: () => vi.fn(),
}))

vi.mock("sonner", () => ({
  toast: { message: vi.fn(), success: vi.fn(), error: vi.fn(), warning: vi.fn() },
}))

import { TooltipProvider } from "@/components/ui/tooltip"

import { HeldCallReview, findHeldCall, heldBodyText, heldReviewCase } from "@/features/agent/HeldCallReview"
import { SessionControls } from "@/features/agent/SessionControls"
import { AgentPanel } from "@/features/agent/AgentPanel"

/** A held `submit_outline` call, shaped like the author's call 282. */
function outlineCall(overrides: Partial<ToolCallDto> = {}): ToolCallDto {
  return {
    id: 282,
    message_id: 124,
    seq: 0,
    call_index: 0,
    name: "submit_outline",
    input: {
      title: "醒来已是洪武元年",
      outline: "## 本章定位\n\n林昭穿越到大明洪武元年，先确认时代，再寻落脚点。",
      reference_notes: "应天府城门、宝钞尚未发行。",
    },
    input_text: "",
    status: "held",
    is_error: false,
    result_text: null,
    appendices: [],
    ui: { outline_chars: 33, notes_chars: 15 },
    sealed: false,
    created_at: "2026-01-01T00:00:00Z",
    finished_at: null,
    ...overrides,
  }
}

/** A held session pointing at the given call. */
function heldSession(callId: number, overrides: Partial<SessionDto> = {}): SessionDto {
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
    status: "held",
    phase: "planning",
    granularity: "free",
    auto_pause: true,
    pause_after_outline: true,
    pause_requested: false,
    held_tool_call_id: callId,
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

describe("heldReviewCase", () => {
  it("gives the two holds that exist for review their own wording", () => {
    expect(heldReviewCase("submit_outline")).toBe("outline")
    expect(heldReviewCase("submit_revision_plan")).toBe("revision_plan")
  })

  it("falls back to the generic wording for every other held tool", () => {
    // `auto_pause` holds any writer tool, and `pause_requested` can hold anything (spec/03 §5.2).
    for (const name of ["insert_text", "finish_draft", "delete_paragraphs", "echo"]) {
      expect(heldReviewCase(name)).toBe("generic")
    }
  })
})

describe("heldBodyText", () => {
  it("reads the outline of submit_outline and the plan of submit_revision_plan", () => {
    expect(heldBodyText(outlineCall())).toContain("林昭穿越")
    expect(
      heldBodyText(outlineCall({ name: "submit_revision_plan", input: { plan: "Extend the ending." } })),
    ).toBe("Extend the ending.")
  })

  it("falls back to the prose field of a held write tool", () => {
    expect(heldBodyText(outlineCall({ name: "insert_text", input: { text: "A new paragraph." } }))).toBe(
      "A new paragraph.",
    )
    expect(
      heldBodyText(outlineCall({ name: "rewrite_paragraph", input: { pid: "p2", text: "Rewritten." } })),
    ).toBe("Rewritten.")
  })

  it("is empty for a call whose input carries no body at all", () => {
    // A held call with a non-object or empty input must not crash the block.
    expect(heldBodyText(outlineCall({ name: "finish_draft", input: {} }))).toBe("")
    expect(heldBodyText(outlineCall({ name: "finish_draft", input: null }))).toBe("")
    expect(heldBodyText(outlineCall({ name: "finish_draft", input: ["not", "an", "object"] }))).toBe("")
  })
})

describe("findHeldCall", () => {
  const call = outlineCall()

  it("resolves the held call from the snapshot's tool calls", () => {
    expect(findHeldCall(heldSession(282), [call], [])).toBe(call)
  })

  it("finds a call that is only reachable through its message", () => {
    const message = { message: { id: 124 }, tool_calls: [call] } as unknown as MessageWithCallsDto
    expect(findHeldCall(heldSession(282), undefined, [message])).toBe(call)
  })

  it("returns nothing when the session is not held, or names no call", () => {
    expect(findHeldCall(heldSession(282, { status: "paused" }), [call], [])).toBeUndefined()
    expect(findHeldCall(heldSession(0, { held_tool_call_id: null }), [call], [])).toBeUndefined()
  })

  it("returns nothing when the call is outside the snapshot window", () => {
    // The snapshot holds the last 200 messages; a call older than that cannot be shown, and the
    // caller has to degrade instead of guessing.
    expect(findHeldCall(heldSession(999), [call], [])).toBeUndefined()
    expect(findHeldCall(undefined, [call], [])).toBeUndefined()
  })
})

describe("HeldCallReview — the two mutation payloads", () => {
  it("renders the outline, so the decision is made on readable text", () => {
    render(<HeldCallReview call={outlineCall()} busy={false} onApprove={vi.fn()} onRequestChanges={vi.fn()} />)
    // The rendered markdown, not the raw input JSON: the heading of the outline is in the DOM.
    expect(screen.getByRole("heading", { name: "本章定位" })).toBeInTheDocument()
    expect(screen.getByText(/林昭穿越到大明洪武元年/)).toBeInTheDocument()
    // The reference notes are available too (they are part of what is submitted, spec/05 §4).
    expect(screen.getByText("Reference notes")).toBeInTheDocument()
    expect(screen.getByText(/宝钞尚未发行/)).toBeInTheDocument()
  })

  it("names the tool through the existing renderer, never its identifier", () => {
    render(<HeldCallReview call={outlineCall()} busy={false} onApprove={vi.fn()} onRequestChanges={vi.fn()} />)
    expect(screen.getByText("A chapter outline is waiting for your review")).toBeInTheDocument()
    // The card summary the stream shows for the same call (spec/11 §5.3), same wording.
    expect(screen.getByText(/Submitted an outline \(33 chars\)/)).toBeInTheDocument()
    expect(screen.queryByText(/submit_outline/)).not.toBeInTheDocument()
  })

  it("approves with onApprove and no feedback", async () => {
    const onApprove = vi.fn()
    const onRequestChanges = vi.fn()
    render(
      <HeldCallReview call={outlineCall()} busy={false} onApprove={onApprove} onRequestChanges={onRequestChanges} />,
    )
    await userEvent.click(screen.getByRole("button", { name: "Approve outline" }))
    expect(onApprove).toHaveBeenCalledTimes(1)
    // The approve path carries no argument at all: there is no feedback to send.
    expect(onApprove).toHaveBeenCalledWith()
    expect(onRequestChanges).not.toHaveBeenCalled()
  })

  it("sends exactly the typed feedback on request-changes", async () => {
    const onApprove = vi.fn()
    const onRequestChanges = vi.fn()
    render(
      <HeldCallReview call={outlineCall()} busy={false} onApprove={onApprove} onRequestChanges={onRequestChanges} />,
    )
    // The box starts closed; the action reveals it.
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole("button", { name: "Request changes" }))
    const box = screen.getByRole("textbox", { name: "What should change?" })
    await userEvent.type(box, "  先把进城那段写细一点。  ")
    await userEvent.click(screen.getByRole("button", { name: "Send and continue" }))
    // Exactly the typed text, trimmed — and never a blank string, which the server would read as
    // "no feedback" and thus as an approval.
    expect(onRequestChanges).toHaveBeenCalledTimes(1)
    expect(onRequestChanges).toHaveBeenCalledWith("先把进城那段写细一点。")
    expect(onApprove).not.toHaveBeenCalled()
  })

  it("cannot request changes with blank or whitespace-only feedback", async () => {
    const onRequestChanges = vi.fn()
    render(
      <HeldCallReview
        call={outlineCall()}
        busy={false}
        onApprove={vi.fn()}
        onRequestChanges={onRequestChanges}
      />,
    )
    await userEvent.click(screen.getByRole("button", { name: "Request changes" }))
    const submit = screen.getByRole("button", { name: "Send and continue" })
    expect(submit).toBeDisabled()
    await userEvent.type(screen.getByRole("textbox", { name: "What should change?" }), "   ")
    // Blank feedback would be filtered to `None` by the server and *approve* the call instead.
    expect(submit).toBeDisabled()
    await userEvent.click(submit)
    expect(onRequestChanges).not.toHaveBeenCalled()
  })

  it("uses the generic wording for an auto-paused tool and still approves/rejects", async () => {
    const onApprove = vi.fn()
    const onRequestChanges = vi.fn()
    const call = outlineCall({
      name: "insert_text",
      input: { text: "雨点敲在船篷上。" },
      ui: { chars: 7 },
    })
    render(<HeldCallReview call={call} busy={false} onApprove={onApprove} onRequestChanges={onRequestChanges} />)
    expect(screen.getByText("The agent is waiting for your approval")).toBeInTheDocument()
    expect(screen.getByText(/雨点敲在船篷上/)).toBeInTheDocument()
    await userEvent.click(screen.getByRole("button", { name: "Approve and continue" }))
    expect(onApprove).toHaveBeenCalledTimes(1)
    await userEvent.click(screen.getByRole("button", { name: "Request changes" }))
    await userEvent.type(screen.getByRole("textbox", { name: "What should change?" }), "再冷一点")
    await userEvent.click(screen.getByRole("button", { name: "Send and continue" }))
    expect(onRequestChanges).toHaveBeenCalledWith("再冷一点")
  })

  it("labels a held revision plan with its own words", async () => {
    const onApprove = vi.fn()
    const call = outlineCall({ name: "submit_revision_plan", input: { plan: "Tighten chapter 2." } })
    render(<HeldCallReview call={call} busy={false} onApprove={onApprove} onRequestChanges={vi.fn()} />)
    expect(screen.getByText("A revision plan is waiting for your review")).toBeInTheDocument()
    await userEvent.click(screen.getByRole("button", { name: "Approve revision plan" }))
    expect(onApprove).toHaveBeenCalledTimes(1)
  })

  it("disables both decisions while a mutation is in flight", () => {
    render(<HeldCallReview call={outlineCall()} busy onApprove={vi.fn()} onRequestChanges={vi.fn()} />)
    expect(screen.getByRole("button", { name: "Approve outline" })).toBeDisabled()
    expect(screen.getByRole("button", { name: "Request changes" })).toBeDisabled()
  })

  it("renders a call with no readable body without crashing", () => {
    // A held `finish_draft` carries no text; the block still names it and offers the decision.
    render(
      <HeldCallReview
        call={outlineCall({ name: "finish_draft", input: { note: "done" } })}
        busy={false}
        onApprove={vi.fn()}
        onRequestChanges={vi.fn()}
      />,
    )
    expect(screen.getByText("The agent is waiting for your approval")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Approve and continue" })).toBeInTheDocument()
  })
})

describe("SessionControls — a held session with a resolvable call", () => {
  const noop = () => undefined

  function renderHeld(heldCall: ToolCallDto | undefined, handlers: {
    onApprove?: () => void
    onRequestChanges?: (feedback: string) => void
    onContinue?: () => void
    onFeedback?: (feedback: string) => void
  } = {}) {
    return render(
      <TooltipProvider>
        <SessionControls
          session={heldSession(282)}
          busy={false}
          onPause={noop}
          onStop={noop}
          onContinue={handlers.onContinue ?? noop}
          onFeedback={handlers.onFeedback ?? noop}
          onAbandon={noop}
          heldCall={heldCall}
          onApproveHeld={handlers.onApprove ?? noop}
          onRequestChanges={handlers.onRequestChanges ?? noop}
        />
      </TooltipProvider>,
    )
  }

  it("replaces the generic pair with the tool-aware review block", async () => {
    const onApprove = vi.fn()
    renderHeld(outlineCall(), { onApprove })
    expect(screen.getByTestId("held-review")).toHaveAttribute("data-held-case", "outline")
    // The generic labels are gone: one decision, one wording.
    expect(screen.queryByRole("button", { name: "Continue" })).not.toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Feedback…" })).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole("button", { name: "Approve outline" }))
    expect(onApprove).toHaveBeenCalledTimes(1)
  })

  it("keeps Abandon reachable but out of the primary path", async () => {
    const onAbandon = vi.fn()
    render(
      <TooltipProvider>
        <SessionControls
          session={heldSession(282)}
          busy={false}
          onPause={noop}
          onStop={noop}
          onContinue={noop}
          onFeedback={noop}
          onAbandon={onAbandon}
          heldCall={outlineCall()}
          onApproveHeld={noop}
          onRequestChanges={noop}
        />
      </TooltipProvider>,
    )
    // It lives in the overflow menu, behind the confirmation every destructive action needs.
    await userEvent.click(screen.getByRole("button", { name: "More" }))
    await userEvent.click(await screen.findByRole("menuitem", { name: "Abandon" }))
    await userEvent.click(screen.getByRole("button", { name: "Abandon session" }))
    expect(onAbandon).toHaveBeenCalledTimes(1)
  })

  it("degrades to the generic controls when the held call cannot be resolved", () => {
    // A session held on a call outside the snapshot window: no review block (there is nothing to
    // show), and the status-driven pair still lets the author approve or reject it.
    renderHeld(undefined)
    expect(screen.queryByTestId("held-review")).not.toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Continue" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Feedback…" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "More" })).toBeInTheDocument()
  })
})

describe("AgentPanel — the mutation payloads end to end", () => {
  beforeEach(() => {
    resumeMutate.mockClear()
  })

  function renderPanel(session: SessionDto, calls: ToolCallDto[]) {
    hooks.session = session
    hooks.snapshot = {
      messages: [
        {
          message: { id: 124, seq: 7, role: "assistant", kind: "assistant", blocks: [], stop_reason: null, usage: null, created_at: "2026-01-01T00:00:00Z" },
          tool_calls: calls,
        },
      ],
      tool_calls: calls,
    }
    return render(
      <TooltipProvider>
        <AgentPanel bookId={3} chapterId={2} />
      </TooltipProvider>,
    )
  }

  it("approves a held outline with NO feedback", async () => {
    renderPanel(heldSession(282), [outlineCall()])
    await userEvent.click(screen.getByRole("button", { name: "Approve outline" }))
    // The exact payload: `{ sessionId }` and nothing else. A `feedback` key here — even an empty
    // one — would take the *reject* path on the server (`resume_inner` filters on `trim`).
    expect(resumeMutate).toHaveBeenCalledTimes(1)
    expect(resumeMutate.mock.calls[0][0]).toEqual({ sessionId: 9 })
  })

  it("rejects a held outline with exactly the typed feedback", async () => {
    renderPanel(heldSession(282), [outlineCall()])
    await userEvent.click(screen.getByRole("button", { name: "Request changes" }))
    await userEvent.type(
      screen.getByRole("textbox", { name: "What should change?" }),
      "  进城那段要更狼狈一点。 ",
    )
    await userEvent.click(screen.getByRole("button", { name: "Send and continue" }))
    expect(resumeMutate).toHaveBeenCalledTimes(1)
    expect(resumeMutate.mock.calls[0][0]).toEqual({ sessionId: 9, feedback: "进城那段要更狼狈一点。" })
  })

  it("shows the review block for a held auto-paused tool and approves it", async () => {
    const call = outlineCall({ id: 300, name: "insert_text", input: { text: "雨点敲在船篷上。" } })
    renderPanel(heldSession(300), [call])
    expect(screen.getByTestId("held-review")).toHaveAttribute("data-held-case", "generic")
    await userEvent.click(screen.getByRole("button", { name: "Approve and continue" }))
    expect(resumeMutate.mock.calls[0][0]).toEqual({ sessionId: 9 })
  })

  it("degrades gracefully when the held call is not in the snapshot", () => {
    // The author's session could be held on a call older than the snapshot window, or the snapshot
    // could still be loading. The panel must not crash and must keep a way forward.
    renderPanel(heldSession(999), [outlineCall()])
    expect(screen.queryByTestId("held-review")).not.toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Continue" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Feedback…" })).toBeInTheDocument()
  })

  it("disables the composer while the review block owns the decision", () => {
    renderPanel(heldSession(282), [outlineCall()])
    const textarea = screen.getByRole("textbox", { name: "Message" })
    expect(textarea).toBeDisabled()
    expect(screen.getByText("Decide on the review above to continue.")).toBeInTheDocument()
  })

  it("leaves the composer in feedback mode when the review block cannot be shown", () => {
    // Nothing was resolved, so the generic pair above is still the way to reject the call — the
    // composer keeps its own feedback path rather than pointing at a block that is not there.
    renderPanel(heldSession(999), [outlineCall()])
    expect(screen.getByText("Feedback & continue")).toBeInTheDocument()
  })
})
