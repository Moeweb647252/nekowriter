/** The canon entity page: a filter bar over a dense table, with the detail pane on the right
 * (PROJECT §8.8, spec/11 §5.1). */
import { useState } from "react"
import { useNavigate, useParams } from "react-router"
import { useTranslation } from "react-i18next"
import { Plus, Star, UserRound, Users } from "lucide-react"

import type { EntityListItemDto } from "@/api/gen/EntityListItemDto"
import type { EntityKind } from "@/api/gen/EntityKind"
import type { EntityStatus } from "@/api/gen/EntityStatus"
import { useEntities, useSetProtagonist } from "@/api/hooks/useCanon"
import { useTracks } from "@/api/hooks/tracks"
import { EntityCreateDialog } from "@/features/canon/EntityCreateDialog"
import { EntityDetail } from "@/features/canon/EntityDetail"
import {
  chapterRefLabel,
  useChapterIndex,
  type ChapterIndex,
} from "@/features/canon/chapterIndex"
import { ENTITY_KINDS, ENTITY_STATUSES } from "@/features/canon/entityOptions"
import { EmptyState } from "@/components/common/EmptyState"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Skeleton } from "@/components/ui/skeleton"
import { cn } from "@/lib/utils"

export default function EntitiesPage() {
  const { t } = useTranslation()
  const { bookId: rawBookId, entityId: rawEntityId } = useParams()
  const bookId = Number(rawBookId)
  const selectedId = rawEntityId === undefined ? undefined : Number(rawEntityId)

  const [kind, setKind] = useState<EntityKind | undefined>(undefined)
  const [status, setStatus] = useState<EntityStatus | undefined>(undefined)
  const [query, setQuery] = useState("")
  const [trackId, setTrackId] = useState<number | undefined>(undefined)
  const [createOpen, setCreateOpen] = useState(false)

  const entities = useEntities(bookId, { kind, status, q: query, trackId })
  const tracks = useTracks(bookId)
  const chapterIndex = useChapterIndex(tracks.data)
  const setProtagonist = useSetProtagonist(bookId)

  if (entities.isPending) {
    return <Skeleton className="m-6 h-64" />
  }

  const rows = entities.data ?? []
  const sideTracks = (tracks.data ?? []).filter(
    (track) => track.kind === "side" && track.canon_mode === "isolated",
  )

  return (
    <div className="flex h-full min-h-0">
      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex flex-wrap items-center gap-2 border-b p-3">
          <Input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={t("canon:search_entities")}
            className="h-8 w-56"
            aria-label={t("canon:search_entities")}
          />
          <div className="flex flex-wrap gap-1">
            <FilterChip
              active={kind === undefined}
              onClick={() => setKind(undefined)}
              label={t("canon:all_kinds")}
            />
            {ENTITY_KINDS.map((value) => (
              <FilterChip
                key={value}
                active={kind === value}
                onClick={() => setKind(kind === value ? undefined : value)}
                label={t(`canon:kind.${value}`)}
              />
            ))}
          </div>
          <select
            value={status ?? ""}
            onChange={(event) =>
              setStatus((event.target.value || undefined) as EntityStatus | undefined)
            }
            className="h-8 rounded-md border bg-background px-2 text-xs"
            aria-label={t("canon:filter_status")}
          >
            <option value="">{t("canon:all_statuses")}</option>
            {ENTITY_STATUSES.map((value) => (
              <option key={value} value={value}>
                {t(`canon:status.${value}`)}
              </option>
            ))}
          </select>
          {sideTracks.length === 0 ? null : (
            <select
              value={trackId ?? ""}
              onChange={(event) =>
                setTrackId(event.target.value === "" ? undefined : Number(event.target.value))
              }
              className="h-8 rounded-md border bg-background px-2 text-xs"
              aria-label={t("canon:filter_track")}
            >
              <option value="">{t("canon:track_shared")}</option>
              {sideTracks.map((track) => (
                <option key={track.id} value={track.id}>
                  {track.title}
                </option>
              ))}
            </select>
          )}
          <Button
            size="sm"
            className="ml-auto"
            onClick={() => setCreateOpen(true)}
          >
            <Plus className="size-3.5" aria-hidden="true" />
            {t("canon:entity_create.action")}
          </Button>
        </header>

        {rows.length === 0 ? (
          <div className="p-6">
            <EmptyState
              title={t("canon:no_entities")}
              description={t("canon:no_entities_hint")}
            />
          </div>
        ) : (
          <div className="min-h-0 flex-1 overflow-y-auto">
            <table className="w-full text-sm">
              <thead className="sticky top-0 bg-background/95 text-left text-xs text-muted-foreground">
                <tr className="border-b">
                  <th className="px-3 py-2 font-medium">{t("canon:column_name")}</th>
                  <th className="px-3 py-2 font-medium">{t("canon:column_kind")}</th>
                  <th className="px-3 py-2 font-medium">{t("canon:column_summary")}</th>
                  <th className="px-3 py-2 font-medium">{t("canon:column_importance")}</th>
                  <th className="px-3 py-2 font-medium">{t("canon:column_last_seen")}</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((row) => (
                  <EntityRow
                    key={row.id}
                    bookId={bookId}
                    entity={row}
                    selected={row.id === selectedId}
                    chapterIndex={chapterIndex}
                    onProtagonist={() => setProtagonist.mutate(row.id)}
                  />
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
      <aside className="w-[26rem] shrink-0 overflow-y-auto border-l">
        {selectedId === undefined ? (
          <div className="p-6">
            <EmptyState
              title={t("canon:select_entity")}
              description={t("canon:select_entity_hint")}
            />
          </div>
        ) : (
          <EntityDetail bookId={bookId} entityId={selectedId} />
        )}
      </aside>
      {createOpen ? (
        <EntityCreateDialog
          bookId={bookId}
          open
          onOpenChange={setCreateOpen}
        />
      ) : null}
    </div>
  )
}

/** One filter chip (PROJECT §8.8: kind chips). */
function FilterChip({
  active,
  onClick,
  label,
}: {
  active: boolean
  onClick: () => void
  label: string
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "rounded-full border px-2 py-0.5 text-xs",
        active
          ? "border-primary bg-primary/10 text-primary"
          : "text-muted-foreground hover:bg-accent",
      )}
    >
      {label}
    </button>
  )
}

/** One table row; clicking it selects the entity. */
function EntityRow({
  bookId,
  entity,
  selected,
  chapterIndex,
  onProtagonist,
}: {
  bookId: number
  entity: EntityListItemDto
  selected: boolean
  chapterIndex: ChapterIndex
  onProtagonist: () => void
}) {
  const { t } = useTranslation()
  const navigate = useNavigate()
  return (
    <tr
      className={cn(
        "cursor-pointer border-b last:border-0 hover:bg-accent/50",
        selected && "bg-accent",
      )}
      onClick={() => void navigate(`/books/${bookId}/canon/entities/${entity.id}`)}
    >
      <td className="px-3 py-1.5">
        <span className="flex items-center gap-1.5">
          {entity.is_protagonist ? (
            <Star className="size-3.5 fill-current text-primary" aria-hidden="true" />
          ) : null}
          <span className="truncate">{entity.name}</span>
          {entity.track_id === null ? null : (
            <span className="rounded bg-muted px-1 text-[0.65rem] text-muted-foreground">
              {t("canon:private_badge")}
            </span>
          )}
        </span>
      </td>
      <td className="px-3 py-1.5 text-xs text-muted-foreground">
        {t(`canon:kind.${entity.kind}`)}
      </td>
      <td className="max-w-0 truncate px-3 py-1.5 text-xs text-muted-foreground">
        {entity.summary}
      </td>
      <td className="px-3 py-1.5">
        <Importance value={entity.importance} />
      </td>
      <td className="px-3 py-1.5 text-xs text-muted-foreground">
        {entity.last_chapter_id === null
          ? t("canon:never_seen")
          : chapterRefLabel(t, chapterIndex, entity.last_chapter_id)}
      </td>
      <td className="px-3 py-1.5 text-right">
        {entity.is_protagonist ? (
          <Button
            size="sm"
            variant="ghost"
            onClick={(event) => {
              event.stopPropagation()
              onProtagonist()
            }}
          >
            <Users className="size-3.5" aria-hidden="true" />
          </Button>
        ) : (
          <Button
            size="sm"
            variant="ghost"
            title={t("canon:make_protagonist")}
            onClick={(event) => {
              event.stopPropagation()
              onProtagonist()
            }}
          >
            <UserRound className="size-3.5" aria-hidden="true" />
          </Button>
        )}
      </td>
    </tr>
  )
}

/** The importance stars (PROJECT §8.8). */
export function Importance({ value }: { value: number }) {
  const { t } = useTranslation()
  return (
    <span
      className="inline-flex items-center gap-0.5"
      title={t("canon:importance", { value })}
      aria-label={t("canon:importance", { value })}
    >
      {Array.from({ length: 5 }, (_, index) => (
        <Star
          key={index}
          className={cn(
            "size-3",
            index < value ? "fill-current text-primary" : "text-muted-foreground/40",
          )}
          aria-hidden="true"
        />
      ))}
    </span>
  )
}
