/** Research queries and mutations (spec/11 §4.1, spec/10 §2.5). */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import * as endpoints from "@/api/endpoints/research"
import { startKnowledgeCheck } from "@/api/endpoints/books"
import { keys } from "@/api/keys"
import type { ResearchTaskCreate } from "@/api/gen/ResearchTaskCreate"
import type { ResearchTaskPatch } from "@/api/gen/ResearchTaskPatch"

/**
 * Invalidates every research query after a write (spec/10 §3 `research.updated`).
 *
 * The overview is invalidated too: `next_action` and `attention` count the book's `ready` tasks
 * (D-136), so deferring or resuming one changes the card the author is about to look at (D-138) —
 * without this, deferring the last runnable task left the overview still offering `run_research`.
 * A write whose response carries the task's book narrows the invalidation to that overview; one
 * that does not (delete) falls back to every book, which is correct for a book-less task too.
 */
function useResearchInvalidator() {
  const queryClient = useQueryClient()
  return (bookId?: number | null) => {
    void queryClient.invalidateQueries({ queryKey: keys.research.all })
    if (typeof bookId === "number") {
      void queryClient.invalidateQueries({ queryKey: keys.books.overview(bookId) })
    } else {
      void queryClient.invalidateQueries({ queryKey: keys.books.all })
    }
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

/** Starts a knowledge check for a book (spec/08 §5, D-158). */
export function useStartKnowledgeCheck(bookId: number) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (requirements: string) =>
      startKnowledgeCheck(bookId, { requirements }),
    onSuccess: (response) => {
      void queryClient.invalidateQueries({ queryKey: keys.research.all })
      void queryClient.invalidateQueries({ queryKey: keys.sessions.list(bookId) })
      void queryClient.invalidateQueries({ queryKey: keys.books.detail(bookId) })
      void queryClient.invalidateQueries({ queryKey: keys.books.all })
      return response
    },
  })
}

export function useCreateResearchTask() {
  const invalidate = useResearchInvalidator()
  return useMutation({
    mutationFn: (input: ResearchTaskCreate) => endpoints.createResearchTask(input),
    onSuccess: (task) => invalidate(task.book_id),
  })
}

export function usePatchResearchTask() {
  const invalidate = useResearchInvalidator()
  return useMutation({
    mutationFn: ({ taskId, patch }: { taskId: number; patch: ResearchTaskPatch }) =>
      endpoints.patchResearchTask(taskId, patch),
    onSuccess: (task) => invalidate(task.book_id),
  })
}

export function useDeleteResearchTask() {
  const invalidate = useResearchInvalidator()
  return useMutation({
    mutationFn: (taskId: number) => endpoints.deleteResearchTask(taskId),
    onSuccess: () => invalidate(),
  })
}

export function useRunResearchTask(bookId?: number) {
  const queryClient = useQueryClient()
  const invalidate = useResearchInvalidator()
  return useMutation({
    mutationFn: (taskId: number) => endpoints.runResearchTask(taskId),
    onSuccess: (response) => {
      // The run response is only the session id; the caller may know the book. A failed
      // research task is continued on its existing session (D-163), so the session list and
      // that session's snapshot must refresh too, or the failed pane keeps showing `failed`.
      invalidate(bookId)
      if (bookId !== undefined) {
        void queryClient.invalidateQueries({ queryKey: keys.sessions.list(bookId) })
      }
      void queryClient.invalidateQueries({ queryKey: keys.sessions.detail(response.session_id) })
      return response
    },
  })
}

/** `POST /api/research-tasks/:id/defer` (spec/10 §2.5, D-138). */
export function useDeferResearchTask() {
  const invalidate = useResearchInvalidator()
  return useMutation({
    mutationFn: (taskId: number) => endpoints.deferResearchTask(taskId),
    onSuccess: (task) => invalidate(task.book_id),
  })
}

/** `POST /api/research-tasks/:id/resume` (spec/10 §2.5, D-138). */
export function useResumeResearchTask() {
  const invalidate = useResearchInvalidator()
  return useMutation({
    mutationFn: (taskId: number) => endpoints.resumeResearchTask(taskId),
    onSuccess: (task) => invalidate(task.book_id),
  })
}
