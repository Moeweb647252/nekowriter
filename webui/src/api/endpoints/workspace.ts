import { getJson, postJson } from "../client"
import type { AddExtraChapterRequest } from "../generated/AddExtraChapterRequest"
import type { AnswerRequest } from "../generated/AnswerRequest"
import type { ApprovalModeRequest } from "../generated/ApprovalModeRequest"
import type { ControlRequest } from "../generated/ControlRequest"
import type { DecideReviewRequest } from "../generated/DecideReviewRequest"
import type { FeedbackRequest } from "../generated/FeedbackRequest"
import type { ReviewId } from "../generated/ReviewId"
import type { RevisionId } from "../generated/RevisionId"
import type { RevisionRequest } from "../generated/RevisionRequest"
import type { RevisionView } from "../generated/RevisionView"
import type { RunId } from "../generated/RunId"
import type { SaveBriefRequest } from "../generated/SaveBriefRequest"
import type { ToolCallId } from "../generated/ToolCallId"
import type { StartChapterRequest } from "../generated/StartChapterRequest"
import type { VersionId } from "../generated/VersionId"
import type { WorkspaceAck } from "../generated/WorkspaceAck"
import type { WorkspaceView } from "../generated/WorkspaceView"

export const workspaceApi = {
  get: (versionId: VersionId) => getJson<WorkspaceView>(`/api/versions/${versionId}/workspace`),
  saveBrief: (versionId: VersionId, request: SaveBriefRequest) =>
    postJson<WorkspaceAck>(`/api/versions/${versionId}/brief`, request),
  startPreparation: (versionId: VersionId, request: RevisionRequest) =>
    postJson<WorkspaceAck>(`/api/versions/${versionId}/preparation/start`, request),
  decideReview: (reviewId: ReviewId, request: DecideReviewRequest) =>
    postJson<WorkspaceAck>(`/api/reviews/${reviewId}/decide`, request),
  continueRun: (runId: RunId, request: RevisionRequest) =>
    postJson<WorkspaceAck>(`/api/runs/${runId}/continue`, request),
  stopRun: (runId: RunId, request: ControlRequest) =>
    postJson<WorkspaceAck>(`/api/runs/${runId}/stop`, request),
  submitFeedback: (runId: RunId, request: FeedbackRequest) =>
    postJson<WorkspaceAck>(`/api/runs/${runId}/feedback`, request),
  pauseRun: (runId: RunId, request: ControlRequest) =>
    postJson<WorkspaceAck>(`/api/runs/${runId}/pause`, request),
  cancelPause: (runId: RunId, request: ControlRequest) =>
    postJson<WorkspaceAck>(`/api/runs/${runId}/cancel-pause`, request),
  resumeRun: (runId: RunId, request: ControlRequest) =>
    postJson<WorkspaceAck>(`/api/runs/${runId}/resume`, request),
  answerQuestions: (callId: ToolCallId, request: AnswerRequest) =>
    postJson<WorkspaceAck>(`/api/questions/${callId}/answer`, request),
  setApprovalMode: (versionId: VersionId, request: ApprovalModeRequest) =>
    postJson<WorkspaceAck>(`/api/versions/${versionId}/approval-mode`, request),
  revision: (revisionId: RevisionId) => getJson<RevisionView>(`/api/revisions/${revisionId}`),
  startChapter: (versionId: VersionId, request: StartChapterRequest) =>
    postJson<WorkspaceAck>(`/api/versions/${versionId}/chapters/start`, request),
  nextChapter: (versionId: VersionId, request: RevisionRequest) =>
    postJson<WorkspaceAck>(`/api/versions/${versionId}/chapters/next`, request),
  complete: (versionId: VersionId, request: RevisionRequest) =>
    postJson<WorkspaceAck>(`/api/versions/${versionId}/complete`, request),
  addExtraChapter: (versionId: VersionId, request: AddExtraChapterRequest) =>
    postJson<WorkspaceAck>(`/api/versions/${versionId}/extra-chapters`, request),
}
