/** The knowledge base page (PROJECT §8.10, spec/10 §2.5): scopes, entries and hybrid search in
 * three columns. */
import { useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { BookOpen, Pencil, Plus, Search, Trash2 } from "lucide-react"

import type { EntryDto } from "@/api/gen/EntryDto"
import type { ScopeDto } from "@/api/gen/ScopeDto"
import {
  useCreateEntry,
  useCreateScope,
  useDeleteEntry,
  useDeleteScope,
  useEntries,
  useEntry,
  useKnowledgeSearch,
  usePatchEntry,
  usePatchScope,
  useScopes,
} from "@/api/hooks/knowledge"
import { useErrorToast } from "@/components/common/ErrorToast"
import { EmptyState } from "@/components/common/EmptyState"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Skeleton } from "@/components/ui/skeleton"
import { Textarea } from "@/components/ui/textarea"
import { cn } from "@/lib/utils"

/** One editable entry draft. */
type EntryDraft = {
  title: string
  content: string
  sources: string
  tags: string
}

const EMPTY_DRAFT: EntryDraft = { title: "", content: "", sources: "", tags: "" }

/** Splits a comma/newline separated field into trimmed items. */
function splitList(value: string): string[] {
  return value
    .split(/[\n,]/)
    .map((item) => item.trim())
    .filter((item) => item.length > 0)
}

/** Wraps every occurrence of a query term in a `<mark>` (PROJECT §8.10 highlights the snippet). */
function highlight(text: string, query: string): React.ReactNode[] {
  const terms = query
    .split(/\s+/)
    .map((term) => term.trim())
    .filter((term) => term.length > 0)
  if (terms.length === 0) {
    return [text]
  }
  const escaped = terms.map((term) => term.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"))
  const pattern = new RegExp(`(${escaped.join("|")})`, "gi")
  return text.split(pattern).map((part, index) =>
    index % 2 === 1 ? (
      <mark key={index} className="rounded-sm bg-primary/15 text-foreground">
        {part}
      </mark>
    ) : (
      part
    ),
  )
}

export default function KnowledgePage() {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const scopes = useScopes()
  const [scopeId, setScopeId] = useState<number | undefined>(undefined)
  const [entryFilter, setEntryFilter] = useState("")
  const [entryId, setEntryId] = useState<number | undefined>(undefined)
  // Without an explicit choice the first scope is shown (no effect: derived state).
  const activeScopeId = scopeId ?? scopes.data?.[0]?.id

  const entries = useEntries(activeScopeId, entryFilter)
  const entry = useEntry(entryId)
  const createScope = useCreateScope()
  const patchScope = usePatchScope()
  const deleteScope = useDeleteScope()
  const createEntry = useCreateEntry(activeScopeId ?? 0)
  const patchEntry = usePatchEntry(entryId ?? 0)
  const deleteEntry = useDeleteEntry()

  const [scopeDialog, setScopeDialog] = useState<
    { mode: "create" } | { mode: "edit"; scope: ScopeDto } | null
  >(null)
  const [entryDialog, setEntryDialog] = useState(false)
  const [searchQuery, setSearchQuery] = useState("")
  const [searchScopeIds, setSearchScopeIds] = useState<number[]>([])
  const hits = useKnowledgeSearch(searchQuery, searchScopeIds)
  const searching = searchQuery.trim().length > 0

  const selectedScope = useMemo(
    () => scopes.data?.find((scope) => scope.id === activeScopeId),
    [scopes.data, activeScopeId],
  )

  const onScopeClick = (scope: ScopeDto) => {
    setScopeId(scope.id)
    setEntryId(undefined)
    setEntryFilter("")
  }

  return (
    <div className="flex h-[calc(100svh-3rem)] min-h-0">
      <aside className="flex w-60 shrink-0 flex-col border-r">
        <div className="flex items-center justify-between p-3">
          <h1 className="text-sm font-semibold">{t("knowledge:scopes")}</h1>
          <Button
            size="icon"
            variant="ghost"
            aria-label={t("knowledge:new_scope")}
            onClick={() => setScopeDialog({ mode: "create" })}
          >
            <Plus className="size-4" />
          </Button>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-2">
          {scopes.isPending ? <Skeleton className="h-32" /> : null}
          {scopes.data?.length === 0 ? (
            <EmptyState title={t("knowledge:no_scopes")} description={t("knowledge:no_scopes_hint")}>
              <Button size="sm" onClick={() => setScopeDialog({ mode: "create" })}>
                <Plus className="size-3.5" /> {t("knowledge:new_scope")}
              </Button>
            </EmptyState>
          ) : null}
          {scopes.data?.map((scope) => (
            <div
              key={scope.id}
              className={cn(
                "group flex items-start gap-1 rounded-md px-2 py-1.5",
                scope.id === activeScopeId && "bg-accent",
              )}
            >
              <button
                type="button"
                className="min-w-0 flex-1 text-left"
                onClick={() => onScopeClick(scope)}
              >
                <span className="flex items-center gap-2 text-[13px] font-medium">
                  <BookOpen className="size-3.5 shrink-0" aria-hidden="true" />
                  <span className="truncate">{scope.name}</span>
                </span>
                <span className="block text-[0.7rem] text-muted-foreground">
                  {t("knowledge:entry_count", { n: scope.entry_count })}
                </span>
              </button>
              <Button
                size="icon"
                variant="ghost"
                className="size-6 opacity-0 group-hover:opacity-100"
                aria-label={t("knowledge:edit_scope")}
                onClick={() => setScopeDialog({ mode: "edit", scope })}
              >
                <Pencil className="size-3" />
              </Button>
              <Button
                size="icon"
                variant="ghost"
                className="size-6 opacity-0 group-hover:opacity-100"
                aria-label={t("knowledge:delete_scope")}
                onClick={() => {
                  if (window.confirm(t("knowledge:delete_scope_confirm", { name: scope.name }))) {
                    deleteScope.mutate(scope.id, {
                      onSuccess: () => {
                        if (scope.id === activeScopeId) {
                          setScopeId(undefined)
                          setEntryId(undefined)
                        }
                      },
                      onError: showError,
                    })
                  }
                }}
              >
                <Trash2 className="size-3" />
              </Button>
            </div>
          ))}
        </div>
      </aside>

      <section className="flex w-80 shrink-0 flex-col border-r">
        <div className="space-y-2 p-3">
          <div className="relative">
            <Search className="absolute top-2 left-2 size-3.5 text-muted-foreground" />
            <Input
              value={searchQuery}
              placeholder={t("knowledge:search_placeholder")}
              className="pl-7"
              onChange={(event) => setSearchQuery(event.target.value)}
            />
          </div>
          {searching ? (
            <div className="flex flex-wrap gap-x-3 gap-y-1">
              {scopes.data?.map((scope) => (
                <label key={scope.id} className="flex items-center gap-1 text-[0.7rem]">
                  <Checkbox
                    checked={searchScopeIds.includes(scope.id)}
                    onCheckedChange={(checked) =>
                      setSearchScopeIds((current) =>
                        checked === true
                          ? [...current, scope.id]
                          : current.filter((id) => id !== scope.id),
                      )
                    }
                  />
                  {scope.name}
                </label>
              ))}
              <button
                type="button"
                className="text-[0.7rem] text-muted-foreground underline"
                onClick={() => setSearchScopeIds([])}
              >
                {t("knowledge:search_all_scopes")}
              </button>
            </div>
          ) : (
            <div className="flex items-center gap-2">
              <Input
                value={entryFilter}
                placeholder={t("knowledge:filter_entries")}
                onChange={(event) => setEntryFilter(event.target.value)}
              />
              <Button
                size="sm"
                disabled={selectedScope === undefined}
                onClick={() => setEntryDialog(true)}
              >
                <Plus className="size-3.5" /> {t("knowledge:new_entry")}
              </Button>
            </div>
          )}
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
          {searching ? (
            hits.isPending ? (
              <Skeleton className="h-24" />
            ) : hits.data?.length === 0 ? (
              <EmptyState title={t("knowledge:search_empty")} />
            ) : (
              hits.data?.map((hit) => (
                <button
                  key={`${hit.entry_id}-${hit.scope_name}`}
                  type="button"
                  className="mb-1 block w-full rounded-md px-2 py-1.5 text-left hover:bg-accent"
                  onClick={() => {
                    setEntryId(hit.entry_id)
                    setScopeId(hit.scope_id)
                  }}
                >
                  <span className="block truncate text-[13px] font-medium">{hit.title}</span>
                  <span className="block text-[0.7rem] text-muted-foreground">
                    {hit.scope_name}
                  </span>
                  <span className="mt-1 line-clamp-3 block text-xs text-muted-foreground">
                    {highlight(hit.snippet, searchQuery)}
                  </span>
                </button>
              ))
            )
          ) : entries.isPending ? (
            <Skeleton className="h-24" />
          ) : entries.data?.items.length === 0 ? (
            <EmptyState
              title={t("knowledge:no_entries")}
              description={t("knowledge:no_entries_hint")}
            />
          ) : (
            entries.data?.items.map((item) => (
              <button
                key={item.id}
                type="button"
                className={cn(
                  "mb-1 block w-full rounded-md px-2 py-1.5 text-left hover:bg-accent",
                  item.id === entryId && "bg-accent",
                )}
                onClick={() => setEntryId(item.id)}
              >
                <span className="block truncate text-[13px] font-medium">{item.title}</span>
                <span className="block text-[0.7rem] text-muted-foreground">
                  {t(`knowledge:source.${item.source}`)}
                  {item.tags.length > 0 ? ` · ${item.tags.join(", ")}` : ""}
                </span>
              </button>
            ))
          )}
        </div>
      </section>

      <section className="min-w-0 flex-1 overflow-y-auto p-4">
        {entry.data === undefined ? (
          <EmptyState
            title={t("knowledge:select_entry")}
            description={t("knowledge:select_entry_hint")}
          />
        ) : (
          <EntryEditor
            key={entry.data.id}
            entry={entry.data}
            pending={patchEntry.isPending}
            onSave={(draftInput) =>
              patchEntry.mutate(
                {
                  title: draftInput.title,
                  content: draftInput.content,
                  source_urls: splitList(draftInput.sources),
                  tags: splitList(draftInput.tags),
                },
                { onError: showError },
              )
            }
            onDelete={() => {
              if (window.confirm(t("knowledge:delete_entry_confirm", { title: entry.data?.title }))) {
                deleteEntry.mutate(entry.data.id, {
                  onSuccess: () => setEntryId(undefined),
                  onError: showError,
                })
              }
            }}
          />
        )}
      </section>

      {scopeDialog === null ? null : (
        <ScopeDialog
          scope={scopeDialog.mode === "edit" ? scopeDialog.scope : undefined}
          pending={createScope.isPending || patchScope.isPending}
          onClose={() => setScopeDialog(null)}
          onSubmit={(name, description) => {
            const onError = (error: unknown) => showError(error)
            if (scopeDialog.mode === "edit") {
              patchScope.mutate(
                { scopeId: scopeDialog.scope.id, patch: { name, description } },
                { onSuccess: () => setScopeDialog(null), onError },
              )
            } else {
              createScope.mutate(
                { name, description },
                {
                  onSuccess: (scope) => {
                    setScopeId(scope.id)
                    setScopeDialog(null)
                  },
                  onError,
                },
              )
            }
          }}
        />
      )}

      {entryDialog && activeScopeId !== undefined ? (
        <EntryDialog
          pending={createEntry.isPending}
          onClose={() => setEntryDialog(false)}
          onSubmit={(draftInput) => {
            createEntry.mutate(
              {
                title: draftInput.title,
                content: draftInput.content,
                source_urls: splitList(draftInput.sources),
                tags: splitList(draftInput.tags),
              },
              {
                onSuccess: (created) => {
                  setEntryId(created.id)
                  setEntryDialog(false)
                },
                onError: showError,
              },
            )
          }}
        />
      ) : null}
    </div>
  )
}

/** The entry editor: its state is initialized from the loaded row and keyed by the entry id, so
 * selecting another entry remounts it (no state-syncing effect). */
function EntryEditor({
  entry,
  pending,
  onSave,
  onDelete,
}: {
  entry: EntryDto
  pending: boolean
  onSave: (draft: EntryDraft) => void
  onDelete: () => void
}) {
  const { t } = useTranslation()
  const [draft, setDraft] = useState<EntryDraft>({
    title: entry.title,
    content: entry.content,
    sources: entry.source_urls.join("\n"),
    tags: entry.tags.join(", "),
  })
  return (
    <div className="mx-auto max-w-3xl space-y-3">
      <div className="flex items-center gap-2">
        <Input
          value={draft.title}
          aria-label={t("knowledge:entry_title")}
          onChange={(event) => setDraft({ ...draft, title: event.target.value })}
        />
        <Button disabled={pending} onClick={() => onSave(draft)}>
          {t("common:save")}
        </Button>
        <Button
          variant="ghost"
          size="icon"
          aria-label={t("knowledge:delete_entry")}
          onClick={onDelete}
        >
          <Trash2 className="size-4" />
        </Button>
      </div>
      <div className="grid grid-cols-2 gap-3">
        <div className="space-y-1">
          <Label>{t("knowledge:entry_sources")}</Label>
          <Textarea
            rows={2}
            value={draft.sources}
            placeholder={t("knowledge:entry_sources_hint")}
            onChange={(event) => setDraft({ ...draft, sources: event.target.value })}
          />
        </div>
        <div className="space-y-1">
          <Label>{t("knowledge:entry_tags")}</Label>
          <Input
            value={draft.tags}
            placeholder={t("knowledge:entry_tags_hint")}
            onChange={(event) => setDraft({ ...draft, tags: event.target.value })}
          />
        </div>
      </div>
      <div className="space-y-1">
        <Label>{t("knowledge:entry_content")}</Label>
        <Textarea
          rows={18}
          className="font-mono text-[13px]"
          value={draft.content}
          onChange={(event) => setDraft({ ...draft, content: event.target.value })}
        />
      </div>
      <p className="text-[0.7rem] text-muted-foreground">{t("knowledge:markdown_hint")}</p>
    </div>
  )
}

function ScopeDialog({
  scope,
  pending,
  onClose,
  onSubmit,
}: {
  scope: ScopeDto | undefined
  pending: boolean
  onClose: () => void
  onSubmit: (name: string, description: string) => void
}) {
  const { t } = useTranslation()
  const [name, setName] = useState(scope?.name ?? "")
  const [description, setDescription] = useState(scope?.description ?? "")
  return (
    <Dialog open onOpenChange={(open) => (open ? undefined : onClose())}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>
            {scope === undefined ? t("knowledge:new_scope") : t("knowledge:edit_scope")}
          </DialogTitle>
        </DialogHeader>
        <div className="space-y-3">
          <div className="space-y-1">
            <Label htmlFor="scope-name">{t("knowledge:scope_name")}</Label>
            <Input id="scope-name" value={name} onChange={(event) => setName(event.target.value)} />
          </div>
          <div className="space-y-1">
            <Label htmlFor="scope-description">{t("knowledge:scope_description")}</Label>
            <Textarea
              id="scope-description"
              rows={3}
              value={description}
              placeholder={t("knowledge:scope_description_hint")}
              onChange={(event) => setDescription(event.target.value)}
            />
          </div>
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={onClose}>
            {t("common:cancel")}
          </Button>
          <Button disabled={pending || name.trim() === ""} onClick={() => onSubmit(name, description)}>
            {t("common:save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function EntryDialog({
  pending,
  onClose,
  onSubmit,
}: {
  pending: boolean
  onClose: () => void
  onSubmit: (draft: EntryDraft) => void
}) {
  const { t } = useTranslation()
  const [draft, setDraft] = useState<EntryDraft>(EMPTY_DRAFT)
  return (
    <Dialog open onOpenChange={(open) => (open ? undefined : onClose())}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{t("knowledge:new_entry")}</DialogTitle>
        </DialogHeader>
        <div className="space-y-3">
          <div className="space-y-1">
            <Label htmlFor="entry-title">{t("knowledge:entry_title")}</Label>
            <Input
              id="entry-title"
              value={draft.title}
              onChange={(event) => setDraft({ ...draft, title: event.target.value })}
            />
          </div>
          <div className="space-y-1">
            <Label htmlFor="entry-content">{t("knowledge:entry_content")}</Label>
            <Textarea
              id="entry-content"
              rows={10}
              className="font-mono text-[13px]"
              value={draft.content}
              placeholder={t("knowledge:markdown_hint")}
              onChange={(event) => setDraft({ ...draft, content: event.target.value })}
            />
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-1">
              <Label htmlFor="entry-sources">{t("knowledge:entry_sources")}</Label>
              <Textarea
                id="entry-sources"
                rows={2}
                value={draft.sources}
                placeholder={t("knowledge:entry_sources_hint")}
                onChange={(event) => setDraft({ ...draft, sources: event.target.value })}
              />
            </div>
            <div className="space-y-1">
              <Label htmlFor="entry-tags">{t("knowledge:entry_tags")}</Label>
              <Input
                id="entry-tags"
                value={draft.tags}
                placeholder={t("knowledge:entry_tags_hint")}
                onChange={(event) => setDraft({ ...draft, tags: event.target.value })}
              />
            </div>
          </div>
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={onClose}>
            {t("common:cancel")}
          </Button>
          <Button
            disabled={pending || draft.title.trim() === "" || draft.content.trim() === ""}
            onClick={() => onSubmit(draft)}
          >
            {t("common:create")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
