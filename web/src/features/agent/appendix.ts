/** Appendix classification shared by the stream and its tool cards (D-165). */
import type { Appendix } from "@/api/gen/Appendix"

/**
 * True when an `inbox` appendix carries the author's own steering message.
 *
 * Rows sealed before `inbox_kind` existed omit it. Both `user_message` and `notice` rows were
 * stored without a discriminator at that time, so absence is not proof of authorship: the fallback
 * keeps the common author-feedback case lifted, and an old runtime notice remains indistinguishable
 * and stays lifted too (documented in D-165). A new runtime `notice` stays in the tool card.
 */
export function isAuthorInbox(appendix: Appendix): boolean {
  return (
    appendix.kind === "inbox" &&
    (appendix.inbox_kind === undefined ||
      appendix.inbox_kind === null ||
      appendix.inbox_kind === "user_message")
  )
}
