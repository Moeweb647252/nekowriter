import path from "path"
import { defineConfig } from "vitest/config"

// 只跑纯逻辑模块：不加载 React/Tailwind 插件，也不需要 jsdom——测试成本保持最小，
// 让「跑一次测试」和「读一遍这个文件」一样便宜。
export default defineConfig({
  resolve: {
    alias: {
      "@": path.resolve(import.meta.dirname, "./src"),
    },
  },
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
})
