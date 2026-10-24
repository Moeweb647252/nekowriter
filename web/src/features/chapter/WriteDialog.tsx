/**
 * The Write dialog (PROJECT §8.5, spec/11 §5.2): guidance, granularity, auto-pause and
 * pause-after-outline, then Start.
 *
 * A `409 chapter.previous_not_ready` from the write endpoint is a normal outcome: the dialog shows
 * the reason and offers **Write anyway**, which resends the same input with `force`.
 */
import { useState } from "react"
import { useTranslation } from "react-i18next"

import { useWriteChapter } from "@/api/hooks/chapters"
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
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import type { BookSettings } from "@/api/gen/BookSettings"
import type { ChapterWriteInput } from "@/api/gen/ChapterWriteInput"
import type { Granularity } from "@/api/gen/Granularity"
import { ApiError } from "@/api/client"

const GRANULARITIES: Granularity[] = ["sentence", "paragraph", "free", "chapter"]

/** The previous chapter's state the API reported in `409 chapter.previous_not_ready`. */
type NotReady = {
  chapterNumber?: number
  status?: string
  canonStatus?: string
}

export type WriteDialogProps = {
  chapterId: number
  bookId: number
  settings: BookSettings
  guidance: string
  open: boolean
  onOpenChange: (open: boolean) => void
  onStarted: (sessionId: number) => void
}

export function WriteDialog({
  chapterId,
  bookId,
  settings,
  guidance,
  open,
  onOpenChange,
  onStarted,
}: WriteDialogProps) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const write = useWriteChapter(chapterId, bookId)
  const [draftGuidance, setDraftGuidance] = useState(guidance)
  const [granularity, setGranularity] = useState<Granularity>(settings.granularity)
  const [autoPause, setAutoPause] = useState(settings.auto_pause)
  const [pauseAfterOutline, setPauseAfterOutline] = useState(settings.pause_after_outline)
  const [notReady, setNotReady] = useState<NotReady | null>(null)

  const start = (force: boolean) => {
    const input: ChapterWriteInput = {
      guidance: draftGuidance,
      granularity,
      auto_pause: autoPause,
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
        // `chapter.previous_not_ready` is the one failure the dialog resolves itself.
        if (error instanceof ApiError && error.code === "chapter.previous_not_ready") {
          const details = (error.details ?? {}) as Record<string, unknown>
          setNotReady({
            chapterNumber:
              typeof details.chapter_number === "number" ? details.chapter_number : undefined,
            status: typeof details.status === "string" ? details.status : undefined,
            canonStatus: typeof details.canon_status === "string" ? details.canon_status : undefined,
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
          <DialogTitle>{t("chapter:write_dialog.title")}</DialogTitle>
          <DialogDescription>{t("chapter:write_dialog.description")}</DialogDescription>
        </DialogHeader>

        {notReady === null ? null : (
          <div className="space-y-2 rounded-md border border-status-waiting/40 bg-status-waiting/10 p-3 text-sm">
            <p className="font-medium">{t("chapter:write_dialog.not_ready_title")}</p>
            <p className="text-muted-foreground">
              {t("chapter:write_dialog.not_ready_body", {
                number: notReady.chapterNumber ?? 0,
                status: notReady.status ?? "",
                canon: notReady.canonStatus ?? "",
              })}
            </p>
          </div>
        )}

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

          <div className="flex items-center gap-2">
            <Switch
              id="write-pause-outline"
              checked={pauseAfterOutline}
              onCheckedChange={setPauseAfterOutline}
              aria-label={t("chapter:write_dialog.pause_after_outline")}
            />
            <Label htmlFor="write-pause-outline" className="text-sm font-normal">
              {t("chapter:write_dialog.pause_after_outline")}
            </Label>
          </div>
        </div>

        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            {t("common:cancel")}
          </Button>
          {notReady === null ? (
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
