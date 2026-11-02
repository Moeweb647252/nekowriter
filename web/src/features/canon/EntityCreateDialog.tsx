/** Create a canon entity by hand (PROJECT §8.8, spec/11 §5.5, D-161). */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { useNavigate } from "react-router"
import { Plus, Star, X } from "lucide-react"

import type { EntityInput } from "@/api/gen/EntityInput"
import type { EntityKind } from "@/api/gen/EntityKind"
import type { EntityStatus } from "@/api/gen/EntityStatus"
import { errorDetails } from "@/api/hooks/errors"
import { useCreateEntity } from "@/api/hooks/useCanon"
import { useTracks } from "@/api/hooks/tracks"
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
import { ENTITY_KINDS, ENTITY_STATUSES } from "@/features/canon/entityOptions"
import { isComposing } from "@/lib/hotkeys"

export type EntityCreateDialogProps = {
  bookId: number
  open: boolean
  onOpenChange: (open: boolean) => void
}

/** The `details.fields` payload of a validation refusal, when present (spec/10 §4). */
function fieldNamesOf(error: unknown): string[] {
  const fields = errorDetails(error)?.fields
  if (typeof fields !== "object" || fields === null) {
    return []
  }
  return Object.keys(fields)
}

export function EntityCreateDialog({ bookId, open, onOpenChange }: EntityCreateDialogProps) {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const showError = useErrorToast()
  const create = useCreateEntity(bookId)
  const tracks = useTracks(bookId)
  // Only an isolated side track has private rows; a canon-mode side track's rows are shared
  // (spec/06 §2), so it must not be offered as a storyline here.
  const isolatedTracks = (tracks.data ?? []).filter(
    (track) => track.kind === "side" && track.canon_mode === "isolated",
  )

  const [kind, setKind] = useState<EntityKind | "">("")
  const [name, setName] = useState("")
  const [aliases, setAliases] = useState<string[]>([])
  const [aliasDraft, setAliasDraft] = useState("")
  const [summary, setSummary] = useState("")
  const [importance, setImportance] = useState(3)
  const [status, setStatus] = useState<EntityStatus>("active")
  const [track, setTrack] = useState("")
  const [note, setNote] = useState("")
  const [fieldErrors, setFieldErrors] = useState<string[]>([])

  const addAlias = () => {
    const value = aliasDraft.trim()
    if (value === "" || aliases.includes(value)) {
      return
    }
    setAliases((current) => [...current, value])
    setAliasDraft("")
  }

  const canSubmit = kind !== "" && name.trim() !== "" && !create.isPending

  const submit = () => {
    if (!canSubmit) {
      return
    }
    setFieldErrors([])
    const input: EntityInput = {
      kind,
      name: name.trim(),
      aliases,
      summary: summary.trim(),
      description: "",
      state: "",
      attributes: {},
      importance,
      status,
      is_protagonist: false,
      // `track_id` is present only when the author chose one isolated track; shared canon is its
      // absence (spec/06 §2).
      ...(track === "" ? {} : { track_id: Number(track) }),
      ...(note.trim() === "" ? {} : { note: note.trim() }),
    }
    create.mutate(input, {
      onSuccess: (entity) => {
        onOpenChange(false)
        void navigate(`/books/${bookId}/canon/entities/${entity.id}`)
      },
      onError: (error) => {
        const fields = fieldNamesOf(error).filter(
          (field) => field === "kind" || field === "name",
        )
        if (fields.length > 0) {
          setFieldErrors(fields)
          return
        }
        showError(error)
      },
    })
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{t("canon:entity_create.title")}</DialogTitle>
          <DialogDescription>{t("canon:entity_create.description")}</DialogDescription>
        </DialogHeader>

        <div className="space-y-3">
          <div className="space-y-1.5">
            <Label htmlFor="entity-kind">{t("canon:entity_create.kind")}</Label>
            <select
              id="entity-kind"
              value={kind}
              onChange={(event) => {
                setKind(event.target.value as EntityKind | "")
                setFieldErrors((current) => current.filter((field) => field !== "kind"))
              }}
              className="h-8 w-full rounded-md border bg-background px-2 text-sm"
            >
              <option value="">{t("canon:entity_create.kind_placeholder")}</option>
              {ENTITY_KINDS.map((value) => (
                <option key={value} value={value}>
                  {t(`canon:kind.${value}`)}
                </option>
              ))}
            </select>
            {fieldErrors.includes("kind") ? (
              <p className="text-xs text-destructive">
                {t("errors:validation.invalid_fields")}
              </p>
            ) : null}
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="entity-name">{t("canon:entity_create.name")}</Label>
            <Input
              id="entity-name"
              value={name}
              onChange={(event) => {
                setName(event.target.value)
                setFieldErrors((current) => current.filter((field) => field !== "name"))
              }}
            />
            {fieldErrors.includes("name") ? (
              <p className="text-xs text-destructive">
                {t("errors:validation.invalid_fields")}
              </p>
            ) : null}
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="entity-alias">{t("canon:entity_create.aliases")}</Label>
            <div className="flex items-center gap-1">
              <Input
                id="entity-alias"
                value={aliasDraft}
                onChange={(event) => setAliasDraft(event.target.value)}
                onKeyDown={(event) => {
                  if (isComposing(event)) {
                    return
                  }
                  if (event.key === "Enter") {
                    event.preventDefault()
                    addAlias()
                  }
                }}
              />
              <Button type="button" size="sm" variant="outline" onClick={addAlias}>
                <Plus className="size-3" aria-hidden="true" />
                {t("canon:entity_create.add_alias")}
              </Button>
            </div>
            {aliases.length === 0 ? null : (
              <div className="flex flex-wrap gap-1">
                {aliases.map((alias) => (
                  <span
                    key={alias}
                    className="inline-flex items-center gap-1 rounded-full bg-muted px-2 py-0.5 text-xs"
                  >
                    {alias}
                    <button
                      type="button"
                      aria-label={t("canon:entity_create.remove_alias", { alias })}
                      onClick={() =>
                        setAliases((current) => current.filter((item) => item !== alias))
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
            <Label htmlFor="entity-summary">{t("canon:entity_create.summary")}</Label>
            <Textarea
              id="entity-summary"
              rows={2}
              value={summary}
              onChange={(event) => setSummary(event.target.value)}
            />
          </div>

          <div className="space-y-1.5">
            <Label>{t("canon:entity_create.importance")}</Label>
            <div
              role="group"
              aria-label={t("canon:entity_create.importance")}
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
            <Label htmlFor="entity-status">{t("canon:entity_create.status")}</Label>
            <select
              id="entity-status"
              value={status}
              onChange={(event) => setStatus(event.target.value as EntityStatus)}
              className="h-8 w-full rounded-md border bg-background px-2 text-sm"
            >
              {ENTITY_STATUSES.map((value) => (
                <option key={value} value={value}>
                  {t(`canon:status.${value}`)}
                </option>
              ))}
            </select>
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="entity-storyline">{t("canon:entity_create.storyline")}</Label>
            <select
              id="entity-storyline"
              value={track}
              onChange={(event) => setTrack(event.target.value)}
              className="h-8 w-full rounded-md border bg-background px-2 text-sm"
            >
              <option value="">{t("canon:track_shared")}</option>
              {isolatedTracks.map((option) => (
                <option key={option.id} value={option.id}>
                  {option.title}
                </option>
              ))}
            </select>
            <p className="text-xs text-muted-foreground">
              {t("canon:entity_create.storyline_hint")}
            </p>
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="entity-note">{t("canon:entity_create.change_note")}</Label>
            <Textarea
              id="entity-note"
              rows={2}
              value={note}
              onChange={(event) => setNote(event.target.value)}
            />
          </div>
        </div>

        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            {t("common:cancel")}
          </Button>
          <Button type="button" disabled={!canSubmit} onClick={submit}>
            {t("common:save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
