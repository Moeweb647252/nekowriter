/**
 * The settings sidebar and the route table must agree (spec/11 §3): every section the nav lists
 * opens a page. The Web search page existed but was never routed, so `/settings/search` rendered
 * an empty shell — this test pins every nav section to a child route.
 */
import { describe, expect, it } from "vitest"

import { routes } from "@/app/router"
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
