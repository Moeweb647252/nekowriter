import { fireEvent, render, screen } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

import type { ContextBreakdownDto } from "@/api/gen/ContextBreakdownDto"

import { RequestStrip } from "./RequestStrip"

const view: ContextBreakdownDto = {
  session_id: 1, seq: null, context_window: 10000, max_output_tokens: 100, rollover_at: 8000,
  estimate: 1000, measured: null, cached: null, scale: 1, calibrated: false, pending_calls: 0, groups: [],
  requests: [
    { seq: 1, prompt_tokens: 300, cached_tokens: 0, estimated: false, estimate: 300, created_at: "2026-01-01T00:00:00Z" },
    { seq: 2, prompt_tokens: 600, cached_tokens: 0, estimated: false, estimate: 600, created_at: "2026-01-01T00:00:01Z" },
  ],
}

describe("RequestStrip", () => {
  it("selects a past column and keeps Current outlined", () => {
    const onSelect = vi.fn()
    render(<RequestStrip view={view} selectedSeq={null} onSelect={onSelect} />)
    fireEvent.click(screen.getByRole("button", { name: /Step 1/ }))
    expect(onSelect).toHaveBeenCalledWith(1)
    const current = screen.getByRole("button", { name: "Current" })
    expect(current.querySelector("span")?.className).toContain("border-dashed")
  })
})
