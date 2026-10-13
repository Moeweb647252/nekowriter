import { createContext, useContext } from "react"

/**
 * - `connecting`: reading the event cursor for the first time.
 * - `live`: following events.
 * - `reconnecting`: the connection dropped; it is being restored automatically.
 * - `stopped`: the service shut down; reloading the page after a restart is needed.
 */
export type ConnectionStatus = "connecting" | "live" | "reconnecting" | "stopped"

export const ConnectionContext = createContext<ConnectionStatus>("connecting")

export function useConnectionStatus(): ConnectionStatus {
  return useContext(ConnectionContext)
}
