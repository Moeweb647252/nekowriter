import type { ContextGroupKey } from "@/api/gen/ContextGroupKey"
import type { ContextPartKey } from "@/api/gen/ContextPartKey"
import type { ContextPartDto } from "@/api/gen/ContextPartDto"
import type { TFunction } from "i18next"

const GROUP_KEYS: ContextGroupKey[] = ["system", "tools", "kickoff", "author", "runtime", "model", "results"]
const PART_KEYS: ContextPartKey[] = ["system_prompt", "tool", "block", "message", "feedback", "edit_report", "notice", "effect_notice", "nudge", "text", "reasoning", "call", "result"]

export const GROUP_COLOR: Record<ContextGroupKey, string> = {
  system: "var(--context-setup)",
  tools: "var(--context-setup)",
  kickoff: "var(--context-setup)",
  author: "var(--context-messages)",
  runtime: "var(--context-messages)",
  model: "var(--context-work)",
  results: "var(--context-work)",
}

export const GROUP_LABEL_KEYS: Record<ContextGroupKey, string> = Object.fromEntries(
  GROUP_KEYS.map((key) => [key, `agent:context.group.${key}`]),
) as Record<ContextGroupKey, string>

export const PART_LABEL_KEYS: Record<ContextPartKey, string> = Object.fromEntries(
  PART_KEYS.map((key) => [key, `agent:context.part.${key}`]),
) as Record<ContextPartKey, string>

const BLOCK_LABELS: Record<string, string> = {
  "block.settings": "agent:context.block.settings",
  "block.settings.outline": "agent:context.block.settings_outline",
  "block.settings.characters": "agent:context.block.settings_characters",
  "block.settings.world": "agent:context.block.settings_world",
  "block.settings.protagonist": "agent:context.block.settings_protagonist",
  "block.canon_systems": "agent:context.block.canon_systems",
  "block.writing_skills": "agent:context.block.writing_skills",
  "block.memory": "agent:context.block.memory",
  "block.memory.recent": "agent:context.block.memory_recent",
  "block.memory.entities": "agent:context.block.memory_entities",
  "block.memory.facts": "agent:context.block.memory_facts",
  "block.memory.foreshadowing": "agent:context.block.memory_foreshadowing",
  "block.recent_chapters": "agent:context.block.recent_chapters",
  "block.goals": "agent:context.block.goals",
  "block.chapter_draft": "agent:context.block.chapter_draft",
  "block.cast_index": "agent:context.block.cast_index",
  "block.chapter_diff": "agent:context.block.chapter_diff",
  "block.book_brief": "agent:context.block.book_brief",
  "block.book_index": "agent:context.block.book_index",
  "assistant.kickoff": "agent:context.block.first_message",
}

export function groupLabel(t: TFunction, key: ContextGroupKey): string {
  return t(GROUP_LABEL_KEYS[key]) as string
}

export function partLabel(t: TFunction, part: ContextPartDto, _group: ContextGroupKey, index: number): string {
  if ((part.key === "tool" || part.key === "call" || part.key === "result") && part.name !== null) {
    return part.name
  }
  if (part.key === "block") {
    const name = part.name
    if (name !== null && BLOCK_LABELS[name] !== undefined) {
      return t(BLOCK_LABELS[name]) as string
    }
    if (name === null) return t("agent:context.block.unlabeled", { ns: "agent", number: index + 1 }) as string
    return t("agent:context.block.task") as string
  }
  return t(PART_LABEL_KEYS[part.key]) as string
}
