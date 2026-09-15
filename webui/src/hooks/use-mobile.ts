import { useSyncExternalStore } from "react"

const MOBILE_BREAKPOINT = 768
const MOBILE_QUERY = `(max-width: ${MOBILE_BREAKPOINT - 1}px)`

/**
 * 断点状态由浏览器持有，因此订阅它而不是镜像进 state：
 * 首帧就能读到真实值，也不必在 effect 里补一次同步 setState。
 */
function subscribe(onStoreChange: () => void) {
  const query = window.matchMedia(MOBILE_QUERY)
  query.addEventListener("change", onStoreChange)
  return () => query.removeEventListener("change", onStoreChange)
}

function getSnapshot() {
  return window.matchMedia(MOBILE_QUERY).matches
}

export function useIsMobile() {
  return useSyncExternalStore(subscribe, getSnapshot)
}
