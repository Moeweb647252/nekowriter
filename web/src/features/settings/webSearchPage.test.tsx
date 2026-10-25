/**
 * Settings › Web search offers DeepSeek as a search provider and shows its own key field
 * (spec/07 §6, spec/02 §4.1, D-127).
 *
 * The field matters not only visually: the save path must send `deepseek_api_key` only when
 * DeepSeek is the selected provider, which is why the page keeps one draft per secret. This test
 * checks the provider is offered and that selecting it reveals the key input; the masking
 * round-trip itself is covered by `web_search_providers.rs`.
 */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import i18n from "i18next"
import { beforeAll, describe, expect, it, vi } from "vitest"

import enSettings from "@/i18n/locales/en/settings.json"

import WebSearchPage from "./WebSearchPage"

beforeAll(() => {
  // The shared test setup only ships a subset of the namespaces; this page is in `settings:`.
  if (!i18n.hasResourceBundle("en", "settings")) {
    i18n.addResourceBundle("en", "settings", enSettings)
  }
  if (!i18n.options.ns?.includes("settings")) {
    i18n.options.ns = [...(i18n.options.ns as string[]), "settings"]
  }
  // jsdom does not implement scrolling; Radix's SelectItem calls it when the list opens.
  Element.prototype.scrollIntoView = vi.fn()
  // Radix's Select trigger guards its pointer-down handler with these jsdom gaps.
  Element.prototype.hasPointerCapture = () => false
  Element.prototype.setPointerCapture = vi.fn()
  Element.prototype.releasePointerCapture = vi.fn()
})

vi.mock("@/api/hooks/app-settings", () => ({
  useAppSettings: () => ({
    isPending: false,
    data: { search: { provider: "none", max_results: 6 } },
  }),
  usePatchAppSettings: () => ({ mutate: vi.fn(), isPending: false }),
  useTestSearch: () => ({ mutate: vi.fn(), isPending: false }),
}))

vi.mock("@/components/common/ErrorToast", () => ({ useErrorToast: () => vi.fn() }))

describe("WebSearchPage", () => {
  it("offers DeepSeek and shows its key field when selected", async () => {
    const user = userEvent.setup()
    render(<WebSearchPage />)

    // No key field while no provider is selected.
    expect(screen.queryByLabelText("API key")).not.toBeInTheDocument()

    await user.click(screen.getByRole("combobox", { name: "Provider" }))
    await user.click(await screen.findByRole("option", { name: "DeepSeek" }))

    const key = await screen.findByLabelText("API key")
    expect(key).toHaveAttribute("placeholder", "sk-…")
  })
})
