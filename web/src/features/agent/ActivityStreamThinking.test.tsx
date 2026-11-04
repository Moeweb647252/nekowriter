/**
 * Literal `<thinking>` wrappers inside a text block (provider quirk).
 *
 * Some providers return reasoning inside the text block with literal tags instead of the
 * structured `reasoning` block. The stream splits the text at render time and sends each segment
 * through the existing reasoning details element; the stored blocks stay untouched.
 */
import { act, render, screen } from "@testing-library/react"
import { beforeEach, describe, expect, it, vi } from "vitest"

import type { MessageWithCallsDto } from "@/api/gen/MessageWithCallsDto"
import type { SessionDto } from "@/api/gen/SessionDto"

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

import {
  ActivityStream,
  splitThinkingSegments,
} from "@/features/agent/ActivityStream"
import { useLiveStore } from "@/stores/live"

function session(): SessionDto {
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

function textMessage(text: string, role: "user" | "assistant" = "assistant"): MessageWithCallsDto {
  return {
    message: {
      id: 1,
      seq: 1,
      role,
      kind: role,
      blocks: [{ type: "text", text }],
      stop_reason: null,
      usage: null,
      created_at: "2026-01-01T00:00:00Z",
    },
    tool_calls: [],
  }
}

function renderStream(messages: MessageWithCallsDto[]) {
  return render(
    <ActivityStream
      session={session()}
      messages={messages}
      inbox={[]}
      paneName="conversation"
    />,
  )
}

/** The text content of the stream, with the details bodies included. */
function streamText(): string {
  return screen.getByTestId("stream").textContent ?? ""
}

beforeEach(() => {
  useLiveStore.setState({ streams: {}, previews: {}, connection: "open" })
})

describe("literal thinking tags", () => {
  it("returns a tag-free block unchanged", () => {
    expect(splitThinkingSegments("plain answer")).toEqual([
      { kind: "text", text: "plain answer" },
    ])
  })

  it("renders the author's literal tags verbatim", () => {
    renderStream([textMessage("<thinking>a</thinking>b", "user")])

    expect(screen.getByText("<thinking>a</thinking>b")).toBeInTheDocument()
    expect(document.querySelector("details")).toBeNull()
    expect(screen.queryByText("Thinking…")).toBeNull()
  })

  it("renders a closed segment as reasoning and leaves the rest as text", () => {
    renderStream([textMessage("before <thinking>why</thinking> after")])

    expect(screen.getByText("before")).toBeInTheDocument()
    expect(screen.getByText("after")).toBeInTheDocument()
    expect(screen.getByText("Thinking…")).toBeInTheDocument()
    expect(screen.getByText("why")).toBeInTheDocument()
    expect(streamText()).not.toContain("<thinking>")
    expect(streamText()).not.toContain("</thinking>")
  })

  it("renders several interleaved segments and drops every tag", () => {
    renderStream([
      textMessage("a <thinking>b</thinking> c <thinking>d</thinking> e"),
    ])

    expect(screen.getAllByText("Thinking…")).toHaveLength(2)
    expect(screen.getByText("a")).toBeInTheDocument()
    expect(screen.getByText("c")).toBeInTheDocument()
    expect(screen.getByText("e")).toBeInTheDocument()
    expect(streamText()).not.toContain("<thinking>")
  })

  it("holds a partially streamed opening tag back instead of showing it as text", () => {
    expect(splitThinkingSegments("prefix <think", true)).toEqual([
      { kind: "text", text: "prefix " },
    ])
    expect(splitThinkingSegments("prefix <think")).toEqual([
      { kind: "text", text: "prefix <think" },
    ])

    act(() => {
      useLiveStore.getState().applyDelta(7, 3, "text", 0, "prefix <think")
    })
    renderStream([])

    expect(streamText()).not.toContain("<think")
    expect(screen.getByText("prefix")).toBeInTheDocument()
  })

  it("treats an unterminated streaming open tag as reasoning", () => {
    act(() => {
      useLiveStore.getState().applyDelta(7, 3, "text", 0, "prefix <thinking>partial")
    })
    renderStream([])

    expect(screen.getByText("prefix")).toBeInTheDocument()
    expect(screen.getByText("partial")).toBeInTheDocument()
    expect(screen.getByText("Thinking…")).toBeInTheDocument()
    expect(streamText()).not.toContain("<thinking>")
  })

  it("streams text normally after a close tag", () => {
    act(() => {
      useLiveStore
        .getState()
        .applyDelta(7, 3, "text", 0, "<thinking>plan</thinking>answer")
    })
    renderStream([])

    expect(screen.getByText("plan")).toBeInTheDocument()
    expect(screen.getByText("answer")).toBeInTheDocument()
    expect(streamText()).not.toContain("</thinking>")
  })
})
