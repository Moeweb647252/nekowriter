import { beforeEach, describe, expect, it } from "vitest"

import i18n, { initialLanguage, setLanguage } from "@/i18n"
import { useUiStore } from "@/stores/ui"

describe("language persistence", () => {
  beforeEach(async () => {
    localStorage.clear()
    await setLanguage("en")
  })

  it("setLanguage switches i18next, the document language, the title and the store", async () => {
    await setLanguage("zh-CN")
    expect(i18n.language).toBe("zh-CN")
    expect(document.documentElement.lang).toBe("zh-CN")
    // The tab title is user-visible text and follows the locale (spec/11 §6), so a scaffold value
    // like `vite-app` cannot reach an author.
    expect(document.title).toBe(i18n.t("common:app_name"))
    expect(document.title).not.toBe("")
    expect(useUiStore.getState().language).toBe("zh-CN")
  })

  it("persists the choice in the ui store so a reload restores it", async () => {
    await setLanguage("zh-CN")
    // The store writes through zustand/persist; a fresh read of the key must carry it.
    const raw = localStorage.getItem("nw.ui")
    expect(raw, "the ui store must persist its state").not.toBeNull()
    expect(JSON.parse(raw as string).state.language).toBe("zh-CN")
  })

  it("initialLanguage falls back to the browser language, then English", () => {
    expect(["en", "zh-CN"]).toContain(initialLanguage())
  })
})
