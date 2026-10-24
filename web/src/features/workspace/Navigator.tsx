/** The workspace navigator (PROJECT §8.4, spec/11 §5.1): overview, tracks with sortable chapter
 * rows, setting materials, canon, knowledge, proposals, prompts and book settings. */
import { Link, NavLink as RouterNavLink, useNavigate, useParams } from "react-router"
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
  arrayMove,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable"
import { CSS } from "@dnd-kit/utilities"
import {
  Brain,
  FileClock,
  FileText,
  GripVertical,
  History,
  Home,
  Library,
  MessageSquareCode,
  NotebookPen,
  Plus,
  Settings,
  Sparkle,
  Sparkles,
  Users,
} from "lucide-react"

import { useChapters, useCreateChapter, useReorderChapters, useTracks } from "@/api/hooks/tracks"
import type { BookDto } from "@/api/gen/BookDto"
import type { ChapterListItemDto } from "@/api/gen/ChapterListItemDto"
import type { TrackDto } from "@/api/gen/TrackDto"
import { StatusBadge } from "@/components/common/StatusBadge"
import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"

export type NavigatorProps = {
  book: BookDto
}

/** One navigator section heading. */
function SectionTitle({ children }: { children: React.ReactNode }) {
  return (
    <div className="px-2 pt-3 pb-1 text-[0.68rem] font-medium tracking-wide text-muted-foreground uppercase">
      {children}
    </div>
  )
}

type NavLinkProps = {
  to: string
  icon: typeof Home
  label: string
  badge?: number
  /** `true` for a row that must match the whole path (the book overview). */
  end?: boolean
}

/**
 * One navigator row. React Router computes the active state, so the row for the page being shown
 * is highlighted (spec/11 §5.1) — passing `active={false}` everywhere left the navigator with no
 * current-page affordance at all.
 */
function NavLink({ to, icon: Icon, label, badge, end }: NavLinkProps) {
  return (
    <RouterNavLink
      to={to}
      end={end}
      className={({ isActive }) =>
        cn(
          "flex items-center gap-2 rounded-md px-2 py-1 text-[13px] text-muted-foreground hover:bg-accent hover:text-foreground",
          isActive && "bg-accent font-medium text-foreground",
        )
      }
    >
      <Icon className="size-3.5 shrink-0" aria-hidden="true" />
      <span className="truncate">{label}</span>
      {badge === undefined || badge === 0 ? null : (
        <span className="ml-auto rounded-full bg-primary px-1.5 text-[0.65rem] text-primary-foreground">
          {badge}
        </span>
      )}
    </RouterNavLink>
  )
}

/** The navigator of one book. */
export function Navigator({ book }: NavigatorProps) {
  const { t } = useTranslation()
  const tracks = useTracks(book.id)
  const mainTrack = tracks.data?.find((track) => track.kind === "main")

  return (
    <nav className="flex h-full flex-col overflow-y-auto border-r bg-sidebar/40 p-2">
      <NavLink to={`/books/${book.id}`} icon={Home} label={t("book:overview")} end />
      {mainTrack === undefined ? null : (
        <TrackGroup book={book} track={mainTrack} />
      )}
      <SectionTitle>{t("book:settings_materials")}</SectionTitle>
      <NavLink
        to={`/books/${book.id}/settings/outline`}
        icon={FileText}
        label={t("book:outline")}
      />
      <NavLink
        to={`/books/${book.id}/settings/world`}
        icon={Library}
        label={t("book:world")}
      />
      <NavLink
        to={`/books/${book.id}/settings/protagonist`}
        icon={Sparkles}
        label={t("book:protagonist")}
      />
      <NavLink
        to={`/books/${book.id}/settings/characters`}
        icon={Users}
        label={t("book:characters")}
      />
      <SectionTitle>{t("book:canon")}</SectionTitle>
      <NavLink
        to={`/books/${book.id}/canon/entities`}
        icon={Users}
        label={t("book:entities")}
      />
      <NavLink
        to={`/books/${book.id}/canon/timeline`}
        icon={History}
        label={t("canon:timeline")}
      />
      <NavLink
        to={`/books/${book.id}/canon/memory`}
        icon={Brain}
        label={t("canon:memory")}
      />
      <NavLink
        to={`/books/${book.id}/canon/notes`}
        icon={NotebookPen}
        label={t("canon:notes")}
      />
      <NavLink
        to={`/books/${book.id}/canon/foreshadowing`}
        icon={Sparkle}
        label={t("canon:foreshadowing")}
      />
      <NavLink
        to={`/books/${book.id}/canon/changes`}
        icon={FileClock}
        label={t("book:changes")}
      />
      <SectionTitle>{t("book:book_settings")}</SectionTitle>
      <NavLink
        to={`/books/${book.id}/prompts`}
        icon={MessageSquareCode}
        label={t("book:prompts")}
      />
      <NavLink
        to={`/books/${book.id}/book-settings`}
        icon={Settings}
        label={t("book:book_settings")}
      />
    </nav>
  )
}

/** A track with its sortable chapters (official rows are fixed, spec/11 §5.1). */
function TrackGroup({ book, track }: { book: BookDto; track: TrackDto }) {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const params = useParams()
  const chapters = useChapters(track.id)
  const createChapter = useCreateChapter(track.id)
  const reorder = useReorderChapters(book.id, track.id)

  const list = chapters.data ?? []
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  )

  const onDragEnd = (event: DragEndEvent) => {
    const { active, over } = event
    if (over === null || active.id === over.id) {
      return
    }
    const oldIndex = list.findIndex((chapter) => chapter.id === active.id)
    const newIndex = list.findIndex((chapter) => chapter.id === over.id)
    if (oldIndex < 0 || newIndex < 0) {
      return
    }
    // No non-official chapter may end up before an official one (spec/08 §1.1).
    const moved = arrayMove(list, oldIndex, newIndex)
    const firstOfficial = list.findIndex((chapter) => chapter.status === "official")
    if (firstOfficial >= 0) {
      const firstOfficialInMoved = moved.findIndex((chapter) => chapter.status === "official")
      if (firstOfficialInMoved !== firstOfficial) {
        return
      }
    }
    reorder.mutate(moved.map((chapter) => chapter.id))
  }

  return (
    <div>
      <SectionTitle>{track.kind === "main" ? t("book:main_story") : track.title}</SectionTitle>
      <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={onDragEnd}>
        <SortableContext
          items={list.map((chapter) => chapter.id)}
          strategy={verticalListSortingStrategy}
        >
          <ul className="space-y-0.5">
            {list.map((chapter) => (
              <ChapterRow
                key={chapter.id}
                bookId={book.id}
                chapter={chapter}
                active={params.chapterId === String(chapter.id)}
              />
            ))}
          </ul>
        </SortableContext>
      </DndContext>
      <Button
        variant="ghost"
        size="sm"
        className="mt-1 h-7 w-full justify-start gap-1 px-2 text-xs text-muted-foreground"
        disabled={createChapter.isPending}
        onClick={() => {
          createChapter.mutate(
            { title: "", guidance: "" },
            {
              onSuccess: (chapter) => {
                void navigate(`/books/${book.id}/chapters/${chapter.id}`)
              },
            },
          )
        }}
      >
        <Plus className="size-3.5" />
        {t("book:next_chapter")}
      </Button>
    </div>
  )
}

/** One draggable chapter row; official rows are not draggable. */
function ChapterRow({
  bookId,
  chapter,
  active,
}: {
  bookId: number
  chapter: ChapterListItemDto
  active: boolean
}) {
  const { t } = useTranslation()
  const fixed = chapter.status === "official"
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: chapter.id,
    disabled: fixed,
  })

  return (
    <li
      ref={setNodeRef}
      style={{ transform: CSS.Transform.toString(transform), transition }}
      className={cn("group flex items-center gap-1 rounded-md pr-1", isDragging && "opacity-60")}
    >
      <button
        type="button"
        className={cn(
          "shrink-0 text-muted-foreground/40 hover:text-muted-foreground",
          fixed && "invisible",
        )}
        aria-label={t("book:drag_chapter")}
        {...attributes}
        {...listeners}
      >
        <GripVertical className="size-3.5" />
      </button>
      <Link
        to={`/books/${bookId}/chapters/${chapter.id}`}
        className={cn(
          "flex min-w-0 flex-1 items-center gap-1.5 rounded-md px-1 py-1 text-[13px] hover:bg-accent",
          active && "bg-accent font-medium",
        )}
      >
        <span className="w-5 shrink-0 text-right text-[0.7rem] tabular-nums text-muted-foreground">
          {chapter.number}
        </span>
        <span className="truncate">{chapter.title || t("chapter:title_placeholder")}</span>
        <span className="ml-auto shrink-0">
          <StatusBadge status={chapter.status} iconOnly />
        </span>
        {chapter.status === "official" && chapter.canon_status !== "none" ? (
          <StatusBadge status={chapter.canon_status} variant="canon" iconOnly />
        ) : null}
      </Link>
    </li>
  )
}
