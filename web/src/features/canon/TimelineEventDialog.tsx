/** Edit one timeline event without touching its position (PROJECT §8.8, spec/11 §5.5, D-161). */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Plus, Star, X } from "lucide-react"

import type { TimelineEventDto } from "@/api/gen/TimelineEventDto"
import type { TimelineEventPatch } from "@/api/gen/TimelineEventPatch"
import { useEntities, usePatchTimelineEvent } from "@/api/hooks/useCanon"
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
import { chapterRefLabel, type ChapterIndex } from "@/features/canon/chapterIndex"
import { EntityPicker } from "@/features/canon/EntityPicker"
import { GoalsReferencedBy } from "@/features/canon/GoalsReferencedBy"

export type TimelineEventDialogProps = {
  bookId: number
  event: TimelineEventDto
  chapterIndex: ChapterIndex
  open: boolean
  onOpenChange: (open: boolean) => void
}

/** Order-insensitive entity-list comparison: selection order is not a change. */
function sameIds(left: number[], right: number[]): boolean {
  if (left.length !== right.length) {
    return false
  }
  const sorted = [...right].sort((a, b) => a - b)
  return [...left].sort((a, b) => a - b).every((id, index) => id === sorted[index])
}

export function TimelineEventDialog({
  bookId,
  event,
  chapterIndex,
  open,
  onOpenChange,
}: TimelineEventDialogProps) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const patchEvent = usePatchTimelineEvent(bookId)
  const entities = useEntities(bookId)
  const entityName = (id: number) =>
    entities.data?.find((entity) => entity.id === id)?.name ?? `#${id}`

  const [title, setTitle] = useState(event.title)
  const [description, setDescription] = useState(event.description)
  const [storyTime, setStoryTime] = useState(event.story_time)
  const [chapter, setChapter] = useState(
    event.chapter_id === null ? "" : String(event.chapter_id),
  )
  const [importance, setImportance] = useState(event.importance)
  const [entityIds, setEntityIds] = useState<number[]>(event.entity_ids)
  const [entityPick, setEntityPick] = useState<number | undefined>(undefined)
  const [note, setNote] = useState("")

  const chapterOptions = Array.from(chapterIndex.values()).sort((left, right) => {
    if (left.trackKind !== right.trackKind) {
      return left.trackKind === "main" ? -1 : 1
    }
    if (left.trackId !== right.trackId) {
      return left.trackId - right.trackId
    }
    return left.number - right.number
  })

  const canSave = title.trim() !== "" && !patchEvent.isPending

  const addEntity = () => {
    if (entityPick === undefined || entityIds.includes(entityPick)) {
      return
    }
    setEntityIds((current) => [...current, entityPick])
    setEntityPick(undefined)
  }

  const submit = () => {
    if (!canSave) {
      return
    }
    const next: TimelineEventPatch = {}
    if (title.trim() !== event.title) {
      next.title = title.trim()
    }
    if (description !== event.description) {
      next.description = description
    }
    if (storyTime !== event.story_time) {
      next.story_time = storyTime
    }
    if (chapter !== (event.chapter_id === null ? "" : String(event.chapter_id))) {
      next.chapter_id = chapter === "" ? null : Number(chapter)
    }
    if (!sameIds(entityIds, event.entity_ids)) {
      next.entity_ids = entityIds
    }
    if (importance !== event.importance) {
      next.importance = importance
    }
    if (note.trim() !== "") {
      next.note = note.trim()
    }
    // Position is never part of this form: drag and drop / Move own after_id and before_id.
    if (Object.keys(next).length === 0) {
      onOpenChange(false)
      return
    }
    patchEvent.mutate(
      { id: event.id, patch: next },
      {
        onSuccess: () => onOpenChange(false),
        onError: showError,
      },
    )
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{t("canon:timeline_event_dialog.edit_title")}</DialogTitle>
          <DialogDescription>
            {t("canon:timeline_event_dialog.description")}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-3">
          <div className="space-y-1.5">
            <Label htmlFor="timeline-event-title">
              {t("canon:timeline_event_dialog.title_field")}
            </Label>
            <Input
              id="timeline-event-title"
              value={title}
              onChange={(event) => setTitle(event.target.value)}
            />
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="timeline-event-description">
              {t("canon:timeline_event_dialog.description_field")}
            </Label>
            <Textarea
              id="timeline-event-description"
              rows={2}
              value={description}
              onChange={(event) => setDescription(event.target.value)}
            />
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="timeline-event-story-time">
              {t("canon:timeline_event_dialog.story_time")}
            </Label>
            <Input
              id="timeline-event-story-time"
              value={storyTime}
              onChange={(event) => setStoryTime(event.target.value)}
            />
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="timeline-event-chapter">
              {t("canon:timeline_event_dialog.chapter")}
            </Label>
            <select
              id="timeline-event-chapter"
              value={chapter}
              onChange={(event) => setChapter(event.target.value)}
              className="h-8 w-full rounded-md border bg-background px-2 text-sm"
            >
              <option value="">{t("canon:timeline_event_dialog.no_chapter")}</option>
              {chapterOptions.map((entry) => (
                <option key={entry.id} value={entry.id}>
                  {chapterRefLabel(t, chapterIndex, entry.id)}
                </option>
              ))}
            </select>
          </div>

          <div className="space-y-1.5">
            <Label>{t("canon:timeline_event_dialog.importance")}</Label>
            <div
              role="group"
              aria-label={t("canon:timeline_event_dialog.importance")}
              className="flex gap-1"
            >
              {[1, 2, 3, 4, 5].map((value) => (
                <button
                  key={value}
                  type="button"
                  aria-pressed={importance === value}
                  aria-label={t("canon:importance", { value })}
                  onClick={() => setImportance(value)}
                >
                  <Star
                    className={
                      value <= importance
                        ? "size-4 fill-current text-primary"
                        : "size-4 text-muted-foreground/40"
                    }
                    aria-hidden="true"
                  />
                </button>
              ))}
            </div>
          </div>

          <div className="space-y-1.5">
            <Label>{t("canon:timeline_event_dialog.entities")}</Label>
            <div className="flex items-center gap-1">
              <EntityPicker
                bookId={bookId}
                value={entityPick}
                onChange={setEntityPick}
                label={t("canon:timeline_event_dialog.entities")}
              />
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={entityPick === undefined}
                onClick={addEntity}
              >
                <Plus className="size-3" aria-hidden="true" />
                {t("canon:timeline_event_dialog.add_entity")}
              </Button>
            </div>
            {entityIds.length === 0 ? null : (
              <div className="flex flex-wrap gap-1">
                {entityIds.map((id) => (
                  <span
                    key={id}
                    className="inline-flex items-center gap-1 rounded-full bg-muted px-2 py-0.5 text-xs"
                  >
                    {entityName(id)}
                    <button
                      type="button"
                      aria-label={t("canon:timeline_event_dialog.remove_entity", {
                        name: entityName(id),
                      })}
                      onClick={() =>
                        setEntityIds((current) =>
                          current.filter((item) => item !== id),
                        )
                      }
                    >
                      <X className="size-3" aria-hidden="true" />
                    </button>
                  </span>
                ))}
              </div>
            )}
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="timeline-event-note">
              {t("canon:timeline_event_dialog.change_note")}
            </Label>
            <Textarea
              id="timeline-event-note"
              rows={2}
              value={note}
              onChange={(event) => setNote(event.target.value)}
            />
          </div>

          <GoalsReferencedBy bookId={bookId} eventId={event.id} />
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
