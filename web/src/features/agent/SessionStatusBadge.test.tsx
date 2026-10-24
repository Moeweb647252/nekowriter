/**
 * The session status badge's retry state (spec/04 §6, spec/10 §2.8).
 *
 * `SessionDto.retrying` is in-flight transport state: the session stays `running` while the
 * provider is retried, so the badge is the only place the author learns that the agent is waiting
 * rather than hung. The component test lives in its own `.tsx` file because `agent.test.ts` holds
 * pure logic and cannot contain JSX.
 *
 * The shared test setup only initializes the `common` and `library` namespaces, so this file
 * initializes the real `agent` resources itself — the point is to assert the actual locale
 * strings, not the i18n key names.
 */
import { render } from "@testing-library/react"
import i18n from "i18next"
import { describe, expect, it } from "vitest"

import enAgent from "@/i18n/locales/en/agent.json"
import { SessionStatusBadge } from "@/features/agent/SessionStatusBadge"

if (!i18n.hasResourceBundle("en", "agent")) {
  i18n.addResourceBundle("en", "agent", enAgent, true, true)
}

describe("status badge retry state", () => {
  it("shows the retry state ahead of the plain status", () => {
    // spec/04 §6: "the UI can show 'Provider overloaded — retrying in 8 s'". The retry outranks
    // the status label: both are true, but the retry is the actionable information.
    const badge = render(
      <SessionStatusBadge
        status="running"
        retrying={{ attempt: 1, wait_seconds: 8, reason: "provider overloaded" }}
      />,
    )
    expect(badge.getByText("Retrying in 8 s")).toBeTruthy()
    expect(badge.queryByText("Running")).toBeNull()
  })

  it("names the wait in the label", () => {
    const badge = render(
      <SessionStatusBadge
        status="running"
        retrying={{ attempt: 2, wait_seconds: 30, reason: "rate limited" }}
      />,
    )
    expect(badge.container.textContent ?? "").toContain("30")
  })

  it("falls back to the status when no retry is in flight", () => {
    const badge = render(<SessionStatusBadge status="running" />)
    expect(badge.getByText("Running")).toBeTruthy()
  })

  it("still ranks pausing above a retry, because the author's action is the newer fact", () => {
    const badge = render(
      <SessionStatusBadge
        status="running"
        pausing
        retrying={{ attempt: 1, wait_seconds: 8, reason: "provider overloaded" }}
      />,
    )
    expect(badge.getByText("Pausing…")).toBeTruthy()
  })
})
