import { useCallback, useEffect, useRef, useState, type FormEvent } from "react"
import { BookOpenText, LoaderCircle, Play, Save } from "lucide-react"

import { TemplateInserter } from "@/components/TemplatePanel"
import { Button } from "@/components/ui/button"
import { useLatestValue } from "@/hooks/use-latest-value"
import { useWorkspaceDraftPersistence } from "@/hooks/use-workspace-draft"
import {
  getChapterGuidance,
  getChapters,
  getPreparation,
  saveChapterGuidance,
  startChapterOutline,
  type ChapterSummary,
  type PreparationSnapshot,
} from "@/lib/api"

type ChapterReadyPanelProps = {
  versionId: string
  dataDir?: string
  preparation: PreparationSnapshot
  onChanged: (snapshot: PreparationSnapshot) => void
  onError: (message: string) => void
  /** 模板列表变化时递增，使插入器不会停留在旧选项上。 */
  templatesRefreshToken?: number
}

function chapterDisplayLabel(
  chapter: ChapterSummary,
  chapters: ChapterSummary[]
) {
  if (chapter.kind === "main") return `第${chapter.orderIndex}章`

  const extraIndex = chapters
    .filter((item) => item.kind === "extra")
    .findIndex((item) => item.id === chapter.id)
  return `番外 ${extraIndex + 1}`
}

export function ChapterReadyPanel({
  versionId,
  dataDir,
  preparation,
  onChanged,
  onError,
  templatesRefreshToken,
}: ChapterReadyPanelProps) {
  const [chapter, setChapter] = useState<ChapterSummary | null>(null)
  const [chapters, setChapters] = useState<ChapterSummary[]>([])
  const [guidance, setGuidance] = useState("")
  const [guidanceRevisionId, setGuidanceRevisionId] = useState<string | null>(
    null
  )
  const [loading, setLoading] = useState(true)
  const [pending, setPending] = useState<"save" | "start" | null>(null)
  const [remoteGuidanceChanged, setRemoteGuidanceChanged] = useState(false)
  const {
    value: persistedGuidance,
    ref: persistedGuidanceRef,
    commit: commitPersistedGuidance,
  } = useLatestValue("")
  const guidanceRef = useRef("")

  const chapterId = preparation.workflow.activeChapterId
  const chapterIsReady = preparation.workflow.stage === "writing_chapter_ready"

  useEffect(() => {
    guidanceRef.current = guidance
  }, [guidance])

  const { clear: clearGuidanceDraft } = useWorkspaceDraftPersistence({
    dataDir,
    versionId,
    phase: "chapter_guidance",
    targetId: chapterId,
    baseRevision: guidanceRevisionId,
    text: guidance,
    dirty: guidance !== persistedGuidance,
    onRestore: (draft) => {
      if (guidanceRef.current !== persistedGuidanceRef.current) return
      guidanceRef.current = draft.text
      setGuidance(draft.text)
      if (draft.baseRevision !== guidanceRevisionId) {
        setRemoteGuidanceChanged(true)
      }
    },
  })

  // 只读写 ref 与 setter，identity 稳定，读取副作用可以直接把它放进依赖。
  const applyGuidance = useCallback(
    (content: string | null, revisionId: string | null) => {
      const nextContent = content ?? ""
      const hasUnsavedChanges =
        guidanceRef.current !== persistedGuidanceRef.current
      const changedRemotely = nextContent !== persistedGuidanceRef.current
      commitPersistedGuidance(nextContent)
      setGuidanceRevisionId(revisionId)
      if (!hasUnsavedChanges || nextContent === guidanceRef.current) {
        guidanceRef.current = nextContent
        setGuidance(nextContent)
        setRemoteGuidanceChanged(false)
      } else if (changedRemotely) {
        setRemoteGuidanceChanged(true)
      }
    },
    [commitPersistedGuidance, persistedGuidanceRef]
  )

  useEffect(() => {
    let isCurrent = true
    if (!chapterId) {
      // eslint-disable-next-line react-hooks/set-state-in-effect -- 这是「为当前章节重新读取」这一步自己的界面状态：先把上一章的结果清空，面板在无活动章节时整体不渲染。
      setChapter(null)
      setChapters([])
      commitPersistedGuidance("")
      guidanceRef.current = ""
      setGuidance("")
      setGuidanceRevisionId(null)
      setRemoteGuidanceChanged(false)
      setLoading(false)
      return () => {
        isCurrent = false
      }
    }

    setLoading(true)
    Promise.all([getChapters(versionId), getChapterGuidance(chapterId)])
      .then(([chapters, nextGuidance]) => {
        if (!isCurrent) return
        const selectedChapter =
          chapters.find((item) => item.id === chapterId) ?? null
        setChapters(chapters)
        setChapter(selectedChapter)
        applyGuidance(nextGuidance.content, nextGuidance.revisionId)
      })
      .catch((error: unknown) => {
        if (!isCurrent) return
        onError(error instanceof Error ? error.message : "无法读取当前章节")
      })
      .finally(() => {
        if (isCurrent) setLoading(false)
      })

    return () => {
      isCurrent = false
    }
  }, [
    applyGuidance,
    chapterId,
    commitPersistedGuidance,
    onError,
    preparation.workflow.stateRevision,
    versionId,
  ])

  async function refreshPreparation() {
    onChanged(await getPreparation(versionId))
  }

  async function handleSave(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!chapterId) return
    setPending("save")
    try {
      const saved = await saveChapterGuidance(
        chapterId,
        preparation.workflow.stateRevision,
        guidance
      )
      applyGuidance(saved.content, saved.revisionId)
      await clearGuidanceDraft()
      await refreshPreparation()
    } catch (error) {
      onError(error instanceof Error ? error.message : "章节指导未保存")
    } finally {
      setPending(null)
    }
  }

  async function handleStart() {
    if (!chapterId) return
    if (guidance !== persistedGuidanceRef.current) {
      onError("请先保存本章指导，再开始生成细纲")
      return
    }
    setPending("start")
    try {
      await startChapterOutline(chapterId, preparation.workflow.stateRevision)
      await refreshPreparation()
    } catch (error) {
      onError(error instanceof Error ? error.message : "细纲任务未能启动")
    } finally {
      setPending(null)
    }
  }

  return (
    <section className="mt-6 rounded-2xl border border-border bg-background/75 p-5 shadow-sm">
      <div className="flex items-start gap-3">
        <BookOpenText
          className="mt-0.5 size-5 text-primary"
          aria-hidden="true"
        />
        <div>
          <p className="text-xs font-semibold tracking-[0.14em] text-muted-foreground uppercase">
            章节指导
          </p>
          <h2 className="mt-1 text-xl font-semibold">
            {loading
              ? "正在读取章节…"
              : chapter
                ? `${chapterDisplayLabel(chapter, chapters)} · ${chapter.title}`
                : "当前章节不可用"}
          </h2>
          <p className="mt-1 text-sm leading-6 text-muted-foreground">
            指导可以留空；保存后会成为本章细纲 Run
            的固定输入快照，不能在运行中被悄悄替换。
          </p>
        </div>
      </div>

      {chapter && (
        <p className="mt-4 rounded-xl border border-primary/15 bg-primary/5 px-4 py-3 text-sm leading-6 text-muted-foreground">
          <span className="font-medium text-foreground">
            {chapter.kind === "extra" ? "新增番外：" : "计划章节："}
          </span>{" "}
          {chapter.title}
          。细纲任务启动后会进入本地队列，并在取得模型许可时封存最新设置与已审核的阶段上下文。
        </p>
      )}

      {chapterIsReady ? (
        <form className="mt-4" onSubmit={handleSave}>
          <label className="block text-sm font-medium">
            本章指导
            <textarea
              name="chapter-guidance"
              autoComplete="off"
              value={guidance}
              onChange={(event) => setGuidance(event.target.value)}
              maxLength={20_000}
              placeholder="可写本章想突出的情节、节奏、人物情绪、必须避免的内容；留空也可以开始。"
              disabled={!chapterId || pending !== null}
              className="mt-1.5 min-h-32 w-full resize-y rounded-xl border border-input bg-background px-3 py-2 text-sm leading-6 transition outline-none placeholder:text-muted-foreground focus:border-ring focus:ring-3 focus:ring-ring/20 disabled:cursor-not-allowed disabled:opacity-60"
            />
          </label>
          {remoteGuidanceChanged && (
            <p className="mt-2 text-xs leading-5 text-amber-700 dark:text-amber-300">
              其他页面已更新已保存指导；你的本地草稿仍被保留。保存前请确认它仍适用于最新上下文。
            </p>
          )}
          <div className="mt-2">
            <TemplateInserter
              label="插入模板到本章指导"
              refreshToken={templatesRefreshToken}
              onInsert={(text) =>
                setGuidance((current) =>
                  current.trim().length > 0 ? `${current}\n${text}` : text
                )
              }
            />
          </div>
          <div className="mt-3 flex flex-wrap gap-2">
            <Button type="submit" disabled={!chapterId || pending !== null}>
              {pending === "save" ? (
                <LoaderCircle className="animate-spin" aria-hidden="true" />
              ) : (
                <Save aria-hidden="true" />
              )}
              保存指导
            </Button>
            <Button
              type="button"
              variant="outline"
              disabled={!chapterId || pending !== null}
              onClick={() => void handleStart()}
            >
              {pending === "start" ? (
                <LoaderCircle className="animate-spin" aria-hidden="true" />
              ) : (
                <Play aria-hidden="true" />
              )}
              开始细纲
            </Button>
          </div>
        </form>
      ) : (
        <div className="mt-4 rounded-xl border border-amber-500/20 bg-amber-500/7 px-4 py-3 text-sm leading-6 text-muted-foreground">
          <LoaderCircle
            className="mr-2 inline size-4 animate-spin text-amber-700"
            aria-hidden="true"
          />
          细纲任务已进入本地队列。模型在取得全局许可后会开始请求；若请求失败，界面会保留这份指导并开放手动细纲提交。
        </div>
      )}
    </section>
  )
}
