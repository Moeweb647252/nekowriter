/**
 * The settings sidebar and the route table must agree (spec/11 §3): every section the nav lists
 * opens a page. The Web search page existed but was never routed, so `/settings/search` rendered
 * an empty shell — this test pins every nav section to a child route.
 */
import { render } from "@testing-library/react"
import { I18nextProvider } from "react-i18next"
import { describe, expect, it } from "vitest"

import i18n from "@/i18n"
import { RouteError, routes } from "@/app/router"
import { SETTINGS_SECTION_KEYS } from "@/features/settings/SettingsPage"

/** The child routes of `/settings`, by path. */
function settingsChildPaths(): Set<string | undefined> {
  const root = routes.find((route) => route.path === "/")
  expect(root, "the root route").toBeDefined()
  const settings = (root?.children ?? []).find((route) => route.path === "settings")
  expect(settings, "the /settings route").toBeDefined()
  return new Set((settings?.children ?? []).map((route) => route.path))
}

describe("settings routing", () => {
  it("routes every settings section the sidebar lists", () => {
    const paths = settingsChildPaths()
    for (const key of SETTINGS_SECTION_KEYS) {
      expect(paths.has(key), `no /settings/${key} route for the "${key}" nav item`).toBe(true)
    }
  })

  it("routes the web search page (spec/07 §6)", () => {
    expect(settingsChildPaths().has("search")).toBe(true)
  })
})

describe("the router error boundary", () => {
  it("renders localized text, not hardcoded English (spec/11 §6, §9)", async () => {
    // The boundary is the only `errorElement` in the app, so a failed lazy-route load is the one
    // screen an author can hit before any page component mounts. A hardcoded English string here
    // showed up in zh-CN, which no other route could compensate for.
    //
    // The language is switched **before** mounting and switched back **after** unmounting: a
    // `changeLanguage` while the component is mounted makes it re-render outside `act()`, which is
    // a warning that hides real ones.
    const previous = i18n.language
    await i18n.changeLanguage("zh-CN")
    const { unmount } = render(
      <I18nextProvider i18n={i18n}>
        <RouteError />
      </I18nextProvider>,
    )
    const zh = i18n.getFixedT("zh-CN")
    expect(document.body.textContent).toContain(zh("common:error_title"))
    expect(document.body.textContent).toContain(zh("common:retry"))
    expect(document.body.textContent).not.toContain("Something went wrong")
    unmount()
    await i18n.changeLanguage(previous)
  })
})
