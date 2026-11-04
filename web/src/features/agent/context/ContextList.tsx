import { useTranslation } from "react-i18next"

import type { ContextBreakdownDto } from "@/api/gen/ContextBreakdownDto"
import type { ContextGroupDto } from "@/api/gen/ContextGroupDto"
import { formatNumber } from "@/lib/format"

import { partLabel, groupLabel } from "./labels"

function tokens(estimate: number, scale: number, language: string): string {
  return formatNumber(Math.round(estimate * scale), language)
}

function share(estimate: number, total: number): number {
  return total > 0 ? Math.round((estimate / total) * 100) : 0
}

function GroupRows({ group, view }: { group: ContextGroupDto; view: ContextBreakdownDto }) {
  const { t, i18n } = useTranslation()
  const language = i18n.language
  return (
    <tbody>
      <tr className="border-t bg-muted/30 font-medium">
        <td className="px-3 py-1.5">{groupLabel(t, group.key)}</td>
        <td className="px-3 py-1.5 text-right tabular-nums">{tokens(group.estimate, view.scale, language)}</td>
        <td className="px-3 py-1.5 text-right tabular-nums">{share(group.estimate, view.estimate)}%</td>
      </tr>
      {group.parts.map((part, index) => (
        <ContextPartRows key={`${group.key}-${part.key}-${part.name ?? index}`} group={group.key} part={part} view={view} index={index} />
      ))}
    </tbody>
  )
}

function ContextPartRows({ group, part, view, index }: { group: ContextGroupDto["key"]; part: ContextGroupDto["parts"][number]; view: ContextBreakdownDto; index: number }) {
  const { t, i18n } = useTranslation()
  const language = i18n.language
  return (
    <>
      <tr className="border-t text-xs">
        <td className="py-1 pl-8 pr-3">{partLabel(t, part, group, index)}</td>
        <td className="px-3 py-1 text-right tabular-nums">{tokens(part.estimate, view.scale, language)}</td>
        <td className="px-3 py-1 text-right tabular-nums">{share(part.estimate, view.estimate)}%</td>
      </tr>
      {part.items.map((item, index) => (
        <tr key={`${item.seq}-${item.call_index ?? "x"}-${index}`} className="border-t text-xs text-muted-foreground">
          <td className="py-1 pl-12 pr-3">{item.excerpt}</td>
          <td className="px-3 py-1 text-right tabular-nums">{tokens(item.estimate, view.scale, language)}</td>
          <td className="px-3 py-1 text-right tabular-nums">{share(item.estimate, view.estimate)}%</td>
        </tr>
      ))}
      {part.rest !== null ? (
        <tr className="border-t text-xs text-muted-foreground">
          <td className="py-1 pl-12 pr-3">{t("agent:context.list.more", { ns: "agent", count: part.rest.count })}</td>
          <td className="px-3 py-1 text-right tabular-nums">{tokens(part.rest.estimate, view.scale, language)}</td>
          <td className="px-3 py-1" />
        </tr>
      ) : null}
    </>
  )
}

export function ContextList({ view }: { view: ContextBreakdownDto }) {
  const { t } = useTranslation()
  return (
    <div className="max-h-[420px] overflow-auto rounded-md border">
      <table className="w-full border-collapse text-sm">
        <thead className="sticky top-0 bg-background">
          <tr className="text-left text-xs text-muted-foreground">
            <th className="px-3 py-2 font-medium">{t("agent:context.list.name")}</th>
            <th className="px-3 py-2 text-right font-medium">{t("agent:context.list.tokens")}</th>
            <th className="px-3 py-2 text-right font-medium">{t("agent:context.list.share")}</th>
          </tr>
        </thead>
        {view.groups.map((group) => <GroupRows key={group.key} group={group} view={view} />)}
      </table>
    </div>
  )
}
