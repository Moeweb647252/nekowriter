/**
 * The hold bar's Continue/Approve contract (spec/11 §5.2).
 *
 * The one thing the bar must not do is resume before the outline save has landed: the agent reads
 * the outline in the same transaction that applies the held call, so an unawaited save races it.
 */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { describe, expect, it, vi } from "vitest"

import { HoldBar } from "@/features/chapter/GhostText"

describe("HoldBar", () => {
  it("labels the text variant with its pids", () => {
    render(<HoldBar outline={false} pids={["p4", "p5"]} busy={false} onContinue={() => {}} onFeedback={() => {}} />)
    expect(screen.getByRole("group")).toHaveAccessibleName(
      "Paused after this change (p4, p5)",
    )
  })

  it("labels the outline variant", () => {
    render(<HoldBar outline pids={[]} busy={false} onContinue={() => {}} onFeedback={() => {}} />)
    expect(screen.getByRole("button", { name: "Approve outline" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Request changes" })).toBeInTheDocument()
  })

  it("calls onContinue directly, so the caller owns the save-then-resume order", async () => {
    const onContinue = vi.fn()
    render(<HoldBar outline={false} pids={[]} busy={false} onContinue={onContinue} onFeedback={() => {}} />)
    await userEvent.click(screen.getByRole("button", { name: /continue/i }))
    expect(onContinue).toHaveBeenCalledTimes(1)
  })

  it("sends trimmed feedback", async () => {
    const onFeedback = vi.fn()
    render(<HoldBar outline={false} pids={[]} busy={false} onContinue={() => {}} onFeedback={onFeedback} />)
    await userEvent.click(screen.getByRole("button", { name: /feedback/i }))
    await userEvent.type(screen.getByRole("textbox"), "  写得再快些  ")
    await userEvent.click(screen.getByRole("button", { name: /send/i }))
    expect(onFeedback).toHaveBeenCalledWith("写得再快些")
  })

  it("disables both buttons while busy", () => {
    render(<HoldBar outline={false} pids={[]} busy onContinue={() => {}} onFeedback={() => {}} />)
    expect(screen.getByRole("button", { name: /continue/i })).toBeDisabled()
  })
})
