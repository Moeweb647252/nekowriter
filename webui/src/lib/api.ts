export type WorkflowStage =
  | "initializing"
  | "preparing_generating"
  | "preparing_reviewing"
  | "preparing_revising"
  | "writing_chapter_ready"
  | "writing_outline_generating"
  | "writing_outline_reviewing"
  | "writing_drafting"
  | "writing_chapter_reviewing"
  | "writing_chapter_done"
  | "completed"

export type ExecutionStatus =
  | "idle"
  | "queued"
  | "request_in_flight"
  | "tool_boundary"
  | "waiting_answer"
  | "waiting_review"
  | "paused"
  | "interrupted"
  | "failed"
  | "stopped"

export type PreparationArtifactKind =
  "background" | "research" | "characters" | "outline"

export type ReviewStatus =
  "open" | "delegated" | "approved" | "rejected" | "stale"
export type DecimalString = string

export interface Project {
  id: string
  name: string
  selectedVersionId: string
  revision: DecimalString
  createdAtMs: DecimalString
  updatedAtMs: DecimalString
}

export interface NovelVersion {
  id: string
  projectId: string
  label: string
  novelTitle: string
  sourceVersionId: string | null
  bodyCompletedAtMs: DecimalString | null
  revision: DecimalString
  createdAtMs: DecimalString
  updatedAtMs: DecimalString
}

export interface WorkflowSnapshot {
  versionId: string
  stage: WorkflowStage
  execution: ExecutionStatus
  stateRevision: DecimalString
  activeRunId: string | null
  activeChapterId: string | null
  approvalMode: "auto_approve" | "approve_required"
  pauseRequested: boolean
  waitReason: "request_limit" | "protocol_error_limit" | "user_stopped" | null
  resumeExecution: ExecutionStatus | null
}

export interface ProjectBundle {
  project: Project
  version: NovelVersion
  workflow: WorkflowSnapshot
}

export interface ProjectListItem {
  project: Project
  selectedVersion: NovelVersion
  workflow: WorkflowSnapshot
}

export interface VersionSnapshot {
  project: Project
  version: NovelVersion
  workflow: WorkflowSnapshot
  eventWatermark: string
}

export interface PersistedEvent {
  seq: string
  version_id: string | null
  entity_id: string
  revision: string
  payload: unknown
  occurred_at: string
}

export interface HealthResponse {
  status: "ok"
  schemaVersion: number
}

export interface ModelCapabilityProfile {
  streaming: boolean
  tools: boolean
  structuredOutput: boolean
  streamUsage: boolean
  /** 是否已确认模型接受显式思考参数（`reasoning_effort`）。 */
  thinking: boolean
}

export type ThinkingLevel = "off" | "low" | "medium" | "high"

export interface ProjectModelReference {
  modelId: string
  /** 能力资料必须与被引用的模型同源，不能用系统模型的能力代替。 */
  capabilities: ModelCapabilityProfile
}

export interface ProjectSettings {
  /** 覆盖系统默认模型；`null` 表示沿用系统设置。 */
  model: ProjectModelReference | null
  thinkingLevel: ThinkingLevel
  auxiliaryToolBudget: DecimalString
  revision: DecimalString
}

export interface PublicSystemModelSettings {
  apiBaseUrl: string
  modelId: string
  capabilities: ModelCapabilityProfile
  maxContextTokens: DecimalString
  reservedOutputTokens: DecimalString
  revision: DecimalString
  credentialConfigured: boolean
  createdAtMs: DecimalString
  updatedAtMs: DecimalString
}

export interface SystemSettingsResponse {
  writingModel: PublicSystemModelSettings | null
}

export interface ModelUsage {
  inputTokens: DecimalString
  outputTokens: DecimalString
  totalTokens: DecimalString
  cachedInputTokens: DecimalString | null
}

export interface TestModelResponse {
  providerRequestId: string | null
  contentReceived: boolean
  toolCallCount: number
  usage: ModelUsage | null
  /** 本次探针是否真的带上了 `reasoning_effort`。 */
  thinkingProbed: boolean
}

export interface CurrentNovelBrief {
  documentId: string
  revisionId: string | null
  content: string | null
}

export interface PreparationReview {
  id: string
  kind: PreparationArtifactKind
  targetRevisionId: string
  status: ReviewStatus
  passed: boolean | null
  issues: string | null
}

export interface CurrentPreparationArtifact {
  documentId: string
  kind: PreparationArtifactKind
  revisionId: string | null
  revisionNo: DecimalString | null
  content: string | null
  chapterPlan: ChapterPlan | null
  review: PreparationReview | null
}

export interface ChapterPlanItem {
  planItemId: string
  orderIndex: DecimalString
  title: string
  goal: string
}

export interface ChapterPlan {
  items: ChapterPlanItem[]
}

export type ChapterKind = "main" | "extra"
export type ChapterStatus = "planned" | "in_progress" | "approved"

export interface ChapterSummary {
  id: string
  versionId: string
  kind: ChapterKind
  planItemId: string | null
  orderIndex: DecimalString
  title: string
  status: ChapterStatus
  approvedRevisionId: string | null
  revision: DecimalString
}

export interface CurrentChapterGuidance {
  documentId: string
  revisionId: string | null
  revisionNo: DecimalString | null
  content: string | null
}

export interface ChapterOutlineReview {
  id: string
  targetRevisionId: string
  status: ReviewStatus
  passed: boolean | null
  issues: string | null
}

export interface CurrentChapterOutline {
  documentId: string
  revisionId: string | null
  revisionNo: DecimalString | null
  content: string | null
  review: ChapterOutlineReview | null
}

export interface ChapterFinalReview {
  id: string
  targetRevisionId: string
  status: ReviewStatus
  passed: boolean | null
  issues: string | null
}

export interface ReviewDelegationResult {
  reviewId: string
  runId: string
  snapshot: PreparationSnapshot
}

export interface CancelReviewDelegationResult {
  reviewId: string
  runId: string
  cancelledRequestIds: string[]
  snapshot: PreparationSnapshot
}

export interface CurrentChapterBody {
  documentId: string
  revisionId: string | null
  revisionNo: DecimalString | null
  content: string | null
  review: ChapterFinalReview | null
}

export interface TextCitationRange {
  startChar: DecimalString
  endChar: DecimalString
  quote: string
}

export interface DocumentFeedbackCitation extends TextCitationRange {
  kind: "document"
  documentId: string
  revisionId: string
  chapterId: string | null
}

export interface CandidateFeedbackCitation {
  kind: "candidate"
  toolCallId: string
  candidateHash: string
  candidateRange: TextCitationRange
}

export type FeedbackCitation =
  DocumentFeedbackCitation | CandidateFeedbackCitation

export interface PendingChapterBodyProposal {
  toolCallId: string
  providerCallId: string
  toolName: "append_content" | "replace_content" | string
  baseRevisionId: string | null
  baseRevisionNo: DecimalString | null
  candidateContent: string
  candidateHash: string
}

export interface ChapterEditReview {
  id: string
  targetToolCallId: string
  status: ReviewStatus
  passed: boolean | null
  issues: string | null
}

export interface HeldChapterBodyTool {
  toolCallId: string
  providerCallId: string
  toolName: "append_content" | "replace_content" | string
  baseRevisionId: string | null
  baseRevisionNo: DecimalString | null
  candidateContent: string | null
  candidateHash: string | null
}

export interface HeldChapterBodySubmission {
  requestId: string
  summary: string
}

export type FeedbackStatus = "queued" | "bound" | "included"

export interface PendingChapterBodyFeedback {
  id: string
  sequence: DecimalString
  text: string
  citations: FeedbackCitation[]
  status: Exclude<FeedbackStatus, "included">
  boundToolCallId: string | null
  createdAtMs: DecimalString
}

export interface ContextFold {
  artifactRevisionIds: string[]
  throughMessageSequence: DecimalString
  plainText: string
  contentHash: string
}

export type UserQuestionKind = "single" | "multiple" | "text"

export interface UserQuestionOption {
  id: string
  label: string
}

export interface UserQuestion {
  id: string
  title: string
  kind: UserQuestionKind
  options: UserQuestionOption[]
  required: boolean
}

export interface UserQuestionSchema {
  questions: UserQuestion[]
}

export interface UserQuestionAnswer {
  questionId: string
  selectedOptionIds: string[]
  customText: string | null
}

export interface PendingUserQuestion {
  id: string
  toolCallId: string
  schema: UserQuestionSchema
  createdAtMs: DecimalString
}

export interface QuestionAnswerResult {
  questionId: string
  runId: string
  workflow: WorkflowSnapshot
}

export interface PreparationSnapshot {
  versionId: string
  workflow: WorkflowSnapshot
  novelBrief: CurrentNovelBrief
  artifacts: CurrentPreparationArtifact[]
  pendingQuestion: PendingUserQuestion | null
  contextFold: ContextFold | null
}

export interface ChapterOutlineSnapshot {
  chapterId: string
  workflow: WorkflowSnapshot
  guidance: CurrentChapterGuidance
  outline: CurrentChapterOutline
  pendingQuestion: PendingUserQuestion | null
  contextFold: ContextFold | null
}

export interface ChapterBodySnapshot {
  chapterId: string
  workflow: WorkflowSnapshot
  body: CurrentChapterBody
  pendingProposal: PendingChapterBodyProposal | null
  pendingProposalReview: ChapterEditReview | null
  heldTool: HeldChapterBodyTool | null
  heldSubmission: HeldChapterBodySubmission | null
  pendingQuestion: PendingUserQuestion | null
  pendingFeedback: PendingChapterBodyFeedback[]
  outlineContextFold: ContextFold | null
  contextFold: ContextFold | null
}

interface ErrorResponse {
  error?: {
    code?: string
    message?: string
    details?: unknown
    request_id?: string
  }
}

export class ApiError extends Error {
  readonly code: string
  readonly details: unknown
  readonly requestId: string | null

  constructor(
    message: string,
    code: string,
    details: unknown = {},
    requestId: string | null = null
  ) {
    super(message)
    this.name = "ApiError"
    this.code = code
    this.details = details
    this.requestId = requestId
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const requestInit: RequestInit = {
    ...init,
    headers: {
      Accept: "application/json",
      ...init?.headers,
    },
  }
  let response: Response
  try {
    response = await fetch(path, requestInit)
  } catch (error) {
    if (!init?.method || init.method === "GET") throw error
    // 浏览器在服务端已提交响应后也可能丢失连接。以同一请求体再试一次，
    // 因而 command_id 不会改变，后端会重放回执而不是重复写入。
    response = await fetch(path, requestInit)
  }
  if (!response.ok) {
    const payload = (await response.json().catch(() => ({}))) as ErrorResponse
    throw new ApiError(
      payload.error?.message ?? `本地服务返回 ${response.status}`,
      payload.error?.code ?? "HTTP_ERROR",
      payload.error?.details,
      payload.error?.request_id ?? null
    )
  }
  return (await response.json()) as T
}

const retryableCommandIds = new Map<string, string>()

function commandRequest<T>(
  method: "POST" | "PUT" | "DELETE",
  path: string,
  payload: Record<string, unknown>,
  commandId?: string
) {
  const retryKey = JSON.stringify([method, path, payload])
  const id =
    commandId ?? retryableCommandIds.get(retryKey) ?? crypto.randomUUID()
  return request<T>(path, {
    method,
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ commandId: id, ...payload }),
  }).then(
    (response) => {
      retryableCommandIds.delete(retryKey)
      return response
    },
    (error: unknown) => {
      if (error instanceof ApiError) {
        retryableCommandIds.delete(retryKey)
      } else if (!commandId) {
        retryableCommandIds.set(retryKey, id)
      }
      throw error
    }
  )
}

export function createCommandId() {
  return crypto.randomUUID()
}

export function getHealth() {
  return request<HealthResponse>("/api/health")
}

export function getSystemSettings() {
  return request<SystemSettingsResponse>("/api/settings")
}

export function saveSystemModelSettings(
  input: {
    expectedRevision: DecimalString
    apiBaseUrl: string
    modelId: string
    capabilities: ModelCapabilityProfile
    maxContextTokens: DecimalString
    reservedOutputTokens: DecimalString
    apiKey?: string
  },
  commandId: string = createCommandId()
) {
  return request<SystemSettingsResponse>("/api/settings", {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ commandId, ...input }),
  })
}

export function testModelConnection(input: {
  modelId: string
  capabilities: ModelCapabilityProfile
}) {
  return request<TestModelResponse>("/api/settings/test-model", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(input),
  })
}

export function getProjectSettings(projectId: string) {
  return request<ProjectSettings>(`/api/projects/${projectId}/settings`)
}

export function saveProjectSettings(
  projectId: string,
  input: {
    expectedRevision: DecimalString
    model: ProjectModelReference | null
    thinkingLevel: ThinkingLevel
    auxiliaryToolBudget: DecimalString
  },
  commandId: string = createCommandId()
) {
  return commandRequest<ProjectSettings>(
    "PUT",
    `/api/projects/${projectId}/settings`,
    input,
    commandId
  )
}

export function listProjects() {
  return request<ProjectListItem[]>("/api/projects")
}

export function createProject(
  input: {
    name: string
    novelTitle: string
    versionLabel?: string
  },
  commandId?: string
) {
  return commandRequest<ProjectBundle>(
    "POST",
    "/api/projects",
    input,
    commandId
  )
}

export function listVersions(projectId: string) {
  return request<NovelVersion[]>(`/api/projects/${projectId}/versions`)
}

export function createVersion(
  projectId: string,
  input: { label: string; novelTitle: string },
  commandId?: string
) {
  return commandRequest<ProjectBundle>(
    "POST",
    `/api/projects/${projectId}/versions`,
    input,
    commandId
  )
}

export function selectVersion(
  projectId: string,
  versionId: string,
  expectedProjectRevision: DecimalString,
  commandId?: string
) {
  return commandRequest<ProjectBundle>(
    "POST",
    `/api/projects/${projectId}/selected-version`,
    { versionId, expectedProjectRevision },
    commandId
  )
}

export function copyVersion(
  sourceVersionId: string,
  label: string,
  commandId?: string
) {
  return commandRequest<ProjectBundle>(
    "POST",
    `/api/versions/${sourceVersionId}/copy`,
    { label },
    commandId
  )
}

export function getPreparation(versionId: string) {
  return request<PreparationSnapshot>(`/api/versions/${versionId}/preparation`)
}

export function getVersionSnapshot(versionId: string) {
  return request<VersionSnapshot>(`/api/versions/${versionId}/snapshot`)
}

export function setApprovalMode(
  versionId: string,
  expectedStateRevision: DecimalString,
  mode: WorkflowSnapshot["approvalMode"],
  commandId?: string
) {
  return commandRequest<WorkflowSnapshot>(
    "POST",
    `/api/versions/${versionId}/approval-mode`,
    { expectedStateRevision, mode },
    commandId
  )
}

export function getChapters(versionId: string) {
  return request<ChapterSummary[]>(`/api/versions/${versionId}/chapters`)
}

export function advanceToNextChapter(
  versionId: string,
  expectedStateRevision: DecimalString,
  commandId?: string
) {
  return commandRequest<PreparationSnapshot>(
    "POST",
    `/api/versions/${versionId}/chapters/next`,
    { expectedStateRevision },
    commandId
  )
}

export function confirmNovelCompletion(
  versionId: string,
  expectedStateRevision: DecimalString,
  commandId?: string
) {
  return commandRequest<PreparationSnapshot>(
    "POST",
    `/api/versions/${versionId}/completion`,
    { expectedStateRevision },
    commandId
  )
}

export function addExtraChapter(
  versionId: string,
  expectedStateRevision: DecimalString,
  title: string,
  commandId?: string
) {
  return commandRequest<PreparationSnapshot>(
    "POST",
    `/api/versions/${versionId}/extras`,
    { expectedStateRevision, title },
    commandId
  )
}

export function getChapterGuidance(chapterId: string) {
  return request<CurrentChapterGuidance>(`/api/chapters/${chapterId}/guidance`)
}

export function saveChapterGuidance(
  chapterId: string,
  expectedStateRevision: DecimalString,
  content: string,
  commandId?: string
) {
  return commandRequest<CurrentChapterGuidance>(
    "PUT",
    `/api/chapters/${chapterId}/guidance`,
    { expectedStateRevision, content },
    commandId
  )
}

export function startChapterOutline(
  chapterId: string,
  expectedStateRevision: DecimalString,
  commandId?: string
) {
  return commandRequest<WorkflowSnapshot>(
    "POST",
    `/api/chapters/${chapterId}/outline/start`,
    { expectedStateRevision },
    commandId
  )
}

export function getChapterOutline(chapterId: string) {
  return request<ChapterOutlineSnapshot>(`/api/chapters/${chapterId}/outline`)
}

export function submitChapterOutline(
  chapterId: string,
  expectedStateRevision: DecimalString,
  content: string,
  commandId?: string
) {
  return commandRequest<ChapterOutlineSnapshot>(
    "POST",
    `/api/chapters/${chapterId}/outline`,
    { expectedStateRevision, content },
    commandId
  )
}

export function getChapterBody(chapterId: string) {
  return request<ChapterBodySnapshot>(`/api/chapters/${chapterId}/body`)
}

export function decideChapterBodyProposal(
  chapterId: string,
  toolCallId: string,
  expectedStateRevision: DecimalString,
  feedback: string,
  citations: FeedbackCitation[],
  commandId?: string
) {
  return commandRequest<ChapterBodySnapshot>(
    "POST",
    `/api/chapters/${chapterId}/body/proposals/${toolCallId}/decision`,
    { expectedStateRevision, feedback, citations },
    commandId
  )
}

export function submitRunFeedback(
  runId: string,
  expectedStateRevision: DecimalString,
  text: string,
  citations: FeedbackCitation[],
  commandId?: string
) {
  return commandRequest<ChapterBodySnapshot>(
    "POST",
    `/api/runs/${runId}/feedback`,
    { expectedStateRevision, text, citations },
    commandId
  )
}

export function answerQuestion(
  questionId: string,
  expectedStateRevision: DecimalString,
  answers: UserQuestionAnswer[],
  commandId?: string
) {
  return commandRequest<QuestionAnswerResult>(
    "POST",
    `/api/questions/${questionId}/answer`,
    { expectedStateRevision, answers },
    commandId
  )
}

export function continueWritingRun(
  runId: string,
  expectedStateRevision: DecimalString,
  commandId?: string
) {
  return commandRequest<WorkflowSnapshot>(
    "POST",
    `/api/runs/${runId}/continue`,
    { expectedStateRevision },
    commandId
  )
}

export function stopWritingRun(
  runId: string,
  expectedStateRevision: DecimalString,
  commandId?: string
) {
  return commandRequest<WorkflowSnapshot>(
    "POST",
    `/api/runs/${runId}/stop`,
    { expectedStateRevision },
    commandId
  )
}

export function requestChapterBodyPause(
  runId: string,
  expectedStateRevision: DecimalString,
  commandId?: string
) {
  return commandRequest<ChapterBodySnapshot>(
    "POST",
    `/api/runs/${runId}/pause`,
    { expectedStateRevision },
    commandId
  )
}

export function resumeChapterBodyPause(
  runId: string,
  expectedStateRevision: DecimalString,
  commandId?: string
) {
  return commandRequest<ChapterBodySnapshot>(
    "POST",
    `/api/runs/${runId}/resume`,
    { expectedStateRevision },
    commandId
  )
}

export function appendChapterBody(
  chapterId: string,
  expectedStateRevision: DecimalString,
  content: string,
  commandId?: string
) {
  return commandRequest<ChapterBodySnapshot>(
    "POST",
    `/api/chapters/${chapterId}/body/append`,
    { expectedStateRevision, content },
    commandId
  )
}

export function replaceChapterBody(
  chapterId: string,
  expectedStateRevision: DecimalString,
  oldStr: string,
  newStr: string,
  commandId?: string
) {
  return commandRequest<ChapterBodySnapshot>(
    "POST",
    `/api/chapters/${chapterId}/body/replace`,
    { expectedStateRevision, oldStr, newStr },
    commandId
  )
}

export function submitChapterFinalReview(
  chapterId: string,
  expectedStateRevision: DecimalString,
  commandId?: string
) {
  return commandRequest<ChapterBodySnapshot>(
    "POST",
    `/api/chapters/${chapterId}/body/review`,
    { expectedStateRevision },
    commandId
  )
}

export function saveNovelBrief(
  versionId: string,
  content: string,
  commandId?: string
) {
  return commandRequest<CurrentNovelBrief>(
    "PUT",
    `/api/versions/${versionId}/brief`,
    { content },
    commandId
  )
}

export function startPreparation(
  versionId: string,
  expectedStateRevision: DecimalString,
  commandId?: string
) {
  return commandRequest<PreparationSnapshot>(
    "POST",
    `/api/versions/${versionId}/preparation/start`,
    { expectedStateRevision },
    commandId
  )
}

export function submitPreparationArtifacts(
  versionId: string,
  expectedStateRevision: DecimalString,
  artifacts: Array<{ kind: PreparationArtifactKind; content: string }>,
  chapterPlan?: Array<{ title: string; goal: string }>,
  commandId?: string
) {
  return commandRequest<PreparationSnapshot>(
    "POST",
    `/api/versions/${versionId}/preparation/artifacts`,
    { expectedStateRevision, artifacts, chapterPlan },
    commandId
  )
}

export function decidePreparationReview(
  reviewId: string,
  expectedStateRevision: DecimalString,
  passed: boolean,
  issues: string,
  commandId?: string
) {
  return commandRequest<PreparationSnapshot>(
    "POST",
    `/api/reviews/${reviewId}/decision`,
    { expectedStateRevision, passed, issues },
    commandId
  )
}

export function decideChapterOutlineReview(
  reviewId: string,
  expectedStateRevision: DecimalString,
  passed: boolean,
  issues: string,
  commandId?: string
) {
  return commandRequest<PreparationSnapshot>(
    "POST",
    `/api/reviews/${reviewId}/decision`,
    { expectedStateRevision, passed, issues },
    commandId
  )
}

export function decideChapterFinalReview(
  reviewId: string,
  expectedStateRevision: DecimalString,
  passed: boolean,
  issues: string,
  commandId?: string
) {
  return commandRequest<PreparationSnapshot>(
    "POST",
    `/api/reviews/${reviewId}/decision`,
    { expectedStateRevision, passed, issues },
    commandId
  )
}

export function delegateReview(
  reviewId: string,
  expectedStateRevision: DecimalString,
  notes: string,
  commandId?: string
) {
  return commandRequest<ReviewDelegationResult>(
    "POST",
    `/api/reviews/${reviewId}/delegate`,
    { expectedStateRevision, notes },
    commandId
  )
}

export function cancelReviewDelegation(
  reviewId: string,
  expectedStateRevision: DecimalString,
  commandId?: string
) {
  return commandRequest<CancelReviewDelegationResult>(
    "POST",
    `/api/reviews/${reviewId}/cancel-delegation`,
    { expectedStateRevision },
    commandId
  )
}

export type PromptSource = "builtin" | "system" | "project"

export interface PromptVariableDefinition {
  name: string
  description: string
  valueType: string
  required: boolean
  example: string
  source: string
}

export interface PromptOverrideContent {
  system: string
  user: string
  revision: DecimalString
  updatedAtMs: DecimalString
}

export interface PromptCatalogEntry {
  key: string
  variables: PromptVariableDefinition[]
  builtinSystem: string
  builtinUser: string
  systemOverride: PromptOverrideContent | null
  projectOverride: PromptOverrideContent | null
  effectiveSource: PromptSource
  effectiveSystem: string
  effectiveUser: string
}

export interface PromptOverride {
  promptKey: string
  scope: "system" | "project"
  projectId: string | null
  system: string
  user: string
  revision: DecimalString
  createdAtMs: DecimalString
  updatedAtMs: DecimalString
}

export interface PromptOverrideReset {
  promptKey: string
  scope: "system" | "project"
  projectId: string | null
  removed: boolean
}

export interface PreviewPromptResponse {
  promptKey: string
  source: PromptSource
  system: string
  user: string
}

export function getSystemPrompts() {
  return request<PromptCatalogEntry[]>("/api/prompts/system")
}

export function getProjectPrompts(projectId: string) {
  return request<PromptCatalogEntry[]>(`/api/projects/${projectId}/prompts`)
}

export function saveSystemPromptOverride(
  promptKey: string,
  expectedRevision: DecimalString,
  system: string,
  user: string,
  commandId?: string
) {
  return commandRequest<PromptOverride>(
    "PUT",
    `/api/prompts/system/${encodeURIComponent(promptKey)}`,
    { expectedRevision, system, user },
    commandId
  )
}

export function resetSystemPromptOverride(
  promptKey: string,
  expectedRevision: DecimalString,
  commandId?: string
) {
  return commandRequest<PromptOverrideReset>(
    "DELETE",
    `/api/prompts/system/${encodeURIComponent(promptKey)}`,
    { expectedRevision },
    commandId
  )
}

export function saveProjectPromptOverride(
  projectId: string,
  promptKey: string,
  expectedRevision: DecimalString,
  system: string,
  user: string,
  commandId?: string
) {
  return commandRequest<PromptOverride>(
    "PUT",
    `/api/projects/${projectId}/prompts/${encodeURIComponent(promptKey)}`,
    { expectedRevision, system, user },
    commandId
  )
}

export function resetProjectPromptOverride(
  projectId: string,
  promptKey: string,
  expectedRevision: DecimalString,
  commandId?: string
) {
  return commandRequest<PromptOverrideReset>(
    "DELETE",
    `/api/projects/${projectId}/prompts/${encodeURIComponent(promptKey)}`,
    { expectedRevision },
    commandId
  )
}

export interface PreviewPromptDraft {
  promptKey: string
  projectId?: string
  system?: string
  user?: string
}

export function previewPrompt(draft: PreviewPromptDraft) {
  return request<PreviewPromptResponse>("/api/prompts/preview", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(draft),
  })
}
