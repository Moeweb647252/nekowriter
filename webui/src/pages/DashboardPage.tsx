import { useQuery } from "@tanstack/react-query"
import { Link } from "react-router"

import { AppFrame } from "@/components/AppFrame"
import { DashboardPanel } from "@/components/DashboardPanel"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { useEventSync } from "@/lib/events"
import { getDashboard, listProjects, type DashboardTask } from "@/lib/api"

const taskCategoryLabels: Record<DashboardTask["category"], string> = {
  writing: "写作",
  review: "审核",
  user_input: "等待你的回答",
  attention: "需要处理",
}

function taskStageLabel(task: DashboardTask) {
  return task.reviewTarget ?? task.stage.replace(/^review\./, "")
}

export function DashboardPage() {
  const { connection } = useEventSync()
  const projects = useQuery({
    queryKey: ["projects"],
    queryFn: ({ signal }) => listProjects(signal),
  })
  const tasks = useQuery({
    queryKey: ["dashboard", "tasks"],
    queryFn: ({ signal }) =>
      getDashboard(-new Date().getTimezoneOffset(), 7, undefined, signal),
  })

  return (
    <AppFrame
      title="任务概览"
      description="本地任务、Token 与正文进度会随事件同步更新。"
      actions={
        <Badge variant={connection === "connected" ? "secondary" : "outline"}>
          {connection === "connected" ? "已同步" : "正在同步"}
        </Badge>
      }
    >
      <div className="mx-auto flex w-full max-w-5xl flex-col gap-6 px-4 py-6 sm:px-6 lg:px-8">
        <DashboardPanel projectId={null} />
        <Card>
          <CardHeader>
            <CardTitle>待处理任务</CardTitle>
            <CardDescription>
              任务来自当前可操作的 Run；打开后会回到对应稿本的当前工作流阶段。
            </CardDescription>
          </CardHeader>
          <CardContent>
            {tasks.isPending ? (
              <p className="text-sm text-muted-foreground">正在读取任务…</p>
            ) : null}
            {tasks.data?.jobs.tasks.length === 0 ? (
              <p className="text-sm text-muted-foreground">当前没有需要处理的任务。</p>
            ) : null}
            {tasks.data?.jobs.tasks.length ? (
              <ul className="flex flex-col gap-2">
                {tasks.data.jobs.tasks.map((task) => {
                  const project = projects.data?.find(
                    (item) => item.project.id === task.projectId
                  )
                  return (
                    <li
                      className="flex flex-col gap-2 rounded-xl border p-3 sm:flex-row sm:items-center"
                      key={task.runId}
                    >
                      <div className="min-w-0 flex-1">
                        <div className="flex flex-wrap items-center gap-2">
                          <Badge variant="outline">
                            {taskCategoryLabels[task.category]}
                          </Badge>
                          <span className="text-sm font-medium">
                            {taskStageLabel(task)}
                          </span>
                        </div>
                        <p className="mt-1 truncate text-xs text-muted-foreground">
                          {project?.selectedVersion.novelTitle ?? task.versionId}
                        </p>
                      </div>
                      <Button
                        render={
                          <Link
                            to={`/works/${task.projectId}/versions/${task.versionId}`}
                          />
                        }
                        size="sm"
                        variant="outline"
                      >
                        打开
                      </Button>
                    </li>
                  )
                })}
              </ul>
            ) : null}
          </CardContent>
        </Card>
        {projects.data?.length ? (
          <section className="flex flex-col gap-3">
            <h2 className="font-heading text-base font-medium">继续处理</h2>
            <div className="flex flex-wrap gap-2">
              {projects.data
                .filter((item) => item.workflow.stage !== "completed")
                .slice(0, 8)
                .map((item) => (
                  <Button
                    key={item.project.id}
                    render={
                      <Link
                        to={`/works/${item.project.id}/versions/${item.selectedVersion.id}`}
                      />
                    }
                    size="sm"
                    variant="outline"
                  >
                    {item.selectedVersion.novelTitle}
                  </Button>
                ))}
            </div>
          </section>
        ) : null}
      </div>
    </AppFrame>
  )
}
