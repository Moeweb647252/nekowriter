import { useCallback, useRef, useState } from "react"

/**
 * 同一个值在界面里的两份表示：state 供 render 读取，ref 供异步回调同步读取。
 *
 * 编辑面板需要判断「当前文本与服务端快照是否一致」，这个判断既出现在 render 里
 * （决定要不要保存草稿、按钮是否可用），也出现在 async 回调里（取回新快照、
 * 恢复草稿、提交成功）。两份表示只能通过 `commit` 一起改动，因此不会各自漂移，
 * render 也不必再读 ref。
 */
export function useLatestValue<T>(initial: T) {
  const [value, setValue] = useState(initial)
  const ref = useRef(initial)
  const commit = useCallback((next: T) => {
    ref.current = next
    setValue(next)
  }, [])
  return { value, ref, commit }
}
