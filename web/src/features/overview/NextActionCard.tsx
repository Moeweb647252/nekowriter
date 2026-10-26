/**
 * The next action card of the book overview (PROJECT §8.3, spec/08 §7).
 *
 * The API sends the action's kind and the data it names (`chapter`, `session_id`, `track_id`); the
 * label and the destination are composed here (D-054). The two actions that start a session
 * (`seed_canon`, `sync_canon`) and the one that creates a chapter (`write_next`) are buttons; the
 * rest navigate.
 */
import { Link, useNavigate } from "react-router"
import { useTranslation } from "react-i18next"
import { ArrowRight, Loader2 } from "lucide-react"

import { useSeedCanon } from "@/api/hooks/conception"
import { useResyncChapter } from "@/api/hooks/reviews"
import { useCreateChapter } from "@/api/hooks/tracks"
import type { BookStatus } from "@/api/gen/BookStatus"
import type { NextActionDto } from "@/api/gen/NextActionDto"
import type { TrackProgressDto } from "@/api/gen/TrackProgressDto"
import { useErrorToast } from "@/components/common/ErrorToast"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { formatChapterLabel, trackName } from "@/lib/chapterLabel"

type NextActionProps = {
  bookId: number
  /** The book's status: creation is only allowed while the book is `active` (spec/08 §1.1). */
  status: BookStatus
  action: NextActionDto
  tracks: TrackProgressDto[]
}

/** The overview's "what to do next" card. */
export function NextActionCard({ bookId, status, action, tracks }: NextActionProps) {
  const { t } = useTranslation()
  const track = tracks.find((entry) => entry.track_id === action.track_id)
  const chapter =
    action.chapter === null || action.chapter === undefined
      ? undefined
      : formatChapterLabel(t, { number: action.chapter.number, track })
  const label = t(`book:next_action.${action.kind}`, {
    chapter: chapter ?? "",
    track: track === undefined ? "" : trackName(t, track),
  })

  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-sm">{t("book:next_action_title")}</CardTitle>
      </CardHeader>
      <CardContent className="flex flex-wrap items-center justify-between gap-3">
        <p className="text-base font-medium">{label}</p>
        <ActionButton bookId={bookId} status={status} action={action} />
      </CardContent>
    </Card>
  )
}

/** The button (or link) that performs the action. */
function ActionButton({
  bookId,
  status,
  action,
}: {
  bookId: number
  status: BookStatus
  action: NextActionDto
}) {
  const { t } = useTranslation()
  const chapterId = action.chapter?.id
  switch (action.kind) {
    case "continue_conception":
      return (
        <Button asChild size="sm">
          <Link to={`/books/${bookId}/conception`}>
            {t("book:next_action.go")}
            <ArrowRight aria-hidden="true" />
          </Link>
        </Button>
      )
    case "seed_canon":
      return <SeedCanonButton bookId={bookId} />
    case "review_chapter":
    case "continue_chapter":
      return chapterId === undefined ? null : (
        <Button asChild size="sm">
          <Link to={`/books/${bookId}/chapters/${chapterId}`}>
            {t("book:next_action.go")}
            <ArrowRight aria-hidden="true" />
          </Link>
        </Button>
      )
    case "sync_canon":
      return chapterId === undefined ? null : (
        <ResyncButton bookId={bookId} chapterId={chapterId} />
      )
    case "write_next":
      return action.track_id === undefined || action.track_id === null ? null : (
        <WriteNextButton trackId={action.track_id} bookId={bookId} status={status} />
      )
    default:
      return null
  }
}

function SeedCanonButton({ bookId }: { bookId: number }) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const seed = useSeedCanon(bookId)
  return (
    <Button size="sm" disabled={seed.isPending} onClick={() => seed.mutate(undefined, { onError: showError })}>
      {seed.isPending ? <Loader2 aria-hidden="true" className="animate-spin" /> : null}
      {t("book:next_action.go")}
    </Button>
  )
}

function ResyncButton({ bookId, chapterId }: { bookId: number; chapterId: number }) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const resync = useResyncChapter(chapterId, bookId)
  return (
    <Button size="sm" disabled={resync.isPending} onClick={() => resync.mutate(undefined, { onError: showError })}>
      {resync.isPending ? <Loader2 aria-hidden="true" className="animate-spin" /> : null}
      {t("book:next_action.go")}
    </Button>
  )
}

/** Creates the next chapter of the track and opens it (the navigator's "+ Next chapter"). */
function WriteNextButton({
  bookId,
  trackId,
  status,
}: {
  bookId: number
  trackId: number
  status: BookStatus
}) {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const showError = useErrorToast()
  const create = useCreateChapter(trackId)
  // A chapter can only be created on an `active` book (spec/08 §1.1, D-130), so the button is a
  // defensive disable rather than the primary rule. `books::next_action` answers
  // `ContinueConception` for a conceiving book before any other branch
  // (`crates/server/src/services/books.rs`), so this button is never rendered for one: `archived`
  // is the only reachable non-active status here. Should that ever change, the reason below has to
  // grow the conceiving wording as well.
  const canCreate = status === "active"
  return (
    <div className="flex flex-col items-end gap-1">
      <Button
        size="sm"
        disabled={!canCreate || create.isPending}
        onClick={() =>
          create.mutate(
            { title: "", guidance: "" },
            {
              onSuccess: (chapter) => void navigate(`/books/${bookId}/chapters/${chapter.id}`),
              onError: showError,
            },
          )
        }
      >
        {create.isPending ? <Loader2 aria-hidden="true" className="animate-spin" /> : null}
        {t("book:next_action.go")}
      </Button>
      {canCreate ? null : (
        // A disabled HTML button takes no pointer events, so a `title` tooltip would never appear;
        // the reason is visible text instead, worded exactly as in the navigator and the side
        // story page.
        <p className="text-xs text-muted-foreground">{t("book:next_chapter_archived")}</p>
      )}
    </div>
  )
}
