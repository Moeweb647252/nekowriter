/** Appendix classification shared by the stream and its tool cards (D-165). */
import type { Appendix } from "@/api/gen/Appendix"

/**
 * True when an `inbox` appendix carries the author's own steering message.
 *
 * Rows sealed before `inbox_kind` existed omit it; they are author messages by construction (the
 * only inbox rows at that time), so they keep the lifted-row behaviour. A runtime `notice` stays
 * inside the tool card as a `NoticeItem`.
 */
export function isAuthorInbox(appendix: Appendix): boolean {
  return (
    appendix.kind === "inbox" &&
    (appendix.inbox_kind === undefined ||
      appendix.inbox_kind === null ||
      appendix.inbox_kind === "user_message")
  )
}
