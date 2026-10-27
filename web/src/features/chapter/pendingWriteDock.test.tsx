/**
 * The draft pane's dock: where a held chapter write is decided (D-141).
 *
 * The author's report this answers: 「正文编写要在中间的正文草稿区进行，正文编辑时的意见提出在草稿
 * 区下方的 dock 栏里进行」. So the assertions are about the two things that would make the dock lie
 * or duplicate:
 *
 * - **the standing it reports**: a write that still applies invites an apply, while a write the
 *   draft has outrun says so and offers only "continue without it" — because
 *   `runner::apply_pending_content` drops that write, so an "apply" button would do nothing;
 * - **the payloads**: approve is `resume` with no feedback, request changes is `resume` with
 *   exactly the trimmed feedback, and a blank request is impossible (the server reads blank
 *   feedback as an approval).
 *
 * Fail-before: before D-141 the only decision surface for these calls was `HeldCallReview` in the
 * agent panel, which showed the tool's *input text* rather than the merged document, and had no
 * concept of a superseded write — so `data-standing` below had no equivalent anywhere.
 */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { describe, expect, it, vi } from "vitest"

import type { ChapterContent } from "@/api/gen/ChapterContent"
import type { ToolCallDto } from "@/api/gen/ToolCallDto"
import { PendingWriteDock } from "@/features/chapter/PendingWriteDock"
import type { PendingDraftView } from "@/features/chapter/pendingWrite"

function content(paragraphs: [string, string][]): ChapterContent {
  return {
    paragraphs: paragraphs.map(([pid, text]) => ({ pid, text })),
    next_pid: paragraphs.length + 1,
    tail_open: false,
  }
}

/** A held `insert_text` that staged the given document. */
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
    pending_content: { content: content([["p3", "雨点敲在船篷上。"]]), expected_version: 4 },
    created_at: "2026-01-01T00:00:00Z",
    finished_at: null,
    ...overrides,
  }
}

/** The merged view the pane computed, as the page would pass it in. */
function view(overrides: Partial<PendingDraftView> = {}): PendingDraftView {
  return {
    standing: "current",
    expectedVersion: 4,
    draftVersion: 4,
    paragraphs: [{ pid: "p3", text: "雨点敲在船篷上。", kind: "added" }],
    stagedWords: 8,
    confirmedWords: 0,
    ...overrides,
  }
}

describe("PendingWriteDock", () => {
  it("invites the apply when the staged write still matches the draft", async () => {
    const onApprove = vi.fn()
    render(
      <PendingWriteDock
        call={writeCall()}
        view={view()}
        busy={false}
        onApprove={onApprove}
        onRequestChanges={vi.fn()}
      />,
    )
    expect(screen.getByTestId("pending-write-dock")).toHaveAttribute("data-standing", "current")
    expect(screen.getByText("The agent wrote this. Apply it?")).toBeInTheDocument()
    await userEvent.click(screen.getByRole("button", { name: "Apply and continue" }))
    expect(onApprove).toHaveBeenCalledTimes(1)
  })

  it("says the text will not be applied when the draft has outrun it", async () => {
    const onApprove = vi.fn()
    render(
      <PendingWriteDock
        call={writeCall()}
        // The author edited the draft after the tool read it, so `apply_pending_content` discards
        // the staged content: the button must not promise an apply that cannot happen.
        view={view({ standing: "superseded", draftVersion: 6 })}
        busy={false}
        onApprove={onApprove}
        onRequestChanges={vi.fn()}
      />,
    )
    const dock = screen.getByTestId("pending-write-dock")
    expect(dock).toHaveAttribute("data-standing", "superseded")
    expect(screen.getByText("The agent's text was not applied")).toBeInTheDocument()
    // Both version numbers are named, so the explanation is checkable rather than vague.
    expect(
      screen.getByText(/the draft is now version 6, the agent read version 4/),
    ).toBeInTheDocument()
    expect(
      screen.queryByRole("button", { name: "Apply and continue" }),
    ).not.toBeInTheDocument()
    // The one thing left to do is resume, which is the same mutation: the write is dropped in the
    // transaction that finalizes the call.
    await userEvent.click(screen.getByRole("button", { name: "Continue without it" }))
    expect(onApprove).toHaveBeenCalledTimes(1)
  })

  it("sends exactly the trimmed feedback on request-changes", async () => {
    const onRequestChanges = vi.fn()
    const onApprove = vi.fn()
    render(
      <PendingWriteDock
        call={writeCall()}
        view={view()}
        busy={false}
        onApprove={onApprove}
        onRequestChanges={onRequestChanges}
      />,
    )
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole("button", { name: "Request changes" }))
    await userEvent.type(
      screen.getByRole("textbox", { name: "What should change?" }),
      "  船篷那段的雨声再细一点。  ",
    )
    await userEvent.click(screen.getByRole("button", { name: "Send and continue" }))
    expect(onRequestChanges).toHaveBeenCalledTimes(1)
    expect(onRequestChanges).toHaveBeenCalledWith("船篷那段的雨声再细一点。")
    expect(onApprove).not.toHaveBeenCalled()
  })

  it("cannot request changes with blank or whitespace-only feedback", async () => {
    const onRequestChanges = vi.fn()
    render(
      <PendingWriteDock
        call={writeCall()}
        view={view()}
        busy={false}
        onApprove={vi.fn()}
        onRequestChanges={onRequestChanges}
      />,
    )
    await userEvent.click(screen.getByRole("button", { name: "Request changes" }))
    const submit = screen.getByRole("button", { name: "Send and continue" })
    expect(submit).toBeDisabled()
    // A blank request would be filtered to `None` by the server, which then *approves* the call.
    await userEvent.type(screen.getByRole("textbox", { name: "What should change?" }), "   ")
    expect(submit).toBeDisabled()
    await userEvent.click(submit)
    expect(onRequestChanges).not.toHaveBeenCalled()
  })

  it("disables both decisions while a resume is in flight", () => {
    render(
      <PendingWriteDock
        call={writeCall()}
        view={view()}
        busy
        onApprove={vi.fn()}
        onRequestChanges={vi.fn()}
      />,
    )
    expect(screen.getByRole("button", { name: "Apply and continue" })).toBeDisabled()
    expect(screen.getByRole("button", { name: "Request changes" })).toBeDisabled()
  })

  it("names the tool through the registry, never its identifier", () => {
    render(
      <PendingWriteDock
        call={writeCall()}
        view={view()}
        busy={false}
        onApprove={vi.fn()}
        onRequestChanges={vi.fn()}
      />,
    )
    // The same summary the tool card in the stream shows (spec/11 §5.3), built from `ui`.
    expect(screen.getByText("Appended after p2")).toBeInTheDocument()
    expect(screen.queryByText(/insert_text/)).not.toBeInTheDocument()
  })
})
