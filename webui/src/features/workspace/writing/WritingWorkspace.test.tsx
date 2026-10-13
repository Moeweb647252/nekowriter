import { screen, waitFor, within } from "@testing-library/react"
import { describe, expect, it } from "vitest"

import { PreviewStore } from "@/api/events/preview"
import type { AddExtraChapterRequest } from "@/api/generated/AddExtraChapterRequest"
import type { AnswerRequest } from "@/api/generated/AnswerRequest"
import type { ApprovalModeRequest } from "@/api/generated/ApprovalModeRequest"
import type { DecideReviewRequest } from "@/api/generated/DecideReviewRequest"
import type { FeedbackRequest } from "@/api/generated/FeedbackRequest"
import type { StartChapterRequest } from "@/api/generated/StartChapterRequest"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"
import {
  RUN_ID,
  VERSION_ID,
  artifacts,
  chapterView,
  review,
  revision,
  runView,
  workspaceView,
} from "@/test/fixtures"
import { fakeWorkspace, renderWorkspace } from "@/test/workspace"

const APPROVED = artifacts({
  background: "approved",
  research: "approved",
  characters: "approved",
  outline: "approved",
})
const FIRST = chapterView(1)
const SECOND = chapterView(2)
const REQUEST_ID = "0190f5c8-0000-7000-8000-000000000400"

function atChapter(
  step: "ready" | "outline_reviewing" | "drafting" | "done",
  chapters = [FIRST, SECOND],
  chapterId = FIRST.id,
) {
  return workspaceView({ phase: "chapter", chapterId, step }, { artifacts: APPROVED, chapters })
}

describe("WritingWorkspace", () => {
  it("starts a chapter's outline from the guidance", async () => {
    const { commands } = fakeWorkspace(atChapter("ready"))
    const { user } = renderWorkspace()

    expect(await screen.findByText("Goal: The keeper finds the letter.")).toBeInTheDocument()
    await user.type(
      screen.getByLabelText("Guidance for Chapter 1: Arrival"),
      "Open with the storm.",
    )
    await user.click(screen.getByRole("button", { name: "Start outline" }))

    await waitFor(() => {
      expect(commands).toHaveLength(1)
    })
    expect(commands[0]?.path).toBe(`/api/versions/${VERSION_ID}/chapters/start`)
    expect((commands[0]?.body as StartChapterRequest).guidance).toBe("Open with the storm.")
  })

  it("shows the outline for review and approves it", async () => {
    const outlined = {
      ...FIRST,
      status: "in_progress" as const,
      outline: revision("0190f5c8-0000-7000-8000-000000000600", "The storm, then the letter."),
      review: review("0190f5c8-0000-7000-8000-000000000601"),
    }
    const { commands } = fakeWorkspace({
      ...atChapter("outline_reviewing", [outlined, SECOND]),
      run: runView({ kind: "chapter_outline", status: "waiting_review" }),
    })
    const { user } = renderWorkspace()

    expect(await screen.findByText("The storm, then the letter.")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Approve outline" }))

    await waitFor(() => {
      expect(commands).toHaveLength(1)
    })
    expect(commands[0]?.path).toBe("/api/reviews/0190f5c8-0000-7000-8000-000000000601/decide")
    expect((commands[0]?.body as DecideReviewRequest).decision).toEqual({ kind: "approve" })
  })

  it("reads the committed body with the text being written apart from it", async () => {
    const drafting = {
      ...FIRST,
      status: "in_progress" as const,
      body: revision("0190f5c8-0000-7000-8000-000000000700", "The keeper woke.", {
        wordCount: 3,
      }),
    }
    fakeWorkspace({
      ...atChapter("drafting", [drafting, SECOND]),
      run: runView({ kind: "chapter_body", requestInFlight: REQUEST_ID }),
    })
    const previews = new PreviewStore()
    previews.set({
      runId: RUN_ID,
      versionId: VERSION_ID,
      requestId: REQUEST_ID,
      text: "",
      toolCalls: [{ index: 0, name: "append_content", arguments: '{"content":" Rain fe' }],
    })
    renderWorkspace(previews)

    const reader = await screen.findByRole("article", { name: "Manuscript" })
    const chapter = within(reader).getByRole("region", { name: "Chapter 1: Arrival" })
    expect(await within(chapter).findByText("The keeper woke.")).toBeInTheDocument()
    expect(within(chapter).getByText("Being written")).toBeInTheDocument()
    expect(within(chapter).getByText("Rain fe")).toBeInTheDocument()
    expect(within(reader).getByRole("region", { name: "Chapter 2: Storm" })).toBeInTheDocument()
    expect(screen.getByText("The agent is writing Chapter 1: Arrival. 3 words")).toBeInTheDocument()
  })

  it("goes on to the next chapter, then confirms the novel complete", async () => {
    const approved = { ...FIRST, status: "approved" as const }
    const { commands } = fakeWorkspace(atChapter("done", [approved, SECOND]))
    const { user, unmount } = renderWorkspace()
    await user.click(await screen.findByRole("button", { name: "Next chapter" }))
    await waitFor(() => {
      expect(commands).toHaveLength(1)
    })
    expect(commands[0]?.path).toBe(`/api/versions/${VERSION_ID}/chapters/next`)
    unmount()

    const last = { ...SECOND, status: "approved" as const }
    const { commands: finish } = fakeWorkspace(atChapter("done", [approved, last], SECOND.id))
    const { user: again } = renderWorkspace()
    expect(await screen.findByText(/Every planned chapter is approved/)).toBeInTheDocument()
    await again.click(screen.getByRole("button", { name: "Confirm the novel complete" }))
    await waitFor(() => {
      expect(finish).toHaveLength(1)
    })
    expect(finish[0]?.path).toBe(`/api/versions/${VERSION_ID}/complete`)
  })

  it("adds an extra chapter to a completed novel", async () => {
    const { commands } = fakeWorkspace(
      workspaceView(
        { phase: "completed" },
        { artifacts: APPROVED, chapters: [{ ...FIRST, status: "approved" }] },
      ),
    )
    const { user } = renderWorkspace()

    expect(await screen.findByText("The novel is complete.")).toBeInTheDocument()
    await user.type(screen.getByLabelText("Title of an extra chapter"), "Coda")
    await user.click(screen.getByRole("button", { name: "Add extra chapter" }))

    await waitFor(() => {
      expect(commands).toHaveLength(1)
    })
    expect(commands[0]?.path).toBe(`/api/versions/${VERSION_ID}/extra-chapters`)
    expect((commands[0]?.body as AddExtraChapterRequest).title).toBe("Coda")
  })

  describe("while the body is written", () => {
    const drafting = { ...FIRST, status: "in_progress" as const }
    const writing = (overrides: Partial<WorkspaceView> = {}) => ({
      ...atChapter("drafting", [drafting, SECOND]),
      run: runView({ kind: "chapter_body", status: "running" }),
      ...overrides,
    })

    it("pauses the writer", async () => {
      const { commands } = fakeWorkspace(writing())
      const { user } = renderWorkspace()

      await user.click(await screen.findByRole("button", { name: "Pause" }))

      await waitFor(() => {
        expect(commands).toHaveLength(1)
      })
      expect(commands[0]?.path).toBe(`/api/runs/${RUN_ID}/pause`)
    })

    it("shows a requested pause, which feedback also lets go on", async () => {
      fakeWorkspace(writing({ pauseRequested: true }))
      renderWorkspace()

      expect(
        await screen.findByText("The agent pauses before its next change."),
      ).toBeInTheDocument()
      expect(screen.getByRole("button", { name: "Cancel pause" })).toBeInTheDocument()
      expect(screen.getByRole("button", { name: "Send feedback and continue" })).toBeDisabled()
    })

    it("shows the held change and continues it", async () => {
      const { commands } = fakeWorkspace(
        writing({
          run: runView({ kind: "chapter_body", status: "paused" }),
          held: {
            id: "0190f5c8-0000-7000-8000-000000000800",
            tool: "replace_content",
            edit: { op: "replace", oldStr: "dark", newStr: "lit" },
          },
        }),
      )
      const { user } = renderWorkspace()

      expect(await screen.findByText("Replace “dark” with “lit”.")).toBeInTheDocument()
      await user.click(screen.getByRole("button", { name: "Continue" }))

      await waitFor(() => {
        expect(commands).toHaveLength(1)
      })
      expect(commands[0]?.path).toBe(`/api/runs/${RUN_ID}/resume`)
    })

    it("sends feedback and shows where each piece is", async () => {
      const { commands } = fakeWorkspace(
        writing({
          feedback: [
            {
              id: "0190f5c8-0000-7000-8000-000000000901",
              text: "Colder.",
              status: "included",
              delivery: "failed",
              createdAt: 0,
            },
            {
              id: "0190f5c8-0000-7000-8000-000000000902",
              text: "Slower.",
              status: "queued",
              delivery: null,
              createdAt: 0,
            },
          ],
        }),
      )
      const { user } = renderWorkspace()

      expect(await screen.findByText("Not delivered: the request failed")).toBeInTheDocument()
      expect(screen.getByText("Waiting for the next result")).toBeInTheDocument()
      await user.type(screen.getByLabelText("Feedback for the writer"), "More rain.")
      await user.click(screen.getByRole("button", { name: "Send feedback" }))

      await waitFor(() => {
        expect(commands).toHaveLength(1)
      })
      expect(commands[0]?.path).toBe(`/api/runs/${RUN_ID}/feedback`)
      expect((commands[0]?.body as FeedbackRequest).text).toBe("More rain.")
    })
  })

  describe("with each edit reviewed", () => {
    const body = revision("0190f5c8-0000-7000-8000-000000000a00", "The lamp was dark.", {
      wordCount: 4,
    })
    const proposing = () => ({
      ...atChapter("drafting", [{ ...FIRST, status: "in_progress" as const, body }, SECOND]),
      run: runView({ kind: "chapter_body", status: "waiting_review" }),
      approvalMode: "approve_required" as const,
      proposal: {
        reviewId: "0190f5c8-0000-7000-8000-000000000a01",
        callId: "0190f5c8-0000-7000-8000-000000000a02",
        edit: { op: "replace" as const, oldStr: "dark", newStr: "lit" },
        base: body.id,
      },
    })

    it("marks the proposed change in place and accepts it with empty feedback", async () => {
      const { commands } = fakeWorkspace(proposing())
      const { user } = renderWorkspace()

      expect(
        await screen.findByText("The agent proposes replacing “dark” with “lit”."),
      ).toBeInTheDocument()
      expect((await screen.findByText("dark")).closest("del")).not.toBeNull()
      expect(screen.getByText("lit").closest("ins")).not.toBeNull()
      await user.click(screen.getByRole("button", { name: "Accept edit" }))

      await waitFor(() => {
        expect(commands).toHaveLength(1)
      })
      expect(commands[0]?.path).toBe("/api/reviews/0190f5c8-0000-7000-8000-000000000a01/decide")
      expect((commands[0]?.body as DecideReviewRequest).decision).toEqual({ kind: "approve" })
    })

    it("rejects the edit once feedback is written", async () => {
      const { commands } = fakeWorkspace(proposing())
      const { user } = renderWorkspace()

      await user.type(
        await screen.findByLabelText("Feedback (leave empty to accept the edit)"),
        "Keep it dark.",
      )
      await user.click(screen.getByRole("button", { name: "Reject with feedback" }))

      await waitFor(() => {
        expect(commands).toHaveLength(1)
      })
      expect((commands[0]?.body as DecideReviewRequest).decision).toEqual({
        kind: "reject",
        feedback: "Keep it dark.",
      })
    })

    it("switches the mode", async () => {
      const { commands } = fakeWorkspace({
        ...atChapter("drafting", [{ ...FIRST, status: "in_progress" as const }, SECOND]),
        run: runView({ kind: "chapter_body", status: "running" }),
      })
      const { user } = renderWorkspace()

      await user.click(
        await screen.findByRole("switch", { name: "Review each edit before it is applied" }),
      )

      await waitFor(() => {
        expect(commands).toHaveLength(1)
      })
      expect(commands[0]?.path).toBe(`/api/versions/${VERSION_ID}/approval-mode`)
      expect((commands[0]?.body as ApprovalModeRequest).mode).toBe("approve_required")
    })
  })

  it("answers the agent's questions with choices and the user's own words", async () => {
    const callId = "0190f5c8-0000-7000-8000-000000000b00"
    const { commands } = fakeWorkspace({
      ...atChapter("drafting", [{ ...FIRST, status: "in_progress" as const }, SECOND]),
      run: runView({ kind: "chapter_body", status: "waiting_answer" }),
      question: {
        callId,
        questions: [
          {
            id: "q1",
            title: "What weather opens the chapter?",
            kind: "single",
            options: [
              { id: "a", label: "Rain" },
              { id: "b", label: "Snow" },
            ],
            required: true,
          },
          { id: "q2", title: "Name the keeper.", kind: "text", options: [], required: false },
        ],
      },
    })
    const { user } = renderWorkspace()

    await user.click(await screen.findByRole("radio", { name: "Snow" }))
    await user.type(screen.getByRole("textbox", { name: "Or answer in your own words" }), "and fog")
    await user.type(screen.getByRole("textbox", { name: "Your answer" }), "Mara")
    await user.click(screen.getByRole("button", { name: "Submit answer" }))

    await waitFor(() => {
      expect(commands).toHaveLength(1)
    })
    expect(commands[0]?.path).toBe(`/api/questions/${callId}/answer`)
    expect((commands[0]?.body as AnswerRequest).answers).toEqual([
      { questionId: "q1", selectedOptionIds: ["b"], customText: "and fog" },
      { questionId: "q2", selectedOptionIds: [], customText: "Mara" },
    ])
  })
})
