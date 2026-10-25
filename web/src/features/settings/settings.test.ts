/** Pure helpers of the prompts and settings screens (spec/11 §9: unit tests for UI logic). */
import { describe, expect, it } from "vitest"

import { parseHeaders } from "@/features/settings/ProvidersPage"
import { SETTINGS_SECTION_KEYS, settingsSection } from "@/features/settings/SettingsPage"
import { groupPrompts } from "@/features/prompts/PromptEditor"
import enPrompts from "@/i18n/locales/en/prompts.json"
import zhPrompts from "@/i18n/locales/zh-CN/prompts.json"
import enSettings from "@/i18n/locales/en/settings.json"
import zhSettings from "@/i18n/locales/zh-CN/settings.json"
import type { PromptEntryDto } from "@/api/gen/PromptEntryDto"
import type { PromptKind } from "@/api/gen/PromptKind"
import type { Protocol } from "@/api/gen/Protocol"

/** The prompt groups the editor builds, mirroring `GROUP_ORDER` (spec/09 §2). */
const GROUPS: Array<{ key: string; kinds: PromptKind[] }> = [
  { key: "system", kinds: ["system"] },
  { key: "tasks", kinds: ["task"] },
  { key: "blocks", kinds: ["block"] },
  { key: "notices", kinds: ["notice"] },
  { key: "results", kinds: ["result"] },
  { key: "shared", kinds: ["shared"] },
  { key: "tools", kinds: ["tool"] },
]

const PROTOCOLS: Protocol[] = ["openai", "anthropic", "mock"]

describe("provider form", () => {
  it("parses one header per line", () => {
    expect(parseHeaders("x-a: 1\nx-b: two words")).toEqual({
      "x-a": "1",
      "x-b": "two words",
    })
  })

  it("ignores blank lines and trims", () => {
    expect(parseHeaders("\n  x-a :  1 \n\n")).toEqual({ "x-a": "1" })
    expect(parseHeaders("")).toEqual({})
  })

  it("rejects a line without a name or a colon", () => {
    expect(parseHeaders("nocolon")).toBeNull()
    expect(parseHeaders(": value")).toBeNull()
  })
})

describe("settings section from the URL", () => {
  // The highlight used to come from `useParams()`, but the settings children are static paths
  // (`app/router.tsx`), so there is no `:section` param: the sidebar stayed on Providers while the
  // body changed. The active section is derived from the pathname instead.
  it("maps every section the sidebar lists to its own URL", () => {
    for (const key of SETTINGS_SECTION_KEYS) {
      expect(settingsSection(`/settings/${key}`), `/settings/${key}`).toBe(key)
    }
  })

  it("agrees with the index route: /settings renders providers", () => {
    expect(settingsSection("/settings")).toBe("providers")
    expect(settingsSection("/settings/")).toBe("providers")
  })

  it("falls back to providers for a segment no route declares", () => {
    expect(settingsSection("/settings/nope")).toBe("providers")
    // The function is total: a path outside `/settings` never names a section, even when its
    // second segment happens to look like one.
    expect(settingsSection("/elsewhere/models")).toBe("providers")
  })

  it("ignores a trailing slash", () => {
    expect(settingsSection("/settings/models/")).toBe("models")
  })

  it("keeps the first known segment of a deeper path", () => {
    expect(settingsSection("/settings/models/extra")).toBe("models")
  })

  it("handles every key in SETTINGS_SECTION_KEYS", () => {
    // A section added to the sidebar without a mapping must fail here instead of silently
    // highlighting Providers (the original bug).
    const handled = SETTINGS_SECTION_KEYS.every((key) => settingsSection(`/settings/${key}`) === key)
    expect(handled).toBe(true)
  })
})

describe("prompts list grouping", () => {
  /** A minimal entry; only the fields `groupPrompts` reads matter. */
  const entry = (agent: string, kind: PromptKind, key: string): PromptEntryDto => ({
    key,
    kind,
    agent,
    title_en: key,
    title_zh: "",
    vars: [],
    default_text: "",
    source: "default",
    variables: [],
  })

  // PROJECT §8.11 / F8.1: "the left list is grouped by agent **and** category".
  it("groups by agent first, then by category", () => {
    const groups = groupPrompts([
      entry("writer", "system", "writer.system"),
      entry("writer", "task", "writer.kickoff.chapter"),
      entry("reviewer", "system", "reviewer.system"),
      entry("shared", "block", "block.settings"),
    ])
    expect(groups.map((group) => group.agent)).toEqual(["writer", "reviewer", "shared"])
    const writer = groups[0]
    expect(writer.groups.map((group) => group.key)).toEqual(["system", "tasks"])
    expect(writer.groups[0].entries[0].key).toBe("writer.system")
    expect(writer.groups[1].entries[0].key).toBe("writer.kickoff.chapter")
  })

  it("orders the agents as the spec lists the roles", () => {
    const groups = groupPrompts([
      entry("shared", "shared", "shared.label"),
      entry("archivist", "task", "archivist.kickoff.seed"),
      entry("writer", "task", "writer.kickoff.chapter"),
      entry("researcher", "task", "researcher.kickoff"),
      entry("conception", "task", "conception.kickoff"),
      entry("reviewer", "task", "reviewer.kickoff"),
    ])
    expect(groups.map((group) => group.agent)).toEqual([
      "writer",
      "reviewer",
      "archivist",
      "conception",
      "researcher",
      "shared",
    ])
  })

  it("drops empty categories and keeps an unknown agent visible", () => {
    const groups = groupPrompts([
      entry("writer", "system", "writer.system"),
      entry("later_role", "task", "later_role.kickoff"),
    ])
    // The writer has no tasks, so that category is not rendered empty.
    expect(groups[0].groups.map((group) => group.key)).toEqual(["system"])
    // An agent added by a later milestone is appended, not silently hidden.
    expect(groups.map((group) => group.agent)).toEqual(["writer", "later_role"])
  })
})

describe("settings locale coverage", () => {
  it("has a label for every protocol the form offers", () => {
    for (const [locale, table] of [
      ["en", enSettings as Record<string, string>],
      ["zh-CN", zhSettings as Record<string, string>],
    ] as const) {
      for (const protocol of PROTOCOLS) {
        expect(table[`protocol.${protocol}`], `${locale} settings:protocol.${protocol}`).toBeTypeOf(
          "string",
        )
      }
      for (const role of ["conception", "researcher", "writer", "reviewer", "archivist"]) {
        expect(table[`role.${role}`], `${locale} settings:role.${role}`).toBeTypeOf("string")
      }
    }
  })
})

describe("prompts locale coverage", () => {
  it("has a group label for every prompt kind", () => {
    for (const [locale, table] of [
      ["en", enPrompts as Record<string, string>],
      ["zh-CN", zhPrompts as Record<string, string>],
    ] as const) {
      for (const group of GROUPS) {
        expect(table[`group.${group.key}`], `${locale} prompts:group.${group.key}`).toBeTypeOf(
          "string",
        )
      }
      for (const source of ["default", "global", "book"]) {
        expect(
          table[`source.${source}`],
          `${locale} prompts:source.${source}`,
        ).toBeTypeOf("string")
      }
    }
  })
})
