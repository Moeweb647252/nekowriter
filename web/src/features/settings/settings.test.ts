/** Pure helpers of the prompts and settings screens (spec/11 §9: unit tests for UI logic). */
import { describe, expect, it } from "vitest"

import { parseHeaders } from "@/features/settings/ProvidersPage"
import enPrompts from "@/i18n/locales/en/prompts.json"
import zhPrompts from "@/i18n/locales/zh-CN/prompts.json"
import enSettings from "@/i18n/locales/en/settings.json"
import zhSettings from "@/i18n/locales/zh-CN/settings.json"
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
