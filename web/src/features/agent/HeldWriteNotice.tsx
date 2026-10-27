/**
 * The panel's pointer to the draft-pane dock (D-141).
 *
 * A held chapter write is decided where its text is: the draft pane shows the merged document the
 * write would produce and the dock under it owns approve / request changes. The panel must not
 * repeat those two mutations under a second wording — that is precisely the split the author
 * reported (「正文编写要在中间的正文草稿区进行，正文编辑时的意见提出在草稿区下方的 dock 栏里
 * 进行」) — but it must not leave the author without an explanation either: a `held` session with
 * no visible decision anywhere reads as a stuck agent.
 *
 * So the panel states what happened and links to the draft. Abandon stays in the controls menu,
 * because it is a session operation rather than a decision about this write.
 */
import { useTranslation } from "react-i18next"
import { ExternalLink } from "lucide-react"
import { Link } from "react-router"

export function HeldWriteNotice({ chapterHref }: { chapterHref?: string }) {
  const { t } = useTranslation()
  return (
    <section
      data-testid="held-write-notice"
      aria-label={t("agent:held.write_title")}
      className="shrink-0 space-y-1 border-b bg-amber-500/5 px-3 py-2"
    >
      <h2 className="text-sm font-semibold">{t("agent:held.write_title")}</h2>
      <p className="text-xs text-muted-foreground">{t("agent:held.write_body")}</p>
      {chapterHref === undefined ? null : (
        <Link
          to={chapterHref}
          className="inline-flex items-center gap-1 text-xs font-medium underline underline-offset-2"
        >
          <ExternalLink aria-hidden="true" className="size-3.5" />
          {t("agent:held.write_open_draft")}
        </Link>
      )}
    </section>
  )
}
