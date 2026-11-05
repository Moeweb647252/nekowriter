/**
 * The author's steering feedback as its own transcript row (spec/03 §5.3, spec/11 §5.3).
 *
 * Feedback is delivered as an `inbox` appendix on a tool call, but the tool card is collapsed, so
 * the author's words used to be invisible. `ActivityStream` now lifts exactly that appendix kind
 * into an author row, de-duplicated by `inbox_id`, while every other kind stays inside the card.
 */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeEach, describe, expect, it, vi } from "vitest"

import type { InboxItemDto } from "@/api/gen/InboxItemDto"
import type { MessageWithCallsDto } from "@/api/gen/MessageWithCallsDto"
import type { SessionDto } from "@/api/gen/SessionDto"
import type { ToolCallDto } from "@/api/gen/ToolCallDto"

vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({ count }: { count: number }) => ({
    getTotalSize: () => count * 120,
    getVirtualItems: () =>
      Array.from({ length: count }, (_, index) => ({
        index,
        key: index,
        start: index * 120,
        size: 120,
        end: (index + 1) * 120,
        lane: 0,
      })),
    measureElement: () => undefined,
    measure: () => undefined,
    scrollToEnd: () => undefined,
  }),
}))
vi.mock("@/api/hooks/sessions", () => ({
  useOlderMessages: () => ({
    data: undefined,
    hasNextPage: false,
    isFetchingNextPage: false,
    isFetchNextPageError: false,
    error: null,
    fetchNextPage: vi.fn(),
  }),
}))
vi.mock("@/components/common/ErrorToast", () => ({ useErrorToast: () => vi.fn() }))

import { ActivityStream, buildRows } from "@/features/agent/ActivityStream"
import { useLiveStore } from "@/stores/live"

function session(): SessionDto {
  return {
    id: 7,
    book_id: 1,
    role: "researcher",
    kind: "research",
    chapter_id: null,
    track_id: null,
    research_task_id: 1,
    chapter_number: null,
    research_topic: null,
    model_label: "Demo model",
    context_window: 100000,
    status: "running",
    phase: "none",
    granularity: null,
    auto_pause: false,
    pause_after_outline: false,
    pause_requested: false,
    pause_after_step: false,
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
  }
}

const INBOX_TEXT = "【用户留言】\n查查模型圈对这些娘化角色的设定"

function toolCall(overrides: Partial<ToolCallDto> = {}): ToolCallDto {
  return {
    id: 9,
    message_id: 1,
    seq: 1,
    call_index: 0,
    name: "web_fetch",
    input: { url: "https://example.com" },
    input_text: '{"url":"https://example.com"}',
    status: "done",
    is_error: false,
    result_text: "the page",
    appendices: [],
    ui: null,
    sealed: true,
    completes: false,
    created_at: "2026-01-01T00:00:00Z",
    finished_at: "2026-01-01T00:00:00Z",
    ...overrides,
  }
}

function messageWith(toolCalls: ToolCallDto[], id = 1): MessageWithCallsDto {
  return {
    message: {
      id,
      seq: id,
      role: "assistant",
      kind: "assistant",
      blocks: [{ type: "text", text: "working" }],
      stop_reason: null,
      usage: null,
      created_at: "2026-01-01T00:00:00Z",
    },
    tool_calls: toolCalls,
  }
}

function renderStream(messages: MessageWithCallsDto[], inbox: InboxItemDto[] = []) {
  return render(
    <ActivityStream
      session={session()}
      messages={messages}
      inbox={inbox}
      paneName="conversation"
    />,
  )
}

function cardTrigger(): HTMLElement {
  const trigger = screen
    .getAllByRole("button")
    .find((button) => button.textContent?.includes("web_fetch"))
  if (trigger === undefined) {
    throw new Error("no web_fetch card trigger")
  }
  return trigger
}

beforeEach(() => {
  useLiveStore.setState({ streams: {}, previews: {}, connection: "open" })
})

describe("inbox appendices", () => {
  it("renders the author's feedback outside the collapsed card with the exact text", async () => {
    renderStream([
      messageWith([
        toolCall({
          appendices: [
            { kind: "inbox", text: INBOX_TEXT, inbox_id: 1, inbox_kind: "user_message" },
            { kind: "effect_notice", text: "the effect notice", inbox_id: null },
          ],
        }),
      ]),
    ])

    const row = screen.getByTestId("inbox-appendix")
    expect(row).toHaveTextContent("Your message")
    expect(row.textContent).toContain(INBOX_TEXT)
    expect(row.closest('[data-slot="collapsible-content"]')).toBeNull()

    // The other appendix kind stays inside the card; expanding it reveals it there.
    const user = userEvent.setup()
    await user.click(cardTrigger())
    const effect = screen.getByText("the effect notice")
    expect(effect.closest('[data-slot="collapsible-content"]')).not.toBeNull()
    // The inbox appendix is not repeated anywhere in the card's appendices container.
    const appendices = (screen.getByText("Attached to this result").parentElement ??
      null) as HTMLElement | null
    expect(appendices).not.toBeNull()
    expect((appendices as HTMLElement).textContent).not.toContain(INBOX_TEXT)
    expect(screen.getAllByTestId("inbox-appendix")).toHaveLength(1)
  })

  it("renders a duplicated inbox appendix only once", () => {
    const call = toolCall({
      appendices: [{ kind: "inbox", text: INBOX_TEXT, inbox_id: 1 }],
    })
    const { rerender } = renderStream([messageWith([call], 1)])

    // A refresh hands the same appendix over on another message copy, as the live buffer and the
    // snapshot can both carry the call.
    rerender(
      <ActivityStream
        session={session()}
        messages={[messageWith([call], 1), messageWith([call], 2)]}
        inbox={[]}
        paneName="conversation"
      />,
    )

    expect(screen.getAllByTestId("inbox-appendix")).toHaveLength(1)
    expect(screen.getAllByText("Your message")).toHaveLength(1)
  })

  it("suppresses a queued copy of an inbox id already lifted", () => {
    renderStream(
      [
        messageWith([
          toolCall({ appendices: [{ kind: "inbox", text: INBOX_TEXT, inbox_id: 1 }] }),
        ]),
      ],
      [{ id: 1, kind: "user_message", content: "queued duplicate" }],
    )

    expect(screen.getAllByTestId("inbox-appendix")).toHaveLength(1)
    expect(screen.queryByText("queued duplicate")).toBeNull()
  })

  it("renders a queued message with a different id", () => {
    renderStream(
      [
        messageWith([
          toolCall({ appendices: [{ kind: "inbox", text: INBOX_TEXT, inbox_id: 1 }] }),
        ]),
      ],
      [{ id: 2, kind: "user_message", content: "queued after the appendix" }],
    )

    expect(screen.getAllByTestId("inbox-appendix")).toHaveLength(1)
    expect(screen.getByText("queued after the appendix")).toBeInTheDocument()
  })

  it("places the lifted row between the message and the next one", () => {
    const call = toolCall({
      appendices: [{ kind: "inbox", text: INBOX_TEXT, inbox_id: 1 }],
    })
    const rows = buildRows(
      [messageWith([call], 1), messageWith([], 2)],
      undefined,
      (key) => key,
      [],
    )

    expect(rows.map((row) => row.kind)).toEqual(["message", "inbox", "message"])
  })

  it("keeps a runtime notice inside the card", async () => {
    renderStream([
      messageWith([
        toolCall({
          appendices: [
            {
              kind: "inbox",
              text: "Granularity changed to paragraph.",
              inbox_id: 2,
              inbox_kind: "notice",
            },
          ],
        }),
      ]),
    ])

    expect(screen.queryByTestId("inbox-appendix")).toBeNull()
    const user = userEvent.setup()
    await user.click(cardTrigger())
    const notice = screen.getByText("Granularity changed to paragraph.")
    expect(notice.closest('[data-slot="collapsible-content"]')).not.toBeNull()
  })

  it("lifts a legacy inbox appendix that has no inbox_kind field", () => {
    renderStream([
      messageWith([
        toolCall({ appendices: [{ kind: "inbox", text: INBOX_TEXT, inbox_id: 3 }] }),
      ]),
    ])

    const row = screen.getByTestId("inbox-appendix")
    expect(row.textContent).toContain(INBOX_TEXT)
  })

  it("leaves a stream without inbox appendices unchanged", () => {
    renderStream([
      messageWith([
        toolCall({
          appendices: [{ kind: "effect_notice", text: "the effect notice", inbox_id: null }],
        }),
      ]),
    ])

    expect(screen.queryByTestId("inbox-appendix")).toBeNull()
    expect(screen.queryByText("Your message")).toBeNull()
  })

  it("labels a failed call with no result line as failed, not with a dangling summary", () => {
    renderStream([
      messageWith([
        toolCall({
          status: "error",
          is_error: true,
          ui: null,
          result_text: null,
        }),
      ]),
    ])

    const trigger = cardTrigger()
    expect(screen.getByText("Failed")).toBeInTheDocument()
    expect(trigger.textContent).not.toContain("—")
  })

  it("labels a failed call with the first result line instead of an empty summary", () => {
    renderStream([
      messageWith([
        toolCall({
          status: "error",
          is_error: true,
          ui: null,
          result_text: "fetch failed: timeout\nmore detail",
        }),
      ]),
    ])

    expect(screen.getByText("Failed — fetch failed: timeout")).toBeInTheDocument()
    expect(screen.queryByText("读取了")).toBeNull()
  })
})
