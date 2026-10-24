/** The Outline tab: the chapter outline, the reference notes and the author's guidance. */
import { useState } from "react"
import { useTranslation } from "react-i18next"

import { HoldBar } from "@/features/chapter/GhostText"
import { useWriterState } from "@/features/chapter/useWriterState"
import { useResumeSession } from "@/api/hooks/sessions"
import { useErrorToast } from "@/components/common/ErrorToast"
import { Button } from "@/components/ui/button"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import type { ChapterDto } from "@/api/gen/ChapterDto"
import type { ChapterPatch } from "@/api/gen/ChapterPatch"

export type OutlineTabProps = {
  chapter: ChapterDto
  unspaced: boolean
  saving: boolean
  onSave: (patch: ChapterPatch) => void
  bookId: number
}

/** Editable Markdown fields; saving is explicit so a held outline can be approved first (M4). */
export function OutlineTab({ chapter, saving, onSave, bookId }: OutlineTabProps) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const resume = useResumeSession(bookId)
  const writer = useWriterState(chapter, undefined)
  const [outline, setOutline] = useState(chapter.outline)
  const [notes, setNotes] = useState(chapter.reference_notes)
  const dirty = outline !== chapter.outline || notes !== chapter.reference_notes

  // A held outline is approvable from here: the outline and notes stay editable while it is held
  // (spec/11 §5.2), and unsaved edits are saved before the resume call so the agent sees them.
  const heldOutline = writer.heldOutline
  const saveThen = (action: () => void) => {
    if (!dirty) {
      action()
      return
    }
    onSave({ outline, reference_notes: notes })
    action()
  }
  const continueHeld = (feedback?: string) => {
    if (writer.session === undefined) {
      return
    }
    resume.mutate(
      { sessionId: writer.session.id, feedback },
      { onError: showError },
    )
  }

  return (
    <div className="mx-auto max-w-[42em] space-y-5">
      <div className="space-y-1.5">
        <Label htmlFor="chapter-outline">{t("book:outline")}</Label>
        <Textarea
          id="chapter-outline"
          value={outline}
          rows={14}
          className="font-mono text-xs leading-relaxed"
          onChange={(event) => setOutline(event.target.value)}
        />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="chapter-notes">{t("chapter:reference_notes")}</Label>
        <Textarea
          id="chapter-notes"
          value={notes}
          rows={6}
          className="font-mono text-xs leading-relaxed"
          onChange={(event) => setNotes(event.target.value)}
        />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="chapter-guidance">{t("chapter:guidance")}</Label>
        <Textarea
          id="chapter-guidance"
          value={chapter.guidance}
          rows={3}
          disabled
          className="font-mono text-xs"
        />
      </div>
      {heldOutline ? (
        <HoldBar
          outline
          pids={[]}
          busy={resume.isPending || saving}
          onContinue={() => saveThen(() => continueHeld())}
          onFeedback={(text) => saveThen(() => continueHeld(text))}
        />
      ) : null}
      <div className="flex items-center gap-2">
        <Button
          size="sm"
          disabled={!dirty || saving}
          onClick={() => onSave({ outline, reference_notes: notes })}
        >
          {saving ? t("common:saving") : t("common:save")}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          disabled={!dirty}
          onClick={() => {
            setOutline(chapter.outline)
            setNotes(chapter.reference_notes)
          }}
        >
          {t("common:reset")}
        </Button>
      </div>
    </div>
  )
}
