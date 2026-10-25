/** Conception queries and mutations (spec/11 §4.1, spec/08 §5). */
import { useMutation, useQueryClient } from "@tanstack/react-query"

import * as endpoints from "@/api/endpoints/conception"
import { keys } from "@/api/keys"

/** Invalidates everything the finalize changes: the book, its sessions and the research list. */
function useFinalizeInvalidator(bookId: number) {
  const queryClient = useQueryClient()
  return () => {
    void queryClient.invalidateQueries({ queryKey: keys.books.all })
    void queryClient.invalidateQueries({ queryKey: keys.books.detail(bookId) })
    void queryClient.invalidateQueries({ queryKey: keys.books.overview(bookId) })
    void queryClient.invalidateQueries({ queryKey: keys.sessions.list(bookId) })
    void queryClient.invalidateQueries({ queryKey: keys.research.all })
  }
}

/** Starts (or returns) the book's conception session (spec/08 §5, D-047). */
export function useStartConception(bookId: number) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () => endpoints.startConception(bookId),
    onSuccess: (response) => {
      void queryClient.invalidateQueries({ queryKey: keys.sessions.list(bookId) })
      void queryClient.invalidateQueries({
        queryKey: keys.sessions.detail(response.session_id),
      })
    },
  })
}

/** Finalizes the book (spec/08 §5). */
export function useFinalizeConception(bookId: number) {
  const invalidate = useFinalizeInvalidator(bookId)
  return useMutation({
    mutationFn: () => endpoints.finalizeConception(bookId),
    onSuccess: invalidate,
  })
}

/** Starts (or returns) the book's canon-seed session (spec/08 §5). */
export function useSeedCanon(bookId: number) {
  const invalidate = useFinalizeInvalidator(bookId)
  return useMutation({
    mutationFn: () => endpoints.seedCanon(bookId),
    onSuccess: invalidate,
  })
}
