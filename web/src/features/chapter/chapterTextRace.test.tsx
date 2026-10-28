/** Real-hook paragraph save/race and Enter-insert tests (R9-01, R9-02). */
import { useState } from "react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeEach, describe, expect, it, vi } from "vitest"

import type { ChapterDto } from "@/api/gen/ChapterDto"
import { useChapterDoc } from "@/stores/chapterDoc"

const endpoints = vi.hoisted(() => ({ patchContent: vi.fn(), putContentText: vi.fn(), restoreVersion: vi.fn() }))
vi.mock("@/api/endpoints/chapters", () => endpoints)
const writer = vi.hoisted(() => ({ locked: false }))
vi.mock("@/features/chapter/useWriterState", () => ({
  useWriterState: () => ({ locked: writer.locked }),
  openWriterSession: () => undefined,
  heldPids: () => [],
}))
vi.mock("@/api/hooks/sessions", () => ({
  useResumeSession: () => ({ mutate: vi.fn(), isPending: false }),
}))
vi.mock("@/components/common/ErrorToast", () => ({
  useErrorToast: () => vi.fn(),
  useWarningToasts: () => vi.fn(),
}))

import { ChapterText, PendingParagraph } from "@/features/chapter/ChapterText"

function chapter(): ChapterDto {
  return {
    id: 1,
    book_id: 1,
    track_id: 1,
    number: 1,
    title: "Chapter",
    status: "drafting",
    canon_status: "none",
    word_count: 2,
    has_outline: false,
    content_version: 1,
    content: {
      paragraphs: [
        { pid: "p1", text: "第一段。" },
        { pid: "p2", text: "第二段。" },
      ],
      next_pid: 3,
      tail_open: false,
    },
    open_sessions: [],
    guidance: "",
    outline: "",
    reference_notes: "",
    summary: "",
    official_version_id: null,
    updated_at: "2026-01-01T00:00:00Z",
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

beforeEach(() => {
  writer.locked = false
  endpoints.patchContent.mockReset()
  endpoints.putContentText.mockReset()
  useChapterDoc.getState().load(1, 1, chapter().content)
})

describe("chapter paragraph editing with real mutations", () => {
  it("saves the next paragraph with the version advanced by the previous save (R9-01)", async () => {
    const user = userEvent.setup()
    const first = deferred<{
      version: number
      ops: { op: "update"; pid: string; text: string }[]
      word_count: number
      tail_open: boolean
    }>()
    endpoints.patchContent.mockReturnValueOnce(first.promise)
    render(
      <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
        <ChapterText chapter={chapter()} bookId={1} unspaced fullTextOpen={false} onFullTextOpenChange={() => {}} />
      </QueryClientProvider>,
    )
    await user.click(screen.getByText("第一段。"))
    await user.type(screen.getByRole("textbox"), "改过一")
    await user.click(screen.getByText("第二段。"))
    await waitFor(() => expect(endpoints.patchContent).toHaveBeenCalledTimes(1))
    await act(async () => {
      first.resolve({
        version: 2,
        ops: [{ op: "update", pid: "p1", text: "第一段。改过一" }],
        word_count: 4,
        tail_open: false,
      })
    })
    await waitFor(() => expect(screen.getByRole("textbox")).not.toBeDisabled())
    await user.type(screen.getByRole("textbox"), "改过二")
    await user.tab()
    await waitFor(() => expect(endpoints.patchContent).toHaveBeenCalledTimes(2))
    expect(endpoints.patchContent.mock.calls[1]?.[1]).toEqual({
      base_version: 2,
      ops: [{ op: "update", pid: "p2", text: "第二段。改过二" }],
    })
  })

  it("refuses and offers reload when an agent changes the edited paragraph (R9-01 negative)", async () => {
    const user = userEvent.setup()
    render(
      <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
        <ChapterText chapter={chapter()} bookId={1} unspaced fullTextOpen={false} onFullTextOpenChange={() => {}} />
      </QueryClientProvider>,
    )
    await user.click(screen.getByText("第二段。"))
    await user.type(screen.getByRole("textbox"), "改过")
    act(() => {
      useChapterDoc.getState().applyOps(
        2,
        [{ op: "update", pid: "p2", text: "Agent 改过" }],
        "agent",
      )
    })
    await user.tab()
    expect(endpoints.patchContent).not.toHaveBeenCalled()
    expect(await screen.findByRole("status")).toHaveTextContent("This paragraph changed")
    await user.click(screen.getByRole("button", { name: "Reload paragraph" }))
    expect(screen.queryByRole("status")).not.toBeInTheDocument()
  })

  it("opens a focused pending paragraph and persists typed text as one insert (R10-01)", async () => {
    const user = userEvent.setup()
    const response = deferred<{
      version: number
      ops: (
        | { op: "update"; pid: string; text: string }
        | { op: "insert"; after: string; pid: string; text: string }
      )[]
      word_count: number
      tail_open: boolean
    }>()
    endpoints.patchContent.mockReturnValueOnce(response.promise)
    render(
      <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
        <ChapterText chapter={chapter()} bookId={1} unspaced fullTextOpen={false} onFullTextOpenChange={() => {}} />
      </QueryClientProvider>,
    )
    await user.click(screen.getByText("第一段。"))
    await user.type(screen.getByRole("textbox"), "改过一")
    await user.keyboard("{Enter}")
    const pending = screen.getByRole("textbox")
    expect(document.activeElement).toBe(pending)
    await user.type(pending, "新段落")
    await user.tab()
    await waitFor(() => expect(endpoints.patchContent).toHaveBeenCalledTimes(1))
    expect(endpoints.patchContent.mock.calls[0]?.[1]).toEqual({
      base_version: 1,
      ops: [
        { op: "update", pid: "p1", text: "第一段。改过一" },
        { op: "insert", after: "p1", text: "新段落" },
      ],
    })
    await act(async () => {
      response.resolve({
        version: 2,
        ops: [
          { op: "update", pid: "p1", text: "第一段。改过一" },
          { op: "insert", after: "p1", pid: "p3", text: "新段落" },
        ],
        word_count: 5,
        tail_open: false,
      })
    })
    await waitFor(() => expect(useChapterDoc.getState().paragraphs).toHaveLength(3))
  })

  it("re-syncs the full-text editor after a blur save response advances the store (R11-03)", async () => {
    const user = userEvent.setup()
    const pendingPatch = deferred<{
      version: number
      ops: { op: "update"; pid: string; text: string }[]
      word_count: number
      tail_open: boolean
    }>()
    endpoints.patchContent.mockReturnValueOnce(pendingPatch.promise)
    function Harness() {
      const [open, setOpen] = useState(false)
      return (
        <ChapterText
          chapter={chapter()}
          bookId={1}
          unspaced
          fullTextOpen={open}
          onFullTextOpenChange={setOpen}
        />
      )
    }
    render(
      <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
        <Harness />
      </QueryClientProvider>,
    )
    await user.click(screen.getByText("第一段。"))
    await user.type(screen.getByRole("textbox"), " edit")
    await user.click(screen.getByRole("button", { name: "Full-text edit" }))
    expect(await screen.findByRole("textbox")).toBeInTheDocument()
    await act(async () => {
      pendingPatch.resolve({
        version: 2,
        ops: [{ op: "update", pid: "p1", text: "第一段。 edit" }],
        word_count: 3,
        tail_open: false,
      })
    })
    expect(screen.queryByRole("status")).not.toBeInTheDocument()
    expect(screen.getByRole("textbox")).toHaveValue("第一段。 edit\n\n第二段。")
    expect(screen.getByRole("button", { name: "Save" })).toBeEnabled()
  })

  it("submits a pending paragraph only once after Ctrl+Enter and blur (R11-02)", async () => {
    const user = userEvent.setup()
    const pendingPatch = deferred<{
      version: number
      ops: { op: "insert"; after: string; pid: string; text: string }[]
      word_count: number
      tail_open: boolean
    }>()
    endpoints.patchContent.mockReturnValueOnce(pendingPatch.promise)
    render(
      <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
        <ChapterText chapter={chapter()} bookId={1} unspaced fullTextOpen={false} onFullTextOpenChange={() => {}} />
      </QueryClientProvider>,
    )
    await user.click(screen.getByText("第一段。"))
    await user.keyboard("{Enter}")
    const pending = screen.getByRole("textbox")
    await user.type(pending, "新段落")
    await user.keyboard("{Control>}{Enter}{/Control}")
    fireEvent.keyDown(pending, { key: "Enter", ctrlKey: true })
    fireEvent.blur(pending)
    expect(endpoints.patchContent).toHaveBeenCalledTimes(1)
    pendingPatch.resolve({
      version: 2,
      ops: [{ op: "insert", after: "p1", pid: "p3", text: "新段落" }],
      word_count: 4,
      tail_open: false,
    })
  })

  it("submits the pending component only once across Ctrl+Enter and blur (R11-02)", async () => {
    const user = userEvent.setup()
    const onPersist = vi.fn()
    render(<PendingParagraph onPersist={onPersist} onDismiss={vi.fn()} />)
    const pending = screen.getByRole("textbox")
    await user.type(pending, "新段落")
    fireEvent.keyDown(pending, { key: "Enter", ctrlKey: true })
    fireEvent.blur(pending)
    expect(onPersist).toHaveBeenCalledTimes(1)
  })

  it("does not persist an empty pending paragraph", async () => {
    const user = userEvent.setup()
    render(
      <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
        <ChapterText chapter={chapter()} bookId={1} unspaced fullTextOpen={false} onFullTextOpenChange={() => {}} />
      </QueryClientProvider>,
    )
    await user.click(screen.getByText("第一段。"))
    const editor = screen.getByRole("textbox") as HTMLTextAreaElement
    editor.setSelectionRange(editor.value.length, editor.value.length)
    await user.keyboard("{Enter}")
    expect(screen.getByRole("textbox")).toBeInTheDocument()
    await user.tab()
    expect(endpoints.patchContent).not.toHaveBeenCalled()
  })

  it("anchors after an external insert that advanced next_pid (R11-01)", async () => {
    const user = userEvent.setup()
    const one = chapter()
    one.content = { paragraphs: [{ pid: "p1", text: "one" }], next_pid: 2, tail_open: false }
    useChapterDoc.getState().load(1, 1, one.content)
    useChapterDoc.getState().applyOps(
      2,
      [{ op: "insert", after: "p1", pid: "p2", text: "external" }],
      "agent",
    )
    endpoints.patchContent.mockResolvedValueOnce({
      version: 3,
      ops: [
        { op: "update", pid: "p1", text: "one" },
        { op: "insert", after: "p1", pid: "p3", text: "two" },
        { op: "insert", after: "p3", pid: "p4", text: "new" },
      ],
      word_count: 3,
      tail_open: false,
    })
    render(
      <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
        <ChapterText chapter={one} bookId={1} unspaced fullTextOpen={false} onFullTextOpenChange={() => {}} />
      </QueryClientProvider>,
    )
    await user.click(screen.getByText("one"))
    const editor = screen.getByRole("textbox") as HTMLTextAreaElement
    fireEvent.change(editor, { target: { value: "one\ntwo" } })
    editor.setSelectionRange(editor.value.length, editor.value.length)
    await user.keyboard("{Enter}")
    await user.type(screen.getByRole("textbox"), "new")
    await user.tab()
    await waitFor(() => expect(endpoints.patchContent).toHaveBeenCalledTimes(1))
    expect(endpoints.patchContent.mock.calls[0]?.[1]).toEqual({
      base_version: 2,
      ops: [
        { op: "update", pid: "p1", text: "one\ntwo" },
        { op: "insert", after: "p3", text: "new" },
      ],
    })
  })

  it("orders a pending paragraph after a line break split (R10-04)", async () => {
    const user = userEvent.setup()
    const one = chapter()
    one.content = { paragraphs: [{ pid: "p1", text: "one" }], next_pid: 2, tail_open: false }
    useChapterDoc.getState().load(1, 1, one.content)
    endpoints.patchContent.mockResolvedValueOnce({
      version: 2,
      ops: [
        { op: "update", pid: "p1", text: "one" },
        { op: "insert", after: "p1", pid: "p2", text: "two" },
      ],
      word_count: 2,
      tail_open: false,
    })
    render(
      <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
        <ChapterText chapter={one} bookId={1} unspaced fullTextOpen={false} onFullTextOpenChange={() => {}} />
      </QueryClientProvider>,
    )
    await user.click(screen.getByText("one"))
    const editor = screen.getByRole("textbox") as HTMLTextAreaElement
    fireEvent.change(editor, { target: { value: "one\ntwo" } })
    editor.setSelectionRange(editor.value.length, editor.value.length)
    await user.keyboard("{Enter}")
    await user.type(screen.getByRole("textbox"), "new")
    await user.tab()
    await waitFor(() => expect(endpoints.patchContent).toHaveBeenCalledTimes(1))
    expect(endpoints.patchContent.mock.calls[0]?.[1]).toEqual({
      base_version: 1,
      ops: [
        { op: "update", pid: "p1", text: "one\ntwo" },
        { op: "insert", after: "p2", text: "new" },
      ],
    })
  })
})
