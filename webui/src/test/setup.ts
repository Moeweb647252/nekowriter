import "@testing-library/jest-dom/vitest"

import { cleanup } from "@testing-library/react"
import { afterAll, afterEach, beforeAll, vi } from "vitest"

import i18n from "@/i18n"

import { server } from "./server"

// jsdom lacks these browser APIs, which the UI components use for layout and theming.
globalThis.ResizeObserver = vi.fn(function ResizeObserver() {
  return { observe: vi.fn(), unobserve: vi.fn(), disconnect: vi.fn() }
})
window.matchMedia = (query: string) =>
  ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    addListener: vi.fn(),
    removeListener: vi.fn(),
    dispatchEvent: () => false,
  }) satisfies MediaQueryList
Element.prototype.scrollIntoView = vi.fn()
globalThis.IntersectionObserver = vi.fn(function IntersectionObserver() {
  return { observe: vi.fn(), unobserve: vi.fn(), disconnect: vi.fn(), takeRecords: () => [] }
}) as unknown as typeof IntersectionObserver

beforeAll(() => {
  server.listen({ onUnhandledFrame: "error" })
})

afterEach(async () => {
  cleanup()
  server.resetHandlers()
  localStorage.clear()
  await i18n.changeLanguage("en")
})

afterAll(() => {
  server.close()
})
