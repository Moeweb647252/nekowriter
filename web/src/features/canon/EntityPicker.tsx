/** Pickers over existing canon rows; ids are never typed (spec/11 §5.5). */
import { useTranslation } from "react-i18next"

import type { EntityKind } from "@/api/gen/EntityKind"
import type { EntityListItemDto } from "@/api/gen/EntityListItemDto"
import type { ForeshadowingDto } from "@/api/gen/ForeshadowingDto"
import type { TimelineEventDto } from "@/api/gen/TimelineEventDto"
import type { TimelineStatus } from "@/api/gen/TimelineStatus"
import { useEntities, useForeshadowing, useTimeline } from "@/api/hooks/useCanon"
import { cn } from "@/lib/utils"

type PickerProps = {
  bookId: number
  value?: number | null
  onChange: (value: number | undefined) => void
  label: string
  disabled?: boolean
  kind?: EntityKind
  trackId?: number
  /** Omit one entity from the choices (the merge target). */
  excludeId?: number
  /**
   * Overrides the query result. The merge dialog passes the cross-canon union of the shared list
   * and every isolated track, which one `useEntities` query cannot express (spec/06 §9).
   */
  entities?: EntityListItemDto[]
  /**
   * `select` is the compact picker of the goal dialog. `list` renders the same choices as a
   * radio list with the kind and the shared/private track badge, for the merge dialog.
   */
  variant?: "select" | "list"
}

/** A select (or radio list) over the book's canon entities. */
export function EntityPicker({
  bookId,
  value,
  onChange,
  label,
  disabled = false,
  kind,
  trackId,
  excludeId,
  entities,
  variant = "select",
}: PickerProps) {
  const { t } = useTranslation()
  const fetched = useEntities(bookId, { kind, trackId })
  const choices = (entities ?? fetched.data ?? []).filter(
    (entity) => entity.id !== excludeId,
  )
  if (variant === "list") {
    return (
      <div
        role="radiogroup"
        aria-label={label}
        className="max-h-56 space-y-1 overflow-y-auto rounded-md border p-1"
      >
        {choices.length === 0 ? (
          <p className="p-2 text-xs text-muted-foreground">{t("canon:picker_empty")}</p>
        ) : null}
        {choices.map((entity) => (
          <button
            key={entity.id}
            type="button"
            role="radio"
            aria-checked={value === entity.id}
            disabled={disabled}
            onClick={() => onChange(entity.id)}
            className={cn(
              "flex w-full items-center gap-2 rounded px-2 py-1 text-left text-sm",
              value === entity.id ? "bg-accent" : "hover:bg-accent/50",
            )}
          >
            <span className="min-w-0 flex-1 truncate">{entity.name}</span>
            <span className="text-xs text-muted-foreground">
              {t(`canon:kind.${entity.kind}`)}
            </span>
            <span className="rounded bg-muted px-1 text-[0.65rem] text-muted-foreground">
              {entity.track_id === null
                ? t("canon:track_shared")
                : t("canon:private_badge")}
            </span>
          </button>
        ))}
      </div>
    )
  }
  return (
    <select
      value={value ?? ""}
      onChange={(event) =>
        onChange(event.target.value === "" ? undefined : Number(event.target.value))
      }
      className="h-8 min-w-0 rounded-md border bg-background px-2 text-xs"
      aria-label={label}
      disabled={disabled}
    >
      <option value="">{t("canon:picker_none")}</option>
      {choices.map((entity) => (
        <option key={entity.id} value={entity.id}>
          {entity.name}
        </option>
      ))}
    </select>
  )
}

/** A select over the book's timeline events. */
export function EventPicker({
  bookId,
  value,
  onChange,
  label,
  disabled = false,
  trackId,
  includeId,
}: Omit<PickerProps, "kind"> & {
  /** Keep a currently stored choice selectable even when it is abandoned. */
  includeId?: number | null
}) {
  const { t } = useTranslation()
  const events = useTimeline(bookId, trackId)
  const all = (events.data ?? []) as TimelineEventDto[]
  // Planned entries first: the goal condition and the foreshadowing anchor mostly name the plan,
  // and an abandoned entry is never a legal new target (D-170 T10/T13b).
  const order: TimelineStatus[] = ["planned", "occurred", "abandoned"]
  const choices = all
    .filter((event) => event.kind !== "root")
    .filter((event) => event.status !== "abandoned" || event.id === includeId)
    .sort(
      (left, right) =>
        order.indexOf(left.status) - order.indexOf(right.status) ||
        left.position - right.position ||
        left.id - right.id,
    )
  const optionLabel = (event: TimelineEventDto): string => {
    if (event.status === "planned") {
      return t("canon:event_option_planned", { title: event.title })
    }
    if (event.status === "abandoned") {
      return t("canon:event_option_abandoned", { title: event.title })
    }
    return event.title
  }
  return (
    <select
      value={value ?? ""}
      onChange={(event) =>
        onChange(event.target.value === "" ? undefined : Number(event.target.value))
      }
      className="h-8 min-w-0 rounded-md border bg-background px-2 text-xs"
      aria-label={label}
      disabled={disabled}
    >
      <option value="">{t("canon:picker_none")}</option>
      {choices.map((event) => (
        <option key={event.id} value={event.id} disabled={event.status === "abandoned"}>
          {optionLabel(event)}
        </option>
      ))}
    </select>
  )
}

/** A select over the book's foreshadowing rows. */
export function ForeshadowingPicker({
  bookId,
  value,
  onChange,
  label,
  disabled = false,
  trackId,
}: Omit<PickerProps, "kind">) {
  const { t } = useTranslation()
  const hooks = useForeshadowing(bookId, undefined, trackId)
  return (
    <select
      value={value ?? ""}
      onChange={(event) =>
        onChange(event.target.value === "" ? undefined : Number(event.target.value))
      }
      className="h-8 min-w-0 rounded-md border bg-background px-2 text-xs"
      aria-label={label}
      disabled={disabled}
    >
      <option value="">{t("canon:picker_none")}</option>
      {(hooks.data ?? []).map((hook: ForeshadowingDto) => (
        <option key={hook.id} value={hook.id}>
          {hook.title}
        </option>
      ))}
    </select>
  )
}
