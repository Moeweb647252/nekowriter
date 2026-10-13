import { keepPreviousData, useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import { useCommandIds } from "../command"
import { workspaceApi } from "../endpoints/workspace"
import type { Answer } from "../generated/Answer"
import type { ApprovalMode } from "../generated/ApprovalMode"
import type { CommandId } from "../generated/CommandId"
import type { ReviewDecisionInput } from "../generated/ReviewDecisionInput"
import type { ReviewId } from "../generated/ReviewId"
import type { RevisionId } from "../generated/RevisionId"
import type { RunId } from "../generated/RunId"
import type { ToolCallId } from "../generated/ToolCallId"
import type { VersionId } from "../generated/VersionId"
import type { WorkspaceAck } from "../generated/WorkspaceAck"
import { queryKeys } from "./keys"

export function useWorkspace(versionId: VersionId) {
  return useQuery({
    queryKey: queryKeys.workspaces.version(versionId),
    queryFn: () => workspaceApi.get(versionId),
  })
}

/**
 * A revision's content, loaded once: revisions never change. With `keepPrevious`, the
 * previous revision stays shown while a newer one of the same document loads.
 */
export function useRevision(
  revisionId: RevisionId | undefined,
  { keepPrevious = false }: { keepPrevious?: boolean } = {},
) {
  return useQuery({
    queryKey: queryKeys.revision(revisionId ?? ""),
    queryFn: () => workspaceApi.revision(revisionId ?? ""),
    enabled: revisionId !== undefined,
    ...(keepPrevious ? { placeholderData: keepPreviousData } : {}),
  })
}

/**
 * A workspace command. Its input carries the workspace revision the user acted on. The
 * event stream refreshes the workspace for every client; the invalidation here shows this
 * client's own change without waiting for it, and after a refusal shows the state that
 * caused it.
 */
function useWorkspaceCommand<Input extends { revision: number }>(
  versionId: VersionId,
  send: (input: Input, commandId: CommandId) => Promise<WorkspaceAck>,
) {
  const queryClient = useQueryClient()
  const commandIds = useCommandIds()
  return useMutation({
    mutationFn: (input: Input) => send(input, commandIds.idFor(input)),
    onSuccess: () => {
      commandIds.succeeded()
    },
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: queryKeys.workspaces.version(versionId) })
    },
  })
}

export function useSaveBrief(versionId: VersionId) {
  return useWorkspaceCommand(
    versionId,
    ({ revision, brief }: { revision: number; brief: string }, commandId) =>
      workspaceApi.saveBrief(versionId, { commandId, expectedRevision: revision, brief }),
  )
}

export function useStartPreparation(versionId: VersionId) {
  return useWorkspaceCommand(versionId, ({ revision }: { revision: number }, commandId) =>
    workspaceApi.startPreparation(versionId, { commandId, expectedRevision: revision }),
  )
}

export function useDecideReview(versionId: VersionId) {
  return useWorkspaceCommand(
    versionId,
    (
      {
        revision,
        reviewId,
        decision,
      }: { revision: number; reviewId: ReviewId; decision: ReviewDecisionInput },
      commandId,
    ) => workspaceApi.decideReview(reviewId, { commandId, expectedRevision: revision, decision }),
  )
}

/** Hands the review to a review agent, with the user's draft feedback as notes. */
export function useDelegateReview(versionId: VersionId) {
  return useWorkspaceCommand(
    versionId,
    (
      { revision, reviewId, notes }: { revision: number; reviewId: ReviewId; notes: string },
      commandId,
    ) =>
      workspaceApi.delegateReview(reviewId, {
        commandId,
        expectedRevision: revision,
        ...(notes.trim() === "" ? {} : { notes }),
      }),
  )
}

export function useContinueRun(versionId: VersionId) {
  return useWorkspaceCommand(
    versionId,
    ({ revision, runId }: { revision: number; runId: RunId }, commandId) =>
      workspaceApi.continueRun(runId, { commandId, expectedRevision: revision }),
  )
}

/** Saves the chapter's guidance and starts its outline. */
export function useStartChapter(versionId: VersionId) {
  return useWorkspaceCommand(
    versionId,
    ({ revision, guidance }: { revision: number; guidance: string }, commandId) =>
      workspaceApi.startChapter(versionId, { commandId, expectedRevision: revision, guidance }),
  )
}

export function useNextChapter(versionId: VersionId) {
  return useWorkspaceCommand(versionId, ({ revision }: { revision: number }, commandId) =>
    workspaceApi.nextChapter(versionId, { commandId, expectedRevision: revision }),
  )
}

/** Confirms the novel complete, or finishes an extra chapter. */
export function useCompleteVersion(versionId: VersionId) {
  return useWorkspaceCommand(versionId, ({ revision }: { revision: number }, commandId) =>
    workspaceApi.complete(versionId, { commandId, expectedRevision: revision }),
  )
}

export function useAddExtraChapter(versionId: VersionId) {
  return useWorkspaceCommand(
    versionId,
    ({ revision, title }: { revision: number; title: string }, commandId) =>
      workspaceApi.addExtraChapter(versionId, { commandId, expectedRevision: revision, title }),
  )
}

/**
 * A control command (`docs/product-spec.md` §16.3): it quotes no workspace revision, since
 * the agent keeps advancing the workspace while the user acts.
 */
function useControlCommand<Input>(
  versionId: VersionId,
  send: (input: Input, commandId: CommandId) => Promise<WorkspaceAck>,
) {
  const queryClient = useQueryClient()
  const commandIds = useCommandIds()
  return useMutation({
    mutationFn: (input: Input) => send(input, commandIds.idFor(input)),
    onSuccess: () => {
      commandIds.succeeded()
    },
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: queryKeys.workspaces.version(versionId) })
    },
  })
}

export function useStopRun(versionId: VersionId) {
  return useControlCommand(versionId, (runId: RunId, commandId) =>
    workspaceApi.stopRun(runId, { commandId }),
  )
}

/** Gives the writing run feedback; while it is paused, this also continues it. */
export function useSubmitFeedback(versionId: VersionId) {
  return useControlCommand(
    versionId,
    ({ runId, text }: { runId: RunId; text: string }, commandId) =>
      workspaceApi.submitFeedback(runId, { commandId, text }),
  )
}

export function usePauseRun(versionId: VersionId) {
  return useControlCommand(versionId, (runId: RunId, commandId) =>
    workspaceApi.pauseRun(runId, { commandId }),
  )
}

export function useCancelPause(versionId: VersionId) {
  return useControlCommand(versionId, (runId: RunId, commandId) =>
    workspaceApi.cancelPause(runId, { commandId }),
  )
}

export function useResumeRun(versionId: VersionId) {
  return useControlCommand(versionId, (runId: RunId, commandId) =>
    workspaceApi.resumeRun(runId, { commandId }),
  )
}

/** Switches between applying edits at once and reviewing each (§9.6). */
export function useSetApprovalMode(versionId: VersionId) {
  return useControlCommand(versionId, (mode: ApprovalMode, commandId) =>
    workspaceApi.setApprovalMode(versionId, { commandId, mode }),
  )
}

/** Answers the agent's questions; the run goes on with the answers. */
export function useAnswerQuestions(versionId: VersionId) {
  return useControlCommand(
    versionId,
    ({ callId, answers }: { callId: ToolCallId; answers: Answer[] }, commandId) =>
      workspaceApi.answerQuestions(callId, { commandId, answers }),
  )
}

/** Takes a delegated review back from the review agent. */
export function useCancelDelegation(versionId: VersionId) {
  return useControlCommand(versionId, (reviewId: ReviewId, commandId) =>
    workspaceApi.cancelDelegation(reviewId, { commandId }),
  )
}
