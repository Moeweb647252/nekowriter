import { useEffect, useRef, useState } from "react"
import { Quote } from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  rangeFromSelection,
  selectionInside,
  type PickedTextRange,
} from "@/lib/citations"

interface CitableTextProps {
  /** 与后端修订逐字一致的原文。 */
  text: string
  disabled?: boolean
  /** 加入一条引用；调用方负责把区间包装成正式文档或候选引用。 */
  onCite: (range: PickedTextRange) => void
  className?: string
}

/**
 * 只读正文块，允许把选中文字加入待提交意见的引用。
 *
 * 文本渲染为单个文本节点，因此浏览器选区偏移可以精确映射回底层正文；按段落拆
 * DOM 会在段落边界丢掉换行，引用区间就不再能被后端逐字复核。
 */
export function CitableText({
  text,
  disabled = false,
  onCite,
  className,
}: CitableTextProps) {
  const containerRef = useRef<HTMLDivElement | null>(null)
  const [hasSelection, setHasSelection] = useState(false)

  useEffect(() => {
    function update() {
      const container = containerRef.current
      const selection = window.getSelection()
      setHasSelection(
        container !== null &&
          selection !== null &&
          selectionInside(container, selection)
      )
    }
    document.addEventListener("selectionchange", update)
    return () => {
      document.removeEventListener("selectionchange", update)
    }
  }, [text])

  const selectable = hasSelection && !disabled

  function handleCite() {
    const container = containerRef.current
    const selection = window.getSelection()
    if (!container || !selection) return
    const range = rangeFromSelection(container, selection)
    if (!range) return
    onCite(range)
    selection.removeAllRanges()
    setHasSelection(false)
  }

  return (
    <div className="relative">
      {selectable && (
        <Button
          type="button"
          size="xs"
          className="absolute -top-3 right-0 z-10 shadow-sm"
          // 阻止默认行为，避免按下按钮时浏览器先清空选区。
          onMouseDown={(event) => event.preventDefault()}
          onClick={handleCite}
        >
          <Quote aria-hidden="true" />
          引用选中文字
        </Button>
      )}
      <div ref={containerRef} className={className}>
        {text}
      </div>
    </div>
  )
}
