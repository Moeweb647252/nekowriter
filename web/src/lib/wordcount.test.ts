import { describe, expect, it } from "vitest"

// The vectors are shared with the Rust test (spec/11 §8).
import vectors from "../../../crates/server/src/text/wordcount_vectors.json"
import { countWords } from "@/lib/wordcount"

type Vector = { text: string; words: number }

const cases = (vectors as { cases: Vector[] }).cases

describe("countWords", () => {
  it("asserts the shared Rust vectors (spec/11 §8)", () => {
    expect(cases.length).toBeGreaterThanOrEqual(12)
    for (const entry of cases) {
      expect(countWords(entry.text), JSON.stringify(entry.text)).toBe(entry.words)
    }
  })

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
