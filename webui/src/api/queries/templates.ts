import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import { useCommandIds } from "../command"
import { templatesApi } from "../endpoints/templates"
import type { CommandId } from "../generated/CommandId"
import type { PromptTemplateView } from "../generated/PromptTemplateView"
import type { TemplateVariable } from "../generated/TemplateVariable"
import { queryKeys } from "./keys"

export function useTemplates() {
  return useQuery({ queryKey: queryKeys.templates, queryFn: templatesApi.list })
}

export interface TemplateInput {
  title: string
  body: string
  variables: TemplateVariable[]
}

/** A template command; it refreshes the list whether it succeeds or not. */
function useTemplateCommand<Input>(
  send: (input: Input, commandId: CommandId) => Promise<PromptTemplateView>,
) {
  const queryClient = useQueryClient()
  const commandIds = useCommandIds()
  return useMutation({
    mutationFn: (input: Input) => send(input, commandIds.idFor(input)),
    onSuccess: () => commandIds.succeeded(),
    onSettled: () => queryClient.invalidateQueries({ queryKey: queryKeys.templates }),
  })
}

export function useCreateTemplate() {
  return useTemplateCommand((input: TemplateInput, commandId) =>
    templatesApi.create({ commandId, ...input }),
  )
}

export function useUpdateTemplate() {
  return useTemplateCommand(
    ({ template, ...input }: TemplateInput & { template: PromptTemplateView }, commandId) =>
      templatesApi.update(template.id, {
        commandId,
        expectedRevision: template.revision,
        ...input,
      }),
  )
}

export function useDeleteTemplate() {
  return useTemplateCommand((template: PromptTemplateView, commandId) =>
    templatesApi.delete(template.id, { commandId, expectedRevision: template.revision }),
  )
}
