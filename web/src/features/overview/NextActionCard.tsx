/**
 * The next action card of the book overview (PROJECT §8.3, spec/08 §7).
 *
 * The API sends the action's kind and the data it names (`chapter`, `session_id`, `track_id`,
 * `research_task_id`) plus the runnable research count; the label and the destination are composed
 * here (D-054). Every kind leads somewhere: the navigation kinds are `Link`s, `write_next` opens the
 * chapter it created, and `run_research` opens the research page (D-136). The two archivist kinds
 * (`seed_canon`, `sync_canon`) have no page — PROJECT §8.4 puts the seeding "live in the agent
 * panel", and spec/11 §5.3 puts a book's sessions there — so they pin the session they started in
 * the panel, expand it if it was collapsed, and name where the run is watched (D-135).
 */
import { useCallback, useState } from "react"
import { Link, useNavigate } from "react-router"
import { useTranslation } from "react-i18next"
import type { TFunction } from "i18next"
import { ArrowRight, CalendarClock, Loader2, PanelRightOpen } from "lucide-react"
import { toast } from "sonner"

import { useSeedCanon } from "@/api/hooks/conception"
import { useMaintainChapter, useResyncChapter } from "@/api/hooks/reviews"
import { useCreateChapter } from "@/api/hooks/tracks"
import type { BookStatus } from "@/api/gen/BookStatus"
import type { NextActionDto } from "@/api/gen/NextActionDto"
import type { NextActionKind } from "@/api/gen/NextActionKind"
import type { TrackProgressDto } from "@/api/gen/TrackProgressDto"
import { useErrorToast } from "@/components/common/ErrorToast"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { formatChapterLabel, trackName } from "@/lib/chapterLabel"
import { BulkDeferReport } from "@/features/research/BulkDeferReport"
import {
  useDeferReadyResearchTasks,
  type BulkDeferOutcome,
} from "@/features/research/useBulkDefer"
import { useBookShell } from "@/features/workspace/shell"
import { useUiStore } from "@/stores/ui"

type NextActionProps = {
  bookId: number
  /** The book's status: creation is only allowed while the book is `active` (spec/08 §1.1). */
  status: BookStatus
  action: NextActionDto
  tracks: TrackProgressDto[]
}

/**
 * The kinds whose run happens in the agent panel rather than on a page (spec/08 §7, PROJECT §8.4).
 * Typed against the generated union, so a new kind has to be classified here or in the `switch`
 * below before it can render.
 */
const ARCHIVIST_KINDS: ReadonlySet<NextActionKind> = new Set<NextActionKind>([
  "seed_canon",
  "sync_canon",
])

/** The title of the session an archivist action starts or continues (composed in the UI, spec/11 §6). */
function archivistSessionTitle(t: TFunction, action: NextActionDto): string {
  if (action.kind !== "sync_canon") {
    return t("agent:session_title.canon_seed")
  }
  return action.chapter?.canon_status === "stale"
    ? t("agent:session_title.chapter_resync")
    : t("agent:session_title.chapter_maintenance")
}

/** The overview's "what to do next" card. */
export function NextActionCard({ bookId, status, action, tracks }: NextActionProps) {
  const { t } = useTranslation()
  // The workspace owns the panel's collapsed state (it observes the real panel size), so the hint
  // and the panel can never disagree (D-135). Outside a workspace (a test page, a future embed)
  // there is nothing to collapse, so an open panel is the honest assumption.
  const shell = useBookShell()
  const panelCollapsed = shell?.agentPanelCollapsed ?? false
  // Remembered per kind: when the overview answers a different action the guidance is back, so no
  // effect has to reset it (the same shape `AgentPanel` uses for its "pausing" hints).
  const [startedKind, setStartedKind] = useState<NextActionKind | null>(null)
  // The kind belongs to the request that was clicked; the equality below hides the line if the
  // overview has moved on to another action while that request was in flight (R9-04).
  const started = startedKind === action.kind
  const archivist = ARCHIVIST_KINDS.has(action.kind)

  const track = tracks.find((entry) => entry.track_id === action.track_id)
  const chapter =
    action.chapter === null || action.chapter === undefined
      ? undefined
      : formatChapterLabel(t, { number: action.chapter.number, track })
  const label = t(`book:next_action.${action.kind}`, {
    chapter: chapter ?? "",
    track: track === undefined ? "" : trackName(t, track),
    // `run_research` is the only plural label: it names how many tasks wait, so the author sees
    // "Run 8 research tasks" rather than an unspecific "Run research tasks" (D-136).
    count: action.ready_research ?? 0,
  })
  const startedText = t("book:next_action.started_in_agent_panel", {
    session: archivistSessionTitle(t, action),
  })

  const onArchivistStarted = useCallback(
    (sessionId: number) => {
      // A started archivist run has exactly one window: the agent panel. Pin the session there,
      // reopen the panel when the author had collapsed it, and say both out loud (D-135).
      const ui = useUiStore.getState()
      ui.setPinnedSession(bookId, sessionId)
      shell?.expandAgentPanel()
      setStartedKind(action.kind)
      toast.success(startedText)
    },
    [action.kind, bookId, shell, startedText],
  )

  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-sm">{t("book:next_action_title")}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-2">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <p className="text-base font-medium">{label}</p>
          <ActionButton
            bookId={bookId}
            status={status}
            action={action}
            onArchivistStarted={onArchivistStarted}
          />
        </div>
        {action.kind === "run_research" ? (
          // The release valve where the demand appears (D-139): the author is on the overview when
          // the card insists on research, so postponing it must not require finding the list
          // first. It is secondary by construction — a ghost button under the primary link, with
          // the count in its label and the consequence in the hint — and it defers the book's
          // runnable tasks through the same shared batch the list uses.
          <DeferResearchSection bookId={bookId} overviewCount={action.ready_research ?? 0} />
        ) : null}
        {archivist ? (
          // Where the run happens, in visible text: without it the card's only reaction is a
          // spinner, which is what made seeding look like it did nothing (D-135). When the author
          // has collapsed the panel the same line says so and offers to open it, because a hint
          // that points at something invisible is no better than no hint at all.
          <div className="space-y-1">
            <p
              // The started line is announced (spec/11 §7: session status changes are announced in
              // a polite live region); the guidance before the click is static text and is not.
              role={started ? "status" : undefined}
              className="text-xs text-muted-foreground"
            >
              {started
                ? startedText
                : panelCollapsed
                  ? t("book:next_action.runs_in_agent_panel_collapsed")
                  : t("book:next_action.runs_in_agent_panel")}
            </p>
            {panelCollapsed ? (
              <Button variant="outline" size="sm" onClick={() => shell?.expandAgentPanel()}>
                <PanelRightOpen aria-hidden="true" />
                {t("book:next_action.show_agent_panel")}
              </Button>
            ) : null}
          </div>
        ) : null}
      </CardContent>
    </Card>
  )
}

/** The button (or link) that performs the action. */
function ActionButton({
  bookId,
  status,
  action,
  onArchivistStarted,
}: {
  bookId: number
  status: BookStatus
  action: NextActionDto
  onArchivistStarted: (sessionId: number) => void
}) {
  const { t } = useTranslation()
  const chapterId = action.chapter?.id
  switch (action.kind) {
    case "continue_conception":
      return (
        <Button asChild size="sm">
          <Link to={`/books/${bookId}/conception`}>
            {t("book:next_action.go")}
            <ArrowRight aria-hidden="true" />
          </Link>
        </Button>
      )
    case "seed_canon":
      return <SeedCanonButton bookId={bookId} onStarted={onArchivistStarted} />
    case "run_research":
      // The step has a page, so it is a link like the chapter kinds (D-136). A single task has its
      // own page, whose header carries the Run button, so the author lands where the click is; with
      // several tasks the author has to choose one, and only the list shows the choice. The count
      // in the label says how many are waiting either way.
      return (
        <Button asChild size="sm">
          <Link
            to={
              action.research_task_id === undefined || action.research_task_id === null
                ? "/research"
                : `/research/${action.research_task_id}`
            }
          >
            {t("book:next_action.go")}
            <ArrowRight aria-hidden="true" />
          </Link>
        </Button>
      )
    case "review_chapter":
    case "continue_chapter":
      return chapterId === undefined ? null : (
        <Button asChild size="sm">
          <Link to={`/books/${bookId}/chapters/${chapterId}`}>
            {t("book:next_action.go")}
            <ArrowRight aria-hidden="true" />
          </Link>
        </Button>
      )
    // The outline and write steps land on the tab that owns the action (D-146).
    case "write_outline":
    case "write_chapter": {
      const tab = action.kind === "write_outline" ? "outline" : "text"
      return chapterId === undefined ? null : (
        <Button asChild size="sm">
          <Link to={`/books/${bookId}/chapters/${chapterId}?tab=${tab}`}>
            {t("book:next_action.go")}
            <ArrowRight aria-hidden="true" />
          </Link>
        </Button>
      )
    }
    case "sync_canon":
      if (chapterId === undefined || action.chapter === null) {
        return null
      }
      // pending/failed canon is retried with `maintain`; only stale text is re-synced (R3-04).
      return action.chapter?.canon_status === "stale" ? (
        <ResyncButton
          bookId={bookId}
          chapterId={chapterId}
          onStarted={onArchivistStarted}
        />
      ) : (
        <MaintainButton
          bookId={bookId}
          chapterId={chapterId}
          onStarted={onArchivistStarted}
        />
      )
    case "write_next":
      return action.track_id === undefined || action.track_id === null ? null : (
        <WriteNextButton trackId={action.track_id} bookId={bookId} status={status} />
      )
    default:
      return null
  }
}

function SeedCanonButton({
  bookId,
  onStarted,
}: {
  bookId: number
  onStarted: (sessionId: number) => void
}) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const seed = useSeedCanon(bookId)
  return (
    <Button
      size="sm"
      disabled={seed.isPending}
      onClick={() =>
        seed.mutate(undefined, {
          // spec/08 §5: the endpoint answers the open `canon_seed` session or starts one, so the
          // id is the session the panel has to show either way.
          onSuccess: (response) => onStarted(response.session_id),
          onError: showError,
        })
      }
    >
      {seed.isPending ? <Loader2 aria-hidden="true" className="animate-spin" /> : null}
      {t("book:next_action.go")}
    </Button>
  )
}

function MaintainButton({
  bookId,
  chapterId,
  onStarted,
}: {
  bookId: number
  chapterId: number
  onStarted: (sessionId: number) => void
}) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const maintain = useMaintainChapter(chapterId, bookId)
  return (
    <Button
      size="sm"
      disabled={maintain.isPending}
      onClick={() =>
        maintain.mutate(undefined, {
          onSuccess: (response) => onStarted(response.session_id),
          onError: showError,
        })
      }
    >
      {maintain.isPending ? <Loader2 aria-hidden="true" className="animate-spin" /> : null}
      {t("book:next_action.go")}
    </Button>
  )
}

function ResyncButton({
  bookId,
  chapterId,
  onStarted,
}: {
  bookId: number
  chapterId: number
  onStarted: (sessionId: number) => void
}) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const resync = useResyncChapter(chapterId, bookId)
  return (
    <Button
      size="sm"
      disabled={resync.isPending}
      onClick={() =>
        resync.mutate(undefined, {
          // spec/08 §1.1: the re-sync starts a `chapter_resync` session; the panel is where its
          // progress shows, exactly like the seeding run.
          onSuccess: (response) => onStarted(response.session_id),
          onError: showError,
        })
      }
    >
      {resync.isPending ? <Loader2 aria-hidden="true" className="animate-spin" /> : null}
      {t("book:next_action.go")}
    </Button>
  )
}

/** Creates the next chapter of the track and opens it (the navigator's "+ Next chapter"). */
function WriteNextButton({
  bookId,
  trackId,
  status,
}: {
  bookId: number
  trackId: number
  status: BookStatus
}) {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const showError = useErrorToast()
  const create = useCreateChapter(trackId)
  // A chapter can only be created on an `active` book (spec/08 §1.1, D-130), so the button is a
  // defensive disable rather than the primary rule. `books::next_action` answers
  // `ContinueConception` for a conceiving book before any other branch
  // (`crates/server/src/services/books.rs`), so this button is never rendered for one: `archived`
  // is the only reachable non-active status here. Should that ever change, the reason below has to
  // grow the conceiving wording as well.
  const canCreate = status === "active"
  return (
    <div className="flex flex-col items-end gap-1">
      <Button
        size="sm"
        disabled={!canCreate || create.isPending}
        onClick={() =>
          create.mutate(
            { title: "", guidance: "" },
            {
              onSuccess: (chapter) => void navigate(`/books/${bookId}/chapters/${chapter.id}`),
              onError: showError,
            },
          )
        }
      >
        {create.isPending ? <Loader2 aria-hidden="true" className="animate-spin" /> : null}
        {t("book:next_action.go")}
      </Button>
      {canCreate ? null : (
        // A disabled HTML button takes no pointer events, so a `title` tooltip would never appear;
        // the reason is visible text instead, worded exactly as in the navigator and the side
        // story page.
        <p className="text-xs text-muted-foreground">{t("book:next_chapter_archived")}</p>
      )}
    </div>
  )
}

/**
 * The secondary "not now" of the `run_research` card (D-139).
 *
 * The primary action stays the primary: the "Go" link is the only `default`-variant control in the
 * card, and this block sits under it in a quieter row (`ghost` button, muted hint). The count comes
 * from the book's list query rather than from `ready_research` on the action, because that query is
 * what the shared batch defers and the two must not disagree; the action's own count is used only
 * until the list has loaded.
 */
function DeferResearchSection({
  bookId,
  overviewCount,
}: {
  bookId: number
  overviewCount: number
}) {
  const { t } = useTranslation()
  const bulk = useDeferReadyResearchTasks({ bookId })
  const [outcome, setOutcome] = useState<BulkDeferOutcome | null>(null)
  // The batch defers the rows of the book's list query, so the count comes from there and not from
  // `ready_research` on the action — the two must never disagree about what the click will do. The
  // action's own count bridges only the moment before the list has loaded. Once the batch runs, the
  // list is empty of `ready` rows and the overview refetches to the next action, so this whole
  // block goes away with the card (D-138 invalidates that book's overview).
  const count = bulk.isLoading ? overviewCount : bulk.readyTasks.length
  if (count === 0) {
    return null
  }
  return (
    <div className="space-y-1">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="text-xs text-muted-foreground">
          {t("book:next_action.defer_research_hint")}
        </p>
        <Button
          size="sm"
          variant="ghost"
          disabled={bulk.isLoading || bulk.isPending}
          onClick={() => void bulk.run().then(setOutcome)}
        >
          {bulk.isPending ? (
            <Loader2 className="animate-spin" aria-hidden="true" />
          ) : (
            <CalendarClock aria-hidden="true" />
          )}
          {t("book:next_action.defer_research", { count })}
        </Button>
      </div>
      <BulkDeferReport outcome={outcome} />
    </div>
  )
}
