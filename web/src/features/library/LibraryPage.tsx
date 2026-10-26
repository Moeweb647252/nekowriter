/** The book library (PROJECT §8.3, spec/11 §3). */
import { useState } from "react"
import { useTranslation } from "react-i18next"

import { useBooks } from "@/api/hooks/books"
import { BookCard } from "@/features/library/BookCard"
import { NewBookDialog } from "@/features/library/NewBookDialog"
import { EmptyState } from "@/components/common/EmptyState"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Label } from "@/components/ui/label"
import { Skeleton } from "@/components/ui/skeleton"

export default function LibraryPage() {
  const { t } = useTranslation()
  const [showArchived, setShowArchived] = useState(false)
  // No `status` filter: the server's filter is exact (`?status=conceiving|active|archived`), so
  // asking for `active` also dropped books still in conception and the author could not reach
  // them from the library until finalize (D-129). The whole list is small — one author's books —
  // and the archived ones are hidden in the browser below, which is what "Show archived" means.
  const books = useBooks()
  const [creating, setCreating] = useState(false)

  const list = books.data ?? []
  const visible = showArchived
    ? list
    : list.filter((book) => book.status !== "archived")

  return (
    <div className="mx-auto max-w-6xl p-6">
      <div className="mb-6 flex items-end justify-between gap-4">
        <div>
          <h1 className="text-2xl font-semibold">{t("library:title")}</h1>
          <p className="mt-1 text-sm text-muted-foreground">{t("library:subtitle")}</p>
        </div>
        <div className="flex items-center gap-4">
          <div className="flex items-center gap-2">
            <Checkbox
              id="show-archived"
              checked={showArchived}
              onCheckedChange={(checked) => setShowArchived(checked === true)}
            />
            <Label htmlFor="show-archived" className="text-sm text-muted-foreground">
              {t("library:show_archived")}
            </Label>
          </div>
          <Button onClick={() => setCreating(true)}>{t("library:new_book")}</Button>
        </div>
      </div>

      {books.isPending ? (
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
          <Skeleton className="h-40" />
          <Skeleton className="h-40" />
          <Skeleton className="h-40" />
        </div>
      ) : books.isError ? (
        <EmptyState title={t("common:error_title")} description={t("common:error_body")}>
          <Button variant="outline" onClick={() => void books.refetch()}>
            {t("common:retry")}
          </Button>
        </EmptyState>
      ) : visible.length === 0 ? (
        <EmptyState
          title={showArchived ? t("library:no_books_filtered") : t("library:empty_title")}
          description={t("library:empty_body")}
        >
          <Button onClick={() => setCreating(true)}>{t("library:new_book")}</Button>
        </EmptyState>
      ) : (
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {visible.map((book) => (
            <BookCard key={book.id} book={book} />
          ))}
        </div>
      )}

      <NewBookDialog open={creating} onOpenChange={setCreating} />
    </div>
  )
}
