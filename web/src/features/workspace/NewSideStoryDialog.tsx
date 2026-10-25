/** The New side story dialog (PROJECT §8.4, F9; spec/08 §6).
 *
 * Collects the title, premise, canon mode, anchor and optional focus character. The defaults are
 * the spec's: `isolated`, and `after_chapter` on the latest official main chapter (`before_start`
 * when none exists). The server applies the same defaults, but showing them lets the author see
 * what the new sessions will be told.
 *
 * The anchor default is *derived* from the chapter list (not copied into state by an effect), so
 * a dialog opened while the list is still loading settles on the right value when it arrives,
 * and an author's explicit choice always wins. */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { useNavigate } from "react-router"

import type { AnchorKind } from "@/api/gen/AnchorKind"
import type { CanonMode } from "@/api/gen/CanonMode"
import { useEntities } from "@/api/hooks/useCanon"
import { useChapters, useCreateTrack, useTracks } from "@/api/hooks/tracks"
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

export type NewSideStoryDialogProps = {
  bookId: number
  open: boolean
  onOpenChange: (open: boolean) => void
}

export function NewSideStoryDialog({
  bookId,
  open,
  onOpenChange,
}: NewSideStoryDialogProps) {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const showError = useErrorToast()
  const createTrack = useCreateTrack(bookId)
  const tracks = useTracks(bookId)
  const main = tracks.data?.find((track) => track.kind === "main")
  const chapters = useChapters(main?.id ?? 0)
  const characters = useEntities(bookId, { kind: "character" })

  const [title, setTitle] = useState("")
  const [synopsis, setSynopsis] = useState("")
  const [canonMode, setCanonMode] = useState<CanonMode>("isolated")
  // `null` = the author has not chosen, so the spec default is derived from the chapter list.
  const [chosenAnchorKind, setChosenAnchorKind] = useState<AnchorKind | null>(null)
  const [chosenAnchorChapterId, setChosenAnchorChapterId] = useState<number | null>(null)
  const [focusEntityId, setFocusEntityId] = useState<number | undefined>(undefined)
  const [error, setError] = useState<string | null>(null)

  const officialChapters = (chapters.data ?? []).filter(
    (chapter) => chapter.status === "official",
  )
  const latestOfficial = officialChapters[officialChapters.length - 1]
  const defaultKind: AnchorKind =
    latestOfficial === undefined ? "before_start" : "after_chapter"
  const anchorKind = chosenAnchorKind ?? defaultKind
  const anchorChapterId =
    anchorKind === "after_chapter"
      ? (chosenAnchorChapterId ?? latestOfficial?.id)
      : undefined
  const anchorReady = anchorKind !== "after_chapter" || anchorChapterId !== undefined

  const reset = () => {
    setTitle("")
    setSynopsis("")
    setCanonMode("isolated")
    setChosenAnchorKind(null)
    setChosenAnchorChapterId(null)
    setFocusEntityId(undefined)
    setError(null)
  }

  const canSubmit =
    title.trim() !== "" && anchorReady && !createTrack.isPending

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
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{t("book:new_side_story")}</DialogTitle>
          <DialogDescription>{t("book:new_side_story_description")}</DialogDescription>
        </DialogHeader>
        <div className="space-y-3">
          <div className="space-y-1.5">
            <Label htmlFor="side-title">{t("book:side_title")}</Label>
            <Input
              id="side-title"
              value={title}
              autoFocus
              onChange={(event) => setTitle(event.target.value)}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="side-synopsis">{t("book:side_synopsis")}</Label>
            <Textarea
              id="side-synopsis"
              rows={3}
              value={synopsis}
              placeholder={t("book:side_synopsis_placeholder")}
              onChange={(event) => setSynopsis(event.target.value)}
            />
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-1.5">
              <Label htmlFor="side-mode">{t("book:side_canon_mode")}</Label>
              <Select
                value={canonMode}
                onValueChange={(value) => setCanonMode(value as CanonMode)}
              >
                <SelectTrigger id="side-mode">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="isolated">
                    {t("book:canon_mode.isolated")}
                  </SelectItem>
                  <SelectItem value="canon">{t("book:canon_mode.canon")}</SelectItem>
                </SelectContent>
              </Select>
              <p className="text-[0.7rem] text-muted-foreground">
                {t("book:canon_mode_hint")}
              </p>
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="side-anchor">{t("book:side_anchor")}</Label>
              <Select
                value={anchorKind}
                onValueChange={(value) => {
                  const kind = value as AnchorKind
                  setChosenAnchorKind(kind)
                  setChosenAnchorChapterId(
                    kind === "after_chapter"
                      ? (latestOfficial?.id ?? (chapters.data ?? [])[0]?.id ?? null)
                      : null,
                  )
                }}
              >
                <SelectTrigger id="side-anchor">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="after_chapter">
                    {t("book:anchor.after_chapter")}
                  </SelectItem>
                  <SelectItem value="before_start">
                    {t("book:anchor.before_start")}
                  </SelectItem>
                  <SelectItem value="after_end">
                    {t("book:anchor.after_end")}
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>
          </div>
          {anchorKind !== "after_chapter" ? null : (
            <div className="space-y-1.5">
              <Label htmlFor="side-anchor-chapter">
                {t("book:side_anchor_chapter")}
              </Label>
              <Select
                value={anchorChapterId === undefined ? "" : String(anchorChapterId)}
                onValueChange={(value) => setChosenAnchorChapterId(Number(value))}
              >
                <SelectTrigger id="side-anchor-chapter">
                  <SelectValue placeholder={t("book:side_anchor_chapter_none")} />
                </SelectTrigger>
                <SelectContent>
                  {(chapters.data ?? []).map((chapter) => (
                    <SelectItem key={chapter.id} value={String(chapter.id)}>
                      {t("book:chapter_option", {
                        number: chapter.number,
                        title: chapter.title,
                      })}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          )}
          <div className="space-y-1.5">
            <Label htmlFor="side-focus">{t("book:side_focus")}</Label>
            <Select
              value={focusEntityId === undefined ? "none" : String(focusEntityId)}
              onValueChange={(value) =>
                setFocusEntityId(value === "none" ? undefined : Number(value))
              }
            >
              <SelectTrigger id="side-focus">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="none">{t("book:side_focus_none")}</SelectItem>
                {(characters.data ?? []).map((entity) => (
                  <SelectItem key={entity.id} value={String(entity.id)}>
                    {entity.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <p className="text-[0.7rem] text-muted-foreground">
              {t("book:side_focus_hint")}
            </p>
          </div>
          {error === null ? null : <p className="text-sm text-status-failed">{error}</p>}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t("common:cancel")}
          </Button>
          <Button
            disabled={!canSubmit}
            onClick={() => {
              setError(null)
              createTrack.mutate(
                {
                  title: title.trim(),
                  synopsis: synopsis.trim(),
                  canon_mode: canonMode,
                  anchor_kind: anchorKind,
                  anchor_chapter_id: anchorChapterId,
                  focus_entity_id: focusEntityId,
                },
                {
                  onSuccess: (track) => {
                    onOpenChange(false)
                    reset()
                    void navigate(`/books/${bookId}/tracks/${track.id}`)
                  },
                  onError: (cause) => {
                    setError(t("common:error_body"))
                    showError(cause)
                  },
                },
              )
            }}
          >
            {createTrack.isPending ? t("common:saving") : t("book:create_side_story")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
