/**
 * The New book dialog (PROJECT §8.3, F2 step 1): title, rough idea, genre, writing language, and
 * "Start with the conception agent" (on by default). With conception the book is created
 * `conceiving`, its session starts through the create response, and the studio opens (PROJECT
 * §8.7); without it the book starts active with empty materials.
 */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { useNavigate } from "react-router"

import { useCreateBook } from "@/api/hooks/books"
import { useErrorToast } from "@/components/common/ErrorToast"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Textarea } from "@/components/ui/textarea"
import { Checkbox } from "@/components/ui/checkbox"
import { WRITING_LANGUAGES } from "@/lib/languages"

export type NewBookDialogProps = {
  open: boolean
  onOpenChange: (open: boolean) => void
}

export function NewBookDialog({ open, onOpenChange }: NewBookDialogProps) {
  const { t, i18n } = useTranslation()
  const navigate = useNavigate()
  const showError = useErrorToast()
  const createBook = useCreateBook()
  const [title, setTitle] = useState("")
  const [idea, setIdea] = useState("")
  const [genre, setGenre] = useState("")
  const [language, setLanguage] = useState<string>(i18n.language.startsWith("zh") ? "zh-CN" : "en")
  // Conception is on by default (PROJECT §8.3 / F2 step 1). The knowledge check runs before it
  // and auto-starts conception unless the author unchecks the second box.
  const [startConception, setStartConception] = useState(true)
  const [autoStartConception, setAutoStartConception] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const reset = () => {
    setTitle("")
    setIdea("")
    setGenre("")
    setError(null)
    setStartConception(true)
    setAutoStartConception(true)
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) {
          reset()
        }
        onOpenChange(next)
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("library:new_book")}</DialogTitle>
          <DialogDescription>
            {startConception
              ? t("library:new_book_conception_description")
              : t("library:new_book_description")}
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-3">
          <div className="space-y-1.5">
            <Label htmlFor="book-title">{t("library:book_title")}</Label>
            <Input
              id="book-title"
              value={title}
              onChange={(event) => setTitle(event.target.value)}
              autoFocus
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="book-idea">{t("library:book_idea")}</Label>
            <Textarea
              id="book-idea"
              value={idea}
              rows={3}
              onChange={(event) => setIdea(event.target.value)}
            />
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-1.5">
              <Label htmlFor="book-genre">{t("library:book_genre")}</Label>
              <Input
                id="book-genre"
                value={genre}
                onChange={(event) => setGenre(event.target.value)}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="book-language">{t("library:book_language")}</Label>
              <Select value={language} onValueChange={setLanguage}>
                <SelectTrigger id="book-language">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {WRITING_LANGUAGES.map((code) => (
                    <SelectItem key={code} value={code}>
                      {t(`common:language_code.${code}`, { defaultValue: code })}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </div>
          <div className="flex items-center gap-2">
            <Checkbox
              id="book-conception"
              checked={startConception}
              onCheckedChange={(checked) => setStartConception(checked === true)}
            />
            <Label htmlFor="book-conception" className="text-sm font-normal">
              {t("library:start_conception")}
            </Label>
          </div>
          {startConception ? (
            <div className="ml-6 flex items-center gap-2">
              <Checkbox
                id="book-auto-start-conception"
                checked={autoStartConception}
                onCheckedChange={(checked) => setAutoStartConception(checked === true)}
              />
              <Label htmlFor="book-auto-start-conception" className="text-sm font-normal">
                {t("library:auto_start_conception")}
              </Label>
            </div>
          ) : null}
          {error === null ? null : <p className="text-sm text-status-failed">{error}</p>}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t("common:cancel")}
          </Button>
          <Button
            disabled={title.trim().length === 0 || createBook.isPending}
            onClick={() => {
              setError(null)
              createBook.mutate(
                {
                  title: title.trim(),
                  idea,
                  genre,
                  writing_language: language,
                  start_conception: startConception,
                  knowledge_check: startConception,
                  auto_start_conception: autoStartConception,
                },
                {
                  onSuccess: (response) => {
                    onOpenChange(false)
                    reset()
                    // A conceiving book opens the studio even before it has a conception
                    // session: the studio owns the knowledge-check states. Every other status
                    // opens the overview.
                    const target =
                      response.book.status === "conceiving"
                        ? `/books/${response.book.id}/conception`
                        : `/books/${response.book.id}`
                    void navigate(target)
                  },
                  onError: (cause) => {
                    setError(t("common:error_body"))
                    showError(cause)
                  },
                },
              )
            }}
          >
            {createBook.isPending ? t("common:saving") : t("library:create_book")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
