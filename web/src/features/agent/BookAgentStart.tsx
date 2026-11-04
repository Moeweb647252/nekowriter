/** Start a book-agent conversation (plan §3.7): first message plus review/full mode. */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Bot, SendHorizontal } from "lucide-react"

import { startBookAgent } from "@/api/endpoints/sessions"
import type { WriteMode } from "@/api/gen/WriteMode"
import { Button } from "@/components/ui/button"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"

export function BookAgentStart({
  bookId,
  onStarted,
}: {
  bookId: number
  onStarted: (sessionId: number) => void
}) {
  const { t } = useTranslation()
  const [message, setMessage] = useState("")
  const [mode, setMode] = useState<WriteMode>("review")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState(false)

  const submit = async () => {
    const text = message.trim()
    if (text === "" || busy) {
      return
    }
    setBusy(true)
    setError(false)
    try {
      const started = await startBookAgent(bookId, { message: text, write_mode: mode })
      setMessage("")
      onStarted(started.session_id)
    } catch {
      setError(true)
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="flex h-full flex-col p-4" data-testid="book-agent-start">
      <div className="flex items-center gap-2">
        <Bot className="size-5" aria-hidden="true" />
        <div>
          <h2 className="text-sm font-semibold">{t("agent:book_agent.start_title")}</h2>
          <p className="text-xs text-muted-foreground">{t("agent:book_agent.start_hint")}</p>
        </div>
      </div>
      <div className="mt-4 space-y-1">
        <Label>{t("agent:book_agent.mode")}</Label>
        <div className="flex gap-1" role="radiogroup" aria-label={t("agent:book_agent.mode")}>
          {(["review", "full"] as const).map((value) => (
            <Button
              key={value}
              type="button"
              size="sm"
              variant={mode === value ? "default" : "outline"}
              role="radio"
              aria-checked={mode === value}
              disabled={busy}
              onClick={() => setMode(value)}
            >
              {t(`agent:book_agent.mode_${value}`)}
            </Button>
          ))}
        </div>
        <p className="text-xs text-muted-foreground">
          {t(`agent:book_agent.mode_${mode}_hint`)}
        </p>
      </div>
      <Textarea
        className="mt-4 min-h-28 resize-none"
        value={message}
        disabled={busy}
        placeholder={t("agent:book_agent.message_placeholder")}
        aria-label={t("agent:book_agent.message_label")}
        onChange={(event) => setMessage(event.target.value)}
        onKeyDown={(event) => {
          if (event.nativeEvent.isComposing) return
          if (event.key === "Enter" && !event.shiftKey) {
            event.preventDefault()
            void submit()
          }
        }}
      />
      {error ? <p className="mt-1 text-xs text-destructive">{t("agent:book_agent.start_failed")}</p> : null}
      <div className="mt-auto flex justify-end pt-3">
        <Button size="sm" disabled={busy || message.trim() === ""} onClick={() => void submit()}>
          <SendHorizontal aria-hidden="true" />
          {t("agent:book_agent.start")}
        </Button>
      </div>
    </div>
  )
}

/** The two-state review/full control, usable while the session runs (B4). */
export function WriteModeControl({
  mode,
  busy,
  disabled,
  onChange,
}: {
  mode: WriteMode | null | undefined
  busy: boolean
  disabled: boolean
  onChange: (mode: WriteMode) => void
}) {
  const { t } = useTranslation()
  return (
    <div className="flex shrink-0 items-center gap-2 border-b px-3 py-2">
      <span className="text-xs font-medium">{t("agent:book_agent.mode")}</span>
      <div className="flex gap-1" role="radiogroup" aria-label={t("agent:book_agent.mode")}>
        {(["review", "full"] as const).map((value) => (
          <Button
            key={value}
            size="sm"
            variant={mode === value ? "default" : "outline"}
            role="radio"
            aria-checked={mode === value}
            disabled={busy || disabled}
            onClick={() => onChange(value)}
          >
            {t(`agent:book_agent.mode_${value}`)}
          </Button>
        ))}
      </div>
      <span className="text-xs text-muted-foreground">
        {t(`agent:book_agent.mode_${mode ?? "review"}_hint`)}
      </span>
    </div>
  )
}
