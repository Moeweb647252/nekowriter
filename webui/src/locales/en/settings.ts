import type { Theme } from "@/app/theme"

export default {
  title: "Settings",
  appearance: {
    title: "Appearance",
    language: "Interface language",
    languageHint: "The language of menus and messages. Each work has its own writing language.",
    theme: "Theme",
    themes: {
      system: "Match the system",
      light: "Light",
      dark: "Dark",
    } satisfies Record<Theme, string>,
  },
  model: {
    title: "Model service",
    description:
      "NekoWriter writes with a Chat Completions-compatible service. Works use the default model unless they choose their own.",
    baseUrl: "API root",
    baseUrlHint: "Including the version prefix, for example https://api.openai.com/v1",
    apiKey: "API key",
    apiKeyStored: "A key is stored. Enter a new one to replace it.",
    apiKeyNone: "Leave empty for services that need no key.",
    clearKey: "Remove the stored key",
    keepKey: "Keep the stored key",
    keyWillClear: "The stored key is removed when you save.",
    defaultModel: "Default model",
    noDefaultModel: "Without a default model, every work needs its own.",
    useDefaultModel: "Set a default model",
    save: "Save",
    saved: "Saved the model settings.",
  },
  embedding: {
    title: "Embeddings",
    description:
      "Semantic knowledge search needs an embedding model. Without one, the knowledge base still works, but cannot be searched by meaning.",
    baseUrlHint:
      "An OpenAI-compatible service with an embeddings endpoint, including the version prefix.",
    model: "Embedding model",
    useModel: "Set an embedding model",
    noModel: "Without an embedding model, entries are kept but not indexed.",
    dimensions: "Dimensions",
    dimensionsHint:
      "The size of the vectors the model returns. Changing the model or this size indexes every entry again.",
    save: "Save",
    saved: "Saved the embedding settings.",
    test: "Test embeddings",
    passed: "Connected. A {{dimensions}}-dimensional vector came back in {{seconds}} s.",
  },
  profile: {
    modelId: "Model ID",
    contextWindow: "Context window (tokens)",
    maxOutputTokens: "Reserved for the answer (tokens)",
    supportsThinking: "Supports thinking",
    supportsThinkingHint: "The model accepts the reasoning_effort parameter.",
    reportsStreamUsage: "Reports token usage",
    reportsStreamUsageHint: "The service sends token usage at the end of a streamed answer.",
  },
  test: {
    run: "Test connection",
    running: "Testing…",
    passed: "Connected. The test answer took {{seconds}} s.",
    usageMissing:
      "The service did not report token usage. Turn off “Reports token usage” or check the service.",
  },
  diagnostics: {
    title: "This installation",
    version: "Version",
    dataDir: "Data directory",
    logDir: "Log directory",
  },
  service: {
    title: "Local service",
    body: "NekoWriter keeps running after you close this page. Exit it here when you are done.",
    exit: "Exit NekoWriter",
    confirmTitle: "Exit NekoWriter?",
    confirmBody:
      "Work in progress stops and can be continued after the next start. All data stays in the data directory.",
    confirm: "Exit",
  },
} as const
