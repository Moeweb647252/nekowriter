/** Writing-skill endpoints (spec/10 §2.9, design §8). */
import { api } from "@/api/client"
import type { BookWritingSkillsInput } from "@/api/gen/BookWritingSkillsInput"
import type { WritingSkillDeleteResponse } from "@/api/gen/WritingSkillDeleteResponse"
import type { WritingSkillDto } from "@/api/gen/WritingSkillDto"
import type { WritingSkillInput } from "@/api/gen/WritingSkillInput"
import type { WritingSkillPatch } from "@/api/gen/WritingSkillPatch"

export function listWritingSkills(bookId?: number): Promise<WritingSkillDto[]> {
  const query = bookId === undefined ? "" : `?book_id=${bookId}`
  return api.get<WritingSkillDto[]>(`/writing-skills${query}`)
}

export function createWritingSkill(input: WritingSkillInput): Promise<WritingSkillDto> {
  return api.post<WritingSkillDto>("/writing-skills", input)
}

export function patchWritingSkill(
  skillId: number,
  patch: WritingSkillPatch,
): Promise<WritingSkillDto> {
  return api.patch<WritingSkillDto>(`/writing-skills/${skillId}`, patch)
}

export function deleteWritingSkill(skillId: number): Promise<WritingSkillDeleteResponse> {
  return api.delete<WritingSkillDeleteResponse>(`/writing-skills/${skillId}`)
}

/** Replaces the book's whole selection (design §8: `PUT /books/:id/writing-skills`). */
export function replaceBookWritingSkills(
  bookId: number,
  input: BookWritingSkillsInput,
): Promise<WritingSkillDto[]> {
  return api.put<WritingSkillDto[]>(`/books/${bookId}/writing-skills`, input)
}
