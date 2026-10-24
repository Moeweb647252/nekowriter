/**
 * Live word-count approximation (spec/11 §8).
 *
 * The server's UAX #29 count is authoritative and arrives with every content event; this
 * approximation keeps the number moving while the author types. Both sides assert the shared
 * vectors in `crates/server/src/text/wordcount_vectors.json`, so the two implementations cannot
 * drift apart on the cases prose actually contains.
 */

/** Ranges that count one each: Han ideographs and Hiragana (spec/11 §8). */
const SINGLE_COUNTED = [
  [0x3041, 0x3096], // Hiragana
  [0x3400, 0x4dbf], // CJK Unified Ideographs Extension A
  [0x4e00, 0x9fff], // CJK Unified Ideographs
  [0xf900, 0xfaff], // CJK Compatibility Ideographs
  [0x20000, 0x2fa1f], // CJK Unified Ideographs Extensions B..F
  [0x3005, 0x3005], // Ideographic iteration mark
  [0x3007, 0x3007], // Ideographic number zero
]

/** Katakana, including `ー`, the iteration marks and the halfwidth block. */
const KATAKANA = [
  [0x30a1, 0x30fa],
  [0x30fc, 0x30ff],
  [0xff66, 0xff9d],
  [0xff9e, 0xff9f],
]

/** Connectors that continue a run in UAX #29 (`ExtendNumLet`). */
const RUN_CONNECTORS = new Set(["_"])

/** Chars that keep a run of letters together when they sit between two letters. */
const LETTER_MID = new Set(["'", "\u2019", ".", ":", "\u00b7"])

/** Chars that keep a run of digits together when they sit between two digits. */
const DIGIT_MID = new Set([".", ",", ";"])

const LETTER = /^\p{L}$/u
const MARK = /^\p{M}$/u
const DECIMAL = /^\p{Nd}$/u

function inRange(code: number, ranges: number[][]): boolean {
  return ranges.some(([start, end]) => code >= start && code <= end)
}

const codeOf = (char: string): number => char.codePointAt(0) ?? 0
const isSingleCounted = (char: string): boolean => inRange(codeOf(char), SINGLE_COUNTED)
const isKatakana = (char: string): boolean =>
  inRange(codeOf(char), KATAKANA) && !isSingleCounted(char)
const isLetter = (char: string): boolean => LETTER.test(char)
const isMark = (char: string): boolean => MARK.test(char)
const isDigit = (char: string): boolean => DECIMAL.test(char)

/** Counts words as described in spec/11 §8. */
export function countWords(text: string): number {
  const chars = Array.from(text)
  let count = 0
  let index = 0
  while (index < chars.length) {
    const char = chars[index]

    if (isSingleCounted(char)) {
      count += 1
      index += 1
      continue
    }
    if (isKatakana(char)) {
      // A run of Katakana (including the long-vowel mark) counts once.
      index += 1
      while (index < chars.length && isKatakana(chars[index])) {
        index += 1
      }
      count += 1
      continue
    }
    if (!isLetter(char) && !isDigit(char)) {
      index += 1
      continue
    }

    // A run of letters, marks, digits and connectors in a spaced script is one word.
    index += 1
    while (index < chars.length) {
      const next = chars[index]
      if (isSingleCounted(next) || isKatakana(next)) {
        break
      }
      if (isLetter(next) || isDigit(next) || isMark(next) || RUN_CONNECTORS.has(next)) {
        index += 1
        continue
      }
      const after = chars[index + 1]
      const previous = chars[index - 1]
      // An apostrophe or a period between two letters keeps the run together ("don't",
      // "e.g"); a period or comma between two digits keeps it together ("3.14", "1,000")
      // — spec/11 §8.
      if (after !== undefined && LETTER_MID.has(next) && isLetter(previous) && isLetter(after)) {
        index += 1
        continue
      }
      if (after !== undefined && DIGIT_MID.has(next) && isDigit(previous) && isDigit(after)) {
        index += 1
        continue
      }
      break
    }
    count += 1
  }
  return count
}
