import { screen, waitFor, within } from "@testing-library/react"
import { HttpResponse, http } from "msw"
import { describe, expect, it } from "vitest"

import { WorksLibraryPage } from "./WorksLibraryPage"
import type { CreateProjectRequest } from "@/api/generated/CreateProjectRequest"
import type { ProjectRevisionRequest } from "@/api/generated/ProjectRevisionRequest"
import type { ProjectView } from "@/api/generated/ProjectView"
import { projectView } from "@/test/fixtures"
import { renderWithProviders } from "@/test/render"
import { server } from "@/test/server"

/** A fake project store behind the list and command endpoints. */
function fakeProjects(initial: ProjectView[]) {
  let projects = [...initial]
  const creates: CreateProjectRequest[] = []
  server.use(
    http.get("*/api/projects", ({ request }) => {
      const deleted = new URL(request.url).searchParams.get("filter") === "deleted"
      return HttpResponse.json(
        projects.filter((project) => (project.deletedAt !== null) === deleted),
      )
    }),
    http.post("*/api/projects", async ({ request }) => {
      const body = (await request.json()) as CreateProjectRequest
      creates.push(body)
      const created = projectView({
        id: crypto.randomUUID(),
        name: body.name,
        writingLanguage: body.writingLanguage,
        selectedVersion: { id: crypto.randomUUID(), number: 1, novelTitle: body.novelTitle },
      })
      projects = [created, ...projects]
      return HttpResponse.json(created, { status: 201 })
    }),
    http.post("*/api/projects/:id/delete", async ({ params, request }) => {
      const body = (await request.json()) as ProjectRevisionRequest
      const index = projects.findIndex((project) => project.id === params.id)
      const existing = projects[index]
      if (!existing) throw new Error("unknown project")
      const deleted = {
        ...existing,
        deletedAt: 1_760_000_100_000,
        revision: body.expectedRevision + 1,
      }
      projects[index] = deleted
      return HttpResponse.json(deleted)
    }),
    http.post("*/api/projects/:id/restore", ({ params }) => {
      const index = projects.findIndex((project) => project.id === params.id)
      const existing = projects[index]
      if (!existing) throw new Error("unknown project")
      const restored = { ...existing, deletedAt: null, revision: existing.revision + 1 }
      projects[index] = restored
      return HttpResponse.json(restored)
    }),
  )
  return { creates }
}

describe("WorksLibraryPage", () => {
  it("lists active works and shows an empty state when there are none", async () => {
    fakeProjects([projectView({ name: "Rain" })])
    renderWithProviders(<WorksLibraryPage />)

    expect(await screen.findByText("Rain")).toBeInTheDocument()
    expect(screen.getByText("Rain Letters")).toBeInTheDocument()
    expect(screen.getByText("English")).toBeInTheDocument()
  })

  it("creates a work with a command ID and the chosen fields", async () => {
    const { creates } = fakeProjects([])
    const { user } = renderWithProviders(<WorksLibraryPage />)
    expect(await screen.findByText("No works yet")).toBeInTheDocument()

    await user.click(screen.getByRole("button", { name: "New work" }))
    const dialog = await screen.findByRole("dialog")
    await user.type(within(dialog).getByLabelText("Work name"), "Rain")
    await user.type(within(dialog).getByLabelText("Novel title"), "Rain Letters")
    await user.click(within(dialog).getByRole("button", { name: "Create work" }))

    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    })
    expect(creates).toHaveLength(1)
    expect(creates[0]).toMatchObject({
      name: "Rain",
      novelTitle: "Rain Letters",
      writingLanguage: "en",
    })
    expect(creates[0]?.commandId).toMatch(/^[0-9a-f-]{36}$/)
    expect(await screen.findByText("Created “Rain”.")).toBeInTheDocument()
  })

  it("shows a validation error under the field it concerns", async () => {
    fakeProjects([])
    server.use(
      http.post("*/api/projects", () =>
        HttpResponse.json(
          {
            code: "FIELD_INVALID",
            params: { field: "project_name", problem: { kind: "empty" } },
            requestId: "r1",
          },
          { status: 422 },
        ),
      ),
    )
    const { user } = renderWithProviders(<WorksLibraryPage />)
    await user.click(await screen.findByRole("button", { name: "New work" }))
    const dialog = await screen.findByRole("dialog")

    await user.click(within(dialog).getByRole("button", { name: "Create work" }))

    expect(await within(dialog).findByText("Work name is required.")).toBeInTheDocument()
    expect(within(dialog).getByLabelText("Work name")).toHaveAttribute("aria-invalid", "true")
  })

  it("retries a failed submission with the same command ID", async () => {
    const commandIds: string[] = []
    let attempts = 0
    server.use(
      http.get("*/api/projects", () => HttpResponse.json([])),
      http.post("*/api/projects", async ({ request }) => {
        const body = (await request.json()) as CreateProjectRequest
        commandIds.push(body.commandId)
        attempts += 1
        if (attempts === 1) return HttpResponse.error()
        return HttpResponse.json(projectView({ name: body.name }), { status: 201 })
      }),
    )
    const { user } = renderWithProviders(<WorksLibraryPage />)
    await user.click(await screen.findByRole("button", { name: "New work" }))
    const dialog = await screen.findByRole("dialog")
    await user.type(within(dialog).getByLabelText("Work name"), "Rain")
    await user.type(within(dialog).getByLabelText("Novel title"), "Rain Letters")

    await user.click(within(dialog).getByRole("button", { name: "Create work" }))
    expect(
      await within(dialog).findByText(
        "NekoWriter could not be reached. Check that it is still running.",
      ),
    ).toBeInTheDocument()
    await user.click(within(dialog).getByRole("button", { name: "Create work" }))

    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    })
    expect(commandIds).toHaveLength(2)
    expect(commandIds[0]).toBe(commandIds[1])
  })

  it("deletes a work and restores it from the deleted list", async () => {
    fakeProjects([projectView({ name: "Rain" })])
    const { user } = renderWithProviders(<WorksLibraryPage />)

    await user.click(await screen.findByRole("button", { name: "Actions for Rain" }))
    await user.click(await screen.findByRole("menuitem", { name: "Delete" }))
    await user.click(await screen.findByRole("button", { name: "Delete work" }))
    expect(await screen.findByText("No works yet")).toBeInTheDocument()

    await user.click(screen.getByRole("tab", { name: "Deleted" }))
    await user.click(await screen.findByRole("button", { name: "Restore" }))
    await user.click(screen.getByRole("tab", { name: "In progress" }))

    expect(await screen.findByText("Rain")).toBeInTheDocument()
  })
})
