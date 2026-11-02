/**
 * The session header (PROJECT §8.6, spec/11 §5.3): role icon, session title, status badge, model
 * name, a session switcher and an info popover showing the frozen system prompt and tool list.
 *
 * Titles are composed here from data (`kind`, `chapter_number`, `research_topic`), never by the
 * server (D-054).
 */
import { useTranslation } from "react-i18next"
import {
  Archive,
  ChevronDown,
  ClipboardCheck,
  Info,
  ListTree,
  PenLine,
  Search,
  Sparkles,
} from "lucide-react"
import type { SessionDto } from "@/api/gen/SessionDto"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import { ScrollArea } from "@/components/ui/scroll-area"
import { SessionStatusBadge } from "@/features/agent/SessionStatusBadge"
import { cn } from "@/lib/utils"

/** The role's icon (PROJECT §8.6), as a component so React sees a stable type. */
export function RoleIcon({ role, className }: { role: SessionDto["role"]; className?: string }) {
  switch (role) {
    case "conception":
      return <Sparkles aria-hidden="true" className={className} />
    case "researcher":
      return <Search aria-hidden="true" className={className} />
    case "planner":
      return <ListTree aria-hidden="true" className={className} />
    case "writer":
      return <PenLine aria-hidden="true" className={className} />
    case "reviewer":
      return <ClipboardCheck aria-hidden="true" className={className} />
    case "archivist":
      return <Archive aria-hidden="true" className={className} />
    default:
      return <Sparkles aria-hidden="true" className={className} />
  }
}

/**
 * The localized session title: `agent:session_title.<kind>` with the chapter number or topic as
 * data (spec/11 §9, D-054).
 */
export function sessionTitle(session: SessionDto, t: (key: string, options?: Record<string, unknown>) => string): string {
  if (session.research_task_kind === "preparation") {
    return t("agent:session_title.knowledge_check")
  }
  const base = t(`agent:session_title.${session.kind}`)
  if (session.chapter_number !== null) {
    return `${base} · ${t("agent:chapter_number", { number: session.chapter_number })}`
  }
  if (session.research_topic !== null && session.research_topic !== "") {
    return `${base} · ${session.research_topic}`
  }
  return base
}

/** The effort part of the header subtitle: " · High" or nothing when no effort is frozen. */
export function sessionEffortLabel(
  t: (key: string, options?: Record<string, unknown>) => string,
  effort: string | null | undefined,
): string | null {
  if (effort === null || effort === undefined) {
    return null
  }
  return t(`settings:value.effort.${effort}`, { defaultValue: effort })
}

export function SessionHeader({
  session,
  sessions,
  onSelect,
  pausing,
  stopping,
  systemPrompt,
  toolNames,
}: {
  session: SessionDto
  sessions: SessionDto[]
  onSelect: (sessionId: number) => void
  pausing?: boolean
  stopping?: boolean
  systemPrompt: string
  toolNames: string[]
}) {
  const { t } = useTranslation()
  const title = sessionTitle(session, t)
  const effort = sessionEffortLabel(t, session.effort)

  return (
    <div className="flex items-center gap-2 border-b px-3 py-2">
      <RoleIcon role={session.role} className="size-4 shrink-0 text-muted-foreground" />
      <div className="min-w-0 flex-1">
        <div className="truncate text-sm font-medium" title={title}>
          {title}
        </div>
        <div className="truncate text-xs text-muted-foreground">
          {session.model_label}
          {effort === null ? null : ` · ${effort}`}
        </div>
      </div>
      <SessionStatusBadge
        status={session.status}
        pausing={pausing}
        stopping={stopping}
        retrying={session.retrying}
      />

      <Popover>
        <PopoverTrigger asChild>
          <Button variant="ghost" size="icon" aria-label={t("agent:session_info")}>
            <Info />
          </Button>
        </PopoverTrigger>
        <PopoverContent align="end" className="w-96">
          <div className="space-y-3">
            <div>
              <div className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
                {t("agent:session_info_prompt")}
              </div>
              <ScrollArea className="mt-1 h-40 rounded border">
                <pre className="p-2 text-xs whitespace-pre-wrap">{systemPrompt}</pre>
              </ScrollArea>
            </div>
            <div>
              <div className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
                {t("agent:session_info_tools")}
              </div>
              <div className="mt-1 flex flex-wrap gap-1">
                {toolNames.map((name) => (
                  <span key={name} className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs">
                    {name}
                  </span>
                ))}
              </div>
            </div>
          </div>
        </PopoverContent>
      </Popover>

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="icon" aria-label={t("agent:session_switch")}>
            <ChevronDown />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-72">
          <DropdownMenuLabel>{t("agent:session_switch")}</DropdownMenuLabel>
          <DropdownMenuSeparator />
          {sessions.length === 0 ? (
            <DropdownMenuItem disabled>{t("agent:empty")}</DropdownMenuItem>
          ) : (
            sessions.map((candidate) => (
              <DropdownMenuItem
                key={candidate.id}
                onSelect={() => onSelect(candidate.id)}
                className={cn("flex items-center gap-2", candidate.id === session.id && "bg-accent")}
              >
                <RoleIcon role={candidate.role} className="size-3.5 shrink-0" />
                <span className="min-w-0 flex-1 truncate">{sessionTitle(candidate, t)}</span>
                <span className="shrink-0 text-xs text-muted-foreground">
                  {t(`agent:status.${candidate.status}`)}
                </span>
              </DropdownMenuItem>
            ))
          )}
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  )
}
