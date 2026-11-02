/** Merge another canon entity into this one (spec/06 §9, spec/11 §5.5, D-161). */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { useNavigate } from "react-router"
import { AlertTriangle } from "lucide-react"
import { toast } from "sonner"

import type { EntityDto } from "@/api/gen/EntityDto"
import { useEntity, useMergeEntity, useMergeSourceEntities } from "@/api/hooks/useCanon"
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
import { EntityPicker } from "@/features/canon/EntityPicker"

export type EntityMergeDialogProps = {
  bookId: number
  /** The survivor A; the entity whose detail page opened this dialog. */
  target: EntityDto
  open: boolean
  onOpenChange: (open: boolean) => void
}

export function EntityMergeDialog({
  bookId,
  target,
  open,
  onOpenChange,
}: EntityMergeDialogProps) {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const showError = useErrorToast()
  const merge = useMergeEntity(bookId)
  // A merge may move a private entity into a shared one, so the sources are the union over the
  // shared list and every isolated track (one canon-scoped list query cannot express that).
  const sources = useMergeSourceEntities(bookId)
  const candidates = (sources.data ?? []).filter(
    (entity) =>
      entity.id !== target.id &&
      // A private target can only absorb shared rows and the private rows of its own canon;
      // another track's private rows would point back at a row that track cannot see.
      (target.track_id === null ||
        entity.track_id === null ||
        entity.track_id === target.track_id),
  )
  const [fromId, setFromId] = useState<number | undefined>(undefined)
  // The chosen row already carries the name; the detail query is only for the appearance count
  // the list DTO lacks, so the confirm never waits on it (18.3-S2).
  const selected = candidates.find((entity) => entity.id === fromId)
  const source = useEntity(fromId)
  const sourceName = selected?.name
  const sourceTrackId =
    selected === undefined ? source.data?.track_id : selected.track_id
  // A shared entity cannot move into an isolated track's private entity (spec/06 §9); the
  // server stays the authority, but the dialog warns before the request.
  const sharedIntoPrivate =
    target.track_id !== null && sourceTrackId === null

  const submit = () => {
    if (fromId === undefined) {
      return
    }
    merge.mutate(
      { id: target.id, fromId },
      {
        onSuccess: (entity) => {
          toast.success(
            t("canon:entity_merge.success", {
              from: sourceName ?? `#${fromId}`,
              to: entity.name,
            }),
          )
          onOpenChange(false)
          // The dialog lives on A, but moving the route explicitly makes a route that showed B
          // land on the survivor.
          void navigate(`/books/${bookId}/canon/entities/${entity.id}`)
        },
        onError: showError,
      },
    )
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{t("canon:entity_merge.title")}</DialogTitle>
          <DialogDescription>
            {t("canon:entity_merge.description", { name: target.name })}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-3">
          <div className="space-y-1.5">
            <p className="text-sm font-medium">
              {t("canon:entity_merge.keep", { name: target.name })}
            </p>
            <EntityPicker
              bookId={bookId}
              value={fromId}
              onChange={setFromId}
              label={t("canon:entity_merge.pick")}
              entities={candidates}
              excludeId={target.id}
              variant="list"
            />
          </div>

          {selected === undefined ? null : (
            <div className="flex flex-wrap items-center gap-2 rounded-md border bg-muted/30 p-2 text-xs">
              <span className="font-medium">{selected.name}</span>
              <span className="text-muted-foreground">
                {t(`canon:kind.${selected.kind}`)}
              </span>
              <span className="rounded bg-muted px-1 text-[0.65rem] text-muted-foreground">
                {selected.track_id === null
                  ? t("canon:track_shared")
                  : t("canon:private_badge")}
              </span>
              {source.data === undefined ? null : (
                <span className="text-muted-foreground">
                  {t("canon:entity_merge.appearances", {
                    count: source.data.appearances.length,
                  })}
                </span>
              )}
            </div>
          )}

          {sharedIntoPrivate ? (
            <p className="flex items-start gap-1 rounded-md border border-destructive/40 bg-destructive/5 p-2 text-xs text-destructive">
              <AlertTriangle className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
              {t("canon:entity_merge.shared_into_private")}
            </p>
          ) : null}

          <section className="space-y-1">
            <h3 className="text-xs font-medium text-muted-foreground">
              {t("canon:entity_merge.what_moves")}
            </h3>
            <ul className="list-disc space-y-0.5 pl-5 text-xs text-muted-foreground">
              <li>{t("canon:entity_merge.moves_relations")}</li>
              <li>{t("canon:entity_merge.moves_appearances")}</li>
              <li>{t("canon:entity_merge.moves_links")}</li>
              <li>{t("canon:entity_merge.moves_conditions")}</li>
              <li>{t("canon:entity_merge.moves_design")}</li>
              <li>{t("canon:entity_merge.moves_focus")}</li>
              <li>{t("canon:entity_merge.moves_protagonist")}</li>
            </ul>
          </section>

          <div className="space-y-1 text-xs text-muted-foreground">
            <p>{t("canon:entity_merge.alias")}</p>
            <p>{t("canon:entity_merge.delete")}</p>
            <p>{t("canon:entity_merge.irreversible")}</p>
            <p>{t("canon:entity_merge.shared_rule")}</p>
          </div>
        </div>

        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            {t("common:cancel")}
          </Button>
          <Button
            type="button"
            variant="destructive"
            disabled={fromId === undefined || merge.isPending}
            onClick={submit}
          >
            {t("canon:entity_merge.confirm", {
              from: sourceName ?? t("canon:entity_merge.this_entity"),
              to: target.name,
            })}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
