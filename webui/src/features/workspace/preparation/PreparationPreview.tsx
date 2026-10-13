import { Allow, parse } from "partial-json"
import { useMemo } from "react"
import { useTranslation } from "react-i18next"

import { ChapterPlanList, ReadingText } from "../DocumentView"
import type { ArtifactKind } from "@/api/generated/ArtifactKind"
import type { Preview } from "@/api/generated/Preview"

/** The preparation submission's tool, whose arguments carry the documents. */
const SUBMISSION_TOOL = "submit_preparation"

const ARTIFACT_KINDS: readonly string[] = [
  "background",
  "research",
  "characters",
  "outline",
] satisfies readonly ArtifactKind[]

interface PartialSubmission {
  artifacts: { kind: ArtifactKind | null; content: string }[]
  plan: { title: string | undefined; goal: string | undefined }[]
}

function text(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined
}

function records(value: unknown): Record<string, unknown>[] {
  return Array.isArray(value)
    ? value.filter(
        (item): item is Record<string, unknown> => typeof item === "object" && item !== null,
      )
    : []
}

/**
 * Reads the submission so far from its incomplete JSON. Anything that does not have the
 * expected shape yet is left out until more of it arrives.
 */
function readSubmission(json: string): PartialSubmission {
  let value: unknown
  try {
    value = parse(json, Allow.ALL)
  } catch {
    return { artifacts: [], plan: [] }
  }
  const root = typeof value === "object" && value !== null ? (value as Record<string, unknown>) : {}
  return {
    artifacts: records(root.artifacts).map((artifact) => {
      const kind = text(artifact.kind)
      return {
        kind: kind !== undefined && ARTIFACT_KINDS.includes(kind) ? (kind as ArtifactKind) : null,
        content: text(artifact.content) ?? "",
      }
    }),
    plan: records(root.chapter_plan).map((item) => ({
      title: text(item.title),
      goal: text(item.goal),
    })),
  }
}

/** What the agent has written so far, before it submits (§4 invariant 1: not content). */
export function PreparationPreview({ preview }: { preview: Preview }) {
  const { t } = useTranslation()
  const call = preview.toolCalls.find((item) => item.name === SUBMISSION_TOOL)
  const submission = useMemo(() => readSubmission(call?.arguments ?? ""), [call?.arguments])

  return (
    <section aria-label={t(($) => $.workspace.preview.label)} className="flex flex-col gap-10">
      <p className="rounded-md border border-dashed px-3 py-2 text-sm text-muted-foreground">
        {t(($) => $.workspace.preview.note)}
      </p>
      {preview.text.trim() !== "" && (
        <div>
          <h3 className="mb-2 text-sm font-medium text-muted-foreground">
            {t(($) => $.workspace.preview.notes)}
          </h3>
          <p className="whitespace-pre-wrap text-muted-foreground">{preview.text}</p>
        </div>
      )}
      {submission.artifacts.map(({ kind, content }, index) => (
        // Artifacts stream in order; their position is stable while the answer grows.
        <article key={index} className="opacity-90">
          <h2 className="mb-6 font-heading text-2xl font-semibold">
            {kind === null ? "…" : t(($) => $.workspace.artifact[kind])}
          </h2>
          <ReadingText text={content} />
        </article>
      ))}
      {submission.plan.length > 0 && <ChapterPlanList plan={submission.plan} />}
    </section>
  )
}
