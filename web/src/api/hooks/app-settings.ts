/** Application-settings queries and mutations (spec/10 §2.1). */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import * as endpoints from "@/api/endpoints/app-settings"
import { keys } from "@/api/keys"
import type { AppSettingsPatch } from "@/api/gen/AppSettingsPatch"
import type { SearchTestInput } from "@/api/gen/SearchTestInput"
import type { SearchTestResponse } from "@/api/gen/SearchTestResponse"

export function useAppSettings() {
  return useQuery({
    queryKey: keys.appSettings,
    queryFn: endpoints.getAppSettings,
  })
}

export function usePatchAppSettings() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (patch: AppSettingsPatch) => endpoints.patchAppSettings(patch),
    onSuccess: (settings) => {
      queryClient.setQueryData(keys.appSettings, settings)
    },
  })
}

/** Runs the Settings › Web search "Test" button (spec/10 §2.1). */
export function useTestSearch() {
  return useMutation({
    mutationFn: (input: SearchTestInput): Promise<SearchTestResponse> =>
      endpoints.testSearch(input),
  })
}
