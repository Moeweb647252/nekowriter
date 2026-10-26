/** One library card (PROJECT §8.3). */
import { Link, useNavigate } from "react-router"
import { useTranslation } from "react-i18next"

import { useDeleteBook, usePatchBook } from "@/api/hooks/books"
import type { BookSummaryDto } from "@/api/gen/BookSummaryDto"
import { ConfirmDialog } from "@/components/common/ConfirmDialog"
import { StatusBadge } from "@/components/common/StatusBadge"
import { useErrorToast } from "@/components/common/ErrorToast"
import { Badge } from "@/components/ui/badge"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Button } from "@/components/ui/button"
import { formatRelativeTime, formatWords } from "@/lib/format"
import { cn } from "@/lib/utils"
import { MoreHorizontal, Sparkles } from "lucide-react"
import { useState } from "react"

export type BookCardProps = {
  book: BookSummaryDto
}

/** A card with the title, genre, language, counts, the latest chapter's status and a menu. */
export function BookCard({ book }: BookCardProps) {
  const { t, i18n } = useTranslation()
  const navigate = useNavigate()
  const showError = useErrorToast()
  const patchBook = usePatchBook(book.id)
  const deleteBook = useDeleteBook()
  const [confirmDelete, setConfirmDelete] = useState(false)
  const [typedName, setTypedName] = useState("")

  const archived = book.status === "archived"
  const conceiving = book.status === "conceiving"
  // A conceiving book's materials live in the studio (PROJECT §8.7), so that is the destination
  // its title, its "Open" item and its studio link use; every other status opens the overview.
  const openTarget = conceiving ? `/books/${book.id}/conception` : `/books/${book.id}`

  return (
    <Card className={cn("flex flex-col", conceiving && "ring-2 ring-status-waiting/40")}>
      <CardHeader className="flex-row items-start justify-between gap-2 space-y-0">
        <div className="min-w-0">
          <CardTitle className="truncate">
            <button
              type="button"
              className="text-left hover:underline"
              onClick={() => void navigate(openTarget)}
            >
              {book.title}
            </button>
          </CardTitle>
          <p className="mt-1 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
            <span>{[book.genre, book.writing_language].filter(Boolean).join(" · ")}</span>
            {conceiving ? (
              <Badge variant="outline" className="gap-1 text-status-waiting">
                <Sparkles aria-hidden="true" />
                {t("library:conceiving")}
              </Badge>
            ) : null}
          </p>
        </div>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="icon" aria-label={t("library:book_menu")}>
              <MoreHorizontal />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onSelect={() => void navigate(openTarget)}>
              {t("library:open")}
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => void navigate(`/books/${book.id}/book-settings`)}>
              {t("library:rename")}
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              disabled={patchBook.isPending}
              onSelect={() => {
                patchBook.mutate(
                  { status: archived ? "active" : "archived" },
                  { onError: showError },
                )
              }}
            >
              {archived ? t("library:unarchive") : t("library:archive")}
            </DropdownMenuItem>
            <DropdownMenuItem
              variant="destructive"
              onSelect={() => {
                setTypedName("")
                setConfirmDelete(true)
              }}
            >
              {t("common:delete")}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </CardHeader>
      <CardContent className="mt-auto space-y-2">
        {/* The counts and the latest chapter are shown for every status, conceiving included: a
            conceiving book can already own a chapter (chapter creation was not gated on finalize
            until later), so replacing this block would hide real data. The conceiving
            affordances are added below instead. */}
        <p className="text-xs text-muted-foreground">
          {t("library:chapters_words", {
            chapters: book.chapter_count,
            words: formatWords(book.word_count, i18n.language),
          })}
        </p>
        {book.latest_chapter === null ? (
          <p className="text-xs text-muted-foreground">{t("library:no_chapters")}</p>
        ) : (
          <p className="flex items-center gap-2 text-xs">
            <span className="truncate">
              {book.latest_chapter.number}. {book.latest_chapter.title || t("library:untitled")}
            </span>
            <StatusBadge status={book.latest_chapter.status} />
          </p>
        )}
        {conceiving ? (
          // The card is the only way back into a book whose conception session failed or was
          // closed, so the studio link belongs here rather than only in the ⋯ menu.
          <div className="space-y-2 rounded-md border border-status-waiting/40 bg-status-waiting/10 p-2">
            <p className="text-xs text-muted-foreground">{t("library:conceiving_hint")}</p>
            <Button asChild variant="outline" size="sm">
              <Link to={openTarget}>{t("book:conception.continue")}</Link>
            </Button>
          </div>
        ) : null}
        <p className="text-xs text-muted-foreground">
          {t("library:recent_activity", {
            when: formatRelativeTime(book.updated_at, i18n.language),
          })}
        </p>
      </CardContent>

      <ConfirmDialog
        open={confirmDelete}
        onOpenChange={setConfirmDelete}
        title={t("library:delete_title", { title: book.title })}
        description={t("library:delete_body")}
        confirmLabel={t("library:delete_confirm")}
        confirmText={book.title}
        typed={typedName}
        onTypedChange={setTypedName}
        pending={deleteBook.isPending}
        onConfirm={() => {
          deleteBook.mutate(book.id, {
            onSuccess: () => setConfirmDelete(false),
            onError: showError,
          })
        }}
      />
    </Card>
  )
}
