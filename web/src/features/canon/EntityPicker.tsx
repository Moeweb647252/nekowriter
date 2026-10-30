/** Pickers over existing canon rows; ids are never typed (spec/11 §5.5). */
import { useTranslation } from "react-i18next"

import type { EntityKind } from "@/api/gen/EntityKind"
import type { ForeshadowingDto } from "@/api/gen/ForeshadowingDto"
import type { TimelineEventDto } from "@/api/gen/TimelineEventDto"
import { useEntities, useForeshadowing, useTimeline } from "@/api/hooks/useCanon"

type PickerProps = {
  bookId: number
  value?: number | null
  onChange: (value: number | undefined) => void
  label: string
  disabled?: boolean
  kind?: EntityKind
  trackId?: number
}

/** A select over the book's canon entities. */
export function EntityPicker({
  bookId,
  value,
  onChange,
  label,
  disabled = false,
  kind,
  trackId,
}: PickerProps) {
  const { t } = useTranslation()
  const entities = useEntities(bookId, { kind, trackId })
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
      {(entities.data ?? []).map((entity) => (
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
}: Omit<PickerProps, "kind" | "trackId">) {
  const { t } = useTranslation()
  const events = useTimeline(bookId)
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
      {(events.data ?? []).map((event: TimelineEventDto) => (
        <option key={event.id} value={event.id}>
          {event.title}
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
}: Omit<PickerProps, "kind" | "trackId">) {
  const { t } = useTranslation()
  const hooks = useForeshadowing(bookId)
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
