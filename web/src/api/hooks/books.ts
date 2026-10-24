/** Book queries and mutations (spec/11 §4.1). */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import { ApiError } from "@/api/client"
import * as endpoints from "@/api/endpoints/books"
import { keys } from "@/api/keys"
import type { BookInput } from "@/api/gen/BookInput"
import type { BookPatch } from "@/api/gen/BookPatch"

export function useBooks(status?: string) {
  return useQuery({
    queryKey: keys.books.list(status),
    queryFn: () => endpoints.listBooks(status),
  })
}

export function useBook(bookId: number) {
  return useQuery({
    queryKey: keys.books.detail(bookId),
    queryFn: () => endpoints.getBook(bookId),
    enabled: Number.isFinite(bookId) && bookId > 0,
  })
}

export function useOverview(bookId: number) {
  return useQuery({
    queryKey: keys.books.overview(bookId),
    queryFn: () => endpoints.getOverview(bookId),
    enabled: Number.isFinite(bookId) && bookId > 0,
  })
}

export function useCreateBook() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (input: BookInput) => endpoints.createBook(input),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.books.all })
    },
  })
}

export function usePatchBook(bookId: number) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (patch: BookPatch) => endpoints.patchBook(bookId, patch),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.books.all })
      void queryClient.invalidateQueries({ queryKey: keys.books.detail(bookId) })
      void queryClient.invalidateQueries({ queryKey: keys.books.overview(bookId) })
    },
  })
}

export function useDeleteBook() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (bookId: number) => endpoints.deleteBook(bookId),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.books.all })
    },
  })
}

/** The API error of a failed mutation, when there is one (for localized toasts). */
export function apiError(error: unknown): ApiError | null {
  return error instanceof ApiError ? error : null
}
