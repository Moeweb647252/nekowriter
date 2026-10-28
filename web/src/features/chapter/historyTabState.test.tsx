/** HistoryTab's note and diff selection are chapter-scoped (M5-06). */
import "@/i18n"
import { act, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { describe, expect, it, vi } from "vitest"

import type { ChapterDto } from "@/api/gen/ChapterDto"

const saveMutate = vi.hoisted(() => vi.fn())
const version = {
  id: 101,
  chapter_id: 1,
  version_no: 1,
  kind: "draft",
  title: "Chapter",
  word_count: 2,
  note: "",
  session_id: null,
  created_at: "2026-01-01T00:00:00Z",
}
const versionDetail = {
  ...version,
  content: { paragraphs: [{ pid: "p1", text: "旧正文" }], next_pid: 2, tail_open: false },
}
vi.mock("@/api/hooks/chapters", () => ({
  useVersions: () => ({ data: [version], isPending: false }),
  useSaveVersion: () => ({ mutate: saveMutate, isPending: false }),
  useRestoreVersion: () => ({ mutate: vi.fn(), isPending: false }),
  useVersion: (id: number | null) => ({
    data: id === version.id ? versionDetail : undefined,
  }),
}))
vi.mock("@/components/common/ErrorToast", () => ({
  useErrorToast: () => vi.fn(),
}))

import { HistoryTab } from "@/features/chapter/HistoryTab"

function chapter(id: number): ChapterDto {
  return {
    id,
    book_id: 1,
    track_id: 1,
    number: id,
    title: "Chapter",
    status: "drafting",
    canon_status: "none",
    word_count: 2,
    has_outline: false,
    content_version: 1,
    content: { paragraphs: [], next_pid: 1, tail_open: false },
    open_sessions: [],
    guidance: "",
    outline: "",
    reference_notes: "",
    summary: "",
    official_version_id: null,
    updated_at: "2026-01-01T00:00:00Z",
  }
}

describe("HistoryTab chapter state", () => {
  it("clears the note and diff when the chapter changes", async () => {
    const user = userEvent.setup()
    const view = (id: number) => <HistoryTab chapter={chapter(id)} bookId={1} unspaced />
    const { rerender } = render(view(1))
    await user.type(screen.getByPlaceholderText("Note for this version (optional)"), "A 章的备注")
    await user.click(screen.getByRole("button", { name: "From" }))
    expect(screen.getByText("Diff #101 → #current")).toBeInTheDocument()
    expect(screen.getByPlaceholderText("Note for this version (optional)")).toHaveValue("A 章的备注")
    rerender(view(2))
    expect(screen.getByPlaceholderText("Note for this version (optional)")).toHaveValue("")
    expect(screen.queryByText("Diff #101 → #current")).not.toBeInTheDocument()
  })

  it("does not let a late chapter-A save callback clear chapter B's note", async () => {
    const user = userEvent.setup()
    let saveSuccess: (() => void) | undefined
    saveMutate.mockImplementation((_input: unknown, options: { onSuccess: () => void }) => {
      saveSuccess = options.onSuccess
    })
    const view = (id: number) => <HistoryTab chapter={chapter(id)} bookId={1} unspaced />
    const { rerender } = render(view(1))
    await user.type(screen.getByPlaceholderText("Note for this version (optional)"), "A 的备注")
    await user.click(screen.getByRole("button", { name: "Save version" }))
    expect(saveSuccess).toBeTypeOf("function")
    rerender(view(2))
    await user.type(screen.getByPlaceholderText("Note for this version (optional)"), "B 的备注")
    await act(async () => {
      saveSuccess?.()
    })
    expect(screen.getByPlaceholderText("Note for this version (optional)")).toHaveValue("B 的备注")
  })
})
