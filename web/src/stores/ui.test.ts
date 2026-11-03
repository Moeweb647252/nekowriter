/** The persisted UI store's manual pin flag (task-296 follow-up). */
import "@/i18n"
import { beforeEach, describe, expect, it } from "vitest"

import { useUiStore } from "@/stores/ui"

beforeEach(() => {
  useUiStore.setState({ pinnedSession: {}, pinnedSessionManual: {} })
})

describe("pinned session manual flag", () => {
  it("marks the session switcher's pick as manual and clears it with the pin", () => {
    useUiStore.getState().setPinnedSession(3, 7, true)
    expect(useUiStore.getState().pinnedSession[3]).toBe(7)
    expect(useUiStore.getState().pinnedSessionManual[3]).toBe(true)

    // Even a manual flag cannot survive clearing the pin.
    useUiStore.getState().setPinnedSession(3, null, true)
    expect(useUiStore.getState().pinnedSession[3]).toBeNull()
    expect(useUiStore.getState().pinnedSessionManual[3]).toBe(false)
  })

  it("keeps a programmatic pin automatic and the flag per book", () => {
    useUiStore.getState().setPinnedSession(3, 7)
    expect(useUiStore.getState().pinnedSession[3]).toBe(7)
    expect(useUiStore.getState().pinnedSessionManual[3]).toBe(false)

    useUiStore.getState().setPinnedSession(3, 7, true)
    useUiStore.getState().setPinnedSession(4, 8)
    expect(useUiStore.getState().pinnedSessionManual[3]).toBe(true)
    expect(useUiStore.getState().pinnedSessionManual[4]).toBe(false)
  })
})
