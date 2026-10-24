import { describe, expect, it } from "vitest"
import i18n, { SUPPORTED_LANGUAGES } from "@/i18n"
import { formatWords } from "@/lib/format"

describe("formatWords", () => {
  it("renders from the i18n resource in both locales (spec/11 §6)", () => {
    expect(formatWords(2350, "en")).toBe("2,350 words")
    expect(formatWords(2350, "zh-CN")).toBe("2,350 字")
  })

  it("groups digits per locale", () => {
    expect(formatWords(1000, "en")).toBe("1,000 words")
    expect(formatWords(1000, "zh-CN")).toBe("1,000 字")
  })

  it("renders every supported locale, so no unit is hardcoded per language", () => {
    // A hardcoded `zh -> 字` branch would pass for these two and fail for a third locale; going
    // through `common:words` keeps the behavior driven by locale data (AGENTS.md).
    for (const language of SUPPORTED_LANGUAGES) {
      expect(formatWords(2350, language)).toMatch(/^2,350 \S+$/)
    }
    expect(formatWords(2350, "en")).not.toBe(formatWords(2350, "zh-CN"))
  })

  it("follows the active language when none is passed", async () => {
    await i18n.changeLanguage("zh-CN")
    expect(formatWords(12)).toBe("12 字")
    await i18n.changeLanguage("en")
    expect(formatWords(12)).toBe("12 words")
  })
})
