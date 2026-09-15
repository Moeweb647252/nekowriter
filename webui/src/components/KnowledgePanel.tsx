import { useEffect, useState, type FormEvent } from "react"
import {
  BookMarked,
  CircleAlert,
  CircleCheck,
  Database,
  KeyRound,
  LoaderCircle,
  Plus,
  RefreshCw,
  Save,
  Search,
  Trash2,
} from "lucide-react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Spinner } from "@/components/ui/spinner"
import {
  ApiError,
  accumulateKnowledge,
  createCommandId,
  createKnowledgeEntry,
  createKnowledgeSpace,
  deleteKnowledgeEntry,
  deleteKnowledgeSpace,
  getCapabilities,
  getEmbeddingSettings,
  getKnowledgeEntry,
  listKnowledgeEntries,
  listKnowledgeSpaces,
  reindexKnowledge,
  renameKnowledgeSpace,
  saveEmbeddingSettings,
  searchKnowledge,
  testEmbeddingConnection,
  updateKnowledgeEntry,
  type AgentCapabilities,
  type EmbeddingSettings,
  type KnowledgeEntry,
  type KnowledgeEntrySummary,
  type KnowledgeIndexCoverage,
  type KnowledgeIndexStatus,
  type KnowledgeSearchResult,
  type KnowledgeSpace,
} from "@/lib/api"

interface KnowledgePanelProps {
  refreshToken?: number
}

type Notice =
  | { tone: "success"; message: string }
  | { tone: "error"; message: string }
  | null

interface EmbeddingDraft {
  apiBaseUrl: string
  modelId: string
  dimension: string
  apiKey: string
}

interface EntryDraft {
  title: string
  keywords: string
  content: string
  source: string
}

const indexStatusLabels: Record<KnowledgeIndexStatus, string> = {
  not_configured: "未配置 Embedding",
  pending: "等待索引",
  running: "正在索引",
  ready: "已索引",
  failed: "索引失败",
}

const emptyEmbeddingDraft: EmbeddingDraft = {
  apiBaseUrl: "https://api.openai.com/v1",
  modelId: "",
  dimension: "1024",
  apiKey: "",
}

const emptyEntryDraft: EntryDraft = {
  title: "",
  keywords: "",
  content: "",
  source: "",
}

function errorText(error: unknown, fallback: string) {
  return error instanceof ApiError || error instanceof Error
    ? error.message
    : fallback
}

function draftFromEntry(entry: KnowledgeEntry): EntryDraft {
  return {
    title: entry.title,
    keywords: entry.keywords.join("、"),
    content: entry.content,
    source: entry.source ?? "",
  }
}

function splitKeywords(value: string) {
  return value
    .split(/[、,，\n]/)
    .map((keyword) => keyword.trim())
    .filter((keyword) => keyword.length > 0)
}

function coverageSummary(coverage: KnowledgeIndexCoverage) {
  const parts = [
    `已索引 ${coverage.readyEntries}`,
    `等待 ${coverage.pendingEntries}`,
    `失败 ${coverage.failedEntries}`,
  ]
  if (BigInt(coverage.notConfiguredEntries) > 0n) {
    parts.push(`未配置 ${coverage.notConfiguredEntries}`)
  }
  return parts.join(" · ")
}

/**
 * 知识库工作台：Embedding 配置、Space 与条目管理、语义检索和显式重建索引。
 *
 * 知识不依赖稿本，因此这里的数据都是应用级的。索引状态由后台索引器推进，并通过
 * `knowledge.index_changed` 事件带来的 `refreshToken` 变化自动刷新。
 */
export function KnowledgePanel({ refreshToken = 0 }: KnowledgePanelProps) {
  const [embedding, setEmbedding] = useState<EmbeddingSettings | null>(null)
  const [embeddingDraft, setEmbeddingDraft] =
    useState<EmbeddingDraft>(emptyEmbeddingDraft)
  const [embeddingLoading, setEmbeddingLoading] = useState(true)
  const [savingEmbedding, setSavingEmbedding] = useState(false)
  const [testingEmbedding, setTestingEmbedding] = useState(false)
  const [clearingKey, setClearingKey] = useState(false)
  const [confirmingClearKey, setConfirmingClearKey] = useState(false)

  const [spaces, setSpaces] = useState<KnowledgeSpace[]>([])
  const [selectedSpaceId, setSelectedSpaceId] = useState<string | null>(null)
  const [spaceTitle, setSpaceTitle] = useState("")
  const [creatingSpace, setCreatingSpace] = useState(false)

  const [entries, setEntries] = useState<KnowledgeEntrySummary[]>([])
  const [keyword, setKeyword] = useState("")
  const [selectedEntryId, setSelectedEntryId] = useState<string | null>(null)
  const [selectedEntry, setSelectedEntry] = useState<KnowledgeEntry | null>(
    null
  )
  const [entryDraft, setEntryDraft] = useState<EntryDraft>(emptyEntryDraft)
  const [savingEntry, setSavingEntry] = useState(false)

  const [query, setQuery] = useState("")
  const [searching, setSearching] = useState(false)
  const [result, setResult] = useState<KnowledgeSearchResult | null>(null)
  const [reindexing, setReindexing] = useState(false)
  const [capabilities, setCapabilities] = useState<AgentCapabilities | null>(
    null
  )
  const [accumulationClues, setAccumulationClues] = useState("")
  const [accumulating, setAccumulating] = useState(false)
  const [notice, setNotice] = useState<Notice>(null)
  /** 本地写操作后强制重读列表，避免每个处理函数各写一份刷新逻辑。 */
  const [reloadToken, setReloadToken] = useState(0)

  useEffect(() => {
    let isCurrent = true
    getEmbeddingSettings()
      .then((response) => {
        if (!isCurrent) return
        setEmbedding(response.embedding)
        if (response.embedding) {
          setEmbeddingDraft({
            apiBaseUrl: response.embedding.profile.apiBaseUrl,
            modelId: response.embedding.profile.modelId,
            dimension: response.embedding.profile.dimension,
            apiKey: "",
          })
        }
      })
      .catch((error: unknown) => {
        if (!isCurrent) return
        setNotice({
          tone: "error",
          message: errorText(error, "无法读取 Embedding 配置"),
        })
      })
      .finally(() => {
        if (isCurrent) setEmbeddingLoading(false)
      })
    getCapabilities()
      .then((next) => {
        if (isCurrent) setCapabilities(next)
      })
      .catch(() => {
        if (isCurrent) setCapabilities(null)
      })
    listKnowledgeSpaces()
      .then((next) => {
        if (!isCurrent) return
        setSpaces(next)
        setSelectedSpaceId((current) =>
          current && next.some((space) => space.id === current)
            ? current
            : (next[0]?.id ?? null)
        )
      })
      .catch((error: unknown) => {
        if (!isCurrent) return
        setNotice({
          tone: "error",
          message: errorText(error, "无法读取知识 Space"),
        })
      })
    return () => {
      isCurrent = false
    }
  }, [refreshToken, reloadToken])

  useEffect(() => {
    let isCurrent = true
    listKnowledgeEntries(selectedSpaceId ?? undefined, keyword)
      .then((next) => {
        if (isCurrent) setEntries(next)
      })
      .catch((error: unknown) => {
        if (!isCurrent) return
        setNotice({
          tone: "error",
          message: errorText(error, "无法读取知识条目"),
        })
      })
    return () => {
      isCurrent = false
    }
  }, [selectedSpaceId, keyword, reloadToken])

  useEffect(() => {
    if (selectedEntryId === null) return
    let isCurrent = true
    getKnowledgeEntry(selectedEntryId)
      .then((entry) => {
        if (!isCurrent) return
        setSelectedEntry(entry)
        setEntryDraft(draftFromEntry(entry))
      })
      .catch((error: unknown) => {
        if (!isCurrent) return
        setNotice({
          tone: "error",
          message: errorText(error, "无法读取知识条目"),
        })
      })
    return () => {
      isCurrent = false
    }
  }, [selectedEntryId])

  function selectSpace(spaceId: string) {
    setSelectedSpaceId(spaceId)
    setSelectedEntryId(null)
    setSelectedEntry(null)
    setEntryDraft(emptyEntryDraft)
  }

  function startNewEntry() {
    setSelectedEntryId(null)
    setSelectedEntry(null)
    setEntryDraft(emptyEntryDraft)
  }

  async function handleSaveEmbedding(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setSavingEmbedding(true)
    setNotice(null)
    try {
      // 密钥留空就是省略字段（保留已保存的密钥）；清除要显式发 `null`，见「清除密钥」。
      const saved = await saveEmbeddingSettings(
        embedding?.revision ?? "0",
        embeddingDraft.apiBaseUrl.trim(),
        embeddingDraft.modelId.trim(),
        embeddingDraft.dimension.trim(),
        embeddingDraft.apiKey.trim() || undefined,
        createCommandId()
      )
      setEmbedding(saved.embedding)
      setEmbeddingDraft((current) => ({ ...current, apiKey: "" }))
      setReloadToken((current) => current + 1)
      setNotice({
        tone: "success",
        message: "Embedding 配置已保存，并通过一次真实调用验证",
      })
    } catch (error) {
      setNotice({
        tone: "error",
        message: errorText(error, "保存 Embedding 配置失败"),
      })
    } finally {
      setSavingEmbedding(false)
    }
  }

  /**
   * 清除已保存的 Embedding 密钥，把知识库退回「未配置 Embedding」降级。
   *
   * 命令只带已持久化的字段加上 `apiKey: null`：既不会顺手保存草稿里的编辑，也不会
   * 触发保存前那次真实探测（拿刚移除的密钥去探测必然失败）。
   */
  async function handleClearEmbeddingKey() {
    if (!embedding?.credentialConfigured) return
    if (!confirmingClearKey) {
      setConfirmingClearKey(true)
      return
    }
    setConfirmingClearKey(false)
    setClearingKey(true)
    setNotice(null)
    try {
      const saved = await saveEmbeddingSettings(
        embedding.revision,
        embedding.profile.apiBaseUrl,
        embedding.profile.modelId,
        embedding.profile.dimension,
        null,
        createCommandId()
      )
      setEmbedding(saved.embedding)
      setReloadToken((current) => current + 1)
      setNotice({
        tone: "success",
        message: "已清除本机保存的 Embedding 密钥；知识库回到「未配置」降级",
      })
    } catch (error) {
      setNotice({
        tone: "error",
        message: errorText(error, "清除 Embedding 密钥失败"),
      })
    } finally {
      setClearingKey(false)
    }
  }

  async function handleTestEmbedding() {
    setTestingEmbedding(true)
    setNotice(null)
    try {
      const response = await testEmbeddingConnection(
        embeddingDraft.apiBaseUrl.trim(),
        embeddingDraft.modelId.trim(),
        embeddingDraft.dimension.trim(),
        embeddingDraft.apiKey.trim() ? embeddingDraft.apiKey.trim() : null
      )
      setNotice({
        tone: "success",
        message: `连接成功，服务返回 ${response.dimension} 维向量`,
      })
    } catch (error) {
      setNotice({
        tone: "error",
        message: errorText(error, "Embedding 连接测试失败"),
      })
    } finally {
      setTestingEmbedding(false)
    }
  }

  async function handleCreateSpace(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setCreatingSpace(true)
    setNotice(null)
    try {
      const space = await createKnowledgeSpace(
        spaceTitle.trim(),
        createCommandId()
      )
      setSpaceTitle("")
      selectSpace(space.id)
      setReloadToken((current) => current + 1)
      setNotice({ tone: "success", message: `已创建 Space「${space.title}」` })
    } catch (error) {
      setNotice({ tone: "error", message: errorText(error, "创建 Space 失败") })
    } finally {
      setCreatingSpace(false)
    }
  }

  async function handleRenameSpace(space: KnowledgeSpace) {
    const title = window.prompt("新的 Space 标题", space.title)
    if (!title || !title.trim() || title.trim() === space.title) return
    setNotice(null)
    try {
      await renameKnowledgeSpace(
        space.id,
        space.revision,
        title.trim(),
        createCommandId()
      )
      setReloadToken((current) => current + 1)
    } catch (error) {
      setNotice({
        tone: "error",
        message: errorText(error, "重命名 Space 失败"),
      })
    }
  }

  async function handleDeleteSpace(space: KnowledgeSpace) {
    setNotice(null)
    try {
      await deleteKnowledgeSpace(space.id, space.revision, createCommandId())
      setReloadToken((current) => current + 1)
      setNotice({ tone: "success", message: `已删除 Space「${space.title}」` })
    } catch (error) {
      setNotice({ tone: "error", message: errorText(error, "删除 Space 失败") })
    }
  }

  async function handleSaveEntry(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setSavingEntry(true)
    setNotice(null)
    try {
      const draft = {
        title: entryDraft.title.trim(),
        keywords: splitKeywords(entryDraft.keywords),
        content: entryDraft.content,
        source: entryDraft.source.trim() ? entryDraft.source.trim() : null,
      }
      const saved =
        selectedEntry === null
          ? await createKnowledgeEntry(
              selectedSpaceId ?? "",
              draft,
              createCommandId()
            )
          : await updateKnowledgeEntry(
              selectedEntry.id,
              selectedEntry.revision,
              draft,
              createCommandId()
            )
      setSelectedEntry(saved)
      setSelectedEntryId(saved.id)
      setEntryDraft(draftFromEntry(saved))
      setReloadToken((current) => current + 1)
      setNotice({
        tone: "success",
        message:
          saved.indexStatus === "not_configured"
            ? "已保存；尚未配置 Embedding，暂不索引"
            : "已保存，索引任务已建立",
      })
    } catch (error) {
      setNotice({
        tone: "error",
        message: errorText(error, "保存知识条目失败"),
      })
    } finally {
      setSavingEntry(false)
    }
  }

  async function handleDeleteEntry() {
    if (!selectedEntry) return
    setNotice(null)
    try {
      await deleteKnowledgeEntry(
        selectedEntry.id,
        selectedEntry.revision,
        createCommandId()
      )
      startNewEntry()
      setReloadToken((current) => current + 1)
      setNotice({ tone: "success", message: "已删除知识条目" })
    } catch (error) {
      setNotice({
        tone: "error",
        message: errorText(error, "删除知识条目失败"),
      })
    }
  }

  async function handleSearch(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setSearching(true)
    setNotice(null)
    try {
      setResult(
        await searchKnowledge(
          query.trim(),
          selectedSpaceId ? [selectedSpaceId] : [],
          5
        )
      )
    } catch (error) {
      setResult(null)
      setNotice({ tone: "error", message: errorText(error, "语义检索失败") })
    } finally {
      setSearching(false)
    }
  }

  async function handleAccumulate(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!selectedSpaceId) return
    setAccumulating(true)
    setNotice(null)
    try {
      const created = await accumulateKnowledge(
        selectedSpaceId,
        accumulationClues.trim()
      )
      setAccumulationClues("")
      setReloadToken((current) => current + 1)
      setNotice({
        tone: "success",
        message: `积累引擎写入了 ${created.length} 条候选知识。`,
      })
    } catch (error) {
      // 没有生产引擎时，服务端以 ENGINE_NOT_AVAILABLE 给出原因；这里如实展示，
      // 不展示任何模拟进度。
      setNotice({ tone: "error", message: errorText(error, "知识积累不可用") })
    } finally {
      setAccumulating(false)
    }
  }

  async function handleReindex() {
    setReindexing(true)
    setNotice(null)
    try {
      const coverage = await reindexKnowledge(createCommandId())
      setReloadToken((current) => current + 1)
      setNotice({
        tone: "success",
        message: `已重建索引：${coverageSummary(coverage)}`,
      })
    } catch (error) {
      setNotice({ tone: "error", message: errorText(error, "重建索引失败") })
    } finally {
      setReindexing(false)
    }
  }

  const selectedSpace =
    spaces.find((space) => space.id === selectedSpaceId) ?? null

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Database className="size-4" aria-hidden="true" />
          知识库
        </CardTitle>
        <CardDescription>
          保存设定与资料，并按 Embedding 配置建立语义索引；向量只可由正文重建。
        </CardDescription>
        <CardAction>
          <Button
            type="button"
            size="sm"
            variant="outline"
            onClick={() => void handleReindex()}
            disabled={reindexing || embedding === null}
          >
            {reindexing ? (
              <LoaderCircle className="animate-spin" aria-hidden="true" />
            ) : (
              <RefreshCw aria-hidden="true" />
            )}
            重建索引
          </Button>
        </CardAction>
      </CardHeader>
      <CardContent className="space-y-5">
        {notice && (
          <div
            className={`flex items-start gap-2 rounded-xl border px-3 py-2 text-sm ${
              notice.tone === "success"
                ? "border-emerald-500/30 bg-emerald-500/8 text-emerald-700"
                : "border-destructive/25 bg-destructive/8 text-destructive"
            }`}
            role={notice.tone === "error" ? "alert" : "status"}
          >
            {notice.tone === "success" ? (
              <CircleCheck
                className="mt-0.5 size-4 shrink-0"
                aria-hidden="true"
              />
            ) : (
              <CircleAlert
                className="mt-0.5 size-4 shrink-0"
                aria-hidden="true"
              />
            )}
            <span>{notice.message}</span>
          </div>
        )}

        <form className="space-y-3" onSubmit={handleSaveEmbedding}>
          <div className="flex items-center justify-between">
            <h3 className="flex items-center gap-2 text-sm font-semibold">
              <KeyRound className="size-4" aria-hidden="true" />
              Embedding 配置
            </h3>
            {embeddingLoading ? (
              <Spinner aria-hidden="true" />
            ) : (
              <Badge variant={embedding ? "default" : "outline"}>
                {embedding
                  ? `已配置 · ${embedding.profile.dimension} 维`
                  : "未配置"}
              </Badge>
            )}
          </div>
          <div className="grid gap-3 sm:grid-cols-3">
            <Label className="text-xs">
              服务地址
              <Input
                className="mt-1"
                value={embeddingDraft.apiBaseUrl}
                onChange={(event) =>
                  setEmbeddingDraft((current) => ({
                    ...current,
                    apiBaseUrl: event.target.value,
                  }))
                }
                placeholder="https://api.openai.com/v1"
                required
              />
            </Label>
            <Label className="text-xs">
              模型 ID
              <Input
                className="mt-1"
                value={embeddingDraft.modelId}
                onChange={(event) =>
                  setEmbeddingDraft((current) => ({
                    ...current,
                    modelId: event.target.value,
                  }))
                }
                placeholder="text-embedding-3-small"
                required
              />
            </Label>
            <Label className="text-xs">
              输出维度
              <Input
                className="mt-1"
                inputMode="numeric"
                value={embeddingDraft.dimension}
                onChange={(event) =>
                  setEmbeddingDraft((current) => ({
                    ...current,
                    dimension: event.target.value,
                  }))
                }
                required
              />
            </Label>
          </div>
          <Label className="block text-xs">
            API 密钥
            <Input
              className="mt-1"
              type="password"
              value={embeddingDraft.apiKey}
              onChange={(event) =>
                setEmbeddingDraft((current) => ({
                  ...current,
                  apiKey: event.target.value,
                }))
              }
              placeholder={
                embedding?.credentialConfigured
                  ? "已保存；留空表示保持不变"
                  : "只写入本机数据库"
              }
            />
          </Label>
          {embedding?.credentialConfigured ? (
            <Button
              type="button"
              size="sm"
              variant={confirmingClearKey ? "destructive" : "ghost"}
              onClick={() => void handleClearEmbeddingKey()}
              disabled={clearingKey || savingEmbedding}
            >
              {clearingKey ? (
                <LoaderCircle className="animate-spin" aria-hidden="true" />
              ) : (
                <Trash2 aria-hidden="true" />
              )}
              {clearingKey
                ? "正在清除…"
                : confirmingClearKey
                  ? "确认清除密钥"
                  : "清除密钥"}
            </Button>
          ) : null}
          <div className="flex flex-wrap gap-2">
            <Button type="submit" size="sm" disabled={savingEmbedding}>
              {savingEmbedding ? (
                <LoaderCircle className="animate-spin" aria-hidden="true" />
              ) : (
                <Save aria-hidden="true" />
              )}
              保存并验证
            </Button>
            <Button
              type="button"
              size="sm"
              variant="outline"
              onClick={() => void handleTestEmbedding()}
              disabled={testingEmbedding}
            >
              {testingEmbedding ? (
                <LoaderCircle className="animate-spin" aria-hidden="true" />
              ) : (
                <Search aria-hidden="true" />
              )}
              测试连接
            </Button>
          </div>
        </form>

        <div className="border-t border-border pt-4">
          <h3 className="flex items-center gap-2 text-sm font-semibold">
            <BookMarked className="size-4" aria-hidden="true" />
            Space
          </h3>
          <div className="mt-3 flex flex-wrap gap-2">
            {spaces.map((space) => (
              <span key={space.id} className="flex items-center gap-1">
                <Button
                  type="button"
                  size="sm"
                  variant={space.id === selectedSpaceId ? "default" : "outline"}
                  onClick={() => selectSpace(space.id)}
                >
                  {space.title}
                  <span className="ml-1 text-xs opacity-70">
                    {space.entryCount}
                  </span>
                </Button>
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  onClick={() => void handleRenameSpace(space)}
                  aria-label={`重命名 ${space.title}`}
                >
                  <Save aria-hidden="true" />
                </Button>
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  onClick={() => void handleDeleteSpace(space)}
                  aria-label={`删除 ${space.title}`}
                >
                  <Trash2 aria-hidden="true" />
                </Button>
              </span>
            ))}
            {spaces.length === 0 && (
              <p className="text-sm text-muted-foreground">
                还没有 Space；先创建一个再录入知识。
              </p>
            )}
          </div>
          <form className="mt-3 flex gap-2" onSubmit={handleCreateSpace}>
            <Input
              value={spaceTitle}
              onChange={(event) => setSpaceTitle(event.target.value)}
              placeholder="新 Space 标题"
              required
            />
            <Button type="submit" size="sm" disabled={creatingSpace}>
              {creatingSpace ? (
                <LoaderCircle className="animate-spin" aria-hidden="true" />
              ) : (
                <Plus aria-hidden="true" />
              )}
              新建
            </Button>
          </form>
        </div>

        <div className="border-t border-border pt-4">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <h3 className="text-sm font-semibold">
              知识条目
              {selectedSpace && (
                <span className="ml-2 font-normal text-muted-foreground">
                  {selectedSpace.title}
                </span>
              )}
            </h3>
            <div className="flex gap-2">
              <Input
                className="h-8 w-40"
                value={keyword}
                onChange={(event) => setKeyword(event.target.value)}
                placeholder="按标题或关键词筛选"
              />
              <Button
                type="button"
                size="sm"
                variant="ghost"
                onClick={startNewEntry}
              >
                <Plus aria-hidden="true" />
                新建条目
              </Button>
            </div>
          </div>
          <ul className="mt-3 space-y-1">
            {entries.map((entry) => (
              <li key={entry.id}>
                <button
                  type="button"
                  onClick={() => setSelectedEntryId(entry.id)}
                  className={`flex w-full items-center justify-between gap-2 rounded-lg border px-3 py-2 text-left text-sm transition ${
                    selectedEntry?.id === entry.id
                      ? "border-primary/50 bg-primary/8"
                      : "border-border hover:border-foreground/25"
                  }`}
                >
                  <span className="min-w-0">
                    <span className="block truncate font-medium">
                      {entry.title}
                    </span>
                    <span className="block truncate text-xs text-muted-foreground">
                      {entry.keywords.join("、") || "无关键词"} · 第{" "}
                      {entry.revision} 版 · {entry.contentChars} 字
                    </span>
                  </span>
                  <Badge
                    variant={
                      entry.indexStatus === "ready" ? "default" : "outline"
                    }
                  >
                    {indexStatusLabels[entry.indexStatus]}
                  </Badge>
                </button>
              </li>
            ))}
            {entries.length === 0 && (
              <li className="rounded-lg border border-dashed border-border px-3 py-4 text-center text-sm text-muted-foreground">
                这个范围内还没有知识条目。
              </li>
            )}
          </ul>

          {selectedSpace && (
            <form className="mt-4 space-y-3" onSubmit={handleSaveEntry}>
              <Label className="text-xs">
                标题
                <Input
                  className="mt-1"
                  value={entryDraft.title}
                  onChange={(event) =>
                    setEntryDraft((current) => ({
                      ...current,
                      title: event.target.value,
                    }))
                  }
                  required
                />
              </Label>
              <Label className="text-xs">
                关键词（用、或逗号分隔）
                <Input
                  className="mt-1"
                  value={entryDraft.keywords}
                  onChange={(event) =>
                    setEntryDraft((current) => ({
                      ...current,
                      keywords: event.target.value,
                    }))
                  }
                />
              </Label>
              <Label className="text-xs">
                正文
                <textarea
                  className="mt-1 min-h-32 w-full rounded-xl border border-input bg-background px-3 py-2 text-sm outline-none focus:border-ring focus:ring-3 focus:ring-ring/20"
                  value={entryDraft.content}
                  onChange={(event) =>
                    setEntryDraft((current) => ({
                      ...current,
                      content: event.target.value,
                    }))
                  }
                  required
                />
              </Label>
              <Label className="text-xs">
                来源说明
                <Input
                  className="mt-1"
                  value={entryDraft.source}
                  onChange={(event) =>
                    setEntryDraft((current) => ({
                      ...current,
                      source: event.target.value,
                    }))
                  }
                />
              </Label>
              {selectedEntry?.indexError && (
                <p className="text-xs text-destructive">
                  上次索引失败：{selectedEntry.indexError}
                </p>
              )}
              <div className="flex gap-2">
                <Button type="submit" size="sm" disabled={savingEntry}>
                  {savingEntry ? (
                    <LoaderCircle className="animate-spin" aria-hidden="true" />
                  ) : (
                    <Save aria-hidden="true" />
                  )}
                  {selectedEntry ? "保存修改" : "创建条目"}
                </Button>
                {selectedEntry && (
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    onClick={() => void handleDeleteEntry()}
                  >
                    <Trash2 aria-hidden="true" />
                    删除
                  </Button>
                )}
              </div>
            </form>
          )}
        </div>

        <form
          className="space-y-3 border-t border-border pt-4"
          onSubmit={handleSearch}
        >
          <h3 className="flex items-center gap-2 text-sm font-semibold">
            <Search className="size-4" aria-hidden="true" />
            语义检索
          </h3>
          <div className="flex gap-2">
            <Input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={
                selectedSpace
                  ? `在「${selectedSpace.title}」内检索`
                  : "在全部知识中检索"
              }
              required
            />
            <Button type="submit" size="sm" disabled={searching}>
              {searching ? (
                <LoaderCircle className="animate-spin" aria-hidden="true" />
              ) : (
                <Search aria-hidden="true" />
              )}
              检索
            </Button>
          </div>
          {result && (
            <div className="space-y-2">
              <p className="text-xs text-muted-foreground">
                {coverageSummary(result.coverage)}
              </p>
              {result.hits.length === 0 ? (
                <p className="rounded-lg border border-dashed border-border px-3 py-3 text-center text-sm text-muted-foreground">
                  这条查询没有命中已索引的原文块。
                </p>
              ) : (
                <ul className="space-y-2">
                  {result.hits.map((hit) => (
                    <li
                      key={hit.chunkId}
                      className="rounded-lg border border-border px-3 py-2 text-sm"
                    >
                      <div className="flex items-center justify-between gap-2">
                        <span className="truncate font-medium">
                          {hit.title}
                        </span>
                        <span className="shrink-0 text-xs text-muted-foreground">
                          相关度 {hit.score.toFixed(3)}
                        </span>
                      </div>
                      <p className="mt-1 line-clamp-4 text-xs whitespace-pre-wrap text-muted-foreground">
                        {hit.text}
                      </p>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          )}
        </form>

        <form
          className="space-y-3 border-t border-border pt-4"
          onSubmit={handleAccumulate}
        >
          <h3 className="flex items-center gap-2 text-sm font-semibold">
            <BookMarked className="size-4" aria-hidden="true" />
            知识积累
          </h3>
          {capabilities === null ||
          capabilities.knowledgeAccumulationEngines.length === 0 ? (
            <p className="rounded-lg border border-dashed border-border px-3 py-3 text-sm text-muted-foreground">
              暂无可用引擎。
              {capabilities?.knowledgeAccumulationUnavailableReason
                ? ` ${capabilities.knowledgeAccumulationUnavailableReason}`
                : ""}
            </p>
          ) : (
            <p className="text-xs text-muted-foreground">
              可用引擎：
              {capabilities.knowledgeAccumulationEngines
                .map((engine) => engine.displayName)
                .join("、")}
            </p>
          )}
          <div className="space-y-1">
            <Label htmlFor="accumulation-clues">线索</Label>
            <Input
              id="accumulation-clues"
              value={accumulationClues}
              onChange={(event) => setAccumulationClues(event.target.value)}
              placeholder="例如：雾港的航路与灯塔制度"
              required
            />
          </div>
          <div className="flex items-center justify-between gap-2">
            <p className="text-xs text-muted-foreground">
              候选条目写入{" "}
              {selectedSpace ? `「${selectedSpace.title}」` : "目标 Space"}
              ，写入后再由你确认修改。
            </p>
            <Button
              type="submit"
              size="sm"
              variant="outline"
              disabled={
                accumulating ||
                selectedSpaceId === null ||
                accumulationClues.trim().length === 0
              }
            >
              {accumulating ? (
                <LoaderCircle className="animate-spin" aria-hidden="true" />
              ) : (
                <BookMarked aria-hidden="true" />
              )}
              开始积累
            </Button>
          </div>
        </form>
      </CardContent>
    </Card>
  )
}
