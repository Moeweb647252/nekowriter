import { Keyboard, Monitor, Moon, Sun } from "lucide-react"

import { Button } from "@/components/ui/button"
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
      <div
        className="flex items-center gap-1 rounded-full border border-border bg-background/75 p-1 shadow-sm backdrop-blur"
        role="group"
        aria-label="界面主题"
      >
        {THEME_OPTIONS.map((option) => {
          const Icon = option.icon
          return (
            <button
              key={option.value}
              type="button"
              aria-pressed={theme === option.value}
              title={`主题：${option.label}`}
              onClick={() => setTheme(option.value)}
              className={`flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs whitespace-nowrap transition focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring ${
                theme === option.value
                  ? "bg-primary text-primary-foreground"
                  : "text-muted-foreground hover:bg-muted hover:text-foreground"
              }`}
            >
              <Icon className="size-3.5" aria-hidden="true" />
              {option.label}
            </button>
          )
        })}
      </div>
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
