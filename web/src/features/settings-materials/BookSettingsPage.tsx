/** Book settings (PROJECT §8.12): language, genre, granularity, pause rules, target length,
 * memory windows, auto-maintenance and the per-book agent-role model overrides (spec/03 §4 step 1:
 * `book.model_overrides[role]` → `app_settings["roles.defaults"][role]`). The knowledge scopes this
 * book's agents may read are their own page (`BookKnowledgePage`). */
import { useState } from "react"
import { Link, useParams } from "react-router"
import { useTranslation } from "react-i18next"
import { Lock, UsersRound } from "lucide-react"

import { useBook, usePatchBook } from "@/api/hooks/books"
import { useCanonCounts } from "@/api/hooks/useCanon"
import { useAppSettings } from "@/api/hooks/app-settings"
import { useModelProfiles, useProviders } from "@/api/hooks/providers"
import { useReplaceBookWritingSkills, useWritingSkills } from "@/api/hooks/skills"
import { useErrorToast } from "@/components/common/ErrorToast"
import { EmptyState } from "@/components/common/EmptyState"
import { ErrorToastInline } from "@/features/settings/ErrorToastInline"
import {
  AGENT_ROLES,
  formatModelLabel,
  roleLabelKey,
  roleSelection,
  type RoleSelection,
} from "@/features/settings/roleModels"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Skeleton } from "@/components/ui/skeleton"
import { Switch } from "@/components/ui/switch"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import type { BookSettings } from "@/api/gen/BookSettings"
import type { BookStatus } from "@/api/gen/BookStatus"
import type { Granularity } from "@/api/gen/Granularity"
import type { CanonSystems } from "@/api/gen/CanonSystems"
import type { CanonCountsDto } from "@/api/gen/CanonCountsDto"

const GRANULARITIES: Granularity[] = ["sentence", "paragraph", "free", "chapter"]

/** The value of the "use the global default" option. Model profile ids are numbers, so this can
 * never collide with one; `null` is the wire value it stands for (spec/03 §4 step 1). */
const GLOBAL_DEFAULT = "global"

const CANON_SYSTEM_KEYS: Array<keyof CanonSystems> = [
  "entities",
  "timeline",
  "memory",
  "foreshadowing",
  "goals",
]

/** The route of each disabled system's data page (View data). */
const CANON_SYSTEM_PATHS: Record<keyof CanonSystems, string> = {
  entities: "entities",
  timeline: "timeline",
  memory: "memory",
  foreshadowing: "foreshadowing",
  goals: "goals",
}

/** The retained-data sentence of one disabled switch. */
function retainedText(
  t: (key: string, options?: Record<string, unknown>) => string,
  system: keyof CanonSystems,
  counts: CanonCountsDto | undefined,
): string {
  if (counts === undefined) return t("book:canon_retained_loading")
  switch (system) {
    case "entities":
      return t("book:canon_retained.entities", {
        entities: counts.entities.entities,
        relations: counts.entities.relations,
        appearances: counts.entities.appearances,
      })
    case "timeline":
      return t("book:canon_retained.timeline", { events: counts.timeline.events })
    case "memory":
      return t("book:canon_retained.memory", {
        notes: counts.memory.notes,
        summaries: counts.memory.summaries,
      })
    case "foreshadowing":
      return t("book:canon_retained.foreshadowing", { items: counts.foreshadowing.items })
    case "goals":
      return t("book:canon_retained.goals", { nodes: counts.goals.nodes })
  }
}

/** The editable form of a book, tagged with the row version it was built from. */
type BookDraft = {
  key: string
  title: string
  genre: string
  language: string
  settings: BookSettings
  /** One entry per role: `null` means "use the global default" for that role. */
  overrides: RoleSelection
}
const LANGUAGES = ["zh-CN", "en", "ja", "ko", "fr", "de", "es", "ru"] as const

export default function BookSettingsPage() {
  const { bookId: rawBookId } = useParams()
  const bookId = Number(rawBookId)
  const { t } = useTranslation()
  const showError = useErrorToast()
  const book = useBook(bookId)
  const patch = usePatchBook(bookId)
  // The model profiles and providers the role selects offer, plus the global role defaults each
  // row compares its override against (spec/10 §2.1 `roles.defaults`).
  const appSettings = useAppSettings()
  const models = useModelProfiles()
  const providers = useProviders()
  // The writing-skill catalogue with this book's `selected` flags (design §8). Selection is its
  // own endpoint, so a checkbox toggles immediately instead of waiting for the form's Save.
  const writingSkills = useWritingSkills(bookId)
  const canonCounts = useCanonCounts(bookId)
  const replaceSkills = useReplaceBookWritingSkills(bookId)
  // One draft object keyed by the book's `updated_at`, so an SSE refresh of the query never
  // clobbers unsaved input and no effect has to copy the query into state.
  const [draft, setDraft] = useState<BookDraft | null>(null)

  if (book.isPending) {
    return <Skeleton className="m-6 h-64" />
  }
  if (book.isError || book.data === undefined) {
    return (
      <div className="p-6">
        <EmptyState title={t("common:error_title")} description={t("common:error_body")} />
      </div>
    )
  }

  const saved: BookDraft = {
    key: book.data.updated_at,
    title: book.data.title,
    genre: book.data.genre,
    language: book.data.writing_language,
    settings: book.data.settings,
    overrides: roleSelection(book.data.model_overrides),
  }
  const current = draft !== null && draft.key === saved.key ? draft : saved
  const setTitle = (title: string) => setDraft({ ...current, title })
  const setGenre = (genre: string) => setDraft({ ...current, genre })
  const setLanguage = (language: string) => setDraft({ ...current, language })
  const update = (patchSettings: Partial<BookSettings>) =>
    setDraft({ ...current, settings: { ...current.settings, ...patchSettings } })
  // A role picker only edits the draft: the author reviews the whole form and the one Save button
  // sends it, so an SSE refresh cannot overwrite an unsaved choice.
  const setOverride = (role: (typeof AGENT_ROLES)[number], value: string) =>
    setDraft({
      ...current,
      overrides: { ...current.overrides, [role]: value === GLOBAL_DEFAULT ? null : Number(value) },
    })
  const settings = current.settings
  const overrides = current.overrides
  const title = current.title
  const genre = current.genre
  const language = current.language
  const status: BookStatus = book.data.status

  const save = () => {
    patch.mutate(
      {
        title: title.trim(),
        genre,
        writing_language: language,
        settings,
        // The whole object every time: a patch that carried only the edited role would silently
        // clear the siblings, because the server replaces `books.model_overrides` as a unit.
        model_overrides: overrides,
      },
      { onError: showError },
    )
  }

  const modelList = models.data ?? []
  const providerList = providers.data ?? []
  const globalDefaults = roleSelection(appSettings.data?.roles_defaults)
  const modelsPending = appSettings.isPending || models.isPending || providers.isPending
  const modelsFailed = appSettings.isError || models.isError || providers.isError
  const skillRows = writingSkills.data ?? []
  const selectedSkills = skillRows.filter((skill) => skill.selected)
  // The design's running total counts characters, not UTF-8 bytes (design §6.4).
  const selectedCharacters = selectedSkills.reduce(
    (total, skill) => total + Array.from(skill.body).length,
    0,
  )
  const toggleSkill = (skillId: number, checked: boolean) => {
    const next = new Set(selectedSkills.map((skill) => skill.id))
    if (checked) {
      next.add(skillId)
    } else {
      next.delete(skillId)
    }
    replaceSkills.mutate(
      { skill_ids: [...next].sort((left, right) => left - right) },
      { onError: showError },
    )
  }
  // A conceiving or archived book's roles are not the author's to pick here: the conception flow
  // chooses them until Finalize, and an archived book is not being worked on (spec/08 §5).
  const rolesLocked = status !== "active"
  const lockedHint =
    status === "conceiving"
      ? t("book:model_overrides_needs_active")
      : status === "archived"
        ? t("book:model_overrides_archived")
        : null

  return (
    <div className="mx-auto max-w-2xl space-y-6 p-6">
      <header className="flex items-end justify-between">
        <h1 className="text-xl font-semibold">{t("book:book_settings")}</h1>
        <Button size="sm" disabled={patch.isPending} onClick={save}>
          {patch.isPending ? t("common:saving") : t("common:save")}
        </Button>
      </header>

      <section className="space-y-3">
        <div className="space-y-1.5">
          <Label htmlFor="settings-title">{t("library:book_title")}</Label>
          <Input id="settings-title" value={title} onChange={(event) => setTitle(event.target.value)} />
        </div>
        <div className="grid grid-cols-2 gap-3">
          <div className="space-y-1.5">
            <Label htmlFor="settings-genre">{t("library:book_genre")}</Label>
            <Input
              id="settings-genre"
              value={genre}
              onChange={(event) => setGenre(event.target.value)}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="settings-language">{t("library:book_language")}</Label>
            <Select value={language} onValueChange={setLanguage}>
              <SelectTrigger id="settings-language">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {LANGUAGES.map((code) => (
                  <SelectItem key={code} value={code}>
                    {t(`common:language_code.${code}`, { defaultValue: code })}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>
      </section>

      <section className="space-y-3">
        <h2 className="text-sm font-medium">{t("book:writer_defaults")}</h2>
        <div className="space-y-1.5">
          <Label htmlFor="settings-granularity">{t("book:granularity")}</Label>
          <Select
            value={settings.granularity}
            onValueChange={(value) => update({ granularity: value as Granularity })}
          >
            <SelectTrigger id="settings-granularity">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {GRANULARITIES.map((granularity) => (
                <SelectItem key={granularity} value={granularity}>
                  {t(`chapter:granularity.${granularity}`, { defaultValue: granularity })}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="flex items-center justify-between">
          <Label htmlFor="settings-auto-pause">{t("book:auto_pause")}</Label>
          <Switch
            id="settings-auto-pause"
            checked={settings.auto_pause}
            onCheckedChange={(checked) => update({ auto_pause: checked })}
          />
        </div>
        <div className="space-y-1">
          <div className="flex items-center justify-between">
            <Label htmlFor="settings-pause-outline">{t("book:pause_after_outline")}</Label>
            <Switch
              id="settings-pause-outline"
              checked={settings.pause_after_outline}
              onCheckedChange={(checked) => update({ pause_after_outline: checked })}
            />
          </div>
          <p className="text-xs text-muted-foreground">{t("book:pause_after_outline_hint")}</p>
        </div>
        <div className="flex items-center justify-between">
          <Label htmlFor="settings-auto-maintenance">{t("book:auto_maintenance")}</Label>
          <Switch
            id="settings-auto-maintenance"
            checked={settings.auto_maintenance}
            onCheckedChange={(checked) => update({ auto_maintenance: checked })}
          />
        </div>
      </section>

      <section className="space-y-3">
        <h2 className="text-sm font-medium">{t("book:target_length")}</h2>
        <div className="grid grid-cols-2 gap-3">
          <div className="space-y-1.5">
            <Label htmlFor="settings-target-min">{t("book:target_words_min")}</Label>
            <Input
              id="settings-target-min"
              type="number"
              value={settings.target_words_min}
              onChange={(event) =>
                update({ target_words_min: Number(event.target.value) || 0 })
              }
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="settings-target-max">{t("book:target_words_max")}</Label>
            <Input
              id="settings-target-max"
              type="number"
              value={settings.target_words_max}
              onChange={(event) =>
                update({ target_words_max: Number(event.target.value) || 0 })
              }
            />
          </div>
        </div>
      </section>

      <section className="space-y-3">
        <h2 className="text-sm font-medium">{t("book:canon_systems")}</h2>
        <p className="text-sm text-muted-foreground">{t("book:canon_systems_hint")}</p>
        <div className="space-y-3">
          {CANON_SYSTEM_KEYS.map((system) => {
            const enabled = settings.canon_systems[system]
            return (
              <div key={system} className="space-y-1" data-testid={`canon-system-${system}`}>
                <div className="flex items-center justify-between gap-3">
                  <Label htmlFor={`settings-canon-${system}`}>
                    {t(`book:canon_system.${system}`)}
                  </Label>
                  <Switch
                    id={`settings-canon-${system}`}
                    checked={enabled}
                    onCheckedChange={(checked) =>
                      update({
                        canon_systems: { ...settings.canon_systems, [system]: checked },
                      })
                    }
                  />
                </div>
                <p className="text-xs text-muted-foreground">
                  {t(`book:canon_system_hint.${system}`)}
                </p>
                {enabled ? null : (
                  <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                    <span>{retainedText(t, system, canonCounts.data)}</span>
                    <Link
                      className="underline"
                      to={`/books/${bookId}/canon/${CANON_SYSTEM_PATHS[system]}`}
                    >
                      {t("book:canon_view_data")}
                    </Link>
                  </div>
                )}
              </div>
            )
          })}
        </div>
        <p className="text-xs text-muted-foreground">{t("book:canon_systems_footnote")}</p>
        {!settings.canon_systems.memory && settings.recent_chapter_count === 0 ? (
          <p className="text-xs text-destructive">{t("book:memory_off_recent_warning")}</p>
        ) : null}
      </section>

      <section className="space-y-3">
        <h2 className="text-sm font-medium">{t("book:memory_windows")}</h2>
        <div className="grid grid-cols-3 gap-3">
          <div className="space-y-1.5">
            <Label htmlFor="settings-recent">{t("book:recent_summary_count")}</Label>
            <Input
              id="settings-recent"
              type="number"
              value={settings.recent_summary_count}
              disabled={!settings.canon_systems.memory}
              onChange={(event) =>
                update({ recent_summary_count: Number(event.target.value) || 0 })
              }
            />
            {!settings.canon_systems.memory ? (
              <p className="text-xs text-muted-foreground">{t("book:recent_summary_disabled")}</p>
            ) : null}
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="settings-midterm">{t("book:midterm_window")}</Label>
            <Input
              id="settings-midterm"
              type="number"
              value={settings.midterm_window}
              disabled={!settings.canon_systems.entities}
              onChange={(event) => update({ midterm_window: Number(event.target.value) || 0 })}
            />
            {!settings.canon_systems.entities ? (
              <p className="text-xs text-muted-foreground">{t("book:midterm_disabled")}</p>
            ) : null}
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="settings-recent-chapters">{t("book:recent_chapter_count")}</Label>
            <Input
              id="settings-recent-chapters"
              type="number"
              value={settings.recent_chapter_count}
              onChange={(event) =>
                update({ recent_chapter_count: Number(event.target.value) || 0 })
              }
            />
          </div>
        </div>
        <p className="text-xs text-muted-foreground">{t("book:recent_chapter_count_hint")}</p>
      </section>

      {/* Writing skills (design §8): a checkbox list; the catalogue page owns create/edit/delete.
          Toggling calls PUT immediately — the note tells the author it applies to new sessions. */}
      <section className="space-y-3">
        <h2 className="text-sm font-medium">{t("book:writing_skills")}</h2>
        <p className="text-sm text-muted-foreground">{t("book:writing_skills_hint")}</p>
        <p className="text-xs text-muted-foreground">
          {t("book:writing_skills_count", {
            count: selectedSkills.length,
            characters: selectedCharacters,
          })}
          {" · "}
          {t("book:writing_skills_applies")}
        </p>
        {selectedCharacters > 8000 ? (
          <p className="text-xs text-destructive">{t("book:writing_skills_warn")}</p>
        ) : null}
        {writingSkills.isError ? (
          <ErrorToastInline />
        ) : writingSkills.isPending ? (
          <Skeleton className="h-20" />
        ) : skillRows.length === 0 ? (
          <EmptyState
            title={t("book:writing_skills")}
            description={t("book:writing_skills_empty")}
          />
        ) : (
          <div className="space-y-2">
            {skillRows.map((skill) => (
              <label
                key={skill.id}
                className="flex items-start gap-3 rounded-md border p-3"
                data-testid={`book-skill-${skill.id}`}
              >
                <Checkbox
                  checked={skill.selected}
                  disabled={replaceSkills.isPending}
                  onCheckedChange={(checked) => toggleSkill(skill.id, checked === true)}
                  aria-label={skill.name}
                />
                <span className="min-w-0">
                  <span className="block text-sm font-medium">
                    {skill.name}
                    {skill.author === "builtin" ? (
                      <span className="ml-2 rounded bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground">
                        {t("skills:built_in")}
                      </span>
                    ) : null}
                  </span>
                  {skill.description === "" ? null : (
                    <span className="block text-xs text-muted-foreground">
                      {skill.description}
                    </span>
                  )}
                </span>
              </label>
            ))}
          </div>
        )}
      </section>

      {/* Model overrides per role (PROJECT §8.12): a book may run one or more roles on a different
          model than the global default; the server resolves the book's choice first. */}
      <section className="space-y-3">
        <div className="space-y-1">
          <h2 className="flex items-center gap-1.5 text-sm font-medium">
            <UsersRound className="size-3.5" aria-hidden="true" />
            {t("book:model_overrides")}
          </h2>
          <p className="text-sm text-muted-foreground">{t("book:model_overrides_hint")}</p>
        </div>

        {lockedHint === null ? null : (
          <p className="flex items-center gap-1.5 text-sm text-muted-foreground">
            <Lock className="size-3.5" aria-hidden="true" />
            {lockedHint}
          </p>
        )}

        {modelsFailed ? (
          <ErrorToastInline />
        ) : modelsPending ? (
          <Skeleton className="h-40" />
        ) : modelList.length === 0 ? (
          <EmptyState title={t("settings:no_models")} description={t("settings:no_models_hint")} />
        ) : (
          <div className="space-y-3">
            {AGENT_ROLES.map((role) => {
              const override = overrides[role]
              const globalDefault = globalDefaults[role]
              // The planner falls back to the writer when it has no assignment of its own
              // (spec/03 §4, D-146); its hint names the model it will actually use.
              const plannerFollows =
                role === "planner" ? (overrides.writer ?? globalDefaults.writer) : null
              // An override whose profile was deleted after the pick: it keeps its value (the
              // server would still resolve it) but says so instead of rendering a blank control.
              const missing =
                override !== null && !modelList.some((model) => model.id === override)
              return (
                <div
                  key={role}
                  data-testid={`model-override-row-${role}`}
                  className="grid grid-cols-[8rem_1fr] items-center gap-x-3 gap-y-1"
                >
                  <Label htmlFor={`book-role-${role}`}>{t(roleLabelKey(role))}</Label>
                  <Select
                    value={override === null ? GLOBAL_DEFAULT : String(override)}
                    onValueChange={(value) => setOverride(role, value)}
                    disabled={rolesLocked}
                  >
                    <SelectTrigger id={`book-role-${role}`} className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value={GLOBAL_DEFAULT}>
                        {t("book:model_use_global_default")}
                      </SelectItem>
                      {modelList.map((model) => (
                        <SelectItem key={model.id} value={String(model.id)}>
                          {formatModelLabel(t, modelList, providerList, model.id)}
                        </SelectItem>
                      ))}
                      {missing ? (
                        <SelectItem value={String(override)}>
                          {t("settings:model_missing")}
                        </SelectItem>
                      ) : null}
                    </SelectContent>
                  </Select>
                  <span aria-hidden="true" />
                  <p className="text-xs text-muted-foreground">
                    {role === "planner" ? (
                      globalDefault !== null ? (
                        t("book:model_planner_default", {
                          model: formatModelLabel(t, modelList, providerList, globalDefault),
                        })
                      ) : plannerFollows !== null ? (
                        t("book:model_planner_follows_writer", {
                          model: formatModelLabel(t, modelList, providerList, plannerFollows),
                        })
                      ) : (
                        t("book:model_global_default_none")
                      )
                    ) : globalDefault === null ? (
                      t("book:model_global_default_none")
                    ) : (
                      t("book:model_global_default", {
                        model: formatModelLabel(t, modelList, providerList, globalDefault),
                      })
                    )}
                  </p>
                  {missing ? (
                    <>
                      <span aria-hidden="true" />
                      <p className="text-xs text-destructive">
                        {t("book:model_override_missing")}
                      </p>
                    </>
                  ) : null}
                </div>
              )
            })}
          </div>
        )}
      </section>
    </div>
  )
}
