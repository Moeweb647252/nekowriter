/** The change log: one row per change with a before/after diff and Revert (PROJECT §8.8,
 * spec/06 §3).
 *
 * The labels are derived in the UI from `actor`, `op`, `target_type` and `reverts_id`: the server
 * sends data only, because `note` holds written text (D-032, D-054). */
import { useTranslation } from "react-i18next"
import { RotateCcw } from "lucide-react"

import type { ChangeDto } from "@/api/gen/ChangeDto"
import { useRevertChange } from "@/api/hooks/useCanon"
import { useErrorToast } from "@/components/common/ErrorToast"
import { DiffView } from "@/components/common/DiffView"
import { EmptyState } from "@/components/common/EmptyState"
import { Button } from "@/components/ui/button"
import { formatDateTime } from "@/lib/format"

export type ChangeListProps = {
  bookId: number
  changes: ChangeDto[]
  /** Compact mode (the entity pane) hides the actor column. */
  compact?: boolean
}

/** One change log list. */
export function ChangeList({ bookId, changes, compact = false }: ChangeListProps) {
  const { t, i18n } = useTranslation()
  const showError = useErrorToast()
  const revert = useRevertChange(bookId)

  if (changes.length === 0) {
    return <EmptyState title={t("book:no_changes")} />
  }

  return (
    <ul className="space-y-2">
      {changes.map((change) => (
        <li key={change.id} className="rounded-md border p-2 text-sm">
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-medium">{changeLabel(t, change)}</span>
            <span className="text-xs text-muted-foreground">
              {t(`book:change_target.${change.target_type}`)} #{change.target_id}
            </span>
            {compact ? null : (
              <span className="text-xs text-muted-foreground">
                {t(`canon:change_actor.${change.actor}`)}
              </span>
            )}
            <span className="ml-auto text-xs text-muted-foreground">
              {formatDateTime(change.created_at, i18n.language)}
            </span>
            {change.reverts_id === null ? null : (
              <span className="rounded bg-muted px-1 text-[0.65rem] text-muted-foreground">
                {t("canon:revert_badge")}
              </span>
            )}
            {change.reverted_by_id !== null ? (
              <span className="rounded bg-muted px-1 text-[0.65rem] text-muted-foreground">
                {t("canon:reverted_badge")}
              </span>
            ) : (
              <Button
                size="sm"
                variant="ghost"
                disabled={revert.isPending}
                onClick={() =>
                  revert.mutate(change.id, {
                    onError: (error) => showError(error),
                  })
                }
              >
                <RotateCcw className="size-3" aria-hidden="true" />
                {t("canon:revert")}
              </Button>
            )}
          </div>
          {change.note === "" ? null : (
            <p className="mt-1 text-xs text-muted-foreground">{change.note}</p>
          )}
          <SnapshotDiff change={change} />
        </li>
      ))}
    </ul>
  )
}

/** A localized label for a change (D-032: the server never writes UI wording). */
function changeLabel(
  t: (key: string, options?: Record<string, unknown>) => string,
  change: ChangeDto,
): string {
  if (change.reverts_id !== null) {
    return t("canon:op.revert")
  }
  return t(`canon:op.${change.op}`)
}

/** The before/after diff of a change's snapshots (spec/02 §4). */
function SnapshotDiff({ change }: { change: ChangeDto }) {
  const { t, i18n } = useTranslation()
  const before = change.before === null ? "" : snapshotText(change.before)
  const after = change.after === null ? "" : snapshotText(change.after)
  if (before === "" && after === "") {
    return null
  }
  const unspaced = i18n.language.startsWith("zh") || i18n.language.startsWith("ja")
  return (
    <details className="mt-1">
      <summary className="cursor-pointer text-xs text-muted-foreground">
        {t("canon:show_diff")}
      </summary>
      <div className="mt-1 space-y-1 rounded bg-muted/40 p-2">
        {before === "" ? null : (
          <div>
            <p className="text-[0.65rem] text-muted-foreground">{t("canon:diff_before")}</p>
            <DiffView before={before} after="" unspaced={unspaced} />
          </div>
        )}
        {after === "" ? null : (
          <div>
            <p className="text-[0.65rem] text-muted-foreground">{t("canon:diff_after")}</p>
            <DiffView before="" after={after} unspaced={unspaced} />
          </div>
        )}
      </div>
    </details>
  )
}

/** One snapshot as display text: the typed fields, one per line (spec/02 §4). */
export function snapshotText(snapshot: Record<string, unknown>): string {
  const lines: string[] = []
  for (const [key, value] of Object.entries(snapshot)) {
    if (key === "type" || key === "track_id") {
      continue
    }
    if (value === null || value === undefined || value === "") {
      continue
    }
    if (Array.isArray(value)) {
      if (value.length === 0) {
        continue
      }
      lines.push(`${key}: ${value.map((item) => formatValue(item)).join(", ")}`)
      continue
    }
    if (typeof value === "object") {
      const entries = Object.entries(value as Record<string, unknown>)
      if (entries.length === 0) {
        continue
      }
      lines.push(
        `${key}: ${entries.map(([entryKey, entryValue]) => `${entryKey}=${formatValue(entryValue)}`).join("; ")}`,
      )
      continue
    }
    lines.push(`${key}: ${formatValue(value)}`)
  }
  return lines.join("\n")
}

function formatValue(value: unknown): string {
  if (typeof value === "string") {
    return value
  }
  if (typeof value === "boolean") {
    return String(value)
  }
  if (typeof value === "number") {
    return String(value)
  }
  if (value !== null && typeof value === "object") {
    return JSON.stringify(value)
  }
  return ""
}
