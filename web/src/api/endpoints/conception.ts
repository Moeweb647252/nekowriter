/** Conception endpoints (spec/10 §2.2, spec/08 §5). */
import { api } from "@/api/client"
import type { CanonSeedResponse } from "@/api/gen/CanonSeedResponse"
import type { ConceptionFinalizeResponse } from "@/api/gen/ConceptionFinalizeResponse"
import type { ConceptionStartResponse } from "@/api/gen/ConceptionStartResponse"

/** `POST /books/:id/conception/start`: the open session or a new one (D-047). */
export function startConception(bookId: number): Promise<ConceptionStartResponse> {
  return api.post<ConceptionStartResponse>(`/books/${bookId}/conception/start`, {})
}

/** `POST /books/:id/conception/finalize` (spec/08 §5). */
export function finalizeConception(bookId: number): Promise<ConceptionFinalizeResponse> {
  return api.post<ConceptionFinalizeResponse>(`/books/${bookId}/conception/finalize`, {})
}

/** `POST /books/:id/canon/seed`: the open seed session or a new one (D-047). */
export function seedCanon(bookId: number): Promise<CanonSeedResponse> {
  return api.post<CanonSeedResponse>(`/books/${bookId}/canon/seed`, {})
}
