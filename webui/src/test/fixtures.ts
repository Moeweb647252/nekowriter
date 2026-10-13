import type { ArtifactKind } from "@/api/generated/ArtifactKind"
import type { ArtifactView } from "@/api/generated/ArtifactView"
import type { ChapterView } from "@/api/generated/ChapterView"
import type { ProjectView } from "@/api/generated/ProjectView"
import type { ReviewStatus } from "@/api/generated/ReviewStatus"
import type { ReviewView } from "@/api/generated/ReviewView"
import type { RevisionSummary } from "@/api/generated/RevisionSummary"
import type { RevisionView } from "@/api/generated/RevisionView"
import type { RunView } from "@/api/generated/RunView"
import type { SystemSettingsView } from "@/api/generated/SystemSettingsView"
import type { WorkflowStage } from "@/api/generated/WorkflowStage"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"

export const PROJECT_ID = "0190f5c8-0000-7000-8000-000000000001"
export const VERSION_ID = "0190f5c8-0000-7000-8000-000000000002"
export const RUN_ID = "0190f5c8-0000-7000-8000-000000000003"
const TIME = 1_760_000_000_000

/** The content of every revision the fixtures reference, served by the fake API. */
export const REVISIONS = new Map<string, RevisionView>()

/** A revision summary whose content the fake API serves. */
export function revision(
  id: string,
  content: string,
  extra: Partial<RevisionView> = {},
): RevisionSummary {
  const view: RevisionView = {
    id,
    number: 1,
    content,
    chapterPlan: null,
    wordCount: null,
    createdAt: TIME,
    ...extra,
  }
  REVISIONS.set(id, view)
  return { id, number: view.number, wordCount: view.wordCount, createdAt: TIME }
}

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
  return `0190f5c8-0000-7000-8000-00000000020${String(KINDS.indexOf(kind))}`
}

export function review(id: string, overrides: Partial<ReviewView> = {}): ReviewView {
  return {
    id,
    kind: "chapter_outline",
    status: "open",
    round: 1,
    target: "0190f5c8-0000-7000-8000-000000000999",
    summary: null,
    feedback: null,
    decidedAt: null,
    ...overrides,
  }
}

/** The four artifacts: unwritten, or written once with a review in the given status. */
export function artifacts(
  reviews: Partial<Record<ArtifactKind, ReviewStatus>> = {},
): ArtifactView[] {
  return KINDS.map((kind, index) => {
    const status = reviews[kind]
    if (status === undefined) return { kind, revision: null, review: null }
    const id = `0190f5c8-0000-7000-8000-00000000010${String(index)}`
    return {
      kind,
      revision: revision(id, `The ${kind} text.`),
      review: review(reviewId(kind), { kind: `preparation_${kind}`, status, target: id }),
    }
  })
}

export function chapterView(number: number, overrides: Partial<ChapterView> = {}): ChapterView {
  return {
    id: `0190f5c8-0000-7000-8000-00000000030${String(number)}`,
    kind: "main",
    number,
    title: number === 1 ? "Arrival" : "Storm",
    goal: number === 1 ? "The keeper finds the letter." : "The lamp fails.",
    status: "not_started",
    guidance: null,
    outline: null,
    body: null,
    review: null,
    ...overrides,
  }
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
    completedAt: null,
    run: null,
    pauseRequested: false,
    feedback: [],
    held: null,
    approvalMode: "auto_approve",
    proposal: null,
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
