/** The chapter header: label, inline title, status, word count and status actions. */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { useQueryClient } from "@tanstack/react-query"

import { useApproveChapter, useSubmitChapter } from "@/api/hooks/chapters"
import { useErrorToast, useWarningToasts } from "@/components/common/ErrorToast"
import { StatusBadge } from "@/components/common/StatusBadge"
import { WordCount } from "@/components/common/WordCount"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import type { BookSettings } from "@/api/gen/BookSettings"
import type { ChapterDto } from "@/api/gen/ChapterDto"
import { keys } from "@/api/keys"
import { isComposing } from "@/lib/hotkeys"

export type ChapterHeaderProps = {
  chapter: ChapterDto
  settings: BookSettings
  language: string
  onRename: (title: string) => void
}

/** Name/status/actions of one chapter. */
export function ChapterHeader({ chapter, settings, language, onRename }: ChapterHeaderProps) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const showError = useErrorToast()
  const showWarnings = useWarningToasts()
  const submit = useSubmitChapter(chapter.id, chapter.book_id)
  const approve = useApproveChapter(chapter.id, chapter.book_id)
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState(chapter.title)

  const save = () => {
    setEditing(false)
    const trimmed = draft.trim()
    if (trimmed !== chapter.title) {
      onRename(trimmed)
    }
  }

  // Status actions from PROJECT §8.5; drafting/revising are driven by the agent panel in M4.
  const canSubmit = chapter.status === "drafting" || chapter.status === "revising"
  const canApprove = chapter.status === "in_review"

  return (
    <header className="space-y-2 border-b px-6 py-4">
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <span>{t("chapter:label", { number: chapter.number })}</span>
        <StatusBadge status={chapter.status} />
        {chapter.status === "official" ? (
          <StatusBadge status={chapter.canon_status} variant="canon" />
        ) : null}
      </div>
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0 flex-1">
          {editing ? (
            <Input
              value={draft}
              autoFocus
              className="h-8 text-lg font-semibold"
              onChange={(event) => setDraft(event.target.value)}
              onBlur={save}
              onKeyDown={(event) => {
                if (isComposing(event)) {
                  return
                }
                if (event.key === "Enter") {
                  save()
                }
                if (event.key === "Escape") {
                  setDraft(chapter.title)
                  setEditing(false)
                }
              }}
            />
          ) : (
            <button
              type="button"
              className="block max-w-full truncate text-left text-lg font-semibold hover:underline"
              onClick={() => {
                setDraft(chapter.title)
                setEditing(true)
              }}
            >
              {chapter.title || t("chapter:title_placeholder")}
            </button>
          )}
        </div>
        <WordCount
          words={chapter.word_count}
          targetMin={settings.target_words_min}
          targetMax={settings.target_words_max}
          language={language}
          className="pt-1.5"
        />
      </div>
      <div className="flex flex-wrap items-center gap-2">
        {canSubmit ? (
          <Button
            size="sm"
            variant="outline"
            disabled={submit.isPending || chapter.content.paragraphs.length === 0}
            onClick={() =>
              submit.mutate(undefined, {
                onSuccess: () => {
                  void queryClient.invalidateQueries({ queryKey: keys.chapters.reviews(chapter.id) })
                },
                onError: showError,
              })
            }
          >
            {t("chapter:submit")}
          </Button>
        ) : null}
        {canApprove ? (
          <Button
            size="sm"
            disabled={approve.isPending}
            onClick={() =>
              approve.mutate(undefined, {
                onSuccess: ({ warnings }) => showWarnings(warnings),
                onError: showError,
              })
            }
          >
            {t("chapter:approve")}
          </Button>
        ) : null}
      </div>
    </header>
  )
}
