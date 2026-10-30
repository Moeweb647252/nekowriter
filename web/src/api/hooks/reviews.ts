/** Review, maintenance and proposal queries and mutations (spec/11 §4.1). */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import * as endpoints from "@/api/endpoints/reviews"
import { keys } from "@/api/keys"
import type { ChapterDto } from "@/api/gen/ChapterDto"
import type { SendBackInput } from "@/api/gen/SendBackInput"

/** The chapter's reports and send-backs (spec/10 §2.2). */
export function useReviews(chapterId: number) {
  return useQuery({
    queryKey: keys.chapters.reviews(chapterId),
    queryFn: () => endpoints.listReviews(chapterId),
    enabled: Number.isFinite(chapterId) && chapterId > 0,
  })
}

/** Invalidates everything a decided review operation changes. */
function useReviewSettle(chapterId: number, bookId: number) {
  const queryClient = useQueryClient()
  return () => {
    // The navigator's chapter rows are keyed by their *track*, not by the chapter: passing the
    // chapter id invalidated a key that does not exist. Send-back also changes the chapter's
    // status without a `chapter.updated` event (the session-creation side effect), so this
    // invalidation is what refreshes the navigator.
    const chapter = queryClient.getQueryData<ChapterDto>(
      keys.chapters.detail(chapterId)
    )
    void queryClient.invalidateQueries({
      queryKey: keys.chapters.detail(chapterId),
    })
    void queryClient.invalidateQueries({
      queryKey: keys.chapters.reviews(chapterId),
    })
    void queryClient.invalidateQueries({ queryKey: keys.sessions.list(bookId) })
    void queryClient.invalidateQueries({
      queryKey: keys.books.overview(bookId),
    })
    if (chapter !== undefined) {
      void queryClient.invalidateQueries({
        queryKey: keys.tracks.chapters(chapter.track_id),
      })
    }
  }
}

/** Ask the reviewer: starts or reuses `chapter_review` (spec/08 §1.1). */
export function useStartReview(chapterId: number, bookId: number) {
  const settle = useReviewSettle(chapterId, bookId)
  return useMutation({
    mutationFn: () => endpoints.startReview(chapterId),
    onSuccess: settle,
  })
}

/** Discuss a report: the comment becomes a re-review (spec/03 §6). */
export function useRereview(chapterId: number, bookId: number) {
  const settle = useReviewSettle(chapterId, bookId)
  return useMutation({
    mutationFn: ({
      reportId,
      comment,
    }: {
      reportId: number
      comment: string
    }) => endpoints.rereview(reportId, comment),
    onSuccess: settle,
  })
}

/** Send the chapter back in revise or redraft mode (spec/08 §1.1). */
export function useSendBack(chapterId: number, bookId: number) {
  const settle = useReviewSettle(chapterId, bookId)
  return useMutation({
    mutationFn: (input: SendBackInput) => endpoints.sendBack(chapterId, input),
    onSuccess: settle,
  })
}

/** Retry maintenance (spec/08 §1.1). */
export function useMaintainChapter(chapterId: number, bookId: number) {
  const settle = useReviewSettle(chapterId, bookId)
  return useMutation({
    mutationFn: () => endpoints.maintainChapter(chapterId),
    onSuccess: settle,
  })
}

/** Re-sync canon after an author edit (spec/06 §8). */
export function useResyncChapter(chapterId: number, bookId: number) {
  const settle = useReviewSettle(chapterId, bookId)
  return useMutation({
    mutationFn: () => endpoints.resyncChapter(chapterId),
    onSuccess: settle,
  })
}

/** The book's setting proposals (spec/10 §2.3). */
export function useProposals(bookId: number, status?: string) {
  return useQuery({
    queryKey: keys.proposals.list(bookId, status),
    queryFn: () => endpoints.listProposals(bookId, status),
    enabled: Number.isFinite(bookId) && bookId > 0,
  })
}

/** Accepting can apply a document edit, a character edit or a new character. */
function useProposalSettle(bookId: number) {
  const queryClient = useQueryClient()
  return () => {
    void queryClient.invalidateQueries({ queryKey: keys.proposals.all })
    void queryClient.invalidateQueries({ queryKey: keys.settings.docs(bookId) })
    void queryClient.invalidateQueries({
      queryKey: keys.settings.characters(bookId),
    })
    void queryClient.invalidateQueries({ queryKey: keys.changes.all })
    void queryClient.invalidateQueries({ queryKey: keys.canon.goalsAll })
    void queryClient.invalidateQueries({
      queryKey: keys.books.overview(bookId),
    })
  }
}

/** Accept a proposal: a stale `find` returns `conflict` and applies nothing (D-044). */
export type AcceptProposalVariables =
  | number
  | { proposalId: number; confirmOpenChildren?: boolean }

export function useAcceptProposal(bookId: number) {
  const settle = useProposalSettle(bookId)
  return useMutation({
    mutationFn: (variables: AcceptProposalVariables) =>
      typeof variables === "number"
        ? endpoints.acceptProposal(variables)
        : endpoints.acceptProposal(variables.proposalId, variables.confirmOpenChildren ?? false),
    onSuccess: settle,
  })
}

/** Reject (dismiss) a pending or conflict proposal. */
export function useRejectProposal(bookId: number) {
  const settle = useProposalSettle(bookId)
  return useMutation({
    mutationFn: (proposalId: number) => endpoints.rejectProposal(proposalId),
    onSuccess: settle,
  })
}
