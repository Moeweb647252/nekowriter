/** The New book dialog (PROJECT §8.3; conception joins in M9). */
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

/** Writing languages offered by the dialog; the API accepts any BCP-47 tag. */
const LANGUAGES = ["zh-CN", "en", "ja", "ko", "fr", "de", "es", "ru"] as const

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
  const [error, setError] = useState<string | null>(null)

  const reset = () => {
    setTitle("")
    setIdea("")
    setGenre("")
    setError(null)
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
          <DialogDescription>{t("library:new_book_description")}</DialogDescription>
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
                  {LANGUAGES.map((code) => (
                    <SelectItem key={code} value={code}>
                      {t(`common:language_code.${code}`, { defaultValue: code })}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </div>
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
                  start_conception: false,
                },
                {
                  onSuccess: (response) => {
                    onOpenChange(false)
                    reset()
                    void navigate(`/books/${response.book.id}`)
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
