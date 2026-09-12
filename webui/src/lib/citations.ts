/**
 * 引用位置换算。
 *
 * 后端按 Unicode scalar value（Rust `char`）计数并逐字校验 quote；浏览器
 * Selection 的 offset 是 UTF-16 code unit。emoji 与部分中文补充平面字符在 JS 里
 * 占两个 code unit，直接把浏览器数字当字符序号会把引用落在错误位置。这里集中处理
 * 两种口径之间的换算，组件不再自行拼接偏移。
 */

/** 选中文本对应的可验证引用区间；位置单位是 Unicode scalar value。 */
export interface PickedTextRange {
  startChar: number
  endChar: number
  quote: string
}

/** 把 UTF-16 code unit 偏移换算成 Unicode scalar value 偏移。 */
export function utf16OffsetToScalar(text: string, utf16Offset: number): number {
  const target = Math.max(0, Math.min(utf16Offset, text.length))
  let scalarOffset = 0
  let consumed = 0
  for (const character of text) {
    if (consumed >= target) break
    consumed += character.length
    scalarOffset += 1
  }
  return scalarOffset
}

/** 把 Unicode scalar value 偏移还原成可直接用于 `String.prototype.slice` 的下标。 */
export function scalarOffsetToUtf16(
  text: string,
  scalarOffset: number
): number {
  if (scalarOffset <= 0) return 0
  let scalars = 0
  let consumed = 0
  for (const character of text) {
    if (scalars >= scalarOffset) break
    consumed += character.length
    scalars += 1
  }
  return consumed
}

/** 取 `[startChar, endChar)` 的原文片段。 */
export function scalarSlice(
  text: string,
  startChar: number,
  endChar: number
): string {
  const start = scalarOffsetToUtf16(text, startChar)
  const end = scalarOffsetToUtf16(text, endChar)
  return text.slice(start, end)
}

function utf16OffsetWithin(
  container: HTMLElement,
  node: Node,
  offset: number
): number {
  const range = document.createRange()
  range.setStart(container, 0)
  range.setEnd(node, offset)
  return range.toString().length
}

/** 选择是否是非空且完整落在该容器内。用于廉价的按钮启用判定。 */
export function selectionInside(
  container: HTMLElement,
  selection: Selection
): boolean {
  if (selection.rangeCount === 0 || selection.isCollapsed) return false
  const range = selection.getRangeAt(0)
  return (
    container.contains(range.startContainer) &&
    container.contains(range.endContainer)
  )
}

/**
 * 从容器内的原生 Selection 计算引用区间。
 *
 * 只在选择完全落在一个容器内时返回结果。容器必须把原文渲染为单个文本节点，
 * 否则 `Range.toString()` 不会带上块级元素之间的换行，偏移就不再对应底层正文。
 */
export function rangeFromSelection(
  container: HTMLElement,
  selection: Selection
): PickedTextRange | null {
  if (selection.rangeCount === 0 || selection.isCollapsed) return null
  const range = selection.getRangeAt(0)
  if (
    !container.contains(range.startContainer) ||
    !container.contains(range.endContainer)
  ) {
    return null
  }
  const text = container.textContent ?? ""
  const startUtf16 = utf16OffsetWithin(
    container,
    range.startContainer,
    range.startOffset
  )
  const endUtf16 = utf16OffsetWithin(
    container,
    range.endContainer,
    range.endOffset
  )
  if (endUtf16 <= startUtf16) return null
  const startChar = utf16OffsetToScalar(text, startUtf16)
  const endChar = utf16OffsetToScalar(text, endUtf16)
  if (endChar <= startChar) return null
  const quote = scalarSlice(text, startChar, endChar)
  if (!quote) return null
  return { startChar, endChar, quote }
}
