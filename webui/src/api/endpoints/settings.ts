import { getJson, postJson } from "../client"
import type { EmbeddingSettingsView } from "../generated/EmbeddingSettingsView"
import type { EmbeddingTestView } from "../generated/EmbeddingTestView"
import type { ModelTestView } from "../generated/ModelTestView"
import type { SaveEmbeddingRequest } from "../generated/SaveEmbeddingRequest"
import type { SaveSettingsRequest } from "../generated/SaveSettingsRequest"
import type { SystemSettingsView } from "../generated/SystemSettingsView"
import type { TestEmbeddingRequest } from "../generated/TestEmbeddingRequest"
import type { TestModelRequest } from "../generated/TestModelRequest"

export const settingsApi = {
  embedding: () => getJson<EmbeddingSettingsView>("/api/settings/embedding"),
  saveEmbedding: (request: SaveEmbeddingRequest) =>
    postJson<EmbeddingSettingsView>("/api/settings/embedding", request),
  testEmbedding: (request: TestEmbeddingRequest) =>
    postJson<EmbeddingTestView>("/api/settings/test-embedding", request),
  get: () => getJson<SystemSettingsView>("/api/settings"),
  save: (request: SaveSettingsRequest) => postJson<SystemSettingsView>("/api/settings", request),
  testModel: (request: TestModelRequest) =>
    postJson<ModelTestView>("/api/settings/test-model", request),
}
