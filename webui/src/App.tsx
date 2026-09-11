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
  Server,
  Sparkles,
} from "lucide-react"

import { PreparationPanel } from "@/components/PreparationPanel"
import { ChapterDirectory } from "@/components/ChapterDirectory"
import { ChapterReader } from "@/components/ChapterReader"
import { ChapterReadyPanel } from "@/components/ChapterReadyPanel"
import { ChapterOutlinePanel } from "@/components/ChapterOutlinePanel"
import { ChapterBodyPanel } from "@/components/ChapterBodyPanel"
import { ModelSettingsPanel } from "@/components/ModelSettingsPanel"
import { ProjectSettingsPanel } from "@/components/ProjectSettingsPanel"
import { PromptSettingsPanel } from "@/components/PromptSettingsPanel"
import { VersionPanel } from "@/components/VersionPanel"
import { Button } from "@/components/ui/button"
import {
  createProject,
  getChapters,
  getPreparation,
  getHealth,
  getVersionSnapshot,
  listProjects,
  type ChapterSummary,
  type HealthResponse,
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
] as const

function formatTime(timestamp: string) {
  const milliseconds = BigInt(timestamp)
  if (milliseconds > BigInt(Number.MAX_SAFE_INTEGER)) return "时间戳超出可显示范围"
  return new Intl.DateTimeFormat("zh-CN", {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(Number(milliseconds))
}

function isStalePreparationSnapshot(
  next: PreparationSnapshot,
  current: PreparationSnapshot | null,
) {
  return (
    current?.versionId === next.versionId &&
    BigInt(next.workflow.stateRevision) < BigInt(current.workflow.stateRevision)
  )
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
  const [eventConnection, setEventConnection] = useState<
    "idle" | "connecting" | "connected" | "reconnecting"
  >("idle")
  const [settingsRefreshToken, setSettingsRefreshToken] = useState(0)
  const [chapters, setChapters] = useState<ChapterSummary[]>([])
  const [browsed, setBrowsed] = useState<{
    versionId: string
    chapterId: string
  } | null>(null)
  const seenPersistentEventSequences = useRef(new Set<string>())
  const [projectName, setProjectName] = useState("")
  const [novelTitle, setNovelTitle] = useState("")
  const [versionLabel, setVersionLabel] = useState("初稿")

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
  }, [selectedVersionId, chaptersRevision])

  // 浏览目标以稿本为键，切换稿本时自然失效，不需要额外的重置副作用。
  const browsedChapterId =
    browsed && browsed.versionId === selectedVersionId
      ? browsed.chapterId
      : null

  const browsedChapter = useMemo(
    () => chapters.find((chapter) => chapter.id === browsedChapterId) ?? null,
    [chapters, browsedChapterId]
  )

  const applyPreparationSnapshot = useCallback((next: PreparationSnapshot) => {
    setPreparation((current) =>
      isStalePreparationSnapshot(next, current) ? current : next,
    )
    setProjects((current) =>
      current.map((item) =>
        item.selectedVersion.id === next.versionId &&
        BigInt(next.workflow.stateRevision) >= BigInt(item.workflow.stateRevision)
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
          : item,
      ),
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
          if (isCurrent && source === eventSource) {
            setEventConnection("reconnecting")
          }
        }
        const resetStream = () => {
          if (!isCurrent || resetInFlight || source !== eventSource) return
          resetInFlight = true
          eventSource.close()
          seenPersistentEventSequences.current.clear()
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
              const oldest = seenPersistentEventSequences.current.values().next()
                .value
              if (oldest) seenPersistentEventSequences.current.delete(oldest)
            }
            if (eventType === "settings.changed") {
              setSettingsRefreshToken((current) => current + 1)
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
        eventSource.addEventListener("stream.reset_required", resetStream)
      } catch (loadError) {
        if (isCurrent) {
          setError(
            loadError instanceof Error ? loadError.message : "无法读取准备阶段"
          )
          setEventConnection("reconnecting")
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
  }, [applyPreparationSnapshot, selectedVersionId])

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
      <div className="mx-auto max-w-6xl">
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
          </div>
          <div className="flex items-center gap-2 rounded-full border border-border bg-background/75 px-3 py-1.5 text-sm shadow-sm backdrop-blur">
            <Server
              className="size-4 text-muted-foreground"
              aria-hidden="true"
            />
            {health?.status === "ok" ? (
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
              <span className="border-l border-border pl-2 text-xs text-muted-foreground">
                {
                  {
                    idle: "事件同步待命",
                    connecting: "正在建立事件同步",
                    connected: "事件同步已连接",
                    reconnecting: "正在重连事件同步",
                  }[eventConnection]
                }
              </span>
            )}
          </div>
        </header>

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
                  return (
                    <button
                      key={item.project.id}
                      type="button"
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
                {browsedChapter && (
                  <ChapterReader
                    chapter={browsedChapter}
                    activeChapterId={
                      preparation?.workflow.activeChapterId ?? null
                    }
                    onBackToActive={() => setBrowsed(null)}
                    onError={setError}
                  />
                )}
                <VersionPanel
                  project={selectedProject.project}
                  selectedVersion={selectedProject.selectedVersion}
                  onChanged={applyProjectBundle}
                  onError={setError}
                />
                <PreparationPanel
                  key={selectedProject.selectedVersion.id}
                  versionId={selectedProject.selectedVersion.id}
                  preparation={preparation}
                  loading={preparationLoading}
                  onChanged={applyPreparationSnapshot}
                  onError={setError}
                />
                {preparation?.workflow.stage === "writing_chapter_ready" && (
                  <ChapterReadyPanel
                    versionId={selectedProject.selectedVersion.id}
                    preparation={preparation}
                    onChanged={applyPreparationSnapshot}
                    onError={setError}
                  />
                )}
                {(preparation?.workflow.stage === "writing_outline_generating" ||
                  preparation?.workflow.stage === "writing_outline_reviewing" ||
                  preparation?.workflow.stage === "writing_drafting") && (
                  <ChapterOutlinePanel
                    versionId={selectedProject.selectedVersion.id}
                    preparation={preparation}
                    onChanged={applyPreparationSnapshot}
                    onError={setError}
                  />
                )}
                {(preparation?.workflow.stage === "writing_drafting" ||
                  preparation?.workflow.stage === "writing_chapter_reviewing" ||
                  preparation?.workflow.stage === "writing_chapter_done" ||
                  preparation?.workflow.stage === "completed") && (
                  <ChapterBodyPanel
                    versionId={selectedProject.selectedVersion.id}
                    preparation={preparation}
                    onChanged={applyPreparationSnapshot}
                    onError={setError}
                  />
                )}
              </>
            )}
          </div>

          <aside className="space-y-6">
            {selectedProject && chapters.length > 0 && (
              <ChapterDirectory
                chapters={chapters}
                activeChapterId={preparation?.workflow.activeChapterId ?? null}
                browsedChapterId={browsedChapterId}
                onSelect={(chapterId) =>
                  setBrowsed(
                    selectedVersionId ? { versionId: selectedVersionId, chapterId } : null
                  )
                }
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
            <ModelSettingsPanel refreshToken={settingsRefreshToken} />
            <ProjectSettingsPanel
              projectId={selectedProject?.project.id ?? null}
              refreshToken={settingsRefreshToken}
            />
            <PromptSettingsPanel
              projectId={selectedProject?.project.id ?? null}
              refreshToken={settingsRefreshToken}
            />
          </aside>
        </section>
      </div>
    </main>
  )
}

export default App
