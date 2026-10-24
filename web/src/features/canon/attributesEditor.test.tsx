/**
 * The entity attributes grid saves on blur (PROJECT §8.8, spec/06 §3).
 *
 * Every save writes one change-log row, so a patch per keystroke would fill the entity's history
 * with half-typed values. Typing must stay local; blur (or Enter) commits once.
 */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeAll, describe, expect, it, vi } from "vitest"
import i18n from "i18next"

import enCanon from "@/i18n/locales/en/canon.json"
import { AttributesEditor } from "@/features/canon/EntityDetail"

beforeAll(() => {
  i18n.addResourceBundle("en", "canon", enCanon, true, true)
})

describe("AttributesEditor", () => {
  it("saves once, on blur, not on every keystroke", async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(<AttributesEditor attributes={{ realm: "Qi" }} onChange={onChange} />)

    const input = screen.getByDisplayValue("Qi")
    await user.clear(input)
    await user.type(input, "Foundation")
    expect(
      onChange,
      "typing is not a save: every PATCH is a canon change row",
    ).not.toHaveBeenCalled()

    await user.tab()
    expect(onChange).toHaveBeenCalledTimes(1)
    expect(onChange).toHaveBeenCalledWith({ realm: "Foundation" })
  })

  it("does not save when the field is left unchanged", async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(<AttributesEditor attributes={{ realm: "Qi" }} onChange={onChange} />)
    await user.click(screen.getByDisplayValue("Qi"))
    await user.tab()
    expect(onChange).not.toHaveBeenCalled()
  })
})
