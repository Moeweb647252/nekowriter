import { useEffect, useRef, useState } from "react"
import {
  CircleAlert,
  CircleCheck,
  LoaderCircle,
  Power,
  Server,
} from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { getHealth, shutdownService, type HealthResponse } from "@/lib/api"
import { useEscapeDismiss } from "@/lib/shortcuts"

interface ServicePanelProps {
  health: HealthResponse | null
  /** 服务已经停止接受连接；工作台其余部分只展示最后读到的数据。 */
  stopped: boolean
  onStopped: () => void
}

/**
 * 本地服务的可见状态与退出入口。
 *
 * 退出服务只由这里发起：浏览器标签页关闭不会终止后端，因此需要一个明确的命令，
 * 以及退出之后如何重新启动的说明。响应是否真的到达不作假设——以后端还能不能应答
 * 健康检查为准。
 */
export function ServicePanel({
  health,
  stopped,
  onStopped,
}: ServicePanelProps) {
  const [confirming, setConfirming] = useState(false)
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const cancelButton = useRef<HTMLButtonElement>(null)

  // 退出确认是可以安全取消的一步：Esc 只收起它；焦点先落到「取消」而不是破坏性按钮上。
  useEscapeDismiss(confirming && !pending, () => setConfirming(false))
  useEffect(() => {
    if (confirming) cancelButton.current?.focus()
  }, [confirming])

  async function handleShutdown() {
    setPending(true)
    setError(null)
    try {
      await shutdownService()
    } catch {
      // 服务可能在写回响应前就已经退出，因此失败不直接当成错误。
    }
    const alive = await getHealth().then(
      () => true,
      () => false
    )
    setPending(false)
    setConfirming(false)
    if (alive) {
      setError(
        "退出请求已送达，但服务仍在响应。请重试，或在终端用 Ctrl+C 结束进程。"
      )
      return
    }
    onStopped()
  }

  return (
    <Card size="sm">
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Server className="size-4 text-muted-foreground" aria-hidden="true" />
          本地服务
        </CardTitle>
        <CardDescription>
          {stopped ? (
            <span className="flex items-center gap-1.5 text-destructive">
              <CircleAlert className="size-3.5" aria-hidden="true" />
              服务已退出，界面不再接收新事件
            </span>
          ) : (
            <span className="flex items-center gap-1.5">
              <CircleCheck
                className="size-3.5 text-emerald-600"
                aria-hidden="true"
              />
              正在本机回环地址上运行
            </span>
          )}
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <dl className="space-y-1.5 text-xs">
          <div className="flex gap-2">
            <dt className="shrink-0 text-muted-foreground">界面地址</dt>
            <dd className="truncate font-mono">{window.location.origin}</dd>
          </div>
          <div className="flex gap-2">
            <dt className="shrink-0 text-muted-foreground">数据与日志</dt>
            <dd
              className="truncate font-mono"
              title={health?.dataDir ?? undefined}
            >
              {health?.dataDir ?? "正在读取…"}
            </dd>
          </div>
        </dl>
        <p className="text-xs leading-5 text-muted-foreground">
          {stopped
            ? "重新运行 NekoWriter（发行目录中的 nekowriter，或源码目录中的 cargo run -p nekowriter-app）后刷新此页面即可继续。已经提交的正文、审核和用量都在本地数据库中。"
            : "关闭浏览器标签页不会终止后端；正在运行的请求会继续保持。数据、向量索引与日志都写在上面这个目录里。"}
        </p>
        {stopped ? null : confirming ? (
          <div className="space-y-2 rounded-lg border border-destructive/30 bg-destructive/5 p-3">
            <p className="text-xs leading-5">
              退出会先停止接受新请求，取消在途的模型调用，再把正在执行的 Run
              收束为「已中断」。待审提案、已提交正文与未投递意见都会保留。
            </p>
            <div className="flex gap-2">
              <Button
                size="sm"
                variant="destructive"
                disabled={pending}
                onClick={() => void handleShutdown()}
              >
                {pending ? (
                  <LoaderCircle className="animate-spin" aria-hidden="true" />
                ) : (
                  <Power aria-hidden="true" />
                )}
                {pending ? "正在退出…" : "确认退出"}
              </Button>
              <Button
                ref={cancelButton}
                size="sm"
                variant="ghost"
                disabled={pending}
                onClick={() => setConfirming(false)}
              >
                取消
              </Button>
            </div>
          </div>
        ) : (
          <Button
            size="sm"
            variant="outline"
            onClick={() => setConfirming(true)}
          >
            <Power aria-hidden="true" />
            退出服务
          </Button>
        )}
        {error ? (
          <p className="text-xs text-destructive" role="alert">
            {error}
          </p>
        ) : null}
      </CardContent>
    </Card>
  )
}
