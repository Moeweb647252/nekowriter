/**
 * The one held-call decision, wired to the resume mutation (PROJECT §8.6, spec/11 §5.2/§5.3, D-141).
 *
 * Both surfaces decide a held call: the agent panel's `HeldCallReview` and the draft pane's
 * `PendingWriteDock`. They must not drift in *behaviour*, so neither builds a payload — they call
 * this hook, which owns the two documented mutations and the busy flag. Only the wording differs
 * (the panel names the tool, the dock speaks about the draft).
 *
 * `resume` is passed in rather than created here so each surface keeps its own `bookId` for cache
 * invalidation. It is the `useResumeSession(bookId)` result, narrowed to what this hook uses.
 */
import { approveHeldInput, requestChangesInput, type ResumeVariables } from "@/features/agent/heldCall"

/** The part of a `useMutation` result this hook uses. */
export type HeldResumeMutation = {
  mutate: (
    variables: ResumeVariables,
    options?: { onError?: (error: unknown) => void },
  ) => void
  isPending: boolean
}

export type HeldDecision = {
  /**
   * Approve: `resume` with no feedback, which finalizes the held call with its deferred effects —
   * for a staged chapter write that is the transaction that applies `pending_content` (D-139).
   */
  approve: () => void
  /**
   * Request changes: `resume` with exactly this feedback, which revokes those effects. A blank or
   * whitespace-only argument does nothing at all — the server reads it as "no feedback" and would
   * *approve* the call instead (spec/03 §5.4).
   */
  requestChanges: (feedback: string) => void
  /** True while a resume is in flight. */
  busy: boolean
}

/**
 * The two decisions for one held call.
 *
 * `sessionId` is `null` when there is nothing to decide, which makes both actions no-ops instead of
 * resuming a session the author did not mean.
 */
export function useHeldDecision(
  sessionId: number | null,
  resume: HeldResumeMutation,
  onError: (error: unknown) => void,
): HeldDecision {
  return {
    approve: () => {
      if (sessionId === null) {
        return
      }
      resume.mutate(approveHeldInput(sessionId), { onError })
    },
    requestChanges: (feedback: string) => {
      if (sessionId === null) {
        return
      }
      const input = requestChangesInput(sessionId, feedback)
      if (input === undefined) {
        return
      }
      resume.mutate(input, { onError })
    },
    busy: resume.isPending,
  }
}
