import { describe, expect, it, vi } from "vitest"

import type { BookShell } from "@/features/workspace/shell"
import { focusSessionInAgentPanel } from "@/features/chapter/focusSession"

describe("focusSessionInAgentPanel", () => {
  it("reopens a collapsed agent panel", () => {
    const expandAgentPanel = vi.fn()
    const shell = { agentPanelCollapsed: true, expandAgentPanel } as unknown as BookShell
    focusSessionInAgentPanel(shell)
    expect(expandAgentPanel).toHaveBeenCalledTimes(1)
  })
})
