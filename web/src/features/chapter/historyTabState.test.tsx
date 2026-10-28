/** HistoryTab's note and diff selection are chapter-scoped (M5-06). */
import "@/i18n"
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { describe, expect, it, vi } from "vitest"

import type { ChapterDto } from "@/api/gen/ChapterDto"

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
vi.mock("@/api/hooks/chapters", () => ({
  useVersions: () => ({ data: [version], isPending: false }),
  useSaveVersion: () => ({ mutate: vi.fn(), isPending: false }),
  useRestoreVersion: () => ({ mutate: vi.fn(), isPending: false }),
  useVersion: () => ({ data: undefined }),
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
    expect(screen.getByPlaceholderText("Note for this version (optional)")).toHaveValue("A 章的备注")
    rerender(view(2))
    expect(screen.getByPlaceholderText("Note for this version (optional)")).toHaveValue("")
    expect(screen.queryByText(/#101/)).not.toBeInTheDocument()
  })
})
