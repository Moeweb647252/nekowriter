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

/** 不落库、不可恢复的模型流式预览片段。 */
export interface PreviewTool {
  index: number
  id?: string
  name?: string
}

export interface PreviewDelta {
  versionId: string
  requestId: string
  tool: PreviewTool | null
  /** Unicode scalar 偏移，由字符串传输以避免 JavaScript 数值精度丢失。 */
  offset: string
  content: string
}

export interface AgentEngineDescriptor {
  id: string
  displayName: string
}

export interface AgentCapabilities {
  modelAdapters: string[]
  researchEngines: AgentEngineDescriptor[]
  researchUnavailableReason: string | null
  knowledgeAccumulationEngines: AgentEngineDescriptor[]
  knowledgeAccumulationUnavailableReason: string | null
  builtinPromptCount: number
}

export interface HealthResponse {
  status: "ok"
  schemaVersion: number
  /** 本地服务实际使用的数据目录；数据库、向量索引与日志都在它下面。 */
  dataDir: string
}

export interface ShutdownResponse {
  status: "shutting_down" | "already_shutting_down"
}

export interface ModelCapabilityProfile {
  streaming: boolean
  /** 每个阶段的终态提交都是一次提交工具调用，因此写作与审核都要求这个能力。 */
  tools: boolean
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
  /** 服务端给出的显示名，与阅读区和导出使用同一套命名。 */
  label: string
  /** 当前已提交正文的字数；阅读区用它估计每章高度。 */
  bodyWordCount: DecimalString
  status: ChapterStatus
  approvedRevisionId: string | null
  revision: DecimalString
}

export interface ManuscriptChapter {
  chapterId: string
  label: string
  revisionId: string | null
  draft: boolean
  content: string
}

export interface ManuscriptWindow {
  totalChapters: number
  offset: number
  chapters: ManuscriptChapter[]
}

export interface ManuscriptMatch {
  chapterId: string
  revisionId: string
  startChar: DecimalString
  endChar: DecimalString
  snippet: string
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

export interface DocumentRevisionView {
  documentId: string
  revisionId: string
  revisionNo: DecimalString
  role: string
  chapterId: string | null
  wordCount: DecimalString
  content: string
  isCurrent: boolean
  createdAtMs: DecimalString
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

export type FeedbackDeliveryStatus = "sent" | "failed" | "unknown" | "cancelled"

export interface DeliveredChapterBodyFeedback {
  id: string
  sequence: DecimalString
  text: string
  citations: FeedbackCitation[]
  requestId: string
  deliveryStatus: FeedbackDeliveryStatus
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
  deliveredFeedback: DeliveredChapterBodyFeedback[]
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
  if (!response.ok) throw await toApiError(response)
  return (await response.json()) as T
}

/** 把一份失败的响应变成界面可以直接展示的错误。 */
async function toApiError(response: Response) {
  const payload = (await response.json().catch(() => ({}))) as ErrorResponse
  return new ApiError(
    payload.error?.message ?? `本地服务返回 ${response.status}`,
    payload.error?.code ?? "HTTP_ERROR",
    payload.error?.details,
    payload.error?.request_id ?? null
  )
}

const retryableCommandIds = new Map<string, string>()

function commandRequest<T>(
  method: "POST" | "PUT" | "PATCH" | "DELETE",
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

/**
 * 请求本地服务退出。响应先写回，服务随后停止接受连接；重复调用不会重复收尾。
 */
export function shutdownService() {
  return request<ShutdownResponse>("/api/system/shutdown", { method: "POST" })
}

/** 本地能力目录：已注册引擎与它们不可用时的原因。 */
export function getCapabilities() {
  return request<AgentCapabilities>("/api/capabilities")
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

/**
 * 阅读窗口：一次只取 `limit` 章已提交正文，因此打开几百章的作品只需要读要显示的
 * 那几章。章节顺序与显示名与目录、导出完全一致。
 */
export function getManuscriptWindow(
  versionId: string,
  offset: number,
  limit: number
) {
  return request<ManuscriptWindow>(
    `/api/versions/${versionId}/manuscript?offset=${offset}&limit=${limit}`
  )
}

/**
 * 整稿查找：服务端读遍全部已提交正文，返回与引用同一单位（Unicode scalar，
 * 左闭右开）的偏移，可直接用于定位与高亮。
 */
export function searchManuscript(versionId: string, query: string) {
  return request<ManuscriptMatch[]>(
    `/api/versions/${versionId}/manuscript/search?q=${encodeURIComponent(query)}`
  )
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

export function getDocumentRevision(documentId: string, revisionId: string) {
  return request<DocumentRevisionView>(
    `/api/documents/${documentId}/revisions/${revisionId}`
  )
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
  citations: FeedbackCitation[] = [],
  commandId?: string
) {
  return commandRequest<PreparationSnapshot>(
    "POST",
    `/api/reviews/${reviewId}/decision`,
    { expectedStateRevision, passed, issues, citations },
    commandId
  )
}

export function decideChapterOutlineReview(
  reviewId: string,
  expectedStateRevision: DecimalString,
  passed: boolean,
  issues: string,
  citations: FeedbackCitation[] = [],
  commandId?: string
) {
  return commandRequest<PreparationSnapshot>(
    "POST",
    `/api/reviews/${reviewId}/decision`,
    { expectedStateRevision, passed, issues, citations },
    commandId
  )
}

export function decideChapterFinalReview(
  reviewId: string,
  expectedStateRevision: DecimalString,
  passed: boolean,
  issues: string,
  citations: FeedbackCitation[] = [],
  commandId?: string
) {
  return commandRequest<PreparationSnapshot>(
    "POST",
    `/api/reviews/${reviewId}/decision`,
    { expectedStateRevision, passed, issues, citations },
    commandId
  )
}

export function delegateReview(
  reviewId: string,
  expectedStateRevision: DecimalString,
  notes: string,
  citations: FeedbackCitation[] = [],
  commandId?: string
) {
  return commandRequest<ReviewDelegationResult>(
    "POST",
    `/api/reviews/${reviewId}/delegate`,
    { expectedStateRevision, notes, citations },
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

export interface PreviewPromptBuiltinResponse {
  kind: "builtin"
  promptKey: string
  source: PromptSource
  system: string
  user: string
}

/** 受限模板语言的一份变量声明；内建提示词与用户模板共用同一形状。 */
export interface TemplateVariable {
  name: string
  description: string
  valueType: string
  required: boolean
  example: string
}

export interface PreviewPromptTemplateResponse {
  kind: "template"
  rendered: string
  usedVariables: string[]
}

export type PreviewPromptResponse =
  PreviewPromptBuiltinResponse | PreviewPromptTemplateResponse

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

export type PreviewPromptDraft =
  | {
      kind: "builtin"
      promptKey: string
      projectId?: string
      system?: string
      user?: string
    }
  | {
      kind: "template"
      body: string
      variables: TemplateVariable[]
      values?: Record<string, string>
    }

export function previewPrompt(draft: PreviewPromptDraft) {
  return request<PreviewPromptResponse>("/api/prompts/preview", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(draft),
  })
}

/** 一份可插入章节指导或意见草稿的用户模板。 */
export interface PromptTemplate {
  id: string
  title: string
  body: string
  variables: TemplateVariable[]
  revision: DecimalString
  createdAtMs: DecimalString
  updatedAtMs: DecimalString
}

export interface PromptTemplateDraft {
  title: string
  body: string
  variables: TemplateVariable[]
}

export function listPromptTemplates() {
  return request<PromptTemplate[]>("/api/prompt-templates")
}

export function createPromptTemplate(
  draft: PromptTemplateDraft,
  commandId?: string
) {
  return commandRequest<PromptTemplate>(
    "POST",
    "/api/prompt-templates",
    { title: draft.title, body: draft.body, variables: draft.variables },
    commandId
  )
}

export function updatePromptTemplate(
  templateId: string,
  expectedRevision: DecimalString,
  draft: PromptTemplateDraft,
  commandId?: string
) {
  return commandRequest<PromptTemplate>(
    "PATCH",
    `/api/prompt-templates/${templateId}`,
    {
      expectedRevision,
      title: draft.title,
      body: draft.body,
      variables: draft.variables,
    },
    commandId
  )
}

export function deletePromptTemplate(
  templateId: string,
  expectedRevision: DecimalString,
  commandId?: string
) {
  return commandRequest<PromptTemplate>(
    "DELETE",
    `/api/prompt-templates/${templateId}`,
    { expectedRevision },
    commandId
  )
}

/** 一次 Dashboard 查询的完整结果；所有数字都从已有记录回算。 */
export interface DashboardTokenRole {
  role: string
  inputTokens: number
  outputTokens: number
  cachedInputTokens: number
  reasoningTokens: number
  requestCount: number
  unknownUsageRequests: number
}

export interface DashboardDailyTokens {
  dayStartMs: DecimalString
  inputTokens: number
  outputTokens: number
  requestCount: number
}

/** 一次没有可用用量的原因；让「用量未知」能说明自己。 */
export type UsageUnknownReason =
  | "provider_silent"
  | "request_failed"
  | "request_cancelled"
  | "pending"
  | "invalid_report"

export interface DashboardUnknownUsage {
  reason: UsageUnknownReason
  requests: number
}

/** 一条与稿本无关的系统操作明细，例如模型连接测试。 */
export interface DashboardSystemOperation {
  id: string
  kind: string
  modelId: string
  apiBaseUrl: string
  status: string
  usageStatus: "reported" | "unknown" | "invalid"
  createdAtMs: DecimalString
  inputTokens: number
  outputTokens: number
  errorKind: string | null
}

export interface DashboardSystemOperations {
  requestCount: number
  inputTokens: number
  outputTokens: number
  cachedInputTokens: number
  reasoningTokens: number
  unknownUsage: DashboardUnknownUsage[]
  recent: DashboardSystemOperation[]
}

export interface DashboardEmbeddingKind {
  kind: string
  requestCount: number
  inputTokens: number
}

export interface DashboardDailyEmbedding {
  dayStartMs: DecimalString
  inputTokens: number
  requestCount: number
}

/** 索引、检索与连接测试的向量请求用量；单独列示，不计入对话总量。 */
export interface DashboardEmbeddingUsage {
  requestCount: number
  inputTokens: number
  unknownUsageRequests: number
  unknownUsage: DashboardUnknownUsage[]
  byKind: DashboardEmbeddingKind[]
  /** 与对话日序列同一套本地日口径，但独立成序列。 */
  daily: DashboardDailyEmbedding[]
  /** 这份数字的归属范围；知识 Space 是全局的，所以它不随项目筛选变化。 */
  scope: string
}

export interface DashboardTokens {
  inputTokens: number
  outputTokens: number
  cachedInputTokens: number
  reasoningTokens: number
  requestCount: number
  unknownUsageRequests: number
  unknownUsage: DashboardUnknownUsage[]
  byRole: DashboardTokenRole[]
  daily: DashboardDailyTokens[]
  systemOperations: DashboardSystemOperations
  embedding: DashboardEmbeddingUsage
}

export interface DashboardReviewJob {
  runId: string
  parentRunId: string | null
  projectId: string
  versionId: string
  stage: string
  status: string
}

export interface DashboardJobs {
  queued: number
  running: number
  waitingUser: number
  waitingReview: number
  paused: number
  needsAttention: number
  reviews: DashboardReviewJob[]
}

export interface DashboardDailyWords {
  dayStartMs: DecimalString
  netIncrement: number
  revisions: number
}

export interface DashboardWords {
  netIncrement: number
  currentTotal: number
  daily: DashboardDailyWords[]
}

export interface Dashboard {
  generatedAtMs: DecimalString
  range: {
    startMs: DecimalString
    endMs: DecimalString
    timezoneOffsetMinutes: number
    days: DecimalString
  }
  projectId: string | null
  tokens: DashboardTokens
  jobs: DashboardJobs
  words: DashboardWords
}

/** 服务端给出的下载文件名；缺失或无法解码时回退到 ASCII 名。 */
function downloadFileName(header: string | null, fallback: string) {
  if (!header) return fallback
  const encoded = /filename\*=UTF-8''([^;]+)/i.exec(header)?.[1]
  if (encoded) {
    try {
      return decodeURIComponent(encoded)
    } catch {
      // 损坏的百分号编码不阻止下载，改用回退名。
    }
  }
  return /filename="([^"]+)"/i.exec(header)?.[1] ?? fallback
}

/**
 * 下载一份稿本导出。
 *
 * 正文由服务端在同一个读取快照里现读，因此拿到的文件与数据库里的已提交修订链
 * 一致，不存在可以过期的导出副本。
 */
export async function downloadVersionExport(
  versionId: string,
  format: "md" | "txt"
) {
  const response = await fetch(
    `/api/versions/${versionId}/export?format=${format}`
  )
  if (!response.ok) throw await toApiError(response)
  return {
    fileName: downloadFileName(
      response.headers.get("content-disposition"),
      `nekowriter-export.${format}`
    ),
    content: await response.text(),
  }
}

export function getDashboard(
  timezoneOffsetMinutes: number,
  days: number,
  projectId?: string
) {
  const query = new URLSearchParams({
    timezoneOffsetMinutes: String(timezoneOffsetMinutes),
    days: String(days),
  })
  if (projectId) query.set("projectId", projectId)
  return request<Dashboard>(`/api/dashboard?${query.toString()}`)
}

export interface EmbeddingSettings {
  profile: {
    apiBaseUrl: string
    modelId: string
    dimension: DecimalString
  }
  credentialConfigured: boolean
  revision: DecimalString
  updatedAtMs: DecimalString
}

export interface EmbeddingSettingsResponse {
  embedding: EmbeddingSettings | null
}

export interface TestEmbeddingResponse {
  dimension: DecimalString
}

export interface KnowledgeSpace {
  id: string
  title: string
  revision: DecimalString
  entryCount: DecimalString
  createdAtMs: DecimalString
  updatedAtMs: DecimalString
}

export type KnowledgeIndexStatus =
  "not_configured" | "pending" | "running" | "ready" | "failed"

export interface KnowledgeEntrySummary {
  id: string
  spaceId: string
  title: string
  keywords: string[]
  revision: DecimalString
  indexStatus: KnowledgeIndexStatus
  contentChars: DecimalString
  updatedAtMs: DecimalString
}

export interface KnowledgeEntry {
  id: string
  spaceId: string
  title: string
  keywords: string[]
  content: string
  source: string | null
  revision: DecimalString
  indexStatus: KnowledgeIndexStatus
  indexedRevision: DecimalString | null
  indexError: string | null
  createdAtMs: DecimalString
  updatedAtMs: DecimalString
}

export interface KnowledgeIndexCoverage {
  readyEntries: DecimalString
  pendingEntries: DecimalString
  failedEntries: DecimalString
  notConfiguredEntries: DecimalString
}

export interface KnowledgeSearchHit {
  chunkId: string
  entryId: string
  entryRevision: DecimalString
  spaceId: string
  chunkIndex: DecimalString
  title: string
  text: string
  /** 向量距离换算出的排序分值，不是事实置信度。 */
  score: number
}

export interface KnowledgeSearchResult {
  hits: KnowledgeSearchHit[]
  coverage: KnowledgeIndexCoverage
}

export interface KnowledgeEntryDraft {
  title: string
  keywords: string[]
  content: string
  source: string | null
}

export function getEmbeddingSettings() {
  return request<EmbeddingSettingsResponse>("/api/settings/embedding")
}

export function saveEmbeddingSettings(
  expectedRevision: DecimalString,
  apiBaseUrl: string,
  modelId: string,
  dimension: string,
  apiKey: string | null,
  commandId?: string
) {
  return commandRequest<EmbeddingSettingsResponse>(
    "PUT",
    "/api/settings/embedding",
    { expectedRevision, apiBaseUrl, modelId, dimension, apiKey },
    commandId
  )
}

export function testEmbeddingConnection(
  apiBaseUrl: string,
  modelId: string,
  dimension: string,
  apiKey: string | null
) {
  return request<TestEmbeddingResponse>("/api/settings/test-embedding", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ apiBaseUrl, modelId, dimension, apiKey }),
  })
}

export function listKnowledgeSpaces() {
  return request<KnowledgeSpace[]>("/api/knowledge/spaces")
}

export function createKnowledgeSpace(title: string, commandId?: string) {
  return commandRequest<KnowledgeSpace>(
    "POST",
    "/api/knowledge/spaces",
    { title },
    commandId
  )
}

export function renameKnowledgeSpace(
  spaceId: string,
  expectedRevision: DecimalString,
  title: string,
  commandId?: string
) {
  return commandRequest<KnowledgeSpace>(
    "PATCH",
    `/api/knowledge/spaces/${spaceId}`,
    { expectedRevision, title },
    commandId
  )
}

export function deleteKnowledgeSpace(
  spaceId: string,
  expectedRevision: DecimalString,
  commandId?: string
) {
  return commandRequest<KnowledgeSpace>(
    "DELETE",
    `/api/knowledge/spaces/${spaceId}`,
    { expectedRevision },
    commandId
  )
}

export function listKnowledgeEntries(spaceId?: string, keyword?: string) {
  const query = new URLSearchParams()
  if (spaceId) query.set("spaceId", spaceId)
  if (keyword && keyword.trim()) query.set("keyword", keyword.trim())
  const suffix = query.size > 0 ? `?${query.toString()}` : ""
  return request<KnowledgeEntrySummary[]>(`/api/knowledge/entries${suffix}`)
}

export function getKnowledgeEntry(entryId: string) {
  return request<KnowledgeEntry>(`/api/knowledge/entries/${entryId}`)
}

export function createKnowledgeEntry(
  spaceId: string,
  draft: KnowledgeEntryDraft,
  commandId?: string
) {
  return commandRequest<KnowledgeEntry>(
    "POST",
    "/api/knowledge/entries",
    { spaceId, ...draft },
    commandId
  )
}

export function updateKnowledgeEntry(
  entryId: string,
  expectedRevision: DecimalString,
  draft: KnowledgeEntryDraft,
  commandId?: string
) {
  return commandRequest<KnowledgeEntry>(
    "PUT",
    `/api/knowledge/entries/${entryId}`,
    { expectedRevision, ...draft },
    commandId
  )
}

export function deleteKnowledgeEntry(
  entryId: string,
  expectedRevision: DecimalString,
  commandId?: string
) {
  return commandRequest<KnowledgeEntry>(
    "DELETE",
    `/api/knowledge/entries/${entryId}`,
    { expectedRevision },
    commandId
  )
}

export function searchKnowledge(
  query: string,
  spaceIds: string[],
  topK?: number
) {
  return request<KnowledgeSearchResult>("/api/knowledge/search", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ query, spaceIds, topK }),
  })
}

export function reindexKnowledge(commandId?: string) {
  return commandRequest<KnowledgeIndexCoverage>(
    "POST",
    "/api/knowledge/reindex",
    {},
    commandId
  )
}

/**
 * 知识积累入口。成功时引擎返回的候选条目会写进知识库，因此这是一条普通写入命令：
 * 网络失败重试沿用同一个 commandId，不会重复写入。MVP 没有生产引擎，服务端会以
 * `ENGINE_NOT_AVAILABLE` 说明原因，界面据此展示「暂无可用引擎」，不做模拟进度。
 */
export function accumulateKnowledge(spaceId: string, clues: string) {
  return commandRequest<KnowledgeEntry[]>("POST", "/api/knowledge/accumulate", {
    spaceId,
    clues,
  })
}

/** 回收站对象的稳定类别字符串，与路由参数一一对应。 */
export type RecycleItemKind =
  | "project"
  | "version"
  | "chapter"
  | "prompt_template"
  | "knowledge_entry"
  | "knowledge_space"

/** 被删除对象所属的仍然可见的父对象。 */
export interface RecycleParent {
  kind: string
  id: string
  title: string
}

/**
 * 回收站里的一条记录。
 *
 * `restorable`/`purgeable` 由服务端在读取时就给出结论与原因，界面据此直接说明
 * 「为什么现在不能恢复/删除」，而不是等用户点下去才失败。
 */
export interface RecycleItem {
  kind: RecycleItemKind
  id: string
  title: string
  detail: string | null
  deletedAtMs: DecimalString
  parent: RecycleParent | null
  restorable: boolean
  restoreBlockedReason: string | null
  purgeable: boolean
  purgeBlockedReason: string | null
}

/** 回收站快照；按删除时间倒序。 */
export interface RecycleBin {
  items: RecycleItem[]
}

export function getRecycleBin() {
  return request<RecycleBin>("/api/recycle-bin")
}

export function restoreRecycleItem(
  kind: RecycleItemKind,
  id: string,
  commandId?: string
) {
  return commandRequest<RecycleBin>(
    "POST",
    `/api/recycle-bin/${kind}/${id}/restore`,
    {},
    commandId
  )
}

export function purgeRecycleItem(
  kind: RecycleItemKind,
  id: string,
  commandId?: string
) {
  return commandRequest<RecycleBin>(
    "POST",
    `/api/recycle-bin/${kind}/${id}/purge`,
    {},
    commandId
  )
}

/**
 * 软删除一个项目；返回刷新后的回收站。
 *
 * 项目下仍有开放审核或活跃写作任务时会被拒绝（`RESOURCE_BUSY`）；删除只标记项目，
 * 不改写它下面的稿本与章节，因此恢复项目会把它们原样带回来。
 */
export function deleteProject(
  projectId: string,
  expectedRevision: DecimalString,
  commandId?: string
) {
  return commandRequest<RecycleBin>(
    "DELETE",
    `/api/projects/${projectId}`,
    { expectedRevision },
    commandId
  )
}

/**
 * 软删除一份番外章。正文计划章会被服务端拒绝（`INVALID_CHAPTER_STATE`），
 * 章节行、文档槽与全部修订都保留，因此从回收站恢复时还是原来那一份。
 */
export function deleteExtraChapter(
  chapterId: string,
  expectedStateRevision: DecimalString,
  commandId?: string
) {
  return commandRequest<PreparationSnapshot>(
    "DELETE",
    `/api/chapters/${chapterId}`,
    { expectedStateRevision },
    commandId
  )
}

/** 软删除一份稿本；返回刷新后的回收站。 */
export function deleteVersion(
  versionId: string,
  expectedRevision: DecimalString,
  commandId?: string
) {
  return commandRequest<RecycleBin>(
    "DELETE",
    `/api/versions/${versionId}`,
    { expectedRevision },
    commandId
  )
}
