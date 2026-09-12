import { useState } from "react"
import { LoaderCircle, X } from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  getDocumentRevision,
  type DocumentFeedbackCitation,
  type DocumentRevisionView,
  type FeedbackCitation,
} from "@/lib/api"
import { scalarOffsetToUtf16 } from "@/lib/citations"

interface CitationListProps {
  citations: FeedbackCitation[]
  /** 当前正文修订；与引用修订不同时提示“来自旧修订”。 */
  currentRevisionId: string | null
  disabled?: boolean
  onRemove?: (index: number) => void
  onError: (message: string) => void
}

/**
 * 展示一条意见所附的可验证引用。
 *
 * 正式文档引用永久锚定在提交时的修订上，当前正文改变后仍必须能读回原文；待审候选
 * 还没有修订，因此只显示提案快照摘要，不伪造 revision ID。
 */
export function CitationList({
  citations,
  currentRevisionId,
  disabled = false,
  onRemove,
  onError,
}: CitationListProps) {
  const [viewing, setViewing] = useState<{
    citation: DocumentFeedbackCitation
    revision: DocumentRevisionView
  } | null>(null)
  const [loadingId, setLoadingId] = useState<string | null>(null)

  if (citations.length === 0) return null

  async function openRevision(citation: DocumentFeedbackCitation) {
    setLoadingId(citation.revisionId)
    try {
      setViewing({
        citation,
        revision: await getDocumentRevision(
          citation.documentId,
          citation.revisionId
        ),
      })
    } catch (error) {
      onError(error instanceof Error ? error.message : "无法读取被引用的修订")
    } finally {
      setLoadingId(null)
    }
  }

  return (
    <div className="mt-2 space-y-2">
      <ul className="space-y-1.5">
        {citations.map((citation, index) => {
          const stale =
            citation.kind === "document" &&
            citation.revisionId !== currentRevisionId
          return (
            <li
              key={`${citation.kind}-${index}`}
              className="flex items-start gap-2 rounded-lg border border-violet-500/25 bg-background/70 px-2.5 py-2 text-xs leading-5"
            >
              <span className="mt-0.5 shrink-0 rounded-sm bg-violet-500/12 px-1.5 py-0.5 font-medium text-violet-900 dark:text-violet-100">
                {citation.kind === "document" ? "正文引用" : "提案快照"}
              </span>
              <span
                className="min-w-0 flex-1 break-words text-foreground"
                title={
                  citation.kind === "document"
                    ? citation.quote
                    : citation.candidateRange.quote
                }
              >
                「
                {citation.kind === "document"
                  ? citation.quote
                  : citation.candidateRange.quote}
                」
              </span>
              {stale && (
                <span className="shrink-0 rounded-sm bg-amber-500/15 px-1.5 py-0.5 font-medium text-amber-900 dark:text-amber-100">
                  来自旧修订
                </span>
              )}
              {citation.kind === "document" && (
                <Button
                  type="button"
                  size="xs"
                  variant="ghost"
                  disabled={disabled || loadingId === citation.revisionId}
                  onMouseDown={(event) => event.preventDefault()}
                  onClick={() => void openRevision(citation)}
                >
                  {loadingId === citation.revisionId ? (
                    <LoaderCircle className="animate-spin" aria-hidden="true" />
                  ) : null}
                  查看原修订
                </Button>
              )}
              {onRemove && (
                <button
                  type="button"
                  aria-label="移除引用"
                  disabled={disabled}
                  className="shrink-0 rounded-sm p-0.5 text-muted-foreground hover:text-foreground disabled:opacity-50"
                  onMouseDown={(event) => event.preventDefault()}
                  onClick={() => onRemove(index)}
                >
                  <X className="size-3.5" aria-hidden="true" />
                </button>
              )}
            </li>
          )
        })}
      </ul>

      {viewing && (
        <div className="rounded-lg border border-border bg-muted/20 p-3">
          <div className="flex items-start justify-between gap-3">
            <p className="text-xs leading-5 text-muted-foreground">
              文档修订 {viewing.revision.revisionNo}；
              {viewing.revision.isCurrent
                ? "仍是当前修订。"
                : "已不是当前修订，下面的内容按提交时的原文只读展示。"}
            </p>
            <Button
              type="button"
              size="xs"
              variant="ghost"
              onClick={() => setViewing(null)}
            >
              关闭
            </Button>
          </div>
          <pre className="mt-2 max-h-64 overflow-auto rounded-md border border-border bg-background/80 p-2 text-xs leading-6 whitespace-pre-wrap">
            <RevisionContent
              citation={viewing.citation}
              content={viewing.revision.content}
            />
          </pre>
        </div>
      )}
    </div>
  )
}

function RevisionContent({
  citation,
  content,
}: {
  citation: DocumentFeedbackCitation
  content: string
}) {
  const start = scalarOffsetToUtf16(content, Number(citation.startChar))
  const end = scalarOffsetToUtf16(content, Number(citation.endChar))
  if (start >= end || content.slice(start, end) !== citation.quote) {
    // 位置与原文不再一致时仍展示全文，而不是把高亮画到错误位置。
    return <>{content}</>
  }
  return (
    <>
      {content.slice(0, start)}
      <mark className="rounded-sm bg-violet-500/25 text-foreground">
        {citation.quote}
      </mark>
      {content.slice(end)}
    </>
  )
}
