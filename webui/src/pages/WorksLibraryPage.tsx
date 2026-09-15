import { useState, type FormEvent } from "react"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import {
  ArrowRight,
  Ellipsis,
  FolderPlus,
  LoaderCircle,
  Trash2,
} from "lucide-react"
import { Link, useNavigate } from "react-router"

import { AppFrame } from "@/components/AppFrame"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import { Field, FieldGroup, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Skeleton } from "@/components/ui/skeleton"
import { useEventSync } from "@/lib/event-sync-context"
import {
  createProject,
  deleteProject,
  listProjects,
  type ProjectListItem,
  type WorkflowStage,
} from "@/lib/api"

const stageLabels: Record<WorkflowStage, string> = {
  initializing: "等待开始准备",
  preparing_generating: "正在准备",
  preparing_reviewing: "等待准备审核",
  preparing_revising: "正在修订准备内容",
  writing_chapter_ready: "等待章节指导",
  writing_outline_generating: "正在生成细纲",
  writing_outline_reviewing: "等待细纲审核",
  writing_drafting: "正在写正文",
  writing_chapter_reviewing: "等待终稿审核",
  writing_chapter_done: "本章已完成",
  completed: "已完本",
}

function workspacePath(item: Pick<ProjectListItem, "project" | "selectedVersion">) {
  return `/works/${item.project.id}/versions/${item.selectedVersion.id}`
}

function formatCount(value: number) {
  return new Intl.NumberFormat("zh-CN").format(value)
}

function formatUpdatedAt(value: string) {
  return new Intl.DateTimeFormat("zh-CN", {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(Number(value))
}

function ProjectCard({
  item,
  onDelete,
}: {
  item: ProjectListItem
  onDelete: (item: ProjectListItem) => void
}) {
  const progress = item.progress
  const destination = workspacePath(item)
  const isComplete = item.workflow.stage === "completed"

  return (
    <Card className="transition-shadow hover:shadow-md">
      <CardHeader>
        <CardTitle className="pr-8">
          <Link
            className="outline-none hover:underline focus-visible:rounded focus-visible:ring-3 focus-visible:ring-ring/50"
            to={destination}
          >
            {item.selectedVersion.novelTitle}
          </Link>
        </CardTitle>
        <CardDescription className="truncate">
          {item.project.name} · {item.selectedVersion.label}
        </CardDescription>
        <CardAction>
          <DropdownMenu>
            <DropdownMenuTrigger
              render={<Button aria-label={`打开《${item.selectedVersion.novelTitle}》菜单`} size="icon-sm" variant="ghost" />}
            >
              <Ellipsis />
            </DropdownMenuTrigger>
            <DropdownMenuContent>
              <DropdownMenuGroup>
                <DropdownMenuItem render={<Link to={destination} />}>
                  打开工作台
                </DropdownMenuItem>
              </DropdownMenuGroup>
              <DropdownMenuSeparator />
              <DropdownMenuGroup>
                <DropdownMenuItem
                  variant="destructive"
                  onClick={() => onDelete(item)}
                >
                  <Trash2 />
                  移到回收站
                </DropdownMenuItem>
              </DropdownMenuGroup>
            </DropdownMenuContent>
          </DropdownMenu>
        </CardAction>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant={isComplete ? "secondary" : "outline"}>
            {stageLabels[item.workflow.stage]}
          </Badge>
          <span className="text-xs text-muted-foreground">
            更新于 {formatUpdatedAt(item.project.updatedAtMs)}
          </span>
        </div>
        <dl className="grid grid-cols-2 gap-3 text-sm sm:grid-cols-4">
          <div>
            <dt className="text-xs text-muted-foreground">主线进度</dt>
            <dd className="mt-1 font-medium">
              {formatCount(progress.mainChapterApproved)} /{" "}
              {formatCount(progress.mainChapterTotal)} 章
            </dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">当前字数</dt>
            <dd className="mt-1 font-medium">
              {formatCount(progress.currentWordCount)} 字
            </dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">番外</dt>
            <dd className="mt-1 font-medium">
              {formatCount(progress.extraChapterTotal)} 章
            </dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">稿本</dt>
            <dd className="mt-1 truncate font-medium">
              {item.selectedVersion.label}
            </dd>
          </div>
        </dl>
      </CardContent>
      <CardFooter>
        <Button render={<Link to={destination} />} size="sm">
          {isComplete ? "阅读作品" : "继续创作"}
          <ArrowRight data-icon="inline-end" />
        </Button>
      </CardFooter>
    </Card>
  )
}

function ProjectCardSkeleton() {
  return (
    <Card>
      <CardHeader>
        <Skeleton className="h-5 w-40" />
        <Skeleton className="h-4 w-28" />
      </CardHeader>
      <CardContent>
        <Skeleton className="h-16 w-full" />
      </CardContent>
    </Card>
  )
}

export function WorksLibraryPage() {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const { connection } = useEventSync()
  const [createOpen, setCreateOpen] = useState(false)
  const [deleteTarget, setDeleteTarget] = useState<ProjectListItem | null>(null)
  const [projectName, setProjectName] = useState("")
  const [novelTitle, setNovelTitle] = useState("")
  const [versionLabel, setVersionLabel] = useState("初稿")
  const [notice, setNotice] = useState<string | null>(null)

  const projects = useQuery({
    queryKey: ["projects"],
    queryFn: ({ signal }) => listProjects(signal),
  })
  const create = useMutation({
    mutationFn: (input: {
      name: string
      novelTitle: string
      versionLabel?: string
    }) => createProject(input),
    onSuccess: (bundle) => {
      void queryClient.invalidateQueries({ queryKey: ["projects"] })
      setCreateOpen(false)
      setProjectName("")
      setNovelTitle("")
      setVersionLabel("初稿")
      navigate(`/works/${bundle.project.id}/versions/${bundle.version.id}`)
    },
    onError: (error) => {
      setNotice(error instanceof Error ? error.message : "项目未能创建")
    },
  })
  const remove = useMutation({
    mutationFn: (item: ProjectListItem) =>
      deleteProject(item.project.id, item.project.revision),
    onSuccess: () => {
      setDeleteTarget(null)
      void queryClient.invalidateQueries({ queryKey: ["projects"] })
      void queryClient.invalidateQueries({ queryKey: ["recycle"] })
    },
    onError: (error) => {
      setNotice(error instanceof Error ? error.message : "项目未能移到回收站")
    },
  })

  function submitProject(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setNotice(null)
    create.mutate({
      name: projectName,
      novelTitle,
      versionLabel: versionLabel.trim() || undefined,
    })
  }

  return (
    <AppFrame
      title="作品库"
      description="从这里进入正在创作的稿本。"
      actions={
        <div className="flex items-center gap-2">
          <Badge variant={connection === "connected" ? "secondary" : "outline"}>
            {connection === "connected" ? "已同步" : "正在同步"}
          </Badge>
          <Button onClick={() => setCreateOpen(true)} size="sm">
            <FolderPlus data-icon="inline-start" />
            新建作品
          </Button>
        </div>
      }
    >
      <div className="mx-auto flex w-full max-w-6xl flex-col gap-6 px-4 py-6 sm:px-6 lg:px-8">
        {notice ? (
          <Alert variant="destructive">
            <AlertTitle>操作未完成</AlertTitle>
            <AlertDescription>{notice}</AlertDescription>
          </Alert>
        ) : null}
        {projects.isError ? (
          <Alert variant="destructive">
            <AlertTitle>无法读取作品库</AlertTitle>
            <AlertDescription>
              {projects.error instanceof Error
                ? projects.error.message
                : "本地服务没有返回作品列表。"}
            </AlertDescription>
          </Alert>
        ) : null}
        {projects.isPending ? (
          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
            {Array.from({ length: 3 }, (_, index) => (
              <ProjectCardSkeleton key={index} />
            ))}
          </div>
        ) : null}
        {projects.data?.length ? (
          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
            {projects.data.map((item) => (
              <ProjectCard
                key={item.project.id}
                item={item}
                onDelete={setDeleteTarget}
              />
            ))}
          </div>
        ) : null}
        {projects.data && projects.data.length === 0 ? (
          <Empty className="min-h-80 border">
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <FolderPlus />
              </EmptyMedia>
              <EmptyTitle>从第一部作品开始</EmptyTitle>
              <EmptyDescription>
                只需填写作品和项目名称。首个稿本默认命名为“初稿”。
              </EmptyDescription>
            </EmptyHeader>
            <EmptyContent>
              <Button onClick={() => setCreateOpen(true)}>
                <FolderPlus data-icon="inline-start" />
                新建作品
              </Button>
            </EmptyContent>
          </Empty>
        ) : null}
      </div>

      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>新建作品</DialogTitle>
            <DialogDescription>
              项目用于整理稿本；第一份稿本会立刻进入工作台。
            </DialogDescription>
          </DialogHeader>
          <form className="flex flex-col gap-6" onSubmit={submitProject}>
            <FieldGroup>
              <Field>
                <FieldLabel htmlFor="project-name">项目名称</FieldLabel>
                <Input
                  id="project-name"
                  name="projectName"
                  autoComplete="off"
                  maxLength={120}
                  onChange={(event) => setProjectName(event.target.value)}
                  required
                  value={projectName}
                />
              </Field>
              <Field>
                <FieldLabel htmlFor="novel-title">作品标题</FieldLabel>
                <Input
                  id="novel-title"
                  name="novelTitle"
                  autoComplete="off"
                  maxLength={200}
                  onChange={(event) => setNovelTitle(event.target.value)}
                  required
                  value={novelTitle}
                />
              </Field>
              <Field>
                <FieldLabel htmlFor="version-label">首个稿本</FieldLabel>
                <Input
                  id="version-label"
                  name="versionLabel"
                  autoComplete="off"
                  maxLength={80}
                  onChange={(event) => setVersionLabel(event.target.value)}
                  value={versionLabel}
                />
              </Field>
            </FieldGroup>
            <DialogFooter>
              <Button
                disabled={create.isPending}
                onClick={() => setCreateOpen(false)}
                type="button"
                variant="outline"
              >
                取消
              </Button>
              <Button disabled={create.isPending} type="submit">
                {create.isPending ? (
                  <LoaderCircle className="animate-spin" data-icon="inline-start" />
                ) : (
                  <FolderPlus data-icon="inline-start" />
                )}
                创建并进入工作台
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      <Dialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open && !remove.isPending) setDeleteTarget(null)
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>移到回收站？</DialogTitle>
            <DialogDescription>
              {deleteTarget
                ? `《${deleteTarget.selectedVersion.novelTitle}》及其稿本会保留在本地回收站中，可在需要时恢复。`
                : ""}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              disabled={remove.isPending}
              onClick={() => setDeleteTarget(null)}
              variant="outline"
            >
              取消
            </Button>
            <Button
              disabled={!deleteTarget || remove.isPending}
              onClick={() => deleteTarget && remove.mutate(deleteTarget)}
              variant="destructive"
            >
              {remove.isPending ? (
                <LoaderCircle className="animate-spin" data-icon="inline-start" />
              ) : (
                <Trash2 data-icon="inline-start" />
              )}
              移到回收站
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </AppFrame>
  )
}
