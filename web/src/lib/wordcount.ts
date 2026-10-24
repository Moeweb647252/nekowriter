/**
 * Live word-count approximation (spec/11 §8).
 *
 * The server's UAX #29 count is authoritative and arrives with every content event; this
 * approximation keeps the number moving while the author types. Shared vectors live in
 * `crates/server/src/text/wordcount_vectors.json` and both sides assert them.
 */

const CJK_SINGLE = [
  [0x3041, 0x3096], // Hiragana
  [0x3400, 0x4dbf], // CJK Extension A
  [0x4e00, 0x9fff], // CJK Unified Ideographs
  [0xf900, 0xfaff], // CJK Compatibility Ideographs
  [0x20000, 0x2fa1f], // CJK Extensions B..F
]

const KATAKANA = [
  [0x30a1, 0x30fa],
  [0x30fc, 0x30ff],
]

function inRange(code: number, ranges: number[][]): boolean {
  return ranges.some(([start, end]) => code >= start && code <= end)
}

function isLetterOrDigit(char: string): boolean {
  return /[\p{L}\p{M}\p{N}]/u.test(char)
}

/** Counts words as described in spec/11 §8. */
export function countWords(text: string): number {
  const chars = Array.from(text)
  let count = 0
  let index = 0
  while (index < chars.length) {
    const char = chars[index]
    const code = char.codePointAt(0) ?? 0

    if (inRange(code, CJK_SINGLE)) {
      count += 1
      index += 1
      continue
    }
    if (inRange(code, KATAKANA)) {
      // A run of Katakana (including the long-vowel mark) counts once.
      index += 1
      while (index < chars.length) {
        const nextCode = chars[index].codePointAt(0) ?? 0
        if (!inRange(nextCode, KATAKANA)) {
          break
        }
        index += 1
      }
      count += 1
      continue
    }
    if (isLetterOrDigit(char)) {
      index += 1
      while (index < chars.length) {
        const next = chars[index]
        if (isLetterOrDigit(next)) {
          index += 1
          continue
        }
        const after = chars[index + 1]
        // An apostrophe or a period between two letters keeps the run together ("don't",
        // "e.g"), and a period or comma between two digits keeps it together ("3.14",
        // "1,000") — spec/11 §8.
        if (
          after !== undefined &&
          (next === "'" || next === "’" || next === ".") &&
          /[\p{L}\p{M}]/u.test(chars[index - 1]) &&
          /[\p{L}\p{M}]/u.test(after)
        ) {
          index += 1
          continue
        }
        if (
          after !== undefined &&
          (next === "." || next === ",") &&
          /\d/.test(chars[index - 1]) &&
          /\d/.test(after)
        ) {
          index += 1
          continue
        }
        break
      }
      count += 1
      continue
    }
    index += 1
  }
  return count
}
