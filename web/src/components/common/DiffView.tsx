/** An inline diff of two texts (spec/11 §5.2 `HistoryTab`).
 *
 * Uses `diffWordsWithSpace` for spaced languages and `diffChars` for unspaced ones, so Chinese and
 * Japanese read as character edits while English reads as word edits. */
import { diffChars, diffWordsWithSpace, type Change } from "diff"
import { useMemo } from "react"

import { cn } from "@/lib/utils"

export type DiffViewProps = {
  before: string
  after: string
  /** True for zh/ja/th/lo/km/my writing languages. */
  unspaced: boolean
  className?: string
}

/** Renders insertions underlined green and deletions struck red. */
export function DiffView({ before, after, unspaced, className }: DiffViewProps) {
  const changes = useMemo<Change[]>(
    () => (unspaced ? diffChars(before, after) : diffWordsWithSpace(before, after)),
    [before, after, unspaced],
  )
  return (
    <div className={cn("prose-latin text-sm leading-relaxed whitespace-pre-wrap", className)}>
      {changes.map((change, index) => {
        if (change.added) {
          return (
            <ins key={index} className="bg-status-done/15 text-status-done no-underline">
              {change.value}
            </ins>
          )
        }
        if (change.removed) {
          return (
            <del key={index} className="bg-status-failed/15 text-status-failed">
              {change.value}
            </del>
          )
        }
        return <span key={index}>{change.value}</span>
      })}
    </div>
  )
}
