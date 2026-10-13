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

  it("marks the changed version's workspace and the project lists stale", async () => {
    const queryClient = new QueryClient()
    const changed = "0190f5c8-0000-7000-8000-000000000002"
    const other = "0190f5c8-0000-7000-8000-000000000009"
    queryClient.setQueryData(queryKeys.workspaces.version(changed), {})
    queryClient.setQueryData(queryKeys.workspaces.version(other), {})
    queryClient.setQueryData(queryKeys.projects.list("active"), [])

    await invalidateFor(queryClient, {
      type: "version_changed",
      projectId: "0190f5c8-0000-7000-8000-000000000001",
      versionId: changed,
    })

    const state = (key: readonly unknown[]) => queryClient.getQueryState(key)?.isInvalidated
    expect(state(queryKeys.workspaces.version(changed))).toBe(true)
    expect(state(queryKeys.workspaces.version(other))).toBe(false)
    expect(state(queryKeys.projects.list("active"))).toBe(true)
  })
})
