/**
 * Scrollback in the activity stream (spec/11 §5.3, D-161): older messages load from the top while
 * a cursor remains, prepend in order without duplicates, keep the stream usable through loading
 * and failure, and disappear when the session changes.
 *
 * `ActivityStream` owns the scrollback so every surface that reuses it (agent panel, conception
 * studio, research task page) inherits the behavior. jsdom has no layout, so the virtualizer is
 * replaced by one that renders every row; the scroll container's metrics are defined by hand,
 * exactly the input the near-top trigger reads.
 */
import { act, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { beforeEach, describe, expect, it, vi } from "vitest"

import type { MessagePageDto } from "@/api/gen/MessagePageDto"
import type { MessageWithCallsDto } from "@/api/gen/MessageWithCallsDto"
import type { SessionDto } from "@/api/gen/SessionDto"

/** The messages endpoint, shared by the real infinite query under test. */
const getMessages = vi.hoisted(() => vi.fn())
/** `useErrorToast`'s report function: scrollback must report failures, not swallow them. */
const reportError = vi.hoisted(() => vi.fn())

vi.mock("@/api/endpoints/sessions", () => ({
  getMessages: (...args: unknown[]) => getMessages(...args),
}))

vi.mock("@/components/common/ErrorToast", () => ({ useErrorToast: () => reportError }))

// Render every row: jsdom has no viewport, and every assertion here is about row content/order.
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

import { ActivityStream } from "@/features/agent/ActivityStream"

/** A minimal session. */
function session(overrides: Partial<SessionDto> = {}): SessionDto {
  return {
    id: 7,
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

/** One persisted message; the id doubles as its seq so the cursor assertions read naturally. */
function message(id: number): MessageWithCallsDto {
  return {
    message: {
      id,
      seq: id,
      role: "assistant",
      kind: "assistant",
      blocks: [{ type: "text", text: `message ${id}` }],
      stop_reason: null,
      usage: null,
      created_at: "2026-01-01T00:00:00Z",
    },
    tool_calls: [],
  }
}

/** A contiguous run of messages, oldest first. */
function run(from: number, to: number): MessageWithCallsDto[] {
  return Array.from({ length: to - from + 1 }, (_, index) => message(from + index))
}

/** The stream's scroll container with a 1000px content range in a 400px viewport. */
function scrollPane(): HTMLElement {
  const pane = screen.getByTestId("stream")
  Object.defineProperty(pane, "scrollHeight", { value: 1000, configurable: true })
  Object.defineProperty(pane, "clientHeight", { value: 400, configurable: true })
  return pane
}

function renderStream({
  session: value = session(),
  messages,
  nextCursor,
}: {
  session?: SessionDto
  messages: MessageWithCallsDto[]
  nextCursor?: string | null
}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return {
    client,
    ...render(
      <QueryClientProvider client={client}>
        <ActivityStream
          session={value}
          messages={messages}
          inbox={[]}
          nextCursor={nextCursor}
        />
      </QueryClientProvider>,
    ),
  }
}

/** The rendered message text in stream order. */
function messageOrder(): string[] {
  return screen.getAllByText(/^message \d+$/).map((element) => element.textContent ?? "")
}

beforeEach(() => {
  getMessages.mockReset()
  reportError.mockReset()
})

describe("ActivityStream scrollback", () => {
  it("shows the top button while the snapshot has a cursor", () => {
    renderStream({ messages: [message(101), message(102)], nextCursor: "101" })

    expect(screen.getByRole("button", { name: "Load older messages" })).toBeInTheDocument()
    expect(getMessages).not.toHaveBeenCalled()
  })

  it("loads the page at the snapshot cursor and prepends it in order", async () => {
    getMessages.mockResolvedValueOnce({
      items: [message(98), message(99), message(100)],
      next_cursor: "98",
    })
    renderStream({ messages: [message(101), message(102)], nextCursor: "101" })

    await userEvent.click(screen.getByRole("button", { name: "Load older messages" }))

    await waitFor(() => expect(getMessages).toHaveBeenCalledWith(7, "101"))
    await waitFor(() => expect(screen.getByText("message 98")).toBeInTheDocument())
    expect(messageOrder()).toEqual([
      "message 98",
      "message 99",
      "message 100",
      "message 101",
      "message 102",
    ])
    // The page returned a cursor of its own, so the button stays for the next page.
    expect(screen.getByRole("button", { name: "Load older messages" })).toBeInTheDocument()
  })

  it("keeps the reading position when a page is prepended", async () => {
    getMessages.mockResolvedValueOnce({
      items: [message(98), message(99), message(100)],
      next_cursor: null,
    })
    renderStream({ messages: [message(101), message(102)], nextCursor: "101" })
    const pane = scrollPane()

    await userEvent.click(screen.getByRole("button", { name: "Load older messages" }))
    await waitFor(() => expect(screen.getByText("message 98")).toBeInTheDocument())

    // The three new rows are 120px each in the mocked virtualizer, so the offset moves by 360px:
    // the content at the top of the viewport stays there instead of jumping to the new page.
    expect(pane.scrollTop).toBe(360)
  })

  it("drops duplicate message ids when a page overlaps the snapshot", async () => {
    getMessages.mockResolvedValueOnce({
      items: [message(100), message(101)],
      next_cursor: null,
    })
    renderStream({ messages: [message(101), message(102)], nextCursor: "101" })

    await userEvent.click(screen.getByRole("button", { name: "Load older messages" }))

    await waitFor(() => expect(screen.getByText("message 100")).toBeInTheDocument())
    expect(messageOrder()).toEqual(["message 100", "message 101", "message 102"])
    expect(screen.getAllByText("message 101")).toHaveLength(1)
    // No cursor is left: the quiet start marker replaces the button.
    expect(screen.getByText("Start of the conversation")).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Load older messages" })).toBeNull()
  })

  it("shows the start marker when there is no cursor at all", () => {
    renderStream({ messages: [message(1)], nextCursor: null })

    expect(screen.getByText("Start of the conversation")).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Load older messages" })).toBeNull()
    expect(getMessages).not.toHaveBeenCalled()
  })

  it("shows the loading row while the older page is in flight", async () => {
    let resolvePage: ((page: MessagePageDto) => void) | undefined
    getMessages.mockReturnValueOnce(
      new Promise<MessagePageDto>((resolve) => {
        resolvePage = resolve
      }),
    )
    renderStream({ messages: [message(101)], nextCursor: "101" })

    await userEvent.click(screen.getByRole("button", { name: "Load older messages" }))
    expect(screen.getByText("Loading older messages…")).toBeInTheDocument()

    await act(async () => {
      resolvePage?.({ items: [message(99)], next_cursor: null })
    })
    await waitFor(() => expect(screen.getByText("message 99")).toBeInTheDocument())
    expect(screen.queryByText("Loading older messages…")).toBeNull()
  })

  it("reports a failed page and retries from the inline row", async () => {
    const failure = new Error("network")
    getMessages.mockRejectedValueOnce(failure)
    renderStream({ messages: [message(101)], nextCursor: "101" })

    await userEvent.click(screen.getByRole("button", { name: "Load older messages" }))

    await waitFor(() => expect(screen.getByText("Couldn't load older messages.")).toBeInTheDocument())
    expect(reportError).toHaveBeenCalledWith(failure)

    getMessages.mockResolvedValueOnce({ items: [message(99)], next_cursor: null })
    await userEvent.click(screen.getByRole("button", { name: "Retry" }))
    await waitFor(() => expect(screen.getByText("message 99")).toBeInTheDocument())
    expect(screen.queryByText("Couldn't load older messages.")).toBeNull()
  })

  it("loads the next page when the author scrolls within one row of the top", async () => {
    getMessages.mockResolvedValueOnce({ items: [message(99)], next_cursor: null })
    renderStream({ messages: [message(101), message(102)], nextCursor: "101" })
    const pane = scrollPane()

    act(() => {
      pane.scrollTop = 20
      pane.dispatchEvent(new Event("scroll"))
    })

    await waitFor(() => expect(getMessages).toHaveBeenCalledWith(7, "101"))
    await waitFor(() => expect(screen.getByText("message 99")).toBeInTheDocument())
  })

  it("pages a full snapshot window back to the kickoff when the server leaves the cursor empty", async () => {
    // The snapshot holds the last 200 messages (51..250); the page before it is the kickoff run.
    getMessages.mockResolvedValueOnce({ items: run(1, 50), next_cursor: null })
    renderStream({ messages: run(51, 250) })

    // The snapshot's `next_cursor` is null, so the cursor is derived from the oldest message's
    // seq (51); the messages endpoint accepts it as "everything older".
    await userEvent.click(screen.getByRole("button", { name: "Load older messages" }))
    await waitFor(() => expect(getMessages).toHaveBeenCalledWith(7, "51"))
    await waitFor(() => expect(screen.getByText("message 1")).toBeInTheDocument())
    expect(messageOrder()).toHaveLength(250)
    expect(messageOrder()[0]).toBe("message 1")
    expect(screen.getByText("Start of the conversation")).toBeInTheDocument()
  })

  it("clears the loaded pages when the session switches", async () => {
    getMessages.mockResolvedValueOnce({ items: [message(99)], next_cursor: null })
    const { client, rerender } = renderStream({ messages: [message(101)], nextCursor: "101" })

    await userEvent.click(screen.getByRole("button", { name: "Load older messages" }))
    await waitFor(() => expect(screen.getByText("message 99")).toBeInTheDocument())

    rerender(
      <QueryClientProvider client={client}>
        <ActivityStream
          session={session({ id: 8 })}
          messages={[message(201)]}
          inbox={[]}
          nextCursor={null}
        />
      </QueryClientProvider>,
    )

    await waitFor(() => expect(screen.queryByText("message 99")).toBeNull())
    expect(screen.getByText("message 201")).toBeInTheDocument()
    expect(screen.getByText("Start of the conversation")).toBeInTheDocument()
  })
})
