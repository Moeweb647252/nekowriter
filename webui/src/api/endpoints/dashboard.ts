import { getJson } from "../client"
import type { DashboardView } from "../generated/DashboardView"
import type { ProjectId } from "../generated/ProjectId"

export const dashboardApi = {
  /** The dashboard over the buckets between `boundaries`, for one project or all. */
  get: (boundaries: readonly number[], project: ProjectId | null) =>
    getJson<DashboardView>("/api/dashboard", {
      boundaries: boundaries.join(","),
      project: project ?? undefined,
    }),
}
