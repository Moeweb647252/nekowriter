/** The book's setting proposals (PROJECT §8.5, spec/06 §5): pending first, decided ones behind a
 * filter. */
import { useState } from "react"
import { useParams } from "react-router"
import { useTranslation } from "react-i18next"

import { useBook } from "@/api/hooks/books"
import { useProposals } from "@/api/hooks/reviews"
import { EmptyState } from "@/components/common/EmptyState"
import { ProposalCard } from "@/features/proposals/ProposalCard"
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Skeleton } from "@/components/ui/skeleton"
import { isUnspaced } from "@/lib/language"

const FILTERS = ["pending", "conflict", "all"] as const
type Filter = (typeof FILTERS)[number]

export default function ProposalsPage() {
  const { bookId: rawBookId } = useParams()
  const bookId = Number(rawBookId)
  const { t } = useTranslation()
  const book = useBook(bookId)
  const [filter, setFilter] = useState<Filter>("pending")
  const proposals = useProposals(bookId, filter === "all" ? undefined : filter)

  if (book.isPending || proposals.isPending) {
    return <Skeleton className="m-6 h-64" />
  }
  if (book.isError || proposals.isError || book.data === undefined) {
    return (
      <div className="p-6">
        <EmptyState title={t("common:error_title")} description={t("common:error_body")} />
      </div>
    )
  }

  const list = proposals.data ?? []
  const unspaced = isUnspaced(book.data.writing_language)

  return (
    <div className="mx-auto max-w-3xl space-y-4 p-6">
      <header className="space-y-1">
        <h1 className="text-xl font-semibold">{t("book:proposals")}</h1>
        <p className="text-xs text-muted-foreground">{t("book:proposal.description")}</p>
      </header>
      <Tabs value={filter} onValueChange={(value) => setFilter(value as Filter)}>
        <TabsList variant="line">
          {FILTERS.map((value) => (
            <TabsTrigger key={value} value={value}>
              {t(`book:proposal.filter.${value}`)}
            </TabsTrigger>
          ))}
        </TabsList>
      </Tabs>
      {list.length === 0 ? (
        <EmptyState
          title={t("book:proposal.empty_title")}
          description={t("book:proposal.empty_body")}
        />
      ) : (
        <div className="space-y-3">
          {list.map((proposal) => (
            <ProposalCard
              key={proposal.id}
              proposal={proposal}
              bookId={bookId}
              unspaced={unspaced}
            />
          ))}
        </div>
      )}
    </div>
  )
}
