import { screen, waitFor } from "@testing-library/react"
import { HttpResponse, http } from "msw"
import { describe, expect, it } from "vitest"

import { PromptsPage } from "./PromptsPage"
import type { PromptKey } from "@/api/generated/PromptKey"
import type { PromptRole } from "@/api/generated/PromptRole"
import type { PromptView } from "@/api/generated/PromptView"
import type { PromptsView } from "@/api/generated/PromptsView"
import type { SavePromptRequest } from "@/api/generated/SavePromptRequest"
import { projectView } from "@/test/fixtures"
import { renderWithProviders } from "@/test/render"
import { server } from "@/test/server"

function prompt(key: PromptKey, role: PromptRole, overrides: Partial<PromptView> = {}): PromptView {
  return {
    key,
    role,
    builtin: `Built-in ${key} ${role} for {{writing_language}}.`,
    system: null,
    project: null,
    variables: [{ variable: "writing_language", required: true }],
    ...overrides,
  }
}

const VIEW: PromptsView = {
  revision: 4,
  prompts: [
    prompt("preparation.generate", "system", { system: "Ours, in {{writing_language}}." }),
    prompt("preparation.generate", "user"),
  ],
}

function fakePrompts() {
  const saves: { path: string; body: SavePromptRequest }[] = []
  const renders: unknown[] = []
  server.use(
    http.get("*/api/projects", () => HttpResponse.json([projectView()])),
    http.get("*/api/prompts", () => HttpResponse.json(VIEW)),
    http.post("*/api/settings/prompts", async ({ request }) => {
      saves.push({
        path: new URL(request.url).pathname,
        body: (await request.json()) as SavePromptRequest,
      })
      return HttpResponse.json({ revision: 5 })
    }),
    http.post("*/api/templates/render", async ({ request }) => {
      renders.push(await request.json())
      return HttpResponse.json({ text: "Ours, in English." })
    }),
  )
  return { saves, renders }
}

describe("PromptsPage", () => {
  it("edits the system's override, previews it with examples and resets it", async () => {
    const { saves, renders } = fakePrompts()
    const { user } = renderWithProviders(<PromptsPage />)

    const text = await screen.findByLabelText("Prompt text")
    expect(text).toHaveValue("Ours, in {{writing_language}}.")
    expect(screen.getByText("In effect: the system's prompt.")).toBeInTheDocument()

    await user.click(screen.getByRole("button", { name: "Preview with examples" }))
    expect(await screen.findByText("Ours, in English.")).toBeInTheDocument()
    expect(renders[0]).toEqual({
      template: "Ours, in {{writing_language}}.",
      values: { writing_language: "English" },
    })

    await user.type(text, " Be brief.")
    await user.click(screen.getByRole("button", { name: "Save" }))
    await waitFor(() => {
      expect(saves).toHaveLength(1)
    })
    expect(saves[0]?.body).toMatchObject({
      expectedRevision: 4,
      key: "preparation.generate",
      role: "system",
      content: "Ours, in {{writing_language}}. Be brief.",
    })

    await user.click(screen.getByRole("button", { name: "Reset to the built-in text" }))
    await waitFor(() => {
      expect(saves).toHaveLength(2)
    })
    expect(saves[1]?.body.content).toBeNull()
  })
})
