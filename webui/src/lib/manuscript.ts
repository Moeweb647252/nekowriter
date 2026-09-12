import { useCallback, useRef, useState } from "react"

import {
  getManuscriptWindow,
  type ChapterSummary,
  type ManuscriptChapter,
  type ManuscriptWindow,
} from "@/lib/api"

/**
 * 一次阅读窗口请求的章节数。正文按窗口取，因此打开一部几百章的作品需要读的章节
 * 数只由视口决定，与总章数无关；后端对窗口长度另有上限（见 docs/02 §10.6）。
 */
export const MANUSCRIPT_PAGE_SIZE = 20

export const READER_FONT_SIZE_DEFAULT = 18
export const READER_FONT_SIZE_MIN = 15
export const READER_FONT_SIZE_MAX = 26

/**
 * 阅读位置：视口顶部那一章的 id，以及块内像素偏移。
 *
 * 保存的是「章 + 块内偏移」而不是绝对滚动像素：字号、窗口大小和正文长度变化后，
 * 绝对像素会落到别的地方，章节身份不会。
 */
export interface ReadingAnchor {
  chapterId: string
  offset: number
}

function storageKey(versionId: string) {
  return `nekowriter.reading-anchor.${versionId}`
}

/** 读取上次离开时的阅读位置；没有记录或本地存储不可用时返回 `null`。 */
export function readReadingAnchor(versionId: string): ReadingAnchor | null {
  try {
    const raw = window.localStorage.getItem(storageKey(versionId))
    if (!raw) return null
    const parsed: unknown = JSON.parse(raw)
    if (typeof parsed !== "object" || parsed === null) return null
    const { chapterId, offset } = parsed as Partial<ReadingAnchor>
    if (typeof chapterId !== "string" || typeof offset !== "number") return null
    return { chapterId, offset }
  } catch {
    return null
  }
}

/** 记住当前阅读位置。本地存储不可用时只是记不住位置，不影响阅读。 */
export function writeReadingAnchor(versionId: string, anchor: ReadingAnchor) {
  try {
    window.localStorage.setItem(storageKey(versionId), JSON.stringify(anchor))
  } catch {
    // 浏览器拒绝写入（隐私模式、配额）时静默跳过：这是一条可选的便利记录。
  }
}

export interface ManuscriptPages {
  /** 该下标所在页已加载的那一章；尚未加载时返回 `null`。 */
  contentAt: (
    index: number,
    chapter: ChapterSummary
  ) => ManuscriptChapter | null
  /** 请求覆盖 `[startIndex, endIndex]` 的页；已加载或在途的页不会重复请求。 */
  request: (startIndex: number, endIndex: number) => void
}

/**
 * 按页加载正文。
 *
 * 页是「下标区间」而不是「滚动位置」，所以快速滚动只会多请求几个窗口，不会重复
 * 请求同一批章节。服务端报告的章节总数与本地目录不一致时通知调用方刷新目录。
 */
export function useManuscriptPages(
  versionId: string,
  chapterCount: number,
  onError: (message: string) => void,
  onIndexStale: () => void
): ManuscriptPages {
  const [pages, setPages] = useState<ReadonlyMap<number, ManuscriptWindow>>(
    () => new Map()
  )
  const inFlight = useRef(new Set<number>())

  const request = useCallback(
    (startIndex: number, endIndex: number) => {
      const firstPage = Math.max(
        0,
        Math.floor(startIndex / MANUSCRIPT_PAGE_SIZE)
      )
      const lastPage = Math.max(
        firstPage,
        Math.floor(endIndex / MANUSCRIPT_PAGE_SIZE)
      )
      for (let page = firstPage; page <= lastPage; page += 1) {
        if (pages.has(page) || inFlight.current.has(page)) continue
        inFlight.current.add(page)
        getManuscriptWindow(
          versionId,
          page * MANUSCRIPT_PAGE_SIZE,
          MANUSCRIPT_PAGE_SIZE
        )
          .then((window) => {
            inFlight.current.delete(page)
            if (window.totalChapters !== chapterCount) onIndexStale()
            setPages((current) => new Map(current).set(page, window))
          })
          .catch((error: unknown) => {
            inFlight.current.delete(page)
            onError(error instanceof Error ? error.message : "无法读取正文")
          })
      }
    },
    [chapterCount, onError, onIndexStale, pages, versionId]
  )

  const contentAt = useCallback(
    (index: number, chapter: ChapterSummary) => {
      const page = pages.get(Math.floor(index / MANUSCRIPT_PAGE_SIZE))
      if (!page) return null
      return page.chapters.find((item) => item.chapterId === chapter.id) ?? null
    },
    [pages]
  )

  return { contentAt, request }
}
