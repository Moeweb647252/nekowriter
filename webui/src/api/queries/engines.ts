import { useQuery } from "@tanstack/react-query"

import { enginesApi } from "../endpoints/engines"
import { queryKeys } from "./keys"

/** The registered engines change only with the build, so they are read once. */
export function useEngines() {
  return useQuery({ queryKey: queryKeys.engines, queryFn: enginesApi.get })
}
