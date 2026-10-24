/** The prompt editor shared by the global page and the book page (PROJECT §8.11, spec/09 §4):
 * grouped list, metadata, variable docs, editor, diff vs default, reset and validation errors. */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { ChevronDown, ChevronRight, RotateCcw, Search, Sparkles } from "lucide-react"

import { useErrorToast } from "@/components/common/ErrorToast"
import { DiffView } from "@/components/common/DiffView"
import { EmptyState } from "@/components/common/EmptyState"
import { ErrorToastInline } from "@/features/settings/ErrorToastInline"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Skeleton } from "@/components/ui/skeleton"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Textarea } from "@/components/ui/textarea"
import { errorDetailNumber, isApiError } from "@/api/hooks/errors"
import { usePrompts, useResetPrompt, useSavePrompt } from "@/api/hooks/prompts"
import type { PromptEntryDto } from "@/api/gen/PromptEntryDto"
import type { PromptKind } from "@/api/gen/PromptKind"
import type { PromptSource } from "@/api/gen/PromptSource"
import { cn } from "@/lib/utils"

/** The category groups inside every agent, in the order of spec/09 §2 (PROJECT §8.11). */
const GROUP_ORDER: Array<{ key: string; kinds: PromptKind[] }> = [
  { key: "system", kinds: ["system"] },
  { key: "tasks", kinds: ["task"] },
  { key: "blocks", kinds: ["block"] },
  { key: "notices", kinds: ["notice"] },
  { key: "results", kinds: ["result"] },
  { key: "shared", kinds: ["shared"] },
  { key: "tools", kinds: ["tool"] },
]

/** The agents of the left list, in the order the spec lists the roles (PROJECT §8.11). */
const AGENT_ORDER = [
  "writer",
  "reviewer",
  "archivist",
  "conception",
  "researcher",
  "shared",
] as const

/** One category group of one agent. */
export type PromptCategoryGroup = { key: string; entries: PromptEntryDto[] }

/** One agent of the left list with its categories (PROJECT §8.11: agent *and* category). */
export type PromptAgentGroup = { agent: string; groups: PromptCategoryGroup[] }

/**
 * Builds the two-level list of the prompts page: one group per agent, each holding its
 * categories (PROJECT §8.11, F8.1). An agent the pack adds later still appears — it is appended
 * after the known roles rather than dropped.
 */
export function groupPrompts(entries: PromptEntryDto[]): PromptAgentGroup[] {
  const agents = [...new Set(entries.map((entry) => entry.agent))]
  agents.sort((left, right) => {
    const leftIndex = AGENT_ORDER.indexOf(left as (typeof AGENT_ORDER)[number])
    const rightIndex = AGENT_ORDER.indexOf(right as (typeof AGENT_ORDER)[number])
    const leftRank = leftIndex === -1 ? AGENT_ORDER.length : leftIndex
    const rightRank = rightIndex === -1 ? AGENT_ORDER.length : rightIndex
    return leftRank - rightRank || left.localeCompare(right)
  })
  return agents
    .map((agent) => ({
      agent,
      groups: GROUP_ORDER.map((group) => ({
        key: group.key,
        entries: entries.filter(
          (entry) => entry.agent === agent && group.kinds.includes(entry.kind),
        ),
      })).filter((group) => group.entries.length > 0),
    }))
    .filter((group) => group.groups.length > 0)
}

/** True for languages written without spaces (used by the diff, spec/11 §5.2). */
const UNSPACED_LANGUAGES = ["zh", "ja", "th", "lo", "km", "my"]

/** The editor of one prompt pack level (global or one book). */
export type PromptEditorProps = {
  /** The book whose overrides are edited; `undefined` is the global level. */
  bookId?: number
  /** True when the page edits book overrides (the book page). */
  bookScope: boolean
}

export function PromptEditor({ bookId, bookScope }: PromptEditorProps) {
  const { t, i18n } = useTranslation()
  const prompts = usePrompts(bookId)
  const [search, setSearch] = useState("")
  const [selected, setSelected] = useState<string | null>(null)

  if (prompts.isPending) {
    return <Skeleton className="h-96" />
  }
  if (prompts.isError) {
    return <ErrorToastInline />
  }
  const entries = prompts.data ?? []
  const filtered =
    search.trim() === ""
      ? entries
      : entries.filter((entry) => {
          const needle = search.trim().toLowerCase()
          return (
            entry.key.toLowerCase().includes(needle) ||
            entry.title_en.toLowerCase().includes(needle) ||
            entry.title_zh.includes(search.trim())
          )
        })
  const current =
    entries.find((entry) => entry.key === selected) ??
    (filtered.length === 0 ? undefined : filtered[0])

  return (
    <div className="flex gap-4">
      <aside className="w-64 shrink-0">
        <div className="relative mb-2">
          <Search className="pointer-events-none absolute top-2 left-2 size-3.5 text-muted-foreground" />
          <Input
            value={search}
            placeholder={t("prompts:search")}
            className="pl-7"
            onChange={(event) => setSearch(event.target.value)}
          />
        </div>
        <div className="max-h-[70vh] space-y-1 overflow-y-auto pr-1">
          {filtered.length === 0 ? (
            <p className="px-1 py-2 text-xs text-muted-foreground">{t("prompts:no_matches")}</p>
          ) : (
            groupPrompts(filtered).map((agentGroup) => (
              <PromptAgentSection
                key={agentGroup.agent}
                agentGroup={agentGroup}
                selected={current?.key}
                onSelect={setSelected}
                bookScope={bookScope}
              />
            ))
          )}
        </div>
      </aside>
      <section className="min-w-0 flex-1">
        {current === undefined ? (
          <EmptyState title={t("prompts:no_selection")} />
        ) : (
          <PromptPane
            key={current.key}
            entry={current}
            bookId={bookId}
            bookScope={bookScope}
            unspaced={UNSPACED_LANGUAGES.some((prefix) =>
              i18n.language.toLowerCase().startsWith(prefix),
            )}
          />
        )}
      </section>
    </div>
  )
}

/** One agent of the left list: its name, then its collapsible categories (PROJECT §8.11). */
function PromptAgentSection({
  agentGroup,
  selected,
  onSelect,
  bookScope,
}: {
  agentGroup: PromptAgentGroup
  selected: string | undefined
  onSelect: (key: string) => void
  bookScope: boolean
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(true)
  const Chevron = open ? ChevronDown : ChevronRight
  const count = agentGroup.groups.reduce((total, group) => total + group.entries.length, 0)
  return (
    <div className="mb-1">
      <button
        type="button"
        className="flex w-full items-center gap-1 rounded-md px-1 py-1 text-xs font-semibold"
        onClick={() => setOpen(!open)}
      >
        <Chevron className="size-3.5" />
        {agentLabel(t, agentGroup.agent)}
        <span className="ml-auto font-normal text-muted-foreground">{count}</span>
      </button>
      {open ? (
        <div className="mt-0.5 ml-1 border-l pl-1.5">
          {agentGroup.groups.map((group) => (
            <PromptGroup
              key={group.key}
              group={group.key}
              entries={group.entries}
              selected={selected}
              onSelect={onSelect}
              bookScope={bookScope}
            />
          ))}
        </div>
      ) : null}
    </div>
  )
}

/** The localized name of a pack agent; `shared` is the pack's own word for cross-role prompts. */
function agentLabel(t: (key: string) => string, agent: string): string {
  return t(`prompts:agent.${agent}`)
}

/** One collapsible category group inside an agent. */
function PromptGroup({
  group,
  entries,
  selected,
  onSelect,
  bookScope,
}: {
  group: string
  entries: PromptEntryDto[]
  selected: string | undefined
  onSelect: (key: string) => void
  bookScope: boolean
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(true)
  const Chevron = open ? ChevronDown : ChevronRight
  return (
    <div>
      <button
        type="button"
        className="flex w-full items-center gap-1 px-1 py-1 text-[0.7rem] font-medium tracking-wide text-muted-foreground uppercase"
        onClick={() => setOpen(!open)}
      >
        <Chevron className="size-3" />
        {t(`prompts:group.${group}`)}
        <span className="ml-auto font-normal normal-case">{entries.length}</span>
      </button>
      {open ? (
        <ul className="space-y-0.5">
          {entries.map((entry) => {
            const overridden = bookScope
              ? entry.book_override !== undefined && entry.book_override !== null
              : entry.global_override !== undefined && entry.global_override !== null
            return (
              <li key={entry.key}>
                <button
                  type="button"
                  className={cn(
                    "flex w-full items-center gap-1.5 rounded-md px-2 py-1 text-left text-[13px] text-muted-foreground hover:bg-accent hover:text-foreground",
                    entry.key === selected && "bg-accent font-medium text-foreground",
                  )}
                  onClick={() => onSelect(entry.key)}
                >
                  <span className="truncate">
                    {entry.title_zh !== "" ? entry.title_zh : entry.title_en}
                  </span>
                  <SourceBadge
                    source={overridden ? (bookScope ? "book" : "global") : "default"}
                    className="ml-auto shrink-0"
                  />
                </button>
              </li>
            )
          })}
        </ul>
      ) : null}
    </div>
  )
}

/** The D / G / B source badge of spec/10 §8.11. */
function SourceBadge({ source, className }: { source: PromptSource; className?: string }) {
  const { t } = useTranslation()
  const label =
    source === "book"
      ? t("prompts:source.book")
      : source === "global"
        ? t("prompts:source.global")
        : t("prompts:source.default")
  return (
    <Badge variant={source === "default" ? "outline" : "secondary"} className={className}>
      {label}
    </Badge>
  )
}

/** The editor of one key. */
function PromptPane({
  entry,
  bookId,
  bookScope,
  unspaced,
}: {
  entry: PromptEntryDto
  bookId?: number
  bookScope: boolean
  unspaced: boolean
}) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const save = useSavePrompt(bookId)
  const reset = useResetPrompt(bookId)
  const stored = bookScope
    ? (entry.book_override ?? entry.global_override ?? entry.default_text)
    : (entry.global_override ?? entry.default_text)
  const [draft, setDraft] = useState<string>(stored)
  const [error, setError] = useState<string | null>(null)
  const dirty = draft !== stored
  // The diff is between the effective text of this level and the shipped default.
  const hasOverride = entry.book_override !== undefined && entry.book_override !== null
    ? true
    : entry.global_override !== undefined && entry.global_override !== null

  const submit = () => {
    setError(null)
    save.mutate(
      {
        key: entry.key,
        input: {
          scope: bookScope ? "book" : "global",
          book_id: bookScope ? (bookId ?? null) : null,
          content: draft,
        },
      },
      {
        onSuccess: () => {
          setError(null)
        },
        onError: (thrown) => {
          if (isApiError(thrown) && thrown.code === "prompt.template_invalid") {
            const line = errorDetailNumber(thrown, "line")
            const column = errorDetailNumber(thrown, "column")
            const message = thrown.message
            setError(
              t("prompts:invalid_at", {
                line: line ?? 0,
                column: column ?? 0,
                message,
              }),
            )
            return
          }
          showError(thrown)
        },
      },
    )
  }

  const resetLevel = () => {
    reset.mutate(
      {
        key: entry.key,
        scope: bookScope ? "book" : "global",
        bookId: bookScope ? bookId : undefined,
      },
      {
        onSuccess: (updated) => {
          const next = bookScope
            ? (updated.book_override ?? updated.global_override ?? updated.default_text)
            : (updated.global_override ?? updated.default_text)
          setDraft(next)
          setError(null)
        },
        onError: showError,
      },
    )
  }

  const title = entry.title_en === "" ? entry.key : `${entry.title_en}`

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="text-sm font-semibold">{title}</h2>
        <span className="font-mono text-xs text-muted-foreground">{entry.key}</span>
        <SourceBadge source={entry.source} />
        {entry.title_zh === "" ? null : (
          <span className="text-xs text-muted-foreground">{entry.title_zh}</span>
        )}
      </div>

      <Tabs defaultValue="editor">
        <TabsList>
          <TabsTrigger value="editor">{t("prompts:editor")}</TabsTrigger>
          <TabsTrigger value="diff">{t("prompts:diff")}</TabsTrigger>
          <TabsTrigger value="variables">{t("prompts:variables")}</TabsTrigger>
        </TabsList>
        <TabsContent value="editor" className="space-y-2">
          <Textarea
            className="min-h-[45vh] font-mono text-xs"
            value={draft}
            spellCheck={false}
            onChange={(event) => setDraft(event.target.value)}
          />
          {error === null ? (
            <p className="flex items-center gap-1.5 text-xs text-status-done">
              <Sparkles className="size-3.5" />
              {t("prompts:valid")}
            </p>
          ) : (
            <p className="text-xs whitespace-pre-wrap text-status-failed">{error}</p>
          )}
          <div className="flex gap-2">
            <Button
              size="sm"
              disabled={!dirty || save.isPending}
              onClick={submit}
            >
              {t("common:save")}
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={bookScope ? !hasOverride : !(entry.global_override ?? null)}
              onClick={() => setDraft(entry.default_text)}
            >
              {t("prompts:show_default")}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              disabled={reset.isPending}
              onClick={resetLevel}
            >
              <RotateCcw className="size-3.5" />
              {t("prompts:reset")}
            </Button>
          </div>
        </TabsContent>
        <TabsContent value="diff">
          <DiffView
            before={entry.default_text}
            after={draft}
            unspaced={unspaced}
            className="max-h-[55vh] overflow-y-auto"
          />
        </TabsContent>
        <TabsContent value="variables">
          {entry.variables.length === 0 ? (
            <EmptyState
              title={t("prompts:no_variables")}
              description={t("prompts:no_variables_hint")}
            />
          ) : (
            <div className="space-y-3">
              {entry.variables.map((variable) => (
                <div key={variable.name} className="rounded-md border p-2">
                  <div className="flex items-baseline gap-2">
                    <span className="font-mono text-xs font-medium">{variable.name}</span>
                    <span className="text-xs text-muted-foreground">
                      {documentation(variable.en, variable.zh, unspaced)}
                    </span>
                  </div>
                  {variable.fields.length === 0 ? null : (
                    <dl className="mt-1 grid grid-cols-[minmax(6rem,auto)_1fr] gap-x-2 gap-y-0.5 text-xs">
                      {variable.fields.map((field) => (
                        <FieldRow key={field.name} name={field.name} en={field.en} zh={field.zh} unspaced={unspaced} />
                      ))}
                    </dl>
                  )}
                </div>
              ))}
            </div>
          )}
        </TabsContent>
      </Tabs>
    </div>
  )
}

/** One documented field of a template variable. */
function FieldRow({
  name,
  en,
  zh,
  unspaced,
}: {
  name: string
  en: string
  zh: string
  unspaced: boolean
}) {
  return (
    <>
      <dt className="font-mono text-muted-foreground">{name}</dt>
      <dd>{documentation(en, zh, unspaced)}</dd>
    </>
  )
}

/** Picks the documentation in the UI language, like the pack titles (spec/09 §1). */
function documentation(en: string, zh: string, unspaced: boolean): string {
  return unspaced && zh !== "" ? zh : en
}
