/** The canon timeline: a vertical rail with drag-and-drop reorder and "+ Add" between rows
 * (PROJECT §8.8, spec/10 §2.4). */
import { useState } from "react"
import { Link, useParams } from "react-router"
import { useTranslation } from "react-i18next"
import {
  DndContext,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core"
import {
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable"
import { CSS } from "@dnd-kit/utilities"
import { GripVertical, Plus, Trash2 } from "lucide-react"

import type { TimelineEventDto } from "@/api/gen/TimelineEventDto"
import {
  useCreateTimelineEvent,
  useDeleteTimelineEvent,
  useEntities,
  useMoveTimelineEvent,
  useTimeline,
} from "@/api/hooks/useCanon"
import { useTracks } from "@/api/hooks/tracks"
import {
  chapterRefLabel,
  useChapterIndex,
  type ChapterIndex,
} from "@/features/canon/chapterIndex"
import { GoalsReferencedBy } from "@/features/canon/GoalsReferencedBy"
import { useErrorToast } from "@/components/common/ErrorToast"
import { EmptyState } from "@/components/common/EmptyState"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Skeleton } from "@/components/ui/skeleton"
import { isComposing } from "@/lib/hotkeys"
import { cn } from "@/lib/utils"

export default function TimelinePage() {
  const { t } = useTranslation()
  const { bookId: rawBookId } = useParams()
  const bookId = Number(rawBookId)
  // The canon pages show shared canon by default; the track filter adds an isolated side
  // track's private rows, badged (spec/06 §2). A chapter is only visible through its own track.
  const [trackId, setTrackId] = useState<number | undefined>(undefined)
  const timeline = useTimeline(bookId, trackId)
  const entities = useEntities(bookId)
  const tracks = useTracks(bookId)
  const isolatedTracks = (tracks.data ?? []).filter(
    (track) => track.kind === "side" && track.canon_mode === "isolated",
  )
  const chapterIndex = useChapterIndex(tracks.data)
  const entityNames = new Map(
    (entities.data ?? []).map((entity) => [entity.id, entity.name]),
  )
  const move = useMoveTimelineEvent(bookId)
  const create = useCreateTimelineEvent(bookId)
  const showError = useErrorToast()
  const [insertAt, setInsertAt] = useState<number | null>(null)

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  )

  if (timeline.isPending) {
    return <Skeleton className="m-6 h-64" />
  }
  const rows = timeline.data ?? []

  const onDragEnd = (event: DragEndEvent) => {
    const { active, over } = event
    if (over === null || active.id === over.id) {
      return
    }
    const from = rows.findIndex((row) => row.id === active.id)
    const to = rows.findIndex((row) => row.id === over.id)
    if (from < 0 || to < 0) {
      return
    }
    // Story order is what the list shows: place the moved event after the row it lands on when it
    // moves down, and before it when it moves up (spec/10 §2.4 `move`).
    const target = rows[to]
    const before = from < to ? undefined : target.id
    const after = from < to ? target.id : undefined
    move.mutate(
      { id: rows[from].id, afterId: after, beforeId: before },
      { onError: showError },
    )
  }

  return (
    <div className="mx-auto max-w-3xl p-6">
      <header className="mb-4 flex items-end justify-between gap-4">
        <div>
          <h1 className="text-xl font-semibold">{t("canon:timeline")}</h1>
          <p className="text-xs text-muted-foreground">{t("canon:timeline_hint")}</p>
        </div>
        <div className="flex items-center gap-2">
          {isolatedTracks.length === 0 ? null : (
            <select
              value={trackId ?? ""}
              onChange={(event) =>
                setTrackId(
                  event.target.value === "" ? undefined : Number(event.target.value),
                )
              }
              className="h-8 rounded-md border bg-background px-2 text-xs"
              aria-label={t("canon:filter_track")}
            >
              <option value="">{t("canon:track_shared")}</option>
              {isolatedTracks.map((track) => (
                <option key={track.id} value={track.id}>
                  {track.title}
                </option>
              ))}
            </select>
          )}
          <Button size="sm" onClick={() => setInsertAt(rows.length)}>
            <Plus className="size-3.5" aria-hidden="true" />
            {t("canon:add_event")}
          </Button>
        </div>
      </header>

      {rows.length === 0 && insertAt === null ? (
        <EmptyState title={t("canon:no_events")} description={t("canon:no_events_hint")} />
      ) : (
        <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={onDragEnd}>
          <SortableContext
            items={rows.map((row) => row.id)}
            strategy={verticalListSortingStrategy}
          >
            <ol className="relative space-y-1 border-l pl-4">
              {rows.map((row, index) => (
                <li key={row.id} className="relative">
                  <InsertRow
                    active={insertAt === index}
                    onOpen={() => setInsertAt(index)}
                    onCancel={() => setInsertAt(null)}
                    onSubmit={(title) =>
                      create.mutate(
                        {
                          title,
                          description: "",
                          story_time: "",
                          importance: 2,
                          entity_ids: [],
                          after_id: index === 0 ? undefined : rows[index - 1].id,
                          before_id: index === 0 ? rows[0]?.id : undefined,
                        },
                        {
                          onSuccess: () => setInsertAt(null),
                          onError: showError,
                        },
                      )
                    }
                  />
                  <TimelineRow
                    bookId={bookId}
                    event={row}
                    chapterIndex={chapterIndex}
                    entityNames={entityNames}
                  />
                </li>
              ))}
              {insertAt === rows.length ? (
                <li>
                  <InsertRow
                    active
                    onOpen={() => setInsertAt(rows.length)}
                    onCancel={() => setInsertAt(null)}
                    onSubmit={(title) =>
                      create.mutate(
                        {
                          title,
                          description: "",
                          story_time: "",
                          importance: 2,
                          entity_ids: [],
                          after_id: rows[rows.length - 1]?.id,
                        },
                        {
                          onSuccess: () => setInsertAt(null),
                          onError: showError,
                        },
                      )
                    }
                  />
                </li>
              ) : null}
            </ol>
          </SortableContext>
        </DndContext>
      )}
    </div>
  )
}

/** One draggable timeline row. */
function TimelineRow({
  bookId,
  event,
  chapterIndex,
  entityNames,
}: {
  bookId: number
  event: TimelineEventDto
  chapterIndex: ChapterIndex
  entityNames: Map<number, string>
}) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const remove = useDeleteTimelineEvent(bookId)
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: event.id,
  })
  return (
    <div
      ref={setNodeRef}
      style={{ transform: CSS.Transform.toString(transform), transition }}
      className={cn(
        "group flex items-start gap-2 rounded-md px-2 py-1.5 hover:bg-accent/40",
        isDragging && "opacity-60",
      )}
    >
      <button
        type="button"
        className="mt-0.5 cursor-grab text-muted-foreground opacity-0 group-hover:opacity-100"
        aria-label={t("canon:drag_event")}
        {...attributes}
        {...listeners}
      >
        <GripVertical className="size-3.5" aria-hidden="true" />
      </button>
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          <span className="truncate text-sm font-medium">{event.title}</span>
          {event.track_id === null ? null : (
            <span className="shrink-0 rounded bg-muted px-1 text-[0.65rem] text-muted-foreground">
              {t("canon:private_badge")}
            </span>
          )}
          {event.story_time === "" ? null : (
            <span className="shrink-0 text-xs text-muted-foreground">{event.story_time}</span>
          )}
          {event.chapter_id === null ? null : (
            <Link
              to={`/books/${bookId}/chapters/${event.chapter_id}`}
              className="shrink-0 text-xs text-muted-foreground underline-offset-2 hover:underline"
            >
              {chapterRefLabel(t, chapterIndex, event.chapter_id)}
            </Link>
          )}
        </div>
        {event.description === "" ? null : (
          <p className="text-xs text-muted-foreground">{event.description}</p>
        )}
        {event.entity_ids.length === 0 ? null : (
          <div className="mt-1 flex flex-wrap gap-1">
            {event.entity_ids.map((id) => (
              <span
                key={id}
                className="rounded-full bg-muted px-1.5 py-0.5 text-[0.65rem] text-muted-foreground"
              >
                {entityNames.get(id) ?? t("canon:entity_ref", { id })}
              </span>
            ))}
          </div>
        )}
        <GoalsReferencedBy bookId={bookId} eventId={event.id} />
      </div>
      <Button
        size="sm"
        variant="ghost"
        className="opacity-0 group-hover:opacity-100"
        aria-label={t("canon:delete_event")}
        onClick={() => remove.mutate(event.id, { onError: showError })}
      >
        <Trash2 className="size-3" aria-hidden="true" />
      </Button>
    </div>
  )
}

/** The "+ Add" affordance between two rows (PROJECT §8.8). */
function InsertRow({
  active,
  onOpen,
  onCancel,
  onSubmit,
}: {
  active: boolean
  onOpen: () => void
  onCancel: () => void
  onSubmit: (title: string) => void
}) {
  const { t } = useTranslation()
  const [title, setTitle] = useState("")
  if (!active) {
    return (
      <button
        type="button"
        onClick={onOpen}
        className="my-0.5 flex w-full items-center gap-1 text-[0.65rem] text-muted-foreground opacity-0 hover:opacity-100"
      >
        <Plus className="size-3" aria-hidden="true" />
        {t("canon:insert_here")}
      </button>
    )
  }
  return (
    <form
      className="my-1 flex items-center gap-1"
      onSubmit={(event) => {
        event.preventDefault()
        if (title.trim() !== "") {
          onSubmit(title.trim())
          setTitle("")
        }
      }}
    >
      <Input
        autoFocus
        value={title}
        onChange={(event) => setTitle(event.target.value)}
        onKeyDown={(event) => {
          if (isComposing(event)) {
            return
          }
          if (event.key === "Escape") {
            onCancel()
          }
        }}
        placeholder={t("canon:event_title")}
        className="h-7 text-sm"
      />
      <Button size="sm" type="submit" disabled={title.trim() === ""}>
        {t("common:save")}
      </Button>
      <Button size="sm" type="button" variant="ghost" onClick={onCancel}>
        {t("common:cancel")}
      </Button>
    </form>
  )
}
