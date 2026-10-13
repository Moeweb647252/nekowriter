import { useMutation, useQuery } from "@tanstack/react-query"

import { systemApi } from "../endpoints/system"
import { queryKeys } from "./keys"

export function useHealth() {
  return useQuery({ queryKey: queryKeys.health, queryFn: systemApi.health })
}

export function useShutdown() {
  return useMutation({ mutationFn: systemApi.shutdown })
}
