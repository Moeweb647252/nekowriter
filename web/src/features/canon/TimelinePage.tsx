/** The canon timeline: a vertical rail with drag-and-drop reorder and "+ Add" between rows
 * (PROJECT §8.8, spec/10 §2.4). */
import { useState } from "react"
import { useParams } from "react-router"
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
  useMoveTimelineEvent,
  useTimeline,
} from "@/api/hooks/useCanon"
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
  const timeline = useTimeline(bookId)
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
        <Button size="sm" onClick={() => setInsertAt(rows.length)}>
          <Plus className="size-3.5" aria-hidden="true" />
          {t("canon:add_event")}
        </Button>
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
                  <TimelineRow bookId={bookId} event={row} />
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
function TimelineRow({ bookId, event }: { bookId: number; event: TimelineEventDto }) {
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
          {event.story_time === "" ? null : (
            <span className="shrink-0 text-xs text-muted-foreground">{event.story_time}</span>
          )}
          {event.chapter_id === null ? null : (
            <span className="shrink-0 text-xs text-muted-foreground">
              {t("canon:chapter_number", { number: event.chapter_id })}
            </span>
          )}
        </div>
        {event.description === "" ? null : (
          <p className="text-xs text-muted-foreground">{event.description}</p>
        )}
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
