import { useEffect, useRef, useState } from "react"
import { ArrowLeft, LoaderCircle } from "lucide-react"

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
  getChapterBody,
  getChapterGuidance,
  getChapterOutline,
  type ChapterSummary,
} from "@/lib/api"

interface ChapterReaderProps {
  chapter: ChapterSummary
  /** 当前写作章节；用于提示用户这里不是写作区。 */
  activeChapterId: string | null
  onBackToActive: () => void
  onError: (message: string) => void
}

type LoadedChapter =
  | {
      ok: true
      chapterId: string
      guidance: string | null
      outline: string | null
      body: string | null
      bodyApproved: boolean
    }
  | { ok: false; chapterId: string }

/**
 * 只读地展示任意章节的指导、细纲与已提交正文。
 *
 * 这里刻意不提供任何写入操作：浏览旧章节不能修改当前写作章，也不能把命令发往
 * 非活跃章节。阅读状态以 `chapterId` 为键，因此切换章节时不需要在副作用里同步
 * 重置状态，也不会短暂显示上一章的内容。
 */
export function ChapterReader({
  chapter,
  activeChapterId,
  onBackToActive,
  onError,
}: ChapterReaderProps) {
  const [loaded, setLoaded] = useState<LoadedChapter | null>(null)
  const containerRef = useRef<HTMLDivElement | null>(null)

  useEffect(() => {
    let isCurrent = true
    Promise.all([
      getChapterGuidance(chapter.id),
      getChapterOutline(chapter.id),
      getChapterBody(chapter.id),
    ])
      .then(([guidance, outline, body]) => {
        if (!isCurrent) return
        setLoaded({
          ok: true,
          chapterId: chapter.id,
          guidance: guidance.content,
          outline: outline.outline.content,
          body: body.body.content,
          bodyApproved: body.body.review?.status === "approved",
        })
      })
      .catch((error: unknown) => {
        if (!isCurrent) return
        setLoaded({ ok: false, chapterId: chapter.id })
        onError(error instanceof Error ? error.message : "读取章节内容失败")
      })
    return () => {
      isCurrent = false
    }
  }, [chapter.id, onError])

  useEffect(() => {
    containerRef.current?.scrollIntoView({ block: "start" })
  }, [chapter.id])

  const reading = loaded?.chapterId === chapter.id ? loaded : null
  const isWritingChapter = chapter.id === activeChapterId
  const title =
    chapter.kind === "extra"
      ? `番外 ${chapter.orderIndex}《${chapter.title}》`
      : `第 ${chapter.orderIndex} 章《${chapter.title}》`

  return (
    <div ref={containerRef}>
      <Card>
        <CardHeader>
          <CardTitle>{title}</CardTitle>
          <CardDescription>
            {isWritingChapter
              ? "这正是当前写作章节；下方为只读视图。"
              : "只读回看：这里不会修改当前写作章节，也不会发出任何写作命令。"}
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex flex-wrap items-center gap-2">
            <Badge variant="outline">
              {chapter.kind === "extra" ? "番外" : "正文"}
            </Badge>
            {isWritingChapter ? (
              <Badge variant="default">当前写作章</Badge>
            ) : activeChapterId ? (
              <Button size="sm" variant="outline" onClick={onBackToActive}>
                <ArrowLeft aria-hidden="true" />
                回到写作章
              </Button>
            ) : null}
          </div>

          {reading === null ? (
            <p className="flex items-center gap-2 text-sm text-muted-foreground">
              <LoaderCircle className="size-4 animate-spin" aria-hidden="true" />
              正在读取章节内容…
            </p>
          ) : !reading.ok ? (
            <p className="text-sm text-muted-foreground">
              章节内容暂时无法读取；请查看上方的错误提示。
            </p>
          ) : (
            <>
              <section>
                <h3 className="text-xs font-semibold tracking-[0.14em] text-muted-foreground uppercase">
                  章节指导
                </h3>
                <p className="mt-2 text-sm leading-6 whitespace-pre-wrap">
                  {reading.guidance?.trim()
                    ? reading.guidance
                    : "本章没有保存过章节指导。"}
                </p>
              </section>

              <section>
                <h3 className="text-xs font-semibold tracking-[0.14em] text-muted-foreground uppercase">
                  章节细纲
                </h3>
                <pre className="mt-2 max-h-72 overflow-auto rounded-lg border border-border bg-muted/20 p-3 text-sm leading-6 whitespace-pre-wrap">
                  {reading.outline?.trim()
                    ? reading.outline
                    : "本章细纲尚未提交。"}
                </pre>
              </section>

              <section>
                <h3 className="text-xs font-semibold tracking-[0.14em] text-muted-foreground uppercase">
                  已提交正文
                  {reading.bodyApproved ? "（本章定稿）" : "（未定稿）"}
                </h3>
                <pre className="mt-2 max-h-[32rem] overflow-auto rounded-lg border border-border p-3 text-base leading-7 whitespace-pre-wrap">
                  {reading.body?.trim()
                    ? reading.body
                    : "本章尚无已接受的正文。"}
                </pre>
              </section>
            </>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
