import { useState } from "react"
import { useQuery } from "@tanstack/react-query"
import {
  CircleAlert,
  Coins,
  Gauge,
  LoaderCircle,
  RefreshCw,
  Type,
} from "lucide-react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import {
  ApiError,
  getDashboard,
  type Dashboard,
  type UsageUnknownReason,
} from "@/lib/api"

const rangeOptions = [7, 30, 90] as const

const roleLabels: Record<string, string> = {
  writing: "写作",
  review: "审核",
  embedding: "Embedding（独立列示）",
}

const unknownReasonLabels: Record<UsageUnknownReason, string> = {
  provider_silent: "供应商未在响应中报告用量",
  request_failed: "请求失败，没有收到可用用量",
  request_cancelled: "请求被取消，从未收到用量",
  pending: "请求尚未结束",
  invalid_report: "用量报告无法解析",
}

const runStatusLabels: Record<string, string> = {
  queued: "排队",
  running: "运行中",
  waiting: "等待中",
  paused: "已暂停",
  interrupted: "已中断",
  failed: "失败",
  succeeded: "已完成",
  stopped: "已停止",
}

const embeddingKindLabels: Record<string, string> = {
  embedding_index: "知识索引",
  embedding_query: "语义检索",
  embedding_connection_test: "连接测试",
}

const systemOperationStatusLabels: Record<string, string> = {
  succeeded: "成功",
  failed: "失败",
  unknown: "结果未知",
}

/** Run 的阶段名去掉内部前缀后的中文说法。 */
const taskStageLabels: Record<string, string> = {
  "preparation.background": "准备·背景",
  "preparation.research": "准备·研究",
  "preparation.characters": "准备·人物",
  "preparation.outline": "准备·大纲",
  "chapter.outline": "本章细纲",
  "chapter.edit": "正文编辑",
  "chapter.final": "整章终稿",
}

function taskStageLabel(stage: string) {
  return taskStageLabels[stage.replace(/^review\./, "")] ?? stage
}

const jobLabels: Array<[keyof Dashboard["jobs"], string]> = [
  ["queued", "排队"],
  ["running", "模型运行"],
  ["waitingUser", "等待用户"],
  ["waitingReview", "等待审核"],
  ["paused", "已暂停"],
  ["needsAttention", "需要处理"],
]

function errorText(error: unknown, fallback: string) {
  return error instanceof ApiError || error instanceof Error
    ? error.message
    : fallback
}

function formatDay(dayStartMs: string) {
  return new Intl.DateTimeFormat("zh-CN", {
    month: "numeric",
    day: "numeric",
  }).format(Number(dayStartMs))
}

function formatCount(value: number) {
  return new Intl.NumberFormat("zh-CN").format(value)
}

function formatTimestamp(timestampMs: string) {
  return new Intl.DateTimeFormat("zh-CN", {
    dateStyle: "short",
    timeStyle: "short",
  }).format(Number(timestampMs))
}

/** 未知用量按原因列出，使「用量未知」不再是一个无法追问的数字。 */
function UnknownUsageList({
  entries,
}: {
  entries: Array<{ reason: UsageUnknownReason; requests: number }>
}) {
  if (entries.length === 0) return null
  return (
    <ul className="space-y-1 text-xs text-muted-foreground">
      {entries.map((entry) => (
        <li key={entry.reason}>
          用量未知·{unknownReasonLabels[entry.reason] ?? entry.reason}：
          {formatCount(entry.requests)} 次
        </li>
      ))}
    </ul>
  )
}

/** 以最大值为基准把一天画成一根柱，负值用另一种颜色表示。 */
function DailyBars({
  days,
  pick,
}: {
  days: Array<{ dayStartMs: string; value: number }>
  pick: (value: number) => string
}) {
  // 全零时留白会被误读成一块空白图；直接说明比画一排看不见的柱子诚实。
  if (days.every((day) => day.value === 0)) {
    return (
      <p className="text-xs text-muted-foreground">
        该范围内没有可绘制的数据。
      </p>
    )
  }
  const peak = Math.max(1, ...days.map((day) => Math.abs(day.value)))
  return (
    <ul className="flex h-24 items-end gap-1">
      {days.map((day) => (
        <li
          key={day.dayStartMs}
          className="flex flex-1 flex-col items-center gap-1"
          title={`${formatDay(day.dayStartMs)}：${pick(day.value)}`}
        >
          <span
            className={`w-full rounded-t ${
              day.value < 0 ? "bg-destructive/70" : "bg-primary/70"
            }`}
            style={{
              height: `${Math.max(2, (Math.abs(day.value) / peak) * 72)}px`,
            }}
          />
          <span className="text-[10px] text-muted-foreground">
            {formatDay(day.dayStartMs)}
          </span>
        </li>
      ))}
    </ul>
  )
}

interface DashboardPanelProps {
  projectId: string | null
}

export function DashboardPanel({ projectId }: DashboardPanelProps) {
  const [days, setDays] = useState<number>(7)
  const dashboardQuery = useQuery({
    queryKey: ["dashboard", projectId, days],
    queryFn: ({ signal }) =>
      getDashboard(
        -new Date().getTimezoneOffset(),
        days,
        projectId ?? undefined,
        signal
      ),
  })
  const dashboard = dashboardQuery.data
  const pending = dashboardQuery.isFetching
  const notice = dashboardQuery.isError
    ? errorText(dashboardQuery.error, "无法读取统计")
    : null

  function handleRefresh() {
    void dashboardQuery.refetch()
  }

  const tokens = dashboard?.tokens
  const words = dashboard?.words

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Gauge className="size-4" aria-hidden="true" />
          概览
        </CardTitle>
        <CardDescription>
          全部指标由已封存请求、Run 行与正文修订实时回算，不保存累计值。
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-5">
        <div className="flex flex-wrap items-center gap-2">
          <select
            aria-label="统计范围"
            className="h-9 rounded-xl border border-input bg-background px-3 text-sm"
            value={days}
            onChange={(event) => setDays(Number(event.target.value))}
          >
            {rangeOptions.map((option) => (
              <option key={option} value={option}>
                近 {option} 天
              </option>
            ))}
          </select>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={pending}
            onClick={() => void handleRefresh()}
          >
            {pending ? (
              <LoaderCircle className="animate-spin" aria-hidden="true" />
            ) : (
              <RefreshCw aria-hidden="true" />
            )}
            刷新
          </Button>
          <span className="text-xs text-muted-foreground">
            {projectId ? "当前项目" : "全部可见稿本"}
          </span>
        </div>

        {notice ? (
          <p
            className="flex items-center gap-2 text-sm text-destructive"
            role="alert"
          >
            <CircleAlert className="size-4" aria-hidden="true" />
            {notice}
          </p>
        ) : null}

        {dashboardQuery.isPending ? (
          <p className="text-sm text-muted-foreground">正在读取统计…</p>
        ) : null}

        {tokens ? (
          <section className="space-y-2">
            <h3 className="flex items-center gap-2 text-sm font-medium">
              <Coins className="size-4" aria-hidden="true" />
              Token 用量
            </h3>
            <div className="grid grid-cols-2 gap-2 text-sm sm:grid-cols-3">
              <span>输入 {formatCount(tokens.inputTokens)}</span>
              <span>输出 {formatCount(tokens.outputTokens)}</span>
              <span>
                缓存命中（子集）{formatCount(tokens.cachedInputTokens)}
              </span>
              <span>思考（子集）{formatCount(tokens.reasoningTokens)}</span>
              <span>请求 {formatCount(tokens.requestCount)}</span>
              <span>用量未知 {formatCount(tokens.unknownUsageRequests)}</span>
            </div>
            <UnknownUsageList entries={tokens.unknownUsage} />
            {tokens.byRole.length > 0 ? (
              <ul className="space-y-1 text-xs text-muted-foreground">
                {tokens.byRole.map((role) => (
                  <li key={role.role}>
                    {roleLabels[role.role] ?? role.role}：输入{" "}
                    {formatCount(role.inputTokens)} · 输出{" "}
                    {formatCount(role.outputTokens)} · 请求{" "}
                    {formatCount(role.requestCount)}
                  </li>
                ))}
              </ul>
            ) : (
              <p className="text-xs text-muted-foreground">
                该范围内还没有模型请求。
              </p>
            )}
            {tokens.daily.length > 0 ? (
              <>
                <DailyBars
                  days={tokens.daily.map((day) => ({
                    dayStartMs: day.dayStartMs,
                    value: day.inputTokens + day.outputTokens,
                  }))}
                  pick={(value) => `${formatCount(value)} token`}
                />
                {tokens.embedding.requestCount > 0 &&
                tokens.daily.some(
                  (day) => day.inputTokens + day.outputTokens !== 0
                ) ? (
                  <p className="text-xs text-muted-foreground">
                    这里的按日柱只包含对话请求；向量用量在下面单独画。
                  </p>
                ) : null}
              </>
            ) : null}
          </section>
        ) : null}

        {tokens && tokens.embedding.requestCount > 0 ? (
          <section className="space-y-2">
            <h3 className="text-sm font-medium">Embedding 用量</h3>
            <p className="text-xs text-muted-foreground">
              索引与检索的向量请求没有输出侧，因此只统计输入；它们单独列示，不混进上面的对话
              Token。
              {projectId
                ? `归属${tokens.embedding.scope === "global" ? "全局知识库" : tokens.embedding.scope}，不受当前项目筛选影响。`
                : ""}
            </p>
            <div className="grid grid-cols-2 gap-2 text-sm sm:grid-cols-3">
              <span>输入 {formatCount(tokens.embedding.inputTokens)}</span>
              <span>请求 {formatCount(tokens.embedding.requestCount)}</span>
              <span>
                用量未知 {formatCount(tokens.embedding.unknownUsageRequests)}
              </span>
            </div>
            <UnknownUsageList entries={tokens.embedding.unknownUsage} />
            <ul className="space-y-1 text-xs text-muted-foreground">
              {tokens.embedding.byKind.map((kind) => (
                <li key={kind.kind}>
                  {embeddingKindLabels[kind.kind] ?? kind.kind}：输入{" "}
                  {formatCount(kind.inputTokens)} · 请求{" "}
                  {formatCount(kind.requestCount)}
                </li>
              ))}
            </ul>
            {tokens.embedding.daily.length > 0 ? (
              <DailyBars
                days={tokens.embedding.daily.map((day) => ({
                  dayStartMs: day.dayStartMs,
                  value: day.inputTokens,
                }))}
                pick={(value) => `${formatCount(value)} token`}
              />
            ) : null}
          </section>
        ) : null}

        {tokens && tokens.systemOperations.requestCount > 0 ? (
          <section className="space-y-2">
            <h3 className="text-sm font-medium">系统操作</h3>
            <p className="text-xs text-muted-foreground">
              连接测试这类请求不属于任何稿本，因此单列，不混进对话
              Token；Embedding 的向量请求在下面单独列示。
            </p>
            <div className="grid grid-cols-2 gap-2 text-sm sm:grid-cols-3">
              <span>
                输入 {formatCount(tokens.systemOperations.inputTokens)}
              </span>
              <span>
                输出 {formatCount(tokens.systemOperations.outputTokens)}
              </span>
              <span>
                请求 {formatCount(tokens.systemOperations.requestCount)}
              </span>
            </div>
            <UnknownUsageList entries={tokens.systemOperations.unknownUsage} />
            <ul className="space-y-1 text-xs text-muted-foreground">
              {tokens.systemOperations.recent.map((operation) => (
                <li key={operation.id}>
                  {formatTimestamp(operation.createdAtMs)} · {operation.modelId}{" "}
                  ·{" "}
                  {systemOperationStatusLabels[operation.status] ??
                    operation.status}
                  {operation.usageStatus === "reported"
                    ? ` · 输入 ${formatCount(operation.inputTokens)} · 输出 ${formatCount(operation.outputTokens)}`
                    : " · 用量未知"}
                  {operation.errorKind ? ` · ${operation.errorKind}` : ""}
                </li>
              ))}
            </ul>
          </section>
        ) : null}

        {dashboard ? (
          <section className="space-y-2">
            <h3 className="text-sm font-medium">进行中任务</h3>
            <ul className="flex flex-wrap gap-2">
              {jobLabels.map(([key, label]) => (
                <li key={key}>
                  <Badge variant="secondary">
                    {label} {formatCount(dashboard.jobs[key] as number)}
                  </Badge>
                </li>
              ))}
            </ul>
            {dashboard.jobs.tasks.length > 0 ? (
              <ul className="space-y-1 text-xs text-muted-foreground">
                {dashboard.jobs.tasks.map((task) => (
                  <li key={task.runId}>
                    {task.category === "review" ? "审核" : "任务"} ·{" "}
                    {taskStageLabel(task.stage)}：
                    {runStatusLabels[task.status] ?? task.status}
                  </li>
                ))}
              </ul>
            ) : null}
          </section>
        ) : null}

        {words ? (
          <section className="space-y-2">
            <h3 className="flex items-center gap-2 text-sm font-medium">
              <Type className="size-4" aria-hidden="true" />
              正文净字数
            </h3>
            <div className="grid grid-cols-2 gap-2 text-sm">
              <span>区间净增 {formatCount(words.netIncrement)}</span>
              <span>当前总量 {formatCount(words.currentTotal)}</span>
            </div>
            {words.daily.length > 0 ? (
              <DailyBars
                days={words.daily.map((day) => ({
                  dayStartMs: day.dayStartMs,
                  value: day.netIncrement,
                }))}
                pick={(value) => `${formatCount(value)} 字`}
              />
            ) : null}
          </section>
        ) : null}
      </CardContent>
    </Card>
  )
}
