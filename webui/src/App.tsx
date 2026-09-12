import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
} from "react"
import {
  BookOpenText,
  CircleAlert,
  CircleCheck,
  LoaderCircle,
  Plus,
  Power,
  Server,
  Sparkles,
  Trash2,
} from "lucide-react"

import { PreparationPanel } from "@/components/PreparationPanel"
import {
  GenerationPreview,
  type GenerationPreviewBuffer,
} from "@/components/GenerationPreview"
import { ChapterDirectory } from "@/components/ChapterDirectory"
import { ManuscriptReader } from "@/components/ManuscriptReader"
import { ChapterReadyPanel } from "@/components/ChapterReadyPanel"
import { ChapterOutlinePanel } from "@/components/ChapterOutlinePanel"
import { ChapterBodyPanel } from "@/components/ChapterBodyPanel"
import { DashboardPanel } from "@/components/DashboardPanel"
import { KnowledgePanel } from "@/components/KnowledgePanel"
import { ModelSettingsPanel } from "@/components/ModelSettingsPanel"
import { ProjectSettingsPanel } from "@/components/ProjectSettingsPanel"
import { ServicePanel } from "@/components/ServicePanel"
import { PromptSettingsPanel } from "@/components/PromptSettingsPanel"
import { RecycleBinPanel } from "@/components/RecycleBinPanel"
import { ShortcutHelpPanel } from "@/components/ShortcutHelpPanel"
import { TemplatePanel } from "@/components/TemplatePanel"
import { VersionPanel } from "@/components/VersionPanel"
import { WorkspaceControls } from "@/components/WorkspaceControls"
import { Button } from "@/components/ui/button"
import { useWorkspaceFocusElement } from "@/lib/shortcuts"
import {
  createProject,
  deleteProject,
  getChapters,
  getPreparation,
  getHealth,
  getVersionSnapshot,
  listProjects,
  type ChapterSummary,
  type HealthResponse,
  type PreviewDelta,
  type PreparationSnapshot,
  type PersistedEvent,
  type ProjectBundle,
  type ProjectListItem,
  type WorkflowStage,
} from "@/lib/api"

const stageLabels: Record<WorkflowStage, string> = {
  initializing: "等待开始准备",
  preparing_generating: "正在准备",
  preparing_reviewing: "准备审核中",
  preparing_revising: "正在修订准备内容",
  writing_chapter_ready: "等待章节指导",
  writing_outline_generating: "正在撰写细纲",
  writing_outline_reviewing: "细纲审核中",
  writing_drafting: "正在写正文",
  writing_chapter_reviewing: "整章审核中",
  writing_chapter_done: "本章已完成",
  completed: "已完本",
}

const persistentEventTypes = [
  "workflow.changed",
  "run.changed",
  "tool.proposed",
  "tool.resolved",
  "question.created",
  "question.answered",
  "review.changed",
  "document.committed",
  "feedback.changed",
  "settings.changed",
  "knowledge.index_changed",
  "prompt_template.changed",
  "system.model_operation",
  "recycle.changed",
] as const

function formatTime(timestamp: string) {
  const milliseconds = BigInt(timestamp)
  if (milliseconds > BigInt(Number.MAX_SAFE_INTEGER))
    return "时间戳超出可显示范围"
  return new Intl.DateTimeFormat("zh-CN", {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(Number(milliseconds))
}

function isStalePreparationSnapshot(
  next: PreparationSnapshot,
  current: PreparationSnapshot | null
) {
  return (
    current?.versionId === next.versionId &&
    BigInt(next.workflow.stateRevision) < BigInt(current.workflow.stateRevision)
  )
}

function previewBufferKey(delta: PreviewDelta) {
  return delta.tool
    ? `${delta.requestId}:tool:${delta.tool.index}`
    : `${delta.requestId}:text`
}

function previewOffset(value: string) {
  try {
    const offset = BigInt(value)
    if (offset < 0n || offset > BigInt(Number.MAX_SAFE_INTEGER)) return null
    return Number(offset)
  } catch {
    return null
  }
}

/** 预览流按 Unicode scalar offset 只接受连续追加或已经收到的重复片段。 */
function appendPreviewContent(
  existing: string,
  offset: number,
  incoming: string
) {
  const characters = Array.from(existing)
  if (offset > characters.length) return null
  if (offset < characters.length) {
    const seen = characters
      .slice(offset, offset + Array.from(incoming).length)
      .join("")
    return seen === incoming ? existing : null
  }
  return existing + incoming
}

function mergePreviewDelta(
  current: Record<string, GenerationPreviewBuffer>,
  delta: PreviewDelta
) {
  const offset = previewOffset(delta.offset)
  if (offset === null) return current
  const key = previewBufferKey(delta)
  const previous = current[key]
  const content = appendPreviewContent(
    previous?.content ?? "",
    offset,
    delta.content
  )
  if (content === null) return current
  const tool = delta.tool
    ? {
        index: delta.tool.index,
        id: delta.tool.id ?? previous?.tool?.id,
        name: delta.tool.name ?? previous?.tool?.name,
      }
    : undefined
  const next: GenerationPreviewBuffer = {
    key,
    requestId: delta.requestId,
    tool,
    content,
  }
  return { ...current, [key]: next }
}

function discardPreviewForRequest(
  current: Record<string, GenerationPreviewBuffer>,
  requestId: string
) {
  let changed = false
  const next: Record<string, GenerationPreviewBuffer> = {}
  for (const [key, preview] of Object.entries(current)) {
    if (preview.requestId === requestId) {
      changed = true
    } else {
      next[key] = preview
    }
  }
  return changed ? next : current
}

function persistentRequestId(event: PersistedEvent) {
  if (!event.payload || typeof event.payload !== "object") return null
  const requestId = (event.payload as { request_id?: unknown }).request_id
  return typeof requestId === "string" ? requestId : null
}

export function App() {
  const [projects, setProjects] = useState<ProjectListItem[]>([])
  const [health, setHealth] = useState<HealthResponse | null>(null)
  const [loading, setLoading] = useState(true)
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [selectedProjectId, setSelectedProjectId] = useState<string | null>(
    null
  )
  const [preparation, setPreparation] = useState<PreparationSnapshot | null>(
    null
  )
  const [preparationLoading, setPreparationLoading] = useState(false)
  const [generationPreviews, setGenerationPreviews] = useState<
    Record<string, GenerationPreviewBuffer>
  >({})
  const [eventConnection, setEventConnection] = useState<
    "idle" | "connecting" | "connected" | "reconnecting"
  >("idle")
  // 退出服务后浏览器仍停留在最后一个快照上，界面必须说清楚这一点。
  const [serviceStopped, setServiceStopped] = useState(false)
  const serviceStoppedRef = useRef(false)
  const [settingsRefreshToken, setSettingsRefreshToken] = useState(0)
  const [knowledgeRefreshToken, setKnowledgeRefreshToken] = useState(0)
  // 回收站不属于任何稿本：删除、恢复与彻底删除都会改变它。
  const [recycleRefreshToken, setRecycleRefreshToken] = useState(0)
  // 模板列表与概览都不属于稿本，却会被写作活动改变，因此各自有一个独立的刷新信号。
  const [templatesRefreshToken, setTemplatesRefreshToken] = useState(0)
  const [metricsRefreshToken, setMetricsRefreshToken] = useState(0)
  const [chapters, setChapters] = useState<ChapterSummary[]>([])
  const [chaptersRefreshToken, setChaptersRefreshToken] = useState(0)
  // 工作台有两种视图：写作流程与长篇阅读。阅读位置由阅读区自己保存，视图切换
  // 因此不会丢掉位置；这里只记住用户想看哪一边，以及一次定位请求。
  const [view, setView] = useState<"writing" | "reading">("writing")
  const [readingTarget, setReadingTarget] = useState<{
    versionId: string
    chapterId: string
    nonce: number
  } | null>(null)
  const [readingChapterId, setReadingChapterId] = useState<string | null>(null)
  const seenPersistentEventSequences = useRef(new Set<string>())
  const [projectName, setProjectName] = useState("")
  const [novelTitle, setNovelTitle] = useState("")
  const [versionLabel, setVersionLabel] = useState("初稿")
  const [deletingProjectId, setDeletingProjectId] = useState<string | null>(
    null
  )
  const [confirmingProjectId, setConfirmingProjectId] = useState<string | null>(
    null
  )
  // 快捷键动作后焦点掉到 body 时的回落点，Tab 顺序保持从工作台开始。
  const workspaceFocusRef = useWorkspaceFocusElement()

  /** 服务已经退出：界面停止重试，并说明如何重新启动。 */
  const markServiceStopped = useCallback(() => {
    if (serviceStoppedRef.current) return
    serviceStoppedRef.current = true
    setServiceStopped(true)
  }, [])

  /**
   * 事件同步断开后确认服务是否还在。
   *
   * 服务可能在别处被退出（Ctrl+C、SIGTERM，或浏览器标签页关闭后由终端退出），
   * 因此「连不上」必须先问一次健康检查，再决定显示重连还是已退出。
   */
  const confirmServiceAlive = useCallback(async () => {
    if (serviceStoppedRef.current) return false
    const alive = await getHealth().then(
      () => true,
      () => false
    )
    if (!alive) markServiceStopped()
    return alive
  }, [markServiceStopped])

  /** 让概览重新读取一次；系统操作（如连接测试）落库后由发起方调用。 */
  const bumpMetrics = useCallback(() => {
    setMetricsRefreshToken((current) => current + 1)
  }, [])

  /** 重新读取章节目录；阅读区发现服务端章节数与本地目录不一致时调用。 */
  const bumpChapters = useCallback(() => {
    setChaptersRefreshToken((current) => current + 1)
  }, [])

  /** 重新读取项目列表；不切换 loading 占位，因为已有内容不需要消失。 */
  const refreshProjects = useCallback(async () => {
    try {
      const nextProjects = await listProjects()
      setProjects(nextProjects)
      setSelectedProjectId((current) => {
        if (
          current &&
          nextProjects.some((item) => item.project.id === current)
        ) {
          return current
        }
        return nextProjects[0]?.project.id ?? null
      })
    } catch (loadError) {
      setError(
        loadError instanceof Error ? loadError.message : "无法读取本地项目"
      )
    }
  }, [])

  /**
   * 回收站里的动作（恢复或彻底删除）会同时改变目录与项目列表。
   *
   * 恢复一个项目会让整个项目重新出现，只刷新章节是不够的；删除或恢复章节则
   * 只需要重读目录。两件事都做，避免任一入口留下陈旧数据。
   */
  const refreshRecycleOutcome = useCallback(() => {
    void refreshProjects()
    bumpChapters()
  }, [bumpChapters, refreshProjects])

  const loadDashboard = useCallback(async () => {
    setLoading(true)
    try {
      const [nextHealth, nextProjects] = await Promise.all([
        getHealth(),
        listProjects(),
      ])
      setHealth(nextHealth)
      setProjects(nextProjects)
      setSelectedProjectId((current) => {
        if (
          current &&
          nextProjects.some((item) => item.project.id === current)
        ) {
          return current
        }
        return nextProjects[0]?.project.id ?? null
      })
      setError(null)
    } catch (loadError) {
      setError(
        loadError instanceof Error ? loadError.message : "无法连接本地服务"
      )
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    let isCurrent = true

    Promise.all([getHealth(), listProjects()])
      .then(([nextHealth, nextProjects]) => {
        if (!isCurrent) return
        setHealth(nextHealth)
        setProjects(nextProjects)
        setSelectedProjectId((current) => {
          if (
            current &&
            nextProjects.some((item) => item.project.id === current)
          ) {
            return current
          }
          return nextProjects[0]?.project.id ?? null
        })
        setError(null)
      })
      .catch((loadError: unknown) => {
        if (!isCurrent) return
        setError(
          loadError instanceof Error ? loadError.message : "无法连接本地服务"
        )
      })
      .finally(() => {
        if (isCurrent) setLoading(false)
      })

    return () => {
      isCurrent = false
    }
  }, [])

  const selectedProject = useMemo(
    () =>
      projects.find((item) => item.project.id === selectedProjectId) ?? null,
    [projects, selectedProjectId]
  )
  const selectedVersionId = selectedProject?.selectedVersion.id ?? null

  // 目录需要与工作流一起前进：章节状态、编号和新增番外都会改变这个列表。
  const chaptersRevision = preparation?.workflow.stateRevision ?? null
  useEffect(() => {
    let isCurrent = true
    const request: Promise<ChapterSummary[]> = selectedVersionId
      ? getChapters(selectedVersionId)
      : Promise.resolve([])
    request
      .then((items) => {
        if (isCurrent) setChapters(items)
      })
      .catch(() => {
        if (isCurrent) setChapters([])
      })
    return () => {
      isCurrent = false
    }
  }, [selectedVersionId, chaptersRevision, chaptersRefreshToken])

  // 阅读位置属于某一份稿本：切换稿本后旧的定位目标自然失效。
  const activeReadingTarget =
    readingTarget && readingTarget.versionId === selectedVersionId
      ? readingTarget
      : null

  const applyPreparationSnapshot = useCallback((next: PreparationSnapshot) => {
    setPreparation((current) =>
      isStalePreparationSnapshot(next, current) ? current : next
    )
    setProjects((current) =>
      current.map((item) =>
        item.selectedVersion.id === next.versionId &&
        BigInt(next.workflow.stateRevision) >=
          BigInt(item.workflow.stateRevision)
          ? { ...item, workflow: next.workflow }
          : item
      )
    )
  }, [])

  const applyProjectBundle = useCallback((bundle: ProjectBundle) => {
    setProjects((current) =>
      current.map((item) =>
        item.project.id === bundle.project.id
          ? {
              project: bundle.project,
              selectedVersion: bundle.version,
              workflow: bundle.workflow,
            }
          : item
      )
    )
    setSelectedProjectId(bundle.project.id)
  }, [])

  useEffect(() => {
    let isCurrent = true
    let source: EventSource | null = null
    let refreshInFlight = false
    let refreshQueued = false
    let resetInFlight = false
    seenPersistentEventSequences.current.clear()
    setGenerationPreviews({})
    if (!selectedVersionId) {
      setEventConnection("idle")
      Promise.resolve().then(() => {
        if (!isCurrent) return
        setPreparation(null)
        setPreparationLoading(false)
      })
      return () => {
        isCurrent = false
      }
    }
    const versionId = selectedVersionId

    async function refreshPreparation() {
      if (refreshInFlight) {
        refreshQueued = true
        return
      }
      refreshInFlight = true
      try {
        const next = await getPreparation(versionId)
        if (isCurrent) applyPreparationSnapshot(next)
      } catch (loadError) {
        if (isCurrent) {
          setError(
            loadError instanceof Error ? loadError.message : "无法同步准备阶段"
          )
        }
      } finally {
        refreshInFlight = false
        if (refreshQueued) {
          refreshQueued = false
          void refreshPreparation()
        }
      }
    }

    async function loadAndSubscribe() {
      setPreparationLoading(true)
      setEventConnection("connecting")
      try {
        const versionSnapshot = await getVersionSnapshot(versionId)
        const nextPreparation = await getPreparation(versionId)
        if (!isCurrent) return
        applyPreparationSnapshot(nextPreparation)

        const eventSource = new EventSource(
          "/api/events?after=" +
            encodeURIComponent(versionSnapshot.eventWatermark)
        )
        source = eventSource
        eventSource.onopen = () => {
          if (isCurrent && source === eventSource) {
            setEventConnection("connected")
          }
        }
        eventSource.onerror = () => {
          if (!isCurrent || source !== eventSource) return
          // 临时流不具备游标恢复语义；连接一旦中断就不能把旧缓冲当作连续输出。
          setGenerationPreviews({})
          setEventConnection("reconnecting")
          void confirmServiceAlive().then((alive) => {
            if (!alive && source === eventSource) eventSource.close()
          })
        }
        const resetStream = () => {
          if (!isCurrent || resetInFlight || source !== eventSource) return
          resetInFlight = true
          eventSource.close()
          seenPersistentEventSequences.current.clear()
          setGenerationPreviews({})
          setEventConnection("reconnecting")
          void loadAndSubscribe().finally(() => {
            resetInFlight = false
          })
        }
        const onPersistentEvent = (event: Event, eventType: string) => {
          try {
            const persisted = JSON.parse(
              (event as MessageEvent<string>).data
            ) as PersistedEvent
            if (
              !isCurrent ||
              source !== eventSource ||
              seenPersistentEventSequences.current.has(persisted.seq)
            ) {
              return
            }
            seenPersistentEventSequences.current.add(persisted.seq)
            if (seenPersistentEventSequences.current.size > 2_048) {
              const oldest = seenPersistentEventSequences.current
                .values()
                .next().value
              if (oldest) seenPersistentEventSequences.current.delete(oldest)
            }
            if (persisted.version_id === versionId) {
              const requestId = persistentRequestId(persisted)
              if (requestId) {
                setGenerationPreviews((current) =>
                  discardPreviewForRequest(current, requestId)
                )
              }
            }
            // 任何持久事件都可能改变统计：请求终结、正文提交、审核与意见变化、
            // 连接测试都会反映在概览里。因此概览跟着事件走，而不是等用户手动刷新。
            setMetricsRefreshToken((current) => current + 1)
            if (eventType === "settings.changed") {
              setSettingsRefreshToken((current) => current + 1)
            } else if (eventType === "knowledge.index_changed") {
              setKnowledgeRefreshToken((current) => current + 1)
            } else if (eventType === "prompt_template.changed") {
              setTemplatesRefreshToken((current) => current + 1)
            } else if (eventType === "recycle.changed") {
              setRecycleRefreshToken((current) => current + 1)
              void refreshPreparation()
            } else if (persisted.version_id === versionId) {
              void refreshPreparation()
            }
          } catch {
            // 损坏事件不会更新本地视图；浏览器会继续消费后续有序事件。
          }
        }
        for (const eventType of persistentEventTypes) {
          eventSource.addEventListener(eventType, (event) =>
            onPersistentEvent(event, eventType)
          )
        }
        eventSource.addEventListener("preview.delta", (event) => {
          try {
            const delta = JSON.parse(
              (event as MessageEvent<string>).data
            ) as PreviewDelta
            if (
              !isCurrent ||
              source !== eventSource ||
              delta.versionId !== versionId ||
              typeof delta.requestId !== "string" ||
              typeof delta.offset !== "string" ||
              typeof delta.content !== "string"
            ) {
              return
            }
            setGenerationPreviews((current) =>
              mergePreviewDelta(current, delta)
            )
          } catch {
            // 临时预览损坏时直接丢弃该片段；正式工作流仍由持久事件刷新。
          }
        })
        eventSource.addEventListener("stream.reset_required", resetStream)
      } catch (loadError) {
        if (isCurrent) {
          setError(
            loadError instanceof Error ? loadError.message : "无法读取准备阶段"
          )
          setEventConnection("reconnecting")
          void confirmServiceAlive()
        }
      } finally {
        if (isCurrent) setPreparationLoading(false)
      }
    }

    void loadAndSubscribe()

    return () => {
      isCurrent = false
      source?.close()
    }
  }, [applyPreparationSnapshot, confirmServiceAlive, selectedVersionId])

  /**
   * 删除一个项目：第一次点击只进入确认状态，第二次才发出请求。
   *
   * 项目删除是软删除，稿本与正文都留在数据库里，因此删掉后能从回收站恢复；
   * 删除成功后重新读取项目列表，界面会自动切到剩下的第一个项目。
   */
  async function handleDeleteProject(item: ProjectListItem) {
    if (confirmingProjectId !== item.project.id) {
      setConfirmingProjectId(item.project.id)
      return
    }
    setConfirmingProjectId(null)
    setDeletingProjectId(item.project.id)
    try {
      await deleteProject(item.project.id, item.project.revision)
      await loadDashboard()
    } catch (deleteError) {
      setError(
        deleteError instanceof Error ? deleteError.message : "项目未能删除"
      )
    } finally {
      setDeletingProjectId(null)
    }
  }

  async function handleCreateProject(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setSubmitting(true)
    setError(null)
    try {
      const created = await createProject({
        name: projectName,
        novelTitle,
        versionLabel,
      })
      setSelectedProjectId(created.project.id)
      setProjectName("")
      setNovelTitle("")
      setVersionLabel("初稿")
      await loadDashboard()
    } catch (createError) {
      setError(
        createError instanceof Error ? createError.message : "创建项目失败"
      )
    } finally {
      setSubmitting(false)
    }
  }

  return (
    <main className="min-h-svh bg-[radial-gradient(circle_at_100%_0%,oklch(0.95_0.025_82),transparent_33rem),radial-gradient(circle_at_0%_100%,oklch(0.96_0.02_245),transparent_30rem)] px-4 py-5 sm:px-7 sm:py-8">
      <div
        ref={workspaceFocusRef}
        tabIndex={-1}
        className="mx-auto max-w-6xl outline-none"
      >
        <header className="mb-8 flex flex-col gap-5 border-b border-border/70 pb-6 sm:flex-row sm:items-end sm:justify-between">
          <div>
            <div className="mb-2 flex items-center gap-2 text-xs font-semibold tracking-[0.18em] text-muted-foreground uppercase">
              <Sparkles className="size-3.5" aria-hidden="true" />
              Local writing workspace
            </div>
            <h1 className="text-3xl font-semibold tracking-tight sm:text-4xl">
              NekoWriter
            </h1>
            <p className="mt-2 max-w-xl text-sm leading-6 text-muted-foreground">
              本地保存项目与稿本。填写创作目标、审核准备产物，再逐步进入章节写作。
            </p>
          </div>{" "}
          <div className="flex w-full flex-col items-start gap-3 sm:w-auto sm:items-end">
            <WorkspaceControls />
            <div className="flex shrink-0 items-center gap-2 rounded-full border border-border bg-background/75 px-3 py-1.5 text-sm whitespace-nowrap shadow-sm backdrop-blur">
              <Server
                className="size-4 text-muted-foreground"
                aria-hidden="true"
              />
              {serviceStopped ? (
                <span className="flex items-center gap-1.5 text-destructive">
                  <CircleAlert className="size-4" aria-hidden="true" />
                  本地服务已退出
                </span>
              ) : health?.status === "ok" ? (
                <span className="flex items-center gap-1.5">
                  <CircleCheck
                    className="size-4 text-emerald-600"
                    aria-hidden="true"
                  />
                  本地服务正常
                </span>
              ) : (
                <span>正在连接本地服务</span>
              )}
              {selectedVersionId && (
                <span className="flex items-center gap-1 border-l border-border pl-2 text-xs">
                  {(["writing", "reading"] as const).map((candidate) => (
                    <button
                      key={candidate}
                      type="button"
                      aria-pressed={view === candidate}
                      onClick={() => setView(candidate)}
                      className={`rounded-full px-2 py-0.5 transition ${
                        view === candidate
                          ? "bg-primary text-primary-foreground"
                          : "text-muted-foreground hover:bg-muted"
                      }`}
                    >
                      {candidate === "writing" ? "写作" : "阅读"}
                    </button>
                  ))}
                </span>
              )}
              {selectedVersionId && view === "writing" && (
                <span className="border-l border-border pl-2 text-xs text-muted-foreground">
                  {serviceStopped
                    ? "事件同步已停止"
                    : {
                        idle: "事件同步待命",
                        connecting: "正在建立事件同步",
                        connected: "事件同步已连接",
                        reconnecting: "正在重连事件同步",
                      }[eventConnection]}
                </span>
              )}
            </div>
          </div>
        </header>

        {serviceStopped && (
          <div
            className="mb-6 flex items-start gap-3 rounded-xl border border-border bg-muted/60 px-4 py-3 text-sm"
            role="status"
          >
            <Power className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
            <div>
              <p className="font-medium">本地服务已退出</p>
              <p className="mt-1 text-muted-foreground">
                下方内容是本机最后读取到的数据，不会继续更新，也暂时无法提交新操作。
                重新运行 NekoWriter（发行目录里的 nekowriter，或源码目录里的{" "}
                cargo run -p nekowriter-app）后刷新此页面即可继续。
              </p>
            </div>
          </div>
        )}

        {error && (
          <div
            className="mb-6 flex items-start gap-3 rounded-xl border border-destructive/25 bg-destructive/8 px-4 py-3 text-sm text-destructive"
            role="alert"
          >
            <CircleAlert
              className="mt-0.5 size-4 shrink-0"
              aria-hidden="true"
            />
            <div>
              <p className="font-medium">操作未完成</p>
              <p className="mt-1 text-destructive/85">{error}</p>
            </div>
          </div>
        )}

        <section className="grid gap-6 lg:grid-cols-[minmax(0,1fr)_21rem]">
          <div className="min-w-0">
            <div className="mb-3 flex items-center justify-between">
              <div>
                <h2 className="text-lg font-semibold">你的项目</h2>
                <p className="mt-1 text-sm text-muted-foreground">
                  每个项目拥有独立的稿本、工作流和创作记录。
                </p>
              </div>
              {health && (
                <span className="text-xs text-muted-foreground">
                  Schema {health.schemaVersion}
                </span>
              )}
            </div>

            {loading ? (
              <div className="flex min-h-52 items-center justify-center rounded-2xl border border-dashed border-border bg-background/60 text-sm text-muted-foreground">
                <LoaderCircle
                  className="mr-2 size-4 animate-spin"
                  aria-hidden="true"
                />
                正在读取本地项目…
              </div>
            ) : projects.length === 0 ? (
              <div className="rounded-2xl border border-dashed border-border bg-background/60 p-8 text-center shadow-sm">
                <BookOpenText
                  className="mx-auto size-8 text-muted-foreground"
                  aria-hidden="true"
                />
                <h3 className="mt-4 font-medium">从第一部作品开始</h3>
                <p className="mx-auto mt-2 max-w-sm text-sm leading-6 text-muted-foreground">
                  创建后会保存一个独立稿本；所有后续设定、章节、审核和上下文都归属于它。
                </p>
              </div>
            ) : (
              <div className="grid gap-3 sm:grid-cols-2">
                {projects.map((item) => {
                  const isSelected = item.project.id === selectedProjectId
                  const isDeleting = deletingProjectId === item.project.id
                  return (
                    <div key={item.project.id} className="flex flex-col gap-1">
                      <button
                        type="button"
                        disabled={isDeleting}
                        onClick={() => setSelectedProjectId(item.project.id)}
                        className={`rounded-2xl border p-5 text-left shadow-sm transition focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring ${
                          isSelected
                            ? "border-primary/50 bg-primary text-primary-foreground"
                            : "border-border bg-background/80 hover:border-foreground/25 hover:bg-background"
                        }`}
                      >
                        <div className="flex items-start justify-between gap-3">
                          <span
                            className={`rounded-full px-2 py-1 text-xs font-medium ${isSelected ? "bg-primary-foreground/15" : "bg-muted text-muted-foreground"}`}
                          >
                            {item.selectedVersion.label}
                          </span>
                          <span
                            className={`text-xs ${isSelected ? "text-primary-foreground/70" : "text-muted-foreground"}`}
                          >
                            {stageLabels[item.workflow.stage]}
                          </span>
                        </div>
                        <h3 className="mt-5 truncate text-lg font-semibold">
                          {item.selectedVersion.novelTitle}
                        </h3>
                        <p
                          className={`mt-1 truncate text-sm ${isSelected ? "text-primary-foreground/75" : "text-muted-foreground"}`}
                        >
                          {item.project.name}
                        </p>
                        <p
                          className={`mt-5 text-xs ${isSelected ? "text-primary-foreground/65" : "text-muted-foreground"}`}
                        >
                          更新于 {formatTime(item.project.updatedAtMs)}
                        </p>
                      </button>
                      <div className="flex justify-end">
                        <Button
                          type="button"
                          variant={
                            confirmingProjectId === item.project.id
                              ? "destructive"
                              : "ghost"
                          }
                          size="sm"
                          disabled={deletingProjectId !== null}
                          onClick={() => void handleDeleteProject(item)}
                        >
                          {isDeleting ? (
                            <LoaderCircle
                              className="animate-spin"
                              aria-hidden="true"
                            />
                          ) : (
                            <Trash2 aria-hidden="true" />
                          )}
                          {confirmingProjectId === item.project.id
                            ? "确认删除项目"
                            : "删除项目"}
                        </Button>
                      </div>
                    </div>
                  )
                })}
              </div>
            )}

            {selectedProject && (
              <>
                <div className="mt-6 rounded-2xl border border-border bg-background/75 p-5 shadow-sm">
                  <p className="text-xs font-semibold tracking-[0.14em] text-muted-foreground uppercase">
                    当前稿本
                  </p>
                  <div className="mt-3 flex flex-wrap items-baseline justify-between gap-3">
                    <div>
                      <h2 className="text-xl font-semibold">
                        {selectedProject.selectedVersion.novelTitle}
                      </h2>
                      <p className="mt-1 text-sm text-muted-foreground">
                        {selectedProject.project.name} ·{" "}
                        {selectedProject.selectedVersion.label}
                      </p>
                    </div>
                    <span className="rounded-full bg-muted px-3 py-1 text-sm text-muted-foreground">
                      {stageLabels[selectedProject.workflow.stage]}
                    </span>
                  </div>
                  <p className="mt-5 border-t border-border pt-4 text-sm leading-6 text-muted-foreground">
                    稿本、工作流、准备产物和审核都保存在本地。准备阶段通过后，下方会展示将交给后续
                    Agent 的纯文本折叠结果。
                  </p>
                </div>
                {view === "reading" ? (
                  <ManuscriptReader
                    key={selectedProject.selectedVersion.id}
                    versionId={selectedProject.selectedVersion.id}
                    chapters={chapters}
                    activeChapterId={
                      preparation?.workflow.activeChapterId ?? null
                    }
                    target={activeReadingTarget}
                    onVisibleChapterChange={setReadingChapterId}
                    onIndexStale={bumpChapters}
                    onBackToWriting={() => setView("writing")}
                    onError={setError}
                  />
                ) : null}
                {view === "writing" ? (
                  <>
                    <VersionPanel
                      project={selectedProject.project}
                      selectedVersion={selectedProject.selectedVersion}
                      onChanged={applyProjectBundle}
                      onError={setError}
                      onDeleted={bumpChapters}
                    />
                    <PreparationPanel
                      key={selectedProject.selectedVersion.id}
                      versionId={selectedProject.selectedVersion.id}
                      preparation={preparation}
                      loading={preparationLoading}
                      onChanged={applyPreparationSnapshot}
                      onError={setError}
                    />
                    <GenerationPreview
                      buffers={Object.values(generationPreviews)}
                    />
                    {preparation?.workflow.stage ===
                      "writing_chapter_ready" && (
                      <ChapterReadyPanel
                        versionId={selectedProject.selectedVersion.id}
                        preparation={preparation}
                        onChanged={applyPreparationSnapshot}
                        onError={setError}
                        templatesRefreshToken={templatesRefreshToken}
                      />
                    )}
                    {(preparation?.workflow.stage ===
                      "writing_outline_generating" ||
                      preparation?.workflow.stage ===
                        "writing_outline_reviewing" ||
                      preparation?.workflow.stage === "writing_drafting") && (
                      <ChapterOutlinePanel
                        versionId={selectedProject.selectedVersion.id}
                        preparation={preparation}
                        onChanged={applyPreparationSnapshot}
                        onError={setError}
                      />
                    )}
                    {(preparation?.workflow.stage === "writing_drafting" ||
                      preparation?.workflow.stage ===
                        "writing_chapter_reviewing" ||
                      preparation?.workflow.stage === "writing_chapter_done" ||
                      preparation?.workflow.stage === "completed") && (
                      <ChapterBodyPanel
                        versionId={selectedProject.selectedVersion.id}
                        preparation={preparation}
                        onChanged={applyPreparationSnapshot}
                        onError={setError}
                        templatesRefreshToken={templatesRefreshToken}
                      />
                    )}
                  </>
                ) : null}
              </>
            )}
          </div>

          <aside className="space-y-6">
            {selectedProject && chapters.length > 0 && (
              <ChapterDirectory
                chapters={chapters}
                activeChapterId={preparation?.workflow.activeChapterId ?? null}
                readingChapterId={readingChapterId}
                onSelect={(chapterId) => {
                  if (!selectedVersionId) return
                  setReadingTarget({
                    versionId: selectedVersionId,
                    chapterId,
                    nonce: Date.now(),
                  })
                  setView("reading")
                }}
              />
            )}
            <div className="rounded-2xl border border-border bg-background/85 p-5 shadow-sm backdrop-blur">
              <div className="flex items-center gap-2">
                <span className="flex size-8 items-center justify-center rounded-xl bg-primary text-primary-foreground">
                  <Plus className="size-4" aria-hidden="true" />
                </span>
                <div>
                  <h2 className="font-semibold">新建项目</h2>
                  <p className="text-xs text-muted-foreground">从空稿本开始</p>
                </div>
              </div>
              <form className="mt-5 space-y-4" onSubmit={handleCreateProject}>
                <label className="block text-sm font-medium">
                  项目名称
                  <input
                    required
                    maxLength={120}
                    value={projectName}
                    onChange={(event) => setProjectName(event.target.value)}
                    placeholder="例如：雨夜计划"
                    className="mt-1.5 h-10 w-full rounded-xl border border-input bg-background px-3 text-sm transition outline-none placeholder:text-muted-foreground focus:border-ring focus:ring-3 focus:ring-ring/20"
                  />
                </label>
                <label className="block text-sm font-medium">
                  小说标题
                  <input
                    required
                    maxLength={200}
                    value={novelTitle}
                    onChange={(event) => setNovelTitle(event.target.value)}
                    placeholder="例如：雨夜的来客"
                    className="mt-1.5 h-10 w-full rounded-xl border border-input bg-background px-3 text-sm transition outline-none placeholder:text-muted-foreground focus:border-ring focus:ring-3 focus:ring-ring/20"
                  />
                </label>
                <label className="block text-sm font-medium">
                  首个稿本标签
                  <input
                    required
                    maxLength={80}
                    value={versionLabel}
                    onChange={(event) => setVersionLabel(event.target.value)}
                    className="mt-1.5 h-10 w-full rounded-xl border border-input bg-background px-3 text-sm transition outline-none placeholder:text-muted-foreground focus:border-ring focus:ring-3 focus:ring-ring/20"
                  />
                </label>
                <Button
                  className="mt-2 w-full"
                  size="lg"
                  type="submit"
                  disabled={submitting}
                >
                  {submitting ? (
                    <LoaderCircle className="animate-spin" aria-hidden="true" />
                  ) : (
                    <Plus aria-hidden="true" />
                  )}
                  {submitting ? "正在创建…" : "创建项目"}
                </Button>
              </form>
            </div>
            <DashboardPanel
              projectId={selectedProject?.project.id ?? null}
              refreshToken={metricsRefreshToken}
            />
            <ModelSettingsPanel
              refreshToken={settingsRefreshToken}
              onModelOperation={bumpMetrics}
            />
            <ProjectSettingsPanel
              projectId={selectedProject?.project.id ?? null}
              refreshToken={settingsRefreshToken}
              onModelOperation={bumpMetrics}
            />
            <PromptSettingsPanel
              projectId={selectedProject?.project.id ?? null}
              refreshToken={settingsRefreshToken}
            />
            <TemplatePanel refreshToken={templatesRefreshToken} />
            <KnowledgePanel refreshToken={knowledgeRefreshToken} />
            <RecycleBinPanel
              refreshToken={recycleRefreshToken}
              onChanged={refreshRecycleOutcome}
              onError={setError}
            />
            <ServicePanel
              health={health}
              stopped={serviceStopped}
              onStopped={markServiceStopped}
            />
          </aside>
        </section>
      </div>
      <ShortcutHelpPanel />
    </main>
  )
}

export default App
