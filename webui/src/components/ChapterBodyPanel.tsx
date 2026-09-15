import { useCallback, useEffect, useRef, useState, type FormEvent } from "react"
import {
  BookOpenCheck,
  Check,
  LoaderCircle,
  Pause,
  Play,
  RotateCcw,
  Save,
  Send,
  Sparkles,
} from "lucide-react"

import { Button } from "@/components/ui/button"
import { CitationList } from "@/components/CitationList"
import { CitableText } from "@/components/CitableText"
import { ExtraChapterPanel } from "@/components/ExtraChapterPanel"
import { TemplateInserter } from "@/components/TemplatePanel"
import { UserQuestionPanel } from "@/components/UserQuestionPanel"
import { WritingRecoveryPanel } from "@/components/WritingRecoveryPanel"
import { type PickedTextRange } from "@/lib/citations"
import { useLatestValue } from "@/hooks/use-latest-value"
import { useWorkspaceDraftPersistence } from "@/hooks/use-workspace-draft"
import {
  advanceToNextChapter,
  appendChapterBody,
  cancelReviewDelegation,
  confirmNovelCompletion,
  decideChapterBodyProposal,
  decideChapterFinalReview,
  delegateReview,
  getChapterBody,
  getChapters,
  getPreparation,
  replaceChapterBody,
  requestChapterBodyPause,
  resumeChapterBodyPause,
  setApprovalMode,
  submitRunFeedback,
  submitChapterFinalReview,
  type ChapterBodySnapshot,
  type ChapterSummary,
  type FeedbackCitation,
  type PreparationSnapshot,
} from "@/lib/api"

type ChapterBodyPanelProps = {
  versionId: string
  /** 本地服务数据目录用于隔离不同作品库的浏览器草稿。 */
  dataDir?: string
  preparation: PreparationSnapshot
  onChanged: (snapshot: PreparationSnapshot) => void
  onError: (message: string) => void
  /** 模板列表变化时递增，使插入器不会停留在旧选项上。 */
  templatesRefreshToken?: number
}

/** 待提交引用按章节隔离，切换章节后不再复用旧目标上的引用。 */
type CitationDraft = {
  chapterId: string
  items: FeedbackCitation[]
}

export function ChapterBodyPanel({
  versionId,
  dataDir,
  preparation,
  onChanged,
  onError,
  templatesRefreshToken,
}: ChapterBodyPanelProps) {
  const [snapshot, setSnapshot] = useState<ChapterBodySnapshot | null>(null)
  const [content, setContent] = useState("")
  const [issues, setIssues] = useState("")
  const [proposalFeedback, setProposalFeedback] = useState("")
  const [freeFeedback, setFreeFeedback] = useState("")
  const [proposalCitationDraft, setProposalCitations] = useState<CitationDraft>(
    {
      chapterId: "",
      items: [],
    }
  )
  const [feedbackCitationDraft, setFeedbackCitations] = useState<CitationDraft>(
    {
      chapterId: "",
      items: [],
    }
  )
  const [finalCitationDraft, setFinalCitations] = useState<CitationDraft>({
    chapterId: "",
    items: [],
  })
  const [loading, setLoading] = useState(true)
  const [nextChapter, setNextChapter] = useState<ChapterSummary | null>(null)
  const [nextChapterLoading, setNextChapterLoading] = useState(false)
  const [allMainChaptersApproved, setAllMainChaptersApproved] = useState(false)
  const [remoteBodyChanged, setRemoteBodyChanged] = useState(false)
  const [pending, setPending] = useState<
    | "save"
    | "submit"
    | "approve"
    | "reject"
    | "delegate"
    | "cancel-delegation"
    | "proposalAccept"
    | "proposalReject"
    | "feedback"
    | "pause"
    | "resume"
    | "mode"
    | "next"
    | "complete"
    | null
  >(null)
  const {
    value: persistedContent,
    ref: persistedContentRef,
    commit: commitPersistedContent,
  } = useLatestValue("")
  const contentRef = useRef("")

  const chapterId = preparation.workflow.activeChapterId
  const stage = preparation.workflow.stage
  // 引用锚定在具体修订或候选上；切换章节后旧引用不再对应可见文本，因此按章节键
  // 丢弃草稿，不用副作用在渲染后补一次清空。
  const proposalCitations =
    proposalCitationDraft.chapterId === chapterId
      ? proposalCitationDraft.items
      : []
  const feedbackCitations =
    feedbackCitationDraft.chapterId === chapterId
      ? feedbackCitationDraft.items
      : []
  const finalCitations =
    finalCitationDraft.chapterId === chapterId ? finalCitationDraft.items : []

  useEffect(() => {
    contentRef.current = content
  }, [content])

  const { clear: clearBodyDraft } = useWorkspaceDraftPersistence({
    dataDir,
    versionId,
    phase: "chapter_body",
    targetId: chapterId,
    baseRevision: snapshot?.body.revisionId ?? null,
    text: content,
    dirty: content !== persistedContent,
    onRestore: (draft) => {
      if (contentRef.current !== persistedContentRef.current) return
      contentRef.current = draft.text
      setContent(draft.text)
      if (draft.baseRevision !== (snapshot?.body.revisionId ?? null)) {
        setRemoteBodyChanged(true)
      }
    },
  })

  const { clear: clearFeedbackDraft } = useWorkspaceDraftPersistence({
    dataDir,
    versionId,
    phase: "feedback",
    targetId: chapterId,
    baseRevision: snapshot?.body.revisionId ?? null,
    text: freeFeedback,
    citations: feedbackCitations,
    dirty: freeFeedback.trim().length > 0 || feedbackCitations.length > 0,
    onRestore: (draft) => {
      if (!chapterId || freeFeedback || feedbackCitations.length > 0) return
      setFreeFeedback(draft.text)
      if (Array.isArray(draft.citations)) {
        setFeedbackCitations({
          chapterId,
          items: draft.citations as FeedbackCitation[],
        })
      }
    },
  })

  const { clear: clearProposalDraft } = useWorkspaceDraftPersistence({
    dataDir,
    versionId,
    phase: "feedback",
    targetId: chapterId ? `proposal:${chapterId}` : null,
    baseRevision: snapshot?.body.revisionId ?? null,
    text: proposalFeedback,
    citations: proposalCitations,
    dirty: proposalFeedback.trim().length > 0 || proposalCitations.length > 0,
    onRestore: (draft) => {
      if (!chapterId || proposalFeedback || proposalCitations.length > 0) return
      setProposalFeedback(draft.text)
      if (Array.isArray(draft.citations)) {
        setProposalCitations({
          chapterId,
          items: draft.citations as FeedbackCitation[],
        })
      }
    },
  })

  const { clear: clearFinalReviewDraft } = useWorkspaceDraftPersistence({
    dataDir,
    versionId,
    phase: "feedback",
    targetId: chapterId ? `final-review:${chapterId}` : null,
    baseRevision: snapshot?.body.revisionId ?? null,
    text: issues,
    citations: finalCitations,
    dirty: issues.trim().length > 0 || finalCitations.length > 0,
    onRestore: (draft) => {
      if (!chapterId || issues || finalCitations.length > 0) return
      setIssues(draft.text)
      if (Array.isArray(draft.citations)) {
        setFinalCitations({
          chapterId,
          items: draft.citations as FeedbackCitation[],
        })
      }
    },
  })

  // 只读写 ref 与 setter，identity 稳定，读取副作用可以直接把它放进依赖。
  const applyBodySnapshot = useCallback(
    (nextSnapshot: ChapterBodySnapshot, preserveDraft = true) => {
      const nextContent = nextSnapshot.body.content ?? ""
      const hasUnsavedChanges =
        contentRef.current !== persistedContentRef.current
      const changedRemotely = nextContent !== persistedContentRef.current
      commitPersistedContent(nextContent)
      setSnapshot(nextSnapshot)
      if (
        !preserveDraft ||
        !hasUnsavedChanges ||
        nextContent === contentRef.current
      ) {
        contentRef.current = nextContent
        setContent(nextContent)
        setRemoteBodyChanged(false)
      } else if (changedRemotely) {
        setRemoteBodyChanged(true)
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
      setRemoteBodyChanged(false)
      setLoading(false)
      return () => {
        isCurrent = false
      }
    }

    setLoading(true)
    getChapterBody(chapterId)
      .then((nextSnapshot) => {
        if (!isCurrent) return
        applyBodySnapshot(nextSnapshot)
        setIssues("")
      })
      .catch((error: unknown) => {
        if (!isCurrent) return
        onError(error instanceof Error ? error.message : "无法读取当前章节正文")
      })
      .finally(() => {
        if (isCurrent) setLoading(false)
      })

    return () => {
      isCurrent = false
    }
  }, [
    applyBodySnapshot,
    chapterId,
    commitPersistedContent,
    onError,
    preparation.workflow.stateRevision,
  ])

  useEffect(() => {
    let isCurrent = true
    if (stage !== "writing_chapter_done" || !chapterId) {
      // eslint-disable-next-line react-hooks/set-state-in-effect -- 这是「为当前章节重新读取」这一步自己的界面状态：先把上一章的结果清空，面板在无活动章节时整体不渲染。
      setNextChapter(null)
      setAllMainChaptersApproved(false)
      setNextChapterLoading(false)
      return () => {
        isCurrent = false
      }
    }

    setNextChapterLoading(true)
    getChapters(versionId)
      .then((chapters) => {
        if (!isCurrent) return
        const currentChapter = chapters.find(
          (chapter) => chapter.id === chapterId
        )
        const followingChapter = currentChapter
          ? chapters.find(
              (chapter) =>
                chapter.kind === currentChapter.kind &&
                BigInt(chapter.orderIndex) > BigInt(currentChapter.orderIndex)
            )
          : undefined
        const mainChapters = chapters.filter(
          (chapter) => chapter.kind === "main"
        )
        setNextChapter(
          followingChapter?.status === "planned" ? followingChapter : null
        )
        setAllMainChaptersApproved(
          mainChapters.length > 0 &&
            mainChapters.every((chapter) => chapter.status === "approved")
        )
      })
      .catch((error: unknown) => {
        if (!isCurrent) return
        onError(error instanceof Error ? error.message : "无法读取章节计划")
      })
      .finally(() => {
        if (isCurrent) setNextChapterLoading(false)
      })

    return () => {
      isCurrent = false
    }
  }, [chapterId, onError, stage, versionId])

  async function refreshBody() {
    if (!chapterId) return
    const nextSnapshot = await getChapterBody(chapterId)
    applyBodySnapshot(nextSnapshot, false)
    setIssues("")
  }

  /**
   * 把正文修订中的选中文本固定成正式文档引用。
   *
   * 引用锚定当前不可变修订；没有修订时明确报错，而不是退化成无法校验的偏移。
   */
  function documentCitation(range: PickedTextRange): FeedbackCitation | null {
    const body = snapshot?.body
    if (!chapterId || !body?.revisionId) {
      onError("当前还没有可引用的正式正文修订。")
      return null
    }
    return {
      kind: "document",
      documentId: body.documentId,
      revisionId: body.revisionId,
      chapterId,
      startChar: String(range.startChar),
      endChar: String(range.endChar),
      quote: range.quote,
    }
  }

  /**
   * 把待审候选中的选中文本固定成候选引用。
   *
   * 候选还没有修订，只能锚定工具调用、候选哈希与候选区间，由服务端写入时重新校验。
   */
  function candidateCitation(range: PickedTextRange): FeedbackCitation | null {
    const currentProposal = snapshot?.pendingProposal
    if (!currentProposal) return null
    return {
      kind: "candidate",
      toolCallId: currentProposal.toolCallId,
      candidateHash: currentProposal.candidateHash,
      candidateRange: {
        startChar: String(range.startChar),
        endChar: String(range.endChar),
        quote: range.quote,
      },
    }
  }

  function citeProposalFromDocument(range: PickedTextRange) {
    const citation = documentCitation(range)
    if (!chapterId || !citation) return
    setProposalCitations((current) => ({
      chapterId,
      items: [
        ...(current.chapterId === chapterId ? current.items : []),
        citation,
      ],
    }))
  }

  function citeProposalFromCandidate(range: PickedTextRange) {
    const citation = candidateCitation(range)
    if (!chapterId || !citation) return
    setProposalCitations((current) => ({
      chapterId,
      items: [
        ...(current.chapterId === chapterId ? current.items : []),
        citation,
      ],
    }))
  }

  function citeFeedbackFromDocument(range: PickedTextRange) {
    const citation = documentCitation(range)
    if (!chapterId || !citation) return
    setFeedbackCitations((current) => ({
      chapterId,
      items: [
        ...(current.chapterId === chapterId ? current.items : []),
        citation,
      ],
    }))
  }

  /** 把待审终稿中的选中文字固定为一条指向当前修订的引用。 */
  function citeFinalReview(range: PickedTextRange) {
    const citation = documentCitation(range)
    if (!chapterId || !citation) return
    setFinalCitations((current) => ({
      chapterId,
      items: [
        ...(current.chapterId === chapterId ? current.items : []),
        citation,
      ],
    }))
  }

  async function handleSave(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!chapterId) return
    const previousContent = snapshot?.body.content ?? ""
    if (!previousContent && !content.trim()) {
      onError("请先写入非空正文")
      return
    }
    setPending("save")
    try {
      const nextSnapshot = previousContent
        ? await replaceChapterBody(
            chapterId,
            preparation.workflow.stateRevision,
            previousContent,
            content
          )
        : await appendChapterBody(
            chapterId,
            preparation.workflow.stateRevision,
            content
          )
      applyBodySnapshot(nextSnapshot, false)
      await clearBodyDraft()
      onChanged(await getPreparation(versionId))
    } catch (error) {
      onError(error instanceof Error ? error.message : "正文未能保存")
    } finally {
      setPending(null)
    }
  }

  async function handleSubmitFinalReview() {
    if (!chapterId || !content.trim()) {
      onError("正文不能为空，不能提交整章审核")
      return
    }
    if (content !== (snapshot?.body.content ?? "")) {
      onError("正文有未保存修改；请先保存当前修订，再提交整章审核")
      return
    }
    setPending("submit")
    try {
      applyBodySnapshot(
        await submitChapterFinalReview(
          chapterId,
          preparation.workflow.stateRevision
        ),
        false
      )
      onChanged(await getPreparation(versionId))
    } catch (error) {
      onError(error instanceof Error ? error.message : "整章审核未能启动")
    } finally {
      setPending(null)
    }
  }

  async function handleDecision(passed: boolean) {
    const review = snapshot?.body.review
    if (!chapterId || !review) return
    if (!passed && !issues.trim() && finalCitations.length === 0) {
      onError("退回终稿时请说明需要调整的内容或引用正文片段")
      return
    }
    setPending(passed ? "approve" : "reject")
    try {
      const next = await decideChapterFinalReview(
        review.id,
        preparation.workflow.stateRevision,
        passed,
        issues,
        finalCitations
      )
      setIssues("")
      setFinalCitations({ chapterId, items: [] })
      await clearFinalReviewDraft()
      onChanged(next)
      await refreshBody()
    } catch (error) {
      onError(error instanceof Error ? error.message : "终稿审核决定未保存")
    } finally {
      setPending(null)
    }
  }

  async function handleDelegate() {
    const review = snapshot?.body.review
    if (!chapterId || !review) return
    setPending("delegate")
    try {
      const result = await delegateReview(
        review.id,
        preparation.workflow.stateRevision,
        issues,
        finalCitations
      )
      setIssues("")
      setFinalCitations({ chapterId, items: [] })
      await clearFinalReviewDraft()
      onChanged(result.snapshot)
      await refreshBody()
    } catch (error) {
      onError(error instanceof Error ? error.message : "审核委托未能启动")
    } finally {
      setPending(null)
    }
  }

  async function handleCancelDelegation() {
    const review = snapshot?.body.review
    if (!review) return
    setPending("cancel-delegation")
    try {
      const result = await cancelReviewDelegation(
        review.id,
        preparation.workflow.stateRevision
      )
      onChanged(result.snapshot)
      await refreshBody()
    } catch (error) {
      onError(error instanceof Error ? error.message : "审核委托未能取消")
    } finally {
      setPending(null)
    }
  }

  async function handleApprovalMode(
    nextMode: "auto_approve" | "approve_required"
  ) {
    if (nextMode === preparation.workflow.approvalMode) return
    setPending("mode")
    try {
      await setApprovalMode(
        versionId,
        snapshot?.workflow.stateRevision ?? preparation.workflow.stateRevision,
        nextMode
      )
      onChanged(await getPreparation(versionId))
    } catch (error) {
      onError(error instanceof Error ? error.message : "正文审核模式未能更新")
    } finally {
      setPending(null)
    }
  }

  async function handleProposalDecision(accept: boolean) {
    const proposal = snapshot?.pendingProposal
    const review = snapshot?.pendingProposalReview
    if (
      !chapterId ||
      !proposal ||
      !review ||
      !snapshot ||
      review.status !== "open"
    ) {
      return
    }
    const hasAttachment =
      proposalFeedback.trim().length > 0 || proposalCitations.length > 0
    if (!accept && !hasAttachment) {
      onError("拒绝编辑时请说明需要调整的内容或引用正文片段")
      return
    }
    setPending(accept ? "proposalAccept" : "proposalReject")
    try {
      applyBodySnapshot(
        await decideChapterBodyProposal(
          chapterId,
          proposal.toolCallId,
          snapshot.workflow.stateRevision,
          accept ? "" : proposalFeedback,
          accept ? [] : proposalCitations
        ),
        false
      )
      setProposalFeedback("")
      setProposalCitations({ chapterId, items: [] })
      await clearProposalDraft()
      onChanged(await getPreparation(versionId))
    } catch (error) {
      onError(error instanceof Error ? error.message : "正文编辑提案未能决定")
    } finally {
      setPending(null)
    }
  }

  async function handleProposalDelegate() {
    const review = snapshot?.pendingProposalReview
    if (!chapterId || !review || review.status !== "open") return
    setPending("delegate")
    try {
      const result = await delegateReview(
        review.id,
        preparation.workflow.stateRevision,
        proposalFeedback,
        proposalCitations
      )
      setProposalFeedback("")
      setProposalCitations({ chapterId, items: [] })
      await clearProposalDraft()
      onChanged(result.snapshot)
      await refreshBody()
    } catch (error) {
      onError(error instanceof Error ? error.message : "编辑审核委托未能启动")
    } finally {
      setPending(null)
    }
  }

  async function handleProposalDelegationCancel() {
    const review = snapshot?.pendingProposalReview
    if (!review || review.status !== "delegated") return
    setPending("cancel-delegation")
    try {
      const result = await cancelReviewDelegation(
        review.id,
        preparation.workflow.stateRevision
      )
      onChanged(result.snapshot)
      await refreshBody()
    } catch (error) {
      onError(error instanceof Error ? error.message : "编辑审核委托未能取消")
    } finally {
      setPending(null)
    }
  }

  async function handleSubmitFreeFeedback() {
    const runId = snapshot?.workflow.activeRunId
    if (!runId || !snapshot || !chapterId) return
    if (!freeFeedback.trim() && feedbackCitations.length === 0) {
      onError("请填写意见或引用至少一段正文")
      return
    }
    setPending("feedback")
    try {
      applyBodySnapshot(
        await submitRunFeedback(
          runId,
          snapshot.workflow.stateRevision,
          freeFeedback,
          feedbackCitations
        )
      )
      setFreeFeedback("")
      setFeedbackCitations({ chapterId, items: [] })
      await clearFeedbackDraft()
      onChanged(await getPreparation(versionId))
    } catch (error) {
      onError(error instanceof Error ? error.message : "自由意见未能保存")
    } finally {
      setPending(null)
    }
  }

  async function handlePause() {
    const runId = snapshot?.workflow.activeRunId
    if (!runId || !snapshot) return
    setPending("pause")
    try {
      applyBodySnapshot(
        await requestChapterBodyPause(runId, snapshot.workflow.stateRevision)
      )
      onChanged(await getPreparation(versionId))
    } catch (error) {
      onError(error instanceof Error ? error.message : "暂停请求未能保存")
    } finally {
      setPending(null)
    }
  }

  async function handleResume() {
    const runId = snapshot?.workflow.activeRunId
    if (!runId || !snapshot) return
    setPending("resume")
    try {
      applyBodySnapshot(
        await resumeChapterBodyPause(runId, snapshot.workflow.stateRevision)
      )
      onChanged(await getPreparation(versionId))
    } catch (error) {
      onError(error instanceof Error ? error.message : "暂停的工具调用未能恢复")
    } finally {
      setPending(null)
    }
  }

  async function handleAdvanceToNextChapter() {
    if (!nextChapter) return
    setPending("next")
    try {
      onChanged(
        await advanceToNextChapter(
          versionId,
          preparation.workflow.stateRevision
        )
      )
    } catch (error) {
      onError(error instanceof Error ? error.message : "无法进入下一章")
    } finally {
      setPending(null)
    }
  }

  async function handleConfirmCompletion() {
    if (!allMainChaptersApproved) return
    setPending("complete")
    try {
      onChanged(
        await confirmNovelCompletion(
          versionId,
          preparation.workflow.stateRevision
        )
      )
    } catch (error) {
      onError(error instanceof Error ? error.message : "无法确认完本")
    } finally {
      setPending(null)
    }
  }

  if (!chapterId) return null

  const hasUnsavedBodyChanges = content !== (snapshot?.body.content ?? "")
  const automaticBodyInFlight =
    stage === "writing_drafting" &&
    (preparation.workflow.execution === "queued" ||
      preparation.workflow.execution === "request_in_flight")
  const approvalMode =
    snapshot?.workflow.approvalMode ?? preparation.workflow.approvalMode
  const proposal = snapshot?.pendingProposal ?? null
  const proposalReview = snapshot?.pendingProposalReview ?? null
  const heldTool = snapshot?.heldTool ?? null
  const heldSubmission = snapshot?.heldSubmission ?? null
  const pendingFeedback = snapshot?.pendingFeedback ?? []
  const deliveredFeedback = snapshot?.deliveredFeedback ?? []
  const currentRevisionId = snapshot?.body.revisionId ?? null
  const hasProposalAttachment =
    proposalFeedback.trim().length > 0 || proposalCitations.length > 0

  return (
    <section className="mt-6 rounded-2xl border border-border bg-background/75 p-5 shadow-sm">
      <div className="flex items-start gap-3">
        <BookOpenCheck
          className="mt-0.5 size-5 text-primary"
          aria-hidden="true"
        />
        <div>
          <p className="text-xs font-semibold tracking-[0.14em] text-muted-foreground uppercase">
            章节正文
          </p>
          <h2 className="mt-1 text-xl font-semibold">
            {loading
              ? "正在读取正文…"
              : stage === "completed"
                ? "本稿本已完本"
                : stage === "writing_chapter_done"
                  ? "本章已定稿"
                  : "让细纲落成完整场景"}
          </h2>
          <p className="mt-1 max-w-2xl text-sm leading-6 text-muted-foreground">
            每次接受的编辑都形成可追溯的正文修订。正文 Agent 只可通过
            append/replace
            修改当前章节；逐次确认模式会先展示候选，模型终态仍必须进入整章审核。
          </p>
        </div>
      </div>

      {snapshot?.outlineContextFold && stage !== "writing_chapter_done" && (
        <p className="mt-4 rounded-xl border border-primary/15 bg-primary/5 px-4 py-3 text-sm leading-6 text-muted-foreground">
          本章细纲已经审核通过并折叠为正文阶段输入。正文保存不会混入细纲审核意见或运行元数据。
        </p>
      )}

      {stage === "writing_drafting" && (
        <div className="mt-5 rounded-xl border border-border bg-muted/20 p-4">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div>
              <h3 className="text-sm font-semibold">正文编辑审核模式</h3>
              <p className="mt-1 text-xs leading-5 text-muted-foreground">
                模式立即作用于下一条完整的 Agent
                编辑调用；已经出现的待审编辑仍须由你明确决定。
              </p>
            </div>
            <div
              className="flex rounded-xl border border-border bg-background p-1"
              role="group"
              aria-label="正文编辑审核模式"
            >
              <Button
                type="button"
                size="sm"
                variant={approvalMode === "auto_approve" ? "default" : "ghost"}
                aria-pressed={approvalMode === "auto_approve"}
                disabled={pending !== null}
                onClick={() => void handleApprovalMode("auto_approve")}
              >
                自动接受
              </Button>
              <Button
                type="button"
                size="sm"
                variant={
                  approvalMode === "approve_required" ? "default" : "ghost"
                }
                aria-pressed={approvalMode === "approve_required"}
                disabled={pending !== null}
                onClick={() => void handleApprovalMode("approve_required")}
              >
                逐次确认
              </Button>
            </div>
          </div>
        </div>
      )}

      {automaticBodyInFlight && (
        <div className="mt-5 rounded-xl border border-primary/20 bg-primary/5 px-4 py-4 text-sm leading-6 text-muted-foreground">
          <LoaderCircle
            className="mr-2 inline size-4 animate-spin text-primary"
            aria-hidden="true"
          />
          正文 Agent
          正在本地队列中运行。每个完整工具调用都会先持久化结果；自动接受模式直接形成正文修订，逐次确认模式则停在候选编辑，之后才可能请求下一轮。
        </div>
      )}

      <WritingRecoveryPanel
        workflow={preparation.workflow}
        disabled={pending !== null}
        onContinued={async () => {
          await refreshBody()
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
            await refreshBody()
            onChanged(await getPreparation(versionId))
          }}
          onError={onError}
        />
      )}

      {stage === "writing_drafting" &&
        snapshot?.workflow.activeRunId &&
        !proposal && (
          <section className="mt-5 rounded-xl border border-violet-500/20 bg-violet-500/5 p-4">
            <div className="flex flex-wrap items-start justify-between gap-3">
              <div>
                <h3 className="text-sm font-semibold">正文执行控制</h3>
                <p className="mt-1 max-w-2xl text-xs leading-5 text-muted-foreground">
                  {snapshot.workflow.execution === "paused"
                    ? "已在一条完整工具调用边界暂停；正式正文尚未因该调用改变。"
                    : snapshot.workflow.pauseRequested
                      ? "正在等待当前或下一次完整工具调用到达暂停边界；已发出的模型请求不会被取消或改写。"
                      : "可请求在下一条完整工具调用边界暂停。"}
                </p>
              </div>
              {snapshot.workflow.execution === "paused" ? (
                <Button
                  type="button"
                  disabled={pending !== null}
                  onClick={() => void handleResume()}
                >
                  {pending === "resume" ? (
                    <LoaderCircle className="animate-spin" aria-hidden="true" />
                  ) : (
                    <Play aria-hidden="true" />
                  )}
                  继续执行
                </Button>
              ) : (
                <Button
                  type="button"
                  variant="outline"
                  disabled={
                    pending !== null ||
                    snapshot.workflow.pauseRequested ||
                    (snapshot.workflow.execution !== "queued" &&
                      snapshot.workflow.execution !== "request_in_flight")
                  }
                  onClick={() => void handlePause()}
                >
                  {pending === "pause" ? (
                    <LoaderCircle className="animate-spin" aria-hidden="true" />
                  ) : (
                    <Pause aria-hidden="true" />
                  )}
                  请求暂停
                </Button>
              )}
            </div>
            {heldTool && (
              <div className="mt-3 rounded-lg border border-violet-500/15 bg-background/75 px-3 py-3 text-xs leading-5 text-muted-foreground">
                <p>
                  已挂起{" "}
                  {heldTool.toolName === "append_content" ? "追加" : "替换"}{" "}
                  调用；
                  {heldTool.baseRevisionNo
                    ? `基础修订为 ${heldTool.baseRevisionNo}。`
                    : "基础正文为空。"}
                </p>
                {heldTool.candidateContent !== null && (
                  <pre className="mt-2 max-h-40 overflow-auto rounded-md border border-border bg-muted/20 p-2 text-sm leading-6 whitespace-pre-wrap text-foreground">
                    {heldTool.candidateContent || "（该候选会清空正文）"}
                  </pre>
                )}
              </div>
            )}
            {heldSubmission && (
              <div className="mt-3 rounded-lg border border-violet-500/15 bg-background/75 px-3 py-3 text-xs leading-5 text-muted-foreground">
                <p>
                  Agent
                  已提交本章完成信号，暂停使它尚未进入终稿审核；继续后会重新核对正文与待投递意见。
                </p>
                <p className="mt-1 font-medium text-foreground">
                  {heldSubmission.summary}
                </p>
              </div>
            )}
          </section>
        )}

      {stage === "writing_drafting" &&
        snapshot?.workflow.activeRunId &&
        !proposal && (
          <section className="mt-5 rounded-xl border border-sky-500/20 bg-sky-500/5 p-4">
            <div className="flex flex-wrap items-start justify-between gap-3">
              <div>
                <h3 className="text-sm font-semibold">交给正文 Agent 的意见</h3>
                <p className="mt-1 max-w-2xl text-xs leading-5 text-muted-foreground">
                  这不是逐次编辑审核。意见会独立持久化，并在同一批次的目标工具结果随下一次模型请求一起交给
                  Agent；已经发出的请求不会被改写。
                  {snapshot.workflow.execution === "paused"
                    ? " 当前已暂停，提交意见会取消暂停并按当前边界恢复；工具结果会携带该批意见。"
                    : ""}
                </p>
              </div>
              {pendingFeedback.length > 0 && (
                <span className="rounded-full bg-sky-500/10 px-2 py-1 text-xs font-medium text-sky-900 dark:text-sky-100">
                  队列 {pendingFeedback.length} 条
                </span>
              )}
            </div>
            {pendingFeedback.length > 0 && (
              <ol className="mt-3 space-y-2">
                {pendingFeedback.map((feedback) => (
                  <li
                    key={feedback.id}
                    className="rounded-lg border border-sky-500/15 bg-background/70 px-3 py-2 text-xs leading-5 text-muted-foreground"
                  >
                    <span className="mr-2 font-medium text-foreground">
                      {feedback.status === "bound"
                        ? "已绑定，等待随请求投递"
                        : "等待下一条工具结果"}
                    </span>
                    {feedback.text}
                    {feedback.citations.length > 0 && (
                      <CitationList
                        citations={feedback.citations}
                        currentRevisionId={currentRevisionId}
                        onError={onError}
                      />
                    )}
                  </li>
                ))}
              </ol>
            )}
            {deliveredFeedback.length > 0 && (
              <div className="mt-3 rounded-lg border border-sky-500/15 bg-background/60 p-3">
                <p className="text-xs font-medium text-foreground">
                  已封存的意见
                </p>
                <ol className="mt-2 space-y-2">
                  {deliveredFeedback.map((feedback) => (
                    <li
                      key={feedback.id}
                      className="rounded-lg border border-sky-500/15 bg-background/70 px-3 py-2 text-xs leading-5 text-muted-foreground"
                    >
                      <span className="mr-2 font-medium text-foreground">
                        {feedback.deliveryStatus === "sent"
                          ? "已随请求发送，等待模型回应"
                          : feedback.deliveryStatus === "failed"
                            ? "请求失败，尚无模型回应"
                            : feedback.deliveryStatus === "unknown"
                              ? "请求状态未知，尚无模型回应"
                              : "请求已取消，尚无模型回应"}
                      </span>
                      {feedback.text}
                      {feedback.citations.length > 0 && (
                        <CitationList
                          citations={feedback.citations}
                          currentRevisionId={currentRevisionId}
                          onError={onError}
                        />
                      )}
                    </li>
                  ))}
                </ol>
              </div>
            )}
            {snapshot.body.revisionId && snapshot.body.content && (
              <details className="mt-3 rounded-lg border border-sky-500/15 bg-background/60 px-3 py-2">
                <summary className="cursor-pointer text-xs font-medium text-muted-foreground">
                  引用当前正文（选中文字后点击“引用选中文字”）
                </summary>
                <CitableText
                  text={snapshot.body.content}
                  disabled={pending !== null}
                  onCite={citeFeedbackFromDocument}
                  className="mt-2 max-h-72 overflow-auto text-sm leading-7 whitespace-pre-wrap text-foreground"
                />
              </details>
            )}
            <div className="mt-3">
              <TemplateInserter
                label="插入模板到意见草稿"
                refreshToken={templatesRefreshToken}
                onInsert={(text) =>
                  setFreeFeedback((current) =>
                    current.trim().length > 0 ? `${current}\n${text}` : text
                  )
                }
              />
            </div>
            <label className="mt-3 block text-xs font-medium text-muted-foreground">
              新意见
              <textarea
                name="run-feedback"
                autoComplete="off"
                value={freeFeedback}
                onChange={(event) => setFreeFeedback(event.target.value)}
                maxLength={20_000}
                disabled={pending !== null}
                placeholder="例如：后续让两人的对话更多由动作和停顿承载，不要急于解释旧日误会。"
                className="mt-1.5 min-h-20 w-full resize-y rounded-lg border border-input bg-background px-3 py-2 text-sm text-foreground outline-none focus:border-ring focus:ring-3 focus:ring-ring/20 disabled:cursor-not-allowed disabled:opacity-60"
              />
            </label>
            <CitationList
              citations={feedbackCitations}
              currentRevisionId={currentRevisionId}
              disabled={pending !== null}
              onRemove={(index) =>
                setFeedbackCitations((current) => ({
                  chapterId,
                  items: (current.chapterId === chapterId
                    ? current.items
                    : []
                  ).filter((_, position) => position !== index),
                }))
              }
              onError={onError}
            />
            <div className="mt-3 flex justify-end">
              <Button
                type="button"
                variant="outline"
                disabled={
                  pending !== null ||
                  (!freeFeedback.trim() && feedbackCitations.length === 0)
                }
                onClick={() => void handleSubmitFreeFeedback()}
              >
                {pending === "feedback" ? (
                  <LoaderCircle className="animate-spin" aria-hidden="true" />
                ) : (
                  <Send aria-hidden="true" />
                )}
                提交意见
              </Button>
            </div>
          </section>
        )}

      {stage === "writing_drafting" && proposal && (
        <div className="mt-5 overflow-hidden rounded-xl border border-amber-500/30 bg-amber-500/8">
          <div className="flex flex-wrap items-start justify-between gap-3 border-b border-amber-500/20 px-4 py-3">
            <div>
              <h3 className="text-sm font-semibold">待确认正文编辑</h3>
              <p className="mt-1 text-xs leading-5 text-muted-foreground">
                Agent 已提出{" "}
                {proposal.toolName === "append_content" ? "追加" : "替换"}
                ；候选内容尚未写入正式正文。
                {proposal.baseRevisionNo
                  ? ` 基于修订 ${proposal.baseRevisionNo}。`
                  : " 基于空正文。"}
              </p>
            </div>
            <span className="rounded-full bg-amber-500/15 px-2 py-1 text-xs font-medium text-amber-900 dark:text-amber-100">
              {proposalReview?.status === "delegated"
                ? "审核 Agent 处理中"
                : "待确认"}
            </span>
          </div>
          <div className="grid gap-px bg-amber-500/15 md:grid-cols-2">
            <section className="bg-background/85 p-4">
              <h4 className="text-xs font-semibold tracking-[0.12em] text-muted-foreground uppercase">
                当前正式正文
              </h4>
              {snapshot?.body.revisionId && snapshot.body.content ? (
                <CitableText
                  text={snapshot.body.content}
                  disabled={pending !== null}
                  onCite={citeProposalFromDocument}
                  className="mt-2 max-h-80 overflow-auto text-sm leading-7 whitespace-pre-wrap text-foreground"
                />
              ) : (
                <p className="mt-2 text-sm text-muted-foreground">
                  （尚无正式正文）
                </p>
              )}
            </section>
            <section className="bg-amber-500/5 p-4">
              <h4 className="text-xs font-semibold tracking-[0.12em] text-amber-900 uppercase dark:text-amber-100">
                候选正文
              </h4>
              {proposal.candidateContent ? (
                <CitableText
                  text={proposal.candidateContent}
                  disabled={pending !== null}
                  onCite={citeProposalFromCandidate}
                  className="mt-2 max-h-80 overflow-auto text-sm leading-7 whitespace-pre-wrap text-foreground"
                />
              ) : (
                <p className="mt-2 text-sm text-muted-foreground">
                  （候选会清空正文）
                </p>
              )}
            </section>
          </div>
          {proposalReview?.status === "open" && (
            <div className="border-t border-amber-500/20 p-4">
              <CitationList
                citations={proposalCitations}
                currentRevisionId={currentRevisionId}
                disabled={pending !== null}
                onRemove={(index) =>
                  setProposalCitations((current) => ({
                    chapterId,
                    items: (current.chapterId === chapterId
                      ? current.items
                      : []
                    ).filter((_, position) => position !== index),
                  }))
                }
                onError={onError}
              />
              {proposalCitations.length > 0 && (
                <p className="mt-2 text-xs leading-5 text-muted-foreground">
                  {" "}
                  引用会随“拒绝并反馈”或“委托审核”一并提交，审核 Agent
                  只看到引文文本。
                </p>
              )}
              <label className="block text-xs font-medium text-muted-foreground">
                反馈（留空且没有引用即接受；有文字或引用即拒绝并反馈给
                Agent；委托时作为关注点）
                <textarea
                  name="proposal-feedback"
                  autoComplete="off"
                  value={proposalFeedback}
                  onChange={(event) => setProposalFeedback(event.target.value)}
                  maxLength={20_000}
                  disabled={pending !== null}
                  placeholder="例如：保留开场雨景，但减少解释性旁白，让人物动作推动场景。"
                  className="mt-1.5 min-h-24 w-full resize-y rounded-lg border border-input bg-background px-3 py-2 text-sm text-foreground outline-none focus:border-ring focus:ring-3 focus:ring-ring/20 disabled:cursor-not-allowed disabled:opacity-60"
                />
              </label>
              <div className="mt-3 flex flex-wrap gap-2">
                <Button
                  type="button"
                  disabled={pending !== null}
                  onClick={() =>
                    void handleProposalDecision(!hasProposalAttachment)
                  }
                >
                  {pending === "proposalAccept" ||
                  pending === "proposalReject" ? (
                    <LoaderCircle className="animate-spin" aria-hidden="true" />
                  ) : hasProposalAttachment ? (
                    <RotateCcw aria-hidden="true" />
                  ) : (
                    <Check aria-hidden="true" />
                  )}
                  {hasProposalAttachment ? "拒绝并反馈" : "接受编辑"}
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  disabled={pending !== null}
                  onClick={() => void handleProposalDelegate()}
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
          {proposalReview?.status === "delegated" && (
            <div className="flex flex-wrap items-center justify-between gap-3 border-t border-amber-500/20 bg-primary/5 p-4 text-sm text-muted-foreground">
              <span className="flex items-center gap-2">
                <LoaderCircle
                  className="size-4 animate-spin text-primary"
                  aria-hidden="true"
                />
                审核 Agent 正在检查这次编辑提案。
              </span>
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={pending !== null}
                onClick={() => void handleProposalDelegationCancel()}
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

      {stage === "writing_drafting" &&
        !automaticBodyInFlight &&
        preparation.workflow.execution !== "waiting_answer" &&
        !proposal &&
        approvalMode === "auto_approve" && (
          <form className="mt-5" onSubmit={handleSave}>
            <div className="rounded-xl border border-amber-500/20 bg-amber-500/7 px-4 py-3 text-sm leading-6 text-muted-foreground">
              当前没有自动正文请求在运行。这里的人工编辑采用自动接受模式：保存即生成正文修订；上方自由意见会在下一条正文工具结果形成后再投递。
            </div>
            <label className="mt-4 block text-sm font-medium">
              本章正文
              <textarea
                name="chapter-body"
                autoComplete="off"
                value={content}
                onChange={(event) => setContent(event.target.value)}
                maxLength={2_000_000}
                disabled={pending !== null}
                placeholder="从一个具体场景开始，让人物行动、感知和选择推动本章。"
                className="mt-1.5 min-h-96 w-full resize-y rounded-xl border border-input bg-background px-3 py-2 text-sm leading-7 transition outline-none placeholder:text-muted-foreground focus:border-ring focus:ring-3 focus:ring-ring/20 disabled:cursor-not-allowed disabled:opacity-60"
              />
            </label>
            {remoteBodyChanged && (
              <p className="mt-3 rounded-lg border border-amber-500/25 bg-amber-500/8 px-3 py-2 text-xs leading-5 text-amber-950 dark:text-amber-100">
                其他页面已更新已保存正文；你的本地草稿仍被保留。保存时会基于最新版本精确替换，如内容已变化会要求你先刷新并处理冲突。
              </p>
            )}
            <div className="mt-3 flex flex-wrap items-center justify-between gap-3">
              <p className="text-xs leading-5 text-muted-foreground">
                {hasUnsavedBodyChanges
                  ? "存在未保存修改；保存后才能提交整章审核。"
                  : "保存会精确替换当前全文；文本不变时不会制造无意义修订。"}
              </p>
              <div className="flex flex-wrap gap-2">
                <Button
                  type="submit"
                  disabled={
                    pending !== null ||
                    (!snapshot?.body.content && !content.trim())
                  }
                >
                  {pending === "save" ? (
                    <LoaderCircle className="animate-spin" aria-hidden="true" />
                  ) : (
                    <Save aria-hidden="true" />
                  )}
                  保存正文
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  disabled={
                    pending !== null || !content.trim() || hasUnsavedBodyChanges
                  }
                  onClick={() => void handleSubmitFinalReview()}
                >
                  {pending === "submit" ? (
                    <LoaderCircle className="animate-spin" aria-hidden="true" />
                  ) : (
                    <Send aria-hidden="true" />
                  )}
                  提交整章审核
                </Button>
              </div>
            </div>
          </form>
        )}

      {stage === "writing_drafting" &&
        !automaticBodyInFlight &&
        preparation.workflow.execution !== "waiting_answer" &&
        !proposal &&
        approvalMode === "approve_required" && (
          <div className="mt-5 rounded-xl border border-muted-foreground/20 bg-muted/25 px-4 py-4 text-sm leading-6 text-muted-foreground">
            逐次确认模式下，正文只会在 Agent
            提出候选编辑后显示对比和决定操作。当前没有可确认的候选；可等待下一次正文请求，或切换到“自动接受”后使用手工修订路径。
          </div>
        )}

      {stage === "writing_chapter_reviewing" && snapshot?.body.content && (
        <div className="mt-5 overflow-hidden rounded-xl border border-border bg-background/70">
          <div className="flex items-center justify-between gap-3 border-b border-border px-4 py-3">
            <div>
              <h3 className="text-sm font-semibold">待审终稿</h3>
              <p className="mt-0.5 text-xs text-muted-foreground">
                修订 {snapshot.body.revisionNo}
              </p>
            </div>
            <span className="rounded-full bg-muted px-2 py-1 text-xs text-muted-foreground">
              {snapshot.body.review?.status === "delegated"
                ? "审核 Agent 处理中"
                : "待审核"}
            </span>
          </div>
          {snapshot.body.review?.status === "open" ? (
            <CitableText
              text={snapshot.body.content}
              disabled={pending !== null}
              onCite={citeFinalReview}
              className="max-h-[28rem] overflow-auto px-4 py-3 text-sm leading-7 whitespace-pre-wrap text-foreground"
            />
          ) : (
            <pre className="max-h-[28rem] overflow-auto px-4 py-3 text-sm leading-7 whitespace-pre-wrap text-foreground">
              {snapshot.body.content}
            </pre>
          )}
          {snapshot.body.review?.status === "open" && (
            <div className="border-t border-border p-4">
              <CitationList
                citations={finalCitations}
                currentRevisionId={snapshot.body.revisionId}
                disabled={pending !== null}
                onRemove={(index) =>
                  setFinalCitations((current) => ({
                    chapterId,
                    items: (current.chapterId === chapterId
                      ? current.items
                      : []
                    ).filter((_, position) => position !== index),
                  }))
                }
                onError={onError}
              />
              <label className="block text-xs font-medium text-muted-foreground">
                审核意见（退回修订时必填）
                <textarea
                  name="final-review-issues"
                  autoComplete="off"
                  value={issues}
                  onChange={(event) => setIssues(event.target.value)}
                  maxLength={20_000}
                  disabled={pending !== null}
                  placeholder="通过可留空；退回时说明情节、叙事、人物或文本层面的修改要求。"
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
                  通过并定稿
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
          {snapshot.body.review?.status === "delegated" && (
            <div className="flex flex-wrap items-center justify-between gap-3 border-t border-border bg-primary/5 p-4 text-sm text-muted-foreground">
              <span className="flex items-center gap-2">
                <LoaderCircle
                  className="size-4 animate-spin text-primary"
                  aria-hidden="true"
                />
                审核 Agent 正在依据当前终稿审查。
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

      {stage === "writing_chapter_done" && snapshot?.contextFold && (
        <div className="mt-5 rounded-xl border border-emerald-600/20 bg-emerald-600/6 p-4">
          <div className="flex items-start gap-3">
            <Check
              className="mt-0.5 size-5 text-emerald-700"
              aria-hidden="true"
            />
            <div>
              <h3 className="font-medium">终稿已通过并完成正文阶段折叠</h3>
              <p className="mt-1 text-sm leading-6 text-muted-foreground">
                后续章节只会收到这份通过的正文全文；修订过程、审核意见及内部记录仍在本地，但不会回流进
                Agent 上下文。
              </p>
            </div>
          </div>
          <pre className="mt-4 max-h-[28rem] overflow-auto rounded-lg border border-emerald-700/15 bg-background/75 p-3 text-sm leading-7 whitespace-pre-wrap">
            {snapshot.contextFold.plainText}
          </pre>
          <div className="mt-4 flex flex-wrap items-center justify-between gap-3 border-t border-emerald-700/15 pt-4">
            {nextChapterLoading ? (
              <p className="text-sm text-muted-foreground">
                正在核对后续章节计划…
              </p>
            ) : nextChapter ? (
              <>
                <p className="text-sm text-muted-foreground">
                  下一章是第 {nextChapter.orderIndex} 章《{nextChapter.title}
                  》。进入后先填写章节指导，不会自动启动模型。
                </p>
                <Button
                  type="button"
                  disabled={pending !== null}
                  onClick={() => void handleAdvanceToNextChapter()}
                >
                  {pending === "next" ? (
                    <LoaderCircle className="animate-spin" aria-hidden="true" />
                  ) : (
                    <Send aria-hidden="true" />
                  )}
                  进入下一章
                </Button>
              </>
            ) : allMainChaptersApproved ? (
              <>
                <p className="text-sm leading-6 text-muted-foreground">
                  所有已排定正文均已通过终稿审核。确认后会为本稿本写入完本时间；这一操作不会删除章节、修订或本章的纯文本折叠。
                </p>
                <Button
                  type="button"
                  disabled={pending !== null}
                  onClick={() => void handleConfirmCompletion()}
                >
                  {pending === "complete" ? (
                    <LoaderCircle className="animate-spin" aria-hidden="true" />
                  ) : (
                    <Check aria-hidden="true" />
                  )}
                  确认完本
                </Button>
              </>
            ) : (
              <p className="text-sm leading-6 text-muted-foreground">
                章节目录仍存在未完成或状态异常的计划项，因此不能确认完本。请刷新目录后从相应章节继续。
              </p>
            )}
          </div>
        </div>
      )}

      {stage === "completed" && snapshot?.contextFold && (
        <div className="mt-5 rounded-xl border border-emerald-600/20 bg-emerald-600/6 p-4">
          <div className="flex items-start gap-3">
            <Check
              className="mt-0.5 size-5 text-emerald-700"
              aria-hidden="true"
            />
            <div>
              <h3 className="font-medium">已确认完本</h3>
              <p className="mt-1 text-sm leading-6 text-muted-foreground">
                所有计划正文章节已经定稿。最后一章的正文折叠和此前各阶段记录仍保存在本地；未来新增番外会走独立章节流程，不会改写这次完本标记。
              </p>
            </div>
          </div>
        </div>
      )}

      {stage === "completed" && (
        <ExtraChapterPanel
          versionId={versionId}
          preparation={preparation}
          onChanged={onChanged}
          onError={onError}
        />
      )}
    </section>
  )
}
