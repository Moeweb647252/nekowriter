/** Pure helpers of the review tab (spec/08 §1.2, §3), kept testable without a DOM. */
import type { ReviewIssue } from "@/api/gen/ReviewIssue"
import type { ReviewReportDto } from "@/api/gen/ReviewReportDto"

/**
 * A report is current when its `content_version` equals the chapter's (spec/08 §1.2); an older
 * report is shown as "for an earlier version".
 */
export function reportIsCurrent(contentVersion: number, report: ReviewReportDto): boolean {
  return report.content_version === contentVersion
}

/**
 * The issues the author ticked, in the report's order. Zero-based indexes into `issues`
 * (spec/08 §1.1 `issue_indexes`); out-of-range indexes are dropped so a stale dialog can never
 * make the request fail.
 */
export function tickedIssues(
  issues: readonly ReviewIssue[],
  ticked: ReadonlySet<number>,
): ReviewIssue[] {
  return issues.filter((_issue, index) => ticked.has(index))
}

/** The issue indexes a fresh dialog ticks: every issue, because the report asks for all of them. */
export function defaultTicked(issueCount: number): Set<number> {
  return new Set(Array.from({ length: issueCount }, (_unused, index) => index))
}

/** The first pid of an issue, for the jump-to-paragraph affordance. */
export function firstPid(issue: ReviewIssue): string | undefined {
  return issue.pids[0]
}
