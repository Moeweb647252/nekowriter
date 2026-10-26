/**
 * The `request_review` form (PROJECT §8.7, spec/11 §5.3, task-41).
 *
 * The author reported 「通过点了没用」: every row opened already approved, so the first click on
 * 「通过」 changed neither the markup nor the payload and read as a dead button. The suite was green
 * while that shipped, so these tests assert the **DOM** after each click (the row's recorded verdict,
 * both buttons' `aria-pressed` and selected styling, the feedback box, the submit button's enabled
 * state) instead of only checking that a handler fired — an assertion that a handler fired is exactly
 * what the green suite had.
 */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { describe, expect, it, vi } from "vitest"

import {
  ReviewRequestForm,
  REVIEW_ITEMS,
  type ReviewItem,
} from "@/features/conception/ReviewRequestForm"

const ALL_ITEMS: ReviewItem[] = [...REVIEW_ITEMS]

/** A form over the given items, with a plain submit spy. */
function renderForm(items: ReviewItem[] = ALL_ITEMS, onSubmit = vi.fn()) {
  render(
    <ReviewRequestForm message="请审阅" items={items} pending={false} onSubmit={onSubmit} />,
  )
  return onSubmit
}

/** The row element of one item — the unit whose state the author can see. */
function row(item: ReviewItem): HTMLElement {
  const element = document.querySelector<HTMLElement>(`[data-review-item="${item}"]`)
  if (element === null) {
    throw new Error(`row ${item} is not rendered`)
  }
  return element
}

/** The two verdict buttons of one item, in their rendered order. */
function verdictButtons(item: ReviewItem) {
  const buttons = Array.from(row(item).querySelectorAll<HTMLButtonElement>("button"))
  const approve = buttons.find((button) => button.textContent?.includes("Approve"))
  const needsChanges = buttons.find((button) => button.textContent?.includes("Needs changes"))
  if (approve === undefined || needsChanges === undefined) {
    throw new Error(`row ${item} does not have both verdict buttons`)
  }
  return { approve, needsChanges }
}

/** The feedback textarea of one item, when it is visible. */
function feedbackBox(item: ReviewItem): HTMLTextAreaElement | null {
  return row(item).querySelector<HTMLTextAreaElement>("textarea")
}

const submitButton = () => screen.getByRole("button", { name: "Submit review" })

describe("ReviewRequestForm — a fresh form has answered nothing", () => {
  it("opens with every item unset, so no verdict looks already chosen", () => {
    renderForm()
    for (const item of ALL_ITEMS) {
      // The regression guard for the reported bug: the old form initialised `approved: true`, so
      // every row rendered the dark 「通过」 button at mount and the first click could not change it.
      expect(row(item).dataset.verdict).toBe("unset")
      const { approve, needsChanges } = verdictButtons(item)
      expect(approve.getAttribute("aria-pressed")).toBe("false")
      expect(needsChanges.getAttribute("aria-pressed")).toBe("false")
      // Neither button carries the selected (filled) variant.
      expect(approve.dataset.variant).toBe("outline")
      expect(needsChanges.dataset.variant).toBe("outline")
      // …and no feedback box is open before asking for changes.
      expect(feedbackBox(item)).toBeNull()
    }
  })

  it("keeps Submit review unavailable and says how many items are left", () => {
    renderForm()
    expect(submitButton()).toBeDisabled()
    expect(screen.getByRole("status")).toHaveTextContent("5 items still need a verdict")
  })

  it("does not consider the review submittable just because nothing was rejected", async () => {
    // The old payload pre-approved every item, so the author could submit without reading anything.
    // Submitting now needs a verdict per row; a disabled button alone is not enough, so the
    // affordance must also be missing: the form never calls `onSubmit` on its own.
    const onSubmit = renderForm()
    await userEvent.click(submitButton())
    expect(onSubmit).not.toHaveBeenCalled()
  })
})

describe("ReviewRequestForm — every click is observable", () => {
  it("moves a fresh row from unset to approved on the first click", async () => {
    renderForm()
    const { approve, needsChanges } = verdictButtons("outline")

    await userEvent.click(approve)

    // The row now says so (the pill), both buttons report the state to assistive tech, and the
    // selected one takes the filled variant — the change the author did not get before.
    expect(row("outline").dataset.verdict).toBe("approved")
    expect(approve.getAttribute("aria-pressed")).toBe("true")
    expect(approve.dataset.variant).toBe("default")
    expect(needsChanges.getAttribute("aria-pressed")).toBe("false")
    expect(needsChanges.dataset.variant).toBe("outline")
    expect(screen.getByText("Approved")).toBeInTheDocument()
    // The untouched rows are untouched, and the count of outstanding items moved.
    expect(row("world").dataset.verdict).toBe("unset")
    expect(screen.getByRole("status")).toHaveTextContent("4 items still need a verdict")
  })

  it("reveals the feedback box on Needs changes and hides it again on Approve", async () => {
    renderForm()
    const { approve, needsChanges } = verdictButtons("outline")

    await userEvent.click(needsChanges)
    expect(row("outline").dataset.verdict).toBe("needs_changes")
    expect(needsChanges.getAttribute("aria-pressed")).toBe("true")
    expect(feedbackBox("outline")).not.toBeNull()
    await userEvent.type(feedbackBox("outline") as HTMLTextAreaElement, "conflict is thin")

    await userEvent.click(approve)
    expect(row("outline").dataset.verdict).toBe("approved")
    expect(feedbackBox("outline")).toBeNull()
    expect(approve.getAttribute("aria-pressed")).toBe("true")

    // Switching back brings the same draft back: the toggle is a verdict change, not a reset.
    await userEvent.click(needsChanges)
    expect(feedbackBox("outline")).toHaveValue("conflict is thin")
  })

  it("releases a selected verdict back to unset instead of silently ignoring the click", async () => {
    renderForm()
    const { approve } = verdictButtons("outline")

    await userEvent.click(approve)
    await userEvent.click(approve)

    // `aria-pressed` is a toggle: pressing it again releases it. Without this the second click
    // would be the original bug again (same state, same payload).
    expect(row("outline").dataset.verdict).toBe("unset")
    expect(approve.getAttribute("aria-pressed")).toBe("false")
    expect(approve.dataset.variant).toBe("outline")
  })

  it("moves the verdict between the two buttons on alternating clicks", async () => {
    renderForm()
    const { approve, needsChanges } = verdictButtons("outline")

    for (const [click, verdict, selected] of [
      [approve, "approved", approve],
      [needsChanges, "needs_changes", needsChanges],
      [approve, "approved", approve],
      [needsChanges, "needs_changes", needsChanges],
    ] as const) {
      await userEvent.click(click)
      expect(row("outline").dataset.verdict).toBe(verdict)
      expect(selected.getAttribute("aria-pressed")).toBe("true")
    }
  })
})

describe("ReviewRequestForm — Approve all is a choice the author makes", () => {
  it("approves every present item and enables Submit review", async () => {
    renderForm()
    await userEvent.click(screen.getByRole("button", { name: "Approve all" }))

    for (const item of ALL_ITEMS) {
      expect(row(item).dataset.verdict).toBe("approved")
      expect(verdictButtons(item).approve.getAttribute("aria-pressed")).toBe("true")
    }
    expect(submitButton()).toBeEnabled()
    expect(screen.queryByRole("status")).toBeNull()
  })

  it("only covers the items the studio shows", async () => {
    const items: ReviewItem[] = ["outline", "world", "protagonist"]
    renderForm(items)
    await userEvent.click(screen.getByRole("button", { name: "Approve all" }))
    expect(submitButton()).toBeEnabled()
    expect(screen.queryByRole("status")).not.toBeInTheDocument()
  })

  it("leaves a rejection as a complete verdict, so Submit review stays available", async () => {
    renderForm()
    await userEvent.click(screen.getByRole("button", { name: "Approve all" }))
    await userEvent.click(verdictButtons("world").needsChanges)

    // "Needs changes" answers the review just as "Approve" does; the form must not treat a
    // rejection as an unanswered item.
    expect(row("world").dataset.verdict).toBe("needs_changes")
    expect(submitButton()).toBeEnabled()
    expect(screen.queryByRole("status")).not.toBeInTheDocument()
  })

  it("returns to a disabled Submit review when a verdict is released back to unset", async () => {
    renderForm()
    await userEvent.click(screen.getByRole("button", { name: "Approve all" }))
    // Clicking the selected verdict again releases it (see the toggle tests above).
    await userEvent.click(verdictButtons("world").approve)

    expect(row("world").dataset.verdict).toBe("unset")
    expect(submitButton()).toBeDisabled()
    expect(screen.getByRole("status")).toHaveTextContent("1 item still needs a verdict")
  })

  it("is disabled once everything is approved, so it cannot be a no-op", async () => {
    renderForm()
    const approveAll = screen.getByRole("button", { name: "Approve all" })
    expect(approveAll).toBeEnabled()
    await userEvent.click(approveAll)
    expect(approveAll).toBeDisabled()
  })
})

describe("ReviewRequestForm — the submitted payload is the final state", () => {
  /** Fills one row: verdict, plus feedback for a rejection. */
  async function decide(item: ReviewItem, verdict: "approve" | "needs", feedback?: string) {
    await userEvent.click(verdictButtons(item)[verdict === "approve" ? "approve" : "needsChanges"])
    if (feedback !== undefined) {
      await userEvent.type(feedbackBox(item) as HTMLTextAreaElement, feedback)
    }
  }

  /** The general comment box, by its visible label. */
  const generalComment = () => screen.getByRole("textbox", { name: "General comment" })

  it("sends the final verdict after the author changes their mind", async () => {
    const onSubmit = renderForm(["outline", "world", "characters"])
    const { approve, needsChanges } = verdictButtons("outline")

    // approve → needs changes → approve, with a draft typed in between.
    await userEvent.click(approve)
    await userEvent.click(needsChanges)
    await userEvent.type(feedbackBox("outline") as HTMLTextAreaElement, "  conflict is thin ")
    await userEvent.click(approve)
    await decide("world", "needs", "more factions")
    await decide("characters", "approve")
    await userEvent.type(generalComment(), " 方向可以 ")

    expect(submitButton()).toBeEnabled()
    await userEvent.click(submitButton())

    expect(onSubmit).toHaveBeenCalledTimes(1)
    expect(onSubmit.mock.calls[0][0]).toEqual({
      docs: {
        outline: { approved: true },
        world: { approved: false, feedback: "more factions" },
        characters: { approved: true },
      },
      general: "方向可以",
    })
  })

  it("omits feedback that was typed and then abandoned by approving", async () => {
    const onSubmit = renderForm(["outline"])
    await decide("outline", "needs", "too short")
    // Approving hides the box; its text must not ride along with `approved: true`.
    await userEvent.click(verdictButtons("outline").approve)
    await userEvent.click(submitButton())
    expect(onSubmit.mock.calls[0][0]).toEqual({
      docs: { outline: { approved: true } },
    })
  })

  it("omits the general comment when it is blank", async () => {
    const onSubmit = renderForm(["outline"])
    await decide("outline", "approve")
    await userEvent.type(generalComment(), "   ")
    await userEvent.click(submitButton())
    expect(onSubmit.mock.calls[0][0]).toEqual({
      docs: { outline: { approved: true } },
    })
  })
})
