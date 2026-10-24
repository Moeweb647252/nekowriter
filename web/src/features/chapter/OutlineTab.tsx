/** The Outline tab: the chapter outline, the reference notes and the author's guidance. */
import { useState } from "react"
import { useTranslation } from "react-i18next"

import { HoldBar } from "@/features/chapter/GhostText"
import { useWriterState } from "@/features/chapter/useWriterState"
import { usePatchChapter } from "@/api/hooks/chapters"
import { useResumeSession } from "@/api/hooks/sessions"
import { useErrorToast } from "@/components/common/ErrorToast"
import { Button } from "@/components/ui/button"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import type { ChapterDto } from "@/api/gen/ChapterDto"

export type OutlineTabProps = {
  chapter: ChapterDto
  unspaced: boolean
  bookId: number
}

/** Editable Markdown fields; saving is explicit so a held outline can be approved first (M4). */
export function OutlineTab({ chapter, bookId }: OutlineTabProps) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const resume = useResumeSession(bookId)
  const patchChapter = usePatchChapter(chapter.id, bookId)
  const writer = useWriterState(chapter, undefined)
  const [outline, setOutline] = useState(chapter.outline)
  const [notes, setNotes] = useState(chapter.reference_notes)
  const dirty = outline !== chapter.outline || notes !== chapter.reference_notes
  const saving = patchChapter.isPending

  // A held outline is approvable from here: the outline and notes stay editable while it is held
  // (spec/11 §5.2), and unsaved edits are saved **before** the resume call so the agent reads them
  // in the same transaction that applies the held call. The save is awaited for that reason: firing
  // it and resuming at once would race the agent's kickoff read and could hand it the old outline.
  const heldOutline = writer.heldOutline
  const continueHeld = async (feedback?: string) => {
    if (writer.session === undefined) {
      return
    }
    if (dirty) {
      try {
        await patchChapter.mutateAsync({ outline, reference_notes: notes })
      } catch (error) {
        // Do not resume on a failed save: the agent would act on the text the author replaced.
        showError(error)
        return
      }
    }
    resume.mutate({ sessionId: writer.session.id, feedback }, { onError: showError })
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
          onContinue={() => void continueHeld()}
          onFeedback={(text) => void continueHeld(text)}
        />
      ) : null}
      <div className="flex items-center gap-2">
        <Button
          size="sm"
          disabled={!dirty || saving}
          onClick={() => patchChapter.mutate({ outline, reference_notes: notes })}
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
