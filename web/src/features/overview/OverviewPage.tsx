/**
 * The book overview (spec/08 §7, PROJECT §8.3): what to do next, everything that needs the author,
 * the progress, the recent canon changes and the book's token usage.
 */
import { Link, useParams } from "react-router"
import { useTranslation } from "react-i18next"

import { useOverview } from "@/api/hooks/books"
import { EmptyState } from "@/components/common/EmptyState"
import { StatusBadge } from "@/components/common/StatusBadge"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Progress } from "@/components/ui/progress"
import { Skeleton } from "@/components/ui/skeleton"
import { Button } from "@/components/ui/button"
import { AttentionList } from "@/features/overview/AttentionList"
import { NextActionCard } from "@/features/overview/NextActionCard"
import { formatNumber } from "@/lib/format"

export default function OverviewPage() {
  const { bookId: rawBookId } = useParams()
  const bookId = Number(rawBookId)
  const { t, i18n } = useTranslation()
  const overview = useOverview(bookId)

  if (overview.isPending) {
    return <Skeleton className="m-6 h-48" />
  }
  if (overview.isError || overview.data === undefined) {
    return (
      <div className="p-6">
        <EmptyState title={t("common:error_title")} description={t("common:error_body")} />
      </div>
    )
  }

  const { book, next_action, attention, progress, recent_changes, usage } = overview.data
  const target = `${formatNumber(book.settings.target_words_min, i18n.language)}–${formatNumber(
    book.settings.target_words_max,
    i18n.language,
  )}`
  const officialRatio =
    progress.total_chapters === 0
      ? 0
      : Math.round((progress.official_chapters / progress.total_chapters) * 100)

  return (
    <div className="mx-auto max-w-4xl space-y-6 p-6">
      <header className="flex items-end justify-between gap-4">
        <div>
          <h1 className="text-xl font-semibold">{book.title}</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            {[book.genre, book.writing_language].filter(Boolean).join(" · ")}
          </p>
        </div>
        {book.status === "conceiving" ? (
          // A conceiving book's next step is the studio (PROJECT §8.7), and the sidebar link is
          // easy to miss before the first chapter exists.
          <Button asChild size="sm">
            <Link to={`/books/${book.id}/conception`}>{t("book:conception.continue")}</Link>
          </Button>
        ) : null}
      </header>

      {next_action === null || next_action === undefined ? null : (
        <NextActionCard
          bookId={bookId}
          status={book.status}
          action={next_action}
          tracks={progress.tracks}
        />
      )}
      <AttentionList bookId={bookId} items={attention} tracks={progress.tracks} />

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-3">
        <Card>
          <CardHeader>
            <CardTitle className="text-sm">{t("book:progress_chapters")}</CardTitle>
          </CardHeader>
          <CardContent>
            <p className="text-2xl font-semibold tabular-nums">
              {progress.official_chapters} / {progress.total_chapters}
            </p>
            <Progress className="mt-2" value={officialRatio} />
          </CardContent>
        </Card>
        <Card>
          <CardHeader>
            <CardTitle className="text-sm">{t("book:progress_words")}</CardTitle>
          </CardHeader>
          <CardContent>
            <p className="text-2xl font-semibold tabular-nums">
              {formatNumber(progress.total_words, i18n.language)}
            </p>
            <p className="mt-1 text-xs text-muted-foreground">
              {t("book:progress_official_words", {
                words: formatNumber(progress.official_words, i18n.language),
              })}
            </p>
          </CardContent>
        </Card>
        <Card>
          <CardHeader>
            <CardTitle className="text-sm">{t("book:progress_target")}</CardTitle>
          </CardHeader>
          <CardContent>
            <p className="text-2xl font-semibold tabular-nums">{target}</p>
            <p className="mt-1 text-xs text-muted-foreground">
              {t("book:progress_target_hint")}
            </p>
          </CardContent>
        </Card>
      </div>

      <Card>
        <CardHeader>
          <CardTitle className="text-sm">{t("book:progress_by_status")}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          {progress.tracks.map((track) => (
            <div key={track.track_id} className="space-y-1">
              <p className="text-xs font-medium">
                {track.kind === "main" ? t("book:main_story") : track.title}
                <span className="ml-2 font-normal text-muted-foreground">
                  {t("book:track_summary", {
                    chapters: track.chapters,
                    words: formatNumber(track.words, i18n.language),
                  })}
                </span>
              </p>
              <ul className="flex flex-wrap gap-3">
                {track.by_status.map((entry) => (
                  <li key={entry.status} className="flex items-center gap-1">
                    <StatusBadge status={entry.status} iconOnly />
                    <span className="text-xs tabular-nums text-muted-foreground">
                      {entry.count}
                    </span>
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-sm">{t("book:usage_title")}</CardTitle>
        </CardHeader>
        <CardContent>
          <p className="text-xs text-muted-foreground">
            {t("book:usage_summary", {
              requests: formatNumber(usage.requests, i18n.language),
              input: formatNumber(usage.input_tokens, i18n.language),
              output: formatNumber(usage.output_tokens, i18n.language),
            })}
          </p>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-sm">{t("book:recent_changes")}</CardTitle>
        </CardHeader>
        <CardContent>
          {recent_changes.length === 0 ? (
            <p className="text-xs text-muted-foreground">{t("book:no_changes")}</p>
          ) : (
            <ul className="space-y-1 text-xs">
              {recent_changes.map((change) => (
                <li key={change.id} className="flex items-center gap-2 text-muted-foreground">
                  <StatusBadge status={change.op} variant="canon" className="capitalize" />
                  <span className="truncate">
                    {t(`book:change_target.${change.target_type}`, {
                      defaultValue: change.target_type,
                    })}
                  </span>
                  {change.note === "" ? null : (
                    <span className="truncate text-foreground">{change.note}</span>
                  )}
                </li>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
