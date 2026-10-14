import { getJson } from "../client"
import type { EnginesView } from "../generated/EnginesView"

export const enginesApi = {
  get: () => getJson<EnginesView>("/api/engines"),
}
