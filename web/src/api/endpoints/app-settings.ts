/** Application settings endpoints (spec/10 §2.1). */
import { api } from "@/api/client"
import type { AppSettingsDto } from "@/api/gen/AppSettingsDto"
import type { AppSettingsPatch } from "@/api/gen/AppSettingsPatch"
import type { SearchTestInput } from "@/api/gen/SearchTestInput"
import type { SearchTestResponse } from "@/api/gen/SearchTestResponse"

export function getAppSettings(): Promise<AppSettingsDto> {
  return api.get<AppSettingsDto>("/app-settings")
}

export function patchAppSettings(patch: AppSettingsPatch): Promise<AppSettingsDto> {
  return api.patch<AppSettingsDto>("/app-settings", patch)
}

/** `POST /api/app-settings/search/test` (spec/10 §2.1): one real search with the saved provider. */
export function testSearch(input: SearchTestInput): Promise<SearchTestResponse> {
  return api.post<SearchTestResponse>("/app-settings/search/test", input)
}
