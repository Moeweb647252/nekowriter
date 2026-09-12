import { useEffect, useState } from "react"
import { LoaderCircle } from "lucide-react"

import {
  getChapterGuidance,
  getChapterOutline,
  type ChapterSummary,
} from "@/lib/api"

interface ChapterDetailsProps {
  chapter: ChapterSummary
  onError: (message: string) => void
}

type LoadedDetails = {
  chapterId: string
  guidance: string | null
  outline: string | null
}

/**
 * 一章的指导与细纲，只在用户展开时加载。
 *
 * 阅读区默认只显示已提交正文；指导与细纲是写作过程材料，按需加载因此不会拖慢
 * 打开长篇的成本。加载状态以 `chapterId` 为键，所以展开另一章不会短暂显示上一章
 * 的内容。
 */
export function ChapterDetails({ chapter, onError }: ChapterDetailsProps) {
  const [loaded, setLoaded] = useState<LoadedDetails | null>(null)

  useEffect(() => {
    let isCurrent = true
    Promise.all([getChapterGuidance(chapter.id), getChapterOutline(chapter.id)])
      .then(([guidance, outline]) => {
        if (!isCurrent) return
        setLoaded({
          chapterId: chapter.id,
          guidance: guidance.content,
          outline: outline.outline.content,
        })
      })
      .catch((error: unknown) => {
        if (!isCurrent) return
        setLoaded({ chapterId: chapter.id, guidance: null, outline: null })
        onError(
          error instanceof Error ? error.message : "读取章节指导与细纲失败"
        )
      })
    return () => {
      isCurrent = false
    }
  }, [chapter.id, onError])

  const details = loaded?.chapterId === chapter.id ? loaded : null

  if (!details) {
    return (
      <p className="flex items-center gap-2 text-sm text-muted-foreground">
        <LoaderCircle className="size-4 animate-spin" aria-hidden="true" />
        正在读取章节指导与细纲…
      </p>
    )
  }

  return (
    <div className="space-y-3">
      <section>
        <h3 className="text-xs font-semibold tracking-[0.14em] text-muted-foreground uppercase">
          章节指导
        </h3>
        <p className="mt-2 text-sm leading-6 whitespace-pre-wrap">
          {details.guidance?.trim()
            ? details.guidance
            : "本章没有保存过章节指导。"}
        </p>
      </section>
      <section>
        <h3 className="text-xs font-semibold tracking-[0.14em] text-muted-foreground uppercase">
          章节细纲
        </h3>
        <p className="mt-2 text-sm leading-6 whitespace-pre-wrap">
          {details.outline?.trim() ? details.outline : "本章细纲尚未提交。"}
        </p>
      </section>
    </div>
  )
}
