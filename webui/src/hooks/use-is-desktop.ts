import { useEffect, useState } from "react"

const desktopQuery = "(min-width: 1024px)"

function matchesDesktop() {
  return (
    typeof window !== "undefined" && window.matchMedia(desktopQuery).matches
  )
}

/** 与 Tailwind 的 `lg` 断点一致，保证带副作用的桌面和窄屏布局不会同时挂载。 */
export function useIsDesktop() {
  const [isDesktop, setIsDesktop] = useState(matchesDesktop)

  useEffect(() => {
    const media = window.matchMedia(desktopQuery)
    const update = () => setIsDesktop(media.matches)
    update()
    media.addEventListener("change", update)
    return () => media.removeEventListener("change", update)
  }, [])

  return isDesktop
}
