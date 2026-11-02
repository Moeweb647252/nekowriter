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
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import i18n from "i18next"
import { beforeAll, describe, expect, it, vi } from "vitest"

import enSettings from "@/i18n/locales/en/settings.json"
import type { ModelProfileDto } from "@/api/gen/ModelProfileDto"
import type { ProviderDto } from "@/api/gen/ProviderDto"
import type { RoleModels } from "@/api/gen/RoleModels"

const { patchMutate } = vi.hoisted(() => ({ patchMutate: vi.fn() }))

/** The stored role → model mapping; tests set it before rendering. */
let rolesDefaults: RoleModels

vi.mock("@/api/hooks/app-settings", () => ({
  useAppSettings: () => ({
    isPending: false,
    isError: false,
    data: { roles_defaults: rolesDefaults },
  }),
  usePatchAppSettings: () => ({ mutate: patchMutate, isPending: false }),
}))

vi.mock("@/api/hooks/providers", () => ({
  useModelProfiles: () => ({
    isPending: false,
    isError: false,
    data: [
      { id: 7, provider_id: 1, display_name: "Writer model" },
      { id: 8, provider_id: 2, display_name: "Review model" },
    ] satisfies Array<Pick<ModelProfileDto, "id" | "provider_id" | "display_name">>,
  }),
  useProviders: () => ({
    isPending: false,
    isError: false,
    data: [
      { id: 1, name: "Anthropic" },
      { id: 2, name: "DeepSeek" },
    ] satisfies Array<Pick<ProviderDto, "id" | "name">>,
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
    ]) {
      const trigger = screen.getByLabelText(t(`settings:role.${role}`))
      expect((trigger.textContent ?? "").trim(), role).not.toBe("")
    }
  })
})
