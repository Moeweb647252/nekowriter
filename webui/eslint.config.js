import js from '@eslint/js'
import globals from 'globals'
import reactHooks from 'eslint-plugin-react-hooks'
import reactRefresh from 'eslint-plugin-react-refresh'
import tseslint from 'typescript-eslint'
import { defineConfig, globalIgnores } from 'eslint/config'

export default defineConfig([
  globalIgnores(['dist']),
  {
    files: ['**/*.{ts,tsx}'],
    extends: [
      js.configs.recommended,
      tseslint.configs.recommended,
      reactHooks.configs.flat.recommended,
      reactRefresh.configs.vite,
    ],
    languageOptions: {
      globals: globals.browser,
    },
  },
  {
    // 只有这两个组件用虚拟化库（`useVirtualizer`）。本仓没有接入 React Compiler
    // （`babel-plugin-react-compiler` 只是 eslint 插件的可选依赖，构建里没有它），
    // 所以这条提示「编译器会跳过记忆化」的规则在这里没有作用对象；将来真的接入
    // 编译器时应当删掉这个覆盖。收在这两个文件里，别处仍会提示。
    files: [
      'src/components/ChapterDirectory.tsx',
      'src/components/ManuscriptReader.tsx',
    ],
    rules: {
      'react-hooks/incompatible-library': 'off',
    },
  },
])
