import { useMemo } from "react"
import { useTranslation } from "react-i18next"

import { ChapterPlanList, ReadingText } from "../DocumentView"
import type { ArtifactKind } from "@/api/generated/ArtifactKind"
import type { Preview } from "@/api/generated/Preview"
import { objectsOf, readPartialObject, textOf } from "@/lib/partialJson"

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

/** The submission so far; parts without the expected shape yet are left out. */
function readSubmission(json: string): PartialSubmission {
  const root = readPartialObject(json)
  return {
    artifacts: objectsOf(root.artifacts).map((artifact) => {
      const kind = textOf(artifact.kind)
      return {
        kind: kind !== undefined && ARTIFACT_KINDS.includes(kind) ? (kind as ArtifactKind) : null,
        content: textOf(artifact.content) ?? "",
      }
    }),
    plan: objectsOf(root.chapter_plan).map((item) => ({
      title: textOf(item.title),
      goal: textOf(item.goal),
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
