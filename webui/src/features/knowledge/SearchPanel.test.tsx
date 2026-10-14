import { screen } from "@testing-library/react"
import { HttpResponse, http } from "msw"
import { describe, expect, it, vi } from "vitest"

import { SearchPanel } from "./SearchPanel"
import type { SearchRequest } from "@/api/generated/SearchRequest"
import type { SearchResults } from "@/api/generated/SearchResults"
import { renderWithProviders } from "@/test/render"
import { server } from "@/test/server"

const SPACE = "0190f5c8-0000-7000-8000-000000000e01"
const ENTRY = "0190f5c8-0000-7000-8000-000000000e02"

describe("SearchPanel", () => {
  it("searches the selected space and opens a result", async () => {
    const requests: SearchRequest[] = []
    const results: SearchResults = {
      hits: [
        {
          entryId: ENTRY,
          entryRevision: 1,
          chunkId: 7,
          title: "Storms",
          spaceId: SPACE,
          spaceName: "Lighthouses",
          text: "A storm rolls in from the sea.",
          relevance: 0.8734,
        },
      ],
    }
    server.use(
      http.post("*/api/knowledge/search", async ({ request }) => {
        requests.push((await request.json()) as SearchRequest)
        return HttpResponse.json(results)
      }),
    )
    const onOpen = vi.fn()
    const { user } = renderWithProviders(<SearchPanel space={SPACE} onOpen={onOpen} />)

    await user.type(screen.getByRole("textbox", { name: "Search by meaning" }), "bad weather")
    await user.click(screen.getByRole("button", { name: "Search" }))

    await user.click(await screen.findByRole("button", { name: /Storms/ }))
    expect(onOpen).toHaveBeenCalledWith(ENTRY)
    expect(screen.getByText("Lighthouses · Relevance 0.87")).toBeInTheDocument()
    expect(requests[0]).toEqual({ query: "bad weather", spaces: [SPACE] })
  })

  it("explains why search is unavailable", async () => {
    server.use(
      http.post("*/api/knowledge/search", () =>
        HttpResponse.json(
          {
            code: "SEARCH_UNAVAILABLE",
            params: { reason: { kind: "indexing", remaining: 3 } },
            requestId: "r1",
          },
          { status: 409 },
        ),
      ),
    )
    const { user } = renderWithProviders(<SearchPanel space={null} onOpen={vi.fn()} />)

    await user.type(screen.getByRole("textbox", { name: "Search by meaning" }), "lamps")
    await user.click(screen.getByRole("button", { name: "Search" }))

    expect(
      await screen.findByText(
        "The knowledge base is being indexed for the new embedding model; 3 entries remain.",
      ),
    ).toBeInTheDocument()
  })
})
