/** Knowledge-base endpoints (spec/10 §2.5). */
import { api } from "@/api/client"
import type { BookScopesInput } from "@/api/gen/BookScopesInput"
import type { EntryDto } from "@/api/gen/EntryDto"
import type { EntryInput } from "@/api/gen/EntryInput"
import type { EntryPageDto } from "@/api/gen/EntryPageDto"
import type { EntryPatch } from "@/api/gen/EntryPatch"
import type { IndexStatusDto } from "@/api/gen/IndexStatusDto"
import type { KnowledgeHitDto } from "@/api/gen/KnowledgeHitDto"
import type { ScopeDto } from "@/api/gen/ScopeDto"
import type { ScopeInput } from "@/api/gen/ScopeInput"
import type { ScopePatch } from "@/api/gen/ScopePatch"

// ───────────────────────────── scopes ─────────────────────────────

export function listScopes(): Promise<ScopeDto[]> {
  return api.get<ScopeDto[]>("/knowledge/scopes")
}

export function createScope(input: ScopeInput): Promise<ScopeDto> {
  return api.post<ScopeDto>("/knowledge/scopes", input)
}

export function patchScope(scopeId: number, patch: ScopePatch): Promise<ScopeDto> {
  return api.patch<ScopeDto>(`/knowledge/scopes/${scopeId}`, patch)
}

export function deleteScope(scopeId: number): Promise<void> {
  return api.delete<void>(`/knowledge/scopes/${scopeId}`)
}

// ───────────────────────────── entries ─────────────────────────────

export function listEntries(
  scopeId: number,
  params: { q?: string; cursor?: string; limit?: number } = {},
): Promise<EntryPageDto> {
  const query = new URLSearchParams()
  if (params.q !== undefined && params.q !== "") query.set("q", params.q)
  if (params.cursor !== undefined) query.set("cursor", params.cursor)
  if (params.limit !== undefined) query.set("limit", String(params.limit))
  const suffix = query.toString()
  return api.get<EntryPageDto>(
    `/knowledge/scopes/${scopeId}/entries${suffix === "" ? "" : `?${suffix}`}`,
  )
}

export function createEntry(scopeId: number, input: EntryInput): Promise<EntryDto> {
  return api.post<EntryDto>(`/knowledge/scopes/${scopeId}/entries`, input)
}

export function getEntry(entryId: number): Promise<EntryDto> {
  return api.get<EntryDto>(`/knowledge/entries/${entryId}`)
}

export function patchEntry(entryId: number, patch: EntryPatch): Promise<EntryDto> {
  return api.patch<EntryDto>(`/knowledge/entries/${entryId}`, patch)
}

export function deleteEntry(entryId: number): Promise<void> {
  return api.delete<void>(`/knowledge/entries/${entryId}`)
}

// ───────────────────────────── search ─────────────────────────────

export function searchKnowledge(params: {
  q: string
  scopeIds?: number[]
  limit?: number
}): Promise<KnowledgeHitDto[]> {
  const query = new URLSearchParams({ q: params.q })
  if (params.scopeIds !== undefined && params.scopeIds.length > 0) {
    query.set("scope_ids", params.scopeIds.join(","))
  }
  if (params.limit !== undefined) query.set("limit", String(params.limit))
  return api.get<KnowledgeHitDto[]>(`/knowledge/search?${query.toString()}`)
}

// ───────────────────────────── book visibility ─────────────────────────────

export function getBookScopes(bookId: number): Promise<ScopeDto[]> {
  return api.get<ScopeDto[]>(`/books/${bookId}/scopes`)
}

export function putBookScopes(bookId: number, input: BookScopesInput): Promise<ScopeDto[]> {
  return api.put<ScopeDto[]>(`/books/${bookId}/scopes`, input)
}

// ───────────────────────────── embedding ─────────────────────────────

export function getEmbeddingStatus(): Promise<IndexStatusDto> {
  return api.get<IndexStatusDto>("/embedding/status")
}

export function rebuildIndex(): Promise<void> {
  return api.post<void>("/embedding/rebuild")
}
