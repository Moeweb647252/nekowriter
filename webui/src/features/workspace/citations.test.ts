import { afterEach, describe, expect, it } from "vitest"

import { citable, citeSelection } from "./citations"
import type { CitationTarget } from "@/api/generated/CitationTarget"

const BODY: CitationTarget = {
  kind: "revision",
  revisionId: "0190f5c8-0000-7000-8000-000000000a00",
}
const OTHER: CitationTarget = {
  kind: "revision",
  revisionId: "0190f5c8-0000-7000-8000-000000000a01",
}

/** An element whose text is a segment of `target` starting at `at`. */
function segment(text: string, target: CitationTarget, at: number): HTMLElement {
  const element = document.createElement("span")
  for (const [name, value] of Object.entries(citable({ target, at }))) {
    element.setAttribute(name, value)
  }
  element.textContent = text
  return element
}

function plain(text: string): HTMLElement {
  const element = document.createElement("del")
  element.textContent = text
  return element
}

function render(...children: HTMLElement[]): HTMLElement {
  const root = document.createElement("div")
  root.append(...children)
  document.body.append(root)
  return root
}

/** Selects from `startOffset` in the first node's text to `endOffset` in the last's. */
function select(start: Node, startOffset: number, end: Node, endOffset: number) {
  const range = document.createRange()
  range.setStart(start.firstChild ?? start, startOffset)
  range.setEnd(end.firstChild ?? end, endOffset)
  document.getSelection()?.removeAllRanges()
  document.getSelection()?.addRange(range)
}

afterEach(() => {
  document.getSelection()?.removeAllRanges()
  document.body.replaceChildren()
})

describe("citeSelection", () => {
  it("counts offsets in Unicode scalar values", () => {
    // Two ideographs outside the Basic Multilingual Plane, then " rain".
    const text = "\u{20000}\u{20001} rain"
    const body = segment(text, BODY, 0)
    render(body)
    select(body, text.indexOf("rain"), body, text.length)

    expect(citeSelection(document.getSelection())).toEqual({
      kind: "citation",
      citation: { target: BODY, start: 3, end: 7, quote: "rain" },
    })
  })

  it("joins the segments of one text and leaves out what is not part of it", () => {
    // "The lamp was dark." with "dark" proposed to become "lit".
    const before = segment("The lamp was ", BODY, 0)
    const added = segment("lit", BODY, 13)
    const after = segment(".", BODY, 16)
    render(before, plain("dark"), added, after)
    select(before, 4, after, 1)

    expect(citeSelection(document.getSelection())).toEqual({
      kind: "citation",
      citation: { target: BODY, start: 4, end: 17, quote: "lamp was lit." },
    })
  })

  it("refuses selections across texts and selections of nothing citable", () => {
    const first = segment("Rain.", BODY, 0)
    const second = segment("Snow.", OTHER, 0)
    const label = plain("Chapter 2")
    render(first, label, second)

    select(first, 0, second, 4)
    expect(citeSelection(document.getSelection())).toEqual({ kind: "several_texts" })

    select(label, 0, label, 7)
    expect(citeSelection(document.getSelection())).toEqual({ kind: "nothing" })

    document.getSelection()?.removeAllRanges()
    expect(citeSelection(document.getSelection())).toEqual({ kind: "nothing" })
  })
})
