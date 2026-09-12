import { useState, type FormEvent } from "react"
import {
  Check,
  FileText,
  LoaderCircle,
  Play,
  RotateCcw,
  Sparkles,
} from "lucide-react"

import { Button } from "@/components/ui/button"
import { CitationList } from "@/components/CitationList"
import { CitableText } from "@/components/CitableText"
import {
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
  FieldLegend,
  FieldSet,
  FieldTitle,
} from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { UserQuestionPanel } from "@/components/UserQuestionPanel"
import { WritingRecoveryPanel } from "@/components/WritingRecoveryPanel"
import { type PickedTextRange } from "@/lib/citations"
import { usePrimaryAction } from "@/lib/shortcuts"
import {
  cancelReviewDelegation,
  decidePreparationReview,
  delegateReview,
  getPreparation,
  saveNovelBrief,
  startPreparation,
  submitPreparationArtifacts,
  type CurrentPreparationArtifact,
  type FeedbackCitation,
  type PreparationArtifactKind,
  type PreparationReview,
  type PreparationSnapshot,
} from "@/lib/api"

const artifactMeta: Record<
  PreparationArtifactKind,
  { title: string; hint: string }
> = {
  background: {
    title: "背景设定",
    hint: "时代、地点、规则与故事氛围。",
  },
  research: {
    title: "资料收集",
    hint: "已知资料、来源、未核验信息与待补充项。",
  },
  characters: {
    title: "人物设定",
    hint: "人物动机、关系、秘密和变化弧线。",
  },
  outline: {
    title: "小说粗纲",
    hint: "完整故事走向；后续会在这里校验章节计划。",
  },
}

const artifactOrder: PreparationArtifactKind[] = [
  "background",
  "research",
  "characters",
  "outline",
]

function statusLabel(status: PreparationReview["status"]) {
  return {
    open: "待审核",
    delegated: "审核 Agent 处理中",
    approved: "已通过",
    rejected: "需修订",
    stale: "依赖已变化",
  }[status]
}

type EditableChapterPlanItem = {
  title: string
  goal: string
}

function ChapterPlanEditor({
  initialItems,
}: {
  initialItems: EditableChapterPlanItem[]
}) {
  const [items, setItems] = useState<EditableChapterPlanItem[]>(() =>
    initialItems.length > 0 ? initialItems : [{ title: "第一章", goal: "" }]
  )

  function updateItem(
    index: number,
    field: keyof EditableChapterPlanItem,
    value: string
  ) {
    setItems((current) =>
      current.map((item, itemIndex) =>
        itemIndex === index ? { ...item, [field]: value } : item
      )
    )
  }

  return (
    <FieldSet className="mt-4 rounded-xl border border-primary/20 bg-primary/5 p-4">
      <FieldLegend variant="label">章节计划</FieldLegend>
      <FieldDescription>
        每一项都会按这里的顺序冻结为对应章节；准备审核全部通过后，可以逐章进入细纲和正文流程。
      </FieldDescription>
      <FieldGroup className="gap-3">
        {items.map((item, index) => (
          <FieldSet
            key={`chapter-plan-${index}`}
            className="rounded-lg border border-border bg-background/70 p-3"
          >
            <div className="flex items-center justify-between gap-3">
              <FieldTitle>第 {index + 1} 章</FieldTitle>
              {items.length > 1 && (
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  onClick={() =>
                    setItems((current) =>
                      current.filter((_, itemIndex) => itemIndex !== index)
                    )
                  }
                >
                  删除
                </Button>
              )}
            </div>
            <FieldGroup className="mt-3 grid gap-3 sm:grid-cols-2">
              <Field>
                <FieldLabel htmlFor={`chapter-plan-title-${index}`}>
                  章节标题
                </FieldLabel>
                <Input
                  id={`chapter-plan-title-${index}`}
                  name="chapterPlanTitle"
                  required
                  maxLength={200}
                  value={item.title}
                  onChange={(event) =>
                    updateItem(index, "title", event.target.value)
                  }
                />
              </Field>
              <Field>
                <FieldLabel htmlFor={`chapter-plan-goal-${index}`}>
                  本章目标
                </FieldLabel>
                <textarea
                  id={`chapter-plan-goal-${index}`}
                  name="chapterPlanGoal"
                  required
                  maxLength={20_000}
                  value={item.goal}
                  onChange={(event) =>
                    updateItem(index, "goal", event.target.value)
                  }
                  placeholder="这一章需要完成的情节、人物推进与读者感受。"
                  className="mt-1.5 min-h-24 w-full resize-y rounded-lg border border-input bg-background px-3 py-2 text-sm leading-6 transition outline-none placeholder:text-muted-foreground focus:border-ring focus:ring-3 focus:ring-ring/20"
                />
              </Field>
            </FieldGroup>
          </FieldSet>
        ))}
      </FieldGroup>
      <Button
        type="button"
        variant="outline"
        size="sm"
        className="mt-3"
        disabled={items.length >= 500}
        onClick={() =>
          setItems((current) => [
            ...current,
            { title: `第${current.length + 1}章`, goal: "" },
          ])
        }
      >
        添加章节
      </Button>
      {items.length >= 500 && (
        <FieldDescription className="mt-2">
          章节计划最多包含 500 项。
        </FieldDescription>
      )}
    </FieldSet>
  )
}

type PreparationPanelProps = {
  versionId: string
  preparation: PreparationSnapshot | null
  loading: boolean
  onChanged: (snapshot: PreparationSnapshot) => void
  onError: (message: string) => void
}

export function PreparationPanel({
  versionId,
  preparation,
  loading,
  onChanged,
  onError,
}: PreparationPanelProps) {
  const [pendingAction, setPendingAction] = useState<string | null>(null)
  const [citationDrafts, setCitationDrafts] = useState<
    Record<string, FeedbackCitation[]>
  >({})

  /**
   * 「开始准备」是准备阶段唯一的主操作：光标不在输入框里时 Ctrl/Cmd+Enter 也能触发。
   * 「保存目标」在表单内，走通用的表单提交路径，两者不会互相抢占同一次按键。
   */
  usePrimaryAction("preparation-start", {
    label: "开始准备",
    run: () => {
      if (!preparation) return
      void runAction("start", () =>
        startPreparation(versionId, preparation.workflow.stateRevision)
      )
    },
    enabled:
      preparation !== null &&
      preparation.workflow.stage === "initializing" &&
      Boolean(preparation.novelBrief.content?.trim()) &&
      pendingAction === null,
  })

  async function runAction(
    action: string,
    operation: () => Promise<PreparationSnapshot>,
    onSuccess?: () => void
  ) {
    setPendingAction(action)
    try {
      onChanged(await operation())
      onSuccess?.()
    } catch (error) {
      onError(error instanceof Error ? error.message : "准备阶段操作未完成")
    } finally {
      setPendingAction(null)
    }
  }

  /** 把产物中的选中文字固定为一条指向当前修订的引用。 */
  function citeArtifact(
    artifact: CurrentPreparationArtifact,
    reviewId: string,
    range: PickedTextRange
  ) {
    if (!artifact.revisionId) {
      onError("当前产物还没有可引用的修订。")
      return
    }
    const citation: FeedbackCitation = {
      kind: "document",
      documentId: artifact.documentId,
      revisionId: artifact.revisionId,
      chapterId: null,
      startChar: String(range.startChar),
      endChar: String(range.endChar),
      quote: range.quote,
    }
    setCitationDrafts((current) => ({
      ...current,
      [reviewId]: [...(current[reviewId] ?? []), citation],
    }))
  }

  function removeCitation(reviewId: string, index: number) {
    setCitationDrafts((current) => ({
      ...current,
      [reviewId]: (current[reviewId] ?? []).filter(
        (_, position) => position !== index
      ),
    }))
  }

  async function handleSaveBrief(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const form = new FormData(event.currentTarget)
    const content = String(form.get("brief") ?? "")
    setPendingAction("brief")
    try {
      await saveNovelBrief(versionId, content)
      onChanged(await getPreparation(versionId))
    } catch (error) {
      onError(error instanceof Error ? error.message : "小说目标未保存")
    } finally {
      setPendingAction(null)
    }
  }

  function handleSubmitArtifacts(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!preparation) return
    const form = new FormData(event.currentTarget)
    const editableKinds = editableArtifactKinds(preparation)
    const chapterPlan = editableKinds.includes("outline")
      ? form.getAll("chapterPlanTitle").map((title, index) => ({
          title: String(title),
          goal: String(form.getAll("chapterPlanGoal")[index] ?? ""),
        }))
      : undefined
    void runAction("artifacts", () =>
      submitPreparationArtifacts(
        versionId,
        preparation.workflow.stateRevision,
        editableKinds.map((kind) => ({
          kind,
          content: String(form.get(kind) ?? ""),
        })),
        chapterPlan
      )
    )
  }

  function handleReview(
    form: HTMLFormElement,
    review: PreparationReview,
    passed: boolean
  ) {
    if (!preparation) return
    const issues = String(new FormData(form).get("issues") ?? "")
    void runAction(
      `review-${review.id}`,
      () =>
        decidePreparationReview(
          review.id,
          preparation.workflow.stateRevision,
          passed,
          issues,
          citationDrafts[review.id] ?? []
        ),
      () => setCitationDrafts((current) => ({ ...current, [review.id]: [] }))
    )
  }

  async function handleDelegate(
    form: HTMLFormElement,
    review: PreparationReview
  ) {
    if (!preparation) return
    const notes = String(new FormData(form).get("issues") ?? "")
    setPendingAction(`delegate-${review.id}`)
    try {
      const result = await delegateReview(
        review.id,
        preparation.workflow.stateRevision,
        notes,
        citationDrafts[review.id] ?? []
      )
      onChanged(result.snapshot)
      setCitationDrafts((current) => ({ ...current, [review.id]: [] }))
    } catch (error) {
      onError(error instanceof Error ? error.message : "审核委托未能启动")
    } finally {
      setPendingAction(null)
    }
  }

  async function handleCancelDelegation(review: PreparationReview) {
    if (!preparation) return
    setPendingAction(`cancel-delegation-${review.id}`)
    try {
      const result = await cancelReviewDelegation(
        review.id,
        preparation.workflow.stateRevision
      )
      onChanged(result.snapshot)
    } catch (error) {
      onError(error instanceof Error ? error.message : "审核委托未能取消")
    } finally {
      setPendingAction(null)
    }
  }

  if (loading || !preparation) {
    return (
      <section className="mt-6 flex min-h-48 items-center justify-center rounded-2xl border border-dashed border-border bg-background/60 text-sm text-muted-foreground">
        <LoaderCircle className="mr-2 size-4 animate-spin" aria-hidden="true" />
        正在读取准备阶段…
      </section>
    )
  }

  const { workflow } = preparation
  const briefReady = Boolean(preparation.novelBrief.content?.trim())
  const editableKinds = editableArtifactKinds(preparation)
  const automaticPreparationInFlight =
    workflow.stage === "preparing_generating" &&
    (workflow.execution === "queued" ||
      workflow.execution === "request_in_flight")

  return (
    <section className="mt-6 rounded-2xl border border-border bg-background/75 p-5 shadow-sm">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <p className="text-xs font-semibold tracking-[0.14em] text-muted-foreground uppercase">
            准备阶段
          </p>
          <h2 className="mt-1 text-xl font-semibold">
            让作品先拥有可靠的创作基础
          </h2>
          <p className="mt-1 max-w-2xl text-sm leading-6 text-muted-foreground">
            背景、资料、人物和粗纲会独立审核。全部当前修订通过后，过程消息将折叠为四项产物的完整纯文本，原始记录仍保留在本地。
          </p>
        </div>
        <span className="w-fit rounded-full bg-muted px-3 py-1 text-sm text-muted-foreground">
          状态版本 {workflow.stateRevision}
        </span>
      </div>

      {workflow.stage === "initializing" && (
        <div className="mt-5 grid gap-5 lg:grid-cols-[minmax(0,1fr)_14rem]">
          <form
            key={preparation.novelBrief.revisionId ?? "empty"}
            onSubmit={handleSaveBrief}
          >
            <label className="block text-sm font-medium">
              小说目标
              <textarea
                name="brief"
                required
                maxLength={20_000}
                defaultValue={preparation.novelBrief.content ?? ""}
                placeholder="你希望这部小说讲述什么？可以写主题、读者感受、人物关系或必须保留的元素。"
                className="mt-1.5 min-h-32 w-full resize-y rounded-xl border border-input bg-background px-3 py-2 text-sm leading-6 transition outline-none placeholder:text-muted-foreground focus:border-ring focus:ring-3 focus:ring-ring/20"
              />
            </label>
            <div className="mt-3 flex flex-wrap gap-2">
              <Button type="submit" disabled={pendingAction !== null}>
                {pendingAction === "brief" && (
                  <LoaderCircle className="animate-spin" aria-hidden="true" />
                )}
                保存目标
              </Button>
              {briefReady && (
                <Button
                  type="button"
                  variant="outline"
                  disabled={pendingAction !== null}
                  onClick={() =>
                    void runAction("start", () =>
                      startPreparation(versionId, workflow.stateRevision)
                    )
                  }
                >
                  {pendingAction === "start" ? (
                    <LoaderCircle className="animate-spin" aria-hidden="true" />
                  ) : (
                    <Play aria-hidden="true" />
                  )}
                  开始准备
                </Button>
              )}
            </div>
          </form>
          <aside className="rounded-xl border border-primary/15 bg-primary/5 p-4 text-sm leading-6 text-muted-foreground">
            <Sparkles className="mb-2 size-4 text-primary" aria-hidden="true" />
            目标会作为准备阶段的明确输入保存，不会被后续审核或折叠悄悄改写。
          </aside>
        </div>
      )}

      {automaticPreparationInFlight && (
        <div className="mt-5 flex items-start gap-3 rounded-xl border border-primary/20 bg-primary/5 px-4 py-3 text-sm leading-6 text-muted-foreground">
          <LoaderCircle
            className="mt-0.5 size-4 shrink-0 animate-spin text-primary"
            aria-hidden="true"
          />
          <div>
            <p className="font-medium text-foreground">
              模型正在准备四项创作基础
            </p>
            <p className="mt-1">
              请求已安全封存；完成后会一次性提交背景、资料、人物、粗纲与章节计划，并进入人工审核。
            </p>
          </div>
        </div>
      )}

      <WritingRecoveryPanel
        workflow={workflow}
        disabled={pendingAction !== null}
        onContinued={async () => onChanged(await getPreparation(versionId))}
        onError={onError}
      />

      {preparation.pendingQuestion && (
        <UserQuestionPanel
          question={preparation.pendingQuestion}
          stateRevision={workflow.stateRevision}
          disabled={pendingAction !== null}
          onAnswered={async () => onChanged(await getPreparation(versionId))}
          onError={onError}
        />
      )}

      {(workflow.stage === "preparing_generating" ||
        workflow.stage === "preparing_revising") &&
        !automaticPreparationInFlight &&
        workflow.execution !== "waiting_answer" && (
          <form
            key={artifactFormKey(preparation)}
            className="mt-5"
            onSubmit={handleSubmitArtifacts}
          >
            <div className="mb-4 rounded-xl border border-amber-500/20 bg-amber-500/7 px-4 py-3 text-sm leading-6 text-muted-foreground">
              <FileText
                className="mr-2 inline size-4 text-amber-700"
                aria-hidden="true"
              />
              {workflow.stage === "preparing_revising"
                ? "请只修订被退回或因依赖变化失效的产物；已通过项不会被覆盖。"
                : "模型请求未启动或已失败时，可在本地手动提交四项产物完成审核闭环。"}
            </div>
            <div className="grid gap-4 lg:grid-cols-2">
              {editableKinds.map((kind) => {
                const existing = preparation.artifacts.find(
                  (artifact) => artifact.kind === kind
                )
                return (
                  <label
                    key={kind}
                    className="block rounded-xl border border-border bg-background/70 p-4 text-sm font-medium"
                  >
                    <span className="flex items-center justify-between gap-3">
                      {artifactMeta[kind].title}
                      {existing?.review && (
                        <span className="rounded-full bg-muted px-2 py-0.5 text-xs font-normal text-muted-foreground">
                          {statusLabel(existing.review.status)}
                        </span>
                      )}
                    </span>
                    <span className="mt-1 block text-xs leading-5 font-normal text-muted-foreground">
                      {artifactMeta[kind].hint}
                    </span>
                    <textarea
                      name={kind}
                      required
                      maxLength={2_000_000}
                      defaultValue={existing?.content ?? ""}
                      className="mt-3 min-h-44 w-full resize-y rounded-lg border border-input bg-background px-3 py-2 leading-6 font-normal transition outline-none placeholder:text-muted-foreground focus:border-ring focus:ring-3 focus:ring-ring/20"
                    />
                  </label>
                )
              })}
            </div>
            {editableKinds.includes("outline") && (
              <ChapterPlanEditor
                initialItems={
                  preparation.artifacts
                    .find((artifact) => artifact.kind === "outline")
                    ?.chapterPlan?.items.map((item) => ({
                      title: item.title,
                      goal: item.goal,
                    })) ?? []
                }
              />
            )}
            <div className="mt-4 flex items-center justify-between gap-3">
              <p className="text-xs leading-5 text-muted-foreground">
                {workflow.stage === "preparing_generating"
                  ? "首次提交必须同时包含四项产物。"
                  : "只提交被拒绝或因依赖变化而失效的产物；已通过项不会被覆盖。"}
              </p>
              <Button
                type="submit"
                disabled={pendingAction !== null || editableKinds.length === 0}
              >
                {pendingAction === "artifacts" && (
                  <LoaderCircle className="animate-spin" aria-hidden="true" />
                )}
                提交审核
              </Button>
            </div>
          </form>
        )}

      {workflow.stage === "preparing_reviewing" && (
        <div className="mt-5 grid gap-4 lg:grid-cols-2">
          {artifactOrder.map((kind) => {
            const artifact = preparation.artifacts.find(
              (item) => item.kind === kind
            )
            const review = artifact?.review
            if (!artifact || !review) return null
            const actionKey = `review-${review.id}`
            return (
              <article
                key={kind}
                className="overflow-hidden rounded-xl border border-border bg-background/70"
              >
                <div className="flex items-center justify-between gap-3 border-b border-border px-4 py-3">
                  <div>
                    <h3 className="text-sm font-semibold">
                      {artifactMeta[kind].title}
                    </h3>
                    <p className="mt-0.5 text-xs text-muted-foreground">
                      修订 {artifact.revisionNo}
                    </p>
                  </div>
                  <span className="rounded-full bg-muted px-2 py-1 text-xs text-muted-foreground">
                    {statusLabel(review.status)}
                  </span>
                </div>
                {review.status === "open" ? (
                  <CitableText
                    text={artifact.content ?? ""}
                    disabled={pendingAction !== null}
                    onCite={(range) => citeArtifact(artifact, review.id, range)}
                    className="max-h-64 overflow-auto px-4 py-3 text-sm leading-6 whitespace-pre-wrap text-foreground"
                  />
                ) : (
                  <pre className="max-h-64 overflow-auto px-4 py-3 text-sm leading-6 whitespace-pre-wrap text-foreground">
                    {artifact.content}
                  </pre>
                )}
                {kind === "outline" && artifact.chapterPlan && (
                  <div className="border-t border-border bg-muted/35 px-4 py-3">
                    <p className="text-xs font-semibold tracking-[0.12em] text-muted-foreground uppercase">
                      章节计划
                    </p>
                    <div className="mt-2 space-y-3 text-sm leading-6">
                      {artifact.chapterPlan.items.map((item) => (
                        <div key={item.planItemId}>
                          <p className="font-medium">
                            第{item.orderIndex}章：{item.title}
                          </p>
                          <p className="text-muted-foreground">{item.goal}</p>
                        </div>
                      ))}
                    </div>
                  </div>
                )}
                {review.status === "open" && (
                  <form className="border-t border-border p-4">
                    <CitationList
                      citations={citationDrafts[review.id] ?? []}
                      currentRevisionId={artifact.revisionId}
                      disabled={pendingAction !== null}
                      onRemove={(index) => removeCitation(review.id, index)}
                      onError={onError}
                    />
                    <label className="block text-xs font-medium text-muted-foreground">
                      审核意见（退回修订时必填）
                      <textarea
                        name="issues"
                        maxLength={20_000}
                        className="mt-1.5 min-h-20 w-full resize-y rounded-lg border border-input bg-background px-3 py-2 text-sm text-foreground outline-none focus:border-ring focus:ring-3 focus:ring-ring/20"
                        placeholder="通过可留空；退回时说明需要调整的内容。"
                      />
                    </label>
                    <div className="mt-3 flex flex-wrap gap-2">
                      <Button
                        type="button"
                        disabled={pendingAction !== null}
                        onClick={(event) =>
                          handleReview(event.currentTarget.form!, review, true)
                        }
                      >
                        {pendingAction === actionKey ? (
                          <LoaderCircle
                            className="animate-spin"
                            aria-hidden="true"
                          />
                        ) : (
                          <Check aria-hidden="true" />
                        )}
                        通过
                      </Button>
                      <Button
                        type="button"
                        variant="outline"
                        disabled={pendingAction !== null}
                        onClick={(event) =>
                          handleReview(event.currentTarget.form!, review, false)
                        }
                      >
                        <RotateCcw aria-hidden="true" />
                        退回修订
                      </Button>
                      <span className="self-center text-xs leading-5 text-muted-foreground">
                        选中产物文字后点“引用选中文字”可把原句随意见一并提交。
                      </span>
                      <Button
                        type="button"
                        variant="outline"
                        disabled={pendingAction !== null}
                        onClick={(event) =>
                          handleDelegate(event.currentTarget.form!, review)
                        }
                      >
                        {pendingAction === `delegate-${review.id}` ? (
                          <LoaderCircle
                            className="animate-spin"
                            aria-hidden="true"
                          />
                        ) : (
                          <Sparkles aria-hidden="true" />
                        )}
                        委托审核
                      </Button>
                    </div>
                  </form>
                )}
                {review.status === "delegated" && (
                  <div className="flex flex-wrap items-center justify-between gap-3 border-t border-border bg-primary/5 p-4 text-sm text-muted-foreground">
                    <span className="flex items-center gap-2">
                      <LoaderCircle
                        className="size-4 animate-spin text-primary"
                        aria-hidden="true"
                      />
                      审核 Agent 正在依据当前修订审查。
                    </span>
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      disabled={pendingAction !== null}
                      onClick={() => void handleCancelDelegation(review)}
                    >
                      {pendingAction === `cancel-delegation-${review.id}` && (
                        <LoaderCircle
                          className="animate-spin"
                          aria-hidden="true"
                        />
                      )}
                      取消并自行审核
                    </Button>
                  </div>
                )}
              </article>
            )
          })}
        </div>
      )}

      {workflow.stage === "writing_chapter_ready" &&
        preparation.contextFold && (
          <div className="mt-5 rounded-xl border border-emerald-600/20 bg-emerald-600/6 p-4">
            <div className="flex items-start gap-3">
              <Check
                className="mt-0.5 size-5 text-emerald-700"
                aria-hidden="true"
              />
              <div>
                <h3 className="font-medium">准备阶段已通过并完成上下文折叠</h3>
                <p className="mt-1 text-sm leading-6 text-muted-foreground">
                  后续 Agent
                  只会接收以下四项完整产物文本；审核意见、运行消息与元数据没有混入该块。
                </p>
              </div>
            </div>
            <pre className="mt-4 max-h-72 overflow-auto rounded-lg border border-emerald-700/15 bg-background/75 p-3 text-sm leading-6 whitespace-pre-wrap">
              {preparation.contextFold.plainText}
            </pre>
          </div>
        )}
    </section>
  )
}

function editableArtifactKinds(
  preparation: PreparationSnapshot
): PreparationArtifactKind[] {
  if (preparation.workflow.stage === "preparing_generating")
    return artifactOrder
  if (preparation.workflow.stage !== "preparing_revising") return []
  return artifactOrder.filter((kind) => {
    const status = preparation.artifacts.find(
      (artifact) => artifact.kind === kind
    )?.review?.status
    return status === "rejected" || status === "stale"
  })
}

function artifactFormKey(preparation: PreparationSnapshot) {
  return preparation.artifacts
    .map(
      (artifact) =>
        `${artifact.kind}-${artifact.revisionId ?? "empty"}-${artifact.review?.status ?? "none"}`
    )
    .join("|")
}
