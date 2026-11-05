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
import { keys } from "@/api/keys"

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
    // The gap anchor restores a row's offset; every mocked row is one 120px estimate tall.
    getOffsetForIndex: (index: number) => [index * 120, "start"] as const,
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
    context_window: 100000,
    status: "running",
    phase: "planning",
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
    // A non-zero start: `scrollTop = added` would also pass from 0, but not from mid-history.
    pane.scrollTop = 500

    await userEvent.click(screen.getByRole("button", { name: "Load older messages" }))
    await waitFor(() => expect(screen.getByText("message 98")).toBeInTheDocument())

    // The three new rows are 120px each in the mocked virtualizer, so the offset moves by 360px:
    // the content at the top of the viewport stays there instead of jumping to the new page.
    expect(pane.scrollTop).toBe(860)
  })

  it("orders a second older page before the first when prepending", async () => {
    getMessages
      .mockResolvedValueOnce({ items: run(51, 100), next_cursor: "51" })
      .mockResolvedValueOnce({ items: run(1, 50), next_cursor: null })
    renderStream({ messages: run(101, 300), nextCursor: "101" })

    await userEvent.click(screen.getByRole("button", { name: "Load older messages" }))
    await waitFor(() => expect(screen.getByText("message 51")).toBeInTheDocument())
    await userEvent.click(screen.getByRole("button", { name: "Load older messages" }))
    await waitFor(() => expect(screen.getByText("message 1")).toBeInTheDocument())

    expect(messageOrder()).toHaveLength(300)
    expect(messageOrder().slice(0, 3)).toEqual(["message 1", "message 2", "message 3"])
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

  it("pages a full snapshot window back to the kickoff", async () => {
    // The snapshot holds the last 200 messages (51..250) and reports the cursor 51; the page
    // before it is the kickoff run.
    getMessages.mockResolvedValueOnce({ items: run(1, 50), next_cursor: null })
    renderStream({ messages: run(51, 250), nextCursor: "51" })

    await userEvent.click(screen.getByRole("button", { name: "Load older messages" }))
    await waitFor(() => expect(getMessages).toHaveBeenCalledWith(7, "51"))
    await waitFor(() => expect(screen.getByText("message 1")).toBeInTheDocument())
    expect(messageOrder()).toHaveLength(250)
    expect(messageOrder()[0]).toBe("message 1")
    expect(screen.getByText("Start of the conversation")).toBeInTheDocument()
  })

  it("offers no scrollback for a complete 200-message history", async () => {
    renderStream({ messages: run(1, 200), nextCursor: null })

    expect(screen.queryByRole("button", { name: "Load older messages" })).toBeNull()
    expect(screen.getByText("Start of the conversation")).toBeInTheDocument()

    // Scrolling to the top must not invent a request: there is nothing older.
    const pane = scrollPane()
    act(() => {
      pane.scrollTop = 0
      pane.dispatchEvent(new Event("scroll"))
    })
    await new Promise((resolve) => setTimeout(resolve, 20))
    expect(getMessages).not.toHaveBeenCalled()
  })

  it("keeps the oldest message reachable when a live message slides a full window", async () => {
    // A complete 200-message history has no cursor and loads nothing.
    const { client, rerender } = renderStream({ messages: run(1, 200), nextCursor: null })
    expect(getMessages).not.toHaveBeenCalled()

    // The 201st live message slides the window to 2..201; the server now reports cursor 2.
    getMessages.mockResolvedValueOnce({ items: [message(1)], next_cursor: null })
    rerender(
      <QueryClientProvider client={client}>
        <ActivityStream
          session={session()}
          messages={run(2, 201)}
          inbox={[]}
          nextCursor="2"
        />
      </QueryClientProvider>,
    )

    await userEvent.click(screen.getByRole("button", { name: "Load older messages" }))
    await waitFor(() => expect(getMessages).toHaveBeenCalledWith(7, "2"))
    await waitFor(() => expect(screen.getByText("message 1")).toBeInTheDocument())
  })

  it("re-fetches the window gap when a live message slides the snapshot after paging", async () => {
    getMessages
      // The page loaded before the slide: it ends at seq 100, below the old cursor 101.
      .mockResolvedValueOnce({ items: run(1, 100), next_cursor: null })
      // The repair request from the new cursor 102 returns the newest 100 messages below it
      // (seq < 102): the slid-out boundary message 101 is last.
      .mockResolvedValueOnce({ items: run(2, 101), next_cursor: null })
    const { client, rerender } = renderStream({ messages: run(101, 300), nextCursor: "101" })

    await userEvent.click(screen.getByRole("button", { name: "Load older messages" }))
    await waitFor(() => expect(screen.getByText("message 1")).toBeInTheDocument())

    rerender(
      <QueryClientProvider client={client}>
        <ActivityStream
          session={session()}
          messages={run(102, 301)}
          inbox={[]}
          nextCursor="102"
        />
      </QueryClientProvider>,
    )

    await waitFor(() => expect(screen.getByText("message 101")).toBeInTheDocument())
    // The repair must start at the NEW cursor: the old anchor 101 would be filtered by
    // `seq < cursor` to an empty page and leave the boundary missing (F8).
    expect(getMessages).toHaveBeenCalledWith(7, "102", 500)
    expect(getMessages).not.toHaveBeenCalledWith(7, "101", 500)
    expect(messageOrder()).toHaveLength(301)
    expect(messageOrder().map((text) => Number(text.slice("message ".length)))).toEqual(
      Array.from({ length: 301 }, (_, index) => index + 1),
    )
  })

  it("keeps the reading position when the repaired gap rows arrive", async () => {
    getMessages
      .mockResolvedValueOnce({ items: run(1, 100), next_cursor: null })
      .mockResolvedValueOnce({ items: run(2, 101), next_cursor: null })
    const { client, rerender } = renderStream({ messages: run(101, 300), nextCursor: "101" })
    const pane = scrollPane()

    await userEvent.click(screen.getByRole("button", { name: "Load older messages" }))
    await waitFor(() => expect(screen.getByText("message 1")).toBeInTheDocument())
    pane.scrollTop = 20000

    rerender(
      <QueryClientProvider client={client}>
        <ActivityStream
          session={session()}
          messages={run(102, 301)}
          inbox={[]}
          nextCursor="102"
        />
      </QueryClientProvider>,
    )

    await waitFor(() => expect(screen.getByText("message 101")).toBeInTheDocument())
    // One 120px row is inserted above the anchored snapshot row at offset 12120, so the viewport
    // moves with it: 20000 + 120, not 20000.
    expect(pane.scrollTop).toBe(20120)
  })

  it("does not leak a previous session's gap rows into another session", async () => {
    getMessages
      .mockResolvedValueOnce({ items: run(1, 100), next_cursor: null })
      .mockResolvedValueOnce({ items: run(2, 101), next_cursor: null })
    const { client, rerender } = renderStream({ messages: run(101, 300), nextCursor: "101" })

    await userEvent.click(screen.getByRole("button", { name: "Load older messages" }))
    await waitFor(() => expect(screen.getByText("message 1")).toBeInTheDocument())
    rerender(
      <QueryClientProvider client={client}>
        <ActivityStream
          session={session()}
          messages={run(102, 301)}
          inbox={[]}
          nextCursor="102"
        />
      </QueryClientProvider>,
    )
    await waitFor(() => expect(screen.getByText("message 101")).toBeInTheDocument())

    // Session 8 already has its own cached page chain; switching to it must not carry session
    // 7's gap row (101) through `placeholderData`.
    client.setQueryData([...keys.sessions.detail(8), "messages"], {
      pages: [{ items: run(401, 500), next_cursor: null }],
      pageParams: ["501"],
    })
    rerender(
      <QueryClientProvider client={client}>
        <ActivityStream
          session={session({ id: 8 })}
          messages={run(501, 600)}
          inbox={[]}
          nextCursor="501"
        />
      </QueryClientProvider>,
    )

    await waitFor(() => expect(screen.getByText("message 501")).toBeInTheDocument())
    expect(screen.getByText("message 401")).toBeInTheDocument()
    expect(screen.queryByText("message 101")).toBeNull()
    expect(screen.queryByText("message 1")).toBeNull()
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
