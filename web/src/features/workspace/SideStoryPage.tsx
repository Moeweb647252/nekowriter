/** The side-story page (PROJECT §8.4, spec/11 §3 `tracks/:trackId`): its premise, canon mode,
 * anchor, focus character and chapters.
 *
 * The track's own metadata is editable here; `canon_mode` is locked once a chapter of the track is
 * official (spec/08 §6), and the anchor may always move (it only affects sessions created
 * afterwards). Deleting a side story is confirmed and takes its chapters with it (spec/08 §1.3). */
import { useState } from "react"
import { Link, useNavigate, useOutletContext, useParams } from "react-router"
import { useTranslation } from "react-i18next"
import { Plus, Trash2 } from "lucide-react"

import type { AnchorKind } from "@/api/gen/AnchorKind"
import type { BookDto } from "@/api/gen/BookDto"
import type { CanonMode } from "@/api/gen/CanonMode"
import type { TrackDto } from "@/api/gen/TrackDto"
import { useEntities } from "@/api/hooks/useCanon"
import {
  useChapters,
  useCreateChapter,
  useDeleteTrack,
  usePatchTrack,
  useTrack,
  useTracks,
} from "@/api/hooks/tracks"
import { ConfirmDialog } from "@/components/common/ConfirmDialog"
import { EmptyState } from "@/components/common/EmptyState"
import { StatusBadge } from "@/components/common/StatusBadge"
import { useErrorToast } from "@/components/common/ErrorToast"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Skeleton } from "@/components/ui/skeleton"
import { Textarea } from "@/components/ui/textarea"
import {
  chapterRefLabel,
  useChapterIndex,
} from "@/features/canon/chapterIndex"

export default function SideStoryPage() {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const { bookId: rawBookId, trackId: rawTrackId } = useParams()
  const bookId = Number(rawBookId)
  const trackId = Number(rawTrackId)
  // `BookLayout` loads the book for the whole workspace and provides it through the outlet
  // context, so the status costs no extra request here (see below).
  const { book } = useOutletContext<{ book: BookDto }>()
  const track = useTrack(trackId)
  const tracks = useTracks(bookId)
  const chapters = useChapters(trackId)
  const createChapter = useCreateChapter(trackId)
  const chapterIndex = useChapterIndex(tracks.data)
  const showError = useErrorToast()
  const deleteTrack = useDeleteTrack(bookId)
  const [confirmDelete, setConfirmDelete] = useState(false)
  const [typedName, setTypedName] = useState("")

  if (track.isPending) {
    return <Skeleton className="m-6 h-64" />
  }
  if (track.isError || track.data === undefined) {
    return (
      <div className="p-6">
        <EmptyState title={t("common:error_title")} description={t("common:error_body")} />
      </div>
    )
  }
  if (track.data.kind !== "side") {
    return (
      <div className="p-6">
        <EmptyState
          title={t("book:not_a_side_story")}
          description={t("book:not_a_side_story_hint")}
        />
      </div>
    )
  }
  const side = track.data
  const main = tracks.data?.find((row) => row.kind === "main")
  const official = (chapters.data ?? []).some(
    (chapter) => chapter.status === "official",
  )
  // Only an `active` book accepts a new chapter (spec/08 §1.1, D-130): a book whose conception is
  // not finalized has no setting materials or canon for the writer, and an archived book stands
  // still. The server refuses both with `409 book.invalid_state`; disabling the button here means
  // the author never meets that raw refusal. Existing chapters stay listed and openable either
  // way — only *creating* is gated.
  const canCreate = book.status === "active"
  const createReason =
    book.status === "conceiving"
      ? t("book:next_chapter_needs_conception")
      : book.status === "archived"
        ? t("book:next_chapter_archived")
        : undefined
  const createReasonId = "side-story-chapter-create-reason"

  return (
    <div className="mx-auto max-w-3xl space-y-6 p-6">
      <header className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h1 className="text-xl font-semibold">{side.title}</h1>
          <p className="text-xs text-muted-foreground">
            {t("book:side_story_hint")}
          </p>
        </div>
        <Button
          variant="outline"
          size="sm"
          onClick={() => setConfirmDelete(true)}
        >
          <Trash2 className="size-3.5" aria-hidden="true" />
          {t("book:delete_side_story")}
        </Button>
      </header>

      <TrackSettingsForm
        key={`${side.id}-${side.updated_at}`}
        bookId={bookId}
        track={side}
        mainTrackId={main?.id}
        locked={official}
      />

      <section>
        <div className="mb-2 flex items-center justify-between">
          <h2 className="text-sm font-medium">{t("book:chapters")}</h2>
          <Button
            size="sm"
            variant="outline"
            disabled={!canCreate || createChapter.isPending}
            aria-describedby={canCreate ? undefined : createReasonId}
            onClick={() =>
              createChapter.mutate(
                { title: "", guidance: "" },
                {
                  onSuccess: (chapter) =>
                    void navigate(`/books/${bookId}/chapters/${chapter.id}`),
                  onError: showError,
                },
              )
            }
          >
            <Plus className="size-3.5" aria-hidden="true" />
            {t("book:next_chapter")}
          </Button>
        </div>
        {canCreate ? null : (
          // A disabled button takes no pointer events, so a `title` tooltip would never show; the
          // reason is written out under the header instead, and names the way forward.
          <div
            id={createReasonId}
            className="mb-3 space-y-0.5 text-[0.7rem] text-muted-foreground"
          >
            <p>{createReason}</p>
            {book.status === "conceiving" ? (
              <Link
                to={`/books/${bookId}/conception`}
                className="block text-primary underline"
              >
                {t("book:next_chapter_open_conception")}
              </Link>
            ) : null}
          </div>
        )}
        {(chapters.data ?? []).length === 0 ? (
          <EmptyState
            title={t("book:no_chapters")}
            description={t("book:side_story_no_chapters_hint")}
          />
        ) : (
          <ul className="divide-y rounded-md border">
            {(chapters.data ?? []).map((chapter) => (
              <li key={chapter.id}>
                <Link
                  to={`/books/${bookId}/chapters/${chapter.id}`}
                  className="flex items-center gap-2 px-3 py-2 text-sm hover:bg-accent/50"
                >
                  <span className="w-6 text-right text-xs tabular-nums text-muted-foreground">
                    {chapter.number}
                  </span>
                  <span className="min-w-0 flex-1 truncate">
                    {chapter.title === ""
                      ? t("chapter:title_placeholder")
                      : chapter.title}
                  </span>
                  <StatusBadge status={chapter.status} />
                  {chapter.status === "official" &&
                  chapter.canon_status !== "none" ? (
                    <StatusBadge status={chapter.canon_status} variant="canon" />
                  ) : null}
                </Link>
              </li>
            ))}
          </ul>
        )}
        <p className="mt-2 text-[0.7rem] text-muted-foreground">
          {t("book:side_story_chapters_hint", {
            label:
              side.anchor_chapter_id === null
                ? t(`book:anchor.${side.anchor_kind}`)
                : chapterRefLabel(t, chapterIndex, side.anchor_chapter_id),
          })}
        </p>
      </section>

      <ConfirmDialog
        open={confirmDelete}
        onOpenChange={setConfirmDelete}
        title={t("book:delete_side_story_title", { title: side.title })}
        description={t("book:delete_side_story_body")}
        confirmLabel={t("book:delete_side_story")}
        confirmText={side.title}
        typed={typedName}
        onTypedChange={setTypedName}
        pending={deleteTrack.isPending}
        onConfirm={() => {
          deleteTrack.mutate(side.id, {
            onSuccess: () => {
              setConfirmDelete(false)
              void navigate(`/books/${bookId}`)
            },
            onError: showError,
          })
        }}
      />
    </div>
  )
}

/** The editable metadata of one side track (spec/08 §6). */
function TrackSettingsForm({
  bookId,
  track,
  mainTrackId,
  locked,
}: {
  bookId: number
  track: TrackDto
  mainTrackId?: number
  locked: boolean
}) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const patch = usePatchTrack(bookId, track.id)
  const mainChapters = useChapters(mainTrackId ?? 0)
  const characters = useEntities(bookId, { kind: "character" })

  const [title, setTitle] = useState(track.title)
  const [synopsis, setSynopsis] = useState(track.synopsis)
  const [canonMode, setCanonMode] = useState<CanonMode>(track.canon_mode)
  const [anchorKind, setAnchorKind] = useState<AnchorKind>(track.anchor_kind)
  const [anchorChapterId, setAnchorChapterId] = useState<number | undefined>(
    track.anchor_chapter_id ?? undefined,
  )
  const [focusEntityId, setFocusEntityId] = useState<number | undefined>(
    track.focus_entity_id ?? undefined,
  )

  const anchorReady =
    anchorKind !== "after_chapter" || anchorChapterId !== undefined

  return (
    <section className="space-y-3 rounded-md border p-4">
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="space-y-1.5">
          <Label htmlFor="track-title">{t("book:side_title")}</Label>
          <Input
            id="track-title"
            value={title}
            onChange={(event) => setTitle(event.target.value)}
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="track-mode">{t("book:side_canon_mode")}</Label>
          <Select
            value={canonMode}
            disabled={locked}
            onValueChange={(value) => setCanonMode(value as CanonMode)}
          >
            <SelectTrigger id="track-mode">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="isolated">
                {t("book:canon_mode.isolated")}
              </SelectItem>
              <SelectItem value="canon">{t("book:canon_mode.canon")}</SelectItem>
            </SelectContent>
          </Select>
          <p className="text-[0.7rem] text-muted-foreground">
            {locked ? t("book:canon_mode_locked") : t("book:canon_mode_hint")}
          </p>
        </div>
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="track-synopsis">{t("book:side_synopsis")}</Label>
        <Textarea
          id="track-synopsis"
          rows={3}
          value={synopsis}
          onChange={(event) => setSynopsis(event.target.value)}
        />
      </div>
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="space-y-1.5">
          <Label htmlFor="track-anchor">{t("book:side_anchor")}</Label>
          <Select
            value={anchorKind}
            onValueChange={(value) => {
              const kind = value as AnchorKind
              setAnchorKind(kind)
              if (kind !== "after_chapter") {
                setAnchorChapterId(undefined)
              }
            }}
          >
            <SelectTrigger id="track-anchor">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="after_chapter">
                {t("book:anchor.after_chapter")}
              </SelectItem>
              <SelectItem value="before_start">
                {t("book:anchor.before_start")}
              </SelectItem>
              <SelectItem value="after_end">
                {t("book:anchor.after_end")}
              </SelectItem>
              <SelectItem value="none">{t("book:anchor.none")}</SelectItem>
            </SelectContent>
          </Select>
        </div>
        {anchorKind !== "after_chapter" ? null : (
          <div className="space-y-1.5">
            <Label htmlFor="track-anchor-chapter">
              {t("book:side_anchor_chapter")}
            </Label>
            <Select
              value={
                anchorChapterId === undefined ? "" : String(anchorChapterId)
              }
              onValueChange={(value) => setAnchorChapterId(Number(value))}
            >
              <SelectTrigger id="track-anchor-chapter">
                <SelectValue placeholder={t("book:side_anchor_chapter_none")} />
              </SelectTrigger>
              <SelectContent>
                {(mainChapters.data ?? []).map((chapter) => (
                  <SelectItem key={chapter.id} value={String(chapter.id)}>
                    {t("book:chapter_option", {
                      number: chapter.number,
                      title: chapter.title,
                    })}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        )}
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="track-focus">{t("book:side_focus")}</Label>
        <Select
          value={focusEntityId === undefined ? "none" : String(focusEntityId)}
          onValueChange={(value) =>
            setFocusEntityId(value === "none" ? undefined : Number(value))
          }
        >
          <SelectTrigger id="track-focus">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="none">{t("book:side_focus_none")}</SelectItem>
            {(characters.data ?? []).map((entity) => (
              <SelectItem key={entity.id} value={String(entity.id)}>
                {entity.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <p className="text-[0.7rem] text-muted-foreground">
          {t("book:side_focus_hint")}
        </p>
      </div>
      <div className="flex justify-end">
        <Button
          disabled={title.trim() === "" || !anchorReady || patch.isPending}
          onClick={() => {
            const clearingFocus =
              focusEntityId === undefined && track.focus_entity_id !== null
            patch.mutate(
              {
                title: title.trim(),
                synopsis: synopsis.trim(),
                canon_mode: canonMode,
                anchor_kind: anchorKind,
                anchor_chapter_id:
                  anchorKind === "after_chapter" ? anchorChapterId : undefined,
                focus_entity_id:
                  focusEntityId === undefined ? undefined : focusEntityId,
                clear_focus_entity: clearingFocus,
              },
              { onError: showError },
            )
          }}
        >
          {patch.isPending ? t("common:saving") : t("common:save")}
        </Button>
      </div>
    </section>
  )
}
