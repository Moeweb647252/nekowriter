import { QuoteIcon, XIcon } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"

import { citeSelection, sameCitation } from "./citations"
import { quote } from "./writing/edits"
import type { Citation } from "@/api/generated/Citation"
import { Button } from "@/components/ui/button"
import { type DraftList } from "@/lib/drafts"

/**
 * The passages a piece of feedback cites, and a button that cites the passage selected in
 * the reader or a document (§18.4). The citations are kept with the feedback draft.
 */
export function CitationField({ citations }: { citations: DraftList<Citation> }) {
  const { t } = useTranslation()
  const [refused, setRefused] = useState<"nothing" | "several_texts" | null>(null)

  const cite = () => {
    const selected = citeSelection(document.getSelection())
    if (selected.kind !== "citation") {
      setRefused(selected.kind)
      return
    }
    setRefused(null)
    if (!citations.items.some((item) => sameCitation(item, selected.citation))) {
      citations.add(selected.citation)
    }
  }

  return (
    <div className="flex flex-col items-start gap-2">
      {citations.items.length > 0 && (
        <ul aria-label={t(($) => $.workspace.citations.label)} className="flex flex-wrap gap-2">
          {citations.items.map((citation, index) => (
            <li
              // Citations have no IDs; a citation's place in the draft is its identity.
              key={index}
              className="flex items-center gap-1 rounded-md border bg-background py-0.5 pr-0.5 pl-2 text-sm"
            >
              <span className="font-reading">“{quote(citation.quote)}”</span>
              <Button
                type="button"
                variant="ghost"
                size="icon-xs"
                onClick={() => citations.remove(index)}
                aria-label={t(($) => $.workspace.citations.remove, {
                  quote: quote(citation.quote),
                })}
              >
                <XIcon />
              </Button>
            </li>
          ))}
        </ul>
      )}
      <Button
        type="button"
        variant="outline"
        size="sm"
        // Keep the selection that the button cites.
        onMouseDown={(event) => event.preventDefault()}
        onClick={cite}
      >
        <QuoteIcon />
        {t(($) => $.workspace.citations.cite)}
      </Button>
      {refused !== null && (
        <p role="status" className="text-sm text-muted-foreground">
          {t(($) => $.workspace.citations.refused[refused])}
        </p>
      )}
    </div>
  )
}

/** The passages that sent feedback cites. */
export function CitedQuotes({ citations }: { citations: readonly Citation[] }) {
  const { t } = useTranslation()
  if (citations.length === 0) return null
  return (
    <ul aria-label={t(($) => $.workspace.citations.label)} className="flex flex-col gap-0.5">
      {citations.map((citation, index) => (
        // Sent citations never change; their order is their identity.
        <li key={index} className="font-reading text-sm text-muted-foreground">
          “{quote(citation.quote)}”
        </li>
      ))}
    </ul>
  )
}
