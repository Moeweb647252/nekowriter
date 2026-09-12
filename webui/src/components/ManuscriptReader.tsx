import { useCallback, useEffect, useRef, useState } from "react"
import { useVirtualizer } from "@tanstack/react-virtual"
import { ArrowLeft, LoaderCircle, Minus, Plus, ScrollText } from "lucide-react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { ChapterDetails } from "@/components/ChapterDetails"
import { ManuscriptSearch } from "@/components/ManuscriptSearch"
import type {
  ChapterSummary,
  ManuscriptChapter,
  ManuscriptMatch,
} from "@/lib/api"
import {
  READER_FONT_SIZE_DEFAULT,
  READER_FONT_SIZE_MAX,
  READER_FONT_SIZE_MIN,
  readReadingAnchor,
  useManuscriptPages,
  writeReadingAnchor,
  type ReadingAnchor,
} from "@/lib/manuscript"

interface ManuscriptReaderProps {
  versionId: string
  /** 目录索引，按阅读顺序排列；正文内容另外按窗口加载。 */
  chapters: ChapterSummary[]
  activeChapterId: string | null
  /**
   * 目录或查找请求定位到的章节。`nonce` 变化表示一次新的定位请求，因此重复点
   * 同一章也能重新对齐。
   */
  target: { chapterId: string; nonce: number } | null
  /** 视口顶部那一章，用于让目录标出「正在读」。 */
  onVisibleChapterChange: (chapterId: string | null) => void
  /** 服务端报告的章节数与本地目录不一致：目录需要重新读取。 */
  onIndexStale: () => void
  onBackToWriting: () => void
  onError: (message: string) => void
}

interface Highlight {
  chapterId: string
  startChar: number
  endChar: number
}

/** 一次待完成的定位：把某章（可选带块内偏移）对齐到视口顶部。 */
interface AlignmentTarget {
  index: number
  offset: number
  lastStart: number | null
  deadline: number
}

/** 章节块上下留白与标题区的粗略高度，只用于估计未渲染章节的高度。 */
const CHAPTER_CHROME_HEIGHT = 130
const LINE_HEIGHT_RATIO = 1.9
/** 定位久候无进展时放弃，避免在未渲染的章节上无限重试。 */
const ALIGN_TIMEOUT_MS = 3000

/**
 * 未加载章节的高度估计。
 *
 * 每章长度相差很大，固定估计值会让远距离定位与滚动条都失真；目录已经带回每章
 * 字数，因此估计值来自数据而不是猜测，剩下的误差由挂载后的测量纠正。
 */
function estimateChapterHeight(
  wordCount: number,
  fontSize: number,
  containerWidth: number
) {
  const contentWidth = Math.max(240, containerWidth - 56)
  const charactersPerLine = Math.max(8, Math.floor(contentWidth / fontSize))
  const lines = Math.ceil(Math.max(wordCount, 1) / charactersPerLine)
  return Math.round(
    lines * fontSize * LINE_HEIGHT_RATIO + CHAPTER_CHROME_HEIGHT
  )
}

/**
 * 长篇阅读区：整部作品在一个连续滚动区里虚拟化呈现。
 *
 * 只有视口附近的章节被挂载，正文按窗口向后端索取，因此渲染开销与总章数无关；
 * 阅读位置以「章 + 块内偏移」记在浏览器本地存储里，切换面板、切换视图和重新加载
 * 页面后都能回到原处。定位从不依赖未挂载 DOM 的 `scrollIntoView()`：先按虚拟
 * 下标滚动，等正文加载、高度被测得后再把标题对齐到视口顶部。
 */
export function ManuscriptReader({
  versionId,
  chapters,
  activeChapterId,
  target,
  onVisibleChapterChange,
  onIndexStale,
  onBackToWriting,
  onError,
}: ManuscriptReaderProps) {
  const scrollRef = useRef<HTMLDivElement | null>(null)
  const alignment = useRef<AlignmentTarget | null>(null)
  const restored = useRef(false)
  const appliedTargetNonce = useRef<number | null>(null)
  const [fontSize, setFontSize] = useState(READER_FONT_SIZE_DEFAULT)
  // 高度估计要跟着字号走，而 `estimateSize` 在渲染时读取；用 ref 保持最新值。
  const fontSizeRef = useRef(fontSize)
  const [highlight, setHighlight] = useState<Highlight | null>(null)
  const [expandedChapterId, setExpandedChapterId] = useState<string | null>(
    null
  )

  const pages = useManuscriptPages(
    versionId,
    chapters.length,
    onError,
    onIndexStale
  )

  const virtualizer = useVirtualizer({
    count: chapters.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) =>
      estimateChapterHeight(
        Number(chapters[index]?.bodyWordCount ?? 0),
        fontSizeRef.current,
        scrollRef.current?.clientWidth ?? 760
      ),
    overscan: 2,
    getItemKey: (index) => chapters[index]?.id ?? index,
  })

  const pinTo = useCallback(
    (chapterId: string, offset: number) => {
      const index = chapters.findIndex((chapter) => chapter.id === chapterId)
      if (index < 0) return
      alignment.current = {
        index,
        offset,
        lastStart: null,
        deadline: performance.now() + ALIGN_TIMEOUT_MS,
      }
      virtualizer.scrollToIndex(index, { align: "start" })
    },
    [chapters, virtualizer]
  )

  /** 视口顶部那一章；它的块顶与视口顶的距离就是块内偏移。 */
  const captureAnchor = useCallback((): ReadingAnchor | null => {
    const scroller = scrollRef.current
    if (!scroller) return null
    const items = virtualizer.getVirtualItems()
    const item =
      items.find((candidate) => candidate.end > scroller.scrollTop + 1) ??
      items[0]
    const chapter = item ? chapters[item.index] : undefined
    if (!item || !chapter) return null
    return {
      chapterId: chapter.id,
      offset: Math.max(0, Math.round(scroller.scrollTop - item.start)),
    }
  }, [chapters, virtualizer])

  // 记住位置：滚动时按帧合并写入，离开阅读区时再写一次。
  useEffect(() => {
    const scroller = scrollRef.current
    if (!scroller) return
    let frame = 0
    const flush = () => {
      frame = 0
      const anchor = captureAnchor()
      if (!anchor) return
      writeReadingAnchor(versionId, anchor)
      onVisibleChapterChange(anchor.chapterId)
    }
    const onScroll = () => {
      if (frame === 0) frame = requestAnimationFrame(flush)
    }
    scroller.addEventListener("scroll", onScroll, { passive: true })
    return () => {
      scroller.removeEventListener("scroll", onScroll)
      if (frame !== 0) cancelAnimationFrame(frame)
      flush()
    }
  }, [captureAnchor, onVisibleChapterChange, versionId])

  // 打开阅读区：回到上次离开的位置，没有记录时停在第一章。一次挂载只恢复一次，
  // 而且用户刚要求打开某一章时以那个目标为准——两个定位不能先后发出。
  useEffect(() => {
    if (restored.current || chapters.length === 0) return
    restored.current = true
    if (target) {
      appliedTargetNonce.current = target.nonce
      pinTo(target.chapterId, 0)
      onVisibleChapterChange(target.chapterId)
      return
    }
    const anchor = readReadingAnchor(versionId)
    if (anchor && chapters.some((chapter) => chapter.id === anchor.chapterId)) {
      pinTo(anchor.chapterId, anchor.offset)
      onVisibleChapterChange(anchor.chapterId)
      return
    }
    onVisibleChapterChange(chapters[0].id)
  }, [chapters, onVisibleChapterChange, pinTo, target, versionId])

  // 已经打开着阅读区时的定位请求。
  useEffect(() => {
    if (!target || chapters.length === 0) return
    if (appliedTargetNonce.current === target.nonce) return
    appliedTargetNonce.current = target.nonce
    pinTo(target.chapterId, 0)
  }, [chapters.length, pinTo, target])

  // 正文按窗口加载：只请求视口覆盖到的页。
  useEffect(() => {
    const items = virtualizer.getVirtualItems()
    if (items.length === 0) return
    pages.request(items[0].index, items[items.length - 1].index)
  })

  //
  // 定位：`pinTo` 先让虚拟器按虚拟下标滚动（不依赖未挂载 DOM 的 scrollIntoView），
  // 这里在渲染后检查真实滚动位置是否已经把目标块放在视口顶部。未挂载章节的高度
  // 是估计值，正文加载与测量会让目标移动，因此每帧重新对齐；只要还在前进就延长
  // 期限，一直到 DOM 上确实对齐或超时（例如目标在末尾、滚动位置被夹住）。
  useEffect(() => {
    const target = alignment.current
    const scroller = scrollRef.current
    if (!target || !scroller) return
    const item = virtualizer
      .getVirtualItems()
      .find((candidate) => candidate.index === target.index)
    if (item) {
      const desired = Math.max(0, item.start + target.offset)
      const maxScroll = Math.max(
        0,
        scroller.scrollHeight - scroller.clientHeight
      )
      const aligned = Math.abs(scroller.scrollTop - desired) < 1
      const settled = target.lastStart === item.start
      // 已经对齐，或位置已经稳定到无法再靠近（末尾几章会被滚动范围夹住）。
      if (aligned || (settled && desired > maxScroll)) {
        alignment.current = null
        return
      }
      if (!settled) {
        target.lastStart = item.start
        target.deadline = performance.now() + ALIGN_TIMEOUT_MS
      }
      virtualizer.scrollToOffset(desired)
    }
    if (performance.now() > target.deadline) alignment.current = null
  })

  /**
   * 字号变化会改变每章高度：把当前章重新对齐到顶部（保证不跳到别的章），并丢弃
   * 旧的高度缓存，让已挂载章节重新测量、未挂载章节按新字号重新估计。
   */
  function changeFontSize(delta: number) {
    const next = Math.min(
      READER_FONT_SIZE_MAX,
      Math.max(READER_FONT_SIZE_MIN, fontSize + delta)
    )
    if (next === fontSize) return
    const anchor = captureAnchor()
    fontSizeRef.current = next
    setFontSize(next)
    virtualizer.measure()
    if (anchor) pinTo(anchor.chapterId, 0)
  }

  /** 跳到一处查找命中并高亮它。 */
  function locate(match: ManuscriptMatch) {
    setHighlight({
      chapterId: match.chapterId,
      startChar: Number(match.startChar),
      endChar: Number(match.endChar),
    })
    pinTo(match.chapterId, 0)
  }

  const totalSize = virtualizer.getTotalSize()
  const items = virtualizer.getVirtualItems()

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <ScrollText className="size-4" aria-hidden="true" />
          长篇阅读
        </CardTitle>
        <CardDescription>
          整部作品在一个连续滚动区里按章虚拟化：只有视口附近的章节被渲染，正文按窗口加载。
          阅读位置记在本机浏览器里，切换面板或重新加载后回到原处。
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="flex flex-wrap items-center gap-2">
          <Button size="sm" variant="outline" onClick={onBackToWriting}>
            <ArrowLeft aria-hidden="true" />
            返回写作
          </Button>
          <span className="flex items-center gap-1">
            <Button
              size="sm"
              variant="outline"
              aria-label="缩小正文字号"
              disabled={fontSize <= READER_FONT_SIZE_MIN}
              onClick={() => changeFontSize(-1)}
            >
              <Minus aria-hidden="true" />
            </Button>
            <span className="w-14 text-center text-xs text-muted-foreground tabular-nums">
              {fontSize} px
            </span>
            <Button
              size="sm"
              variant="outline"
              aria-label="放大正文字号"
              disabled={fontSize >= READER_FONT_SIZE_MAX}
              onClick={() => changeFontSize(1)}
            >
              <Plus aria-hidden="true" />
            </Button>
          </span>
          <span className="text-xs text-muted-foreground">
            共 {chapters.length} 章
          </span>
        </div>

        <ManuscriptSearch
          versionId={versionId}
          chapters={chapters}
          onLocate={locate}
          onError={onError}
        />

        {chapters.length === 0 ? (
          <p className="rounded-xl border border-dashed border-border p-6 text-center text-sm text-muted-foreground">
            这部稿本还没有章节；粗纲审核通过后章节会出现在这里。
          </p>
        ) : (
          <div
            ref={scrollRef}
            data-manuscript-scroll="true"
            className="h-[70svh] overflow-y-auto overscroll-contain rounded-xl border border-border bg-background/70"
          >
            <div className="relative w-full" style={{ height: totalSize }}>
              {items.map((item) => {
                const chapter = chapters[item.index]
                if (!chapter) return null
                const content = pages.contentAt(item.index, chapter)
                return (
                  <div
                    key={item.key}
                    data-index={item.index}
                    // 只测量真正呈现出来的章节：加载占位的高度不是这一章的高度，
                    // 把它当成尺寸会让未加载区域的几何逐帧塌缩，定位随之错位。
                    ref={content ? virtualizer.measureElement : undefined}
                    className="absolute inset-x-0 top-0 px-4 sm:px-7"
                    style={{ transform: `translateY(${item.start}px)` }}
                  >
                    <ChapterBlock
                      chapter={chapter}
                      content={content}
                      fontSize={fontSize}
                      isWriting={chapter.id === activeChapterId}
                      isExpanded={chapter.id === expandedChapterId}
                      highlight={
                        highlight?.chapterId === chapter.id ? highlight : null
                      }
                      onToggleDetails={() =>
                        setExpandedChapterId((current) =>
                          current === chapter.id ? null : chapter.id
                        )
                      }
                      onError={onError}
                    />
                  </div>
                )
              })}
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  )
}

interface ChapterBlockProps {
  chapter: ChapterSummary
  content: ManuscriptChapter | null
  fontSize: number
  isWriting: boolean
  isExpanded: boolean
  highlight: Highlight | null
  onToggleDetails: () => void
  onError: (message: string) => void
}

function ChapterBlock({
  chapter,
  content,
  fontSize,
  isWriting,
  isExpanded,
  highlight,
  onToggleDetails,
  onError,
}: ChapterBlockProps) {
  const hasContent = Boolean(content?.content.trim())
  return (
    <article
      data-chapter-block={chapter.id}
      className="border-b border-border/70 py-7 last:border-b-0"
    >
      <header className="mb-3 flex flex-wrap items-center gap-2">
        <h3 className="text-lg font-semibold tracking-tight">
          {chapter.label}
        </h3>
        {isWriting ? <Badge variant="default">写作中</Badge> : null}
        {content ? (
          <Badge variant="outline">{content.draft ? "草稿" : "定稿"}</Badge>
        ) : null}
      </header>

      {content === null ? (
        <p className="flex items-center gap-2 py-4 text-sm text-muted-foreground">
          <LoaderCircle className="size-4 animate-spin" aria-hidden="true" />
          正在读取本章…
        </p>
      ) : hasContent ? (
        <ChapterText
          content={content.content}
          fontSize={fontSize}
          highlight={highlight}
        />
      ) : (
        <p className="text-sm text-muted-foreground">（本章尚无正文）</p>
      )}

      <div className="mt-4">
        <Button
          size="sm"
          variant="ghost"
          aria-expanded={isExpanded}
          onClick={onToggleDetails}
        >
          章节指导与细纲
        </Button>
      </div>
      {isExpanded && (
        <div className="mt-2 rounded-xl border border-border bg-muted/20 p-3">
          <ChapterDetails chapter={chapter} onError={onError} />
        </div>
      )}
    </article>
  )
}

function ChapterText({
  content,
  fontSize,
  highlight,
}: {
  content: string
  fontSize: number
  highlight: Highlight | null
}) {
  const style = { fontSize, lineHeight: 1.9 }
  if (!highlight) {
    return (
      <p className="font-serif whitespace-pre-wrap" style={style}>
        {content}
      </p>
    )
  }
  // 偏移以 Unicode scalar 计数；`Array.from` 按 scalar 拆分，因此高亮范围与
  // 查找结果、引用校验使用同一套位置。
  const characters = Array.from(content)
  return (
    <p className="font-serif whitespace-pre-wrap" style={style}>
      {characters.slice(0, highlight.startChar).join("")}
      <mark
        data-search-match="true"
        className="rounded-sm bg-amber-300/60 px-0.5 text-inherit dark:bg-amber-500/30"
      >
        {characters.slice(highlight.startChar, highlight.endChar).join("")}
      </mark>
      {characters.slice(highlight.endChar).join("")}
    </p>
  )
}
