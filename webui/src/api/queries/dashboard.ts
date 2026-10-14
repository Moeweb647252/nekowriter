import { useQuery } from "@tanstack/react-query"

import { dashboardApi } from "../endpoints/dashboard"
import type { ProjectId } from "../generated/ProjectId"
import { queryKeys } from "./keys"

export function useDashboard(boundaries: readonly number[], project: ProjectId | null) {
  return useQuery({
    queryKey: queryKeys.dashboard.view(boundaries, project),
    queryFn: () => dashboardApi.get(boundaries, project),
  })
}
