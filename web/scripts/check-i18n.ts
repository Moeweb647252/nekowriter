/**
 * i18n parity and coverage check (spec/11 §6, §9).
 *
 * Fails when:
 *  - `en` and `zh-CN` key sets differ per namespace;
 *  - a string-literal key in `t("…")` / `i18nKey="…"` under `src/` is missing from `en`.
 */
import { readFileSync, readdirSync, statSync } from "node:fs"
import { join, relative } from "node:path"

const root = new URL("..", import.meta.url).pathname
const localesDir = join(root, "src/i18n/locales")
const srcDir = join(root, "src")

type Json = Record<string, unknown>

function flatten(value: Json, prefix = ""): string[] {
  const keys: string[] = []
  for (const [key, child] of Object.entries(value)) {
    const path = prefix === "" ? key : `${prefix}.${key}`
    if (child !== null && typeof child === "object" && !Array.isArray(child)) {
      keys.push(...flatten(child as Json, path))
    } else {
      keys.push(path)
    }
  }
  return keys
}

function loadLocale(language: string): Map<string, string[]> {
  const dir = join(localesDir, language)
  const namespaces = new Map<string, string[]>()
  for (const file of readdirSync(dir)) {
    if (!file.endsWith(".json")) {
      continue
    }
    const namespace = file.slice(0, -".json".length)
    const parsed = JSON.parse(readFileSync(join(dir, file), "utf8")) as Json
    namespaces.set(namespace, flatten(parsed))
  }
  return namespaces
}

function walk(dir: string): string[] {
  const files: string[] = []
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry)
    if (statSync(path).isDirectory()) {
      if (entry === "gen" || entry === "locales" || entry === "node_modules") {
        continue
      }
      files.push(...walk(path))
    } else if (/\.(ts|tsx)$/.test(entry)) {
      files.push(path)
    }
  }
  return files
}

const problems: string[] = []

const en = loadLocale("en")
const zh = loadLocale("zh-CN")

const enNamespaces = [...en.keys()].sort()
const zhNamespaces = [...zh.keys()].sort()
if (enNamespaces.join(",") !== zhNamespaces.join(",")) {
  problems.push(`namespace sets differ: en=[${enNamespaces}] zh-CN=[${zhNamespaces}]`)
}

for (const namespace of enNamespaces) {
  if (!zh.has(namespace)) {
    continue
  }
  const enKeys = new Set(en.get(namespace) ?? [])
  const zhKeys = new Set(zh.get(namespace) ?? [])
  for (const key of enKeys) {
    if (!zhKeys.has(key)) {
      problems.push(`missing in zh-CN: ${namespace}:${key}`)
    }
  }
  for (const key of zhKeys) {
    if (!enKeys.has(key)) {
      problems.push(`missing in en: ${namespace}:${key}`)
    }
  }
}

// Every literal key used in code must exist in `en`.
const allEnKeys = new Set<string>()
for (const [namespace, keys] of en) {
  for (const key of keys) {
    allEnKeys.add(`${namespace}:${key}`)
  }
}

const keyPattern =
  /\b(?:t|i18nKey)\(?\s*["']([a-zA-Z0-9_.:-]+)["']|\bi18nKey=["']([^"']+)["']/g
for (const file of walk(srcDir)) {
  const text = readFileSync(file, "utf8")
  for (const match of text.matchAll(keyPattern)) {
    const key = match[1] ?? match[2]
    if (!key || key.includes("${")) {
      continue
    }
    const full = key.includes(":") ? key : `common:${key}`
    if (!allEnKeys.has(full)) {
      problems.push(`missing key in en: ${full} (${relative(root, file)})`)
    }
  }
}

if (problems.length > 0) {
  console.error("i18n check failed:")
  for (const problem of problems) {
    console.error(`  - ${problem}`)
  }
  process.exit(1)
}

console.log(`i18n OK: ${enNamespaces.length} namespaces, ${allEnKeys.size} keys in both locales`)
