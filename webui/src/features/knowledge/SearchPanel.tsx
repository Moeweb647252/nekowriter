import { SearchIcon } from "lucide-react"
import { type SubmitEvent, useState } from "react"
import { useTranslation } from "react-i18next"

import type { KnowledgeEntryId } from "@/api/generated/KnowledgeEntryId"
import type { KnowledgeSpaceId } from "@/api/generated/KnowledgeSpaceId"
import { useKnowledgeSearch } from "@/api/queries/knowledge"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"

/** Searching the selected space, or all, by meaning (§12.2). */
export function SearchPanel({
  space,
  onOpen,
}: {
  space: KnowledgeSpaceId | null
  onOpen: (entry: KnowledgeEntryId) => void
}) {
  const { t } = useTranslation()
  const search = useKnowledgeSearch()
  const [query, setQuery] = useState("")

  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    search.mutate({ query, spaces: space === null ? [] : [space] })
  }

  return (
    <section aria-label={t(($) => $.knowledge.search.label)} className="flex flex-col gap-3">
      <form onSubmit={submit} className="flex max-w-2xl gap-2">
        <Input
          aria-label={t(($) => $.knowledge.search.label)}
          placeholder={t(($) => $.knowledge.search.placeholder)}
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
        <Button type="submit" disabled={search.isPending || query.trim() === ""}>
          <SearchIcon />
          {t(($) => $.knowledge.search.submit)}
        </Button>
      </form>
      {search.isError && <ErrorAlert error={search.error} />}
      {search.isSuccess &&
        (search.data.hits.length === 0 ? (
          <p className="text-sm text-muted-foreground">{t(($) => $.knowledge.search.none)}</p>
        ) : (
          <ol aria-label={t(($) => $.knowledge.search.results)} className="flex flex-col gap-2">
            {search.data.hits.map((hit) => (
              <li key={hit.chunkId}>
                <button
                  type="button"
                  onClick={() => onOpen(hit.entryId)}
                  className="flex w-full flex-col gap-1 rounded-md border px-3 py-2 text-left text-sm hover:bg-muted/60"
                >
                  <span className="flex flex-wrap items-baseline justify-between gap-2">
                    <span className="font-medium">{hit.title}</span>
                    <span className="text-xs text-muted-foreground">
                      {hit.spaceName} ·{" "}
                      {t(($) => $.knowledge.search.relevance, {
                        relevance: Math.round(hit.relevance * 100) / 100,
                      })}
                    </span>
                  </span>
                  <span className="line-clamp-3 whitespace-pre-wrap text-muted-foreground">
                    {hit.text}
                  </span>
                </button>
              </li>
            ))}
          </ol>
        ))}
    </section>
  )
}
