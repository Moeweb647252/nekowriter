/**
 * The stored error of a session, localized (spec/02 §4 `agent_sessions.error`, spec/10 §4).
 *
 * A failed session keeps its error in the row (`SessionDto.error`), and its `code` is one of the
 * session error codes spec/10 §4 lists — `provider.refusal`, `agent.tool_error_streak`,
 * `model.missing_provider` and friends. Those codes arrive as data, so `check-i18n` cannot see
 * them; `i18n/locales.test.ts` asserts that every one of them has an `errors:` key in both
 * locales. This notice is where an author actually reads it: the status toast says only that the
 * session stopped.
 */
import type { TFunction } from "i18next"
import { useTranslation } from "react-i18next"
import { CircleAlert } from "lucide-react"

import type { SessionError } from "@/api/gen/SessionError"

/**
 * The localized text of a session error: the `errors:<code>` message, or the stored English
 * message when a newer server sends a code this build does not know (spec/10 §1's fallback rule).
 */
export function sessionErrorText(t: TFunction, error: SessionError): string {
  return t(`errors:${error.code}`, { defaultValue: error.message })
}

/** A red notice under the session controls while the session carries an error. */
export function SessionErrorNotice({ error }: { error: SessionError | null }) {
  const { t } = useTranslation()
  if (error === null) {
    return null
  }
  return (
    <div
      role="alert"
      className="flex items-start gap-2 border-b bg-destructive/10 px-3 py-2 text-xs text-destructive"
    >
      <CircleAlert aria-hidden="true" className="mt-0.5 size-3.5 shrink-0" />
      <span>{sessionErrorText(t, error)}</span>
    </div>
  )
}
