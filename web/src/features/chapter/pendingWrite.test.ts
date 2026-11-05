/**
 * The draft pane's view of a held chapter write (D-139, D-141).
 *
 * Two behaviours decide whether the pane tells the truth, and both are pure functions here so they
 * can be asserted without a React tree:
 *
 * 1. `pending_content.content` is a **complete** `ChapterContent`, not a delta. The merged view is
 *    therefore the staged document itself (with the confirmed draft used only to classify each
 *    paragraph), never a concatenation of the two.
 * 2. A staged write whose `expected_version` no longer matches the draft is **discarded** by
 *    `runner::apply_pending_content` (the author's edit wins). The pane must not render it as if it
 *    will apply: `pendingStanding` returns `superseded` and `pendingDraftView` is what the dock
 *    reads to say so.
 */
import { describe, expect, it } from "vitest"

import type { ChapterContent } from "@/api/gen/ChapterContent"
import type { ToolCallDto } from "@/api/gen/ToolCallDto"
import {
  contentWordCount,
  mergedPending,
  pendingDraftView,
  pendingStanding,
  pendingWrite,
} from "@/features/chapter/pendingWrite"

/** A chapter content of the given paragraphs, with a plausible `next_pid`. */
function content(paragraphs: [string, string][], tailOpen = false): ChapterContent {
  return {
    paragraphs: paragraphs.map(([pid, text]) => ({ pid, text })),
    next_pid: paragraphs.length + 1,
    tail_open: tailOpen,
  }
}

/** A held write call carrying the given staged content. */
function writeCall(
  staged: ChapterContent,
  expectedVersion: number,
  overrides: Partial<ToolCallDto> = {},
): ToolCallDto {
  return {
    id: 283,
    message_id: 124,
    seq: 3,
    call_index: 0,
    name: "insert_text",
    input: { text: "雨点敲在船篷上。" },
    input_text: "",
    status: "held",
    is_error: false,
    result_text: null,
    appendices: [],
    ui: { pid: "p3", chars: 8 },
    sealed: false,
    completes: false,
    pending_content: { content: staged, expected_version: expectedVersion },
    created_at: "2026-01-01T00:00:00Z",
    finished_at: null,
    ...overrides,
  }
}

describe("pendingWrite", () => {
  it("reads the staged content of a held call", () => {
    const staged = content([["p1", "A"], ["p2", "B"]])
    const write = pendingWrite(writeCall(staged, 4))
    expect(write?.content).toBe(staged)
    expect(write?.expectedVersion).toBe(4)
  })

  it("ignores a call that is not held, or carries no staged content", () => {
    // `pending_content` is cleared once the call is finalized, but a snapshot fetched just before
    // the decision can still carry one; a decided call must never keep a preview on screen.
    const staged = content([["p1", "A"]])
    expect(pendingWrite(writeCall(staged, 1, { status: "done" }))).toBeUndefined()
    expect(pendingWrite(writeCall(staged, 1, { pending_content: null }))).toBeUndefined()
    expect(pendingWrite(undefined)).toBeUndefined()
  })
})

describe("pendingStanding", () => {
  it("is current while the draft is still at the version the tool read", () => {
    const staged = content([["p1", "A"]])
    const write = pendingWrite(writeCall(staged, 4))
    expect(pendingStanding(write, 4)).toBe("current")
  })

  it("is superseded once the author moved the draft on", () => {
    // This is the case `runner::apply_pending_content` drops: the author edited while the call was
    // held, so their text wins and the staged content is never applied (spec/03 §5.2).
    const staged = content([["p1", "A"]])
    const write = pendingWrite(writeCall(staged, 4))
    expect(pendingStanding(write, 5)).toBe("superseded")
  })

  it("has no standing when nothing is staged", () => {
    expect(pendingStanding(undefined, 4)).toBeUndefined()
  })
  it("is current when the newer draft already equals the staged content", () => {
    // Approval bumps the chapter version as the write lands. Until the call is refetched, the
    // version differs; the matching text proves the write is already applied, not superseded.
    const staged = content([["p1", "A"], ["p2", "B"]])
    const write = pendingWrite(writeCall(staged, 4))
    expect(
      pendingStanding(write, 5, [
        { pid: "p1", text: "A" },
        { pid: "p2", text: "B" },
      ]),
    ).toBe("current")
    expect(
      pendingStanding(write, 5, [
        { pid: "p1", text: "author" },
        { pid: "p2", text: "B" },
      ]),
    ).toBe("superseded")
  })
})

describe("mergedPending", () => {
  it("renders the staged document itself, not a concatenation with the draft", () => {
    // The staged content is the *whole* chapter as the tool would store it: a rewrite of p1 and a
    // new p3. A delta-shaped reading would append p3 to the confirmed paragraphs and show p1 twice.
    const staged = content([["p1", "改写后的第一段。"], ["p2", "第二段。"], ["p3", "新加的一段。"]])
    const merged = mergedPending(pendingWrite(writeCall(staged, 4))!, {
      version: 4,
      paragraphs: [
        { pid: "p1", text: "第一段。" },
        { pid: "p2", text: "第二段。" },
      ],
    })
    expect(merged).toEqual([
      { pid: "p1", text: "改写后的第一段。", kind: "changed", highlightFrom: 0 },
      { pid: "p2", text: "第二段。", kind: "confirmed" },
      { pid: "p3", text: "新加的一段。", kind: "added" },
    ])
  })

  it("drops a confirmed paragraph the staged document no longer has", () => {
    // A delete tool stages the document without that paragraph, so it must not appear at all.
    const staged = content([["p1", "A"]])
    const merged = mergedPending(pendingWrite(writeCall(staged, 2))!, {
      version: 2,
      paragraphs: [
        { pid: "p1", text: "A" },
        { pid: "p2", text: "B" },
      ],
    })
    expect(merged.map((paragraph) => paragraph.pid)).toEqual(["p1"])
  })

  it("marks an empty chapter's staged text as added", () => {
    const staged = content([["p1", "第一段。"], ["p2", "第二段。"]])
    const merged = mergedPending(pendingWrite(writeCall(staged, 0))!, { version: 0, paragraphs: [] })
    expect(merged.every((paragraph) => paragraph.kind === "added")).toBe(true)
  })
})

describe("pendingDraftView", () => {
  const draft = {
    version: 4,
    paragraphs: [
      { pid: "p1", text: "第一段。" },
      { pid: "p2", text: "第二段。" },
    ],
  }

  it("is absent when the call staged nothing, so the pane renders the confirmed draft", () => {
    expect(pendingDraftView(undefined, draft)).toBeUndefined()
    expect(
      pendingDraftView(writeCall(content([["p1", "A"]]), 4, { pending_content: null }), draft),
    ).toBeUndefined()
  })

  it("reports the standing, the versions and both word counts", () => {
    // The count is the shared one: "第一段。" is three ideographs (the full stop is not a word), so
    // the confirmed draft is six. What matters is that each count comes from its own document.
    const staged = content([["p1", "改写后的第一段。"], ["p2", "第二段。"], ["p3", "新加的一段。"]])
    const view = pendingDraftView(writeCall(staged, 4), draft)!
    expect(view.standing).toBe("current")
    expect(view.expectedVersion).toBe(4)
    expect(view.draftVersion).toBe(4)
    expect(view.paragraphs).toHaveLength(3)
    expect(view.stagedWords).toBe(contentWordCount(staged))
    expect(view.confirmedWords).toBe(6)
    expect(view.stagedWords).toBeGreaterThan(view.confirmedWords)
  })

  it("still describes a superseded write, so the dock can explain it", () => {
    // The view is returned (not discarded) precisely so the dock can say the text will not be
    // applied; `standing` is what keeps it off the pane (see `ChapterPage`).
    const view = pendingDraftView(writeCall(content([["p1", "A"]]), 4), { ...draft, version: 6 })!
    expect(view.standing).toBe("superseded")
    expect(view.expectedVersion).toBe(4)
    expect(view.draftVersion).toBe(6)
  })
})

describe("contentWordCount", () => {
  it("folds the shared count over the paragraph texts like the server does", () => {
    // `crates/server/src/text/content.rs::content_word_count` is the authority; the client's
    // `countWords` is asserted against the same vectors, so this projection matches it.
    expect(contentWordCount(content([["p1", "one two three"], ["p2", "four"]]))).toBe(4)
    expect(contentWordCount(content([["p1", "凌晨两点。"], ["p2", "林昭醒来。"]]))).toBe(8)
    expect(contentWordCount(content([]))).toBe(0)
  })
})
