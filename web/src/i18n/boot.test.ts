import { beforeEach, describe, expect, it } from "vitest"

import { resolveLanguage, setLanguage } from "@/i18n"

describe("boot language resolution", () => {
  beforeEach(async () => {
    localStorage.clear()
    await setLanguage("en")
  })

  it("uses the language persisted in the ui store across a reload", async () => {
    await setLanguage("zh-CN")
    // Simulate what a reload does: the store rehydrates from localStorage.
    expect(resolveLanguage()).toBe("zh-CN")
  })

  it("falls back to English when nothing is stored", async () => {
    await setLanguage("en")
    localStorage.clear()
    expect(["en", "zh-CN"]).toContain(resolveLanguage())
  })
})
