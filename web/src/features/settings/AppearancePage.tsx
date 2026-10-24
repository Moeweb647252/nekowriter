/** Appearance section (PROJECT §8.12, spec/11 §6–§7): language, theme, editor font and size. */
import { useTranslation } from "react-i18next"
import { Monitor, Moon, Sun } from "lucide-react"

import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { useTheme } from "@/components/theme-provider"
import { SUPPORTED_LANGUAGES, setLanguage, type Language } from "@/i18n"
import { useUiStore, type EditorFont, type EditorSize } from "@/stores/ui"

const FONTS: EditorFont[] = ["serif", "sans"]
const SIZES: EditorSize[] = ["s", "m", "l"]

export default function AppearancePage() {
  const { t, i18n } = useTranslation()
  const { theme, setTheme } = useTheme()
  const editorFont = useUiStore((state) => state.editorFont)
  const editorSize = useUiStore((state) => state.editorSize)
  const setEditorFont = useUiStore((state) => state.setEditorFont)
  const setEditorSize = useUiStore((state) => state.setEditorSize)

  return (
    <div className="max-w-xl space-y-6">
      <div>
        <h2 className="text-base font-semibold">{t("settings:appearance")}</h2>
        <p className="text-sm text-muted-foreground">{t("settings:appearance_hint")}</p>
      </div>

      <div className="flex items-center gap-3">
        <Label htmlFor="appearance-language" className="w-40 shrink-0">
          {t("common:language")}
        </Label>
        <Select
          value={i18n.language}
          onValueChange={(value) => {
            void setLanguage(value as Language)
          }}
        >
          <SelectTrigger id="appearance-language" className="flex-1">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {SUPPORTED_LANGUAGES.map((language) => (
              <SelectItem key={language} value={language}>
                {language === "en" ? t("common:language_en") : t("common:language_zh_cn")}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <div className="flex items-center gap-3">
        <Label className="w-40 shrink-0">{t("common:theme")}</Label>
        <ToggleGroup
          type="single"
          value={theme}
          onValueChange={(value) => {
            if (value === "light" || value === "dark" || value === "system") {
              setTheme(value)
            }
          }}
        >
          <ToggleGroupItem value="light" aria-label={t("common:theme_light")}>
            <Sun className="size-4" />
          </ToggleGroupItem>
          <ToggleGroupItem value="dark" aria-label={t("common:theme_dark")}>
            <Moon className="size-4" />
          </ToggleGroupItem>
          <ToggleGroupItem value="system" aria-label={t("common:theme_system")}>
            <Monitor className="size-4" />
          </ToggleGroupItem>
        </ToggleGroup>
      </div>

      <div className="flex items-center gap-3">
        <Label className="w-40 shrink-0">{t("settings:appearance_editor_font")}</Label>
        <ToggleGroup
          type="single"
          value={editorFont}
          onValueChange={(value) => {
            if (value === "serif" || value === "sans") {
              setEditorFont(value)
            }
          }}
        >
          {FONTS.map((font) => (
            <ToggleGroupItem key={font} value={font}>
              {t(`settings:font_${font}`)}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      </div>

      <div className="flex items-center gap-3">
        <Label className="w-40 shrink-0">{t("settings:appearance_editor_size")}</Label>
        <ToggleGroup
          type="single"
          value={editorSize}
          onValueChange={(value) => {
            if (value === "s" || value === "m" || value === "l") {
              setEditorSize(value)
            }
          }}
        >
          {SIZES.map((size) => (
            <ToggleGroupItem key={size} value={size}>
              {t(`settings:size_${size}`)}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      </div>
    </div>
  )
}
