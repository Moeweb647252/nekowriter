/** Book endpoints (spec/10 §2.2). */
import { api } from "@/api/client"
import type { BookDto } from "@/api/gen/BookDto"
import type { BookInput } from "@/api/gen/BookInput"
import type { BookOverviewDto } from "@/api/gen/BookOverviewDto"
import type { BookPatch } from "@/api/gen/BookPatch"
import type { BookSummaryDto } from "@/api/gen/BookSummaryDto"
import type { CreateBookResponse } from "@/api/gen/CreateBookResponse"
import type { KnowledgeCheckInput } from "@/api/gen/KnowledgeCheckInput"
import type { KnowledgeCheckResponse } from "@/api/gen/KnowledgeCheckResponse"

export function listBooks(status?: string): Promise<BookSummaryDto[]> {
  const query = status === undefined ? "" : `?status=${encodeURIComponent(status)}`
  return api.get<BookSummaryDto[]>(`/books${query}`)
}

export function createBook(input: BookInput): Promise<CreateBookResponse> {
  return api.post<CreateBookResponse>("/books", input)
}

export function getBook(bookId: number): Promise<BookDto> {
  return api.get<BookDto>(`/books/${bookId}`)
}

/** `POST /books/:id/knowledge-check` (spec/10 §2.2, D-158). */
export function startKnowledgeCheck(
  bookId: number,
  input: KnowledgeCheckInput,
): Promise<KnowledgeCheckResponse> {
  return api.post<KnowledgeCheckResponse>(`/books/${bookId}/knowledge-check`, input)
}

export function patchBook(bookId: number, patch: BookPatch): Promise<BookDto> {
  return api.patch<BookDto>(`/books/${bookId}`, patch)
}

export function deleteBook(bookId: number): Promise<void> {
  return api.delete<void>(`/books/${bookId}`)
}

export function getOverview(bookId: number): Promise<BookOverviewDto> {
  return api.get<BookOverviewDto>(`/books/${bookId}/overview`)
}
