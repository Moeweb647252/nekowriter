import { Keyboard, Monitor, Moon, Sun } from "lucide-react"

import { Button } from "@/components/ui/button"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { useTheme } from "@/components/theme-provider"
import { useShortcutHelp } from "@/lib/shortcuts"

const THEME_OPTIONS = [
  { value: "light", label: "浅色", icon: Sun },
  { value: "dark", label: "深色", icon: Moon },
  { value: "system", label: "跟随系统", icon: Monitor },
] as const

/**
 * 工作台头部控件：主题选择与快捷键说明。
 *
 * 主题原来绑定单键 `d`，会在阅读和文本操作里误触（见 docs/01 §5）；改为这里的选择器，
 * 选择结果仍然由 `ThemeProvider` 保存在浏览器本地。
 */
export function WorkspaceControls() {
  const { theme, setTheme } = useTheme()
  const { open, setOpen } = useShortcutHelp()

  return (
    <div className="flex items-center gap-2">
      <ToggleGroup
        aria-label="界面主题"
        className="hidden md:flex"
        onValueChange={(values) => {
          const next = values[0]
          if (next === "light" || next === "dark" || next === "system") {
            setTheme(next)
          }
        }}
        size="sm"
        spacing={0}
        value={[theme]}
        variant="outline"
      >
        {THEME_OPTIONS.map((option) => {
          const Icon = option.icon
          return (
            <ToggleGroupItem
              key={option.value}
              title={`主题：${option.label}`}
              value={option.value}
            >
              <Icon data-icon="inline-start" aria-hidden="true" />
              {option.label}
            </ToggleGroupItem>
          )
        })}
      </ToggleGroup>
      <Button
        type="button"
        size="sm"
        variant="outline"
        aria-expanded={open}
        aria-haspopup="dialog"
        onClick={() => setOpen(!open)}
      >
        <Keyboard aria-hidden="true" />
        快捷键
      </Button>
    </div>
  )
}
