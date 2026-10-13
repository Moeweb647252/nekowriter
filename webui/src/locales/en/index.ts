import common from "./common"
import errors from "./errors"
import library from "./library"
import settings from "./settings"

/** The English catalog. It defines the keys every other language must provide. */
export const en = { common, errors, library, settings } as const

export type EnglishCatalog = typeof en

/** Every namespace of the catalog. Selectors name their namespace: `$.library.title`. */
export const NAMESPACES = ["common", "errors", "library", "settings"] as const

// Fails to compile when a namespace is added to the catalog but not to `NAMESPACES`.
type AssertTrue<Condition extends true> = Condition
export type EveryNamespaceListed = AssertTrue<
  Exclude<keyof EnglishCatalog, (typeof NAMESPACES)[number]> extends never ? true : false
>
