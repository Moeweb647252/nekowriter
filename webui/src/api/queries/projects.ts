import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import { useCommandIds } from "../command"
import { projectsApi } from "../endpoints/projects"
import type { ProjectListFilter } from "../generated/ProjectListFilter"
import type { ProjectView } from "../generated/ProjectView"
import type { WritingLanguage } from "../generated/WritingLanguage"
import { queryKeys } from "./keys"

export function useProjects(filter: ProjectListFilter) {
  return useQuery({
    queryKey: queryKeys.projects.list(filter),
    queryFn: () => projectsApi.list({ filter }),
  })
}

/**
 * A project command. The event stream refreshes project lists for every client; the
 * invalidation here makes this client's own change visible without waiting for it.
 */
function useProjectCommand<Input>(send: (input: Input, commandId: string) => Promise<ProjectView>) {
  const queryClient = useQueryClient()
  const commandIds = useCommandIds()
  return useMutation({
    mutationFn: (input: Input) => send(input, commandIds.idFor(input)),
    onSuccess: async () => {
      commandIds.succeeded()
      await queryClient.invalidateQueries({ queryKey: queryKeys.projects.all })
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
