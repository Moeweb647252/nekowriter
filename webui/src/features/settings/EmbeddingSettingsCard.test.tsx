import { screen, waitFor } from "@testing-library/react"
import { HttpResponse, http } from "msw"
import { describe, expect, it } from "vitest"

import { EmbeddingSettingsCard } from "./EmbeddingSettingsCard"
import type { EmbeddingSettingsView } from "@/api/generated/EmbeddingSettingsView"
import type { SaveEmbeddingRequest } from "@/api/generated/SaveEmbeddingRequest"
import type { TestEmbeddingRequest } from "@/api/generated/TestEmbeddingRequest"
import { renderWithProviders } from "@/test/render"
import { server } from "@/test/server"

const STORED: EmbeddingSettingsView = {
  apiBaseUrl: "https://embed.example/v1",
  apiKeySet: true,
  model: null,
  revision: 2,
}

describe("EmbeddingSettingsCard", () => {
  it("tests and saves a model without ever sending the stored key", async () => {
    const saves: SaveEmbeddingRequest[] = []
    const tests: TestEmbeddingRequest[] = []
    server.use(
      http.get("*/api/settings/embedding", () => HttpResponse.json(STORED)),
      http.post("*/api/settings/test-embedding", async ({ request }) => {
        tests.push((await request.json()) as TestEmbeddingRequest)
        return HttpResponse.json(
          {
            code: "EMBEDDING_FAILED",
            params: { failure: { kind: "wrong_dimensions", expected: 768, actual: 1024 } },
            requestId: "r1",
          },
          { status: 502 },
        )
      }),
      http.post("*/api/settings/embedding", async ({ request }) => {
        const body = (await request.json()) as SaveEmbeddingRequest
        saves.push(body)
        return HttpResponse.json({ ...STORED, model: body.model, revision: 3 })
      }),
    )
    const { user } = renderWithProviders(<EmbeddingSettingsCard />)

    await user.click(await screen.findByRole("switch", { name: "Set an embedding model" }))
    await user.type(screen.getByLabelText("Model ID"), "text-embedding-3-small")
    await user.clear(screen.getByLabelText("Dimensions"))
    await user.type(screen.getByLabelText("Dimensions"), "768")
    await user.click(screen.getByRole("button", { name: "Test embeddings" }))

    expect(
      await screen.findByText(
        "The vectors have 1,024 dimensions, but the settings say 768. Check the model and its dimensions.",
      ),
    ).toBeInTheDocument()
    expect(tests[0]).toEqual({
      apiBaseUrl: "https://embed.example/v1",
      model: { modelId: "text-embedding-3-small", dimensions: 768 },
    })

    await user.click(screen.getByRole("button", { name: "Save" }))
    await waitFor(() => {
      expect(saves).toHaveLength(1)
    })
    expect(saves[0]).toMatchObject({
      expectedRevision: 2,
      model: { modelId: "text-embedding-3-small", dimensions: 768 },
    })
    expect(saves[0]).not.toHaveProperty("apiKey")
  })
})
