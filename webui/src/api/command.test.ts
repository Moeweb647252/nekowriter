import { renderHook } from "@testing-library/react"
import { describe, expect, it } from "vitest"

import { useCommandIds } from "./command"

describe("useCommandIds", () => {
  it("reuses the ID while the same input is retried", () => {
    const { result } = renderHook(() => useCommandIds())
    const first = result.current.idFor({ name: "Rain" })
    expect(result.current.idFor({ name: "Rain" })).toBe(first)
  })

  it("uses a new ID for different input", () => {
    const { result } = renderHook(() => useCommandIds())
    const first = result.current.idFor({ name: "Rain" })
    expect(result.current.idFor({ name: "Snow" })).not.toBe(first)
  })

  it("uses a new ID after a success, even for the same input", () => {
    const { result } = renderHook(() => useCommandIds())
    const first = result.current.idFor({ name: "Rain" })
    result.current.succeeded()
    expect(result.current.idFor({ name: "Rain" })).not.toBe(first)
  })
})
