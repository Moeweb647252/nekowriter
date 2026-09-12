import { useEffect, useRef } from "react"
import { Keyboard, X } from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  SHORTCUT_ENTRIES,
  useEscapeDismiss,
  useShortcutHelp,
} from "@/lib/shortcuts"

/**
 * 快捷键说明。
 *
 * 写作流程的主要动作都必须能被发现：它由头部一个可聚焦的按钮与 `?` 打开，打开时把
 * 焦点交给关闭按钮，关闭后把焦点还给打开它的那个元素；Esc 关闭它（非破坏性）。
 */
export function ShortcutHelpPanel() {
  const { open, setOpen } = useShortcutHelp()
  const closeButton = useRef<HTMLButtonElement>(null)
  const opener = useRef<HTMLElement | null>(null)

  useEffect(() => {
    if (open) {
      opener.current =
        document.activeElement instanceof HTMLElement
          ? document.activeElement
          : null
      closeButton.current?.focus()
      return
    }
    opener.current?.focus()
    opener.current = null
  }, [open])

  useEscapeDismiss(open, () => setOpen(false))

  if (!open) return null

  return (
    <div className="fixed inset-x-0 bottom-0 z-50 flex justify-center px-4 pb-4">
      <section
        role="dialog"
        aria-modal="false"
        aria-label="快捷键"
        className="w-full max-w-2xl rounded-2xl border border-border bg-background/95 p-5 shadow-lg backdrop-blur"
      >
        <div className="flex items-start justify-between gap-4">
          <div className="flex items-start gap-3">
            <Keyboard
              className="mt-0.5 size-4 shrink-0 text-muted-foreground"
              aria-hidden="true"
            />
            <div>
              <h2 className="text-sm font-semibold">快捷键</h2>
              <p className="mt-1 text-xs leading-5 text-muted-foreground">
                中文输入法组合输入期间不会触发任何快捷键；输入框内的复制、粘贴、撤销与全选
                保持浏览器原生行为。通过、拒绝、停止与退出这类操作只绑定按钮，没有快捷键。
              </p>
            </div>
          </div>
          <Button
            ref={closeButton}
            size="sm"
            variant="ghost"
            onClick={() => setOpen(false)}
          >
            <X aria-hidden="true" />
            关闭
          </Button>
        </div>
        <dl className="mt-4 space-y-3">
          {SHORTCUT_ENTRIES.map((entry) => (
            <div
              key={`${entry.keys}-${entry.title}`}
              className="grid gap-1 sm:grid-cols-[10rem_minmax(0,1fr)] sm:gap-3"
            >
              <dt>
                <kbd className="rounded-md border border-border bg-muted px-2 py-1 font-mono text-xs">
                  {entry.keys}
                </kbd>
              </dt>
              <dd className="text-xs leading-5">
                <span className="font-medium">{entry.title}</span>
                <span className="mt-0.5 block text-muted-foreground">
                  {entry.detail}
                </span>
              </dd>
            </div>
          ))}
        </dl>
      </section>
    </div>
  )
}
