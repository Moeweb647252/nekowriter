/** The in-book breadcrumb with a book switcher (PROJECT §8.2). */
import { Link, useNavigate } from "react-router"
import { useTranslation } from "react-i18next"
import { ChevronDown } from "lucide-react"

import { useBooks } from "@/api/hooks/books"
import type { BookDto } from "@/api/gen/BookDto"
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbList,
  BreadcrumbSeparator,
} from "@/components/ui/breadcrumb"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"

export type BookBreadcrumbProps = {
  book: BookDto
}

/** `◆ NekoWriter / 青云劫 ▾`. */
export function BookBreadcrumb({ book }: BookBreadcrumbProps) {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const books = useBooks()
  const others = (books.data ?? []).filter((candidate) => candidate.id !== book.id)

  return (
    <div className="flex h-9 shrink-0 items-center gap-2 border-b px-3">
      <Breadcrumb>
        <BreadcrumbList>
          <BreadcrumbItem>
            <Link to="/" className="text-xs text-muted-foreground hover:text-foreground">
              {t("common:app_name")}
            </Link>
          </BreadcrumbItem>
          <BreadcrumbSeparator />
          <BreadcrumbItem>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="ghost" size="sm" className="h-7 gap-1 px-1.5 text-xs font-medium">
                  {book.title}
                  <ChevronDown className="size-3" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start">
                <DropdownMenuLabel>{t("library:switch_book")}</DropdownMenuLabel>
                {others.length === 0 ? (
                  <DropdownMenuItem disabled>{t("library:no_other_books")}</DropdownMenuItem>
                ) : (
                  others.map((candidate) => (
                    <DropdownMenuItem
                      key={candidate.id}
                      onSelect={() => void navigate(`/books/${candidate.id}`)}
                    >
                      {candidate.title}
                    </DropdownMenuItem>
                  ))
                )}
              </DropdownMenuContent>
            </DropdownMenu>
          </BreadcrumbItem>
        </BreadcrumbList>
      </Breadcrumb>
    </div>
  )
}
