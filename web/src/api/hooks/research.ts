/** Research queries and mutations (spec/11 §4.1, spec/10 §2.5). */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import * as endpoints from "@/api/endpoints/research"
import { keys } from "@/api/keys"
import type { ResearchTaskCreate } from "@/api/gen/ResearchTaskCreate"
import type { ResearchTaskPatch } from "@/api/gen/ResearchTaskPatch"

/** Invalidates every research query after a write (spec/10 §3 `research.updated`). */
function useResearchInvalidator() {
  const queryClient = useQueryClient()
  return () => {
    void queryClient.invalidateQueries({ queryKey: keys.research.all })
  }
}

export function useResearchTasks(params: { bookId?: number } = {}) {
  return useQuery({
    queryKey: keys.research.list(params.bookId),
    queryFn: () => endpoints.listResearchTasks({ bookId: params.bookId }),
  })
}

export function useResearchTask(taskId: number) {
  return useQuery({
    queryKey: keys.research.detail(taskId),
    queryFn: () => endpoints.getResearchTask(taskId),
  })
}

export function useCreateResearchTask() {
  const invalidate = useResearchInvalidator()
  return useMutation({
    mutationFn: (input: ResearchTaskCreate) => endpoints.createResearchTask(input),
    onSuccess: invalidate,
  })
}

export function usePatchResearchTask() {
  const invalidate = useResearchInvalidator()
  return useMutation({
    mutationFn: ({ taskId, patch }: { taskId: number; patch: ResearchTaskPatch }) =>
      endpoints.patchResearchTask(taskId, patch),
    onSuccess: invalidate,
  })
}

export function useDeleteResearchTask() {
  const invalidate = useResearchInvalidator()
  return useMutation({
    mutationFn: (taskId: number) => endpoints.deleteResearchTask(taskId),
    onSuccess: invalidate,
  })
}

export function useRunResearchTask() {
  const invalidate = useResearchInvalidator()
  return useMutation({
    mutationFn: (taskId: number) => endpoints.runResearchTask(taskId),
    onSuccess: (response) => {
      invalidate()
      // The run settles the task and opens a session; the session store shows the panel.
      return response
    },
  })
}
