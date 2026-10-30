/** Goal endpoints (spec/10 §2.4, spec/06 §11). */
import { api } from "@/api/client"
import type { GoalNodeDto } from "@/api/gen/GoalNodeDto"
import type { GoalNodeInput } from "@/api/gen/GoalNodeInput"
import type { GoalQuery } from "@/api/gen/GoalQuery"
import type { GoalNodePatch } from "@/api/gen/GoalNodePatch"

export type { GoalNodePatch } from "@/api/gen/GoalNodePatch"

function queryString(query: GoalQuery): string {
  const params = new URLSearchParams()
  if (query.track_id !== undefined && query.track_id !== null) {
    params.set("track_id", String(query.track_id))
  }
  if (query.status !== undefined && query.status !== null) {
    params.set("status", query.status)
  }
  if (query.entity_id !== undefined && query.entity_id !== null) {
    params.set("entity_id", String(query.entity_id))
  }
  if (query.event_id !== undefined && query.event_id !== null) {
    params.set("event_id", String(query.event_id))
  }
  if (query.foreshadowing_id !== undefined && query.foreshadowing_id !== null) {
    params.set("foreshadowing_id", String(query.foreshadowing_id))
  }
  const suffix = params.toString()
  return suffix === "" ? "" : `?${suffix}`
}

export function listGoals(bookId: number, query: GoalQuery = {}): Promise<GoalNodeDto[]> {
  return api.get<GoalNodeDto[]>(`/books/${bookId}/goals${queryString(query)}`)
}

export function createGoal(bookId: number, input: GoalNodeInput): Promise<GoalNodeDto> {
  return api.post<GoalNodeDto>(`/books/${bookId}/goals`, input)
}

export function patchGoal(goalId: number, patch: GoalNodePatch): Promise<GoalNodeDto> {
  return api.patch<GoalNodeDto>(`/goals/${goalId}`, patch)
}

export function deleteGoal(goalId: number): Promise<void> {
  return api.delete<void>(`/goals/${goalId}`)
}
