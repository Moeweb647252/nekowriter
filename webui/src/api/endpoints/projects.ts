import { getJson, postJson } from "../client"
import type { CreateProjectRequest } from "../generated/CreateProjectRequest"
import type { CreateVersionRequest } from "../generated/CreateVersionRequest"
import type { ListProjectsQuery } from "../generated/ListProjectsQuery"
import type { ProjectId } from "../generated/ProjectId"
import type { ProjectView } from "../generated/ProjectView"
import type { RenameProjectRequest } from "../generated/RenameProjectRequest"
import type { RevisionRequest } from "../generated/RevisionRequest"
import type { SelectVersionRequest } from "../generated/SelectVersionRequest"
import type { UpdateProjectSettingsRequest } from "../generated/UpdateProjectSettingsRequest"

export const projectsApi = {
  list: (query: ListProjectsQuery) => getJson<ProjectView[]>("/api/projects", query),
  create: (request: CreateProjectRequest) => postJson<ProjectView>("/api/projects", request),
  rename: (id: ProjectId, request: RenameProjectRequest) =>
    postJson<ProjectView>(`/api/projects/${id}/rename`, request),
  delete: (id: ProjectId, request: RevisionRequest) =>
    postJson<ProjectView>(`/api/projects/${id}/delete`, request),
  restore: (id: ProjectId, request: RevisionRequest) =>
    postJson<ProjectView>(`/api/projects/${id}/restore`, request),
  updateSettings: (id: ProjectId, request: UpdateProjectSettingsRequest) =>
    postJson<ProjectView>(`/api/projects/${id}/settings`, request),
  createVersion: (id: ProjectId, request: CreateVersionRequest) =>
    postJson<ProjectView>(`/api/projects/${id}/versions`, request),
  selectVersion: (id: ProjectId, request: SelectVersionRequest) =>
    postJson<ProjectView>(`/api/projects/${id}/select-version`, request),
}
