import { describe, expect, it } from "vitest"

import type { ReviewIssue } from "@/api/gen/ReviewIssue"
import type { ReviewReportDto } from "@/api/gen/ReviewReportDto"
import {
  defaultTicked,
  firstPid,
  reportIsCurrent,
  tickedIssues,
} from "@/features/chapter/reviewUtils"

function issue(pids: string[]): ReviewIssue {
  return {
    severity: "minor",
    category: "prose",
    pids,
    quote: "quote",
    problem: "problem",
    suggestion: "suggestion",
  }
}

function report(contentVersion: number): ReviewReportDto {
  return {
    id: 1,
    chapter_id: 2,
    content_version: contentVersion,
    session_id: 3,
    attempt: 1,
    verdict: "revise",
    summary: "summary",
    issues: [issue(["p1"]), issue(["p2", "p3"])],
    feedback: "feedback",
    user_comment: "",
    created_at: "2026-01-01T00:00:00Z",
  }
}

describe("reviewUtils", () => {
  it("tells a current report from one for an earlier version", () => {
    expect(reportIsCurrent(7, report(7))).toBe(true)
    expect(reportIsCurrent(8, report(7))).toBe(false)
  })

  it("selects exactly the ticked issues, in the report's order", () => {
    const issues = [issue(["p1"]), issue(["p2"]), issue(["p3"])]
    expect(tickedIssues(issues, new Set([2, 0]))).toEqual([issues[0], issues[2]])
    expect(tickedIssues(issues, new Set())).toEqual([])
  })

  it("drops out-of-range ticks instead of failing the request", () => {
    const issues = [issue(["p1"])]
    expect(tickedIssues(issues, new Set([0, 5]))).toEqual([issues[0]])
  })

  it("ticks every issue by default so the author can untick what does not apply", () => {
    expect([...defaultTicked(3)]).toEqual([0, 1, 2])
    expect(defaultTicked(0).size).toBe(0)
  })

  it("uses the first pid for the jump-to-paragraph affordance", () => {
    expect(firstPid(issue(["p9", "p10"]))).toBe("p9")
    expect(firstPid(issue([]))).toBeUndefined()
  })
})
