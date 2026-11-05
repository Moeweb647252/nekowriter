/**
 * The activity stream's stick-to-bottom rule (PROJECT §8.6, spec/11 §5.3): "auto-scroll unless the
 * author scrolled up".
 *
 * The bug this covers: the follow effect depended on `rows.length` only. While the provider streams
 * a reply the live row keeps growing and no row is added, so the view stopped following after the
 * first chunk and the author had to scroll down by hand for the rest of the message. The stream now
 * also follows when the **tail's content** changes, and it stops following as soon as the author
 * scrolls up — and re-pins when they scroll back to the bottom.
 *
 * jsdom performs no layout, so the scroll container's `scrollHeight`/`clientHeight` are defined on
 * the element and the scroll events are dispatched by hand: that is exactly the input the component
 * reads.
 */
import { act, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import type { MessageWithCallsDto } from "@/api/gen/MessageWithCallsDto"
import type { SessionDto } from "@/api/gen/SessionDto"

/** Every call the component makes to the virtualizer's scroll API, in order. */
const scrollCalls = vi.hoisted(() => ({ scrollToEnd: [] as unknown[], measure: 0 }))

/**
 * A virtualizer stand-in: it renders every row (there is no viewport in jsdom) and records the
 * scroll commands. `getTotalSize` is only used for the sizer's height, which no assertion reads.
 */
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
    measure: () => {
      scrollCalls.measure += 1
    },
    scrollToEnd: (options?: unknown) => scrollCalls.scrollToEnd.push(options),
  }),
}))

// Scrollback has its own file (`ActivityStreamScrollback.test.tsx`); these tests only cover the
// stick-to-bottom rule, so the infinite query is kept inert.
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

import { ActivityStream } from "@/features/agent/ActivityStream"
import { useLiveStore } from "@/stores/live"

/** A minimal running session. */
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

/** One persisted message. */
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

/**
 * Gives the stream's scroll container a scroll range (1000px of content in a 400px viewport, so
 * the maximum `scrollTop` is 600) and returns it.
 */
function scrollPane(): HTMLElement {
  const pane = screen.getByTestId("stream")
  Object.defineProperty(pane, "scrollHeight", { value: 1000, configurable: true })
  Object.defineProperty(pane, "clientHeight", { value: 400, configurable: true })
  return pane
}

/** Simulates one scroll event after moving the element to `top`. */
function scrollTo(pane: HTMLElement, top: number): void {
  act(() => {
    pane.scrollTop = top
    pane.dispatchEvent(new Event("scroll"))
  })
}

/** Pushes one streamed chunk into the live store, as the SSE dispatcher does. */
function stream(kind: "text" | "reasoning", delta: string): void {
  act(() => {
    useLiveStore.getState().applyDelta(7, 3, kind, 0, delta)
  })
}

describe("ActivityStream stick-to-bottom", () => {
  beforeEach(() => {
    scrollCalls.scrollToEnd.length = 0
    scrollCalls.measure = 0
    useLiveStore.setState({ streams: {}, previews: {}, connection: "open" })
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  it("follows the growing live row while the author stays at the bottom", async () => {
    render(
      <ActivityStream
        session={session()}
        messages={[message(1)]}
        inbox={[]}
        pendingPreview={undefined}
        paneName="conversation"
      />,
    )
    scrollPane()
    // The stream starts pinned even before any scroll event arrives.
    scrollCalls.scrollToEnd.length = 0

    // Chunk 1: a new row, so the row count changes.
    stream("text", "Hel")
    await waitFor(() => expect(scrollCalls.scrollToEnd.length).toBeGreaterThan(0))
    const afterFirst = scrollCalls.scrollToEnd.length

    // Chunk 2: the same row, one more character. This is the case that used to be missed: the
    // row count is unchanged, only the tail's text grew.
    stream("text", "lo")
    await waitFor(() => expect(scrollCalls.scrollToEnd.length).toBeGreaterThan(afterFirst))
  })

  it("stops following while the author is scrolled up and re-pins at the bottom", async () => {
    render(
      <ActivityStream session={session()} messages={[message(1)]} inbox={[]} paneName="conversation" />,
    )
    const pane = scrollPane()

    // The author scrolls to the top of the history.
    scrollTo(pane, 0)
    scrollCalls.scrollToEnd.length = 0

    stream("text", "streaming text")
    // Give the animation frame a chance to run; nothing may be scrolled.
    await new Promise((resolve) => setTimeout(resolve, 50))
    expect(scrollCalls.scrollToEnd).toHaveLength(0)

    // Back at the bottom: the next content change follows again.
    scrollTo(pane, 600)
    stream("text", " more")
    await waitFor(() => expect(scrollCalls.scrollToEnd.length).toBeGreaterThan(0))
  })

  it("follows a tool result settling in the last row", async () => {
    // The message has no blocks of its own: its height comes from the tool card.
    const call = {
      id: 9,
      message_id: 1,
      seq: 1,
      call_index: 0,
      name: "echo",
      input: { text: "hi" },
      input_text: '{"text":"hi"}',
      status: "executing" as const,
      is_error: false,
      result_text: null,
      appendices: [],
      ui: null,
      sealed: false,
      completes: false,
      created_at: "2026-01-01T00:00:00Z",
      finished_at: null,
    }
    const { rerender } = render(
      <ActivityStream
        session={session()}
        messages={[{ message: message(1).message, tool_calls: [call] }]}
        inbox={[]}
        paneName="conversation"
      />,
    )
    scrollPane()
    scrollCalls.scrollToEnd.length = 0

    // The result arrives in place: same rows, same message, only the call's state changed.
    rerender(
      <ActivityStream
        session={session()}
        messages={[
          {
            message: message(1).message,
            tool_calls: [{ ...call, status: "done", result_text: "hi", sealed: true }],
          },
        ]}
        inbox={[]}
        paneName="conversation"
      />,
    )
    await waitFor(() => expect(scrollCalls.scrollToEnd.length).toBeGreaterThan(0))
  })

  it("renders a queued author row and removes it when the snapshot consumes it", () => {
    const { rerender } = render(
      <ActivityStream
        session={session()}
        messages={[]}
        inbox={[{ id: 11, kind: "user_message", content: "steer" }]}
        paneName="conversation"
      />,
    )
    expect(screen.getByText("steer")).toBeInTheDocument()
    expect(screen.getByText("Queued · not yet seen by the model")).toBeInTheDocument()

    const persisted = {
      message: {
        id: 9,
        seq: 9,
        role: "user" as const,
        kind: "user" as const,
        blocks: [{ type: "text" as const, text: "steer" }],
        stop_reason: null,
        usage: null,
        created_at: "2026-01-01T00:00:00Z",
      },
      tool_calls: [],
    }
    rerender(
      <ActivityStream
        session={session()}
        messages={[persisted]}
        inbox={[]}
        paneName="conversation"
      />,
    )
    expect(screen.getAllByText("steer")).toHaveLength(1)
    expect(screen.queryByText("Queued · not yet seen by the model")).toBeNull()
  })

  it("names its pane so the caller can address the scroll container", () => {
    render(
      <ActivityStream session={session()} messages={[message(1)]} inbox={[]} paneName="conversation" />,
    )
    expect(screen.getByTestId("stream").getAttribute("data-scroll-pane")).toBe("conversation")
  })
})
