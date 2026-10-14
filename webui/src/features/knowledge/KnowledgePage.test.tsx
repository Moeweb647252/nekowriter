import { screen, waitFor } from "@testing-library/react"
import { HttpResponse, http } from "msw"
import { describe, expect, it } from "vitest"

import { KnowledgePage } from "./KnowledgePage"
import type { CreateEntryRequest } from "@/api/generated/CreateEntryRequest"
import type { EntryView } from "@/api/generated/EntryView"
import type { SpaceView } from "@/api/generated/SpaceView"
import { renderWithProviders } from "@/test/render"
import { server } from "@/test/server"

const SPACE: SpaceView = {
  id: "0190f5c8-0000-7000-8000-000000000e01",
  name: "Lighthouses",
  entryCount: 0,
  revision: 1,
  updatedAt: 0,
}

describe("KnowledgePage", () => {
  it("creates an entry in the selected space with its keywords", async () => {
    const creates: CreateEntryRequest[] = []
    server.use(
      http.get("*/api/knowledge/spaces", () => HttpResponse.json([SPACE])),
      http.get("*/api/knowledge/entries", () => HttpResponse.json([])),
      http.post("*/api/knowledge/entries", async ({ request }) => {
        const body = (await request.json()) as CreateEntryRequest
        creates.push(body)
        const created: EntryView = {
          id: "0190f5c8-0000-7000-8000-000000000e02",
          spaceId: body.spaceId,
          title: body.title,
          keywords: body.keywords,
          content: body.content,
          revision: 1,
          updatedAt: 0,
          index: { state: "pending" },
        }
        return HttpResponse.json(created, { status: 201 })
      }),
    )
    const { user } = renderWithProviders(<KnowledgePage />)

    await user.click(await screen.findByRole("button", { name: /^Lighthouses/ }))
    await user.click(screen.getByRole("button", { name: "New entry" }))
    await user.type(screen.getByLabelText("Title"), "Lamp oil")
    await user.type(screen.getByLabelText("Keywords"), "fuel\uff0clamp, ")
    await user.type(screen.getByLabelText("Content"), "Carried up the stairs.")
    await user.click(screen.getByRole("button", { name: "Create entry" }))

    await waitFor(() => {
      expect(creates).toHaveLength(1)
    })
    expect(creates[0]).toMatchObject({
      spaceId: SPACE.id,
      title: "Lamp oil",
      keywords: ["fuel", "lamp", " "],
      content: "Carried up the stairs.",
    })
  })
})
