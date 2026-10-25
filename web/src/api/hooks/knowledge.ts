/** Knowledge queries and mutations (spec/11 §4.1, spec/10 §2.5). */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import * as endpoints from "@/api/endpoints/knowledge"
import { keys } from "@/api/keys"
import type { EntryInput } from "@/api/gen/EntryInput"
import type { EntryPatch } from "@/api/gen/EntryPatch"
import type { ScopeInput } from "@/api/gen/ScopeInput"
import type { ScopePatch } from "@/api/gen/ScopePatch"

/** Invalidates every knowledge query after a write (spec/10 §3 `knowledge.changed`). */
function useKnowledgeInvalidator() {
  const queryClient = useQueryClient()
  return () => {
    void queryClient.invalidateQueries({ queryKey: keys.knowledge.all })
  }
}

export function useScopes() {
  return useQuery({
    queryKey: keys.knowledge.scopes(),
    queryFn: () => endpoints.listScopes(),
  })
}

export function useCreateScope() {
  const invalidate = useKnowledgeInvalidator()
  return useMutation({
    mutationFn: (input: ScopeInput) => endpoints.createScope(input),
    onSuccess: invalidate,
  })
}

export function usePatchScope() {
  const invalidate = useKnowledgeInvalidator()
  return useMutation({
    mutationFn: ({ scopeId, patch }: { scopeId: number; patch: ScopePatch }) =>
      endpoints.patchScope(scopeId, patch),
    onSuccess: invalidate,
  })
}

export function useDeleteScope() {
  const invalidate = useKnowledgeInvalidator()
  return useMutation({
    mutationFn: (scopeId: number) => endpoints.deleteScope(scopeId),
    onSuccess: invalidate,
  })
}

export function useEntries(scopeId: number | undefined, q: string) {
  return useQuery({
    queryKey: keys.knowledge.entries(scopeId ?? 0, q),
    queryFn: () => endpoints.listEntries(scopeId ?? 0, { q, limit: 100 }),
    enabled: scopeId !== undefined && scopeId > 0,
  })
}

export function useEntry(entryId: number | undefined) {
  return useQuery({
    queryKey: keys.knowledge.entry(entryId ?? 0),
    queryFn: () => endpoints.getEntry(entryId ?? 0),
    enabled: entryId !== undefined && entryId > 0,
  })
}

export function useCreateEntry(scopeId: number) {
  const invalidate = useKnowledgeInvalidator()
  return useMutation({
    mutationFn: (input: EntryInput) => endpoints.createEntry(scopeId, input),
    onSuccess: invalidate,
  })
}

export function usePatchEntry(entryId: number) {
  const invalidate = useKnowledgeInvalidator()
  return useMutation({
    mutationFn: (patch: EntryPatch) => endpoints.patchEntry(entryId, patch),
    onSuccess: invalidate,
  })
}

export function useDeleteEntry() {
  const invalidate = useKnowledgeInvalidator()
  return useMutation({
    mutationFn: (entryId: number) => endpoints.deleteEntry(entryId),
    onSuccess: invalidate,
  })
}

export function useKnowledgeSearch(q: string, scopeIds: number[]) {
  return useQuery({
    queryKey: keys.knowledge.search(q, scopeIds),
    queryFn: () => endpoints.searchKnowledge({ q, scopeIds, limit: 20 }),
    enabled: q.trim().length > 0,
  })
}

export function useBookScopes(bookId: number) {
  return useQuery({
    queryKey: keys.knowledge.bookScopes(bookId),
    queryFn: () => endpoints.getBookScopes(bookId),
    enabled: bookId > 0,
  })
}

export function usePutBookScopes(bookId: number) {
  const invalidate = useKnowledgeInvalidator()
  return useMutation({
    mutationFn: (scopeIds: number[]) =>
      endpoints.putBookScopes(bookId, { scope_ids: scopeIds }),
    onSuccess: invalidate,
  })
}

export function useEmbeddingStatus() {
  return useQuery({
    queryKey: keys.embedding.status,
    queryFn: () => endpoints.getEmbeddingStatus(),
    refetchInterval: 5_000,
  })
}

export function useRebuildIndex() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () => endpoints.rebuildIndex(),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.embedding.status })
    },
  })
}
