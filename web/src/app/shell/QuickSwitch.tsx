/**
 * Quick switch (PROJECT §8.13, spec/11 §7): ⌘/Ctrl+K opens a searchable list of the app's pages,
 * the libraries' books and — when a book is open — that book's pages and chapters.
 *
 * It reads from the same query keys the screens use (`api/keys.ts`), so the list is usually
 * already cached and filtering never blocks on the network.
 */
import { useMemo } from "react"
import { useQueries } from "@tanstack/react-query"
import { useLocation, useNavigate } from "react-router"
import { useTranslation } from "react-i18next"
import { BookOpen, FileText } from "lucide-react"

import type { ChapterListItemDto } from "@/api/gen/ChapterListItemDto"
import type { TrackDto } from "@/api/gen/TrackDto"
import { keys } from "@/api/keys"
import * as trackEndpoints from "@/api/endpoints/tracks"
import { useBooks } from "@/api/hooks/books"
import { useTracks } from "@/api/hooks/tracks"
import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from "@/components/ui/command"
import { formatChapterLabel } from "@/lib/chapterLabel"

type QuickSwitchProps = {
  open: boolean
  onOpenChange: (open: boolean) => void
}

/** One destination: its label and its route. */
type Destination = { label: string; to: string }

/** The book id of the current route, when the route is inside a book. */
export function bookIdOf(pathname: string): number | null {
  const match = /^\/books\/(\d+)(?:\/|$)/.exec(pathname)
  if (match === null) {
    return null
  }
  const id = Number(match[1])
  return Number.isFinite(id) && id > 0 ? id : null
}

/** The current book's pages, in navigator order (spec/11 §5.1). */
export function bookPages(t: (key: string) => string, bookId: number): Destination[] {
  return [
    { label: t("book:overview"), to: `/books/${bookId}` },
    { label: t("book:conception.title"), to: `/books/${bookId}/conception` },
    { label: t("book:canon"), to: `/books/${bookId}/canon/entities` },
    { label: t("canon:timeline"), to: `/books/${bookId}/canon/timeline` },
    { label: t("canon:memory"), to: `/books/${bookId}/canon/memory` },
    { label: t("canon:foreshadowing"), to: `/books/${bookId}/canon/foreshadowing` },
    { label: t("book:changes"), to: `/books/${bookId}/canon/changes` },
    { label: t("book:proposals"), to: `/books/${bookId}/proposals` },
    { label: t("book:knowledge"), to: `/books/${bookId}/knowledge` },
    { label: t("book:prompts"), to: `/books/${bookId}/prompts` },
    { label: t("book:book_settings"), to: `/books/${bookId}/book-settings` },
  ]
}

export function QuickSwitch({ open, onOpenChange }: QuickSwitchProps) {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const location = useLocation()
  const bookId = useMemo(() => bookIdOf(location.pathname), [location.pathname])

  const books = useBooks()
  const tracks = useTracks(bookId ?? 0)
  const chapterQueries = useQueries({
    queries: (tracks.data ?? []).map((track: TrackDto) => ({
      queryKey: keys.tracks.chapters(track.id),
      queryFn: () => trackEndpoints.listChapters(track.id),
      enabled: bookId !== null,
    })),
  })

  const globalPages: Destination[] = [
    { label: t("common:library"), to: "/" },
    { label: t("common:knowledge"), to: "/knowledge" },
    { label: t("knowledge:research.title"), to: "/research" },
    { label: t("common:prompts"), to: "/prompts" },
    { label: t("common:settings"), to: "/settings" },
  ]
  const pages = bookId === null ? globalPages : [...globalPages, ...bookPages(t, bookId)]

  /** Every chapter of the open book, labelled with its track. */
  const chapters: Destination[] = []
  if (bookId !== null) {
    const trackRows = tracks.data ?? []
    trackRows.forEach((track, index) => {
      const rows: ChapterListItemDto[] = chapterQueries[index]?.data ?? []
      for (const chapter of rows) {
        chapters.push({
          label: formatChapterLabel(t, { number: chapter.number, track }),
          to: `/books/${bookId}/chapters/${chapter.id}`,
        })
      }
    })
  }

  const open_ = (to: string) => {
    onOpenChange(false)
    void navigate(to)
  }

  return (
    <CommandDialog open={open} onOpenChange={onOpenChange} title={t("common:quick_switch")}>
      <CommandInput placeholder={t("common:quick_switch_hint")} />
      <CommandList>
        <CommandEmpty>{t("common:quick_switch_empty")}</CommandEmpty>
        <CommandGroup heading={t("common:quick_switch_pages")}>
          {pages.map((page) => (
            <CommandItem key={page.to} value={`${page.label} ${page.to}`} onSelect={() => open_(page.to)}>
              <FileText aria-hidden="true" />
              <span>{page.label}</span>
            </CommandItem>
          ))}
        </CommandGroup>
        {chapters.length === 0 ? null : (
          <>
            <CommandSeparator />
            <CommandGroup heading={t("common:quick_switch_chapters")}>
              {chapters.map((chapter) => (
                <CommandItem
                  key={chapter.to}
                  value={`${chapter.label} ${chapter.to}`}
                  onSelect={() => open_(chapter.to)}
                >
                  <FileText aria-hidden="true" />
                  <span>{chapter.label}</span>
                </CommandItem>
              ))}
            </CommandGroup>
          </>
        )}
        <CommandSeparator />
        <CommandGroup heading={t("common:quick_switch_books")}>
          {(books.data ?? []).map((book) => (
            <CommandItem
              key={book.id}
              value={`${book.title} ${book.genre}`}
              onSelect={() => open_(`/books/${book.id}`)}
            >
              <BookOpen aria-hidden="true" />
              <span>{book.title}</span>
            </CommandItem>
          ))}
        </CommandGroup>
      </CommandList>
    </CommandDialog>
  )
}
