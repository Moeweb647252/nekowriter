/** Provider, model-profile and preset endpoints (spec/10 §2.1, spec/04 §8). */
import { api } from "@/api/client"
import type { ModelProfileDto } from "@/api/gen/ModelProfileDto"
import type { ModelProfileInput } from "@/api/gen/ModelProfileInput"
import type { ProviderDto } from "@/api/gen/ProviderDto"
import type { ProviderInput } from "@/api/gen/ProviderInput"
import type { ProviderPresetDto } from "@/api/gen/ProviderPresetDto"
import type { ProviderTestDto } from "@/api/gen/ProviderTestDto"
import type { RemoteModelDto } from "@/api/gen/RemoteModelDto"

export function listProviders(): Promise<ProviderDto[]> {
  return api.get<ProviderDto[]>("/providers")
}

export function createProvider(input: ProviderInput): Promise<ProviderDto> {
  return api.post<ProviderDto>("/providers", input)
}

export function patchProvider(id: number, input: ProviderInput): Promise<ProviderDto> {
  return api.patch<ProviderDto>(`/providers/${id}`, input)
}

export function deleteProvider(id: number): Promise<void> {
  return api.delete<void>(`/providers/${id}`)
}

export function testProvider(id: number, model?: string): Promise<ProviderTestDto> {
  return api.post<ProviderTestDto>(`/providers/${id}/test`, { model: model ?? null })
}

export function remoteModels(id: number): Promise<RemoteModelDto[]> {
  return api.get<RemoteModelDto[]>(`/providers/${id}/remote-models`)
}

export function listProviderPresets(): Promise<ProviderPresetDto[]> {
  return api.get<ProviderPresetDto[]>("/provider-presets")
}

export function listModelProfiles(): Promise<ModelProfileDto[]> {
  return api.get<ModelProfileDto[]>("/model-profiles")
}

export function createModelProfile(input: ModelProfileInput): Promise<ModelProfileDto> {
  return api.post<ModelProfileDto>("/model-profiles", input)
}

export function patchModelProfile(
  id: number,
  input: ModelProfileInput,
): Promise<ModelProfileDto> {
  return api.patch<ModelProfileDto>(`/model-profiles/${id}`, input)
}

export function deleteModelProfile(id: number): Promise<void> {
  return api.delete<void>(`/model-profiles/${id}`)
}
