import { getJson, postJson } from "../client"
import type { ModelTestView } from "../generated/ModelTestView"
import type { SaveSettingsRequest } from "../generated/SaveSettingsRequest"
import type { SystemSettingsView } from "../generated/SystemSettingsView"
import type { TestModelRequest } from "../generated/TestModelRequest"

export const settingsApi = {
  get: () => getJson<SystemSettingsView>("/api/settings"),
  save: (request: SaveSettingsRequest) => postJson<SystemSettingsView>("/api/settings", request),
  testModel: (request: TestModelRequest) =>
    postJson<ModelTestView>("/api/settings/test-model", request),
}
