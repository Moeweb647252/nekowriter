import { QueryClient } from "@tanstack/react-query"
import { describe, expect, it } from "vitest"

import { queryKeys } from "../queries/keys"
import { invalidateFor } from "./invalidation"

describe("invalidateFor", () => {
  it("marks every project list stale when a project changes", async () => {
    const queryClient = new QueryClient()
    queryClient.setQueryData(queryKeys.projects.list("active"), [])
    queryClient.setQueryData(queryKeys.projects.list("deleted"), [])
    queryClient.setQueryData(queryKeys.health, {})

    await invalidateFor(queryClient, {
      type: "project_changed",
      projectId: "0190f5c8-0000-7000-8000-000000000001",
    })

    const state = (key: readonly unknown[]) => queryClient.getQueryState(key)?.isInvalidated
    expect(state(queryKeys.projects.list("active"))).toBe(true)
    expect(state(queryKeys.projects.list("deleted"))).toBe(true)
    expect(state(queryKeys.health)).toBe(false)
  })
})
