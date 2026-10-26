/** Provider, model-profile and role-default queries and mutations (spec/11 §4.1). */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import * as endpoints from "@/api/endpoints/providers"
import { keys } from "@/api/keys"
import type { ModelProfileInput } from "@/api/gen/ModelProfileInput"
import type { ProviderInput } from "@/api/gen/ProviderInput"

export function useProviders() {
  return useQuery({
    queryKey: keys.providers.all,
    queryFn: endpoints.listProviders,
  })
}

export function useProviderPresets() {
  return useQuery({
    queryKey: keys.providers.presets,
    queryFn: endpoints.listProviderPresets,
    // The preset table is derived from the server's demo-provider flag, which cannot change
    // while it runs.
    staleTime: Number.POSITIVE_INFINITY,
  })
}

/** The option → protocol table (spec/04 §5). It is derived from the adapters, so it cannot change
 * while the server runs. */
export function useOptionSpecs() {
  return useQuery({
    queryKey: keys.providers.optionSpecs,
    queryFn: endpoints.listOptionSpecs,
    staleTime: Number.POSITIVE_INFINITY,
  })
}

export function useCreateProvider() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (input: ProviderInput) => endpoints.createProvider(input),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.providers.all })
      void queryClient.invalidateQueries({ queryKey: keys.modelProfiles.all })
    },
  })
}

export function usePatchProvider() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ id, input }: { id: number; input: ProviderInput }) =>
      endpoints.patchProvider(id, input),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.providers.all })
    },
  })
}

export function useDeleteProvider() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (id: number) => endpoints.deleteProvider(id),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.providers.all })
      void queryClient.invalidateQueries({ queryKey: keys.modelProfiles.all })
      void queryClient.invalidateQueries({ queryKey: keys.appSettings })
    },
  })
}

export function useTestProvider() {
  return useMutation({
    mutationFn: ({ id, model }: { id: number; model?: string }) =>
      endpoints.testProvider(id, model),
  })
}

export function useRemoteModels() {
  return useMutation({
    mutationFn: (id: number) => endpoints.remoteModels(id),
  })
}

export function useModelProfiles() {
  return useQuery({
    queryKey: keys.modelProfiles.all,
    queryFn: endpoints.listModelProfiles,
  })
}

export function useCreateModelProfile() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (input: ModelProfileInput) => endpoints.createModelProfile(input),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.modelProfiles.all })
      void queryClient.invalidateQueries({ queryKey: keys.providers.all })
    },
  })
}

export function usePatchModelProfile() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ id, input }: { id: number; input: ModelProfileInput }) =>
      endpoints.patchModelProfile(id, input),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.modelProfiles.all })
      void queryClient.invalidateQueries({ queryKey: keys.providers.all })
    },
  })
}

export function useDeleteModelProfile() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (id: number) => endpoints.deleteModelProfile(id),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.modelProfiles.all })
      void queryClient.invalidateQueries({ queryKey: keys.providers.all })
      void queryClient.invalidateQueries({ queryKey: keys.appSettings })
    },
  })
}
