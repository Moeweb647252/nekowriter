/** Prompt registry endpoints (spec/10 §2.6). */
import { api } from "@/api/client"
import type { PromptEntryDto } from "@/api/gen/PromptEntryDto"
import type { PromptSaveInput } from "@/api/gen/PromptSaveInput"
import type { PromptScope } from "@/api/gen/PromptScope"

export function listPrompts(bookId?: number): Promise<PromptEntryDto[]> {
  const query = bookId === undefined ? "" : `?book_id=${bookId}`
  return api.get<PromptEntryDto[]>(`/prompts${query}`)
}

export function savePrompt(key: string, input: PromptSaveInput): Promise<PromptEntryDto> {
  return api.put<PromptEntryDto>(`/prompts/${encodeURIComponent(key)}`, input)
}

export function resetPrompt(
  key: string,
  scope: PromptScope,
  bookId?: number,
): Promise<PromptEntryDto> {
  const query = new URLSearchParams({ scope })
  if (bookId !== undefined) {
    query.set("book_id", String(bookId))
  }
  return api.delete<PromptEntryDto>(`/prompts/${encodeURIComponent(key)}?${query.toString()}`)
}
