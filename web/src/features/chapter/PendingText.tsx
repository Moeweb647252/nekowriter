/**
 * The draft pane's merged view of a held chapter write (PROJECT §8.5, spec/11 §5.2, D-141).
 *
 * When a write tool holds, the pane shows the document the author is being asked to accept: the
 * confirmed draft with the staged content merged in, so the decision is made on the *final shape*
 * rather than on an increment. Paragraphs the staged write adds or rewrites carry the waiting
 * status tint — they are the part that does not exist yet (spec/11 §7's `--color-status-waiting`,
 * the same token `HoldBar` uses for a held call).
 *
 * The text is deliberately **not editable** here. It is not what the draft holds: a click would
 * edit a paragraph whose text the author is not looking at, and the save would bump the chapter's
 * version, which is exactly what makes a staged write stale (spec/03 §5.2). The pane returns to the
 * normal editable document once the decision is made. The waiting tint is the only marker; the
 * author asked for the explanatory note above the text to be removed.
 */

import type { PendingParagraph } from "@/features/chapter/pendingWrite"

export type PendingTextProps = {
  paragraphs: PendingParagraph[]
  /** The book's writing language is unspaced (CJK): paragraphs are not separated by blank lines. */
  unspaced: boolean
}

export function PendingText({ paragraphs, unspaced }: PendingTextProps) {
  return (
    <div>
      <div className={unspaced ? "prose-cjk" : "prose-latin"}>
        {paragraphs.map((paragraph) => (
          <p
            key={paragraph.pid}
            data-pid={paragraph.pid}
            data-kind={paragraph.kind}
            title={paragraph.pid}
            className={paragraph.kind === "added" ? "rounded-sm bg-status-waiting/15" : "rounded-sm"}
          >
            {paragraph.kind === "changed" && paragraph.highlightFrom !== undefined ? (
              <>
                {paragraph.text.slice(0, paragraph.highlightFrom)}
                <span className="bg-status-waiting/30">
                  {paragraph.text.slice(paragraph.highlightFrom)}
                </span>
              </>
            ) : (
              paragraph.text
            )}
          </p>
        ))}
      </div>
    </div>
  )
}
