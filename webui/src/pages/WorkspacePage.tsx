import { useMemo, useState } from "react"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import {
  ArrowLeft,
  FolderKanban,
  Menu,
  PanelBottom,
  Settings2,
} from "lucide-react"
import { Link, useNavigate, useParams, useSearchParams } from "react-router"

import { ChapterBodyPanel } from "@/components/ChapterBodyPanel"
import { ChapterDirectory } from "@/components/ChapterDirectory"
import { ChapterOutlinePanel } from "@/components/ChapterOutlinePanel"
import { ChapterReadyPanel } from "@/components/ChapterReadyPanel"
import { GenerationPreview } from "@/components/GenerationPreview"
import { ManuscriptReader } from "@/components/ManuscriptReader"
import { PreparationPanel } from "@/components/PreparationPanel"
import { VersionPanel } from "@/components/VersionPanel"
import { WorkspaceControls } from "@/components/WorkspaceControls"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from "@/components/ui/resizable"
import { ScrollArea } from "@/components/ui/scroll-area"
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet"
import { Skeleton } from "@/components/ui/skeleton"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { useIsDesktop } from "@/hooks/use-is-desktop"
import { useEventSync } from "@/lib/events"
import {
  getChapters,
  getHealth,
  getPreparation,
  getVersionSnapshot,
  type PreparationSnapshot,
  type WorkflowStage,
} from "@/lib/api"

const stageLabels: Record<WorkflowStage, string> = {
  initializing: "设定作品目标",
  preparing_generating: "正在生成准备材料",
  preparing_reviewing: "审核准备材料",
  preparing_revising: "正在修订准备材料",
  writing_chapter_ready: "设置本章指导",
  writing_outline_generating: "正在生成本章细纲",
  writing_outline_reviewing: "审核本章细纲",
  writing_drafting: "正在写作正文",
  writing_chapter_reviewing: "审核本章终稿",
  writing_chapter_done: "推进到下一章",
  completed: "作品已完本",
}

function stageDescription(stage: WorkflowStage) {
  switch (stage) {
    case "initializing":
      return "先写下这部作品想抵达的地方，准备阶段会据此建立可审核的创作基础。"
    case "preparing_generating":
    case "preparing_revising":
      return "模型正在处理可验证的准备材料。已保存的内容仍以本地快照为准。"
    case "preparing_reviewing":
      return "逐项确认背景、资料、人物和粗纲，全部通过后才会开始章节写作。"
    case "writing_chapter_ready":
      return "为当前章节补充可选指导，然后开始生成细纲。"
    case "writing_outline_generating":
    case "writing_outline_reviewing":
      return "当前章节的细纲决定正文写作边界与审核目标。"
    case "writing_drafting":
      return "正文会通过工具调用持续提交；可以给出反馈、引用或暂停。"
    case "writing_chapter_reviewing":
      return "确认终稿后，才会将这一章计入已完成进度。"
    case "writing_chapter_done":
      return "本章已完成。确认后进入下一章，或在最后一章完成整部作品。"
    case "completed":
      return "写作流程已经收束。可切换到阅读视图查看整部作品。"
  }
}

function WorkspaceLoading() {
  return (
    <div className="flex min-h-svh flex-col">
      <header className="flex h-14 items-center gap-3 border-b px-4">
        <Skeleton className="size-7" />
        <Skeleton className="h-5 w-48" />
      </header>
      <div className="flex flex-1 items-center justify-center">
        <div className="flex w-full max-w-2xl flex-col gap-4 px-6">
          <Skeleton className="h-9 w-2/3" />
          <Skeleton className="h-5 w-full" />
          <Skeleton className="h-48 w-full" />
        </div>
      </div>
    </div>
  )
}

function Directory({
  versionId,
  onOpenChapter,
}: {
  versionId: string
  onOpenChapter: (chapterId: string) => void
}) {
  const chapters = useQuery({
    queryKey: ["chapters", versionId],
    queryFn: ({ signal }) => getChapters(versionId, signal),
  })
  const preparation = useQuery({
    queryKey: ["preparation", versionId],
    queryFn: ({ signal }) => getPreparation(versionId, signal),
  })

  if (chapters.isPending || preparation.isPending) {
    return <Skeleton className="h-80 w-full" />
  }
  if (!chapters.data || !preparation.data) {
    return (
      <p className="px-3 py-4 text-sm text-muted-foreground">
        暂时无法读取章节目录。
      </p>
    )
  }
  return (
    <ChapterDirectory
      activeChapterId={preparation.data.workflow.activeChapterId}
      chapters={chapters.data}
      onSelect={onOpenChapter}
      readingChapterId={null}
    />
  )
}

function WorkflowDock({
  dataDir,
  versionId,
  preparation,
  onChanged,
  onError,
}: {
  dataDir?: string
  versionId: string
  preparation: PreparationSnapshot
  onChanged: (snapshot: PreparationSnapshot) => void
  onError: (message: string) => void
}) {
  const stage = preparation.workflow.stage
  const isPreparationStage = [
    "initializing",
    "preparing_generating",
    "preparing_reviewing",
    "preparing_revising",
  ].includes(stage)

  return (
    <section className="flex h-full min-h-0 flex-col bg-card">
      <div className="flex shrink-0 items-center gap-2 border-b px-4 py-3 sm:px-6">
        <PanelBottom aria-hidden="true" />
        <div className="min-w-0 flex-1">
          <h2 className="font-heading text-sm font-medium">协作 Dock</h2>
          <p className="truncate text-xs text-muted-foreground">
            {stageLabels[stage]}
          </p>
        </div>
        <Badge variant="outline">
          状态 {preparation.workflow.stateRevision}
        </Badge>
      </div>
      <ScrollArea className="min-h-0 flex-1">
        <div className="px-4 pb-6 sm:px-6">
          {isPreparationStage ? (
            <PreparationPanel
              dataDir={dataDir}
              loading={false}
              onChanged={onChanged}
              onError={onError}
              preparation={preparation}
              versionId={versionId}
            />
          ) : null}
          {stage === "writing_chapter_ready" ? (
            <ChapterReadyPanel
              dataDir={dataDir}
              onChanged={onChanged}
              onError={onError}
              preparation={preparation}
              versionId={versionId}
            />
          ) : null}
          {[
            "writing_outline_generating",
            "writing_outline_reviewing",
            "writing_drafting",
          ].includes(stage) ? (
            <ChapterOutlinePanel
              dataDir={dataDir}
              onChanged={onChanged}
              onError={onError}
              preparation={preparation}
              versionId={versionId}
            />
          ) : null}
          {[
            "writing_drafting",
            "writing_chapter_reviewing",
            "writing_chapter_done",
            "completed",
          ].includes(stage) ? (
            <ChapterBodyPanel
              dataDir={dataDir}
              onChanged={onChanged}
              onError={onError}
              preparation={preparation}
              versionId={versionId}
            />
          ) : null}
        </div>
      </ScrollArea>
    </section>
  )
}

export function WorkspacePage() {
  const { projectId, versionId } = useParams()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [searchParams, setSearchParams] = useSearchParams()
  const isDesktop = useIsDesktop()
  const { previews } = useEventSync()
  const [directoryOpen, setDirectoryOpen] = useState(false)
  const [versionsOpen, setVersionsOpen] = useState(false)
  const [notice, setNotice] = useState<string | null>(null)

  const snapshot = useQuery({
    enabled: Boolean(versionId),
    queryKey: ["version", versionId],
    queryFn: ({ signal }) => getVersionSnapshot(versionId!, signal),
  })
  const preparation = useQuery({
    enabled: Boolean(versionId),
    queryKey: ["preparation", versionId],
    queryFn: ({ signal }) => getPreparation(versionId!, signal),
  })
  const chapters = useQuery({
    enabled: Boolean(versionId),
    queryKey: ["chapters", versionId],
    queryFn: ({ signal }) => getChapters(versionId!, signal),
  })
  const health = useQuery({
    queryKey: ["health"],
    queryFn: ({ signal }) => getHealth(signal),
  })

  const view = searchParams.get("view") === "read" ? "read" : "write"
  const requestedChapterId = searchParams.get("chapter")
  const readingTarget = useMemo(
    () =>
      requestedChapterId
        ? { chapterId: requestedChapterId, nonce: requestedChapterId.length }
        : null,
    [requestedChapterId]
  )

  function setView(next: "write" | "read") {
    setSearchParams((current) => {
      const result = new URLSearchParams(current)
      if (next === "read") {
        result.set("view", "read")
      } else {
        result.delete("view")
        result.delete("chapter")
      }
      return result
    })
  }

  function openChapter(chapterId: string) {
    setDirectoryOpen(false)
    setSearchParams({ view: "read", chapter: chapterId })
  }

  function applyPreparation(next: PreparationSnapshot) {
    if (!versionId) return
    queryClient.setQueryData(["preparation", versionId], next)
    void queryClient.invalidateQueries({ queryKey: ["version", versionId] })
    void queryClient.invalidateQueries({ queryKey: ["projects"] })
    void queryClient.invalidateQueries({ queryKey: ["chapters", versionId] })
    void queryClient.invalidateQueries({ queryKey: ["dashboard"] })
  }

  if (!projectId || !versionId || snapshot.isPending || preparation.isPending) {
    return <WorkspaceLoading />
  }

  if (
    snapshot.isError ||
    preparation.isError ||
    !snapshot.data ||
    !preparation.data
  ) {
    const error = snapshot.error ?? preparation.error
    return (
      <div className="flex min-h-svh items-center justify-center p-6">
        <Alert className="max-w-lg" variant="destructive">
          <AlertTitle>无法打开工作台</AlertTitle>
          <AlertDescription>
            {error instanceof Error
              ? error.message
              : "稿本不存在或本地服务没有返回状态。"}
          </AlertDescription>
        </Alert>
      </div>
    )
  }

  const current = snapshot.data
  const currentPreparation = preparation.data
  const previewBuffers = Object.values(previews).filter(
    (preview) => preview.versionId === versionId
  )

  return (
    <div className="flex min-h-svh flex-col bg-background">
      <a
        className="sr-only fixed top-3 left-3 z-50 rounded-md bg-primary px-3 py-2 text-sm font-medium text-primary-foreground focus:not-sr-only focus-visible:ring-3 focus-visible:ring-ring focus-visible:outline-none"
        href="#workspace-main"
        onClick={(event) => {
          event.preventDefault()
          document.getElementById("workspace-main")?.focus()
        }}
      >
        跳到主要内容
      </a>
      <header className="flex min-h-14 shrink-0 items-center gap-2 border-b bg-background/90 px-3 backdrop-blur sm:px-4">
        <Button
          aria-label="返回作品库"
          render={<Link to="/works" />}
          size="icon-sm"
          variant="ghost"
        >
          <ArrowLeft />
        </Button>
        <div className="hidden min-w-0 items-center gap-1 text-sm text-muted-foreground sm:flex">
          <Link className="truncate hover:text-foreground" to="/works">
            作品库
          </Link>
          <span>/</span>
          <span className="truncate">{current.project.name}</span>
        </div>
        <div className="min-w-0 flex-1">
          <h1 className="truncate font-heading text-sm font-medium sm:text-base">
            {current.version.novelTitle}
          </h1>
          <p className="truncate text-xs text-muted-foreground">
            {current.version.label} ·{" "}
            {stageLabels[currentPreparation.workflow.stage]}
          </p>
        </div>
        <Badge className="hidden sm:inline-flex" variant="outline">
          {currentPreparation.workflow.execution}
        </Badge>
        <Button
          aria-label="打开章节目录"
          className="lg:hidden"
          onClick={() => setDirectoryOpen(true)}
          size="icon-sm"
          variant="ghost"
        >
          <Menu />
        </Button>
        <ToggleGroup
          className="hidden sm:flex"
          onValueChange={(values) => {
            const next = values[0]
            if (next === "write" || next === "read") setView(next)
          }}
          size="sm"
          value={[view]}
          variant="outline"
        >
          <ToggleGroupItem value="write">写作</ToggleGroupItem>
          <ToggleGroupItem value="read">阅读</ToggleGroupItem>
        </ToggleGroup>
        {view === "write" ? (
          <Button
            aria-label="打开稿本管理"
            onClick={() => setVersionsOpen(true)}
            size="icon-sm"
            variant="ghost"
          >
            <FolderKanban />
          </Button>
        ) : null}
        {view === "write" ? (
          <Button
            aria-label="打开项目设置"
            render={<Link to={`/settings?project=${projectId}`} />}
            size="icon-sm"
            variant="ghost"
          >
            <Settings2 />
          </Button>
        ) : null}
        <WorkspaceControls />
      </header>

      {notice ? (
        <div className="shrink-0 px-3 pt-3 sm:px-4">
          <Alert variant="destructive">
            <AlertTitle>操作未完成</AlertTitle>
            <AlertDescription>{notice}</AlertDescription>
          </Alert>
        </div>
      ) : null}

      <div className="flex min-h-0 flex-1">
        {view === "write" && isDesktop ? (
          <aside className="w-72 shrink-0 border-r p-3">
            <Directory onOpenChapter={openChapter} versionId={versionId} />
          </aside>
        ) : null}
        <main className="min-w-0 flex-1" id="workspace-main" tabIndex={-1}>
          {view === "read" ? (
            chapters.data ? (
              <ManuscriptReader
                activeChapterId={currentPreparation.workflow.activeChapterId}
                chapters={chapters.data}
                onBackToWriting={() => setView("write")}
                onError={setNotice}
                onIndexStale={() => {
                  void queryClient.invalidateQueries({
                    queryKey: ["chapters", versionId],
                  })
                }}
                onVisibleChapterChange={() => {}}
                target={readingTarget}
                versionId={versionId}
              />
            ) : (
              <WorkspaceLoading />
            )
          ) : (
            <>
              {isDesktop ? (
                <div className="h-[calc(100svh-3.5rem)]">
                  <ResizablePanelGroup orientation="vertical">
                    <ResizablePanel defaultSize="58%" minSize="30%">
                      <ScrollArea className="h-full">
                        <article className="mx-auto flex w-full max-w-3xl flex-col gap-6 px-6 py-10 lg:px-10">
                          <div className="flex flex-wrap items-center gap-2">
                            <Badge variant="outline">
                              {stageLabels[currentPreparation.workflow.stage]}
                            </Badge>
                            <span className="text-sm text-muted-foreground">
                              {current.version.label}
                            </span>
                          </div>
                          <div className="flex flex-col gap-3">
                            <h2 className="font-serif text-3xl leading-tight font-medium tracking-tight sm:text-4xl">
                              {current.version.novelTitle}
                            </h2>
                            <p className="max-w-2xl text-base leading-8 text-muted-foreground">
                              {stageDescription(
                                currentPreparation.workflow.stage
                              )}
                            </p>
                          </div>
                          <dl className="grid grid-cols-2 gap-3 text-sm sm:grid-cols-4">
                            <div>
                              <dt className="text-xs text-muted-foreground">
                                当前阶段
                              </dt>
                              <dd className="mt-1 font-medium">
                                {stageLabels[currentPreparation.workflow.stage]}
                              </dd>
                            </div>
                            <div>
                              <dt className="text-xs text-muted-foreground">
                                执行状态
                              </dt>
                              <dd className="mt-1 font-medium">
                                {currentPreparation.workflow.execution}
                              </dd>
                            </div>
                            <div>
                              <dt className="text-xs text-muted-foreground">
                                当前章节
                              </dt>
                              <dd className="mt-1 font-medium">
                                {currentPreparation.workflow.activeChapterId
                                  ? "进行中"
                                  : "尚未进入章节"}
                              </dd>
                            </div>
                            <div>
                              <dt className="text-xs text-muted-foreground">
                                目录
                              </dt>
                              <dd className="mt-1 font-medium">
                                {chapters.data?.length ?? 0} 章
                              </dd>
                            </div>
                          </dl>
                          <GenerationPreview buffers={previewBuffers} />
                        </article>
                      </ScrollArea>
                    </ResizablePanel>
                    <ResizableHandle withHandle />
                    <ResizablePanel
                      defaultSize="42%"
                      maxSize="70%"
                      minSize="20%"
                    >
                      <WorkflowDock
                        dataDir={health.data?.dataDir}
                        onChanged={applyPreparation}
                        onError={setNotice}
                        preparation={currentPreparation}
                        versionId={versionId}
                      />
                    </ResizablePanel>
                  </ResizablePanelGroup>
                </div>
              ) : (
                <div className="flex min-h-[calc(100svh-3.5rem)] flex-col">
                  <article className="flex flex-col gap-4 px-5 pt-8 pb-4">
                    <Badge className="w-fit" variant="outline">
                      {stageLabels[currentPreparation.workflow.stage]}
                    </Badge>
                    <h2 className="font-serif text-3xl leading-tight font-medium">
                      {current.version.novelTitle}
                    </h2>
                    <p className="text-sm leading-7 text-muted-foreground">
                      {stageDescription(currentPreparation.workflow.stage)}
                    </p>
                    <GenerationPreview buffers={previewBuffers} />
                  </article>
                  <div className="min-h-0 flex-1 border-t">
                    <WorkflowDock
                      dataDir={health.data?.dataDir}
                      onChanged={applyPreparation}
                      onError={setNotice}
                      preparation={currentPreparation}
                      versionId={versionId}
                    />
                  </div>
                </div>
              )}
            </>
          )}
        </main>
      </div>

      {!isDesktop ? (
        <Sheet open={directoryOpen} onOpenChange={setDirectoryOpen}>
          <SheetContent side="left">
            <SheetHeader>
              <SheetTitle>章节目录</SheetTitle>
              <SheetDescription>打开任意章节进入阅读视图。</SheetDescription>
            </SheetHeader>
            <div className="min-h-0 flex-1 overflow-auto px-4 pb-6">
              <Directory onOpenChapter={openChapter} versionId={versionId} />
            </div>
          </SheetContent>
        </Sheet>
      ) : null}

      <Sheet open={versionsOpen} onOpenChange={setVersionsOpen}>
        <SheetContent>
          <SheetHeader>
            <SheetTitle>稿本管理</SheetTitle>
            <SheetDescription>
              创建、复制或切换这部作品的稿本。
            </SheetDescription>
          </SheetHeader>
          <ScrollArea className="min-h-0 flex-1 px-4 pb-6">
            <VersionPanel
              onChanged={(bundle) => {
                setVersionsOpen(false)
                void queryClient.invalidateQueries({ queryKey: ["projects"] })
                navigate(
                  `/works/${bundle.project.id}/versions/${bundle.version.id}`
                )
              }}
              onDeleted={() => {
                setVersionsOpen(false)
                void queryClient.invalidateQueries({ queryKey: ["projects"] })
                navigate("/works")
              }}
              onError={setNotice}
              project={current.project}
              selectedVersion={current.version}
            />
          </ScrollArea>
        </SheetContent>
      </Sheet>
    </div>
  )
}
