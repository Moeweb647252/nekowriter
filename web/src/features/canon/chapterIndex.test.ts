/**
 * Chapter labels of the canon pages (spec/10 §2.4, D-054).
 *
 * The canon DTOs carry chapter ids; a chapter's number is its `ord` inside its track. The helper
 * must render the number and the track, never the row id — and it must survive a chapter row that
 * is gone (change log entries keep their reference).
 */
import { describe, expect, it } from "vitest"

import {
  chapterRefLabel,
  type ChapterEntry,
  type ChapterIndex,
  type Translate,
} from "@/features/canon/chapterIndex"

/** A `t` stand-in that includes the options, so a test can see the number it was given. */
const t: Translate = (key, options) => `${key}|${JSON.stringify(options ?? {})}`

function index(...entries: ChapterEntry[]): ChapterIndex {
  return new Map(entries.map((entry) => [entry.id, entry]))
}

describe("chapterRefLabel", () => {
  it("renders the chapter number, not its row id", () => {
    // Chapter id 3 is the first chapter of a second book (`ord` 1). Rendering "3" is the bug.
    const label = chapterRefLabel(
      t,
      index({
        id: 3,
        number: 1,
        title: "The Gate",
        trackId: 9,
        trackKind: "main",
        trackTitle: "",
      }),
      3,
    )
    expect(label).toBe('canon:chapter_number|{"number":1}')
    expect(label).not.toContain("3")
  })

  it("labels a side-track chapter with its track", () => {
    const label = chapterRefLabel(
      t,
      index({
        id: 8,
        number: 2,
        title: "Side",
        trackId: 2,
        trackKind: "side",
        trackTitle: "Sword Tomb",
      }),
      8,
    )
    expect(label).toBe(
      'canon:chapter_number_side|{"number":2,"track":"Sword Tomb"}',
    )
  })

  it("falls back to the reference when the chapter row is gone", () => {
    expect(chapterRefLabel(t, index(), 42)).toBe('canon:chapter_unknown|{"id":42}')
    expect(chapterRefLabel(t, index(), null)).toBe("")
  })
})
