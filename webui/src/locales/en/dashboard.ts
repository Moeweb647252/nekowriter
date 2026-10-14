import type { RunKind } from "@/api/generated/RunKind"
import type { DashboardRange } from "@/features/dashboard/range"

export default {
  title: "Dashboard",
  allWorks: "All works",
  work: "Work",
  range: "Time range",
  ranges: {
    today: "Today",
    week: "Last 7 days",
    month: "Last 30 days",
  } satisfies Record<DashboardRange, string>,
  jobs: {
    title: "Jobs",
    running: "Working or waiting",
    noneRunning: "Nothing is running.",
    attention: "Needs attention",
    noneAttention: "Nothing needs attention.",
    where: "{{work}}, version {{version}}",
    kind: {
      preparation: "Preparation",
      chapter_outline: "Outline",
      chapter_body: "Chapter text",
      review: "Review",
    } satisfies Record<RunKind, string>,
    reviewAgent: "The review agent: {{status}}",
  },
  words: {
    title: "Words",
    description:
      "Words added to chapter text minus words removed. Each Chinese character counts as one word.",
    net: "Net growth",
    total: "Words in all chapters now",
    chart: "Net growth over time",
  },
  usage: {
    title: "Token usage",
    description:
      "Tokens the model service reported for each request. Requests it did not report are counted apart, never as zero.",
    role: "Used for",
    roles: {
      writing: "Writing",
      review: "Review agents",
      connectionTests: "Connection tests",
      embeddings: "Embeddings",
    },
    requests: "Requests",
    input: "Input",
    cachedInput: "Cached input",
    output: "Output",
    reasoning: "Reasoning",
    known: "Known total",
    unknown: "Unknown",
    unknownNote:
      "Requests with unknown usage: {{notReported}} answered without usage, {{noAnswer}} without an answer, {{pending}} still in flight, {{invalid}} with contradictory counts.",
    chart: "Known tokens over time",
  },
}
