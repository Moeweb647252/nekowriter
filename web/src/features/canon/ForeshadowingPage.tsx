/** The foreshadowing page: three tabs (Open, Resolved, Abandoned) with planted and resolved
 * chapter links and the planned payoff (PROJECT §8.8, spec/10 §2.4). */
import { useState } from "react"
import { Link, useParams } from "react-router"
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
import { useTracks } from "@/api/hooks/tracks"
import {
  chapterRefLabel,
  useChapterIndex,
  type ChapterIndex,
} from "@/features/canon/chapterIndex"
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
  const tracks = useTracks(bookId)
  const chapterIndex = useChapterIndex(tracks.data)
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
                    bookId={bookId}
                    item={item}
                    chapterIndex={chapterIndex}
                    onResolve={(resolution, resolvedChapterId) =>
                      patch.mutate(
                        {
                          id: item.id,
                          patch: {
                            status: "resolved",
                            resolution,
                            resolved_chapter_id: resolvedChapterId ?? null,
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
  bookId,
  item,
  chapterIndex,
  onResolve,
  onAbandon,
  onDelete,
}: {
  bookId: number
  item: ForeshadowingDto
  chapterIndex: ChapterIndex
  onResolve: (resolution: string, resolvedChapterId?: number) => void
  onAbandon: () => void
  onDelete: () => void
}) {
  const { t } = useTranslation()
  // Resolving stores the author's own words (spec/06 §1.5): the resolution is written text, so
  // the UI asks for it instead of inserting a localized phrase into canon.
  const [resolving, setResolving] = useState(false)
  const [resolution, setResolution] = useState("")
  const [resolvedChapterId, setResolvedChapterId] = useState<number | undefined>(undefined)
  const chapters = [...chapterIndex.values()].sort(
    (a, b) => a.trackId - b.trackId || a.number - b.number,
  )
  return (
    <li className="flex items-start gap-2 rounded-md border p-2">
      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium">{item.title}</p>
        {item.description === "" ? null : (
          <p className="text-xs text-muted-foreground">{item.description}</p>
        )}
        <p className="mt-0.5 text-xs text-muted-foreground">
          {item.planted_chapter_id === null ? (
            t("canon:not_planted")
          ) : (
            <Link
              to={`/books/${bookId}/chapters/${item.planted_chapter_id}`}
              className="underline-offset-2 hover:underline"
            >
              {t("canon:planted_at", {
                label: chapterRefLabel(t, chapterIndex, item.planted_chapter_id),
              })}
            </Link>
          )}
          {item.planned_payoff === ""
            ? ""
            : ` · ${t("canon:planned_payoff", { text: item.planned_payoff })}`}
          {item.resolved_chapter_id === null ? (
            ""
          ) : (
            <>
              {" · "}
              <Link
                to={`/books/${bookId}/chapters/${item.resolved_chapter_id}`}
                className="underline-offset-2 hover:underline"
              >
                {t("canon:resolved_at", {
                  label: chapterRefLabel(t, chapterIndex, item.resolved_chapter_id),
                })}
              </Link>
            </>
          )}
        </p>
        {item.resolution === "" ? null : (
          <p className="text-xs text-muted-foreground">{item.resolution}</p>
        )}
      </div>
      {item.status !== "open" ? null : resolving ? (
        <form
          className="flex items-center gap-1"
          onSubmit={(event) => {
            event.preventDefault()
            if (resolution.trim() !== "") {
              onResolve(resolution.trim(), resolvedChapterId)
              setResolving(false)
              setResolution("")
              setResolvedChapterId(undefined)
            }
          }}
        >
          <Input
            autoFocus
            value={resolution}
            onChange={(event) => setResolution(event.target.value)}
            placeholder={t("canon:resolve_prompt")}
            className="h-7 w-56 text-xs"
          />
          <select
            value={resolvedChapterId ?? ""}
            onChange={(event) =>
              setResolvedChapterId(
                event.target.value === "" ? undefined : Number(event.target.value),
              )
            }
            className="h-7 rounded-md border bg-background px-2 text-xs"
            aria-label={t("canon:resolve_chapter")}
          >
            <option value="">{t("canon:no_chapter")}</option>
            {chapters.map((chapter) => (
              <option key={chapter.id} value={chapter.id}>
                {chapterRefLabel(t, chapterIndex, chapter.id)}
              </option>
            ))}
          </select>
          <Button size="sm" type="submit" disabled={resolution.trim() === ""}>
            {t("common:save")}
          </Button>
          <Button
            size="sm"
            type="button"
            variant="ghost"
            onClick={() => {
              setResolving(false)
              setResolution("")
              setResolvedChapterId(undefined)
            }}
          >
            {t("common:cancel")}
          </Button>
        </form>
      ) : (
        <div className="flex gap-1">
          <Button size="sm" variant="outline" onClick={() => setResolving(true)}>
            {t("canon:resolve")}
          </Button>
          <Button size="sm" variant="ghost" onClick={onAbandon}>
            {t("canon:abandon")}
          </Button>
        </div>
      )}
      <Button size="sm" variant="ghost" aria-label={t("common:delete")} onClick={onDelete}>
        <Trash2 className="size-3" aria-hidden="true" />
      </Button>
    </li>
  )
}
