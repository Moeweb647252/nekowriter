/**
 * The settings sidebar marks the section the URL shows (author's bug report: "the settings
 * list's active item never changes; clicking anything always shows Providers").
 *
 * The bug was invisible to every existing test: the page read `params.section`, but the route
 * table declares static children (`providers`, `models`, …), not a `:section` dynamic segment, so
 * `useParams()` was always empty and only a Tailwind class ever moved. This test mounts the page
 * in the real `/settings` routes and asserts `aria-current`, so the highlight is checked as
 * behaviour rather than as a CSS class.
 *
 * The lazily loaded section modules are replaced by an inert stub: they fetch through the query
 * client, which is not what this test is about. Their paths still come from `app/router.tsx`, so
 * the sidebar, the routes and the highlight cannot drift apart without failing here.
 */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import i18n from "i18next"
import { Outlet, RouterProvider, createMemoryRouter, type RouteObject } from "react-router"
import { beforeAll, describe, expect, it } from "vitest"

import enSettings from "@/i18n/locales/en/settings.json"
import { routes } from "@/app/router"
import SettingsPage, { SETTINGS_SECTION_KEYS } from "@/features/settings/SettingsPage"

beforeAll(() => {
  // The global test setup only ships a subset of the namespaces; this screen is in `settings:`.
  if (!i18n.hasResourceBundle("en", "settings")) {
    i18n.addResourceBundle("en", "settings", enSettings)
  }
  if (!i18n.options.ns?.includes("settings")) {
    i18n.options.ns = [...(i18n.options.ns as string[]), "settings"]
  }
})

/** An inert stand-in for one lazily loaded settings section. */
function SectionStub() {
  return <p>section body</p>
}

/** The real `/settings` route table with inert section modules, mounted under a plain shell. */
function settingsRoutes(): RouteObject[] {
  const root = routes.find((route) => route.path === "/")
  const settings = (root?.children ?? []).find((route) => route.path === "settings")
  if (settings === undefined) {
    throw new Error("the /settings route is missing from app/router.tsx")
  }
  const sections: RouteObject[] = (settings.children ?? []).map((child) =>
    child.index === true
      ? { index: true, element: <SectionStub /> }
      : { path: child.path, element: <SectionStub /> },
  )
  return [
    {
      path: "/",
      element: <Outlet />,
      children: [{ path: "settings", element: <SettingsPage />, children: sections }],
    },
  ]
}

/** Renders the settings shell at `pathname`, inside a memory router. */
function mountSettings(pathname: string) {
  const router = createMemoryRouter(settingsRoutes(), { initialEntries: [pathname] })
  render(<RouterProvider router={router} />)
}

/** The nav link of a section, found by its visible label (the icons are `aria-hidden`). */
function sectionLink(key: (typeof SETTINGS_SECTION_KEYS)[number]) {
  return screen.getByRole("link", { name: enSettings[key] })
}

describe("settings sidebar highlight", () => {
  it("marks the section the URL shows, and only that one", () => {
    mountSettings("/settings/roles")
    expect(sectionLink("roles")).toHaveAttribute("aria-current", "page")
    expect(sectionLink("providers")).not.toHaveAttribute("aria-current")
  })

  it("marks providers on the index route, which renders the providers page", () => {
    mountSettings("/settings")
    expect(sectionLink("providers")).toHaveAttribute("aria-current", "page")
    for (const key of SETTINGS_SECTION_KEYS) {
      if (key !== "providers") {
        expect(sectionLink(key)).not.toHaveAttribute("aria-current")
      }
    }
  })

  it("moves the highlight when the author clicks another section", async () => {
    const user = userEvent.setup()
    mountSettings("/settings/providers")

    await user.click(sectionLink("models"))

    expect(sectionLink("models")).toHaveAttribute("aria-current", "page")
    expect(sectionLink("providers")).not.toHaveAttribute("aria-current")
  })
})
