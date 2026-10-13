import { useTheme } from "next-themes"
import { useTranslation } from "react-i18next"

import { THEMES, type Theme } from "@/app/theme"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Field, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  UI_LANGUAGE_NAMES,
  UI_LANGUAGES,
  type UiLanguage,
  uiLanguageFor,
} from "@/locales/languages"

export function AppearanceSettings() {
  const { t, i18n } = useTranslation()
  const { theme, setTheme } = useTheme()
  const language = uiLanguageFor(i18n.resolvedLanguage ?? i18n.language)

  const languageItems = UI_LANGUAGES.map((value) => ({ value, label: UI_LANGUAGE_NAMES[value] }))
  const themeItems = THEMES.map((value) => ({
    value,
    label: t(($) => $.settings.appearance.themes[value]),
  }))

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t(($) => $.settings.appearance.title)}</CardTitle>
      </CardHeader>
      <CardContent>
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="ui-language">
              {t(($) => $.settings.appearance.language)}
            </FieldLabel>
            <Select
              items={languageItems}
              value={language}
              onValueChange={(value: UiLanguage | null) => {
                if (value) void i18n.changeLanguage(value)
              }}
            >
              <SelectTrigger id="ui-language" className="w-56">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {languageItems.map((item) => (
                  <SelectItem key={item.value} value={item.value} lang={item.value}>
                    {item.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <FieldDescription>{t(($) => $.settings.appearance.languageHint)}</FieldDescription>
          </Field>
          <Field>
            <FieldLabel htmlFor="ui-theme">{t(($) => $.settings.appearance.theme)}</FieldLabel>
            <Select
              items={themeItems}
              value={(theme as Theme | undefined) ?? "system"}
              onValueChange={(value: Theme | null) => {
                if (value) setTheme(value)
              }}
            >
              <SelectTrigger id="ui-theme" className="w-56">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {themeItems.map((item) => (
                  <SelectItem key={item.value} value={item.value}>
                    {item.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Field>
        </FieldGroup>
      </CardContent>
    </Card>
  )
}
