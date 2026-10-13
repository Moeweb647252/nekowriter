import { screen, waitFor } from "@testing-library/react"
import { HttpResponse, http } from "msw"
import { describe, expect, it } from "vitest"

import { ModelSettingsCard } from "./ModelSettingsCard"
import type { SaveSettingsRequest } from "@/api/generated/SaveSettingsRequest"
import type { TestModelRequest } from "@/api/generated/TestModelRequest"
import { systemSettings } from "@/test/fixtures"
import { renderWithProviders } from "@/test/render"
import { server } from "@/test/server"

describe("ModelSettingsCard", () => {
  it("keeps the stored key unless a new one is entered, and saves the default model", async () => {
    let settings = systemSettings({
      apiBaseUrl: "https://api.example.com/v1/",
      apiKeySet: true,
      revision: 2,
    })
    const saves: SaveSettingsRequest[] = []
    server.use(
      http.get("*/api/settings", () => HttpResponse.json(settings)),
      http.post("*/api/settings", async ({ request }) => {
        const body = (await request.json()) as SaveSettingsRequest
        saves.push(body)
        settings = { ...settings, defaultModel: body.defaultModel, revision: 3 }
        return HttpResponse.json(settings)
      }),
    )
    const { user } = renderWithProviders(<ModelSettingsCard />)

    expect(
      await screen.findByText("A key is stored. Enter a new one to replace it."),
    ).toBeInTheDocument()
    expect(screen.getByLabelText("API key")).toHaveValue("")
    await user.click(screen.getByRole("switch", { name: "Set a default model" }))
    await user.type(screen.getByLabelText("Model ID"), "writer")
    await user.click(screen.getByRole("button", { name: "Save" }))

    await waitFor(() => {
      expect(saves).toHaveLength(1)
    })
    expect(saves[0]).not.toHaveProperty("apiKey")
    expect(saves[0]).toMatchObject({
      expectedRevision: 2,
      apiBaseUrl: "https://api.example.com/v1/",
      defaultModel: {
        modelId: "writer",
        contextWindow: 128_000,
        maxOutputTokens: 8_192,
        supportsThinking: false,
        reportsStreamUsage: true,
      },
    })
    expect(await screen.findByText("Saved the model settings.")).toBeInTheDocument()
  })

  it("tests the model as entered and reports the outcome", async () => {
    const tests: TestModelRequest[] = []
    server.use(
      http.get("*/api/settings", () => HttpResponse.json(systemSettings())),
      http.post("*/api/settings/test-model", async ({ request }) => {
        const body = (await request.json()) as TestModelRequest
        tests.push(body)
        if (tests.length === 1) {
          return HttpResponse.json(
            {
              code: "MODEL_SERVICE_FAILED",
              params: { failure: { kind: "http_status", status: 401 } },
              requestId: "r1",
            },
            { status: 502 },
          )
        }
        return HttpResponse.json({ elapsedMillis: 1234, usageReported: true })
      }),
    )
    const { user } = renderWithProviders(<ModelSettingsCard />)

    await user.type(await screen.findByLabelText("API root"), "http://127.0.0.1:8000/v1")
    await user.type(screen.getByLabelText("API key"), "sk-test")
    await user.click(screen.getByRole("switch", { name: "Set a default model" }))
    await user.type(screen.getByLabelText("Model ID"), "writer")
    await user.click(screen.getByRole("button", { name: "Test connection" }))
    expect(
      await screen.findByText("The test failed: the service answered with HTTP status 401."),
    ).toBeInTheDocument()

    await user.click(screen.getByRole("button", { name: "Test connection" }))
    expect(await screen.findByText("Connected. The test answer took 1.2 s.")).toBeInTheDocument()
    expect(tests[0]).toMatchObject({
      apiBaseUrl: "http://127.0.0.1:8000/v1",
      apiKey: "sk-test",
      model: { modelId: "writer" },
    })
  })
})
