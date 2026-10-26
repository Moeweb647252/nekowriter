/**
 * Settings › Models offers only the options the selected provider's protocol can send (spec/04 §5,
 * D-132).
 *
 * The bug this replaces: the form rendered a flat list of every option for every model, so the
 * author picked the Anthropic `effort` for an OpenAI-protocol profile. The OpenAI adapter sends only
 * `reasoning_effort`, the request carried no reasoning parameter at all, and the turn still
 * succeeded — a silent no-op the UI made easy to create and impossible to notice.
 *
 * The rule is served by the API (`GET /api/provider-presets/options`) rather than hard-coded in
 * TypeScript, so these tests fix the *behaviour*: what the form shows for a protocol, what it does
 * when the protocol changes, and how it reports a profile that is already mismatched.
 */
import { render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import i18n from "i18next"
import { beforeAll, describe, expect, it, vi } from "vitest"

import enSettings from "@/i18n/locales/en/settings.json"
import zhSettings from "@/i18n/locales/zh-CN/settings.json"

import ModelsPage from "./ModelsPage"
import {
  OPTION_KEYS,
  clearUnreachableOptions,
  optionsForProtocol,
  providerProtocol,
  removeOptionKeys,
  unreachableOptionKeys,
} from "./modelOptions"
import type { ModelOptionSpecDto } from "@/api/gen/ModelOptionSpecDto"
import type { ModelProfileDto } from "@/api/gen/ModelProfileDto"
import type { ModelProfileInput } from "@/api/gen/ModelProfileInput"
import type { ProviderDto } from "@/api/gen/ProviderDto"

beforeAll(() => {
  // jsdom does not implement scrolling, and Radix's SelectItem calls it when the list opens.
  Element.prototype.scrollIntoView = vi.fn()
  // Radix's Select trigger guards its pointer-down handler with these jsdom gaps.
  Element.prototype.hasPointerCapture = () => false
  Element.prototype.setPointerCapture = vi.fn()
  Element.prototype.releasePointerCapture = vi.fn()
  if (!i18n.hasResourceBundle("en", "settings")) {
    i18n.addResourceBundle("en", "settings", enSettings)
  }
  if (!i18n.hasResourceBundle("zh-CN", "settings")) {
    i18n.addResourceBundle("zh-CN", "settings", zhSettings)
  }
  if (!i18n.options.ns?.includes("settings")) {
    i18n.options.ns = [...(i18n.options.ns as string[]), "settings"]
  }
})

// ───────────────────────────── the served table ─────────────────────────────

/** The option table exactly as the server publishes it (pinned by the Rust tests). */
const SPECS: ModelOptionSpecDto[] = [
  { key: "reasoning_replay", protocols: ["openai", "anthropic"], kind: "choice", values: ["never", "always"], default_value: "never" },
  { key: "cache_ttl", protocols: ["openai", "anthropic"], kind: "choice", values: ["auto", "5m", "1h"], default_value: "auto" },
  { key: "reasoning_effort", protocols: ["openai"], kind: "text", values: [], default_value: null },
  { key: "max_tokens_field", protocols: ["openai"], kind: "choice", values: ["max_tokens", "max_completion_tokens"], default_value: "max_tokens" },
  { key: "send_parallel_tool_calls", protocols: ["openai"], kind: "boolean", values: [], default_value: null },
  { key: "stream_usage", protocols: ["openai"], kind: "boolean", values: [], default_value: null },
  { key: "temperature", protocols: ["openai"], kind: "number", values: [], default_value: null },
  { key: "extra_body", protocols: ["openai"], kind: "json", values: [], default_value: null },
  { key: "effort", protocols: ["anthropic"], kind: "choice", values: ["low", "medium", "high", "xhigh", "max"], default_value: null },
  { key: "thinking_display", protocols: ["anthropic"], kind: "choice", values: ["summarized", "omitted"], default_value: null },
  { key: "refusal_fallback", protocols: ["anthropic"], kind: "boolean", values: [], default_value: null },
  { key: "eager_input_streaming", protocols: ["anthropic"], kind: "boolean", values: [], default_value: null },
]

const PROVIDERS: ProviderDto[] = [
  {
    id: 1,
    name: "Anthropic",
    protocol: "anthropic",
    base_url: "https://api.anthropic.com",
    has_api_key: true,
    extra_headers: {},
    models: [],
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
  },
  {
    id: 2,
    name: "Magpie",
    protocol: "openai",
    base_url: "http://127.0.0.1:3425/v1",
    has_api_key: true,
    extra_headers: {},
    models: [],
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
  },
]

/** The author's profile #4 exactly as their database stores it. */
const MISMATCHED: ModelProfileDto = {
  id: 4,
  provider_id: 2,
  display_name: "DeepSeek V4 Pro",
  model: "deepseek-v4-pro",
  context_window: 200_000,
  max_output_tokens: 16_384,
  options: {
    reasoning_replay: "always",
    max_tokens_field: "max_tokens",
    effort: "high",
    thinking_display: "omitted",
    cache_ttl: "1h",
  },
  ignored_options: ["effort", "thinking_display"],
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
}

const PATCHED: ModelProfileInput[] = []
const mockHooks = vi.hoisted(() => ({
  models: [] as unknown[],
  providers: [] as unknown[],
}))

vi.mock("@/api/hooks/providers", () => ({
  useModelProfiles: () => ({ isPending: false, isError: false, data: mockHooks.models }),
  useProviders: () => ({ isPending: false, isError: false, data: mockHooks.providers }),
  useOptionSpecs: () => ({ isPending: false, isError: false, data: SPECS }),
  useCreateModelProfile: () => ({ mutate: vi.fn(), isPending: false }),
  usePatchModelProfile: () => ({
    mutate: (
      variables: { id: number; input: ModelProfileInput },
      options?: { onSuccess?: () => void },
    ) => {
      PATCHED.push(variables.input)
      options?.onSuccess?.()
    },
    isPending: false,
  }),
  useDeleteModelProfile: () => ({ mutate: vi.fn(), isPending: false }),
  useRemoteModels: () => ({ mutate: vi.fn(), isPending: false }),
}))
vi.mock("@/components/common/ErrorToast", () => ({ useErrorToast: () => vi.fn() }))
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() } }))

function setData(models: ModelProfileDto[], providers: ProviderDto[]) {
  mockHooks.models = models
  mockHooks.providers = providers
}

const t = (key: string) => i18n.t(key) as string

// ───────────────────────────── pure helpers ─────────────────────────────

describe("model options by protocol (D-132)", () => {
  it("classifies every option by the protocol that can send it", () => {
    const openai = optionsForProtocol(SPECS, "openai").map((spec) => spec.key)
    const anthropic = optionsForProtocol(SPECS, "anthropic").map((spec) => spec.key)
    // The whole point: the two reasoning controls never appear together.
    expect(openai).toContain("reasoning_effort")
    expect(openai).not.toContain("effort")
    expect(anthropic).toContain("effort")
    expect(anthropic).not.toContain("reasoning_effort")
    // Both adapters read these.
    for (const key of ["reasoning_replay", "cache_ttl"]) {
      expect(openai).toContain(key)
      expect(anthropic).toContain(key)
    }
    // No protocol offers an option it cannot send, and none is dropped from the table.
    const union = new Set([...openai, ...anthropic])
    for (const key of union) {
      expect(SPECS.some((spec) => spec.key === key)).toBe(true)
    }
    expect(openai.length + anthropic.length).toBeGreaterThan(union.size)
  })

  it("shows nothing while no provider is selected", () => {
    expect(optionsForProtocol(SPECS, null)).toEqual([])
  })

  it("offers no option for the demo (mock) protocol", () => {
    // The Demo provider sends no request at all, so every option would be inert there.
    expect(optionsForProtocol(SPECS, "mock")).toEqual([])
    expect(unreachableOptionKeys({ reasoning_effort: "high" }, SPECS, "mock")).toEqual([
      "reasoning_effort",
    ])
  })

  it("reads the protocol from the provider row, not from the option", () => {
    expect(providerProtocol(PROVIDERS, 2)).toBe("openai")
    expect(providerProtocol(PROVIDERS, "1")).toBe("anthropic")
    expect(providerProtocol(PROVIDERS, 99)).toBeNull()
    expect(providerProtocol(PROVIDERS, "not-a-number")).toBeNull()
  })

  it("names the options a profile stores but its protocol cannot send", () => {
    expect(unreachableOptionKeys(MISMATCHED.options, SPECS, "openai")).toEqual([
      "effort",
      "thinking_display",
    ])
    // The same options are fine on Anthropic.
    expect(unreachableOptionKeys(MISMATCHED.options, SPECS, "anthropic")).toEqual([])
    // Defaults and empty values are not "set", so they are never reported.
    expect(
      unreachableOptionKeys({ extra_body: {}, temperature: null }, SPECS, "anthropic"),
    ).toEqual([])
  })

  it("clears the stranded options when the protocol changes, and reports them", () => {
    const { options, removed } = clearUnreachableOptions(MISMATCHED.options, SPECS, "openai")
    expect(removed).toEqual(["effort", "thinking_display"])
    expect(options.effort).toBeUndefined()
    expect(options.thinking_display).toBeUndefined()
    // Everything the new protocol can send survives untouched.
    expect(options.reasoning_replay).toBe("always")
    expect(options.cache_ttl).toBe("1h")

    // Anthropic → OpenAI strands the OpenAI-only fields; OpenAI → Anthropic the other way round.
    const { removed: openaiRemoved } = clearUnreachableOptions(
      { reasoning_effort: "high", temperature: 0.5 },
      SPECS,
      "anthropic",
    )
    expect(openaiRemoved).toEqual(["reasoning_effort", "temperature"])
    // A no-op change reports nothing.
    expect(clearUnreachableOptions({}, SPECS, "openai").removed).toEqual([])
  })

  it("removes only known option keys", () => {
    const cleaned = removeOptionKeys(MISMATCHED.options, ["effort", "thinking_display", "bogus"])
    expect(cleaned.effort).toBeUndefined()
    expect(cleaned.thinking_display).toBeUndefined()
    expect(cleaned.reasoning_replay).toBe("always")
  })

  it("covers every option key the DTO can carry", () => {
    // A new option in `ModelOptions` must appear in the table, or the form would never render it.
    const served = new Set(SPECS.map((spec) => spec.key))
    for (const key of OPTION_KEYS) {
      expect(served).toContain(key)
    }
    expect(SPECS.map((spec) => spec.key).sort()).toEqual([...OPTION_KEYS].sort())
  })
})

// ───────────────────────────── the page ─────────────────────────────

describe("ModelsPage protocol handling", () => {
  it("offers only the options the provider's protocol can send", async () => {
    setData([], PROVIDERS)
    const user = userEvent.setup()
    render(<ModelsPage />)
    await user.click(screen.getByRole("button", { name: "Add model" }))

    const dialog = await screen.findByRole("dialog")
    // The default provider is the first row (Anthropic): the Anthropic control is there.
    expect(within(dialog).getByLabelText(t("settings:option.effort"))).toBeInTheDocument()
    // The OpenAI control is not, because no OpenAI protocol receives it.
    expect(
      within(dialog).queryByLabelText(t("settings:option.reasoning_effort")),
    ).not.toBeInTheDocument()
    // Free-form fields are not rendered as bare identifiers.
    expect(within(dialog).queryByText("reasoning_replay")).not.toBeInTheDocument()

    // Switching the provider to the OpenAI-protocol row swaps the option set.
    await user.click(within(dialog).getByRole("combobox", { name: /Provider|提供商/ }))
    await user.click(await screen.findByRole("option", { name: /Magpie/ }))
    expect(
      within(dialog).getByLabelText(t("settings:option.reasoning_effort")),
    ).toBeInTheDocument()
    expect(within(dialog).queryByLabelText(t("settings:option.effort"))).not.toBeInTheDocument()
  })

  it("says what a provider switch removed instead of dropping it silently", async () => {
    setData([], PROVIDERS)
    const user = userEvent.setup()
    render(<ModelsPage />)
    await user.click(screen.getByRole("button", { name: "Add model" }))
    const dialog = await screen.findByRole("dialog")

    // Configure an Anthropic-only option, then switch to the OpenAI provider.
    const effort = within(dialog).getByLabelText(t("settings:option.effort"))
    await user.click(effort)
    await user.click(await screen.findByRole("option", { name: t("settings:value.effort.high") }))
    await user.click(within(dialog).getByRole("combobox", { name: /Provider|提供商/ }))
    await user.click(await screen.findByRole("option", { name: /Magpie/ }))

    const notice = await within(dialog).findByTestId("cleared-options-alert")
    expect(notice).toHaveTextContent(t("settings:option_cleared_title"))
    expect(notice).toHaveTextContent(t("settings:option.effort"))
  })

  it("reports an already-stored mismatch and fixes it in one action", async () => {
    PATCHED.length = 0
    setData([MISMATCHED], PROVIDERS)
    const user = userEvent.setup()
    render(<ModelsPage />)

    // The table flags it instead of rendering a profile that looks fine.
    const badge = await screen.findByTestId("ignored-options-4")
    expect(badge).toHaveTextContent(t("settings:option.effort"))
    expect(badge).toHaveTextContent(t("settings:option.thinking_display"))

    // The badge opens the repair confirmation, which names the fields and the protocol.
    await user.click(badge)
    const confirm = await screen.findByRole("alertdialog")
    expect(confirm).toHaveTextContent(t("settings:option_mismatch"))
    expect(confirm).toHaveTextContent(t("settings:protocol.openai"))
    await user.click(within(confirm).getByRole("button", { name: t("settings:option_mismatch_fix") }))

    // One PATCH: the inert options are gone, everything the protocol can send is kept.
    expect(PATCHED).toHaveLength(1)
    const sent = PATCHED[0]
    expect(sent.options.effort).toBeUndefined()
    expect(sent.options.thinking_display).toBeUndefined()
    expect(sent.options.reasoning_replay).toBe("always")
    expect(sent.options.cache_ttl).toBe("1h")
    expect(sent.provider_id).toBe(2)
    expect(sent.model).toBe("deepseek-v4-pro")
  })

  it("shows the mismatch inside the edit dialog with a one-click removal", async () => {
    PATCHED.length = 0
    setData([MISMATCHED], PROVIDERS)
    const user = userEvent.setup()
    render(<ModelsPage />)
    await user.click(await screen.findByRole("button", { name: "Edit" }))
    const dialog = await screen.findByRole("dialog")

    const alert = await within(dialog).findByTestId("ignored-options-alert")
    expect(alert).toHaveTextContent(t("settings:option.effort"))
    // The dialog does not offer the field, because the protocol cannot send it.
    expect(within(dialog).queryByLabelText(t("settings:option.effort"))).not.toBeInTheDocument()

    await user.click(
      within(alert).getByRole("button", { name: t("settings:option_mismatch_fix") }),
    )
    await user.click(within(dialog).getByRole("button", { name: t("common:save") }))
    expect(PATCHED).toHaveLength(1)
    expect(PATCHED[0].options.effort).toBeUndefined()
    expect(PATCHED[0].options.reasoning_replay).toBe("always")
  })

  it("labels every option in both locales, with a hint that explains it", async () => {
    setData([], PROVIDERS)
    const user = userEvent.setup()
    render(<ModelsPage />)
    await user.click(screen.getByRole("button", { name: "Add model" }))
    const dialog = await screen.findByRole("dialog")
    for (const spec of SPECS) {
      // Both locales carry a real label and a non-empty hint: a bare identifier is not a label.
      for (const bundle of [enSettings, zhSettings] as Array<Record<string, string>>) {
        expect(bundle[`option.${spec.key}`], spec.key).toBeTypeOf("string")
        expect((bundle[`option.${spec.key}`] ?? "").length, spec.key).toBeGreaterThan(0)
        expect(bundle[`option.${spec.key}_hint`], spec.key).toBeTypeOf("string")
        expect((bundle[`option.${spec.key}_hint`] ?? "").length, spec.key).toBeGreaterThan(0)
        // A label is prose, not the JSON field name.
        expect(bundle[`option.${spec.key}`]).not.toBe(spec.key)
      }
      // Every closed value the table publishes has a label in both locales.
      for (const value of spec.values) {
        for (const bundle of [enSettings, zhSettings] as Array<Record<string, string>>) {
          expect(bundle[`value.${spec.key}.${value}`], `${spec.key}=${value}`).toBeTypeOf(
            "string",
          )
        }
      }
    }
    // The Anthropic-only options carry the Anthropic provider context.
    expect(within(dialog).getByText(t("settings:option.effort_hint"))).toBeInTheDocument()
  })
})
