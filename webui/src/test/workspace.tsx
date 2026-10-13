import { HttpResponse, http } from "msw"
import { Route, Routes } from "react-router"

import { PROJECT_ID, REVISIONS, VERSION_ID, runView } from "./fixtures"
import { renderWithProviders } from "./render"
import { server } from "./server"
import { PreviewContext, PreviewStore } from "@/api/events/preview"
import type { SaveBriefRequest } from "@/api/generated/SaveBriefRequest"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"
import { WorkspacePage } from "@/features/workspace/WorkspacePage"

const PATH = `/works/${PROJECT_ID}/versions/${VERSION_ID}`

/** A fake workspace endpoint, and the commands it received by path. */
export function fakeWorkspace(initial: WorkspaceView) {
  let view = initial
  const commands: { path: string; body: unknown }[] = []
  const update = (change: (current: WorkspaceView) => WorkspaceView) => {
    view = change({ ...view, revision: view.revision + 1 })
    return HttpResponse.json({ revision: view.revision })
  }
  server.use(
    http.get("*/api/versions/:id/workspace", () => HttpResponse.json(view)),
    http.get("*/api/revisions/:id", ({ params }) => {
      const revision = REVISIONS.get(String(params.id))
      return revision === undefined
        ? HttpResponse.json(
            { code: "NOT_FOUND", params: { resource: "revision" } },
            { status: 404 },
          )
        : HttpResponse.json(revision)
    }),
    http.post("*/api/*", async ({ request }) => {
      const path = new URL(request.url).pathname
      const body: unknown = await request.json()
      commands.push({ path, body })
      if (path.endsWith("/brief")) {
        const { brief } = body as SaveBriefRequest
        return update((current) => ({
          ...current,
          brief: {
            id: "0190f5c8-0000-7000-8000-000000000500",
            number: 1,
            content: brief,
            chapterPlan: null,
            wordCount: null,
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

export function renderWorkspace(previews = new PreviewStore()) {
  return renderWithProviders(
    <PreviewContext value={previews}>
      <Routes>
        <Route path="/works/:projectId/versions/:versionId" element={<WorkspacePage />} />
      </Routes>
    </PreviewContext>,
    { route: PATH },
  )
}
