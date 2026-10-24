/** The History tab: versions, a two-version diff and restore (PROJECT §8.5, spec/11 §5.2). */
import { useState } from "react"
import { useTranslation } from "react-i18next"

import { useRestoreVersion, useSaveVersion, useVersion, useVersions } from "@/api/hooks/chapters"
import { useErrorToast } from "@/components/common/ErrorToast"
import { DiffView } from "@/components/common/DiffView"
import { EmptyState } from "@/components/common/EmptyState"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Skeleton } from "@/components/ui/skeleton"
import type { ChapterDto } from "@/api/gen/ChapterDto"
import { formatDateTime } from "@/lib/format"

export type HistoryTabProps = {
  chapter: ChapterDto
  bookId: number
  unspaced: boolean
}

/** Version list plus a diff between any two versions. */
export function HistoryTab({ chapter, bookId, unspaced }: HistoryTabProps) {
  const { t, i18n } = useTranslation()
  const showError = useErrorToast()
  const versions = useVersions(chapter.id)
  const save = useSaveVersion(chapter.id)
  const restore = useRestoreVersion(bookId)
  const [note, setNote] = useState("")
  const [beforeId, setBeforeId] = useState<number | null>(null)
  const [afterId, setAfterId] = useState<number | null>(null)
  const before = useVersion(beforeId)
  const after = useVersion(afterId)

  const list = versions.data ?? []

  return (
    <div className="mx-auto max-w-[52em] space-y-5">
      <div className="flex items-end gap-2">
        <div className="flex-1 space-y-1">
          <Input
            value={note}
            placeholder={t("chapter:version_note")}
            onChange={(event) => setNote(event.target.value)}
          />
        </div>
        <Button
          size="sm"
          variant="outline"
          disabled={save.isPending}
          onClick={() =>
            save.mutate(
              { note },
              {
                onSuccess: () => setNote(""),
                onError: showError,
              },
            )
          }
        >
          {t("chapter:save_version")}
        </Button>
      </div>

      {versions.isPending ? (
        <Skeleton className="h-24" />
      ) : list.length === 0 ? (
        <EmptyState
          title={t("chapter:no_versions")}
          description={t("chapter:no_versions_hint")}
        />
      ) : (
        <ul className="divide-y rounded-md border">
          {list.map((version) => (
            <li key={version.id} className="flex flex-wrap items-center gap-2 px-3 py-2 text-xs">
              <span className="font-medium">#{version.version_no}</span>
              <span className="rounded bg-muted px-1.5 py-0.5">
                {t(`chapter:version_kind.${version.kind}`, { defaultValue: version.kind })}
              </span>
              <span className="text-muted-foreground">
                {formatDateTime(version.created_at, i18n.language)}
              </span>
              <span className="tabular-nums text-muted-foreground">
                {t("chapter:version_words", { count: version.word_count })}
              </span>
              {version.note === "" ? null : (
                <span className="truncate text-muted-foreground">{version.note}</span>
              )}
              <div className="ml-auto flex items-center gap-1">
                <Button size="xs" variant="ghost" onClick={() => setBeforeId(version.id)}>
                  {t("chapter:diff_from")}
                </Button>
                <Button size="xs" variant="ghost" onClick={() => setAfterId(version.id)}>
                  {t("chapter:diff_to")}
                </Button>
                <Button
                  size="xs"
                  variant="outline"
                  disabled={restore.isPending}
                  onClick={() => restore.mutate(version.id, { onError: showError })}
                >
                  {t("chapter:restore")}
                </Button>
              </div>
            </li>
          ))}
        </ul>
      )}

      {before.data === undefined && after.data === undefined ? null : (
        <div className="space-y-2">
          <div className="flex items-center justify-between text-xs text-muted-foreground">
            <span>
              {t("chapter:diff_heading", {
                before: beforeId ?? t("chapter:diff_current"),
                after: afterId ?? t("chapter:diff_current"),
              })}
            </span>
            <Button
              size="xs"
              variant="ghost"
              onClick={() => {
                setBeforeId(null)
                setAfterId(null)
              }}
            >
              {t("common:close")}
            </Button>
          </div>
          <DiffView
            before={before.data?.content.paragraphs.map((p) => p.text).join("\n\n") ?? currentText(chapter)}
            after={after.data?.content.paragraphs.map((p) => p.text).join("\n\n") ?? currentText(chapter)}
            unspaced={unspaced}
            className="max-h-96 overflow-y-auto rounded-md border p-3"
          />
        </div>
      )}
    </div>
  )
}

function currentText(chapter: ChapterDto): string {
  return chapter.content.paragraphs.map((paragraph) => paragraph.text).join("\n\n")
}
