/** Pure helpers for the Goals page (spec/06 §11, spec/11 §5.5). */
import type { TFunction } from "i18next"

import type { GoalCondition } from "@/api/gen/GoalCondition"
import type { GoalConditionStateDto } from "@/api/gen/GoalConditionStateDto"
import type { GoalNodeDto } from "@/api/gen/GoalNodeDto"

/** The ancestor chain from the root to the node, inclusive. */
export function goalPath(nodes: GoalNodeDto[], nodeId: number): GoalNodeDto[] {
  const byId = new Map(nodes.map((node) => [node.id, node]))
  const path: GoalNodeDto[] = []
  let current = byId.get(nodeId)
  const seen = new Set<number>()
  while (current !== undefined && !seen.has(current.id)) {
    seen.add(current.id)
    path.unshift(current)
    current = current.parent_id === null ? undefined : byId.get(current.parent_id)
  }
  return path
}

/** True when one of the node's ancestors is closed (done or dropped). */
export function hasClosedAncestor(nodes: GoalNodeDto[], node: GoalNodeDto): boolean {
  return goalPath(nodes, node.id)
    .slice(0, -1)
    .some((ancestor) => ancestor.status === "done" || ancestor.status === "dropped")
}

/** The sentence beside one condition, using the evaluator's target label. */
export function conditionSentence(t: TFunction, state: GoalConditionStateDto): string {
  const condition = state.condition
  const label = state.target_label ?? t("canon:goal_condition.unknown_target")
  switch (condition.kind) {
    case "chapter_at_least":
      return t("canon:goal_condition.chapter_at_least", { chapter: condition.chapter })
    case "foreshadowing_resolved":
      return t("canon:goal_condition.foreshadowing_resolved", { target: label })
    case "entity_status_is":
      return t("canon:goal_condition.entity_status_is", {
        target: label,
        status: t(`canon:status.${condition.status}`),
      })
  }
}

/** A human-readable path such as “Goal › Plan › Task”. */
export function pathLabel(nodes: GoalNodeDto[], nodeId: number): string {
  return goalPath(nodes, nodeId)
    .map((node) => node.title)
    .join(" › ")
}

/** The conditions of a node as editable wire values. */
export function conditionsOf(node: GoalNodeDto | null | undefined): GoalCondition[] {
  return node?.conditions.map((state) => state.condition) ?? []
}
