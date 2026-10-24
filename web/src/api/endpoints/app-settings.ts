/** Application settings endpoints (spec/10 §2.1). */
import { api } from "@/api/client"
import type { AppSettingsDto } from "@/api/gen/AppSettingsDto"
import type { AppSettingsPatch } from "@/api/gen/AppSettingsPatch"

export function getAppSettings(): Promise<AppSettingsDto> {
  return api.get<AppSettingsDto>("/app-settings")
}

export function patchAppSettings(patch: AppSettingsPatch): Promise<AppSettingsDto> {
  return api.patch<AppSettingsDto>("/app-settings", patch)
}
