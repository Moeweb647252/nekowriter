import common from "./common"
import dashboard from "./dashboard"
import errors from "./errors"
import library from "./library"
import prompts from "./prompts"
import settings from "./settings"
import templates from "./templates"
import workspace from "./workspace"

/** The English catalog. It defines the keys every other language must provide. */
export const en = {
  common,
  dashboard,
  errors,
  library,
  prompts,
  settings,
  templates,
  workspace,
} as const

export type EnglishCatalog = typeof en

/** Every namespace of the catalog. Selectors name their namespace: `$.library.title`. */
export const NAMESPACES = [
  "common",
  "dashboard",
  "errors",
  "library",
  "prompts",
  "settings",
  "templates",
  "workspace",
] as const

// Fails to compile when a namespace is added to the catalog but not to `NAMESPACES`.
type AssertTrue<Condition extends true> = Condition
export type EveryNamespaceListed = AssertTrue<
  Exclude<keyof EnglishCatalog, (typeof NAMESPACES)[number]> extends never ? true : false
>
