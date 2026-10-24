/** `chapterDoc.applyOps` (spec/11 §9, PLAN M1.4). */
import { beforeEach, describe, expect, it } from "vitest"

import type { ChapterContent } from "@/api/gen/ChapterContent"
import type { ContentOp } from "@/api/gen/ContentOp"
import { useChapterDoc } from "@/stores/chapterDoc"

const content = (paragraphs: [string, string][], tailOpen = false): ChapterContent => ({
  paragraphs: paragraphs.map(([pid, text]) => ({ pid, text })),
  next_pid: paragraphs.length + 1,
  tail_open: tailOpen,
})

const reset = () => useChapterDoc.getState().reset()

const paragraphs = () => useChapterDoc.getState().paragraphs.map((p) => [p.pid, p.text])
const version = () => useChapterDoc.getState().version
const tailOpen = () => useChapterDoc.getState().tailOpen

describe("chapterDoc.applyOps", () => {
  beforeEach(reset)

  it("loads a chapter document", () => {
    useChapterDoc.getState().load(7, 3, content([["p1", "one"], ["p2", "two"]], true))
    expect(useChapterDoc.getState().chapterId).toBe(7)
    expect(version()).toBe(3)
    expect(paragraphs()).toEqual([["p1", "one"], ["p2", "two"]])
    expect(tailOpen()).toBe(true)
    expect(useChapterDoc.getState().highlights).toEqual({})
  })

  it("inserts at the start, in the middle and at the end", () => {
    useChapterDoc.getState().load(1, 1, content([["p1", "one"], ["p2", "two"]]))

    const insert = (after: string | null, pid: string, text: string): ContentOp => ({
      op: "insert",
      after,
      pid,
      text,
    })

    expect(useChapterDoc.getState().applyOps(2, [insert(null, "p3", "start")], "user")).toBe(
      "applied",
    )
    expect(paragraphs()).toEqual([
      ["p3", "start"],
      ["p1", "one"],
      ["p2", "two"],
    ])

    expect(useChapterDoc.getState().applyOps(3, [insert("p1", "p4", "middle")], "user")).toBe(
      "applied",
    )
    expect(paragraphs()).toEqual([
      ["p3", "start"],
      ["p1", "one"],
      ["p4", "middle"],
      ["p2", "two"],
    ])

    expect(useChapterDoc.getState().applyOps(4, [insert("p2", "p5", "end")], "user")).toBe(
      "applied",
    )
    expect(paragraphs().at(-1)).toEqual(["p5", "end"])
    expect(version()).toBe(4)
  })

  it("updates and deletes a paragraph", () => {
    useChapterDoc.getState().load(1, 1, content([["p1", "one"], ["p2", "two"]]))
    useChapterDoc
      .getState()
      .applyOps(2, [{ op: "update", pid: "p1", text: "uno" }], "user")
    expect(paragraphs()).toEqual([["p1", "uno"], ["p2", "two"]])
    useChapterDoc.getState().applyOps(3, [{ op: "delete", pid: "p2" }], "user")
    expect(paragraphs()).toEqual([["p1", "uno"]])
  })

  it("applies the tail op", () => {
    useChapterDoc.getState().load(1, 1, content([["p1", "open"]], true))
    useChapterDoc.getState().applyOps(2, [{ op: "tail", open: false }], "agent")
    expect(tailOpen()).toBe(false)
  })

  it("marks agent insertions and updates and ignores a duplicate version", () => {
    useChapterDoc.getState().load(1, 1, content([["p1", "one"]]))
    useChapterDoc
      .getState()
      .applyOps(2, [{ op: "insert", after: null, pid: "p2", text: "zero" }], "agent")
    expect(useChapterDoc.getState().highlights["p2"]).toMatchObject({ kind: "inserted" })
    useChapterDoc
      .getState()
      .applyOps(3, [{ op: "update", pid: "p1", text: "uno" }], "agent")
    expect(useChapterDoc.getState().highlights["p1"]).toMatchObject({ kind: "updated" })

    // A replayed version must not apply twice (our own mutation response arrived first).
    expect(
      useChapterDoc.getState().applyOps(2, [{ op: "delete", pid: "p1" }], "agent"),
    ).toBe("applied")
    expect(paragraphs().some(([pid]) => pid === "p1")).toBe(true)
  })

  it("reports a version gap instead of applying it", () => {
    useChapterDoc.getState().load(1, 4, content([["p1", "one"]]))
    const result = useChapterDoc
      .getState()
      .applyOps(9, [{ op: "delete", pid: "p1" }], "user")
    expect(result).toBe("gap")
    expect(version()).toBe(4)
    expect(paragraphs()).toEqual([["p1", "one"]])
  })

  it("clears a highlight and resets the document", () => {
    useChapterDoc.getState().load(1, 1, content([["p1", "one"]]))
    useChapterDoc
      .getState()
      .applyOps(2, [{ op: "insert", after: null, pid: "p2", text: "x" }], "agent")
    useChapterDoc.getState().clearHighlight("p2")
    expect(useChapterDoc.getState().highlights["p2"]).toBeUndefined()

    useChapterDoc.getState().reset()
    expect(useChapterDoc.getState().chapterId).toBeNull()
    expect(paragraphs()).toEqual([])
  })
})
