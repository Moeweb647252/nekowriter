/** Settings › Web search (spec/07 §6, spec/02 §4.1): the provider, its key and the Test button. */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Search } from "lucide-react"

import {
  useAppSettings,
  usePatchAppSettings,
  useTestSearch,
} from "@/api/hooks/app-settings"
import { useErrorToast } from "@/components/common/ErrorToast"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import type { SearchProvider } from "@/api/gen/SearchProvider"

/** The most results `web_search` can ever return (spec/05 §7). */
const MAX_RESULTS = 10

export default function WebSearchPage() {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const settings = useAppSettings()
  const patch = usePatchAppSettings()
  const test = useTestSearch()

  // The author edits a local draft; Save writes it and Test runs the saved configuration.
  const [provider, setProvider] = useState<SearchProvider | null>(null)
  const [tavilyKey, setTavilyKey] = useState<string | null>(null)
  const [searxngUrl, setSearxngUrl] = useState<string | null>(null)
  const [maxResults, setMaxResults] = useState<number | null>(null)
  const [query, setQuery] = useState("")

  if (settings.isPending || settings.data === undefined) {
    return <Skeleton className="h-64" />
  }

  const saved = settings.data.search
  const current: SearchProvider = provider ?? saved.provider
  const key = tavilyKey ?? saved.tavily_api_key ?? ""
  const baseUrl = searxngUrl ?? saved.searxng_base_url ?? ""
  const results = maxResults ?? saved.max_results
  const dirty =
    current !== saved.provider ||
    (tavilyKey !== null && tavilyKey !== (saved.tavily_api_key ?? "")) ||
    (searxngUrl !== null && searxngUrl !== (saved.searxng_base_url ?? "")) ||
    results !== saved.max_results

  const save = () => {
    patch.mutate(
      {
        search: {
          provider: current,
          // An untouched masked key is sent back as stored: the server treats a mask as "keep"
          // (spec/10 §2.1), so the author can change the provider without retyping a secret.
          tavily_api_key: current === "tavily" ? key : undefined,
          searxng_base_url: current === "searxng" ? baseUrl : undefined,
          max_results: results,
        },
      },
      {
        onSuccess: () => {
          setTavilyKey(null)
          setSearxngUrl(null)
          setMaxResults(null)
          setProvider(null)
        },
        onError: showError,
      },
    )
  }

  return (
    <div className="max-w-2xl space-y-6">
      <div>
        <h2 className="text-base font-semibold">{t("settings:search")}</h2>
        <p className="text-sm text-muted-foreground">{t("settings:search_hint")}</p>
      </div>

      <div className="flex items-center gap-3">
        <Label htmlFor="search-provider" className="w-40 shrink-0">
          {t("settings:search_provider")}
        </Label>
        <Select
          value={current}
          onValueChange={(value) => setProvider(value as SearchProvider)}
        >
          <SelectTrigger id="search-provider" className="flex-1">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="none">{t("settings:search_provider_none")}</SelectItem>
            <SelectItem value="tavily">Tavily</SelectItem>
            <SelectItem value="searxng">SearXNG</SelectItem>
          </SelectContent>
        </Select>
      </div>

      {current === "tavily" ? (
        <div className="flex items-start gap-3">
          <Label htmlFor="search-key" className="mt-1.5 w-40 shrink-0">
            {t("settings:search_api_key")}
          </Label>
          <div className="flex-1 space-y-1">
            <Input
              id="search-key"
              type="password"
              value={key}
              placeholder={t("settings:search_api_key_placeholder")}
              onChange={(event) => setTavilyKey(event.target.value)}
            />
            <p className="text-xs text-muted-foreground">{t("settings:search_api_key_hint")}</p>
          </div>
        </div>
      ) : null}

      {current === "searxng" ? (
        <div className="flex items-start gap-3">
          <Label htmlFor="search-url" className="mt-1.5 w-40 shrink-0">
            {t("settings:search_base_url")}
          </Label>
          <div className="flex-1 space-y-1">
            <Input
              id="search-url"
              value={baseUrl}
              placeholder="https://searxng.example"
              onChange={(event) => setSearxngUrl(event.target.value)}
            />
            <p className="text-xs text-muted-foreground">{t("settings:search_base_url_hint")}</p>
          </div>
        </div>
      ) : null}

      <div className="flex items-center gap-3">
        <Label htmlFor="search-results" className="w-40 shrink-0">
          {t("settings:search_max_results")}
        </Label>
        <Input
          id="search-results"
          type="number"
          min={1}
          max={MAX_RESULTS}
          className="w-24"
          value={results}
          onChange={(event) => {
            const next = Number.parseInt(event.target.value, 10)
            setMaxResults(Number.isNaN(next) ? 1 : Math.min(MAX_RESULTS, Math.max(1, next)))
          }}
        />
        <span className="text-xs text-muted-foreground">
          {t("settings:search_max_results_hint", { max: MAX_RESULTS })}
        </span>
      </div>

      <div className="flex items-center gap-2">
        <Button size="sm" onClick={save} disabled={!dirty || patch.isPending}>
          {patch.isPending ? <Spinner /> : null}
          {t("common:save")}
        </Button>
        <Button
          size="sm"
          variant="outline"
          onClick={() => test.mutate({ query }, { onError: showError })}
          disabled={
            test.isPending ||
            query.trim() === "" ||
            saved.provider === "none" ||
            dirty
          }
          title={dirty ? t("settings:search_test_save_first") : undefined}
        >
          {test.isPending ? <Spinner /> : <Search className="size-3.5" aria-hidden="true" />}
          {t("settings:search_test")}
        </Button>
        <Input
          className="max-w-xs"
          value={query}
          placeholder={t("settings:search_test_placeholder")}
          onChange={(event) => setQuery(event.target.value)}
          aria-label={t("settings:search_test_placeholder")}
        />
      </div>

      {saved.provider === "none" ? (
        <p className="text-xs text-muted-foreground">{t("settings:search_not_configured_hint")}</p>
      ) : null}

      {test.data !== undefined ? (
        <div className="space-y-2">
          <h3 className="text-sm font-medium">
            {t("settings:search_test_results", { count: test.data.hits.length })}
          </h3>
          {test.data.hits.length === 0 ? (
            <p className="text-sm text-muted-foreground">{t("settings:search_test_empty")}</p>
          ) : (
            <ul className="space-y-2">
              {test.data.hits.map((hit) => (
                <li key={hit.url} className="rounded-md border p-2">
                  <a
                    className="text-sm font-medium text-primary underline-offset-2 hover:underline"
                    href={hit.url}
                    target="_blank"
                    rel="noreferrer"
                  >
                    {hit.title}
                  </a>
                  <p className="mt-1 line-clamp-2 text-xs text-muted-foreground">{hit.snippet}</p>
                  {hit.published !== undefined ? (
                    <p className="mt-1 text-[11px] text-muted-foreground">{hit.published}</p>
                  ) : null}
                </li>
              ))}
            </ul>
          )}
        </div>
      ) : null}
    </div>
  )
}
