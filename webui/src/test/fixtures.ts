import type { ArtifactKind } from "@/api/generated/ArtifactKind"
import type { ArtifactView } from "@/api/generated/ArtifactView"
import type { ProjectView } from "@/api/generated/ProjectView"
import type { ReviewStatus } from "@/api/generated/ReviewStatus"
import type { RunView } from "@/api/generated/RunView"
import type { SystemSettingsView } from "@/api/generated/SystemSettingsView"
import type { WorkflowStage } from "@/api/generated/WorkflowStage"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"

export const PROJECT_ID = "0190f5c8-0000-7000-8000-000000000001"
export const VERSION_ID = "0190f5c8-0000-7000-8000-000000000002"
export const RUN_ID = "0190f5c8-0000-7000-8000-000000000003"
const TIME = 1_760_000_000_000

export function projectView(overrides: Partial<ProjectView> = {}): ProjectView {
  return {
    id: PROJECT_ID,
    name: "Rain",
    writingLanguage: "en",
    model: null,
    thinkingLevel: "off",
    selectedVersion: {
      id: VERSION_ID,
      number: 1,
      novelTitle: "Rain Letters",
      stage: { phase: "preparation", step: "not_started" },
    },
    revision: 1,
    createdAt: TIME,
    updatedAt: TIME,
    deletedAt: null,
    ...overrides,
  }
}

const KINDS: ArtifactKind[] = ["background", "research", "characters", "outline"]

/** The review of `kind` in the artifacts below. */
export function reviewId(kind: ArtifactKind): string {
  return `0190f5c8-0000-7000-8000-00000000020${KINDS.indexOf(kind)}`
}

/** The four artifacts: unwritten, or written once with a review in the given status. */
export function artifacts(
  reviews: Partial<Record<ArtifactKind, ReviewStatus>> = {},
): ArtifactView[] {
  return KINDS.map((kind, index) => {
    const status = reviews[kind]
    return {
      kind,
      revision:
        status === undefined
          ? null
          : {
              id: `0190f5c8-0000-7000-8000-00000000010${index}`,
              number: 1,
              content: `The ${kind} text.`,
              createdAt: TIME,
            },
      chapterPlan: null,
      review:
        status === undefined
          ? null
          : { id: reviewId(kind), status, round: 1, feedback: null, decidedAt: null },
    }
  })
}

export function runView(overrides: Partial<RunView> = {}): RunView {
  return {
    id: RUN_ID,
    kind: "preparation",
    status: "running",
    halt: null,
    requestCount: 1,
    requestAllowanceEnd: 100,
    consecutiveProtocolErrors: 0,
    requestInFlight: null,
    ...overrides,
  }
}

export function workspaceView(
  stage: WorkflowStage,
  overrides: Partial<WorkspaceView> = {},
): WorkspaceView {
  const project = projectView()
  const version = { ...project.selectedVersion, stage }
  return {
    project: { ...project, selectedVersion: version },
    version,
    versions: [version],
    revision: 3,
    run: null,
    brief: null,
    artifacts: artifacts(),
    chapters: [],
    ...overrides,
  }
}

export function systemSettings(overrides: Partial<SystemSettingsView> = {}): SystemSettingsView {
  return {
    apiBaseUrl: null,
    apiKeySet: false,
    defaultModel: null,
    revision: 0,
    ...overrides,
  }
}
