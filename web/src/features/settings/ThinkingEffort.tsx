/** The Thinking effort section of the model dialog (spec/04 §5.1, spec/11 §5.1, D-162).
 *
 * Anthropic has a closed, canonical set, so it gets five checkboxes in canonical order. The
 * OpenAI-compatible protocol accepts arbitrary level tokens, so it gets a chip input in entry
 * order. The Demo protocol sends no request at all and shows only a note. A protocol switch removes
 * the levels the new protocol cannot send; the caller names them in the existing "Options removed"
 * notice, so the removal is visible rather than silent.
 */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Plus, TriangleAlert, X } from "lucide-react"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import type { Protocol } from "@/api/gen/Protocol"
import { isComposing } from "@/lib/hotkeys"
import {
  ANTHROPIC_EFFORT_LEVELS,
  MAX_EFFORT_LEVELS,
  addEffortLevel,
  unsendableEffortLevels,
  type EffortAddError,
} from "@/features/settings/effortLevels"

/** The Select value for "no default"; invalid as a level token, so it cannot collide. */
const DEFAULT_EFFORT_NONE = "$none"

export function ThinkingEffortSection({
  protocol,
  levels,
  defaultEffort,
  ignoredEffortLevels,
  onChange,
}: {
  protocol: Protocol | null
  levels: string[]
  defaultEffort: string | null
  ignoredEffortLevels: readonly string[]
  onChange: (levels: string[], defaultEffort: string | null) => void
}) {
  const { t } = useTranslation()
  const [error, setError] = useState<EffortAddError | null>(null)
  const fromServer = ignoredEffortLevels.filter((level) => levels.includes(level))
  const unsendable = [
    ...new Set([...unsendableEffortLevels(levels, protocol), ...fromServer]),
  ]
  const protocolName =
    protocol === null ? "" : t(`settings:protocol.${protocol}`, { defaultValue: protocol })

  const toggleAnthropic = (level: string, checked: boolean) => {
    setError(null)
    const selected = new Set(levels)
    if (checked) {
      selected.add(level)
    } else {
      selected.delete(level)
    }
    const next = ANTHROPIC_EFFORT_LEVELS.filter((candidate) => selected.has(candidate))
    onChange(next, !checked && defaultEffort === level ? null : defaultEffort)
  }
  const add = (raw: string) => {
    const result = addEffortLevel(levels, raw)
    setError(result.error)
    if (result.error === null) {
      onChange(result.levels, defaultEffort)
    }
  }
  const remove = (level: string) => {
    setError(null)
    onChange(
      levels.filter((candidate) => candidate !== level),
      defaultEffort === level ? null : defaultEffort,
    )
  }
  const removeUnsupported = () => {
    const remaining = levels.filter((level) => !unsendable.includes(level))
    onChange(remaining, unsendable.includes(defaultEffort ?? "") ? null : defaultEffort)
  }

  if (protocol === "mock") {
    return (
      <div className="space-y-2" data-testid="thinking-effort">
        <p className="text-xs text-muted-foreground" data-testid="effort-mock-note">
          {t("settings:effort.mock_note")}
        </p>
      </div>
    )
  }

  return (
    <div className="space-y-2" data-testid="thinking-effort">
      <div className="space-y-1">
        <Label>{t("settings:effort.title")}</Label>
        <p className="text-xs text-muted-foreground">{t("settings:effort.hint")}</p>
      </div>

      {protocol === "anthropic" ? (
        <div
          className="space-y-1.5"
          role="group"
          aria-label={t("settings:effort.levels")}
        >
          {ANTHROPIC_EFFORT_LEVELS.map((level) => (
            <label key={level} className="flex items-center gap-2 text-sm">
              <Checkbox
                checked={levels.includes(level)}
                onCheckedChange={(checked) => toggleAnthropic(level, checked === true)}
              />
              {t(`settings:value.effort.${level}`)}
            </label>
          ))}
        </div>
      ) : (
        <OpenAiEffortChips levels={levels} onAdd={add} onRemove={remove} />
      )}

      <div className="flex items-center justify-between text-xs text-muted-foreground">
        <span>{t("settings:effort.count", { current: levels.length, max: MAX_EFFORT_LEVELS })}</span>
        {error === null ? null : (
          <span role="alert" data-testid="effort-error" className="text-status-failed">
            {t(`settings:effort.error.${error}`, { max: MAX_EFFORT_LEVELS })}
          </span>
        )}
      </div>

      {unsendable.length === 0 ? null : (
        <Alert variant="destructive" data-testid="ignored-effort-alert">
          <TriangleAlert />
          <AlertTitle>{t("settings:effort.ignored_title")}</AlertTitle>
          <AlertDescription>
            <p>
              {t("settings:effort.ignored_body", {
                protocol: protocolName,
                levels: unsendable.join(", "),
              })}
            </p>
            <Button variant="outline" size="sm" className="mt-2" onClick={removeUnsupported}>
              {t("settings:effort.ignored_fix")}
            </Button>
          </AlertDescription>
        </Alert>
      )}

      {levels.length === 0 ? null : (
        <div className="space-y-1.5">
          <Label htmlFor="model-effort-default">{t("settings:effort.default")}</Label>
          <Select
            value={defaultEffort ?? DEFAULT_EFFORT_NONE}
            onValueChange={(value) =>
              onChange(levels, value === DEFAULT_EFFORT_NONE ? null : value)
            }
          >
            <SelectTrigger id="model-effort-default" aria-label={t("settings:effort.default")}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={DEFAULT_EFFORT_NONE}>{t("settings:effort.default_none")}</SelectItem>
              {levels.map((level) => (
                <SelectItem key={level} value={level}>
                  {t(`settings:value.effort.${level}`, { defaultValue: level })}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      )}
    </div>
  )
}

/** The OpenAI-protocol chip input: typed text is appended in entry order. */
function OpenAiEffortChips({
  levels,
  onAdd,
  onRemove,
}: {
  levels: string[]
  onAdd: (value: string) => void
  onRemove: (level: string) => void
}) {
  const { t } = useTranslation()
  const [query, setQuery] = useState("")

  const submit = () => {
    onAdd(query)
    setQuery("")
  }

  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <Input
          aria-label={t("settings:effort.level_input")}
          placeholder={t("settings:effort.placeholder")}
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={(event) => {
            // An Enter that belongs to an IME candidate window must not add a chip (AGENTS.md §6).
            if (event.key === "Enter" && !isComposing(event)) {
              event.preventDefault()
              submit()
            }
          }}
        />
        <Button type="button" variant="outline" size="sm" onClick={submit}>
          <Plus className="size-3.5" />
          {t("settings:effort.add")}
        </Button>
      </div>
      {levels.length === 0 ? null : (
        <div className="flex flex-wrap gap-1" aria-label={t("settings:effort.levels")}>
          {levels.map((level) => (
            <Badge key={level} variant="secondary" data-testid={`effort-level-${level}`}>
              <span>{level}</span>
              <button
                type="button"
                aria-label={t("settings:effort.remove", { level })}
                className="ml-1 rounded-sm hover:text-destructive"
                onClick={() => onRemove(level)}
              >
                <X className="size-3" />
              </button>
            </Badge>
          ))}
        </div>
      )}
    </div>
  )
}
