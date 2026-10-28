/**
 * The Outline tab (PROJECT §8.5, spec/11 §5.2, D-146).
 *
 * The tab renders the outline's derived state: missing (write it), in progress (the planner is
 * working), awaiting approval (the held proposal and its two decisions) and accepted (editable
 * fields plus Redo the outline). A held `submit_outline` in a legacy `chapter_write` session
 * keeps the pre-split path: save the fields, then resume the call.
 */
import { useState } from "react"
import { useTranslation } from "react-i18next"

import { HoldBar } from "@/features/chapter/GhostText"
import { OutlineDialog } from "@/features/chapter/OutlineDialog"
import { useWriterState } from "@/features/chapter/useWriterState"
import { usePatchChapter } from "@/api/hooks/chapters"
import { useResumeSession } from "@/api/hooks/sessions"
import { useErrorToast } from "@/components/common/ErrorToast"
import { EmptyState } from "@/components/common/EmptyState"
import { Button } from "@/components/ui/button"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import type { BookSettings } from "@/api/gen/BookSettings"
import type { ChapterDto } from "@/api/gen/ChapterDto"
import type { SessionDto } from "@/api/gen/SessionDto"
import type { ToolCallDto } from "@/api/gen/ToolCallDto"
import type { OutlineState } from "@/features/chapter/outlineState"

export type OutlineTabProps = {
  chapter: ChapterDto
  unspaced: boolean
  bookId: number
  settings: BookSettings
  state: OutlineState
  session?: SessionDto
  calls?: readonly ToolCallDto[]
  onSessionStarted?: (sessionId: number) => void
}

/** One read-only Markdown field of a held or running outline. */
function ReadOnlyField({ id, label, value }: { id: string; label: string; value: string }) {
  return (
    <div className="space-y-1.5">
      <Label htmlFor={id}>{label}</Label>
      <Textarea
        id={id}
        value={value}
        rows={8}
        readOnly
        className="font-mono text-xs leading-relaxed"
      />
    </div>
  )
}

export function OutlineTab({
  chapter,
  bookId,
  settings,
  state,
  session,
  calls,
  onSessionStarted,
}: OutlineTabProps) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const resume = useResumeSession(bookId)
  const patchChapter = usePatchChapter(chapter.id, bookId)
  const writer = useWriterState(chapter, undefined)
  const [outline, setOutline] = useState(chapter.outline)
  const [notes, setNotes] = useState(chapter.reference_notes)
  const [outlineOpen, setOutlineOpen] = useState(false)
  const dirty = outline !== chapter.outline || notes !== chapter.reference_notes
  const saving = patchChapter.isPending

  // The pre-split path: a held submit_outline in a chapter_write session (legacy branch).
  const legacyHeld = writer.heldOutline && writer.session?.kind === "chapter_write"
  const continueLegacy = async (feedback?: string) => {
    if (writer.session === undefined) {
      return
    }
    if (dirty) {
      try {
        await patchChapter.mutateAsync({ outline, reference_notes: notes })
      } catch (error) {
        showError(error)
        return
      }
    }
    resume.mutate({ sessionId: writer.session.id, feedback }, { onError: showError })
  }

  const heldCall = calls?.find((call) => call.id === session?.held_tool_call_id)
  const proposal = (heldCall?.input ?? {}) as {
    outline?: string
    reference_notes?: string
    title?: string
  }
  const decide = (feedback?: string) => {
    if (session === undefined) {
      return
    }
    resume.mutate({ sessionId: session.id, feedback }, { onError: showError })
  }

  const editable = state === "missing" || state === "accepted" || legacyHeld

  return (
    <div className="mx-auto max-w-[42em] space-y-5">
      {state === "missing" && !legacyHeld ? (
        <div className="space-y-3">
          <EmptyState
            title={t("chapter:outline_tab.empty_title")}
            description={t("chapter:outline_tab.empty_hint")}
          />
          <div className="flex justify-center">
            <Button size="sm" onClick={() => setOutlineOpen(true)}>
              {t("chapter:write_outline")}
            </Button>
          </div>
        </div>
      ) : null}
      {state === "in_progress" && !legacyHeld ? (
        <div className="rounded-md border p-3 text-sm">
          <p className="font-medium">{t("chapter:outline_tab.planner_working")}</p>
          <p className="text-muted-foreground">{t("chapter:outline_tab.planner_working_hint")}</p>
        </div>
      ) : null}

      {state === "awaiting_approval" && !legacyHeld ? (
        <>
          <div className="space-y-1">
            <h3 className="text-sm font-semibold">{t("chapter:outline_tab.proposal_title")}</h3>
            <p className="text-xs text-muted-foreground">
              {t("chapter:outline_tab.proposal_hint")}
            </p>
          </div>
          <ReadOnlyField
            id="outline-proposal-outline"
            label={t("book:outline")}
            value={proposal.outline ?? ""}
          />
          <ReadOnlyField
            id="outline-proposal-notes"
            label={t("chapter:reference_notes")}
            value={proposal.reference_notes ?? ""}
          />
          {proposal.title ? (
            <div className="space-y-1.5">
              <Label htmlFor="outline-proposal-title">{t("chapter:outline_tab.proposed_title")}</Label>
              <p id="outline-proposal-title" className="text-sm">
                {proposal.title}
              </p>
            </div>
          ) : null}
          <HoldBar
            outline
            pids={[]}
            busy={resume.isPending}
            onContinue={() => decide()}
            onFeedback={(text) => decide(text)}
          />
          {chapter.outline.trim() === "" ? null : (
            <p className="border-t pt-3 text-xs text-muted-foreground">
              {t("chapter:outline_tab.current_replaced")}
            </p>
          )}
        </>
      ) : null}

      <div className="space-y-1.5">
        <Label htmlFor="chapter-outline">{t("book:outline")}</Label>
        <Textarea
          id="chapter-outline"
          value={outline}
          rows={14}
          className="font-mono text-xs leading-relaxed"
          readOnly={!editable}
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
          readOnly={!editable}
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

      {legacyHeld ? (
        <HoldBar
          outline
          pids={[]}
          busy={resume.isPending || saving}
          onContinue={() => void continueLegacy()}
          onFeedback={(text) => void continueLegacy(text)}
        />
      ) : null}

      {editable && !legacyHeld ? (
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
          {state === "accepted" && chapter.status !== "official" ? (
            <Button size="sm" variant="outline" onClick={() => setOutlineOpen(true)}>
              {t("chapter:redo_outline")}
            </Button>
          ) : null}
        </div>
      ) : null}

      <OutlineDialog
        chapterId={chapter.id}
        bookId={bookId}
        settings={settings}
        guidance={chapter.guidance}
        open={outlineOpen}
        onOpenChange={setOutlineOpen}
        onStarted={(sessionId) => onSessionStarted?.(sessionId)}
      />
    </div>
  )
}
