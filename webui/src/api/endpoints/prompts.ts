import { getJson, postJson } from "../client"
import type { ProjectId } from "../generated/ProjectId"
import type { PromptSaved } from "../generated/PromptSaved"
import type { PromptsView } from "../generated/PromptsView"
import type { RenderTemplateRequest } from "../generated/RenderTemplateRequest"
import type { RenderedTemplate } from "../generated/RenderedTemplate"
import type { SavePromptRequest } from "../generated/SavePromptRequest"

export const promptsApi = {
  /** Every prompt with the system's overrides and, for a project, its overrides. */
  get: (project: ProjectId | null) =>
    getJson<PromptsView>("/api/prompts", { project: project ?? undefined }),
  saveSystem: (request: SavePromptRequest) =>
    postJson<PromptSaved>("/api/settings/prompts", request),
  saveProject: (project: ProjectId, request: SavePromptRequest) =>
    postJson<PromptSaved>(`/api/projects/${project}/prompts`, request),
  render: (request: RenderTemplateRequest) =>
    postJson<RenderedTemplate>("/api/templates/render", request),
}
