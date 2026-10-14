import { screen, waitFor } from "@testing-library/react"
import { HttpResponse, http } from "msw"
import type { SubmitEvent } from "react"
import { describe, expect, it, vi } from "vitest"

import { InsertTemplate } from "./InsertTemplate"
import { TemplatesPanel } from "./TemplatesPanel"
import type { CreateTemplateRequest } from "@/api/generated/CreateTemplateRequest"
import type { PromptTemplateView } from "@/api/generated/PromptTemplateView"
import { renderWithProviders } from "@/test/render"
import { server } from "@/test/server"

const SLOW: PromptTemplateView = {
  id: "0190f5c8-0000-7000-8000-000000000d01",
  title: "Slow down",
  body: "Slow {{scene}} down.",
  variables: [{ name: "scene", description: "The scene to slow down" }],
  revision: 1,
  updatedAt: 0,
}

describe("templates", () => {
  it("creates a template with its placeholders", async () => {
    const creates: CreateTemplateRequest[] = []
    server.use(
      http.get("*/api/templates", () => HttpResponse.json([])),
      http.post("*/api/templates", async ({ request }) => {
        const body = (await request.json()) as CreateTemplateRequest
        creates.push(body)
        return HttpResponse.json({ ...SLOW, ...body }, { status: 201 })
      }),
    )
    const { user } = renderWithProviders(<TemplatesPanel />)

    await user.type(await screen.findByLabelText("Title"), "Slow down")
    await user.type(screen.getByLabelText("Text"), "Slow {{{{scene}} down.")
    await user.click(screen.getByRole("button", { name: "Add placeholder" }))
    await user.type(screen.getByLabelText("Name"), "scene")
    await user.type(screen.getByLabelText("What to fill in"), "The scene to slow down")
    await user.click(screen.getByRole("button", { name: "Create template" }))

    await waitFor(() => {
      expect(creates).toHaveLength(1)
    })
    expect(creates[0]).toMatchObject({
      title: "Slow down",
      body: "Slow {{scene}} down.",
      variables: [{ name: "scene", description: "The scene to slow down" }],
    })
  })

  it("inserts a template with its placeholders filled in, leaving the draft's form alone", async () => {
    const renders: unknown[] = []
    server.use(
      http.get("*/api/templates", () => HttpResponse.json([SLOW])),
      http.post("*/api/templates/render", async ({ request }) => {
        renders.push(await request.json())
        return HttpResponse.json({ text: "Slow the storm down." })
      }),
    )
    const onInsert = vi.fn()
    const onSubmit = vi.fn((event: SubmitEvent<HTMLFormElement>) => {
      event.preventDefault()
    })
    // It sits inside the form of the draft it adds to, as in the dock.
    const { user } = renderWithProviders(
      <form onSubmit={onSubmit}>
        <InsertTemplate onInsert={onInsert} />
      </form>,
    )

    await user.click(screen.getByRole("button", { name: "Insert template" }))
    await user.click(await screen.findByRole("combobox", { name: "Template" }))
    await user.click(await screen.findByRole("option", { name: "Slow down" }))
    await user.type(await screen.findByLabelText("scene"), "the storm")
    await user.click(screen.getByRole("button", { name: "Insert" }))

    await waitFor(() => {
      expect(onInsert).toHaveBeenCalledWith("Slow the storm down.")
    })
    expect(renders[0]).toEqual({ template: "Slow {{scene}} down.", values: { scene: "the storm" } })
    expect(onSubmit).not.toHaveBeenCalled()
  })
})
