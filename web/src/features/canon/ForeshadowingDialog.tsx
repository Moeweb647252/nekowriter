/** Edit a foreshadowing item's direction and its timeline anchor (PROJECT §8.8,
 * spec/11 §5.5, D-170). */
import { useState } from "react"
import { useTranslation } from "react-i18next"

import type { ForeshadowingDto } from "@/api/gen/ForeshadowingDto"
import type { ForeshadowingPatch } from "@/api/gen/ForeshadowingPatch"
import type { PayoffPosition } from "@/api/gen/PayoffPosition"
import { usePatchForeshadowing } from "@/api/hooks/useCanon"
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
import { Textarea } from "@/components/ui/textarea"
import { EventPicker } from "@/features/canon/EntityPicker"
import { timelineErrorText } from "@/features/canon/timelineErrors"

const POSITIONS: PayoffPosition[] = ["before", "after", "at"]

export type ForeshadowingDialogProps = {
  bookId: number
  item: ForeshadowingDto
  open: boolean
  onOpenChange: (open: boolean) => void
}

export function ForeshadowingDialog({
  bookId,
  item,
  open,
  onOpenChange,
}: ForeshadowingDialogProps) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const patch = usePatchForeshadowing(bookId)
  const [title, setTitle] = useState(item.title)
  const [description, setDescription] = useState(item.description)
  const [direction, setDirection] = useState(item.planned_payoff)
  const [anchorId, setAnchorId] = useState<number | undefined>(
    item.payoff_event_id ?? undefined,
  )
  const [position, setPosition] = useState<PayoffPosition>(
    item.payoff_position ?? "after",
  )
  const [note, setNote] = useState("")
  const [errorText, setErrorText] = useState<string | null>(null)

  const canSave = title.trim() !== "" && !patch.isPending
  const anchorChanged = (item.payoff_event_id ?? undefined) !== anchorId

  const submit = () => {
    if (!canSave) {
      return
    }
    setErrorText(null)
    const next: ForeshadowingPatch = {}
    if (title.trim() !== item.title) {
      next.title = title.trim()
    }
    if (description !== item.description) {
      next.description = description
    }
    if (direction !== item.planned_payoff) {
      next.planned_payoff = direction
    }
    if (anchorChanged) {
      // The pair is cleared together; `null` means "no anchor" on the wire (D-170 T10).
      next.payoff_event_id = anchorId ?? null
      if (anchorId !== undefined) {
        next.payoff_position = position
      }
    } else if (anchorId !== undefined && position !== item.payoff_position) {
      next.payoff_position = position
    }
    if (note.trim() !== "") {
      next.note = note.trim()
    }
    if (Object.keys(next).length === 0) {
      onOpenChange(false)
      return
    }
    patch.mutate(
      { id: item.id, patch: next },
      {
        onSuccess: () => onOpenChange(false),
        onError: (error) => {
          const message = timelineErrorText(t, error)
          if (message !== null) {
            setErrorText(message)
            return
          }
          showError(error)
        },
      },
    )
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{t("canon:foreshadowing_dialog.title")}</DialogTitle>
          <DialogDescription>
            {t("canon:foreshadowing_dialog.description")}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-3">
          <div className="space-y-1.5">
            <Label htmlFor="foreshadowing-title">
              {t("canon:foreshadowing_dialog.title_field")}
            </Label>
            <Input
              id="foreshadowing-title"
              value={title}
              onChange={(event) => setTitle(event.target.value)}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="foreshadowing-description">
              {t("canon:foreshadowing_dialog.description_field")}
            </Label>
            <Textarea
              id="foreshadowing-description"
              rows={2}
              value={description}
              onChange={(event) => setDescription(event.target.value)}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="foreshadowing-direction">
              {t("canon:foreshadowing_dialog.direction")}
            </Label>
            <Textarea
              id="foreshadowing-direction"
              rows={2}
              value={direction}
              onChange={(event) => setDirection(event.target.value)}
            />
          </div>

          <div className="space-y-2 rounded-md border p-2">
            <Label>{t("canon:foreshadowing_dialog.anchor")}</Label>
            <div className="flex flex-wrap items-center gap-2">
              <EventPicker
                bookId={bookId}
                trackId={item.track_id ?? undefined}
                label={t("canon:foreshadowing_dialog.anchor_event")}
                value={anchorId}
                onChange={setAnchorId}
                includeId={item.payoff_event_id}
              />
              <select
                value={position}
                onChange={(event) => setPosition(event.target.value as PayoffPosition)}
                aria-label={t("canon:foreshadowing_dialog.anchor_position")}
                disabled={anchorId === undefined}
                className="h-8 rounded-md border bg-background px-2 text-xs disabled:opacity-50"
              >
                {POSITIONS.map((value) => (
                  <option key={value} value={value}>
                    {t(`canon:payoff_position.${value}`)}
                  </option>
                ))}
              </select>
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={anchorId === undefined}
                onClick={() => setAnchorId(undefined)}
              >
                {t("canon:foreshadowing_dialog.clear_anchor")}
              </Button>
            </div>
            <p className="text-xs text-muted-foreground">
              {t("canon:foreshadowing_dialog.anchor_hint")}
            </p>
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="foreshadowing-note">
              {t("canon:foreshadowing_dialog.change_note")}
            </Label>
            <Textarea
              id="foreshadowing-note"
              rows={2}
              value={note}
              onChange={(event) => setNote(event.target.value)}
            />
          </div>

          {errorText === null ? null : (
            <p role="alert" className="text-sm text-destructive">
              {errorText}
            </p>
          )}
        </div>

        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            {t("common:cancel")}
          </Button>
          <Button type="button" disabled={!canSave} onClick={submit}>
            {t("common:save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
