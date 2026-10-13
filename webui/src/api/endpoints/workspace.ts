import { getJson, postJson } from "../client"
import type { DecideReviewRequest } from "../generated/DecideReviewRequest"
import type { ReviewId } from "../generated/ReviewId"
import type { RevisionRequest } from "../generated/RevisionRequest"
import type { RunId } from "../generated/RunId"
import type { SaveBriefRequest } from "../generated/SaveBriefRequest"
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
}
