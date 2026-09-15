/* eslint-disable react-refresh/only-export-components */
// 这个模块同时提供 Provider、注册用 Hook 与说明文案常量：它们共享同一份注册表，
// 拆成多个文件反而会让「组合输入与快捷键只有一处判定」这个约束变得难以确认。
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react"

/**
 * 全局快捷键只有这一处实现。
 *
 * 三条硬性约束：
 * 1. 中文输入法组合输入期间（含用 Enter 确认候选词的那一次按键）任何快捷键都不触发，
 *    也不会有表单被隐式提交。仅看 `event.isComposing` 不够：部分浏览器在确认候选词时
 *    先给出 `isComposing === false` 的 keydown，因此同时维护组合状态并识别 keyCode 229。
 * 2. 不抢浏览器原生按键。只处理明确列出的组合键，从不拦截输入框内的复制/粘贴/撤销/
 *    全选/移动/翻页，也没有任何单键绑定会在输入时生效（`?` 只在不在输入框里时打开说明）。
 * 3. 破坏性操作不绑定快捷键：审核通过/退回、停止执行、退出服务仍然只能由按钮发起。
 */

/** 每个可用主操作都必须带一个可读名字，界面与说明面板都用它。 */
export interface PrimaryAction {
  /** 显示在主操作提示里的动词，例如「开始准备」。 */
  label: string
  run: () => void
  /** 当前是否真的可以执行（按钮禁用时同步为 false）。 */
  enabled: boolean
}

interface ShortcutRegistry {
  registerPrimaryAction: (id: string, action: PrimaryAction) => void
  unregisterPrimaryAction: (id: string) => void
  registerFocusTarget: (key: string, element: HTMLElement | null) => void
  registerDismissible: (
    id: string,
    active: boolean,
    dismiss: () => void
  ) => void
  unregisterDismissible: (id: string) => void
  shortcutsOpen: boolean
  setShortcutsOpen: (open: boolean) => void
}

const ShortcutContext = createContext<ShortcutRegistry | undefined>(undefined)

const FOCUS_TARGET_SEARCH = "manuscript-search"
const FOCUS_TARGET_WORKSPACE = "workspace"

/**
 * 组合输入只可能发生在一个正在编辑的元素上，因此记录的是「哪个输入框在组合」而
 * 不是一个全局布尔值。放在模块作用域是为了让表单提交守卫也读到同一个事实，
 * 而不是各处再抄一遍判断；只认可编辑目标则避免一个没有配对的 compositionstart
 * （例如输入法在页面获得焦点时发出的那一次）把整个键盘永久关掉。
 */
let composingTarget: EventTarget | null = null
/** 确认候选词的那一次按键是否仍可能触发这次提交。 */
let pendingCompositionEnter = false

/**
 * 这次按键是否属于输入法组合过程。
 *
 * `keyCode === 229` 覆盖了「keydown 里 `isComposing` 已经是 false、但按键其实在确认
 * 候选词」的浏览器（Safari 与部分 Windows 输入法）。
 */
export function isCompositionEvent(event: KeyboardEvent) {
  if (event.isComposing || event.keyCode === 229) return true
  return composingTarget !== null && event.target === composingTarget
}

const IS_MAC =
  typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform)

/** 快捷键说明里的修饰键写法：Mac 上是 ⌘，其余平台是 Ctrl。 */
const COMMAND_KEY = IS_MAC ? "⌘" : "Ctrl"

/** 快捷键说明面板里的条目，顺序即展示顺序。 */
export const SHORTCUT_ENTRIES = [
  {
    keys: `${COMMAND_KEY} + Enter`,
    title: "提交当前表单",
    detail:
      "光标在任何一个输入框里时，提交这个表单的主操作（保存目标、提交产物、提交细纲、追加正文、查找）。",
  },
  {
    keys: `${COMMAND_KEY} + Enter`,
    title: "触发唯一的阶段主操作",
    detail:
      "光标不在输入框里、且当前只有一个可用的阶段操作时（例如「开始准备」「显式继续」），同一次按键也能触发它。",
  },
  {
    keys: `${COMMAND_KEY} + K`,
    title: "整稿查找",
    detail:
      "在阅读视图里把光标放到整稿查找框。写作视图里没有查找框，因此不生效。",
  },
  {
    keys: "Esc",
    title: "关闭非破坏性面板",
    detail:
      "关闭查找结果、快捷键说明这类可以安全关掉的层。不会提交或跳转任何内容。",
  },
  {
    keys: "?",
    title: "打开或关闭这份说明",
    detail: "只在光标不在输入框里时生效。",
  },
] as const

function isEditableTarget(target: EventTarget | null) {
  if (!(target instanceof HTMLElement)) return false
  if (target.isContentEditable) return true
  return (
    target.closest("input, textarea, select, [contenteditable='true']") !== null
  )
}

export function ShortcutProvider({ children }: { children: ReactNode }) {
  const [shortcutsOpen, setShortcutsOpen] = useState(false)
  const actions = useRef(new Map<string, PrimaryAction>())
  const focusTargets = useRef(new Map<string, HTMLElement>())
  const dismissibles = useRef(
    new Map<string, { active: boolean; dismiss: () => void }>()
  )

  const registerPrimaryAction = useCallback(
    (id: string, action: PrimaryAction) => {
      actions.current.set(id, action)
    },
    []
  )
  const unregisterPrimaryAction = useCallback((id: string) => {
    actions.current.delete(id)
  }, [])
  const registerFocusTarget = useCallback(
    (key: string, element: HTMLElement | null) => {
      if (element) {
        focusTargets.current.set(key, element)
      } else {
        focusTargets.current.delete(key)
      }
    },
    []
  )
  const registerDismissible = useCallback(
    (id: string, active: boolean, dismiss: () => void) => {
      dismissibles.current.set(id, { active, dismiss })
    },
    []
  )
  const unregisterDismissible = useCallback((id: string) => {
    dismissibles.current.delete(id)
  }, [])

  /**
   * 动作可能让触发它的按钮消失（提交表单、继续执行等）。焦点这时会掉到 `<body>`，
   * 键盘用户只能从页面顶部重新 Tab；这里把焦点交给工作台主区域，Tab 起点保持合理。
   */
  const restoreFocusAfterAction = useCallback(() => {
    requestAnimationFrame(() => {
      const active = document.activeElement
      if (active && active !== document.body) return
      focusTargets.current.get(FOCUS_TARGET_WORKSPACE)?.focus()
    })
  }, [])

  const runPrimaryAction = useCallback(() => {
    const active = document.activeElement
    const form = active instanceof HTMLElement ? active.closest("form") : null
    if (form) {
      const submit = form.querySelector<HTMLButtonElement>(
        'button[type="submit"]'
      )
      if (submit?.disabled) return
      form.requestSubmit()
      restoreFocusAfterAction()
      return
    }
    const enabled = [...actions.current.values()].filter(
      (action) => action.enabled
    )
    if (enabled.length !== 1) return
    enabled[0].run()
    restoreFocusAfterAction()
  }, [restoreFocusAfterAction])

  useEffect(() => {
    const onCompositionStart = (event: CompositionEvent) => {
      // 不在输入框里的 compositionstart 不代表有任何拼音正在上屏，忽略它。
      composingTarget = isEditableTarget(event.target) ? event.target : null
    }
    const onCompositionEnd = () => {
      composingTarget = null
      pendingCompositionEnter = false
    }
    // 页面失去焦点时不可能还在组合输入；不清掉的话快捷键会一直静默失效。
    const onWindowBlur = () => {
      composingTarget = null
      pendingCompositionEnter = false
    }
    /**
     * 输入框里按 Enter 会隐式提交所在表单。组合输入期间那次 Enter 属于「确认候选词」，
     * 必须在提交层面拦下——而不是 preventDefault 按键本身，否则会取消候选词上屏。
     * 用捕获阶段记录按键事实、按提交阶段消费，两者之间是同一个同步事件循环，不会串台。
     *
     * 这里必须同时阻断传播：表单由 React 接管，`preventDefault` 只取消浏览器默认动作，
     * React 的 onSubmit 仍会执行（因此仍会发请求）；在 document 捕获阶段 stopPropagation
     * 才能让这次提交真正不发生。
     */
    const onKeyDownCapture = (event: KeyboardEvent) => {
      if (event.key !== "Enter") return
      pendingCompositionEnter = isCompositionEvent(event)
      if (pendingCompositionEnter) {
        // 隐式提交是同步发生的；即便它没发生，也不该把这个标记留到下一次按键。
        setTimeout(() => {
          pendingCompositionEnter = false
        }, 0)
      }
    }
    const onSubmitCapture = (event: Event) => {
      if (!pendingCompositionEnter) return
      pendingCompositionEnter = false
      event.preventDefault()
      event.stopPropagation()
    }

    document.addEventListener("compositionstart", onCompositionStart, true)
    document.addEventListener("compositionend", onCompositionEnd, true)
    document.addEventListener("keydown", onKeyDownCapture, true)
    document.addEventListener("submit", onSubmitCapture, true)
    window.addEventListener("blur", onWindowBlur)
    return () => {
      document.removeEventListener("compositionstart", onCompositionStart, true)
      document.removeEventListener("compositionend", onCompositionEnd, true)
      document.removeEventListener("keydown", onKeyDownCapture, true)
      document.removeEventListener("submit", onSubmitCapture, true)
      window.removeEventListener("blur", onWindowBlur)
    }
  }, [])

  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      // 组合输入期间：Enter 可能是在确认候选词，Esc 可能是在取消候选，方向键在选词，
      // 字母是拼音本身。任何一种都不能被当成快捷键。
      if (isCompositionEvent(event)) return

      const command = event.metaKey || event.ctrlKey
      if (command && event.key === "Enter") {
        event.preventDefault()
        runPrimaryAction()
        return
      }
      if (command && !event.shiftKey && event.key.toLowerCase() === "k") {
        const search = focusTargets.current.get(FOCUS_TARGET_SEARCH)
        if (!search) return
        event.preventDefault()
        search.focus()
        return
      }
      if (event.key === "Escape" && !command) {
        const ordered = [...dismissibles.current.entries()].reverse()
        const top = ordered.find(([, entry]) => entry.active)
        if (!top) return
        event.preventDefault()
        top[1].dismiss()
        return
      }
      if (
        !command &&
        !event.altKey &&
        event.key === "?" &&
        !isEditableTarget(event.target)
      ) {
        event.preventDefault()
        setShortcutsOpen((current) => !current)
      }
    }

    window.addEventListener("keydown", handleKeyDown)
    return () => window.removeEventListener("keydown", handleKeyDown)
  }, [runPrimaryAction])

  const value = useMemo<ShortcutRegistry>(
    () => ({
      registerPrimaryAction,
      unregisterPrimaryAction,
      registerFocusTarget,
      registerDismissible,
      unregisterDismissible,
      shortcutsOpen,
      setShortcutsOpen,
    }),
    [
      registerPrimaryAction,
      unregisterPrimaryAction,
      registerFocusTarget,
      registerDismissible,
      unregisterDismissible,
      shortcutsOpen,
    ]
  )

  return (
    <ShortcutContext.Provider value={value}>
      {children}
    </ShortcutContext.Provider>
  )
}

function useRegistry() {
  const registry = useContext(ShortcutContext)
  if (!registry) throw new Error("快捷键必须在 ShortcutProvider 内使用")
  return registry
}

/** 说明面板的开关；打开的按钮与提示都读它。 */
export function useShortcutHelp() {
  const registry = useRegistry()
  return {
    open: registry.shortcutsOpen,
    setOpen: registry.setShortcutsOpen,
  }
}

/** 注册当前可用主操作；同一 id 重复注册以最后一次为准。 */
export function usePrimaryAction(id: string, action: PrimaryAction) {
  const registry = useRegistry()
  const { label, run, enabled } = action
  useEffect(() => {
    registry.registerPrimaryAction(id, { label, run, enabled })
    return () => registry.unregisterPrimaryAction(id)
  }, [registry, id, label, run, enabled])
}

/**
 * 注册可以被 Ctrl/Cmd+K 聚焦的元素（目前只有整稿查找框）。
 * 返回 ref 回调，在元素真实挂载/卸载时才改动注册表，不依赖重新渲染。
 */
export function useFocusTarget(key: string) {
  const registry = useRegistry()
  return useCallback(
    (element: HTMLElement | null) => {
      registry.registerFocusTarget(key, element)
    },
    [registry, key]
  )
}

/** 注册可被 Esc 关闭的层；后注册的先关闭。 */
export function useEscapeDismiss(active: boolean, dismiss: () => void) {
  const registry = useRegistry()
  const id = useId()
  useEffect(() => {
    registry.registerDismissible(id, active, dismiss)
    return () => registry.unregisterDismissible(id)
  }, [registry, id, active, dismiss])
}

/**
 * 工作台主区域的 ref 回调；动作后焦点掉到 `<body>` 时回落到这里，Tab 起点保持合理。
 * 调用方需要给该元素加 `tabIndex={-1}`。
 */
export function useWorkspaceFocusElement() {
  const registry = useRegistry()
  return useCallback(
    (element: HTMLElement | null) => {
      registry.registerFocusTarget(FOCUS_TARGET_WORKSPACE, element)
    },
    [registry]
  )
}

export const SEARCH_FOCUS_KEY = FOCUS_TARGET_SEARCH
