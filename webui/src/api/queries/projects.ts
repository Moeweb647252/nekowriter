import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import { useCommandIds } from "../command"
import { projectsApi } from "../endpoints/projects"
import type { ModelProfile } from "../generated/ModelProfile"
import type { ProjectListFilter } from "../generated/ProjectListFilter"
import type { ProjectView } from "../generated/ProjectView"
import type { ThinkingLevel } from "../generated/ThinkingLevel"
import type { VersionId } from "../generated/VersionId"
import type { WritingLanguage } from "../generated/WritingLanguage"
import { queryKeys } from "./keys"

export function useProjects(filter: ProjectListFilter) {
  return useQuery({
    queryKey: queryKeys.projects.list(filter),
    queryFn: () => projectsApi.list({ filter }),
  })
}

/**
 * A project command. The event stream refreshes projects and workspaces for every client;
 * the invalidation here makes this client's own change visible without waiting for it,
 * and after a refusal shows the state that caused it.
 */
function useProjectCommand<Input>(send: (input: Input, commandId: string) => Promise<ProjectView>) {
  const queryClient = useQueryClient()
  const commandIds = useCommandIds()
  return useMutation({
    mutationFn: (input: Input) => send(input, commandIds.idFor(input)),
    onSuccess: () => {
      commandIds.succeeded()
    },
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.projects.all }),
        queryClient.invalidateQueries({ queryKey: queryKeys.workspaces.all }),
      ])
    },
  })
}

export interface NewProjectInput {
  name: string
  novelTitle: string
  writingLanguage: WritingLanguage
}

export function useCreateProject() {
  return useProjectCommand((input: NewProjectInput, commandId) =>
    projectsApi.create({ commandId, ...input }),
  )
}

export function useRenameProject() {
  return useProjectCommand(({ project, name }: { project: ProjectView; name: string }, commandId) =>
    projectsApi.rename(project.id, { commandId, expectedRevision: project.revision, name }),
  )
}

export function useDeleteProject() {
  return useProjectCommand((project: ProjectView, commandId) =>
    projectsApi.delete(project.id, { commandId, expectedRevision: project.revision }),
  )
}

export function useRestoreProject() {
  return useProjectCommand((project: ProjectView, commandId) =>
    projectsApi.restore(project.id, { commandId, expectedRevision: project.revision }),
  )
}

export interface ProjectSettingsInput {
  writingLanguage: WritingLanguage
  model: ModelProfile | null
  thinkingLevel: ThinkingLevel
  auxiliaryBudget: number
}

export function useUpdateProjectSettings() {
  return useProjectCommand(
    ({ project, settings }: { project: ProjectView; settings: ProjectSettingsInput }, commandId) =>
      projectsApi.updateSettings(project.id, {
        commandId,
        expectedRevision: project.revision,
        ...settings,
      }),
  )
}

/** Creates an empty version and selects it. */
export function useCreateVersion() {
  return useProjectCommand(
    ({ project, novelTitle }: { project: ProjectView; novelTitle: string }, commandId) =>
      projectsApi.createVersion(project.id, {
        commandId,
        expectedRevision: project.revision,
        novelTitle,
      }),
  )
}

/** Remembers which version the work shows; it never starts any work. */
export function useSelectVersion() {
  return useProjectCommand(
    ({ project, versionId }: { project: ProjectView; versionId: VersionId }, commandId) =>
      projectsApi.selectVersion(project.id, {
        commandId,
        expectedRevision: project.revision,
        versionId,
      }),
  )
}
