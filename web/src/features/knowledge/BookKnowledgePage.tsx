/** Book › Knowledge (PROJECT §8.10): the scopes this book's writer and reviewer may search. */
import { useEffect, useState } from "react"
import { Link, useNavigate, useParams } from "react-router"
import { useTranslation } from "react-i18next"
import { BookOpen, ExternalLink } from "lucide-react"

import { useBookScopes, usePutBookScopes, useScopes } from "@/api/hooks/knowledge"
import { useResearchTasks, useStartKnowledgeCheck } from "@/api/hooks/research"
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
import { Label } from "@/components/ui/label"
import { Skeleton } from "@/components/ui/skeleton"
import { Textarea } from "@/components/ui/textarea"
import { NewResearchTaskDialog } from "@/features/research/NewResearchTaskDialog"
import { ResearchTaskList } from "@/features/research/ResearchTaskList"
import { latestPreparationTask, knowledgeCheckOpen } from "@/features/research/researchTask"

export default function BookKnowledgePage() {
  const { t } = useTranslation()
  const { bookId: rawBookId } = useParams()
  const bookId = Number(rawBookId)
  const navigate = useNavigate()
  const showError = useErrorToast()
  const scopes = useScopes()
  const attached = useBookScopes(bookId)
  const save = usePutBookScopes(bookId)
  // The attached set is the initial selection; a toggle overrides it (derived state). `knownIds`
  // keeps the scopes that were already loaded: a scope created after the page loaded must not be
  // sent as unchecked, because it is visible by default (D-158).
  const [selection, setSelection] = useState<number[] | null>(null)
  const [knownIds, setKnownIds] = useState<Set<number> | null>(null)
  const [saved, setSaved] = useState(false)
  const [creating, setCreating] = useState(false)
  const [checking, setChecking] = useState(false)
  const [checkFocus, setCheckFocus] = useState("")
  const checkTasks = useResearchTasks({ bookId })
  const startCheck = useStartKnowledgeCheck(bookId)
  const all = scopes.data ?? []
  const selected = selection ?? attached.data?.map((scope) => scope.id) ?? []
  const checkTask = latestPreparationTask(checkTasks.data ?? [])
  const checkIsOpen = knowledgeCheckOpen(checkTask)

  useEffect(() => {
    if (scopes.data === undefined) {
      return
    }
    const ids = scopes.data.map((scope) => scope.id)
    if (knownIds === null) {
      // Capture the scopes that were loaded with the page. A scope that arrives later is
      // visible by default and must never be sent as an exclusion by a stale save (D-158).
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setKnownIds(new Set(ids))
      return
    }
    const added = ids.filter((id) => !knownIds.has(id))
    if (added.length === 0) {
      return
    }
    // A late scope is visible by default: select it so the author can still uncheck it. The
    // `knownIds` guard keeps it out of a stale save if the author never sees this update.
    setKnownIds(new Set([...knownIds, ...added]))
    setSelection((previous) => [
      ...new Set([...(previous ?? attached.data?.map((scope) => scope.id) ?? []), ...added]),
    ])
  }, [scopes.data, knownIds, attached.data])

  if (scopes.isPending || attached.isPending || attached.data === undefined) {
    return <Skeleton className="m-6 h-64" />
  }

  return (
    <div className="mx-auto max-w-2xl space-y-4 p-6">
      <div>
        <h2 className="text-base font-semibold">{t("knowledge:book_scopes_title")}</h2>
        <p className="text-sm text-muted-foreground">{t("knowledge:book_scopes_hint")}</p>
      </div>

      {all.length === 0 ? (
        <EmptyState
          title={t("knowledge:no_scopes")}
          description={t("knowledge:no_scopes_hint")}
        >
          <Button asChild size="sm">
            <Link to="/knowledge">{t("knowledge:open_global")}</Link>
          </Button>
        </EmptyState>
      ) : (
        <div className="space-y-1 rounded-md border p-2">
          {all.map((scope) => (
            <label
              key={scope.id}
              className="flex cursor-pointer items-start gap-2 rounded-md px-2 py-1.5 hover:bg-accent"
            >
              <Checkbox
                checked={selected.includes(scope.id)}
                onCheckedChange={(checked) => {
                  setSaved(false)
                  setSelection(
                    checked === true
                      ? [...selected, scope.id]
                      : selected.filter((id) => id !== scope.id),
                  )
                }}
              />
              <span className="min-w-0">
                <span className="flex items-center gap-1.5 text-[13px] font-medium">
                  <BookOpen className="size-3.5" aria-hidden="true" />
                  {scope.name}
                </span>
                {scope.description === "" ? null : (
                  <span className="block text-xs text-muted-foreground">{scope.description}</span>
                )}
                <span className="block text-[0.7rem] text-muted-foreground">
                  {t("knowledge:entry_count", { n: scope.entry_count })}
                </span>
              </span>
            </label>
          ))}
        </div>
      )}

      <div className="flex items-center gap-2">
        <Button
          disabled={save.isPending}
          onClick={() =>
            save.mutate(
              all
                .filter(
                  (scope) =>
                    knownIds?.has(scope.id) === true &&
                    !selected.includes(scope.id),
                )
                .map((scope) => scope.id),
              {
              onSuccess: () => setSaved(true),
              onError: showError,
              },
            )
          }
        >
          {t("common:save")}
        </Button>
        {saved ? (
          <span className="text-xs text-muted-foreground">{t("knowledge:book_scopes_saved")}</span>
        ) : null}
        <Button
          size="sm"
          variant="outline"
          disabled={checkIsOpen || startCheck.isPending}
          title={checkIsOpen ? t("knowledge:check_disabled_open") : undefined}
          onClick={() => {
            setCheckFocus("")
            setChecking(true)
          }}
        >
          {t("knowledge:check_knowledge")}
        </Button>
        {checkIsOpen ? (
          <span className="text-xs text-muted-foreground">
            {t("knowledge:check_disabled_open")}
          </span>
        ) : null}
        <Button asChild variant="link" size="sm" className="ml-auto">
          <Link to="/knowledge">
            <ExternalLink className="size-3.5" /> {t("knowledge:open_global")}
          </Link>
        </Button>
      </div>

      {/* The book's research tasks (PROJECT §8.10): they collect into the scopes above. */}
      <ResearchTaskList bookId={bookId} onCreate={() => setCreating(true)} />
      <NewResearchTaskDialog
        open={creating}
        onOpenChange={setCreating}
        bookId={bookId}
      />
      <KnowledgeCheckDialog
        open={checking}
        focus={checkFocus}
        pending={startCheck.isPending}
        onFocusChange={setCheckFocus}
        onClose={() => setChecking(false)}
        onSubmit={() => {
          startCheck.mutate(checkFocus.trim(), {
            onSuccess: (response) => {
              setChecking(false)
              void navigate(`/research/${response.task.id}`)
            },
            onError: showError,
          })
        }}
      />
    </div>
  )
}

function KnowledgeCheckDialog({
  open,
  focus,
  pending,
  onFocusChange,
  onClose,
  onSubmit,
}: {
  open: boolean
  focus: string
  pending: boolean
  onFocusChange: (value: string) => void
  onClose: () => void
  onSubmit: () => void
}) {
  const { t } = useTranslation()
  return (
    <Dialog open={open} onOpenChange={(next) => (next ? undefined : onClose())}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("knowledge:check_dialog_title")}</DialogTitle>
        </DialogHeader>
        <div className="space-y-1">
          <Label htmlFor="knowledge-check-focus">{t("knowledge:check_focus")}</Label>
          <Textarea
            id="knowledge-check-focus"
            rows={3}
            value={focus}
            placeholder={t("knowledge:check_focus_placeholder")}
            onChange={(event) => onFocusChange(event.target.value)}
          />
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={onClose}>
            {t("common:cancel")}
          </Button>
          <Button disabled={pending} onClick={onSubmit}>
            {pending ? t("common:saving") : t("knowledge:check_start")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
