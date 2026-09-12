import { useEffect, useRef } from "react"
import { useVirtualizer } from "@tanstack/react-virtual"

import { Badge } from "@/components/ui/badge"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import type { ChapterSummary } from "@/lib/api"

interface ChapterDirectoryProps {
  chapters: ChapterSummary[]
  /** 当前正在写作的章节；它与被阅读的章节是两件事。 */
  activeChapterId: string | null
  /** 阅读区视口顶部那一章；为空表示还没进入阅读区。 */
  readingChapterId: string | null
  onSelect: (chapterId: string) => void
}

const statusLabels: Record<ChapterSummary["status"], string> = {
  planned: "未开始",
  in_progress: "撰写中",
  approved: "已通过",
}

/** 目录条目的初始高度估计；真实高度由测量得到。 */
const ESTIMATED_ITEM_HEIGHT = 58

/**
 * 章节目录。
 *
 * 目录是虚拟化列表，以章节 id 作 key：几百章的目录只渲染视口里的那几行，点击
 * 任意一行只是把目标 id 交给阅读区，由阅读区按虚拟下标定位再对齐——目录不依赖
 * 任何章节内容已经挂载。
 */
export function ChapterDirectory({
  chapters,
  activeChapterId,
  readingChapterId,
  onSelect,
}: ChapterDirectoryProps) {
  const scrollRef = useRef<HTMLDivElement | null>(null)
  const virtualizer = useVirtualizer({
    count: chapters.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ESTIMATED_ITEM_HEIGHT,
    overscan: 6,
    getItemKey: (index) => chapters[index]?.id ?? index,
  })

  // 阅读位置变化时让目录跟上，否则读着第 300 章时目录还停在开头。
  useEffect(() => {
    const index = chapters.findIndex(
      (chapter) => chapter.id === readingChapterId
    )
    if (index >= 0) virtualizer.scrollToIndex(index, { align: "auto" })
  }, [chapters, readingChapterId, virtualizer])

  return (
    <Card>
      <CardHeader>
        <CardTitle>章节目录</CardTitle>
        <CardDescription>
          点击任意章节在阅读区打开它；查看不会改动写作进度。
        </CardDescription>
      </CardHeader>
      <CardContent>
        {chapters.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            粗纲审核通过后会在这里按顺序列出章节。
          </p>
        ) : (
          <div
            ref={scrollRef}
            data-chapter-directory="true"
            className="max-h-[26rem] overflow-y-auto overscroll-contain pr-1"
          >
            <ol
              className="relative w-full"
              style={{ height: virtualizer.getTotalSize() }}
            >
              {virtualizer.getVirtualItems().map((item) => {
                const chapter = chapters[item.index]
                if (!chapter) return null
                const isWriting = chapter.id === activeChapterId
                const isReading = chapter.id === readingChapterId
                return (
                  <li
                    key={item.key}
                    data-index={item.index}
                    ref={virtualizer.measureElement}
                    className="absolute inset-x-0 top-0"
                    style={{ transform: `translateY(${item.start}px)` }}
                  >
                    <button
                      type="button"
                      aria-current={isReading ? "true" : undefined}
                      onClick={() => onSelect(chapter.id)}
                      className={`flex w-full items-center justify-between gap-2 rounded-xl border px-2.5 py-2 text-left text-sm transition focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring ${
                        isReading
                          ? "border-primary/50 bg-primary/10"
                          : "border-transparent hover:border-border hover:bg-muted/40"
                      }`}
                    >
                      <span className="min-w-0">
                        <span className="block truncate font-medium">
                          {chapter.label}
                        </span>
                        <span className="block text-xs text-muted-foreground">
                          {statusLabels[chapter.status]}
                        </span>
                      </span>
                      <span className="flex shrink-0 items-center gap-1">
                        {isWriting ? (
                          <Badge variant="default">写作中</Badge>
                        ) : null}
                        {isReading ? (
                          <Badge variant="secondary">阅读中</Badge>
                        ) : null}
                      </span>
                    </button>
                  </li>
                )
              })}
            </ol>
          </div>
        )}
      </CardContent>
    </Card>
  )
}
