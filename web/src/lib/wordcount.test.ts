import { describe, expect, it } from "vitest"

import { countWords } from "@/lib/wordcount"

describe("countWords", () => {
  it("counts each CJK ideograph as one word", () => {
    expect(countWords("风吹过山谷")).toBe(5)
  })

  it("counts latin runs as words", () => {
    expect(countWords("The wind blew through the valley")).toBe(6)
  })

  it("keeps contractions and numbers together", () => {
    expect(countWords("don't e.g. 3.14 1,000")).toBe(4)
  })

  it("splits hyphenated words", () => {
    expect(countWords("well-known")).toBe(2)
  })

  it("counts hiragana one by one and katakana runs once", () => {
    expect(countWords("ひらがな")).toBe(4)
    expect(countWords("カタカナ")).toBe(1)
  })

  it("ignores punctuation and whitespace", () => {
    expect(countWords("  ,.!? \n\t ")).toBe(0)
    expect(countWords("")).toBe(0)
  })

  it("mixes scripts", () => {
    expect(countWords("他说 hello world 了")).toBe(5)
  })
})
