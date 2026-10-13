import { renderHook } from "@testing-library/react"
import { act } from "react"
import { describe, expect, it } from "vitest"

import i18n from "."
import { useErrorText } from "./errors"
import { ApiError, NetworkError } from "@/api/client"
import type { ApiErrorBody } from "@/api/generated/ApiErrorBody"
import { zhCN } from "@/locales/zh-CN"

function apiError(body: ApiErrorBody) {
  return new ApiError(422, body)
}

const tooLong = apiError({
  code: "FIELD_INVALID",
  params: { field: "novel_title", problem: { kind: "too_long", maxChars: 200 } },
  requestId: "r1",
})

describe("useErrorText", () => {
  it("describes a validation error with the field's name and the limit", () => {
    const { result } = renderHook(() => useErrorText())
    expect(result.current(tooLong)).toBe("Novel title can be at most 200 characters.")
  })

  it("describes errors in the interface language", async () => {
    await act(() => i18n.changeLanguage("zh-CN"))
    const { result } = renderHook(() => useErrorText())
    const text = result.current(tooLong)
    expect(text).toContain(zhCN.errors.field.novel_title)
    expect(text).toContain("200")
    expect(text).not.toContain("Novel title")
  })

  it("names the object a conflict concerns", () => {
    const { result } = renderHook(() => useErrorText())
    const conflict = apiError({
      code: "REVISION_CONFLICT",
      params: { resource: "project" },
      requestId: "r2",
    })
    expect(result.current(conflict)).toBe(
      "This work was changed elsewhere. The latest version is shown now; please try again.",
    )
  })

  it("adds the log reference to unexpected server failures", () => {
    const { result } = renderHook(() => useErrorText())
    const internal = apiError({ code: "INTERNAL", requestId: "0190-abc" })
    expect(result.current(internal)).toContain("Reference: 0190-abc")
  })

  it("explains when the service cannot be reached", () => {
    const { result } = renderHook(() => useErrorText())
    expect(result.current(new NetworkError("offline"))).toBe(
      "NekoWriter could not be reached. Check that it is still running.",
    )
  })
})
