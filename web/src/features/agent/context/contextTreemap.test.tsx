import { render, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

import type { ContextBreakdownDto } from "@/api/gen/ContextBreakdownDto"

import { ContextTreemap } from "./ContextTreemap"

const view: ContextBreakdownDto = {
  session_id: 1, seq: null, context_window: 1000, max_output_tokens: 100, rollover_at: 750,
  estimate: 1000, measured: null, cached: null, scale: 1, calibrated: false, pending_calls: 0,
  groups: [{
    key: "author", estimate: 1000, parts: [{ key: "message", name: null, estimate: 1000, items: [{ seq: 1, call_index: null, estimate: 1000, excerpt: "hello" }], rest: null }],
  }],
  requests: [],
}

afterEach(() => { vi.unstubAllGlobals() })

describe("ContextTreemap", () => {
  it("uses the measured width and stretches the SVG instead of letterboxing", async () => {
    class Observer {
      private readonly callback: ResizeObserverCallback
      constructor(callback: ResizeObserverCallback) { this.callback = callback }
      observe() { this.callback([{ contentRect: { width: 700 } } as ResizeObserverEntry], this as unknown as ResizeObserver) }
      disconnect() {}
      unobserve() {}
    }
    vi.stubGlobal("ResizeObserver", Observer)
    const { container } = render(<ContextTreemap view={view} zoom={{ group: null, part: null }} onZoom={vi.fn()} onReset={vi.fn()} />)
    await waitFor(() => expect(container.querySelector("svg")).toHaveAttribute("viewBox", "0 0 700 420"))
    expect(container.querySelector("svg")).toHaveAttribute("preserveAspectRatio", "none")
  })
})
