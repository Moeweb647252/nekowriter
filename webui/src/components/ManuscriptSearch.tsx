import { useState, type FormEvent } from "react"
import { LoaderCircle, Search } from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  searchManuscript,
  type ChapterSummary,
  type ManuscriptMatch,
} from "@/lib/api"
import {
  SEARCH_FOCUS_KEY,
  useEscapeDismiss,
  useFocusTarget,
} from "@/lib/shortcuts"

interface ManuscriptSearchProps {
  versionId: string
  /** 目录索引，用于把命中显示成章节名。 */
  chapters: ChapterSummary[]
  /** 选中一条命中：阅读区需要滚动到该章并高亮该范围。 */
  onLocate: (match: ManuscriptMatch) => void
  onError: (message: string) => void
}

type SearchState =
  | { kind: "idle" }
  | { kind: "searching"; query: string }
  | { kind: "done"; query: string; matches: ManuscriptMatch[] }

/**
 * 整稿查找。
 *
 * 阅读区是窗口化的，浏览器自带的页内查找只能看到已挂载的几章；要在一部几百章的
 * 作品里可靠地找到一处文字，就得由服务端读遍全部已提交正文。命中带偏移，因此
 * 定位到章节后可以直接把那一句高亮出来。
 */
export function ManuscriptSearch({
  versionId,
  chapters,
  onLocate,
  onError,
}: ManuscriptSearchProps) {
  const [query, setQuery] = useState("")
  const [state, setState] = useState<SearchState>({ kind: "idle" })
  // Ctrl/Cmd+K 直接落到查找框；这是唯一注册的查找焦点目标。
  const searchInputRef = useFocusTarget(SEARCH_FOCUS_KEY)
  // 结果面板是可安全关闭的层：Esc 只收起它，不会跳转或清空已输入的关键词。
  useEscapeDismiss(state.kind === "done", () => setState({ kind: "idle" }))

  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const trimmed = query.trim()
    if (!trimmed || state.kind === "searching") return
    setState({ kind: "searching", query: trimmed })
    searchManuscript(versionId, trimmed)
      .then((matches) => setState({ kind: "done", query: trimmed, matches }))
      .catch((error: unknown) => {
        setState({ kind: "idle" })
        onError(error instanceof Error ? error.message : "查找失败")
      })
  }

  const searching = state.kind === "searching"

  return (
    <div className="space-y-2">
      <form className="flex items-center gap-2" onSubmit={handleSubmit}>
        <label className="sr-only" htmlFor="manuscript-search">
          在全部已提交正文里查找
        </label>
        <input
          ref={searchInputRef}
          id="manuscript-search"
          value={query}
          maxLength={80}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="在全部已提交正文里查找…"
          className="h-9 min-w-0 flex-1 rounded-xl border border-input bg-background px-3 text-sm transition outline-none placeholder:text-muted-foreground focus:border-ring focus:ring-3 focus:ring-ring/20"
        />
        <Button size="sm" type="submit" disabled={searching || !query.trim()}>
          {searching ? (
            <LoaderCircle className="animate-spin" aria-hidden="true" />
          ) : (
            <Search aria-hidden="true" />
          )}
          查找
        </Button>
      </form>

      {state.kind === "done" && (
        <div className="rounded-xl border border-border bg-muted/20 p-2">
          <p className="px-1 pb-1 text-xs text-muted-foreground">
            {state.matches.length === 0
              ? `没有找到「${state.query}」。`
              : `找到 ${state.matches.length} 处（最多显示 50 处）。`}
          </p>
          {state.matches.length > 0 && (
            <ul className="max-h-56 space-y-1 overflow-y-auto">
              {state.matches.map((match) => {
                const chapter = chapters.find(
                  (item) => item.id === match.chapterId
                )
                return (
                  <li key={`${match.chapterId}-${match.startChar}`}>
                    <button
                      type="button"
                      onClick={() => onLocate(match)}
                      className="w-full rounded-lg px-2 py-1.5 text-left text-sm transition hover:bg-background focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
                    >
                      <span className="font-medium">
                        {chapter?.label ?? "未知章节"}
                      </span>
                      <span className="mt-0.5 block truncate text-xs text-muted-foreground">
                        {match.snippet}
                      </span>
                    </button>
                  </li>
                )
              })}
            </ul>
          )}
        </div>
      )}
    </div>
  )
}
