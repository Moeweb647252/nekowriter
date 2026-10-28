/** The Outline tab's states and decisions (PROJECT §8.5, D-146). */
import { render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeEach, describe, expect, it, vi } from "vitest"

import type { ChapterDto } from "@/api/gen/ChapterDto"
import type { SessionDto } from "@/api/gen/SessionDto"

const resume = vi.hoisted(() => ({ mutate: vi.fn(), isPending: false }))
const patchChapter = vi.hoisted(() => ({
  mutate: vi.fn(),
  mutateAsync: vi.fn(async () => ({})),
  isPending: false,
}))
const writerState = vi.hoisted(() => ({
  current: { session: undefined as SessionDto | undefined, heldOutline: undefined as unknown },
}))

vi.mock("@/api/hooks/chapters", () => ({
  usePatchChapter: () => patchChapter,
  useWriteOutlineChapter: () => ({ mutate: vi.fn(), isPending: false }),
}))
vi.mock("@/api/hooks/sessions", () => ({
  useResumeSession: () => resume,
  useSession: () => ({ data: undefined }),
}))
vi.mock("@/features/chapter/useWriterState", () => ({
  useWriterState: () => writerState.current,
}))
vi.mock("@/features/chapter/OutlineDialog", () => ({ OutlineDialog: () => null }))
vi.mock("@/components/common/ErrorToast", () => ({ useErrorToast: () => vi.fn() }))

import { OutlineTab } from "@/features/chapter/OutlineTab"

function chapter(overrides: Partial<ChapterDto> = {}): ChapterDto {
  return {
    id: 4,
    book_id: 1,
    track_id: 2,
    number: 4,
    title: "Four",
    status: "planned",
    canon_status: "none",
    word_count: 0,
    has_outline: false,
    updated_at: "2026-01-01T00:00:00Z",
    guidance: "",
    outline: "",
    reference_notes: "",
    summary: "",
    content: { paragraphs: [], next_pid: 1, tail_open: false },
    content_version: 0,
    official_version_id: null,
    open_sessions: [],
    ...overrides,
  }
}

const SETTINGS = {
  granularity: "paragraph" as const,
  auto_pause: false,
  pause_after_outline: true,
  target_words_min: 3000,
  target_words_max: 4000,
  recent_summary_count: 5,
  recent_chapter_count: 2,
  midterm_window: 10,
  auto_maintenance: true,
}

const HELD_SESSION = {
  id: 9,
  book_id: 1,
  role: "planner",
  kind: "chapter_outline",
  status: "held",
  held_tool_call_id: 5,
} as unknown as SessionDto

beforeEach(() => {
  resume.mutate.mockReset()
  patchChapter.mutate.mockReset()
  patchChapter.mutateAsync.mockClear()
  writerState.current = { session: undefined, heldOutline: undefined }
})

describe("OutlineTab", () => {
  it("approves a held proposal without feedback", async () => {
    const user = userEvent.setup()
    render(
      <OutlineTab
        chapter={chapter({ has_outline: true, outline: "旧细纲" })}
        bookId={1}
        unspaced={false}
        settings={SETTINGS}
        state="awaiting_approval"
        session={HELD_SESSION}
        calls={[
          {
            id: 5,
            name: "submit_outline",
            input: { outline: "提案细纲", reference_notes: "提案要点", title: "提案标题" },
          } as never,
        ]}
      />,
    )
    expect(screen.getByDisplayValue("提案细纲")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Approve outline" }))
    expect(resume.mutate).toHaveBeenCalledWith({ sessionId: 9, feedback: undefined }, expect.anything())
  })

  it("requests changes with the author's feedback", async () => {
    const user = userEvent.setup()
    render(
      <OutlineTab
        chapter={chapter()}
        bookId={1}
        unspaced={false}
        settings={SETTINGS}
        state="awaiting_approval"
        session={HELD_SESSION}
        calls={[
          {
            id: 5,
            name: "submit_outline",
            input: { outline: "提案细纲", reference_notes: "提案要点" },
          } as never,
        ]}
      />,
    )
    await user.click(screen.getByRole("button", { name: "Request changes" }))
    await user.type(screen.getByPlaceholderText("What should change?"), "再紧一点")
    await user.click(screen.getByRole("button", { name: "Send and continue" }))
    expect(resume.mutate).toHaveBeenCalledWith(
      { sessionId: 9, feedback: "再紧一点" },
      expect.anything(),
    )
  })

  it("keeps the legacy path: save the fields, then resume the held call", async () => {
    const user = userEvent.setup()
    writerState.current = {
      session: { id: 7, kind: "chapter_write", status: "held" } as unknown as SessionDto,
      heldOutline: { id: 1 },
    }
    render(
      <OutlineTab
        chapter={chapter({ outline: "旧细纲", reference_notes: "旧要点" })}
        bookId={1}
        unspaced={false}
        settings={SETTINGS}
        state="in_progress"
      />,
    )
    await user.type(screen.getByLabelText("Outline"), " 改过")
    await user.click(screen.getByRole("button", { name: "Approve outline" }))
    await waitFor(() => {
      expect(patchChapter.mutateAsync).toHaveBeenCalledWith({
        outline: "旧细纲 改过",
        reference_notes: "旧要点",
      })
      expect(resume.mutate).toHaveBeenCalledWith({ sessionId: 7, feedback: undefined }, expect.anything())
    })
  })

  it("adopts a new server outline when the author has not typed", () => {
    const view = (chapterValue: ChapterDto, state: "awaiting_approval" | "accepted") => (
      <OutlineTab
        chapter={chapterValue}
        bookId={1}
        unspaced={false}
        settings={SETTINGS}
        state={state}
        session={state === "awaiting_approval" ? HELD_SESSION : undefined}
        calls={
          state === "awaiting_approval"
            ? [
                {
                  id: 5,
                  name: "submit_outline",
                  input: { outline: "", reference_notes: "" },
                } as never,
              ]
            : undefined
        }
      />
    )
    const { rerender } = render(view(chapter({ outline: "" }), "awaiting_approval"))
    rerender(view(chapter({ outline: "提议细纲", has_outline: true }), "accepted"))
    expect(screen.getByLabelText("Outline")).toHaveValue("提议细纲")
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled()
  })

  it("keeps the author's typing when a refetch brings a different server value", async () => {
    const user = userEvent.setup()
    const view = (outline: string) => (
      <OutlineTab
        chapter={chapter({ outline, has_outline: true })}
        bookId={1}
        unspaced={false}
        settings={SETTINGS}
        state="accepted"
      />
    )
    const { rerender } = render(view("原始细纲"))
    await user.clear(screen.getByLabelText("Outline"))
    await user.type(screen.getByLabelText("Outline"), "作者正在写的细纲")
    rerender(view("服务器新值"))
    expect(screen.getByLabelText("Outline")).toHaveValue("作者正在写的细纲")
  })

  it("clears a typed draft when the chapter id changes", async () => {
    const user = userEvent.setup()
    const view = (chapterValue: ChapterDto) => (
      <OutlineTab
        chapter={chapterValue}
        bookId={1}
        unspaced={false}
        settings={SETTINGS}
        state="accepted"
      />
    )
    const { rerender } = render(view(chapter({ id: 1, outline: "第一章细纲" })))
    await user.clear(screen.getByLabelText("Outline"))
    await user.type(screen.getByLabelText("Outline"), "第一章的作者草稿")
    rerender(view(chapter({ id: 2, outline: "第二章细纲" })))
    expect(screen.getByLabelText("Outline")).toHaveValue("第二章细纲")
  })

  it("gates Save and Write the outline while an owner session is open", () => {
    const runningWriter = {
      id: 7,
      book_id: 1,
      role: "writer",
      kind: "chapter_write",
      status: "running",
    } as unknown as SessionDto
    render(
      <OutlineTab
        chapter={chapter({ open_sessions: [runningWriter] })}
        bookId={1}
        unspaced={false}
        settings={SETTINGS}
        state="missing"
      />,
    )
    expect(screen.queryByRole("button", { name: "Write the outline" })).not.toBeInTheDocument()
    expect(screen.getByLabelText("Outline")).toHaveAttribute("readonly")
  })

  it("offers the in-progress focus link", async () => {
    const user = userEvent.setup()
    const onFocusSession = vi.fn()
    const session = { id: 9, kind: "chapter_outline", status: "running" } as unknown as SessionDto
    render(
      <OutlineTab
        chapter={chapter()}
        bookId={1}
        unspaced={false}
        settings={SETTINGS}
        state="in_progress"
        session={session}
        onFocusSession={onFocusSession}
      />,
    )
    await user.click(screen.getByRole("button", { name: "Focus in the agent panel" }))
    expect(onFocusSession).toHaveBeenCalledWith(9)
  })

  it("shows an empty state with Write the outline when no outline exists", () => {
    render(
      <OutlineTab
        chapter={chapter()}
        bookId={1}
        unspaced={false}
        settings={SETTINGS}
        state="missing"
      />,
    )
    expect(screen.getByText("No outline yet")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Write the outline" })).toBeInTheDocument()
    // The fields stay editable, so the author can write the outline by hand (PROJECT §8.5).
    expect(screen.getByLabelText("Outline")).not.toHaveAttribute("readonly")
  })

  it("shows read-only fields while the planner is working", () => {
    render(
      <OutlineTab
        chapter={chapter({ outline: "进行中的旧细纲", has_outline: true })}
        bookId={1}
        unspaced={false}
        settings={SETTINGS}
        state="in_progress"
        session={{ id: 9, kind: "chapter_outline", status: "running" } as unknown as SessionDto}
      />,
    )
    expect(screen.getByText("The planner is working")).toBeInTheDocument()
    expect(screen.getByLabelText("Outline")).toHaveAttribute("readonly")
  })
})
