/**
 * Settings › Embeddings shows the resolved model cache path read-only (PROJECT §8.12, spec/01
 * §6, D-097). If the path is not rendered the author cannot tell where fastembed downloads to.
 */
import { render, screen } from "@testing-library/react"
import i18n from "i18next"
import { beforeAll, describe, expect, it, vi } from "vitest"

import enSettings from "@/i18n/locales/en/settings.json"

import EmbeddingsPage from "./EmbeddingsPage"

// The shared test setup registers the namespaces the M0-M6 pages use; this page also needs
// `settings`. Registering it here keeps the shared file untouched.
beforeAll(() => {
  if (!i18n.hasResourceBundle("en", "settings")) {
    i18n.addResourceBundle("en", "settings", enSettings)
  }
  if (!i18n.options.ns?.includes("settings")) {
    i18n.options.ns = [...(i18n.options.ns as string[]), "settings"]
  }
})

vi.mock("@/api/hooks/app-settings", () => ({
  useAppSettings: () => ({
    isPending: false,
    data: { embedding: { model: "multilingual-e5-small" } },
  }),
  usePatchAppSettings: () => ({ mutate: vi.fn(), isPending: false }),
}))

vi.mock("@/api/hooks/knowledge", () => ({
  useEmbeddingStatus: () => ({
    isPending: false,
    data: {
      model: "multilingual-e5-small",
      model_cache_dir: "/var/lib/nekowriter/models",
      state: "ready",
      message: null,
      pending: 2,
      failed: 0,
      done: 7,
    },
  }),
  useRebuildIndex: () => ({ mutate: vi.fn(), isPending: false }),
}))

vi.mock("@/components/common/ErrorToast", () => ({ useErrorToast: () => vi.fn() }))

describe("EmbeddingsPage", () => {
  it("shows the model cache path read-only", async () => {
    render(<EmbeddingsPage />)
    const path = await screen.findByTestId("model-cache-dir")
    expect(path).toHaveTextContent("/var/lib/nekowriter/models")
    // Read-only: no input carries the path.
    expect(screen.queryByDisplayValue("/var/lib/nekowriter/models")).not.toBeInTheDocument()
    // The label is translated in both locales (i18n parity covers the keys).
    expect(screen.getByText("Model cache")).toBeInTheDocument()
  })

  it("shows the index counts and state", async () => {
    render(<EmbeddingsPage />)
    expect(await screen.findByText(/Pending/)).toHaveTextContent("2")
    expect(screen.getByText(/Embedded/)).toHaveTextContent("7")
    expect(screen.getByText(/Failed/)).toHaveTextContent("0")
  })
})
