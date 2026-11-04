/** The canon timeline: a tree of history and plan with drag-and-drop reorder among siblings
 * (PROJECT §8.8, spec/10 §2.4, spec/11 §5.5, D-170). */
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
import {
  Ban,
  ChevronDown,
  ChevronRight,
  CircleCheck,
  GripVertical,
  MoreHorizontal,
  Pencil,
  Plus,
  Trash2,
} from "lucide-react"
import { toast } from "sonner"

import type { TimelineEventDto } from "@/api/gen/TimelineEventDto"
import {
  useCreateTimelineEvent,
  useDeleteTimelineEvent,
  useEntities,
  useMoveTimelineEvent,
  usePatchTimelineEvent,
  useTimeline,
} from "@/api/hooks/useCanon"
import { useTracks } from "@/api/hooks/tracks"
import {
  chapterRefLabel,
  useChapterIndex,
  type ChapterIndex,
} from "@/features/canon/chapterIndex"
import { ConfirmDialog } from "@/components/common/ConfirmDialog"
import { GoalsReferencedBy } from "@/features/canon/GoalsReferencedBy"
import { TimelineEventDialog } from "@/features/canon/TimelineEventDialog"
import { descendantsOf, siblingMove } from "@/features/canon/timelineTree"
import { timelineErrorText } from "@/features/canon/timelineErrors"
import { useErrorToast } from "@/components/common/ErrorToast"
import { EmptyState } from "@/components/common/EmptyState"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
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
  const [createParent, setCreateParent] = useState<number | null | undefined>(
    undefined,
  )
  const [showAbandoned, setShowAbandoned] = useState(true)
  const [collapsed, setCollapsed] = useState<Set<number>>(new Set())

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  )

  if (timeline.isPending) {
    return <Skeleton className="m-6 h-64" />
  }
  const rows = (timeline.data ?? []).filter((row) => row.kind !== "root")
  const byId = new Map(rows.map((row) => [row.id, row]))
  const childrenOf = (parentId: number | null) =>
    rows
      .filter((row) => row.parent_id === parentId)
      .sort((left, right) => left.position - right.position || left.id - right.id)
  const visibleRows: Array<{ event: TimelineEventDto; depth: number }> = []
  const walk = (parentId: number | null, depth: number) => {
    for (const row of childrenOf(parentId)) {
      if (!showAbandoned && row.status === "abandoned") {
        continue
      }
      visibleRows.push({ event: row, depth })
      if (!collapsed.has(row.id)) {
        walk(row.id, depth + 1)
      }
    }
  }
  walk(null, 0)
  const toggle = (id: number) =>
    setCollapsed((current) => {
      const next = new Set(current)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })

  const onDragEnd = (event: DragEndEvent) => {
    const { active, over } = event
    if (over === null || active.id === over.id) {
      return
    }
    const activeRow = byId.get(Number(active.id))
    const overRow = byId.get(Number(over.id))
    // The tree never changes parent here: a drop outside the row's sibling list is ignored.
    if (
      activeRow === undefined ||
      overRow === undefined ||
      activeRow.parent_id !== overRow.parent_id
    ) {
      return
    }
    const siblings = childrenOf(activeRow.parent_id)
    const payload = siblingMove(activeRow, overRow, siblings)
    if (payload === null) {
      return
    }
    // Story order is what the list shows: place the moved event after the row it lands on when it
    // moves down, and before it when it moves up (spec/10 §2.4 `move`).
    move.mutate(
      { id: activeRow.id, ...payload },
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
          <label className="flex items-center gap-1 text-xs text-muted-foreground">
            <input
              type="checkbox"
              checked={showAbandoned}
              onChange={(event) => setShowAbandoned(event.target.checked)}
            />
            {t("canon:show_abandoned")}
          </label>
          <Button size="sm" onClick={() => setCreateParent(null)}>
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
            items={visibleRows.map(({ event }) => event.id)}
            strategy={verticalListSortingStrategy}
          >
            <ol className="relative space-y-1 border-l pl-4">
              {visibleRows.map(({ event: row, depth }, index) => {
                const hasChildren = row.child_count > 0
                const isCollapsed = collapsed.has(row.id)
                return (
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
                            parent_id: row.parent_id,
                            after_id: index === 0 ? undefined : visibleRows[index - 1]?.event.id,
                          },
                          {
                            onSuccess: () => setInsertAt(null),
                            onError: showError,
                          },
                        )
                      }
                    />
                    <div style={{ marginLeft: depth * 16 }}>
                      <TimelineRow
                        bookId={bookId}
                        event={row}
                        events={rows}
                        chapterIndex={chapterIndex}
                        entityNames={entityNames}
                        expanded={!isCollapsed}
                        hasChildren={hasChildren}
                        onToggle={() => toggle(row.id)}
                        onAddChild={() => setCreateParent(row.id)}
                      />
                    </div>
                  </li>
                )
              })}
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

      {createParent === undefined ? null : (
        <TimelineEventDialog
          bookId={bookId}
          event={null}
          events={rows}
          parentId={createParent}
          chapterIndex={chapterIndex}
          open
          onOpenChange={(open) => {
            if (!open) setCreateParent(undefined)
          }}
        />
      )}
    </div>
  )
}

/** One draggable timeline row and its actions. */
function TimelineRow({
  bookId,
  event,
  events,
  chapterIndex,
  entityNames,
  expanded,
  hasChildren,
  onToggle,
  onAddChild,
}: {
  bookId: number
  event: TimelineEventDto
  events: TimelineEventDto[]
  chapterIndex: ChapterIndex
  entityNames: Map<number, string>
  expanded: boolean
  hasChildren: boolean
  onToggle: () => void
  onAddChild: () => void
}) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const remove = useDeleteTimelineEvent(bookId)
  const patch = usePatchTimelineEvent(bookId)
  const [editing, setEditing] = useState<{ status?: TimelineEventDto["status"] } | null>(
    null,
  )
  const [abandoning, setAbandoning] = useState(false)
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: event.id,
  })
  const plannedDescendants = descendantsOf(event.id, events).filter(
    (row) => row.status === "planned",
  )
  const deleteError = (error: unknown) => {
    const message = timelineErrorText(t, error)
    if (message !== null) {
      toast.error(message)
      return
    }
    showError(error)
  }
  return (
    <>
      <div
        ref={setNodeRef}
        style={{ transform: CSS.Transform.toString(transform), transition }}
        className={cn(
          "group flex items-start gap-2 rounded-md px-2 py-1.5 hover:bg-accent/40",
          isDragging && "opacity-60",
          event.status === "planned" && "border border-dashed",
          event.status === "abandoned" && "opacity-60 line-through",
        )}
      >
        <button
          type="button"
          className="mt-0.5 text-muted-foreground"
          aria-label={expanded ? t("canon:collapse") : t("canon:expand")}
          onClick={onToggle}
        >
          {hasChildren ? (
            expanded ? (
              <ChevronDown className="size-3.5" />
            ) : (
              <ChevronRight className="size-3.5" />
            )
          ) : (
            <span className="inline-block size-3.5" />
          )}
        </button>
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
            {event.status === "planned" ? (
              <span className="shrink-0 rounded border border-dashed px-1 text-[0.65rem]">
                {t("canon:planned_badge")}
              </span>
            ) : null}
            {event.status === "abandoned" ? (
              <span className="shrink-0 rounded bg-muted px-1 text-[0.65rem]">
                {t("canon:abandoned_badge")}
              </span>
            ) : null}
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
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              size="sm"
              variant="ghost"
              aria-label={t("canon:timeline_actions", { title: event.title })}
            >
              <MoreHorizontal className="size-3.5" aria-hidden="true" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onSelect={() => setEditing({})}>
              <Pencil className="size-3.5" />
              {t("canon:timeline_event_dialog.edit")}
            </DropdownMenuItem>
            <DropdownMenuItem
              disabled={event.status === "abandoned"}
              onSelect={onAddChild}
            >
              <Plus className="size-3.5" />
              {t("canon:add_sub_entry")}
            </DropdownMenuItem>
            {event.status === "planned" ? (
              <>
                <DropdownMenuItem
                  onSelect={() => setEditing({ status: "occurred" })}
                >
                  <CircleCheck className="size-3.5" />
                  {t("canon:mark_happened")}
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={() => setAbandoning(true)}>
                  <Ban className="size-3.5" />
                  {t("canon:abandon")}
                </DropdownMenuItem>
              </>
            ) : null}
            <DropdownMenuSeparator />
            <DropdownMenuItem
              variant="destructive"
              disabled={event.child_count > 0}
              title={
                event.child_count > 0
                  ? t("canon:delete_has_children_hint", {
                      count: event.child_count,
                    })
                  : undefined
              }
              onSelect={() => {
                if (event.child_count === 0) {
                  remove.mutate(event.id, { onError: deleteError })
                }
              }}
            >
              <Trash2 className="size-3.5" />
              {t("common:delete")}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
      {editing === null ? null : (
        <TimelineEventDialog
          bookId={bookId}
          event={event}
          events={events}
          initialStatus={editing.status}
          chapterIndex={chapterIndex}
          open
          onOpenChange={(open) => {
            if (!open) setEditing(null)
          }}
        />
      )}
      <ConfirmDialog
        open={abandoning}
        onOpenChange={setAbandoning}
        title={t("canon:abandon_confirm_title")}
        description={t("canon:abandon_confirm_body")}
        confirmLabel={t("canon:abandon")}
        pending={patch.isPending}
        details={
          plannedDescendants.length === 0 ? undefined : (
            <div>
              <p>{t("canon:abandon_sub_entries")}</p>
              <ul className="list-disc pl-4">
                {plannedDescendants.map((row) => (
                  <li key={row.id}>{row.title}</li>
                ))}
              </ul>
            </div>
          )
        }
        onConfirm={() =>
          patch.mutate(
            {
              id: event.id,
              patch: {
                status: "abandoned",
                note: t("canon:abandon_note"),
              },
            },
            {
              onSuccess: () => setAbandoning(false),
              onError: deleteError,
            },
          )
        }
      />
    </>
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
