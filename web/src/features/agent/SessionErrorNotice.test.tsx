/**
 * Session error localization (spec/10 §4): a failed session's stored error code must render as an
 * author-facing message, and an unknown code falls back to the server's message.
 */
import { render, screen } from "@testing-library/react"
import { describe, expect, it } from "vitest"

import type { SessionError } from "@/api/gen/SessionError"
import { SessionErrorNotice, sessionErrorText } from "@/features/agent/SessionErrorNotice"
import i18n from "@/i18n"

function error(code: string, message = "the stored english message"): SessionError {
  return { code, message, retryable: false, at: "2026-01-01T00:00:00Z" }
}

describe("session error localization", () => {
  it("renders the localized message for a known session error code", () => {
    render(<SessionErrorNotice error={error("provider.refusal")} />)
    expect(
      screen.getByText(/The model declined this request/),
    ).toBeInTheDocument()
  })

  it("renders nothing without an error", () => {
    const { container } = render(<SessionErrorNotice error={null} />)
    expect(container).toBeEmptyDOMElement()
  })

  it("falls back to the stored message for an unknown code", () => {
    render(<SessionErrorNotice error={error("future.unknown_code")} />)
    expect(screen.getByText("the stored english message")).toBeInTheDocument()
  })

  it("resolves every session error code in both locales", () => {
    // spec/11 §9: these keys are composed from data, so this is the assert that catches a
    // missing one. `i18n/locales.test.ts` checks the raw tables; this checks the resolver.
    const codes = [
      "provider.unavailable",
      "provider.auth",
      "provider.bad_request",
      "provider.context_overflow",
      "provider.refusal",
      "agent.tool_error_streak",
      "agent.no_tool_call",
      "model.missing_provider",
      "internal",
    ]
    for (const language of ["en", "zh-CN"] as const) {
      const t = i18n.getFixedT(language)
      for (const code of codes) {
        const text = sessionErrorText(t, error(code, code))
        expect(text, `${language} errors:${code}`).not.toBe(code)
        expect(text, `${language} errors:${code}`).toBeTruthy()
      }
    }
  })
})
