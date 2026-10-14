import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import { useCommandIds } from "../command"
import { settingsApi } from "../endpoints/settings"
import type { ModelProfile } from "../generated/ModelProfile"
import type { SystemSettingsView } from "../generated/SystemSettingsView"
import type { TestModelRequest } from "../generated/TestModelRequest"
import { queryKeys } from "./keys"

export function useSystemSettings() {
  return useQuery({ queryKey: queryKeys.settings, queryFn: settingsApi.get })
}

/** What saving does with the stored API key. */
export type ApiKeyChange = { kind: "keep" } | { kind: "clear" } | { kind: "replace"; key: string }

export interface SystemSettingsInput {
  settings: SystemSettingsView
  apiBaseUrl: string | null
  apiKey: ApiKeyChange
  defaultModel: ModelProfile | null
}

export function useSaveSystemSettings() {
  const queryClient = useQueryClient()
  const commandIds = useCommandIds()
  return useMutation({
    mutationFn: ({ settings, apiBaseUrl, apiKey, defaultModel }: SystemSettingsInput) => {
      const input = {
        expectedRevision: settings.revision,
        apiBaseUrl,
        defaultModel,
        // Omitted keeps the stored key, `null` clears it, a string replaces it.
        ...(apiKey.kind === "keep" ? {} : { apiKey: apiKey.kind === "clear" ? null : apiKey.key }),
      }
      return settingsApi.save({ commandId: commandIds.idFor(input), ...input })
    },
    onSuccess: async (saved) => {
      commandIds.succeeded()
      queryClient.setQueryData(queryKeys.settings, saved)
      await queryClient.invalidateQueries({ queryKey: queryKeys.settings })
    },
  })
}

/** Sends a real request with the model under test (§13.3). The dashboard counts it. */
export function useModelTest() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (request: TestModelRequest) => settingsApi.testModel(request),
    onSettled: () => queryClient.invalidateQueries({ queryKey: queryKeys.dashboard.all }),
  })
}
