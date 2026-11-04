/**
 * Settings › Agent roles shows which model each role runs on, and "Not set" for a role that falls
 * back to the global default (PROJECT §8.12, spec/10 §2.1 `roles.defaults`).
 *
 * The bug this test closes is a display one: the "no model" choice was `<SelectItem value="">`,
 * and Radix's `SelectValue` treats an empty string as "no value" — it renders its placeholder
 * branch. Here the trigger did pass a `placeholder`, so the role did not look blank, but the state
 * was styled and announced as hint text instead of as the selected value, and the empty item was
 * not a selectable value of its own. `BookSettingsPage` had already solved the same problem with a
 * sentinel (`GLOBAL_DEFAULT`); this page now uses the same rule (D-133).
 */
import { fireEvent, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import i18n from "i18next"
import { beforeAll, describe, expect, it, vi } from "vitest"

import enSettings from "@/i18n/locales/en/settings.json"
import type { ModelProfileDto } from "@/api/gen/ModelProfileDto"
import type { ProviderDto } from "@/api/gen/ProviderDto"
import type { RoleModels } from "@/api/gen/RoleModels"
import type { RoleEfforts } from "@/api/gen/RoleEfforts"
import { resolveRoleEffort } from "@/features/settings/roleModels"

const { patchMutate } = vi.hoisted(() => ({ patchMutate: vi.fn() }))

/** The stored role → model mapping; tests set it before rendering. */
let rolesDefaults: RoleModels

vi.mock("@/api/hooks/app-settings", () => ({
  useAppSettings: () => ({
    isPending: false,
    isError: false,
    data: {
      roles_defaults: rolesDefaults,
      tool_error_streak_limit: 5,
      research_max_tool_calls: 40,
      research_max_entries: 25,
    },
  }),
  usePatchAppSettings: () => ({ mutate: patchMutate, isPending: false }),
}))

vi.mock("@/api/hooks/providers", () => ({
  useModelProfiles: () => ({
    isPending: false,
    isError: false,
    data: [
      {
        id: 7,
        provider_id: 1,
        display_name: "Writer model",
        effort_levels: ["low", "high", "max"],
        default_effort: "high",
      },
      {
        id: 8,
        provider_id: 2,
        display_name: "Review model",
        effort_levels: ["low", "medium"],
        default_effort: "low",
      },
      {
        id: 10,
        provider_id: 1,
        display_name: "No default model",
        effort_levels: ["low"],
        default_effort: null,
      },
      {
        id: 11,
        provider_id: 1,
        display_name: "No levels model",
        effort_levels: [],
        default_effort: null,
      },
      {
        id: 12,
        provider_id: 1,
        display_name: "Protocol mismatch model",
        effort_levels: ["low", "ultra"],
        default_effort: "low",
      },
    ],
  }),
  useProviders: () => ({
    isPending: false,
    isError: false,
    data: [
      { id: 1, name: "Anthropic", protocol: "anthropic" },
      { id: 2, name: "DeepSeek", protocol: "openai" },
    ],
  }),
}))

vi.mock("@/components/common/ErrorToast", () => ({ useErrorToast: () => vi.fn() }))

import RolesPage from "./RolesPage"

beforeAll(() => {
  if (!i18n.hasResourceBundle("en", "settings")) {
    i18n.addResourceBundle("en", "settings", enSettings)
  }
  if (!i18n.options.ns?.includes("settings")) {
    i18n.options.ns = [...(i18n.options.ns as string[]), "settings"]
  }
  Element.prototype.scrollIntoView = vi.fn()
  Element.prototype.hasPointerCapture = () => false
  Element.prototype.setPointerCapture = vi.fn()
  Element.prototype.releasePointerCapture = vi.fn()
  rolesDefaults = { efforts: {} }
})

const t = (key: string) => i18n.t(key) as string

describe("RolesPage model pickers (PROJECT §8.12)", () => {
  it("renders 'Not set' as the selected value, not as a hint", () => {
    rolesDefaults = { efforts: {} }
    render(<RolesPage />)

    const trigger = screen.getByLabelText(t("settings:role.writer"))
    expect(trigger).toHaveTextContent(t("settings:role_unset"))
    // A real value is not styled as a placeholder: Radix marks an empty value with this attribute,
    // and a control that reads as "nothing chosen" when it means "the global default" is the same
    // class of defect as a blank one.
    expect(trigger).not.toHaveAttribute("data-placeholder")
  })

  it("shows the assigned model, provider-qualified", () => {
    rolesDefaults = { writer: 7, researcher: 8, efforts: {} }
    render(<RolesPage />)

    expect(screen.getByLabelText(t("settings:role.writer"))).toHaveTextContent(
      "Anthropic · Writer model",
    )
    expect(screen.getByLabelText(t("settings:role.researcher"))).toHaveTextContent(
      "DeepSeek · Review model",
    )
    // The roles left unassigned still say so rather than rendering blank.
    for (const role of ["conception", "planner", "reviewer", "archivist"]) {
      expect(screen.getByLabelText(t(`settings:role.${role}`))).toHaveTextContent(
        role === "planner" ? t("settings:role_planner_unset") : t("settings:role_unset"),
      )
    }
  })

  it("labels the planner's unset option as following the writer", () => {
    rolesDefaults = { efforts: {} }
    render(<RolesPage />)
    const trigger = screen.getByLabelText(t("settings:role.planner"))
    expect(trigger).toHaveTextContent(t("settings:role_planner_unset"))
  })

  it("saves the agent limits and states that 0 means unlimited", async () => {
    patchMutate.mockClear()
    rolesDefaults = { efforts: {} }
    const user = userEvent.setup()
    render(<RolesPage />)

    expect(screen.getByText(t("settings:tool_error_streak_limit_hint"))).toHaveTextContent(
      "0 means unlimited",
    )
    fireEvent.change(screen.getByLabelText(t("settings:tool_error_streak_limit")), {
      target: { value: "0" },
    })
    fireEvent.change(screen.getByLabelText(t("settings:research_max_tool_calls")), {
      target: { value: "3" },
    })
    fireEvent.change(screen.getByLabelText(t("settings:research_max_entries")), {
      target: { value: "4" },
    })
    await user.click(screen.getByRole("button", { name: t("settings:agent_limits_save") }))

    expect(patchMutate).toHaveBeenCalledTimes(1)
    expect(patchMutate.mock.calls[0][0]).toMatchObject({
      tool_error_streak_limit: 0,
      research_max_tool_calls: 3,
      research_max_entries: 4,
    })
  })

  it("writes null when a role is returned to 'Not set'", async () => {
    patchMutate.mockClear()
    rolesDefaults = { writer: 7, efforts: {} }
    const user = userEvent.setup()
    render(<RolesPage />)

    await user.click(screen.getByLabelText(t("settings:role.writer")))
    await user.click(await screen.findByRole("option", { name: t("settings:role_unset") }))

    expect(patchMutate).toHaveBeenCalledTimes(1)
    // `null`, not `""` and not the sentinel: the server expects a nullable role model.
    expect(patchMutate.mock.calls[0][0].roles_defaults.writer).toBeNull()
  })

  it("writes the model id when a role is assigned", async () => {
    patchMutate.mockClear()
    rolesDefaults = { efforts: {} }
    const user = userEvent.setup()
    render(<RolesPage />)

    await user.click(screen.getByLabelText(t("settings:role.conception")))
    await user.click(
      await screen.findByRole("option", { name: "Anthropic · Writer model" }),
    )

    expect(patchMutate).toHaveBeenCalledTimes(1)
    expect(patchMutate.mock.calls[0][0].roles_defaults.conception).toBe(7)
    // The sibling roles are preserved, not dropped by the patch.
    expect(patchMutate.mock.calls[0][0].roles_defaults.writer).toBeNull()
  })

  it("round-trips the stored per-role efforts when a model changes", async () => {
    patchMutate.mockClear()
    // The model PATCH replaces `roles_defaults` as a unit, so the loaded efforts must ride along.
    rolesDefaults = { writer: 7, efforts: { writer: "max", reviewer: "low" } }
    const user = userEvent.setup()
    render(<RolesPage />)

    await user.click(screen.getByLabelText(t("settings:role.writer")))
    await user.click(await screen.findByRole("option", { name: "DeepSeek · Review model" }))

    expect(patchMutate).toHaveBeenCalledTimes(1)
    expect(patchMutate.mock.calls[0][0].roles_defaults.efforts).toEqual({
      writer: "max",
      reviewer: "low",
    })
  })

  it("renders an effort select per role with the three unset labels", () => {
    rolesDefaults = { writer: 7, researcher: 10, efforts: {} }
    render(<RolesPage />)

    const writer = screen.getByTestId("role-writer")
    expect(within(writer).getAllByRole("combobox")[1]).toHaveTextContent(
      "Model default (high)",
    )
    const researcher = screen.getByTestId("role-researcher")
    expect(within(researcher).getAllByRole("combobox")[1]).toHaveTextContent(
      "Provider default",
    )
    const planner = screen.getByTestId("role-planner")
    expect(within(planner).getAllByRole("combobox")[1]).toHaveTextContent("Same as Writer")
  })

  it("disables the effort select with No effort levels when the model has no levels", () => {
    rolesDefaults = { writer: 11, efforts: {} }
    render(<RolesPage />)

    const effort = within(screen.getByTestId("role-writer")).getAllByRole("combobox")[1]
    expect(effort).toHaveTextContent("No effort levels")
    expect(effort).toBeDisabled()
  })

  it("marks a stored level that the resolved model does not list as stale", () => {
    rolesDefaults = { writer: 7, efforts: { writer: "ultra" } }
    render(<RolesPage />)

    const row = screen.getByTestId("role-writer")
    expect(within(row).getAllByRole("combobox")[1]).toHaveTextContent("ultra (not supported)")
    expect(row).toHaveTextContent("Sessions will use high instead.")
  })

  it("renders one stale item when the protocol cannot send a level still in effort_levels", () => {
    rolesDefaults = { writer: 12, efforts: { writer: "ultra" } }
    render(<RolesPage />)

    const trigger = within(screen.getByTestId("role-writer")).getAllByRole("combobox")[1]
    expect(trigger).toHaveTextContent("ultra (not supported)")
    expect(trigger).not.toHaveTextContent("ultra (not supported)ultra")
  })

  it("PATCHes the whole object when an effort is changed", async () => {
    patchMutate.mockClear()
    rolesDefaults = { writer: 7, researcher: 8, efforts: { writer: "low" } }
    const user = userEvent.setup()
    render(<RolesPage />)

    const effort = within(screen.getByTestId("role-writer")).getAllByRole("combobox")[1]
    await user.click(effort)
    await user.click(await screen.findByRole("option", { name: "max" }))

    expect(patchMutate).toHaveBeenCalledTimes(1)
    expect(patchMutate.mock.calls[0][0].roles_defaults).toEqual({
      conception: null,
      researcher: 8,
      planner: null,
      writer: 7,
      reviewer: null,
      archivist: null,
      assistant: null,
      efforts: { writer: "max" },
    })
  })

  it("clears both models and efforts from Clear all roles", async () => {
    patchMutate.mockClear()
    rolesDefaults = { writer: 7, researcher: 8, efforts: { writer: "max", reviewer: "low" } }
    const user = userEvent.setup()
    render(<RolesPage />)

    await user.click(screen.getByRole("button", { name: t("settings:roles_clear_all") }))

    expect(patchMutate).toHaveBeenCalledTimes(1)
    expect(patchMutate.mock.calls[0][0].roles_defaults).toEqual({
      conception: null,
      researcher: null,
      planner: null,
      writer: null,
      reviewer: null,
      archivist: null,
      assistant: null,
      efforts: {},
    })
  })

  it("never renders a blank role trigger", () => {
    rolesDefaults = { conception: 7, researcher: null, writer: 8, reviewer: null, archivist: 7, efforts: {} }
    render(<RolesPage />)

    for (const role of [
      "conception",
      "researcher",
      "planner",
      "writer",
      "reviewer",
      "archivist",
      "assistant",
    ]) {
      const trigger = screen.getByLabelText(t(`settings:role.${role}`))
      expect((trigger.textContent ?? "").trim(), role).not.toBe("")
    }
  })
})

describe("resolveRoleEffort mirrors the §4.4 cases", () => {
  const profile = (
    id: number,
    levels: string[],
    defaultEffort: string | null,
  ): ModelProfileDto =>
    ({
      id,
      provider_id: 1,
      display_name: `Model ${id}`,
      model: `model-${id}`,
      context_window: 1000,
      max_output_tokens: 100,
      options: {},
      effort_levels: levels,
      default_effort: defaultEffort,
      ignored_options: [],
      ignored_effort_levels: [],
      created_at: "2026-01-01T00:00:00Z",
      updated_at: "2026-01-01T00:00:00Z",
    }) as ModelProfileDto
  const models = [
    profile(1, ["low", "medium", "high", "max"], "high"),
    profile(2, ["low", "medium"], "medium"),
    profile(3, ["low"], null),
  ]
  const providers = [
    { id: 1, protocol: "anthropic" },
  ] as unknown as ProviderDto[]
  const roles = (model: Partial<RoleModels>, efforts: RoleEfforts = {}): RoleModels => ({
    ...model,
    efforts,
  })
  const resolve = (role: "writer" | "planner", book: RoleModels | null, global: RoleModels) =>
    resolveRoleEffort({ role, book, global, models, providers })

  it("1: uses the model default when no candidate is set", () => {
    expect(resolve("writer", null, roles({ writer: 1 })).effectiveEffort).toBe("high")
  })

  it("2: uses the global role effort", () => {
    expect(
      resolve("writer", null, roles({ writer: 1 }, { writer: "low" })).effectiveEffort,
    ).toBe("low")
  })

  it("3: a book effort beats the global one", () => {
    expect(
      resolve(
        "writer",
        roles({ writer: 1 }, { writer: "max" }),
        roles({ writer: 1 }, { writer: "low" }),
      ).effectiveEffort,
    ).toBe("max")
  })

  it("4: an unsupported book level falls through to the global level", () => {
    const result = resolve(
      "writer",
      roles({ writer: 3 }, { writer: "max" }),
      roles({ writer: 3 }, { writer: "low" }),
    )
    expect(result.effectiveEffort).toBe("low")
    expect(result.stale).toBe(true)
  })

  it("5: returns no effort when every candidate is unsupported and there is no default", () => {
    const result = resolve(
      "writer",
      roles({ writer: 3 }, { writer: "ultra" }),
      roles({ writer: 3 }, { writer: "max" }),
    )
    expect(result.effectiveEffort).toBeNull()
    expect(result.stale).toBe(true)
  })

  it("6: a planner following the writer's model follows the writer's effort", () => {
    const result = resolve("planner", null, roles({ writer: 1 }, { writer: "low" }))
    expect(result.modelFromWriter).toBe(true)
    expect(result.effectiveEffort).toBe("low")
  })

  it("7: a planner on its own model does not inherit the writer's effort", () => {
    const result = resolve(
      "planner",
      null,
      roles({ planner: 2, writer: 1 }, { writer: "low" }),
    )
    expect(result.modelFromWriter).toBe(false)
    expect(result.effectiveEffort).toBe("medium")
  })

  it("8: a planner following the writer's model uses its own effort first", () => {
    const result = resolve(
      "planner",
      null,
      roles({ writer: 1 }, { planner: "high", writer: "low" }),
    )
    expect(result.modelFromWriter).toBe(true)
    expect(result.effectiveEffort).toBe("high")
  })
})
