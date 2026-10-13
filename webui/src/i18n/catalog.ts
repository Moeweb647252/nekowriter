/**
 * The shape every translation must have: the same keys as the English catalog, with
 * strings at the leaves. A translation that misses or adds a key fails type checking.
 */
export type Catalog<T> = {
  readonly [K in keyof T]: T[K] extends string ? string : Catalog<T[K]>
}
