import { getJson, postJson } from "../client"
import type { EventCursorView } from "../generated/EventCursorView"
import type { HealthView } from "../generated/HealthView"
import type { ShutdownView } from "../generated/ShutdownView"

export const systemApi = {
  health: () => getJson<HealthView>("/api/health"),
  shutdown: () => postJson<ShutdownView>("/api/system/shutdown", {}),
  eventCursor: () => getJson<EventCursorView>("/api/events/cursor"),
}
