/** The book's prompt overrides (PROJECT §8.11): the same editor over one book's level. */
import { useParams } from "react-router"
import { useTranslation } from "react-i18next"

import { useBook } from "@/api/hooks/books"
import { PromptEditor } from "@/features/prompts/PromptEditor"
import { Skeleton } from "@/components/ui/skeleton"

export default function BookPromptsPage() {
  const { bookId: rawBookId } = useParams()
  const bookId = Number(rawBookId)
  const { t } = useTranslation()
  const book = useBook(bookId)

  return (
    <div className="h-full overflow-y-auto p-6">
      <div className="mb-4">
        <h1 className="text-lg font-semibold">
          {t("prompts:book_title", { title: book.data?.title ?? "" })}
        </h1>
        <p className="text-sm text-muted-foreground">{t("prompts:book_hint")}</p>
      </div>
      {book.isPending ? <Skeleton className="h-96" /> : <PromptEditor bookId={bookId} bookScope />}
    </div>
  )
}
