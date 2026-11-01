/** Book › Knowledge (PROJECT §8.10): the scopes this book's writer and reviewer may search. */
import { useState } from "react"
import { Link, useParams } from "react-router"
import { useTranslation } from "react-i18next"
import { BookOpen, ExternalLink } from "lucide-react"

import { useBookScopes, usePutBookScopes, useScopes } from "@/api/hooks/knowledge"
import { useErrorToast } from "@/components/common/ErrorToast"
import { EmptyState } from "@/components/common/EmptyState"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Skeleton } from "@/components/ui/skeleton"
import { NewResearchTaskDialog } from "@/features/research/NewResearchTaskDialog"
import { ResearchTaskList } from "@/features/research/ResearchTaskList"

export default function BookKnowledgePage() {
  const { t } = useTranslation()
  const { bookId: rawBookId } = useParams()
  const bookId = Number(rawBookId)
  const showError = useErrorToast()
  const scopes = useScopes()
  const attached = useBookScopes(bookId)
  const save = usePutBookScopes(bookId)
  // The attached set is the initial selection; a toggle overrides it (derived state).
  const [selection, setSelection] = useState<number[] | null>(null)
  const [saved, setSaved] = useState(false)
  const [creating, setCreating] = useState(false)

  if (scopes.isPending || attached.isPending || attached.data === undefined) {
    return <Skeleton className="m-6 h-64" />
  }

  const all = scopes.data ?? []
  const selected = selection ?? attached.data.map((scope) => scope.id)

  return (
    <div className="mx-auto max-w-2xl space-y-4 p-6">
      <div>
        <h2 className="text-base font-semibold">{t("knowledge:book_scopes_title")}</h2>
        <p className="text-sm text-muted-foreground">{t("knowledge:book_scopes_hint")}</p>
      </div>

      {all.length === 0 ? (
        <EmptyState
          title={t("knowledge:no_scopes")}
          description={t("knowledge:no_scopes_hint")}
        >
          <Button asChild size="sm">
            <Link to="/knowledge">{t("knowledge:open_global")}</Link>
          </Button>
        </EmptyState>
      ) : (
        <div className="space-y-1 rounded-md border p-2">
          {all.map((scope) => (
            <label
              key={scope.id}
              className="flex cursor-pointer items-start gap-2 rounded-md px-2 py-1.5 hover:bg-accent"
            >
              <Checkbox
                checked={selected.includes(scope.id)}
                onCheckedChange={(checked) => {
                  setSaved(false)
                  setSelection(
                    checked === true
                      ? [...selected, scope.id]
                      : selected.filter((id) => id !== scope.id),
                  )
                }}
              />
              <span className="min-w-0">
                <span className="flex items-center gap-1.5 text-[13px] font-medium">
                  <BookOpen className="size-3.5" aria-hidden="true" />
                  {scope.name}
                </span>
                {scope.description === "" ? null : (
                  <span className="block text-xs text-muted-foreground">{scope.description}</span>
                )}
                <span className="block text-[0.7rem] text-muted-foreground">
                  {t("knowledge:entry_count", { n: scope.entry_count })}
                </span>
              </span>
            </label>
          ))}
        </div>
      )}

      <div className="flex items-center gap-2">
        <Button
          disabled={save.isPending}
          onClick={() =>
            save.mutate(
              all.filter((scope) => !selected.includes(scope.id)).map((scope) => scope.id),
              {
              onSuccess: () => setSaved(true),
              onError: showError,
              },
            )
          }
        >
          {t("common:save")}
        </Button>
        {saved ? (
          <span className="text-xs text-muted-foreground">{t("knowledge:book_scopes_saved")}</span>
        ) : null}
        <Button asChild variant="link" size="sm" className="ml-auto">
          <Link to="/knowledge">
            <ExternalLink className="size-3.5" /> {t("knowledge:open_global")}
          </Link>
        </Button>
      </div>

      {/* The book's research tasks (PROJECT §8.10): they collect into the scopes above. */}
      <ResearchTaskList bookId={bookId} onCreate={() => setCreating(true)} />
      <NewResearchTaskDialog
        open={creating}
        onOpenChange={setCreating}
        bookId={bookId}
      />
    </div>
  )
}
