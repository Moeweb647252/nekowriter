import { screen } from "@testing-library/react"
import { HttpResponse, http } from "msw"
import { beforeEach, describe, expect, it, vi } from "vitest"

import { SettingsPage } from "./SettingsPage"
import type { HealthView } from "@/api/generated/HealthView"
import { LANGUAGE_STORAGE_KEY } from "@/i18n"
import { UI_LANGUAGE_NAMES } from "@/locales/languages"
import { zhCN } from "@/locales/zh-CN"
import { systemSettings } from "@/test/fixtures"
import { renderWithProviders } from "@/test/render"
import { server } from "@/test/server"

const health: HealthView = {
  appVersion: "0.1.0",
  dataDir: "/home/writer/.local/share/NekoWriter",
  logDir: "/home/writer/.local/share/NekoWriter/logs",
  uiEmbedded: true,
}

describe("SettingsPage", () => {
  beforeEach(() => {
    server.use(
      http.get("*/api/settings", () => HttpResponse.json(systemSettings())),
      http.get("*/api/settings/embedding", () =>
        HttpResponse.json({ apiBaseUrl: null, apiKeySet: false, model: null, revision: 0 }),
      ),
    )
  })

  it("shows where the installation keeps its data", async () => {
    server.use(http.get("*/api/health", () => HttpResponse.json(health)))
    renderWithProviders(<SettingsPage />)

    expect(await screen.findByText(health.dataDir)).toBeInTheDocument()
    expect(screen.getByText(health.logDir)).toBeInTheDocument()
  })

  it("switches the interface language and remembers the choice", async () => {
    server.use(http.get("*/api/health", () => HttpResponse.json(health)))
    const { user } = renderWithProviders(<SettingsPage />)
    expect(document.documentElement.lang).toBe("en")

    await user.click(screen.getByLabelText("Interface language"))
    await user.click(await screen.findByRole("option", { name: UI_LANGUAGE_NAMES["zh-CN"] }))

    expect(await screen.findByRole("heading", { name: zhCN.settings.title })).toBeInTheDocument()
    expect(document.documentElement.lang).toBe("zh-CN")
    expect(localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe("zh-CN")
  })

  it("asks for confirmation before exiting the service", async () => {
    let shutdowns = 0
    server.use(
      http.get("*/api/health", () => HttpResponse.json(health)),
      http.post("*/api/system/shutdown", () => {
        shutdowns += 1
        return HttpResponse.json({ status: "shutting_down" }, { status: 202 })
      }),
    )
    const { user } = renderWithProviders(<SettingsPage />)

    await user.click(screen.getByRole("button", { name: "Exit NekoWriter" }))
    expect(shutdowns).toBe(0)
    await user.click(await screen.findByRole("button", { name: "Exit" }))

    await vi.waitFor(() => {
      expect(shutdowns).toBe(1)
    })
  })
})
