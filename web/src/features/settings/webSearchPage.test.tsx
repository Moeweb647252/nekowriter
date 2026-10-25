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

const { patchMutate } = vi.hoisted(() => ({ patchMutate: vi.fn() }))

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
    data: { search: { provider: "none", fetch_via: "direct", max_results: 6 } },
  }),
  usePatchAppSettings: () => ({ mutate: patchMutate, isPending: false }),
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

  it("shows the DeepSeek key field when DeepSeek is only the fetch backend", async () => {
    const user = userEvent.setup()
    render(<WebSearchPage />)

    await user.click(screen.getByRole("combobox", { name: "Provider" }))
    await user.click(await screen.findByRole("option", { name: "Tavily" }))
    await user.click(screen.getByRole("combobox", { name: "Page fetching" }))
    await user.click(await screen.findByRole("option", { name: "DeepSeek" }))

    // Tavily search + DeepSeek fetch is a supported combination, so both key fields are offered.
    expect(await screen.findByLabelText("DeepSeek API key")).toHaveAttribute(
      "placeholder",
      "sk-…",
    )
    expect(screen.getByLabelText("API key")).toBeInTheDocument()
  })

  it("offers both fetch backends and persists the selected one", async () => {
    const user = userEvent.setup()
    render(<WebSearchPage />)

    // The direct fetcher is the default, so the select starts there.
    const select = screen.getByRole("combobox", { name: "Page fetching" })
    expect(select).toHaveTextContent("Direct (built-in)")

    await user.click(select)
    await user.click(await screen.findByRole("option", { name: "DeepSeek" }))
    expect(select).toHaveTextContent("DeepSeek")

    // Saving sends the new backend; the server validates that DeepSeek has a key.
    await user.click(screen.getByRole("button", { name: "Save" }))
    expect(patchMutate).toHaveBeenCalledWith(
      expect.objectContaining({
        search: expect.objectContaining({ fetch_via: "deepseek" }),
      }),
      expect.anything(),
    )
  })
})
