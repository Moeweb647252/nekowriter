import { describe, expect, it } from "vitest"

import { isCompositionEvent } from "@/lib/shortcuts"

/** 按键事件只需要这三个字段，测试因此不依赖 DOM。 */
function keydown(fields: {
  isComposing?: boolean
  keyCode?: number
  target?: unknown
}) {
  return {
    isComposing: fields.isComposing ?? false,
    keyCode: fields.keyCode ?? 13,
    target: fields.target ?? null,
  } as unknown as KeyboardEvent
}

describe("输入法组合期间的按键判定", () => {
  it("组合中（Safari / Windows 输入法）的按键不算快捷键", () => {
    expect(isCompositionEvent(keydown({ isComposing: true }))).toBe(true)
  })

  // 部分浏览器在确认候选词时先给出 isComposing === false 的 keydown；只认这个字段
  // 会让一次中文输入的 Enter 触发「提交当前表单」。
  it("确认候选词那一次按键（keyCode 229）也算组合中", () => {
    expect(isCompositionEvent(keydown({ keyCode: 229 }))).toBe(true)
  })

  it("普通 Enter 不是组合事件", () => {
    expect(isCompositionEvent(keydown({ keyCode: 13 }))).toBe(false)
  })
})
