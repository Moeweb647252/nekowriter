import { Braces, LoaderCircle, TextCursorInput } from "lucide-react"

import { type PreviewTool } from "@/lib/api"

export type GenerationPreviewBuffer = {
  key: string
  requestId: string
  tool?: PreviewTool
  content: string
}

function labelFor(buffer: GenerationPreviewBuffer) {
  if (!buffer.tool) return "模型文本"
  return (
    buffer.tool.name ?? buffer.tool.id ?? `工具调用 ${buffer.tool.index + 1}`
  )
}

/**
 * 独立展示尚未形成任何业务修订的字符流。
 *
 * 这里的内容来自不带 SSE id 的 `preview.delta`：断线、切换稿本或持久状态落库后都
 * 可以直接丢弃，绝不把半截文本当成正文或阶段产物。
 */
export function GenerationPreview({
  buffers,
}: {
  buffers: GenerationPreviewBuffer[]
}) {
  if (buffers.length === 0) return null

  return (
    <section
      aria-live="polite"
      className="mt-6 rounded-2xl border border-dashed border-primary/35 bg-primary/5 p-5 shadow-sm"
    >
      <div className="flex items-start gap-3">
        <LoaderCircle
          className="mt-0.5 size-5 shrink-0 animate-spin text-primary"
          aria-hidden="true"
        />
        <div>
          <p className="text-xs font-semibold tracking-[0.14em] text-muted-foreground uppercase">
            临时生成预览
          </p>
          <h2 className="mt-1 text-lg font-semibold">模型正在输出</h2>
          <p className="mt-1 text-sm leading-6 text-muted-foreground">
            这只是尚未验证和提交的流式内容；断线或状态变化后会自动丢弃。
          </p>
        </div>
      </div>
      <div className="mt-4 space-y-3">
        {buffers.map((buffer) => {
          const isTool = Boolean(buffer.tool)
          const Icon = isTool ? Braces : TextCursorInput
          return (
            <div
              key={buffer.key}
              className="rounded-xl border border-border/70 bg-background/80 p-3"
            >
              <div className="flex items-center gap-2 text-sm font-medium">
                <Icon className="size-4 text-primary" aria-hidden="true" />
                <span>{labelFor(buffer)}</span>
                <span className="ml-auto font-mono text-xs text-muted-foreground">
                  {buffer.requestId.slice(0, 8)}
                </span>
              </div>
              <pre className="mt-2 max-h-52 overflow-auto text-xs leading-5 break-words whitespace-pre-wrap text-muted-foreground">
                {buffer.content || "正在接收工具调用信息…"}
              </pre>
            </div>
          )
        })}
      </div>
    </section>
  )
}
