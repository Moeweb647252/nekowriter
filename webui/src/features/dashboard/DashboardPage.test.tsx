import { screen, within } from "@testing-library/react"
import { HttpResponse, http } from "msw"
import { describe, expect, it } from "vitest"

import { DashboardPage } from "./DashboardPage"
import type { DashboardView } from "@/api/generated/DashboardView"
import type { UsageTotals } from "@/api/generated/UsageTotals"
import { PROJECT_ID, VERSION_ID, projectView } from "@/test/fixtures"
import { renderWithProviders } from "@/test/render"
import { server } from "@/test/server"

const NONE: UsageTotals = {
  requests: 0,
  reported: 0,
  inputTokens: 0,
  outputTokens: 0,
  cachedInputTokens: 0,
  reasoningTokens: 0,
  pending: 0,
  notReported: 0,
  noAnswer: 0,
  invalid: 0,
}

const WRITING: UsageTotals = {
  ...NONE,
  requests: 3,
  reported: 2,
  inputTokens: 1800,
  outputTokens: 600,
  cachedInputTokens: 1000,
  notReported: 1,
}

const VIEW: DashboardView = {
  usage: {
    writing: WRITING,
    review: NONE,
    connectionTests: NONE,
    buckets: [NONE, NONE, NONE, NONE, NONE, NONE, WRITING],
    projects: [{ projectId: PROJECT_ID, name: "Rain", writing: WRITING, review: NONE }],
  },
  words: {
    net: 1200,
    buckets: [0, 0, 0, 0, 0, -50, 1250],
    projects: [{ projectId: PROJECT_ID, name: "Rain", net: 1200 }],
    total: 5400,
  },
  jobs: [
    {
      runId: "0190f5c8-0000-7000-8000-000000000c01",
      kind: "chapter_body",
      status: "running",
      halt: null,
      projectId: PROJECT_ID,
      projectName: "Rain",
      versionId: VERSION_ID,
      versionNumber: 1,
      chapter: { kind: "main", number: 2, title: "The Storm" },
      reviewAgent: null,
      updatedAt: 0,
    },
  ],
  attention: [],
}

describe("DashboardPage", () => {
  it("shows jobs, word growth and token usage for the last seven days", async () => {
    const queries: URLSearchParams[] = []
    server.use(
      http.get("*/api/projects", () => HttpResponse.json([projectView()])),
      http.get("*/api/dashboard", ({ request }) => {
        queries.push(new URL(request.url).searchParams)
        return HttpResponse.json(VIEW)
      }),
    )
    renderWithProviders(<DashboardPage />)

    expect(await screen.findByText("Chapter text · Chapter 2: The Storm")).toBeInTheDocument()
    expect(screen.getByRole("link", { name: "Rain, version 1" })).toHaveAttribute(
      "href",
      `/works/${PROJECT_ID}/versions/${VERSION_ID}`,
    )
    // The net growth of the range and of the only work.
    expect(screen.getAllByText("+1,200")).toHaveLength(2)
    expect(screen.getByText("5,400")).toBeInTheDocument()
    const writing = screen.getByRole("row", { name: /^Writing/ })
    expect(within(writing).getByText("2,400")).toBeInTheDocument()
    expect(screen.getByText(/1 answered without usage, 0 without an answer/)).toBeInTheDocument()
    expect(screen.getByRole("list", { name: "Net growth over time" }).children).toHaveLength(7)

    // Seven daily buckets: eight boundaries, and no project filter.
    expect(queries[0]?.get("boundaries")?.split(",")).toHaveLength(8)
    expect(queries[0]?.has("project")).toBe(false)
  })
})
