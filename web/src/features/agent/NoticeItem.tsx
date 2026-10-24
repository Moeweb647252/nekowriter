/**
 * One appendix of a tool result, rendered under its card (spec/03 §5.3, spec/11 §5.3).
 *
 * The appendix text is already rendered by the prompt pack (Chinese, model-facing), so it is
 * shown verbatim as data; only the *label* of the kind is localized here.
 */
import { useTranslation } from "react-i18next"
import type { LucideIcon } from "lucide-react"
import { Info, MessageSquare, ScrollText, User } from "lucide-react"

import type { Appendix } from "@/api/gen/Appendix"

const ICONS = {
  effect_notice: Info,
  edit_report: ScrollText,
  user_feedback: MessageSquare,
  inbox: User,
} as const

/** The appendix kind's icon (a wrapper keeps the component type stable for React). */
function NoticeIcon({ icon: Icon }: { icon: LucideIcon }) {
  return <Icon aria-hidden="true" className="size-3.5" />
}

export function NoticeItem({ appendix }: { appendix: Appendix }) {
  const { t } = useTranslation()
  const icon = ICONS[appendix.kind] ?? Info
  return (
    <div className="rounded border border-dashed bg-muted/40 p-2">
      <div className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
        <NoticeIcon icon={icon} />
        {t(`agent:appendix.${appendix.kind}`)}
      </div>
      <pre className="mt-1 text-xs whitespace-pre-wrap">{appendix.text}</pre>
    </div>
  )
}
