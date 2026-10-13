import { screen, waitFor, within } from "@testing-library/react"
import { HttpResponse, http } from "msw"
import { Route, Routes } from "react-router"
import { describe, expect, it } from "vitest"

import { WorkspacePage } from "./WorkspacePage"
import { PreviewContext, PreviewStore } from "@/api/events/preview"
import type { DecideReviewRequest } from "@/api/generated/DecideReviewRequest"
import type { RevisionRequest } from "@/api/generated/RevisionRequest"
import type { SaveBriefRequest } from "@/api/generated/SaveBriefRequest"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"
import {
  PROJECT_ID,
  RUN_ID,
  VERSION_ID,
  artifacts,
  reviewId,
  runView,
  workspaceView,
} from "@/test/fixtures"
import { renderWithProviders } from "@/test/render"
import { server } from "@/test/server"

const PATH = `/works/${PROJECT_ID}/versions/${VERSION_ID}`

/** A fake workspace endpoint, and the commands it received by path. */
function fakeWorkspace(initial: WorkspaceView) {
  let view = initial
  const commands: { path: string; body: unknown }[] = []
  const update = (change: (current: WorkspaceView) => WorkspaceView) => {
    view = change({ ...view, revision: view.revision + 1 })
    return HttpResponse.json({ revision: view.revision })
  }
  server.use(
    http.get("*/api/versions/:id/workspace", () => HttpResponse.json(view)),
    http.post("*/api/*", async ({ request }) => {
      const path = new URL(request.url).pathname
      const body: unknown = await request.json()
      commands.push({ path, body })
      if (path.endsWith("/brief")) {
        const { brief } = body as SaveBriefRequest
        return update((current) => ({
          ...current,
          brief: {
            id: "0190f5c8-0000-7000-8000-000000000300",
            number: 1,
            content: brief,
            createdAt: 0,
          },
        }))
      }
      if (path.endsWith("/preparation/start")) {
        const stage = { phase: "preparation", step: "generating" } as const
        return update((current) => ({
          ...current,
          version: { ...current.version, stage },
          run: runView({ status: "queued", requestCount: 0 }),
        }))
      }
      return update((current) => current)
    }),
  )
  return { commands }
}

function renderWorkspace(previews = new PreviewStore()) {
  return renderWithProviders(
    <PreviewContext value={previews}>
      <Routes>
        <Route path="/works/:projectId/versions/:versionId" element={<WorkspacePage />} />
      </Routes>
    </PreviewContext>,
    { route: PATH },
  )
}

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
})
