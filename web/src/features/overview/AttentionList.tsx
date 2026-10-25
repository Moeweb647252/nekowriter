/**
 * The overview's attention list (spec/08 §7): every open session that needs the author, every
 * chapter in review or with unsynced canon, the pending proposals and the ready research tasks.
 *
 * The server lists sessions, chapters and canon one per row with their target; proposals and
 * research tasks arrive as one row carrying their count. Labels and links are composed here from
 * data (D-054).
 */
import type { TFunction } from "i18next"
import { Link } from "react-router"
import { useTranslation } from "react-i18next"
import { ChevronRight } from "lucide-react"

import type { AttentionItemDto } from "@/api/gen/AttentionItemDto"
import type { TrackProgressDto } from "@/api/gen/TrackProgressDto"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { formatChapterLabel } from "@/lib/chapterLabel"

type AttentionProps = {
  bookId: number
  items: AttentionItemDto[]
  tracks: TrackProgressDto[]
}

/** One row of the attention list. */
export function AttentionList({ bookId, items, tracks }: AttentionProps) {
  const { t } = useTranslation()

  if (items.length === 0) {
    return (
      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-sm">{t("book:attention_title")}</CardTitle>
        </CardHeader>
        <CardContent>
          <p className="text-xs text-muted-foreground">{t("book:attention_empty")}</p>
        </CardContent>
      </Card>
    )
  }

  const trackOf = (trackId: number | null | undefined) =>
    tracks.find((track) => track.track_id === trackId)

  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-sm">{t("book:attention_title")}</CardTitle>
      </CardHeader>
      <CardContent>
        <ul className="divide-y">
          {items.map((item, index) => (
            <li key={`${item.kind}-${item.session_id ?? item.chapter?.id ?? index}`}>
              <Link
                to={target(item, bookId)}
                className="flex items-center justify-between gap-2 py-2 text-xs hover:text-foreground"
              >
                <span className="min-w-0 truncate">{label(t, item, trackOf(item.track_id))}</span>
                <ChevronRight aria-hidden="true" className="size-3.5 shrink-0 text-muted-foreground" />
              </Link>
            </li>
          ))}
        </ul>
      </CardContent>
    </Card>
  )
}

/** The localized text of one attention row. */
function label(
  t: TFunction,
  item: AttentionItemDto,
  track: TrackProgressDto | undefined,
): string {
  const chapter =
    item.chapter === null || item.chapter === undefined
      ? t("book:no_chapter")
      : formatChapterLabel(t, { number: item.chapter.number, track })
  switch (item.kind) {
    case "open_session":
      return t("book:attention.open_session", {
        kind:
          item.session_kind === null || item.session_kind === undefined
            ? t("agent:panel_title")
            : t(`agent:session_title.${item.session_kind}`),
        chapter,
        status:
          item.session_status === null || item.session_status === undefined
            ? ""
            : t(`agent:status.${item.session_status}`),
      })
    case "chapter_in_review":
      return t("book:attention.chapter_in_review", { chapter })
    case "canon_unsynced":
      return t("book:attention.canon_unsynced", { chapter })
    case "pending_proposals":
      return item.count === 1
        ? t("book:attention.pending_proposals_one", { count: item.count })
        : t("book:attention.pending_proposals_other", { count: item.count })
    case "ready_research":
      return item.count === 1
        ? t("book:attention.ready_research_one", { count: item.count })
        : t("book:attention.ready_research_other", { count: item.count })
    default:
      return t("book:attention_title")
  }
}

/** Where an attention row leads. */
function target(item: AttentionItemDto, bookId: number): string {
  if (item.kind === "pending_proposals") {
    return `/books/${bookId}/proposals`
  }
  if (item.kind === "ready_research") {
    return "/research"
  }
  if (item.chapter !== null && item.chapter !== undefined) {
    return `/books/${bookId}/chapters/${item.chapter.id}`
  }
  return `/books/${bookId}`
}
