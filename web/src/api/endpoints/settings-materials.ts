/** Setting material endpoints (spec/10 §2.3). */
import { api } from "@/api/client"
import type { ChangeDto } from "@/api/gen/ChangeDto"
import type { CharacterDto } from "@/api/gen/CharacterDto"
import type { CharacterInput } from "@/api/gen/CharacterInput"
import type { CharacterPatch } from "@/api/gen/CharacterPatch"
import type { SettingDocDto } from "@/api/gen/SettingDocDto"
import type { SettingDocInput } from "@/api/gen/SettingDocInput"

export function listSettingDocs(bookId: number): Promise<SettingDocDto[]> {
  return api.get<SettingDocDto[]>(`/books/${bookId}/setting-docs`)
}

export function putSettingDoc(
  bookId: number,
  kind: string,
  input: SettingDocInput,
): Promise<SettingDocDto> {
  return api.put<SettingDocDto>(`/books/${bookId}/setting-docs/${kind}`, input)
}

export function settingDocHistory(bookId: number, kind: string): Promise<ChangeDto[]> {
  return api.get<ChangeDto[]>(`/books/${bookId}/setting-docs/${kind}/history`)
}

export function listCharacters(bookId: number): Promise<CharacterDto[]> {
  return api.get<CharacterDto[]>(`/books/${bookId}/characters`)
}

export function createCharacter(
  bookId: number,
  input: CharacterInput,
): Promise<CharacterDto> {
  return api.post<CharacterDto>(`/books/${bookId}/characters`, input)
}

export function patchCharacter(
  characterId: number,
  patch: CharacterPatch,
): Promise<CharacterDto> {
  return api.patch<CharacterDto>(`/characters/${characterId}`, patch)
}

export function deleteCharacter(characterId: number): Promise<void> {
  return api.delete<void>(`/characters/${characterId}`)
}

export function reorderCharacters(bookId: number, ids: number[]): Promise<CharacterDto[]> {
  return api.post<CharacterDto[]>(`/books/${bookId}/characters/reorder`, { ids })
}
