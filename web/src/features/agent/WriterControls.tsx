/**
 * The writer-only session controls (PROJECT §8.6, spec/03 §6, §8): a Granularity segmented control
 * and an Auto-pause switch, both persisted through `PATCH /sessions/:id`.
 *
 * The granularity control carries a tooltip on the cost/quality trade-off (PROJECT §8.6).
 */
import { useTranslation } from "react-i18next"

import type { Granularity } from "@/api/gen/Granularity"
import type { SessionDto } from "@/api/gen/SessionDto"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"

/** The four granularity values, in the order PROJECT §8.6 shows them. */
const GRANULARITIES: Granularity[] = ["sentence", "paragraph", "free", "chapter"]

export function GranularityControl({
  session,
  disabled,
  onChange,
}: {
  session: SessionDto
  disabled: boolean
  onChange: (granularity: Granularity) => void
}) {
  const { t } = useTranslation()
  const value = session.granularity ?? "paragraph"
  return (
    <div className="flex flex-wrap items-center gap-2 px-3 py-2">
      <Tooltip>
        <TooltipTrigger asChild>
          <span className="text-xs font-medium text-muted-foreground">
            {t("agent:writer.granularity")}
          </span>
        </TooltipTrigger>
        <TooltipContent className="max-w-72">{t("agent:writer.granularity_hint")}</TooltipContent>
      </Tooltip>
      <ToggleGroup
        type="single"
        size="sm"
        variant="outline"
        // The panel is 380 px wide by default: the four labels must shrink rather than clip.
        className="grid min-w-0 flex-1 grid-cols-4"
        value={value}
        disabled={disabled}
        onValueChange={(next) => {
          if (next !== "") {
            onChange(next as Granularity)
          }
        }}
        aria-label={t("agent:writer.granularity")}
      >
        {GRANULARITIES.map((granularity) => (
          <ToggleGroupItem key={granularity} value={granularity}>
            {t(`agent:granularity.${granularity}`)}
          </ToggleGroupItem>
        ))}
      </ToggleGroup>
    </div>
  )
}

export function AutoPauseSwitch({
  session,
  disabled,
  onChange,
}: {
  session: SessionDto
  disabled: boolean
  onChange: (autoPause: boolean) => void
}) {
  const { t } = useTranslation()
  return (
    <div className="flex items-center gap-2 px-3 pb-2">
      <Switch
        id="auto-pause"
        checked={session.auto_pause}
        disabled={disabled}
        onCheckedChange={onChange}
        aria-label={t("agent:writer.auto_pause")}
      />
      <Label htmlFor="auto-pause" className="text-xs text-muted-foreground">
        {t("agent:writer.auto_pause")}
      </Label>
      <Tooltip>
        <TooltipTrigger asChild>
          <span className="cursor-help text-xs text-muted-foreground">ⓘ</span>
        </TooltipTrigger>
        <TooltipContent className="max-w-72">{t("agent:writer.auto_pause_hint")}</TooltipContent>
      </Tooltip>
    </div>
  )
}
