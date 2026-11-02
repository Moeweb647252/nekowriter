/**
 * The Write dialog (PROJECT §8.5, spec/11 §5.2, D-146): guidance, granularity and auto-pause,
 * then Start. It drafts from the accepted outline; the pause-after-outline switch belongs to the
 * Outline dialog.
 *
 * A `409 chapter.previous_not_ready` is a normal outcome: the dialog shows the reason and offers
 * **Write anyway**, which resends the same input with `force`. A `409 chapter.outline_missing`
 * (a race with an author edit) offers **Write the outline** instead.
 */
import { useState } from "react"
import { useTranslation } from "react-i18next"

import { useWriteChapter } from "@/api/hooks/chapters"
import { useErrorToast, useWarningToasts } from "@/components/common/ErrorToast"
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
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import type { BookSettings } from "@/api/gen/BookSettings"
import type { ChapterWriteInput } from "@/api/gen/ChapterWriteInput"
import type { Granularity } from "@/api/gen/Granularity"
import { ApiError } from "@/api/client"
import {
  PreviousNotReadyNotice,
  type PreviousNotReady,
} from "@/features/chapter/PreviousNotReady"

const GRANULARITIES: Granularity[] = ["sentence", "paragraph", "free", "chapter"]

export type WriteDialogProps = {
  chapterId: number
  bookId: number
  settings: BookSettings
  guidance: string
  open: boolean
  onOpenChange: (open: boolean) => void
  onStarted: (sessionId: number) => void
  /** Opens the Outline dialog when the chapter lost its outline (a race). */
  onWriteOutline: () => void
  /** The chapter now has an outline: clear a previously shown race warning. */
  hasOutline: boolean
}

export function WriteDialog({
  chapterId,
  bookId,
  settings,
  guidance,
  open,
  onOpenChange,
  onStarted,
  onWriteOutline,
  hasOutline,
}: WriteDialogProps) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const showWarnings = useWarningToasts()
  const write = useWriteChapter(chapterId, bookId)
  const [draftGuidance, setDraftGuidance] = useState(guidance)
  // The dialog is mounted permanently by `ChapterHeader`: adopt a guidance value changed by the
  // other dialog, but never overwrite what the author is typing (R2-01).
  const [lastServerGuidance, setLastServerGuidance] = useState(guidance)
  if (guidance !== lastServerGuidance) {
    setLastServerGuidance(guidance)
    if (draftGuidance === lastServerGuidance) {
      setDraftGuidance(guidance)
    }
  }
  const [granularity, setGranularity] = useState<Granularity>(settings.granularity)
  const [autoPause, setAutoPause] = useState(settings.auto_pause)
  const [notReady, setNotReady] = useState<PreviousNotReady | null>(null)
  const [outlineMissing, setOutlineMissing] = useState(false)
  // A latched warning made a reopened dialog insist the chapter had no outline after one was
  // created. Adjust state while rendering on the two prop transitions (F10), so no effect can
  // cascade renders and a refetch cannot clobber the author's typing.
  const [prevOpen, setPrevOpen] = useState(open)
  const [prevHasOutline, setPrevHasOutline] = useState(hasOutline)
  if (open !== prevOpen) {
    setPrevOpen(open)
    if (open) {
      setOutlineMissing(false)
      setNotReady(null)
    }
  }
  if (hasOutline !== prevHasOutline) {
    setPrevHasOutline(hasOutline)
    if (hasOutline) {
      setOutlineMissing(false)
    }
  }
  // `ChapterHeader` is keyed by chapter id in production; this reset is the same guard for a
  // direct remount that is not keyed (R3-02): per-chapter drafts and warnings never cross chapters.
  const [lastChapterId, setLastChapterId] = useState(chapterId)
  if (chapterId !== lastChapterId) {
    setLastChapterId(chapterId)
    setDraftGuidance(guidance)
    setLastServerGuidance(guidance)
    setNotReady(null)
    setOutlineMissing(false)
  }

  const start = (force: boolean) => {
    const input: ChapterWriteInput = {
      guidance: draftGuidance,
      granularity,
      auto_pause: autoPause,
      force,
    }
    write.mutate(input, {
      onSuccess: ({ session_id, warnings }) => {
        showWarnings(warnings ?? [])
        setNotReady(null)
        setOutlineMissing(false)
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
        if (error instanceof ApiError && error.code === "chapter.outline_missing") {
          setOutlineMissing(true)
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
          <DialogTitle>{t("chapter:write_dialog.title")}</DialogTitle>
          <DialogDescription>{t("chapter:write_dialog.description")}</DialogDescription>
        </DialogHeader>

        {notReady === null ? null : <PreviousNotReadyNotice notReady={notReady} />}
        {outlineMissing ? (
          <div className="space-y-2 rounded-md border border-status-waiting/40 bg-status-waiting/10 p-3 text-sm">
            <p className="font-medium">{t("chapter:write_dialog.outline_missing_title")}</p>
            <p className="text-muted-foreground">
              {t("chapter:write_dialog.outline_missing_hint")}
            </p>
          </div>
        ) : null}

        <div className="space-y-4">
          <div className="space-y-1.5">
            <Label htmlFor="write-guidance">{t("chapter:write_dialog.guidance")}</Label>
            <Textarea
              id="write-guidance"
              value={draftGuidance}
              rows={3}
              placeholder={t("chapter:write_dialog.guidance_placeholder")}
              onChange={(event) => setDraftGuidance(event.target.value)}
            />
          </div>

          <div className="space-y-1.5">
            <div className="flex items-center gap-1.5">
              <Label>{t("agent:writer.granularity")}</Label>
              <Tooltip>
                <TooltipTrigger asChild>
                  <span className="cursor-help text-xs text-muted-foreground">ⓘ</span>
                </TooltipTrigger>
                <TooltipContent className="max-w-72">
                  {t("agent:writer.granularity_hint")}
                </TooltipContent>
              </Tooltip>
            </div>
            <ToggleGroup
              type="single"
              variant="outline"
              className="grid grid-cols-4"
              value={granularity}
              onValueChange={(next) => {
                if (next !== "") {
                  setGranularity(next as Granularity)
                }
              }}
              aria-label={t("agent:writer.granularity")}
            >
              {GRANULARITIES.map((value) => (
                <ToggleGroupItem key={value} value={value}>
                  {t(`agent:granularity.${value}`)}
                </ToggleGroupItem>
              ))}
            </ToggleGroup>
          </div>

          <div className="flex items-center gap-2">
            <Switch
              id="write-auto-pause"
              checked={autoPause}
              onCheckedChange={setAutoPause}
              aria-label={t("agent:writer.auto_pause")}
            />
            <Label htmlFor="write-auto-pause" className="text-sm font-normal">
              {t("agent:writer.auto_pause")}
            </Label>
          </div>
        </div>

        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            {t("common:cancel")}
          </Button>
          {outlineMissing ? (
            <Button
              onClick={() => {
                onOpenChange(false)
                onWriteOutline()
              }}
            >
              {t("chapter:write_dialog.write_outline")}
            </Button>
          ) : notReady === null ? (
            <Button disabled={write.isPending} onClick={() => start(false)}>
              {write.isPending ? t("common:loading") : t("chapter:write_dialog.start")}
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
