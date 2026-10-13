import { getJson, postJson } from "../client"
import type { CreateProjectRequest } from "../generated/CreateProjectRequest"
import type { ListProjectsQuery } from "../generated/ListProjectsQuery"
import type { ProjectId } from "../generated/ProjectId"
import type { ProjectRevisionRequest } from "../generated/ProjectRevisionRequest"
import type { ProjectView } from "../generated/ProjectView"
import type { RenameProjectRequest } from "../generated/RenameProjectRequest"

export const projectsApi = {
  list: (query: ListProjectsQuery) => getJson<ProjectView[]>("/api/projects", query),
  create: (request: CreateProjectRequest) => postJson<ProjectView>("/api/projects", request),
  rename: (id: ProjectId, request: RenameProjectRequest) =>
    postJson<ProjectView>(`/api/projects/${id}/rename`, request),
  delete: (id: ProjectId, request: ProjectRevisionRequest) =>
    postJson<ProjectView>(`/api/projects/${id}/delete`, request),
  restore: (id: ProjectId, request: ProjectRevisionRequest) =>
    postJson<ProjectView>(`/api/projects/${id}/restore`, request),
}
