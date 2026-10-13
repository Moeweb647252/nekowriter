import type { Citation } from "@/api/generated/Citation"
import type { CitationTarget } from "@/api/generated/CitationTarget"
import type { ProposalView } from "@/api/generated/ProposalView"
import type { RevisionId } from "@/api/generated/RevisionId"

/**
 * Citable text is rendered in segments (§18.4). Each segment is an element whose text is
 * a contiguous part of one target text, marked with the target and the offset where the
 * part starts, in Unicode scalar values. A target's segments cover its text in order, so
 * the selected parts of its segments, joined, are the quoted passage.
 */
const TARGET = "data-cite"
const AT = "data-cite-at"

/** A segment: the text it belongs to, and the offset in that text where it starts. */
export interface CiteSpan {
  target: CitationTarget
  at: number
}

/** The attributes that mark an element's text as a segment. */
export function citable({ target, at }: CiteSpan): Record<string, string> {
  return { [TARGET]: JSON.stringify(target), [AT]: String(at) }
}

/** A segment that holds the whole of a revision's text. */
export function wholeRevision(revisionId: RevisionId): CiteSpan {
  return { target: { kind: "revision", revisionId }, at: 0 }
}

/** The text a pending proposal would produce. */
export function proposalTarget(proposal: ProposalView): CitationTarget {
  return { kind: "proposal", callId: proposal.callId, candidateHash: proposal.candidateHash }
}

/** The length of `text` in Unicode scalar values, which the server counts in. */
export function scalarLength(text: string): number {
  return Array.from(text).length
}

export type SelectionCitation =
  | { kind: "citation"; citation: Citation }
  /** Nothing citable is selected. */
  | { kind: "nothing" }
  /** The selection spans more than one text. */
  | { kind: "several_texts" }

/** Reads the passage the user selected as a citation. */
export function citeSelection(selection: Selection | null): SelectionCitation {
  if (selection === null || selection.rangeCount === 0 || selection.isCollapsed) {
    return { kind: "nothing" }
  }
  const range = selection.getRangeAt(0)
  const parts = segmentsIn(range).flatMap((segment) => {
    const part = document.createRange()
    part.selectNodeContents(segment)
    if (range.compareBoundaryPoints(Range.START_TO_START, part) > 0) {
      part.setStart(range.startContainer, range.startOffset)
    }
    if (range.compareBoundaryPoints(Range.END_TO_END, part) < 0) {
      part.setEnd(range.endContainer, range.endOffset)
    }
    const text = part.toString()
    return text === "" ? [] : [{ segment, part, text }]
  })
  const first = parts[0]
  if (first === undefined) return { kind: "nothing" }
  const target = first.segment.getAttribute(TARGET) ?? ""
  if (parts.some(({ segment }) => segment.getAttribute(TARGET) !== target)) {
    return { kind: "several_texts" }
  }

  const before = document.createRange()
  before.selectNodeContents(first.segment)
  before.setEnd(first.part.startContainer, first.part.startOffset)
  const quote = parts.map((part) => part.text).join("")
  const start = Number(first.segment.getAttribute(AT)) + scalarLength(before.toString())
  return {
    kind: "citation",
    citation: {
      target: JSON.parse(target) as CitationTarget,
      start,
      end: start + scalarLength(quote),
      quote,
    },
  }
}

/** The segments the range touches, in document order. */
function segmentsIn(range: Range): HTMLElement[] {
  const ancestor = range.commonAncestorContainer
  const element = ancestor instanceof HTMLElement ? ancestor : ancestor.parentElement
  const enclosing = element?.closest<HTMLElement>(`[${TARGET}]`)
  if (enclosing !== null && enclosing !== undefined) return [enclosing]
  return Array.from(element?.querySelectorAll<HTMLElement>(`[${TARGET}]`) ?? []).filter(
    (segment) => range.intersectsNode(segment),
  )
}

/** Whether two citations quote the same passage of the same text. */
export function sameCitation(a: Citation, b: Citation): boolean {
  return JSON.stringify(a) === JSON.stringify(b)
}
