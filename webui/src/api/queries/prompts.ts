import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import { useCommandIds } from "../command"
import { promptsApi } from "../endpoints/prompts"
import type { ProjectId } from "../generated/ProjectId"
import type { PromptKey } from "../generated/PromptKey"
import type { PromptRole } from "../generated/PromptRole"
import type { RenderTemplateRequest } from "../generated/RenderTemplateRequest"
import { queryKeys } from "./keys"

export function usePrompts(project: ProjectId | null) {
  return useQuery({
    queryKey: queryKeys.prompts.scope(project),
    queryFn: () => promptsApi.get(project),
  })
}

export interface SavePromptInput {
  key: PromptKey
  role: PromptRole
  /** `null` deletes the override, so the next level applies. */
  content: string | null
  revision: number
}

/** Saves an override of the system's prompts, or of `project`'s. */
export function useSavePrompt(project: ProjectId | null) {
  const queryClient = useQueryClient()
  const commandIds = useCommandIds()
  return useMutation({
    mutationFn: ({ key, role, content, revision }: SavePromptInput) => {
      const input = { key, role, content, expectedRevision: revision }
      const request = { commandId: commandIds.idFor({ project, ...input }), ...input }
      return project === null
        ? promptsApi.saveSystem(request)
        : promptsApi.saveProject(project, request)
    },
    onSuccess: () => commandIds.succeeded(),
    // Refreshes after a refusal too, which may come from an outdated revision.
    onSettled: () => queryClient.invalidateQueries({ queryKey: queryKeys.prompts.all }),
  })
}

/** Renders a template with values, as the agents would. */
export function useRenderTemplate() {
  return useMutation({ mutationFn: (request: RenderTemplateRequest) => promptsApi.render(request) })
}
