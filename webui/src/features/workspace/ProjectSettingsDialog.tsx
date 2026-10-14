import { SettingsIcon } from "lucide-react"
import { type SubmitEvent, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"

import { errorForField, errorForForm } from "@/api/fieldErrors"
import type { ProjectView } from "@/api/generated/ProjectView"
import type { ThinkingLevel } from "@/api/generated/ThinkingLevel"
import type { WritingLanguage } from "@/api/generated/WritingLanguage"
import { useUpdateProjectSettings } from "@/api/queries/projects"
import { useSystemSettings } from "@/api/queries/settings"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog"
import {
  Field,
  FieldContent,
  FieldDescription,
  FieldError,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { ModelProfileFields } from "@/features/models/ModelProfileFields"
import { profileForm, profileFromForm } from "@/features/models/profileForm"
import { ModelTest } from "@/features/models/ModelTest"
import { useErrorText } from "@/i18n/errors"

const WRITING_LANGUAGES = ["zh-CN", "en"] as const satisfies readonly WritingLanguage[]
const THINKING_LEVELS = ["off", "low", "medium", "high"] as const satisfies readonly ThinkingLevel[]

/** The work's writing language, model and thinking level (§13.3). */
export function ProjectSettingsDialog({ project }: { project: ProjectView }) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger
        render={
          <Button variant="ghost" size="sm" aria-label={t(($) => $.workspace.header.settings)} />
        }
      >
        <SettingsIcon />
        <span className="hidden md:inline">{t(($) => $.workspace.header.settings)}</span>
      </DialogTrigger>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{t(($) => $.workspace.settings.title)}</DialogTitle>
        </DialogHeader>
        {open && <ProjectSettingsForm project={project} onSaved={() => setOpen(false)} />}
      </DialogContent>
    </Dialog>
  )
}

function ProjectSettingsForm({ project, onSaved }: { project: ProjectView; onSaved: () => void }) {
  const { t } = useTranslation()
  const errorText = useErrorText()
  const update = useUpdateProjectSettings()
  const system = useSystemSettings()
  const [writingLanguage, setWritingLanguage] = useState(project.writingLanguage)
  const [ownModel, setOwnModel] = useState(project.model !== null)
  const [profile, setProfile] = useState(() => profileForm(project.model))
  const [thinkingLevel, setThinkingLevel] = useState(project.thinkingLevel)
  const [budget, setBudget] = useState(String(project.auxiliaryBudget))

  // The model the work's requests use, to say whether thinking is possible with it.
  const effective = ownModel ? profileFromForm(profile) : (system.data?.defaultModel ?? null)
  const thinkingUnsupported =
    thinkingLevel !== "off" && effective !== null && !effective.supportsThinking

  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    update.mutate(
      {
        project,
        settings: {
          writingLanguage,
          model: ownModel ? profileFromForm(profile) : null,
          thinkingLevel,
          auxiliaryBudget: Number(budget),
        },
      },
      {
        onSuccess: () => {
          toast.success(t(($) => $.workspace.settings.done))
          onSaved()
        },
      },
    )
  }

  const languageItems = WRITING_LANGUAGES.map((value) => ({
    value,
    label: t(($) => $.common.writingLanguage[value]),
  }))
  const thinkingItems = THINKING_LEVELS.map((value) => ({
    value,
    label: t(($) => $.workspace.settings.thinkingLevel[value]),
  }))
  const budgetError = errorForField(update.error, "auxiliary_budget")
  const formError = errorForForm(update.error, [
    "model_id",
    "context_window",
    "max_output_tokens",
    "auxiliary_budget",
  ])

  return (
    <form onSubmit={submit} className="flex flex-col gap-6">
      <FieldGroup>
        <Field>
          <FieldLabel htmlFor="project-writing-language">
            {t(($) => $.workspace.settings.writingLanguage)}
          </FieldLabel>
          <Select
            items={languageItems}
            value={writingLanguage}
            onValueChange={(value: WritingLanguage | null) => {
              if (value) setWritingLanguage(value)
            }}
          >
            <SelectTrigger id="project-writing-language" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {languageItems.map((item) => (
                <SelectItem key={item.value} value={item.value}>
                  {item.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <FieldDescription>{t(($) => $.workspace.settings.writingLanguageHint)}</FieldDescription>
        </Field>
        <Field orientation="horizontal">
          <Switch id="project-own-model" checked={ownModel} onCheckedChange={setOwnModel} />
          <FieldContent>
            <FieldLabel htmlFor="project-own-model">
              {t(($) => $.workspace.settings.ownModel)}
            </FieldLabel>
            <FieldDescription>{t(($) => $.workspace.settings.ownModelHint)}</FieldDescription>
          </FieldContent>
        </Field>
        {ownModel && (
          <>
            <ModelProfileFields
              idPrefix="project-model"
              form={profile}
              onChange={setProfile}
              error={update.error}
            />
            <ModelTest request={() => ({ model: profileFromForm(profile) })} />
          </>
        )}
        <Field data-invalid={thinkingUnsupported}>
          <FieldLabel htmlFor="project-thinking">
            {t(($) => $.workspace.settings.thinking)}
          </FieldLabel>
          <Select
            items={thinkingItems}
            value={thinkingLevel}
            onValueChange={(value: ThinkingLevel | null) => {
              if (value) setThinkingLevel(value)
            }}
          >
            <SelectTrigger id="project-thinking" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {thinkingItems.map((item) => (
                <SelectItem key={item.value} value={item.value}>
                  {item.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {thinkingUnsupported && (
            <FieldDescription>
              {t(($) => $.workspace.settings.thinkingUnsupported)}
            </FieldDescription>
          )}
        </Field>
        <Field data-invalid={budgetError !== undefined}>
          <FieldLabel htmlFor="project-budget">{t(($) => $.workspace.settings.budget)}</FieldLabel>
          <Input
            id="project-budget"
            type="number"
            min={0}
            value={budget}
            onChange={(event) => setBudget(event.target.value)}
            aria-invalid={budgetError !== undefined}
            className="w-32"
          />
          <FieldDescription>{t(($) => $.workspace.settings.budgetHint)}</FieldDescription>
          {budgetError !== undefined && <FieldError>{errorText(budgetError)}</FieldError>}
        </Field>
      </FieldGroup>
      {formError !== undefined && <ErrorAlert error={formError} />}
      <DialogFooter>
        <Button type="submit" disabled={update.isPending}>
          {update.isPending
            ? t(($) => $.common.status.saving)
            : t(($) => $.workspace.settings.submit)}
        </Button>
      </DialogFooter>
    </form>
  )
}
