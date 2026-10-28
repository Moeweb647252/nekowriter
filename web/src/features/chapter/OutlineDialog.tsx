/**
 * The Outline dialog (PROJECT §8.5, spec/11 §5.2, D-146): guidance, the pause-after-outline
 * switch and Start. A `409 chapter.previous_not_ready` is a normal outcome: the dialog shows the
 * reason and offers **Write anyway**, which resends the same input with `force`.
 */
import { useState } from "react"
import { useTranslation } from "react-i18next"

import { useWriteOutlineChapter } from "@/api/hooks/chapters"
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
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import { Textarea } from "@/components/ui/textarea"
import type { BookSettings } from "@/api/gen/BookSettings"
import type { ChapterOutlineInput } from "@/api/gen/ChapterOutlineInput"
import { ApiError } from "@/api/client"
import {
  PreviousNotReadyNotice,
  type PreviousNotReady,
} from "@/features/chapter/PreviousNotReady"

export type OutlineDialogProps = {
  chapterId: number
  bookId: number
  settings: BookSettings
  guidance: string
  open: boolean
  onOpenChange: (open: boolean) => void
  onStarted: (sessionId: number) => void
}

export function OutlineDialog({
  chapterId,
  bookId,
  settings,
  guidance,
  open,
  onOpenChange,
  onStarted,
}: OutlineDialogProps) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const write = useWriteOutlineChapter(chapterId, bookId)
  const [draftGuidance, setDraftGuidance] = useState(guidance)
  // Same last-seen-server-value rule as WriteDialog: the outline dialog is permanently mounted,
  // so a guidance edit from a later write dialog must be adopted when untouched (R2-01).
  const [lastServerGuidance, setLastServerGuidance] = useState(guidance)
  if (guidance !== lastServerGuidance) {
    setLastServerGuidance(guidance)
    if (draftGuidance === lastServerGuidance) {
      setDraftGuidance(guidance)
    }
  }
  const [pauseAfterOutline, setPauseAfterOutline] = useState(settings.pause_after_outline)
  const [notReady, setNotReady] = useState<PreviousNotReady | null>(null)
  // `previous_not_ready` is a transient decision state: reopening starts from Start again (R2-02).
  const [prevOpen, setPrevOpen] = useState(open)
  if (open !== prevOpen) {
    setPrevOpen(open)
    if (open) {
      setNotReady(null)
    }
  }
  // The chapter-id reset mirrors WriteDialog (R3-02); the permanent mount must not carry a
  // draft or a previous 409 into another chapter.
  const [lastChapterId, setLastChapterId] = useState(chapterId)
  if (chapterId !== lastChapterId) {
    setLastChapterId(chapterId)
    setDraftGuidance(guidance)
    setLastServerGuidance(guidance)
    setNotReady(null)
  }

  const start = (force: boolean) => {
    const input: ChapterOutlineInput = {
      guidance: draftGuidance,
      pause_after_outline: pauseAfterOutline,
      force,
    }
    write.mutate(input, {
      onSuccess: ({ session_id }) => {
        setNotReady(null)
        onOpenChange(false)
        onStarted(session_id)
      },
      onError: (error) => {
        if (error instanceof ApiError && error.code === "chapter.previous_not_ready") {
          const details = (error.details ?? {}) as Record<string, unknown>
          setNotReady({
            chapterNumber:
              typeof details.chapter_number === "number" ? details.chapter_number : undefined,
            reason: typeof details.reason === "string" ? details.reason : undefined,
          })
          return
        }
        showError(error)
      },
    })
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>{t("chapter:outline_dialog.title")}</DialogTitle>
          <DialogDescription>{t("chapter:outline_dialog.description")}</DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          <div className="space-y-1.5">
            <Label htmlFor="outline-guidance">{t("chapter:write_dialog.guidance")}</Label>
            <Textarea
              id="outline-guidance"
              value={draftGuidance}
              rows={4}
              placeholder={t("chapter:outline_dialog.guidance_placeholder")}
              onChange={(event) => setDraftGuidance(event.target.value)}
            />
          </div>
          <div className="flex items-center justify-between">
            <Label htmlFor="outline-pause-after-outline" className="text-sm font-normal">
              {t("book:pause_after_outline")}
            </Label>
            <Switch
              id="outline-pause-after-outline"
              checked={pauseAfterOutline}
              onCheckedChange={setPauseAfterOutline}
            />
          </div>
          {notReady === null ? null : <PreviousNotReadyNotice notReady={notReady} />}
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            {t("common:cancel")}
          </Button>
          {notReady === null ? (
            <Button disabled={write.isPending} onClick={() => start(false)}>
              {write.isPending ? t("common:loading") : t("chapter:outline_dialog.start")}
            </Button>
          ) : (
            <Button disabled={write.isPending} onClick={() => start(true)}>
              {t("chapter:write_dialog.write_anyway")}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
