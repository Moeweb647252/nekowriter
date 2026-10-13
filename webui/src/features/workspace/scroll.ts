import { type RefObject, createContext } from "react"

/** The element the workspace's center scrolls in. */
export const ScrollContainerContext = createContext<RefObject<HTMLElement | null>>({
  current: null,
})
