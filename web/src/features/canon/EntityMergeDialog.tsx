/** Merge another canon entity into this one (spec/06 §9, spec/11 §5.5, D-161). */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { useNavigate } from "react-router"
import { AlertTriangle } from "lucide-react"
import { toast } from "sonner"

import type { EntityDto } from "@/api/gen/EntityDto"
import { useEntity, useMergeEntity } from "@/api/hooks/useCanon"
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
  const [fromId, setFromId] = useState<number | undefined>(undefined)
  // The detail carries the appearance count the list DTO does not: show it for the chosen B.
  const source = useEntity(fromId)
  const sourceName = source.data?.name
  // A shared entity cannot move into an isolated track's private entity (spec/06 §9); the
  // server stays the authority, but the dialog warns before the request once B is loaded.
  const sharedIntoPrivate =
    target.track_id !== null && source.data !== undefined && source.data.track_id === null

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
              excludeId={target.id}
              variant="list"
            />
          </div>

          {source.data === undefined ? null : (
            <div className="flex flex-wrap items-center gap-2 rounded-md border bg-muted/30 p-2 text-xs">
              <span className="font-medium">{source.data.name}</span>
              <span className="text-muted-foreground">
                {t(`canon:kind.${source.data.kind}`)}
              </span>
              <span className="rounded bg-muted px-1 text-[0.65rem] text-muted-foreground">
                {source.data.track_id === null
                  ? t("canon:track_shared")
                  : t("canon:private_badge")}
              </span>
              <span className="text-muted-foreground">
                {t("canon:entity_merge.appearances", {
                  count: source.data.appearances.length,
                })}
              </span>
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
