import { useCallback } from "react"
import { useTranslation } from "react-i18next"

import type { BodyEdit } from "@/api/generated/BodyEdit"

/** Quotes stay readable in a sentence however long the text is. */
const QUOTE_CHARS = 60

export function quote(text: string): string {
  const chars = Array.from(text)
  return chars.length > QUOTE_CHARS ? `${chars.slice(0, QUOTE_CHARS).join("")}…` : text
}

/** Says what a proposed edit would do. */
export function useProposalSummary(): (edit: BodyEdit) => string {
  const { t } = useTranslation()
  return useCallback(
    (edit: BodyEdit) => {
      if (edit.op === "append") return t(($) => $.workspace.proposal.append)
      if (edit.newStr === "") {
        return t(($) => $.workspace.proposal.delete, { old: quote(edit.oldStr) })
      }
      return t(($) => $.workspace.proposal.replace, {
        old: quote(edit.oldStr),
        new: quote(edit.newStr),
      })
    },
    [t],
  )
}
