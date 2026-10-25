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
import type { NextActionDto } from "@/api/gen/NextActionDto"
import type { TrackProgressDto } from "@/api/gen/TrackProgressDto"
import { useErrorToast } from "@/components/common/ErrorToast"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { formatChapterLabel, trackName } from "@/lib/chapterLabel"

type NextActionProps = {
  bookId: number
  action: NextActionDto
  tracks: TrackProgressDto[]
}

/** The overview's "what to do next" card. */
export function NextActionCard({ bookId, action, tracks }: NextActionProps) {
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
        <ActionButton bookId={bookId} action={action} />
      </CardContent>
    </Card>
  )
}

/** The button (or link) that performs the action. */
function ActionButton({ bookId, action }: { bookId: number; action: NextActionDto }) {
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
        <WriteNextButton trackId={action.track_id} bookId={bookId} />
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
function WriteNextButton({ bookId, trackId }: { bookId: number; trackId: number }) {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const showError = useErrorToast()
  const create = useCreateChapter(trackId)
  return (
    <Button
      size="sm"
      disabled={create.isPending}
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
  )
}
