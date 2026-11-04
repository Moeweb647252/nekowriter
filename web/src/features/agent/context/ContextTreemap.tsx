import { useMemo, useRef, useState, type KeyboardEvent } from "react"
import { useTranslation } from "react-i18next"

import type { ContextBreakdownDto } from "@/api/gen/ContextBreakdownDto"
import type { ContextGroupKey } from "@/api/gen/ContextGroupKey"
import type { TFunction } from "i18next"

import { formatNumber } from "@/lib/format"
import { layoutTreemap } from "@/lib/treemap"

import { GROUP_COLOR, groupLabel, partLabel } from "./labels"

export type ZoomPath = { group: ContextGroupKey | null; part: { key: string; name: string | null } | null }

type Leaf = {
  id: string
  label: string
  weight: number
  color: string
  group: ContextGroupKey
  part: string
  excerpt: string
  onClick?: () => void
}

function leavesFor(view: ContextBreakdownDto, zoom: ZoomPath, t: TFunction): Leaf[] {
  if (zoom.group === null) {
    return view.groups.map((group) => ({
      id: `group-${group.key}`,
      label: groupLabel(t, group.key),
      weight: group.estimate,
      color: GROUP_COLOR[group.key],
      group: group.key,
      part: "",
      excerpt: "",
      onClick: undefined,
    }))
  }
  const group = view.groups.find((candidate) => candidate.key === zoom.group)
  if (group === undefined) return []
  if (zoom.part === null) {
    return group.parts.map((part, index) => ({
      id: `part-${group.key}-${part.key}-${part.name ?? index}`,
      label: partLabel(t, part, group.key, index),
      weight: part.estimate,
      color: GROUP_COLOR[group.key],
      group: group.key,
      part: part.name ?? "",
      excerpt: part.items[0]?.excerpt ?? "",
      onClick: part.items.length > 0 ? undefined : undefined,
    }))
  }
  const part = group.parts.find((candidate) => candidate.key === zoom.part?.key && candidate.name === zoom.part?.name)
  if (part === undefined) return []
  return part.items.map((item, index) => ({
    id: `item-${part.key}-${index}`,
    label: item.excerpt,
    weight: item.estimate,
    color: GROUP_COLOR[group.key],
    group: group.key,
    part: part.name ?? "",
    excerpt: item.excerpt,
  }))
}

export function ContextTreemap({
  view,
  zoom,
  onZoom,
  onReset,
}: {
  view: ContextBreakdownDto
  zoom: ZoomPath
  onZoom: (path: ZoomPath) => void
  onReset: () => void
}) {
  const { t, i18n } = useTranslation()
  const width = 1000
  const [selected, setSelected] = useState(0)
  const container = useRef<HTMLDivElement>(null)
  const rects = useMemo(() => {
    const leaves = leavesFor(view, zoom, t)
    return { leaves, layout: layoutTreemap(leaves.map((leaf) => ({ key: leaf.id, weight: leaf.weight })), { x: 0, y: 0, width, height: 420 }) }
  }, [view, zoom, width, t])
  const language = i18n.language
  const current = rects.leaves[Math.min(selected, Math.max(0, rects.leaves.length - 1))]
  const total = view.estimate

  function zoomLeaf(leaf: Leaf) {
    if (zoom.group === null) {
      onZoom({ group: leaf.group, part: null })
    } else if (zoom.part === null) {
      const group = view.groups.find((candidate) => candidate.key === zoom.group)
      const part = group?.parts.find((candidate) => partLabel(t, candidate, group.key, 0) === leaf.label)
      if (part !== undefined && part.items.length > 0) {
        onZoom({ group: zoom.group, part: { key: part.key, name: part.name } })
      }
    }
  }

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      if (zoom.group !== null) {
        event.preventDefault()
        if (zoom.part !== null) onZoom({ group: zoom.group, part: null })
        else onReset()
      }
      return
    }
    if (event.key === "ArrowRight" || event.key === "ArrowDown") {
      event.preventDefault()
      setSelected((value) => Math.min(value + 1, Math.max(0, rects.leaves.length - 1)))
    } else if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
      event.preventDefault()
      setSelected((value) => Math.max(0, value - 1))
    } else if (event.key === "Home") {
      event.preventDefault()
      setSelected(0)
    } else if (event.key === "End") {
      event.preventDefault()
      setSelected(Math.max(0, rects.leaves.length - 1))
    } else if (event.key === "Enter" && current !== undefined) {
      event.preventDefault()
      zoomLeaf(current)
    }
  }

  return (
    <div
      ref={container}
      data-testid="context-treemap"
      role="group"
      tabIndex={0}
      aria-label={t("agent:context.title")}
      className="relative h-[420px] w-full overflow-hidden rounded-md border bg-muted/20 outline-none focus-visible:ring-2 focus-visible:ring-ring"
      onKeyDown={onKeyDown}
    >
      <svg width="100%" height="420" viewBox={`0 0 ${width} 420`} role="img" aria-label={t("agent:context.title")}>
        {rects.layout.map((rect, index) => {
          const leaf = rects.leaves.find((candidate) => candidate.id === rect.key)
          if (leaf === undefined) return null
          const isSelected = index === Math.min(selected, rects.leaves.length - 1)
          return (
            <g
              key={rect.key}
              data-testid={`context-cell-${leaf.id}`}
              role="button"
              tabIndex={-1}
              onClick={() => {
                setSelected(index)
                zoomLeaf(leaf)
              }}
              onFocus={() => setSelected(index)}
              className="cursor-pointer"
            >
              <title>{`${leaf.label} · ≈ ${formatNumber(Math.round(leaf.weight * view.scale), language)} tokens${leaf.excerpt === "" ? "" : `\n${leaf.excerpt}`}`}</title>
              <rect
                x={rect.x + 1}
                y={rect.y + 1}
                width={Math.max(1, rect.width - 2)}
                height={Math.max(1, rect.height - 2)}
                rx={2}
                fill={leaf.color}
                stroke={isSelected ? "var(--foreground)" : "transparent"}
                strokeWidth={isSelected ? 2 : 0}
              />
              {rect.width > 44 && rect.height > 18 ? (
                <text x={rect.x + 6} y={rect.y + 15} fill="#ffffff" fontSize={11} fontWeight={600} pointerEvents="none">
                  {leaf.label}
                </text>
              ) : null}
              {rect.width > 80 && rect.height > 34 ? (
                <text x={rect.x + 6} y={rect.y + 30} fill="#ffffff" fontSize={10} pointerEvents="none">
                  {Math.round((leaf.weight / total) * 100)}%
                </text>
              ) : null}
            </g>
          )
        })}
      </svg>
      {rects.leaves.length === 0 ? (
        <p className="absolute inset-0 flex items-center justify-center text-sm text-muted-foreground">{t("agent:context.empty")}</p>
      ) : null}
    </div>
  )
}
