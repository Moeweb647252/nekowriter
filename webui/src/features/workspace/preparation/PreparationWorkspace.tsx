import { useTranslation } from "react-i18next"

import { ContentsNav } from "../ContentsNav"
import { type DocumentKey, firstOpenReview, useSelectedDocument } from "../documents"
import { DocumentView } from "../DocumentView"
import { isHalted } from "../run"
import { RunProblem, RunProgress } from "../RunDock"
import { WorkspaceLayout } from "../WorkspaceLayout"
import { BriefEditor } from "./BriefEditor"
import { PreparationPreview } from "./PreparationPreview"
import { ReviewDock } from "./ReviewDock"
import { StartDock } from "./StartDock"
import { usePreview } from "@/api/events/preview"
import type { PreparationStep } from "@/api/generated/PreparationStep"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"
import { useDraft } from "@/lib/drafts"

/** A version in preparation: brief, generation, review and revision (§6). */
export function PreparationWorkspace({
  view,
  step,
}: {
  view: WorkspaceView
  step: PreparationStep
}) {
  const { t } = useTranslation()
  const run = view.run
  const preview = usePreview(run?.id, run?.requestInFlight)
  const streaming = preview !== null && run?.status === "running"
  const brief = useDraft(`${view.version.id}.brief`, view.brief?.content ?? "")

  const fallback: DocumentKey | null =
    step === "not_started"
      ? "brief"
      : step === "reviewing"
        ? (firstOpenReview(view) ?? "background")
        : streaming
          ? null
          : "background"
  const { selected, select } = useSelectedDocument(fallback)

  const center =
    step === "not_started" && selected === "brief" ? (
      <BriefEditor view={view} brief={brief} />
    ) : selected === null && streaming ? (
      <PreparationPreview preview={preview} />
    ) : (
      <DocumentView view={view} doc={selected ?? "background"} />
    )

  const revising = view.artifacts
    .filter(({ review }) => review?.status === "rejected" || review?.status === "stale")
    .map(({ kind }) => t(($) => $.workspace.artifact[kind]))
  const dock =
    step === "not_started" ? (
      <StartDock view={view} brief={brief} />
    ) : run !== null && isHalted(run.status) ? (
      <RunProblem view={view} run={run} />
    ) : step === "reviewing" ? (
      <ReviewDock view={view} selected={selected} onSelect={select} />
    ) : run !== null ? (
      <RunProgress view={view} run={run}>
        {step === "revising"
          ? t(($) => $.workspace.preparation.revising, { documents: revising.join(", ") })
          : t(($) => $.workspace.preparation.generating)}
      </RunProgress>
    ) : null

  return (
    <WorkspaceLayout
      view={view}
      contents={
        <ContentsNav view={view} selected={selected} onSelect={select} previewing={streaming} />
      }
      center={center}
      dock={dock}
    />
  )
}
