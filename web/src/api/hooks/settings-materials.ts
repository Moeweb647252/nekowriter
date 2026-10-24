/** Setting material queries and mutations (spec/11 §4.1). */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import * as endpoints from "@/api/endpoints/settings-materials"
import { keys } from "@/api/keys"
import type { CharacterInput } from "@/api/gen/CharacterInput"
import type { CharacterPatch } from "@/api/gen/CharacterPatch"
import type { SettingDocInput } from "@/api/gen/SettingDocInput"

export function useSettingDocs(bookId: number) {
  return useQuery({
    queryKey: keys.settings.docs(bookId),
    queryFn: () => endpoints.listSettingDocs(bookId),
    enabled: Number.isFinite(bookId) && bookId > 0,
  })
}

export function usePutSettingDoc(bookId: number, kind: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (input: SettingDocInput) => endpoints.putSettingDoc(bookId, kind, input),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.settings.docs(bookId) })
      void queryClient.invalidateQueries({ queryKey: keys.settings.docHistory(bookId, kind) })
      void queryClient.invalidateQueries({ queryKey: keys.changes.list(bookId, "setting_doc") })
    },
  })
}

export function useSettingDocHistory(bookId: number, kind: string) {
  return useQuery({
    queryKey: keys.settings.docHistory(bookId, kind),
    queryFn: () => endpoints.settingDocHistory(bookId, kind),
    enabled: Number.isFinite(bookId) && bookId > 0,
  })
}

export function useCharacters(bookId: number) {
  return useQuery({
    queryKey: keys.settings.characters(bookId),
    queryFn: () => endpoints.listCharacters(bookId),
    enabled: Number.isFinite(bookId) && bookId > 0,
  })
}

export function useCreateCharacter(bookId: number) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (input: CharacterInput) => endpoints.createCharacter(bookId, input),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.settings.characters(bookId) })
    },
  })
}

export function usePatchCharacter(bookId: number) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ id, patch }: { id: number; patch: CharacterPatch }) =>
      endpoints.patchCharacter(id, patch),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.settings.characters(bookId) })
    },
  })
}

export function useDeleteCharacter(bookId: number) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (id: number) => endpoints.deleteCharacter(id),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.settings.characters(bookId) })
    },
  })
}

export function useReorderCharacters(bookId: number) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (ids: number[]) => endpoints.reorderCharacters(bookId, ids),
    onSuccess: (list) => {
      queryClient.setQueryData(keys.settings.characters(bookId), list)
    },
  })
}
