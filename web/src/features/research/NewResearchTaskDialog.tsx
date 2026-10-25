/** The "New research task" dialog (PROJECT §8.10, spec/10 §2.5). */
import { useState } from "react"
import { useTranslation } from "react-i18next"

import { useCreateResearchTask } from "@/api/hooks/research"
import { useBooks } from "@/api/hooks/books"
import { useErrorToast } from "@/components/common/ErrorToast"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
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
import { Spinner } from "@/components/ui/spinner"
import { Textarea } from "@/components/ui/textarea"
import { useUiStore } from "@/stores/ui"

/** The dialog's own state; `bookId` presets the book inside a book workspace. */
export function NewResearchTaskDialog({
  open,
  onOpenChange,
  bookId,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  bookId?: number
}) {
  const { t, i18n } = useTranslation()
  const showError = useErrorToast()
  const create = useCreateResearchTask()
  const books = useBooks()
  // A standalone task has no book to take a language from, so the UI language is the default
  // (PROJECT §8.10); the author can change it on the task page.
  const language = useUiStore((state) => state.language)

  const [topic, setTopic] = useState("")
  const [requirements, setRequirements] = useState("")
  const [scopeName, setScopeName] = useState("")
  const [book, setBook] = useState<string>(bookId === undefined ? "none" : String(bookId))
  const [error, setError] = useState<string | null>(null)

  const reset = () => {
    setTopic("")
    setRequirements("")
    setScopeName("")
    setBook(bookId === undefined ? "none" : String(bookId))
    setError(null)
  }

  const submit = () => {
    // The language rule of spec/10 §2.5: required without a book, ignored with one.
    const trimmedTopic = topic.trim()
    const trimmedScope = scopeName.trim()
    if (trimmedTopic === "") {
      setError(t("knowledge:research.topic_required"))
      return
    }
    if (trimmedScope === "") {
      setError(t("knowledge:research.scope_required"))
      return
    }
    const linkedBook = book === "none" ? undefined : Number(book)
    create.mutate(
      {
        topic: trimmedTopic,
        requirements: requirements.trim(),
        scope_name: trimmedScope,
        book_id: linkedBook,
        // The UI language is the default for a standalone task (PROJECT §8.10); with a book the
        // field is ignored (spec/10 §2.5).
        language: linkedBook === undefined ? language : undefined,
      },
      {
        onSuccess: () => {
          reset()
          onOpenChange(false)
        },
        onError: showError,
      },
    )
  }

  return (
    <Dialog open={open} onOpenChange={(next) => (next ? onOpenChange(true) : (reset(), onOpenChange(false)))}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>{t("knowledge:research.new_task")}</DialogTitle>
        </DialogHeader>
        <div className="space-y-3">
          <div className="space-y-1">
            <Label htmlFor="new-research-topic">{t("knowledge:research.topic")}</Label>
            <Input
              id="new-research-topic"
              value={topic}
              placeholder={t("knowledge:research.topic_placeholder")}
              onChange={(event) => setTopic(event.target.value)}
            />
          </div>
          <div className="space-y-1">
            <Label htmlFor="new-research-requirements">{t("knowledge:research.requirements")}</Label>
            <Textarea
              id="new-research-requirements"
              rows={2}
              value={requirements}
              placeholder={t("knowledge:research.requirements_placeholder")}
              onChange={(event) => setRequirements(event.target.value)}
            />
          </div>
          <div className="space-y-1">
            <Label htmlFor="new-research-scope">{t("knowledge:research.scope_name")}</Label>
            <Input
              id="new-research-scope"
              value={scopeName}
              placeholder={t("knowledge:research.scope_name_placeholder")}
              onChange={(event) => setScopeName(event.target.value)}
            />
          </div>
          <div className="space-y-1">
            <Label htmlFor="new-research-book">{t("knowledge:research.book")}</Label>
            <Select value={book} onValueChange={setBook}>
              <SelectTrigger id="new-research-book">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="none">{t("knowledge:research.book_none")}</SelectItem>
                {(books.data ?? []).map((item) => (
                  <SelectItem key={item.id} value={String(item.id)}>
                    {item.title}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <p className="text-xs text-muted-foreground">
            {book === "none"
              ? t("knowledge:research.language_hint", { language: i18n.language })
              : t("knowledge:research.language_book_hint")}
          </p>
          {error === null ? null : <p className="text-xs text-destructive">{error}</p>}
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={() => (reset(), onOpenChange(false))}>
            {t("knowledge:research.cancel")}
          </Button>
          <Button onClick={submit} disabled={create.isPending}>
            {create.isPending ? <Spinner /> : null}
            {t("knowledge:research.create")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
