import { Power } from "lucide-react"

import { useEventSync } from "@/lib/event-sync-context"

/**
 * 「本地服务已退出」提示条。
 *
 * 服务可能在别处被退出（终端 Ctrl+C、SIGTERM，或界面上的退出动作），因此事件同步断开时
 * 先探一次健康检查，只有确认不可达才进入这个状态：界面停止重连，并说明如何重新启动。
 * 工作台与管理页共用这一份文案。
 */
export function ServiceStoppedNotice() {
  const { serviceStopped } = useEventSync()
  if (!serviceStopped) return null

  return (
    <div
      className="flex items-start gap-3 border-b border-border bg-muted/60 px-4 py-3 text-sm sm:px-6"
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
  )
}
