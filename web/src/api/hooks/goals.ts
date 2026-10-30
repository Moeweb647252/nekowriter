/** Goal queries and mutations (spec/11 §4.1, spec/06 §11). */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import * as endpoints from "@/api/endpoints/goals"
import type { GoalNodePatch } from "@/api/endpoints/goals"
import { keys } from "@/api/keys"
import type { GoalNodeInput } from "@/api/gen/GoalNodeInput"
import type { GoalQuery } from "@/api/gen/GoalQuery"

export type { GoalNodePatch } from "@/api/endpoints/goals"

/** A stable filter string for the goal query key. */
function goalFilter(query: GoalQuery): string {
  return [
    query.track_id ?? "shared",
    query.status ?? "all",
    query.entity_id ?? "",
    query.event_id ?? "",
    query.foreshadowing_id ?? "",
  ].join("|")
}

export function useGoals(bookId: number, query: GoalQuery = {}, enabled = true) {
  return useQuery({
    queryKey: keys.canon.goals(bookId, goalFilter(query)),
    queryFn: () => endpoints.listGoals(bookId, query),
    enabled: enabled && Number.isFinite(bookId) && bookId > 0,
  })
}

function useGoalInvalidator(bookId: number) {
  const queryClient = useQueryClient()
  return () => {
    void queryClient.invalidateQueries({ queryKey: keys.canon.goalsAll })
    void queryClient.invalidateQueries({ queryKey: keys.books.overview(bookId) })
    void queryClient.invalidateQueries({ queryKey: keys.changes.all })
  }
}

export function useCreateGoal(bookId: number) {
  const invalidate = useGoalInvalidator(bookId)
  return useMutation({
    mutationFn: (input: GoalNodeInput) => endpoints.createGoal(bookId, input),
    onSuccess: invalidate,
  })
}

export function usePatchGoal(bookId: number) {
  const invalidate = useGoalInvalidator(bookId)
  return useMutation({
    mutationFn: ({ id, patch }: { id: number; patch: GoalNodePatch }) =>
      endpoints.patchGoal(id, patch),
    onSuccess: invalidate,
  })
}

export function useDeleteGoal(bookId: number) {
  const invalidate = useGoalInvalidator(bookId)
  return useMutation({
    mutationFn: (id: number) => endpoints.deleteGoal(id),
    onSuccess: invalidate,
  })
}
