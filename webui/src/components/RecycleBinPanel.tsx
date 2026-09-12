import { useCallback, useEffect, useState } from "react"
import {
  CircleAlert,
  LoaderCircle,
  RefreshCw,
  RotateCcw,
  Trash2,
  Trash,
} from "lucide-react"

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
  ApiError,
  getRecycleBin,
  purgeRecycleItem,
  restoreRecycleItem,
  type RecycleBin,
  type RecycleItem,
  type RecycleItemKind,
} from "@/lib/api"

const kindLabels: Record<RecycleItemKind, string> = {
  project: "项目",
  version: "稿本",
  chapter: "章节",
  prompt_template: "提示词模板",
  knowledge_entry: "知识条目",
  knowledge_space: "知识 Space",
}

function errorText(error: unknown, fallback: string) {
  return error instanceof ApiError || error instanceof Error
    ? error.message
    : fallback
}

function formatDeletedAt(timestampMs: string) {
  return new Intl.DateTimeFormat("zh-CN", {
    dateStyle: "short",
    timeStyle: "short",
  }).format(Number(timestampMs))
}

/** 父对象用「项目《雨夜计划》」这样的可读路径表示；没有父对象时不显示。 */
function parentText(item: RecycleItem) {
  if (!item.parent) return null
  return `${item.parent.kind}《${item.parent.title}》`
}

interface RecycleBinPanelProps {
  /** 由事件同步递增：回收站的内容会随删除、恢复与彻底删除变化。 */
  refreshToken?: number
  /** 恢复或彻底删除成功后调用，使目录、项目列表与概览跟上。 */
  onChanged?: () => void
  onError: (message: string) => void
}

/**
 * 回收站。
 *
 * 这里只呈现服务端算好的结论：可恢复/可彻底删除与它们被拒绝的原因都由 Store
 * 判定，界面不重新推断一遍。「彻底删除」需要二次确认，且只有服务端标记为可
 * 物理删除的类别才有这个按钮——稿本与正文为保留审计链而只留在回收站里。
 */
export function RecycleBinPanel({
  refreshToken,
  onChanged,
  onError,
}: RecycleBinPanelProps) {
  const [bin, setBin] = useState<RecycleBin | null>(null)
  const [pending, setPending] = useState<string | null>(null)
  const [confirmingPurge, setConfirmingPurge] = useState<string | null>(null)
  const [notice, setNotice] = useState<string | null>(null)

  const load = useCallback(() => getRecycleBin(), [])

  useEffect(() => {
    let isCurrent = true
    load()
      .then((next) => {
        if (isCurrent) setBin(next)
      })
      .catch((error: unknown) => {
        if (isCurrent) onError(errorText(error, "无法读取回收站"))
      })
    return () => {
      isCurrent = false
    }
  }, [load, onError, refreshToken])

  async function handleRestore(item: RecycleItem) {
    const key = `${item.kind}:${item.id}`
    setPending(key)
    setNotice(null)
    try {
      setBin(await restoreRecycleItem(item.kind, item.id))
      setNotice(`已恢复${kindLabels[item.kind]}「${item.title}」`)
      onChanged?.()
    } catch (error) {
      setNotice(errorText(error, "恢复失败"))
    } finally {
      setPending(null)
    }
  }

  async function handlePurge(item: RecycleItem) {
    const key = `${item.kind}:${item.id}`
    if (confirmingPurge !== key) {
      setConfirmingPurge(key)
      return
    }
    setConfirmingPurge(null)
    setPending(key)
    setNotice(null)
    try {
      setBin(await purgeRecycleItem(item.kind, item.id))
      setNotice(
        `已彻底删除${kindLabels[item.kind]}「${item.title}」，无法再恢复`
      )
      onChanged?.()
    } catch (error) {
      setNotice(errorText(error, "彻底删除失败"))
    } finally {
      setPending(null)
    }
  }

  async function handleRefresh() {
    setPending("refresh")
    setNotice(null)
    try {
      setBin(await load())
    } catch (error) {
      setNotice(errorText(error, "无法读取回收站"))
    } finally {
      setPending(null)
    }
  }

  const items = bin?.items ?? []

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Trash className="size-4" aria-hidden="true" />
          回收站
        </CardTitle>
        <CardDescription>
          删除只做软删除：恢复时内容、修订链与在作品里的位置都复用原来那一份，
          不会重写一条新修订。稿本与正文为保留审计链只留在回收站里，不做物理删除。
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="flex items-center gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={pending !== null}
            onClick={() => void handleRefresh()}
          >
            {pending === "refresh" ? (
              <LoaderCircle className="animate-spin" aria-hidden="true" />
            ) : (
              <RefreshCw aria-hidden="true" />
            )}
            刷新
          </Button>
          <span className="text-xs text-muted-foreground">
            {items.length > 0 ? `${items.length} 项已删除` : "回收站是空的"}
          </span>
        </div>

        {notice ? (
          <p
            role="status"
            className="flex items-start gap-2 text-xs text-muted-foreground"
          >
            <CircleAlert
              className="mt-0.5 size-3.5 shrink-0"
              aria-hidden="true"
            />
            {notice}
          </p>
        ) : null}

        <ul className="space-y-3">
          {items.map((item) => {
            const key = `${item.kind}:${item.id}`
            const busy = pending === key
            const parent = parentText(item)
            return (
              <li
                key={key}
                className="rounded-xl border border-border bg-background/60 px-3 py-3"
              >
                <div className="flex flex-wrap items-start justify-between gap-2">
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <Badge variant="secondary">{kindLabels[item.kind]}</Badge>
                      <span className="truncate font-medium">{item.title}</span>
                    </div>
                    <p className="mt-1 text-xs text-muted-foreground">
                      {formatDeletedAt(item.deletedAtMs)} 删除
                      {parent ? ` · 原属${parent}` : ""}
                      {item.detail ? ` · ${item.detail}` : ""}
                    </p>
                  </div>
                  <div className="flex shrink-0 items-center gap-2">
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      disabled={busy || !item.restorable}
                      title={item.restoreBlockedReason ?? undefined}
                      onClick={() => void handleRestore(item)}
                    >
                      {busy ? (
                        <LoaderCircle
                          className="animate-spin"
                          aria-hidden="true"
                        />
                      ) : (
                        <RotateCcw aria-hidden="true" />
                      )}
                      恢复
                    </Button>
                    {item.purgeable ? (
                      <Button
                        type="button"
                        variant={
                          confirmingPurge === key ? "destructive" : "ghost"
                        }
                        size="sm"
                        disabled={busy}
                        onClick={() => void handlePurge(item)}
                      >
                        <Trash2 aria-hidden="true" />
                        {confirmingPurge === key ? "确认彻底删除" : "彻底删除"}
                      </Button>
                    ) : null}
                  </div>
                </div>
                {!item.restorable && item.restoreBlockedReason ? (
                  <p className="mt-2 text-xs text-muted-foreground">
                    现在不能恢复：{item.restoreBlockedReason}
                  </p>
                ) : null}
                {!item.purgeable && item.purgeBlockedReason ? (
                  <p className="mt-1 text-xs text-muted-foreground">
                    不能彻底删除：{item.purgeBlockedReason}
                  </p>
                ) : null}
              </li>
            )
          })}
        </ul>
      </CardContent>
    </Card>
  )
}
