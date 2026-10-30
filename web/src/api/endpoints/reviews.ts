/** Review, maintenance and proposal endpoints (spec/10 §2.2–§2.3, spec/08 §1.1). */
import { api } from "@/api/client"
import type { ProposalDto } from "@/api/gen/ProposalDto"
import type { ReviewSessionResponse } from "@/api/gen/ReviewSessionResponse"
import type { ReviewsResponse } from "@/api/gen/ReviewsResponse"
import type { SendBackInput } from "@/api/gen/SendBackInput"

/** `GET /chapters/:id/reviews` (spec/10 §2.2). */
export function listReviews(chapterId: number): Promise<ReviewsResponse> {
  return api.get<ReviewsResponse>(`/chapters/${chapterId}/reviews`)
}

/** `POST /chapters/:id/review` — ask the reviewer (spec/08 §1.1). */
export function startReview(chapterId: number): Promise<ReviewSessionResponse> {
  return api.post<ReviewSessionResponse>(`/chapters/${chapterId}/review`)
}

/** `POST /review-reports/:id/rereview` (spec/03 §6). */
export function rereview(reportId: number, comment: string): Promise<ReviewSessionResponse> {
  return api.post<ReviewSessionResponse>(`/review-reports/${reportId}/rereview`, { comment })
}

/** `POST /chapters/:id/send-back` (spec/08 §1.1). */
export function sendBack(
  chapterId: number,
  input: SendBackInput,
): Promise<ReviewSessionResponse> {
  return api.post<ReviewSessionResponse>(`/chapters/${chapterId}/send-back`, input)
}

/** `POST /chapters/:id/maintain` — Retry maintenance (spec/08 §1.1). */
export function maintainChapter(chapterId: number): Promise<ReviewSessionResponse> {
  return api.post<ReviewSessionResponse>(`/chapters/${chapterId}/maintain`)
}

/** `POST /chapters/:id/resync` (spec/06 §8). */
export function resyncChapter(chapterId: number): Promise<ReviewSessionResponse> {
  return api.post<ReviewSessionResponse>(`/chapters/${chapterId}/resync`)
}

/** `GET /books/:id/proposals` (spec/10 §2.3). */
export function listProposals(bookId: number, status?: string): Promise<ProposalDto[]> {
  const query = status === undefined ? "" : `?status=${encodeURIComponent(status)}`
  return api.get<ProposalDto[]>(`/books/${bookId}/proposals${query}`)
}

/** `POST /proposals/:id/accept` (spec/10 §2.3): a stale `find` comes back as `conflict`. */
export function acceptProposal(
  proposalId: number,
  confirmOpenChildren = false,
): Promise<ProposalDto> {
  return api.post<ProposalDto>(`/proposals/${proposalId}/accept`, {
    confirm_open_children: confirmOpenChildren,
  })
}

/** `POST /proposals/:id/reject` (spec/10 §2.3). */
export function rejectProposal(proposalId: number): Promise<ProposalDto> {
  return api.post<ProposalDto>(`/proposals/${proposalId}/reject`)
}
