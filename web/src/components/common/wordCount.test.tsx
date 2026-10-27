/**
 * The header word count (spec/11 §5.2, §8; D-141).
 *
 * The header states what the chapter **holds**, because the server's `word_count` is authoritative
 * (spec/11 §8) — an agent's staged write is not in the chapter yet (D-139). Before D-141 the draft
 * pane reported its own number beside the header's, so a held write put two different counts on one
 * screen with nothing saying why. The rule now: the confirmed count keeps its place, and the
 * projection is a labelled second number in the waiting token.
 */
import { render, screen } from "@testing-library/react"
import { describe, expect, it } from "vitest"

import { WordCount } from "@/components/common/WordCount"

describe("WordCount", () => {
  it("shows the confirmed count against the target range", () => {
    render(<WordCount words={1823} targetMin={3000} targetMax={4000} language="en" />)
    expect(screen.getByText(/1,823/)).toBeInTheDocument()
    expect(screen.getByText(/3,000–4,000/)).toBeInTheDocument()
  })

  it("adds a staged projection without replacing the confirmed count", () => {
    render(<WordCount words={1823} projected={1912} targetMin={3000} targetMax={4000} />)
    // Both numbers are present: the chapter's own count, and what it would read once the agent's
    // text is applied.
    expect(screen.getByText(/1,823/)).toBeInTheDocument()
    expect(screen.getByText(/1,912/)).toBeInTheDocument()
    // The projection is labelled, not a bare second figure.
    expect(screen.getByTitle("Word count once the staged write applies (projection only, not stored)")).toBeInTheDocument()
    expect(screen.getByText("once applied")).toBeInTheDocument()
  })

  it("has no projection when nothing is staged", () => {
    render(<WordCount words={1823} targetMin={3000} targetMax={4000} />)
    expect(screen.queryByText("once applied")).not.toBeInTheDocument()
  })
})
