import i18n from "i18next"
import { beforeAll, describe, expect, it } from "vitest"

import enCanon from "@/i18n/locales/en/canon.json"

import { conditionSentence } from "@/features/canon/goalUtils"

beforeAll(() => {
  i18n.addResourceBundle("en", "canon", enCanon, true, true)
})

describe("goal condition sentences", () => {
  it("lowercases an entity status in the middle of the sentence", () => {
    const t = i18n.getFixedT("en", "canon")
    expect(
      conditionSentence(t, {
        condition: { kind: "entity_status_is", entity_id: 1, status: "destroyed" },
        satisfied: false,
        broken: false,
        target_label: "Jade Pendant",
      }),
    ).toBe("Waiting for Jade Pendant to be destroyed")
  })
})
