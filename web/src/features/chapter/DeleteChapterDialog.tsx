/** Deleting a chapter (spec/08 §1.1, the Delete row).
 *
 * The server cascades the chapter's rows, renumbers the rest of the track and recomputes the first
 * and last appearances of the entities that appeared in it (spec/06 §1.1, D-059), so the dialog
 * states those consequences instead of asking a bare "are you sure?". For an official chapter it
 * also warns that canon keeps its records but the chapter references in them become null.
 *
 * Two refusals are normal outcomes, not failures, and both are surfaced inline with the names the
 * author needs: `409 chapter.session_exists` (a session is open on the chapter) and
 * `409 chapter.is_anchor` (the chapter anchors a side story — `details.track_ids` names which). A
 * toast carrying only the error code would leave the author with no next step, so the dialog stays
 * open, explains, and links to the places where the blocker can be removed.
 */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Link } from "react-router"

import type { ChapterListItemDto } from "@/api/gen/ChapterListItemDto"
import type { TrackDto } from "@/api/gen/TrackDto"
import { useDeleteChapter } from "@/api/hooks/chapters"
import { errorDetailNumber, isApiError } from "@/api/hooks/errors"
import { useTracks } from "@/api/hooks/tracks"
import { ConfirmDialog } from "@/components/common/ConfirmDialog"
import { useErrorToast } from "@/components/common/ErrorToast"

/** The side tracks an `chapter.is_anchor` refusal names (`details.track_ids`, spec/10 §4). */
export function anchorTrackIds(error: unknown): number[] {
  if (!isApiError(error) || error.code !== "chapter.is_anchor") {
    return []
  }
  const raw = error.details?.["track_ids"]
  if (!Array.isArray(raw)) {
    return []
  }
  return raw.filter((id): id is number => typeof id === "number")
}

/** The open session of a `chapter.session_exists` refusal (`details.session_id`, spec/10 §4). */
export function openSessionId(error: unknown): number | null {
  if (!isApiError(error) || error.code !== "chapter.session_exists") {
    return null
  }
  return errorDetailNumber(error, "session_id")
}

/** Why the server refused the delete, as the dialog renders it. */
type Refusal =
  | { kind: "session"; sessionId: number | null }
  | { kind: "anchor"; trackIds: number[] }

export type DeleteChapterDialogProps = {
  bookId: number
  track: TrackDto
  /** The chapter to delete; `null` keeps the dialog closed. */
  chapter: ChapterListItemDto | null
  onOpenChange: (open: boolean) => void
  /** Called after the server accepted the delete (the navigator leaves the deleted chapter). */
  onDeleted: (chapter: ChapterListItemDto) => void
}

/** The confirmation dialog behind the chapter row's Delete item. */
export function DeleteChapterDialog({
  bookId,
  track,
  chapter,
  onOpenChange,
  onDeleted,
}: DeleteChapterDialogProps) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  // Hooks cannot be conditional, so the mutation is bound to the track and takes the id as input;
  // the chapter is only read for wording.
  const deleteChapter = useDeleteChapter(bookId, track.id)
  // The navigator already loads every track of the book, so naming the anchored side stories below
  // reads the cached query instead of making a request of its own.
  const trackTitles = new Map(
    (useTracks(bookId).data ?? []).map((candidate) => [candidate.id, candidate.title]),
  )
  const [refusal, setRefusal] = useState<Refusal | null>(null)

  if (chapter === null) {
    return null
  }

  // One already-translated chapter label, passed into the title as a value; the sentence itself
  // lives in the locale file (AGENTS.md §6: no concatenated fragments).
  const label =
    track.kind === "main"
      ? t("chapter:label", { number: chapter.number })
      : t("chapter:side_label", { track: track.title, number: chapter.number })

  const close = () => {
    setRefusal(null)
    onOpenChange(false)
  }

  const deleteNow = () => {
    deleteChapter.mutate(chapter.id, {
      onSuccess: () => {
        const deleted = chapter
        setRefusal(null)
        onOpenChange(false)
        onDeleted(deleted)
      },
      onError: (error) => {
        // The two documented refusals stay in the dialog with the names the author needs; anything
        // else is a real failure and goes to the localized error toast.
        if (isApiError(error) && error.code === "chapter.session_exists") {
          setRefusal({ kind: "session", sessionId: openSessionId(error) })
          return
        }
        if (isApiError(error) && error.code === "chapter.is_anchor") {
          setRefusal({ kind: "anchor", trackIds: anchorTrackIds(error) })
          return
        }
        showError(error)
      },
    })
  }

  const refused = refusal !== null

  return (
    <ConfirmDialog
      open
      onOpenChange={(next) => {
        if (!next) {
          setRefusal(null)
        }
        onOpenChange(next)
      }}
      title={t("chapter:delete.title", { label })}
      description={t("chapter:delete.body")}
      confirmLabel={refused ? t("common:close") : t("chapter:delete.confirm")}
      destructive={!refused}
      pending={deleteChapter.isPending}
      onConfirm={() => {
        // A refused delete cannot be retried until the blocker is gone, so the primary button only
        // acknowledges the message.
        if (refused) {
          close()
          return
        }
        deleteNow()
      }}
      details={
        refusal === null ? (
          <div className="space-y-1">
            <p>{t("chapter:delete.renumber")}</p>
            {chapter.status === "official" ? (
              <p>{t("chapter:delete.canon_kept")}</p>
            ) : null}
          </div>
        ) : (
          <RefusalNotice
            bookId={bookId}
            chapter={chapter}
            refusal={refusal}
            trackTitles={trackTitles}
          />
        )
      }
    />
  )
}

/** What the author has to change before the delete can succeed (spec/08 §1.1). */
function RefusalNotice({
  bookId,
  chapter,
  refusal,
  trackTitles,
}: {
  bookId: number
  chapter: ChapterListItemDto
  refusal: Refusal
  /** Names for the anchored side tracks; a missing entry falls back to the track id. */
  trackTitles: Map<number, string>
}) {
  const { t } = useTranslation()

  if (refusal.kind === "session") {
    return (
      <div className="space-y-1">
        <p>
          {refusal.sessionId === null
            ? t("chapter:delete.session_open")
            : t("chapter:delete.session_open_id", { id: refusal.sessionId })}
        </p>
        <Link
          to={`/books/${bookId}/chapters/${chapter.id}`}
          className="text-primary underline"
        >
          {t("chapter:delete.session_open_action")}
        </Link>
      </div>
    )
  }

  const ids = refusal.trackIds
  return (
    <div className="space-y-1">
      <p>{t("chapter:delete.is_anchor")}</p>
      {ids.length === 0 ? null : (
        <ul className="list-disc pl-4">
          {ids.map((trackId) => {
            const title = trackTitles.get(trackId)
            return (
              <li key={trackId}>
                <Link
                  to={`/books/${bookId}/tracks/${trackId}`}
                  className="text-primary underline"
                >
                  {title === undefined || title === ""
                    ? t("chapter:delete.side_story_ref", { id: trackId })
                    : title}
                </Link>
              </li>
            )
          })}
        </ul>
      )}
      <p>{t("chapter:delete.is_anchor_hint")}</p>
    </div>
  )
}
