import { screen, waitFor, within } from "@testing-library/react"
import { describe, expect, it } from "vitest"

import { PreviewStore } from "@/api/events/preview"
import type { DecideReviewRequest } from "@/api/generated/DecideReviewRequest"
import type { RevisionRequest } from "@/api/generated/RevisionRequest"
import { RUN_ID, VERSION_ID, artifacts, reviewId, runView, workspaceView } from "@/test/fixtures"
import { fakeWorkspace, renderWorkspace } from "@/test/workspace"

describe("WorkspacePage", () => {
  it("saves the brief, then starts preparation", async () => {
    const { commands } = fakeWorkspace(workspaceView({ phase: "preparation", step: "not_started" }))
    const { user } = renderWorkspace()

    const brief = await screen.findByLabelText("Novel brief")
    await user.type(brief, "A ghost story in a lighthouse.")
    expect(screen.getByText("Draft saved in this browser")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Start preparation" })).toBeDisabled()

    await user.click(screen.getByRole("button", { name: "Save brief" }))
    expect(await screen.findByText("Saved")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Start preparation" }))

    expect(await screen.findByText("Waiting for the model")).toBeInTheDocument()
    expect(commands.map((command) => command.path)).toEqual([
      `/api/versions/${VERSION_ID}/brief`,
      `/api/versions/${VERSION_ID}/preparation/start`,
    ])
    expect(commands[0]?.body).toMatchObject({
      expectedRevision: 3,
      brief: "A ghost story in a lighthouse.",
    })
    expect((commands[1]?.body as RevisionRequest).expectedRevision).toBe(4)
  })

  it("approves a document and requests changes to another with feedback", async () => {
    const { commands } = fakeWorkspace(
      workspaceView(
        { phase: "preparation", step: "reviewing" },
        {
          run: runView({ status: "waiting_review" }),
          artifacts: artifacts({
            background: "open",
            research: "open",
            characters: "open",
            outline: "open",
          }),
        },
      ),
    )
    const { user } = renderWorkspace()

    expect(await screen.findByText("The background text.")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Approve Background" }))
    await waitFor(() => {
      expect(commands).toHaveLength(1)
    })

    await user.click(screen.getByRole("tab", { name: "Research" }))
    expect(await screen.findByText("The research text.")).toBeInTheDocument()
    const requestChanges = screen.getByRole("button", { name: "Request changes" })
    expect(requestChanges).toBeDisabled()
    await user.type(screen.getByLabelText("What should change?"), "Cite the lamp manuals.")
    await user.click(requestChanges)

    await waitFor(() => {
      expect(commands).toHaveLength(2)
    })
    expect(commands[0]?.path).toBe(`/api/reviews/${reviewId("background")}/decide`)
    expect((commands[0]?.body as DecideReviewRequest).decision).toEqual({ kind: "approve" })
    expect(commands[1]?.path).toBe(`/api/reviews/${reviewId("research")}/decide`)
    expect((commands[1]?.body as DecideReviewRequest).decision).toEqual({
      kind: "reject",
      feedback: "Cite the lamp manuals.",
    })
  })

  it("explains why a run failed and continues it", async () => {
    const { commands } = fakeWorkspace(
      workspaceView(
        { phase: "preparation", step: "generating" },
        {
          run: runView({
            status: "failed",
            halt: { kind: "provider", failure: { kind: "http_status", status: 401 } },
          }),
        },
      ),
    )
    const { user } = renderWorkspace()

    expect(
      await screen.findByText(
        "The model service failed: the service answered with HTTP status 401. Check the service, then continue.",
      ),
    ).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Continue" }))

    await waitFor(() => {
      expect(commands).toHaveLength(1)
    })
    expect(commands[0]?.path).toBe(`/api/runs/${RUN_ID}/continue`)
    expect((commands[0]?.body as RevisionRequest).expectedRevision).toBe(3)
  })

  it("shows the live preview while the agent writes", async () => {
    fakeWorkspace(
      workspaceView(
        { phase: "preparation", step: "generating" },
        { run: runView({ status: "running" }) },
      ),
    )
    const previews = new PreviewStore()
    previews.set({
      runId: RUN_ID,
      versionId: VERSION_ID,
      requestId: "0190f5c8-0000-7000-8000-000000000400",
      text: "",
      toolCalls: [
        {
          index: 0,
          name: "submit_preparation",
          arguments: '{"artifacts":[{"kind":"background","content":"A lighthouse on a cold co',
        },
      ],
    })
    renderWorkspace(previews)

    const preview = await screen.findByRole("region", { name: "Live preview" })
    expect(within(preview).getByRole("heading", { name: "Background" })).toBeInTheDocument()
    expect(within(preview).getByText("A lighthouse on a cold co")).toBeInTheDocument()
    expect(screen.getByText("The agent is writing the preparation documents.")).toBeInTheDocument()
  })

  it("stops a run that is writing", async () => {
    const { commands } = fakeWorkspace(
      workspaceView(
        { phase: "preparation", step: "generating" },
        { run: runView({ status: "running" }) },
      ),
    )
    const { user } = renderWorkspace()

    await user.click(await screen.findByRole("button", { name: "Stop" }))

    await waitFor(() => {
      expect(commands).toHaveLength(1)
    })
    expect(commands[0]?.path).toBe(`/api/runs/${RUN_ID}/stop`)
    expect(commands[0]?.body).toEqual({ commandId: expect.any(String) as string })
  })

  it("shows who decided a review and how confident the review agent was", async () => {
    const reviewed = artifacts({
      background: "open",
      research: "approved",
      characters: "open",
      outline: "open",
    }).map((artifact) =>
      artifact.kind === "research" && artifact.review !== null
        ? {
            ...artifact,
            review: { ...artifact.review, decidedBy: "delegate" as const, confidencePermille: 900 },
          }
        : artifact,
    )
    fakeWorkspace(
      workspaceView(
        { phase: "preparation", step: "reviewing" },
        { run: runView({ status: "waiting_review" }), artifacts: reviewed },
      ),
    )
    const { user } = renderWorkspace()

    await user.click(await screen.findByRole("tab", { name: "Research" }))

    expect(
      await screen.findByText("Decided by the review agent, 90% confident."),
    ).toBeInTheDocument()
  })
})
