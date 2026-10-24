/** Prompt registry queries and mutations (spec/10 §2.6, spec/09 §4). */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import * as endpoints from "@/api/endpoints/prompts"
import { keys } from "@/api/keys"
import type { PromptSaveInput } from "@/api/gen/PromptSaveInput"
import type { PromptScope } from "@/api/gen/PromptScope"

export function usePrompts(bookId?: number) {
  return useQuery({
    queryKey: keys.prompts.list(bookId),
    queryFn: () => endpoints.listPrompts(bookId),
  })
}

export function useSavePrompt(bookId?: number) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ key, input }: { key: string; input: PromptSaveInput }) =>
      endpoints.savePrompt(key, input),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.prompts.list(bookId) })
    },
  })
}

export function useResetPrompt(bookId?: number) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({
      key,
      scope,
      bookId: targetBookId,
    }: {
      key: string
      scope: PromptScope
      bookId?: number
    }) => endpoints.resetPrompt(key, scope, targetBookId),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.prompts.list(bookId) })
    },
  })
}
