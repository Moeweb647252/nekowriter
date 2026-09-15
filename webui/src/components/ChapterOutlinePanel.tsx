import { useCallback, useEffect, useRef, useState, type FormEvent } from "react"
import {
  Check,
  FileText,
  LoaderCircle,
  RotateCcw,
  Send,
  Sparkles,
} from "lucide-react"

import { Button } from "@/components/ui/button"
import { CitationList } from "@/components/CitationList"
import { CitableText } from "@/components/CitableText"
import { UserQuestionPanel } from "@/components/UserQuestionPanel"
import { WritingRecoveryPanel } from "@/components/WritingRecoveryPanel"
import { useLatestValue } from "@/hooks/use-latest-value"
import { useWorkspaceDraftPersistence } from "@/hooks/use-workspace-draft"
import { type PickedTextRange } from "@/lib/citations"
import {
  cancelReviewDelegation,
  decideChapterOutlineReview,
  delegateReview,
  getChapterOutline,
  getPreparation,
  submitChapterOutline,
  type ChapterOutlineSnapshot,
  type FeedbackCitation,
  type PreparationSnapshot,
} from "@/lib/api"

type ChapterOutlinePanelProps = {
  versionId: string
  dataDir?: string
  preparation: PreparationSnapshot
  onChanged: (snapshot: PreparationSnapshot) => void
  onError: (message: string) => void
}

export function ChapterOutlinePanel({
  versionId,
  dataDir,
  preparation,
  onChanged,
  onError,
}: ChapterOutlinePanelProps) {
  const [snapshot, setSnapshot] = useState<ChapterOutlineSnapshot | null>(null)
  const [content, setContent] = useState("")
  const [issues, setIssues] = useState("")
  const [citations, setCitations] = useState<FeedbackCitation[]>([])
  const [loading, setLoading] = useState(true)
  const [remoteOutlineChanged, setRemoteOutlineChanged] = useState(false)
  const [pending, setPending] = useState<
    "submit" | "approve" | "reject" | "delegate" | "cancel-delegation" | null
  >(null)
  const {
    value: persistedContent,
    ref: persistedContentRef,
    commit: commitPersistedContent,
  } = useLatestValue("")
  const contentRef = useRef("")
  /** 引用只在取得它的那次细纲修订上成立；修订一变就丢弃草稿引用。 */
  const citationRevisionRef = useRef<string | null>(null)

  const chapterId = preparation.workflow.activeChapterId
  const stage = preparation.workflow.stage
  const automaticOutlineInFlight =
    stage === "writing_outline_generating" &&
    (preparation.workflow.execution === "queued" ||
      preparation.workflow.execution === "request_in_flight")

  useEffect(() => {
    contentRef.current = content
  }, [content])

  const { clear: clearOutlineDraft } = useWorkspaceDraftPersistence({
    dataDir,
    versionId,
    phase: "chapter_outline",
    targetId: chapterId,
    baseRevision: snapshot?.outline.revisionId ?? null,
    text: content,
    dirty: content !== persistedContent,
    onRestore: (draft) => {
      if (contentRef.current !== persistedContentRef.current) return
      contentRef.current = draft.text
      setContent(draft.text)
      if (draft.baseRevision !== (snapshot?.outline.revisionId ?? null)) {
        setRemoteOutlineChanged(true)
      }
    },
  })

  const { clear: clearReviewDraft } = useWorkspaceDraftPersistence({
    dataDir,
    versionId,
    phase: "feedback",
    targetId: chapterId ? `outline:${chapterId}` : null,
    baseRevision: snapshot?.outline.revisionId ?? null,
    text: issues,
    citations,
    dirty: issues.trim().length > 0 || citations.length > 0,
    onRestore: (draft) => {
      if (issues || citations.length > 0) return
      setIssues(draft.text)
      if (Array.isArray(draft.citations)) {
        setCitations(draft.citations as FeedbackCitation[])
      }
    },
  })

  // 只读写 ref 与 setter，identity 稳定，读取副作用可以直接把它放进依赖。
  const applyOutlineSnapshot = useCallback(
    (nextSnapshot: ChapterOutlineSnapshot, preserveDraft = true) => {
      const nextContent = nextSnapshot.outline.content ?? ""
      const hasUnsavedChanges =
        contentRef.current !== persistedContentRef.current
      const changedRemotely = nextContent !== persistedContentRef.current
      commitPersistedContent(nextContent)
      if (nextSnapshot.outline.revisionId !== citationRevisionRef.current) {
        citationRevisionRef.current = nextSnapshot.outline.revisionId
        setCitations([])
      }
      setSnapshot(nextSnapshot)
      if (
        !preserveDraft ||
        !hasUnsavedChanges ||
        nextContent === contentRef.current
      ) {
        contentRef.current = nextContent
        setContent(nextContent)
        setRemoteOutlineChanged(false)
      } else if (changedRemotely) {
        setRemoteOutlineChanged(true)
      }
    },
    [commitPersistedContent, persistedContentRef]
  )

  useEffect(() => {
    let isCurrent = true
    if (!chapterId) {
      // eslint-disable-next-line react-hooks/set-state-in-effect -- 这是「为当前章节重新读取」这一步自己的界面状态：先把上一章的结果清空，面板在无活动章节时整体不渲染。
      setSnapshot(null)
      commitPersistedContent("")
      contentRef.current = ""
      setRemoteOutlineChanged(false)
      setLoading(false)
      return () => {
        isCurrent = false
      }
    }

    setLoading(true)
    getChapterOutline(chapterId)
      .then((nextSnapshot) => {
        if (!isCurrent) return
        applyOutlineSnapshot(nextSnapshot)
        setIssues("")
      })
      .catch((error: unknown) => {
        if (!isCurrent) return
        onError(error instanceof Error ? error.message : "无法读取当前章节细纲")
      })
      .finally(() => {
        if (isCurrent) setLoading(false)
      })

    return () => {
      isCurrent = false
    }
  }, [
    applyOutlineSnapshot,
    chapterId,
    commitPersistedContent,
    onError,
    preparation.workflow.stateRevision,
  ])

  async function refreshOutline() {
    if (!chapterId) return
    const nextSnapshot = await getChapterOutline(chapterId)
    applyOutlineSnapshot(nextSnapshot, false)
    setIssues("")
  }

  /** 把待审细纲中的选中文字固定为一条指向当前修订的引用。 */
  function citeOutline(range: PickedTextRange) {
    const outline = snapshot?.outline
    const revisionId = outline?.revisionId
    if (!outline || !revisionId) {
      onError("当前细纲还没有可引用的修订。")
      return
    }
    setCitations((current) => [
      ...current,
      {
        kind: "document",
        documentId: outline.documentId,
        revisionId,
        chapterId,
        startChar: String(range.startChar),
        endChar: String(range.endChar),
        quote: range.quote,
      },
    ])
  }

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!chapterId) return
    setPending("submit")
    try {
      const nextSnapshot = await submitChapterOutline(
        chapterId,
        preparation.workflow.stateRevision,
        content
      )
      applyOutlineSnapshot(nextSnapshot, false)
      await clearOutlineDraft()
      onChanged(await getPreparation(versionId))
    } catch (error) {
      onError(error instanceof Error ? error.message : "细纲未能提交审核")
    } finally {
      setPending(null)
    }
  }

  async function handleDecision(passed: boolean) {
    const review = snapshot?.outline.review
    if (!chapterId || !review) return
    if (!passed && !issues.trim() && citations.length === 0) {
      onError("退回细纲时请说明需要调整的内容或引用细纲片段")
      return
    }
    setPending(passed ? "approve" : "reject")
    try {
      const next = await decideChapterOutlineReview(
        review.id,
        preparation.workflow.stateRevision,
        passed,
        issues,
        citations
      )
      setIssues("")
      setCitations([])
      await clearReviewDraft()
      onChanged(next)
      await refreshOutline()
    } catch (error) {
      onError(error instanceof Error ? error.message : "细纲审核决定未保存")
    } finally {
      setPending(null)
    }
  }

  async function handleDelegate() {
    const review = snapshot?.outline.review
    if (!review) return
    setPending("delegate")
    try {
      const result = await delegateReview(
        review.id,
        preparation.workflow.stateRevision,
        issues,
        citations
      )
      setIssues("")
      setCitations([])
      await clearReviewDraft()
      onChanged(result.snapshot)
      await refreshOutline()
    } catch (error) {
      onError(error instanceof Error ? error.message : "审核委托未能启动")
    } finally {
      setPending(null)
    }
  }

  async function handleCancelDelegation() {
    const review = snapshot?.outline.review
    if (!review) return
    setPending("cancel-delegation")
    try {
      const result = await cancelReviewDelegation(
        review.id,
        preparation.workflow.stateRevision
      )
      onChanged(result.snapshot)
      await refreshOutline()
    } catch (error) {
      onError(error instanceof Error ? error.message : "审核委托未能取消")
    } finally {
      setPending(null)
    }
  }

  if (!chapterId) return null

  return (
    <section className="mt-6 rounded-2xl border border-border bg-background/75 p-5 shadow-sm">
      <div className="flex items-start gap-3">
        <FileText className="mt-0.5 size-5 text-primary" aria-hidden="true" />
        <div>
          <p className="text-xs font-semibold tracking-[0.14em] text-muted-foreground uppercase">
            章节细纲
          </p>
          <h2 className="mt-1 text-xl font-semibold">
            {loading
              ? "正在读取细纲…"
              : stage === "writing_drafting"
                ? "细纲已通过"
                : "为本章确定推进路径"}
          </h2>
          <p className="mt-1 max-w-2xl text-sm leading-6 text-muted-foreground">
            细纲通过前始终保留当前 Run
            的完整过程；通过后只留下审核通过的细纲正文，作为正文阶段的折叠上下文。
          </p>
        </div>
      </div>

      {automaticOutlineInFlight && (
        <div className="mt-5 flex items-start gap-3 rounded-xl border border-primary/20 bg-primary/5 px-4 py-3 text-sm leading-6 text-muted-foreground">
          <LoaderCircle
            className="mt-0.5 size-4 shrink-0 animate-spin text-primary"
            aria-hidden="true"
          />
          <div>
            <p className="font-medium text-foreground">模型正在生成本章细纲</p>
            <p className="mt-1">
              已审核的准备阶段产物会作为纯文本折叠上下文随本次请求封存；完成后细纲将直接进入人工审核。
            </p>
          </div>
        </div>
      )}

      <WritingRecoveryPanel
        workflow={preparation.workflow}
        disabled={pending !== null}
        onContinued={async () => {
          await refreshOutline()
          onChanged(await getPreparation(versionId))
        }}
        onError={onError}
      />

      {snapshot?.pendingQuestion && (
        <UserQuestionPanel
          key={snapshot.pendingQuestion.id}
          question={snapshot.pendingQuestion}
          stateRevision={preparation.workflow.stateRevision}
          disabled={pending !== null}
          onAnswered={async () => {
            await refreshOutline()
            onChanged(await getPreparation(versionId))
          }}
          onError={onError}
        />
      )}

      {stage === "writing_outline_generating" &&
        !automaticOutlineInFlight &&
        preparation.workflow.execution !== "waiting_answer" && (
          <form className="mt-5" onSubmit={handleSubmit}>
            <div className="rounded-xl border border-amber-500/20 bg-amber-500/7 px-4 py-3 text-sm leading-6 text-muted-foreground">
              模型请求未能启动或已失败时，你可以直接录入细纲来继续本地创作；它会与模型提交共用同一份审核、依赖和折叠流程，不会被当作临时草稿绕过审核。
            </div>
            <label className="mt-4 block text-sm font-medium">
              本章细纲
              <textarea
                name="chapter-outline"
                autoComplete="off"
                value={content}
                onChange={(event) => setContent(event.target.value)}
                required
                maxLength={2_000_000}
                disabled={pending !== null}
                placeholder="按场景或段落写清本章的冲突、信息揭示、人物选择与结尾钩子。"
                className="mt-1.5 min-h-64 w-full resize-y rounded-xl border border-input bg-background px-3 py-2 text-sm leading-6 transition outline-none placeholder:text-muted-foreground focus:border-ring focus:ring-3 focus:ring-ring/20 disabled:cursor-not-allowed disabled:opacity-60"
              />
            </label>
            {remoteOutlineChanged && (
              <p className="mt-3 rounded-lg border border-amber-500/25 bg-amber-500/8 px-3 py-2 text-xs leading-5 text-amber-950 dark:text-amber-100">
                其他页面已更新已保存细纲；你的本地草稿仍被保留。提交前请确认它仍适用于最新上下文。
              </p>
            )}
            <div className="mt-3 flex flex-wrap items-center justify-between gap-3">
              <p className="text-xs leading-5 text-muted-foreground">
                提交后会固定本次指导和准备产物修订作为审核依赖。
              </p>
              <Button type="submit" disabled={pending !== null}>
                {pending === "submit" ? (
                  <LoaderCircle className="animate-spin" aria-hidden="true" />
                ) : (
                  <Send aria-hidden="true" />
                )}
                提交细纲审核
              </Button>
            </div>
          </form>
        )}

      {stage === "writing_outline_reviewing" && snapshot?.outline.content && (
        <div className="mt-5 overflow-hidden rounded-xl border border-border bg-background/70">
          <div className="flex items-center justify-between gap-3 border-b border-border px-4 py-3">
            <div>
              <h3 className="text-sm font-semibold">待审细纲</h3>
              <p className="mt-0.5 text-xs text-muted-foreground">
                修订 {snapshot.outline.revisionNo}
              </p>
            </div>
            <span className="rounded-full bg-muted px-2 py-1 text-xs text-muted-foreground">
              {snapshot.outline.review?.status === "delegated"
                ? "审核 Agent 处理中"
                : "待审核"}
            </span>
          </div>
          {snapshot.outline.review?.status === "open" ? (
            <CitableText
              text={snapshot.outline.content}
              disabled={pending !== null}
              onCite={citeOutline}
              className="max-h-96 overflow-auto px-4 py-3 text-sm leading-6 whitespace-pre-wrap text-foreground"
            />
          ) : (
            <pre className="max-h-96 overflow-auto px-4 py-3 text-sm leading-6 whitespace-pre-wrap text-foreground">
              {snapshot.outline.content}
            </pre>
          )}
          {snapshot.outline.review?.status === "open" && (
            <div className="border-t border-border p-4">
              <CitationList
                citations={citations}
                currentRevisionId={snapshot.outline.revisionId}
                disabled={pending !== null}
                onRemove={(index) =>
                  setCitations((current) =>
                    current.filter((_, position) => position !== index)
                  )
                }
                onError={onError}
              />
              <label className="block text-xs font-medium text-muted-foreground">
                审核意见（退回修订时必填）
                <textarea
                  name="outline-review-issues"
                  autoComplete="off"
                  value={issues}
                  onChange={(event) => setIssues(event.target.value)}
                  maxLength={20_000}
                  disabled={pending !== null}
                  placeholder="通过可留空；退回时说明需要调整的情节、节奏或信息。"
                  className="mt-1.5 min-h-20 w-full resize-y rounded-lg border border-input bg-background px-3 py-2 text-sm text-foreground outline-none focus:border-ring focus:ring-3 focus:ring-ring/20 disabled:cursor-not-allowed disabled:opacity-60"
                />
              </label>
              <div className="mt-3 flex flex-wrap gap-2">
                <Button
                  type="button"
                  disabled={pending !== null}
                  onClick={() => void handleDecision(true)}
                >
                  {pending === "approve" ? (
                    <LoaderCircle className="animate-spin" aria-hidden="true" />
                  ) : (
                    <Check aria-hidden="true" />
                  )}
                  通过并进入正文
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  disabled={pending !== null}
                  onClick={() => void handleDecision(false)}
                >
                  {pending === "reject" ? (
                    <LoaderCircle className="animate-spin" aria-hidden="true" />
                  ) : (
                    <RotateCcw aria-hidden="true" />
                  )}
                  退回修订
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  disabled={pending !== null}
                  onClick={() => void handleDelegate()}
                >
                  {pending === "delegate" ? (
                    <LoaderCircle className="animate-spin" aria-hidden="true" />
                  ) : (
                    <Sparkles aria-hidden="true" />
                  )}
                  委托审核
                </Button>
              </div>
            </div>
          )}
          {snapshot.outline.review?.status === "delegated" && (
            <div className="flex flex-wrap items-center justify-between gap-3 border-t border-border bg-primary/5 p-4 text-sm text-muted-foreground">
              <span className="flex items-center gap-2">
                <LoaderCircle
                  className="size-4 animate-spin text-primary"
                  aria-hidden="true"
                />
                审核 Agent 正在依据当前细纲审查。
              </span>
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={pending !== null}
                onClick={() => void handleCancelDelegation()}
              >
                {pending === "cancel-delegation" && (
                  <LoaderCircle className="animate-spin" aria-hidden="true" />
                )}
                取消并自行审核
              </Button>
            </div>
          )}
        </div>
      )}

      {stage === "writing_drafting" && snapshot?.contextFold && (
        <div className="mt-5 rounded-xl border border-emerald-600/20 bg-emerald-600/6 p-4">
          <div className="flex items-start gap-3">
            <Check
              className="mt-0.5 size-5 text-emerald-700"
              aria-hidden="true"
            />
            <div>
              <h3 className="font-medium">细纲已通过并完成上下文折叠</h3>
              <p className="mt-1 text-sm leading-6 text-muted-foreground">
                正文阶段只会使用下面这份细纲全文；审核意见、队列状态和内部元数据没有混入。
              </p>
            </div>
          </div>
          <pre className="mt-4 max-h-80 overflow-auto rounded-lg border border-emerald-700/15 bg-background/75 p-3 text-sm leading-6 whitespace-pre-wrap">
            {snapshot.contextFold.plainText}
          </pre>
        </div>
      )}
    </section>
  )
}
