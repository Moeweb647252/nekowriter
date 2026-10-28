/** Writing-skill queries and mutations (spec/11 §4.1, spec/10 §2.9). */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import * as endpoints from "@/api/endpoints/skills"
import { keys } from "@/api/keys"
import type { BookWritingSkillsInput } from "@/api/gen/BookWritingSkillsInput"
import type { WritingSkillInput } from "@/api/gen/WritingSkillInput"
import type { WritingSkillPatch } from "@/api/gen/WritingSkillPatch"

/** The catalogue; `bookId` adds the per-book `selected` flags (design §8). */
export function useWritingSkills(bookId?: number) {
  return useQuery({
    queryKey: keys.skills.catalogue(bookId),
    queryFn: () => endpoints.listWritingSkills(bookId),
  })
}

function useSkillInvalidator() {
  const queryClient = useQueryClient()
  return () => {
    void queryClient.invalidateQueries({ queryKey: keys.skills.all })
  }
}

export function useCreateWritingSkill() {
  const invalidate = useSkillInvalidator()
  return useMutation({
    mutationFn: (input: WritingSkillInput) => endpoints.createWritingSkill(input),
    onSuccess: invalidate,
  })
}

export function usePatchWritingSkill() {
  const invalidate = useSkillInvalidator()
  return useMutation({
    mutationFn: ({ skillId, patch }: { skillId: number; patch: WritingSkillPatch }) =>
      endpoints.patchWritingSkill(skillId, patch),
    onSuccess: invalidate,
  })
}

export function useDeleteWritingSkill() {
  const invalidate = useSkillInvalidator()
  return useMutation({
    mutationFn: (skillId: number) => endpoints.deleteWritingSkill(skillId),
    onSuccess: invalidate,
  })
}

export function useReplaceBookWritingSkills(bookId: number) {
  const invalidate = useSkillInvalidator()
  return useMutation({
    mutationFn: (input: BookWritingSkillsInput) =>
      endpoints.replaceBookWritingSkills(bookId, input),
    onSuccess: invalidate,
  })
}
