import { useState } from "react"
import { CircleAlert, LoaderCircle, Play, Square } from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  continueWritingRun,
  stopWritingRun,
  type WorkflowSnapshot,
} from "@/lib/api"
import { usePrimaryAction } from "@/lib/shortcuts"

type WritingRecoveryPanelProps = {
  workflow: WorkflowSnapshot
  disabled?: boolean
  onContinued: () => Promise<void>
  onError: (message: string) => void
}

function recoveryCopy(workflow: WorkflowSnapshot) {
  switch (workflow.execution) {
    case "interrupted":
      return {
        title: "上次运行已中断",
        description:
          "未形成稳定结果的模型请求已标记为中断，已提交的内容、工具结果和未投递意见仍会保留。",
      }
    case "failed":
      return {
        title: "模型请求未能完成",
        description:
          "请检查模型设置或网络后显式继续。系统会复用同一 Run 与上下文，不会重复应用已保存的编辑。",
      }
    case "stopped":
      if (workflow.waitReason === "user_stopped") {
        const preservedBoundary =
          workflow.resumeExecution === "waiting_answer"
            ? "待答问题仍会保留"
            : workflow.resumeExecution === "waiting_review"
              ? "待审提案仍需由你作出决定"
              : workflow.resumeExecution === "paused"
                ? "已暂停的工具边界仍会保留"
                : "已提交内容和未投递意见仍会保留"
        return {
          title: "本轮执行已停止",
          description: `已取消尚在本地执行的模型请求；${preservedBoundary}。显式继续后才会恢复这一 Run，并重新授予最多 100 次请求额度。`,
        }
      }
      if (workflow.waitReason === "request_limit") {
        return {
          title: "本轮请求额度已用完",
          description:
            "当前 Run 已达到本轮模型请求上限。显式继续会保留已保存内容、工具结果和审计记录，并再授予最多 100 次请求额度。",
        }
      }
      if (workflow.waitReason === "protocol_error_limit") {
        return {
          title: "连续协议错误已停止",
          description:
            "模型已连续三次返回不符合当前阶段协议的响应。原始响应和纠正记录仍在本地；检查模型兼容性或设置后，显式继续可重新开始一轮请求额度。",
        }
      }
      return {
        title: "本轮执行已停止",
        description:
          "继续会保留现有内容和审计记录，并重新授予这一 Run 一轮模型请求额度。",
      }
    default:
      return null
  }
}

function canStop(workflow: WorkflowSnapshot) {
  if (!workflow.activeRunId) return false
  if (
    [
      "queued",
      "request_in_flight",
      "tool_boundary",
      "waiting_answer",
      "paused",
    ].includes(workflow.execution)
  ) {
    return true
  }
  return (
    workflow.execution === "waiting_review" &&
    workflow.stage === "writing_drafting"
  )
}

export function WritingRecoveryPanel({
  workflow,
  disabled = false,
  onContinued,
  onError,
}: WritingRecoveryPanelProps) {
  const [pending, setPending] = useState(false)
  const copy = recoveryCopy(workflow)
  const stoppable = canStop(workflow)

  /**
   * 「显式继续」是这里唯一可绑定的主操作；「停止执行」是破坏性操作，只留按钮。
   * 光标在输入框里时由表单提交路径处理，这条只在没有表单承接时生效。
   */
  usePrimaryAction("writing-recovery-continue", {
    label: "显式继续",
    run: () => void handleContinue(),
    enabled:
      Boolean(copy) && !disabled && !pending && Boolean(workflow.activeRunId),
  })

  if ((!copy && !stoppable) || !workflow.activeRunId) return null

  async function handleContinue() {
    setPending(true)
    try {
      await continueWritingRun(workflow.activeRunId!, workflow.stateRevision)
      await onContinued()
    } catch (error) {
      onError(error instanceof Error ? error.message : "无法继续当前写作任务")
    } finally {
      setPending(false)
    }
  }

  async function handleStop() {
    setPending(true)
    try {
      await stopWritingRun(workflow.activeRunId!, workflow.stateRevision)
      await onContinued()
    } catch (error) {
      onError(error instanceof Error ? error.message : "无法停止当前写作任务")
    } finally {
      setPending(false)
    }
  }

  if (!copy) {
    return (
      <section className="mt-5 rounded-xl border border-rose-500/30 bg-rose-500/8 p-4">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div className="flex max-w-2xl items-start gap-3">
            <CircleAlert
              className="mt-0.5 size-5 shrink-0 text-rose-700 dark:text-rose-300"
              aria-hidden="true"
            />
            <div>
              <h3 className="text-sm font-semibold">停止本轮执行</h3>
              <p className="mt-1 text-xs leading-5 text-muted-foreground">
                将立即阻止下一次模型请求，并取消仍在本地执行的请求。已保存内容、待答问题、待审提案与未投递意见不会丢失。
              </p>
            </div>
          </div>
          <Button
            type="button"
            variant="destructive"
            disabled={disabled || pending}
            onClick={() => void handleStop()}
          >
            {pending ? (
              <LoaderCircle className="animate-spin" aria-hidden="true" />
            ) : (
              <Square aria-hidden="true" />
            )}
            停止执行
          </Button>
        </div>
      </section>
    )
  }

  return (
    <section className="mt-5 rounded-xl border border-amber-500/30 bg-amber-500/8 p-4">
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div className="flex max-w-2xl items-start gap-3">
          <CircleAlert
            className="mt-0.5 size-5 shrink-0 text-amber-700 dark:text-amber-300"
            aria-hidden="true"
          />
          <div>
            <h3 className="text-sm font-semibold">{copy.title}</h3>
            <p className="mt-1 text-xs leading-5 text-muted-foreground">
              {copy.description}
            </p>
          </div>
        </div>
        <Button
          type="button"
          disabled={disabled || pending}
          onClick={() => void handleContinue()}
        >
          {pending ? (
            <LoaderCircle className="animate-spin" aria-hidden="true" />
          ) : (
            <Play aria-hidden="true" />
          )}
          显式继续
        </Button>
      </div>
    </section>
  )
}
