import { expect, it } from "vitest"

import "@/i18n"
import i18n from "@/i18n"
import type { ContextPartDto } from "@/api/gen/ContextPartDto"

import { partLabel } from "./labels"

function part(name: string): ContextPartDto {
  return { key: "block", name, estimate: 1, items: [], rest: null }
}

const subdivisions = [
  ["block.settings.outline", "大纲"],
  ["block.settings.characters", "角色"],
  ["block.settings.world", "世界观"],
  ["block.settings.protagonist", "主角"],
  ["block.memory.recent", "近期"],
  ["block.memory.entities", "实体"],
  ["block.memory.facts", "事实"],
  ["block.memory.foreshadowing", "伏笔"],
] as const

it("labels the eight subdivisions in zh-CN", () => {
  const t = i18n.getFixedT("zh-CN")
  for (const [key, label] of subdivisions) {
    expect(partLabel(t, part(key), "kickoff", 0)).toBe(label)
  }
})

it("keeps the old parent labels only as fallbacks", () => {
  const t = i18n.getFixedT("zh-CN")
  expect(partLabel(t, part("block.settings"), "kickoff", 0)).toBe("设定材料")
  expect(partLabel(t, part("block.memory"), "kickoff", 0)).toBe("记忆")
  expect(partLabel(t, part("block.settings.outline"), "kickoff", 0)).toBe("大纲")
  expect(partLabel(t, part("block.memory.facts"), "kickoff", 0)).toBe("事实")
})
