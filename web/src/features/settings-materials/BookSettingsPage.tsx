/** Book settings (PROJECT §8.12): language, genre, granularity, pause rules, target length,
 * memory windows, auto-maintenance and model overrides. */
import { useState } from "react"
import { useParams } from "react-router"
import { useTranslation } from "react-i18next"

import { useBook, usePatchBook } from "@/api/hooks/books"
import { useErrorToast } from "@/components/common/ErrorToast"
import { EmptyState } from "@/components/common/EmptyState"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Skeleton } from "@/components/ui/skeleton"
import { Switch } from "@/components/ui/switch"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import type { BookSettings } from "@/api/gen/BookSettings"
import type { Granularity } from "@/api/gen/Granularity"

const GRANULARITIES: Granularity[] = ["sentence", "paragraph", "free", "chapter"]

/** The editable form of a book, tagged with the row version it was built from. */
type BookDraft = {
  key: string
  title: string
  genre: string
  language: string
  settings: BookSettings
}
const LANGUAGES = ["zh-CN", "en", "ja", "ko", "fr", "de", "es", "ru"] as const

export default function BookSettingsPage() {
  const { bookId: rawBookId } = useParams()
  const bookId = Number(rawBookId)
  const { t } = useTranslation()
  const showError = useErrorToast()
  const book = useBook(bookId)
  const patch = usePatchBook(bookId)
  // One draft object keyed by the book's `updated_at`, so an SSE refresh of the query never
  // clobbers unsaved input and no effect has to copy the query into state.
  const [draft, setDraft] = useState<BookDraft | null>(null)

  if (book.isPending) {
    return <Skeleton className="m-6 h-64" />
  }
  if (book.isError || book.data === undefined) {
    return (
      <div className="p-6">
        <EmptyState title={t("common:error_title")} description={t("common:error_body")} />
      </div>
    )
  }

  const saved: BookDraft = {
    key: book.data.updated_at,
    title: book.data.title,
    genre: book.data.genre,
    language: book.data.writing_language,
    settings: book.data.settings,
  }
  const current = draft !== null && draft.key === saved.key ? draft : saved
  const setTitle = (title: string) => setDraft({ ...current, title })
  const setGenre = (genre: string) => setDraft({ ...current, genre })
  const setLanguage = (language: string) => setDraft({ ...current, language })
  const update = (patchSettings: Partial<BookSettings>) =>
    setDraft({ ...current, settings: { ...current.settings, ...patchSettings } })
  const settings = current.settings
  const title = current.title
  const genre = current.genre
  const language = current.language

  const save = () => {
    patch.mutate(
      {
        title: title.trim(),
        genre,
        writing_language: language,
        settings,
      },
      { onError: showError },
    )
  }

  return (
    <div className="mx-auto max-w-2xl space-y-6 p-6">
      <header className="flex items-end justify-between">
        <h1 className="text-xl font-semibold">{t("book:book_settings")}</h1>
        <Button size="sm" disabled={patch.isPending} onClick={save}>
          {patch.isPending ? t("common:saving") : t("common:save")}
        </Button>
      </header>

      <section className="space-y-3">
        <div className="space-y-1.5">
          <Label htmlFor="settings-title">{t("library:book_title")}</Label>
          <Input id="settings-title" value={title} onChange={(event) => setTitle(event.target.value)} />
        </div>
        <div className="grid grid-cols-2 gap-3">
          <div className="space-y-1.5">
            <Label htmlFor="settings-genre">{t("library:book_genre")}</Label>
            <Input
              id="settings-genre"
              value={genre}
              onChange={(event) => setGenre(event.target.value)}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="settings-language">{t("library:book_language")}</Label>
            <Select value={language} onValueChange={setLanguage}>
              <SelectTrigger id="settings-language">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {LANGUAGES.map((code) => (
                  <SelectItem key={code} value={code}>
                    {t(`common:language_code.${code}`, { defaultValue: code })}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>
      </section>

      <section className="space-y-3">
        <h2 className="text-sm font-medium">{t("book:writer_defaults")}</h2>
        <div className="space-y-1.5">
          <Label htmlFor="settings-granularity">{t("book:granularity")}</Label>
          <Select
            value={settings.granularity}
            onValueChange={(value) => update({ granularity: value as Granularity })}
          >
            <SelectTrigger id="settings-granularity">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {GRANULARITIES.map((granularity) => (
                <SelectItem key={granularity} value={granularity}>
                  {t(`chapter:granularity.${granularity}`, { defaultValue: granularity })}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="flex items-center justify-between">
          <Label htmlFor="settings-auto-pause">{t("book:auto_pause")}</Label>
          <Switch
            id="settings-auto-pause"
            checked={settings.auto_pause}
            onCheckedChange={(checked) => update({ auto_pause: checked })}
          />
        </div>
        <div className="flex items-center justify-between">
          <Label htmlFor="settings-pause-outline">{t("book:pause_after_outline")}</Label>
          <Switch
            id="settings-pause-outline"
            checked={settings.pause_after_outline}
            onCheckedChange={(checked) => update({ pause_after_outline: checked })}
          />
        </div>
        <div className="flex items-center justify-between">
          <Label htmlFor="settings-auto-maintenance">{t("book:auto_maintenance")}</Label>
          <Switch
            id="settings-auto-maintenance"
            checked={settings.auto_maintenance}
            onCheckedChange={(checked) => update({ auto_maintenance: checked })}
          />
        </div>
      </section>

      <section className="space-y-3">
        <h2 className="text-sm font-medium">{t("book:target_length")}</h2>
        <div className="grid grid-cols-2 gap-3">
          <div className="space-y-1.5">
            <Label htmlFor="settings-target-min">{t("book:target_words_min")}</Label>
            <Input
              id="settings-target-min"
              type="number"
              value={settings.target_words_min}
              onChange={(event) =>
                update({ target_words_min: Number(event.target.value) || 0 })
              }
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="settings-target-max">{t("book:target_words_max")}</Label>
            <Input
              id="settings-target-max"
              type="number"
              value={settings.target_words_max}
              onChange={(event) =>
                update({ target_words_max: Number(event.target.value) || 0 })
              }
            />
          </div>
        </div>
      </section>

      <section className="space-y-3">
        <h2 className="text-sm font-medium">{t("book:memory_windows")}</h2>
        <div className="grid grid-cols-2 gap-3">
          <div className="space-y-1.5">
            <Label htmlFor="settings-recent">{t("book:recent_summary_count")}</Label>
            <Input
              id="settings-recent"
              type="number"
              value={settings.recent_summary_count}
              onChange={(event) =>
                update({ recent_summary_count: Number(event.target.value) || 0 })
              }
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="settings-midterm">{t("book:midterm_window")}</Label>
            <Input
              id="settings-midterm"
              type="number"
              value={settings.midterm_window}
              onChange={(event) => update({ midterm_window: Number(event.target.value) || 0 })}
            />
          </div>
        </div>
      </section>
    </div>
  )
}
