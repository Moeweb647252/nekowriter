/** The foreshadowing page: three tabs (Open, Resolved, Abandoned) with planted and resolved
 * chapter links and the planned payoff (PROJECT §8.8, spec/10 §2.4). */
import { useState } from "react"
import { useParams } from "react-router"
import { useTranslation } from "react-i18next"
import { Plus, Trash2 } from "lucide-react"

import type { ForeshadowingDto } from "@/api/gen/ForeshadowingDto"
import type { ForeshadowingStatus } from "@/api/gen/ForeshadowingStatus"
import {
  useCreateForeshadowing,
  useDeleteForeshadowing,
  useForeshadowing,
  usePatchForeshadowing,
} from "@/api/hooks/useCanon"
import { useErrorToast } from "@/components/common/ErrorToast"
import { EmptyState } from "@/components/common/EmptyState"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Skeleton } from "@/components/ui/skeleton"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"

const STATUSES: ForeshadowingStatus[] = ["open", "resolved", "abandoned"]

export default function ForeshadowingPage() {
  const { t } = useTranslation()
  const { bookId: rawBookId } = useParams()
  const bookId = Number(rawBookId)
  const [status, setStatus] = useState<ForeshadowingStatus>("open")
  const items = useForeshadowing(bookId, status)
  const create = useCreateForeshadowing(bookId)
  const patch = usePatchForeshadowing(bookId)
  const remove = useDeleteForeshadowing(bookId)
  const showError = useErrorToast()
  const [draft, setDraft] = useState("")

  if (items.isPending) {
    return <Skeleton className="m-6 h-64" />
  }
  const rows = items.data ?? []

  return (
    <div className="mx-auto max-w-4xl p-6">
      <header className="mb-3">
        <h1 className="text-xl font-semibold">{t("canon:foreshadowing")}</h1>
        <p className="text-xs text-muted-foreground">{t("canon:foreshadowing_hint")}</p>
      </header>

      <Tabs
        value={status}
        onValueChange={(value) => setStatus(value as ForeshadowingStatus)}
      >
        <TabsList>
          {STATUSES.map((value) => (
            <TabsTrigger key={value} value={value}>
              {t(`canon:foreshadowing_status.${value}`)}
            </TabsTrigger>
          ))}
        </TabsList>
        {STATUSES.map((value) => (
          <TabsContent key={value} value={value} className="space-y-2">
            {value === "open" ? (
              <form
                className="flex items-center gap-1"
                onSubmit={(event) => {
                  event.preventDefault()
                  if (draft.trim() === "") {
                    return
                  }
                  create.mutate(
                    {
                      title: draft.trim(),
                      description: "",
                      planned_payoff: "",
                      importance: 2,
                      entity_ids: [],
                    },
                    { onSuccess: () => setDraft(""), onError: showError },
                  )
                }}
              >
                <Input
                  value={draft}
                  onChange={(event) => setDraft(event.target.value)}
                  placeholder={t("canon:foreshadowing_placeholder")}
                  className="h-8 flex-1"
                />
                <Button size="sm" type="submit" disabled={draft.trim() === ""}>
                  <Plus className="size-3.5" aria-hidden="true" />
                  {t("canon:add_foreshadowing")}
                </Button>
              </form>
            ) : null}
            {rows.length === 0 ? (
              <EmptyState
                title={t("canon:no_foreshadowing")}
                description={t("canon:no_foreshadowing_hint")}
              />
            ) : (
              <ul className="space-y-1">
                {rows.map((item) => (
                  <ForeshadowingRow
                    key={item.id}
                    item={item}
                    onResolve={() =>
                      patch.mutate(
                        {
                          id: item.id,
                          patch: {
                            status: "resolved",
                            resolution: t("canon:resolved_by_author"),
                          },
                        },
                        { onError: showError },
                      )
                    }
                    onAbandon={() =>
                      patch.mutate(
                        { id: item.id, patch: { status: "abandoned" } },
                        { onError: showError },
                      )
                    }
                    onDelete={() => remove.mutate(item.id, { onError: showError })}
                  />
                ))}
              </ul>
            )}
          </TabsContent>
        ))}
      </Tabs>
    </div>
  )
}

/** One foreshadowing item. */
function ForeshadowingRow({
  item,
  onResolve,
  onAbandon,
  onDelete,
}: {
  item: ForeshadowingDto
  onResolve: () => void
  onAbandon: () => void
  onDelete: () => void
}) {
  const { t } = useTranslation()
  return (
    <li className="flex items-start gap-2 rounded-md border p-2">
      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium">{item.title}</p>
        {item.description === "" ? null : (
          <p className="text-xs text-muted-foreground">{item.description}</p>
        )}
        <p className="mt-0.5 text-xs text-muted-foreground">
          {item.planted_chapter_id === null
            ? t("canon:not_planted")
            : t("canon:planted_at", { number: item.planted_chapter_id })}
          {item.planned_payoff === ""
            ? ""
            : ` · ${t("canon:planned_payoff", { text: item.planned_payoff })}`}
          {item.resolved_chapter_id === null
            ? ""
            : ` · ${t("canon:resolved_at", { number: item.resolved_chapter_id })}`}
        </p>
        {item.resolution === "" ? null : (
          <p className="text-xs text-muted-foreground">{item.resolution}</p>
        )}
      </div>
      {item.status === "open" ? (
        <div className="flex gap-1">
          <Button size="sm" variant="outline" onClick={onResolve}>
            {t("canon:resolve")}
          </Button>
          <Button size="sm" variant="ghost" onClick={onAbandon}>
            {t("canon:abandon")}
          </Button>
        </div>
      ) : null}
      <Button size="sm" variant="ghost" aria-label={t("common:delete")} onClick={onDelete}>
        <Trash2 className="size-3" aria-hidden="true" />
      </Button>
    </li>
  )
}
