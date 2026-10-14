import { getJson, postJson } from "../client"
import type { CreateTemplateRequest } from "../generated/CreateTemplateRequest"
import type { PromptTemplateId } from "../generated/PromptTemplateId"
import type { PromptTemplateView } from "../generated/PromptTemplateView"
import type { RevisionRequest } from "../generated/RevisionRequest"
import type { UpdateTemplateRequest } from "../generated/UpdateTemplateRequest"

export const templatesApi = {
  list: () => getJson<PromptTemplateView[]>("/api/templates"),
  create: (request: CreateTemplateRequest) =>
    postJson<PromptTemplateView>("/api/templates", request),
  update: (id: PromptTemplateId, request: UpdateTemplateRequest) =>
    postJson<PromptTemplateView>(`/api/templates/${id}/update`, request),
  delete: (id: PromptTemplateId, request: RevisionRequest) =>
    postJson<PromptTemplateView>(`/api/templates/${id}/delete`, request),
}
