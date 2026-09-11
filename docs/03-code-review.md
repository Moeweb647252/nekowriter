# NekoWriter 实现进度审查报告（更新版）

- 审查日期：2026-09-11
- 审查范围：`crates/`（core / agent / store / app）、`webui/`、`docs/01-implementation-plan.md`、`docs/02-implementation-spec.md`
- 审查方式：代码审查 + 实测验证；本次补充审核委托实现与回归验证

## 1. 实测验证结果

| 验证项 | 结果 |
| --- | --- |
| `cargo test --workspace` | ✅ 107 个测试全部通过（agent 23 / app 31 / core 15 / store 38） |
| `cargo clippy --workspace --all-targets` | 上次完整审查编译通过；约 255 条 pedantic 警告待消化，本次未重新统计 |
| `pnpm lint` | ⚠️ 未通过：8 个既有 React effect / Fast Refresh 错误与 1 个 Hooks 依赖警告；本次没有新增 lint 诊断 |
| `tsc --noEmit` + `vite build` | ✅ 前端类型检查与构建成功（bundle 345.40 KB / gzip 104.35 KB） |

## 2. 总体结论：整体完成度约 60%（上次审查约 35%）

按方案 §8 六个里程碑加权估算（M0 10% / M1 25% / M2 25% / M3 15% / M4 15% / M5 10%）：

| 里程碑 | 上次 | 本次 | 关键变化 |
| --- | --- | --- | --- |
| M0 本地骨架 | ~95% | ~95% | 无变化。仍差启动时打开浏览器、`logs/` 日志落地 |
| M1 最小写作闭环 | ~60% | **~90%** | **上次最大的硬阻塞（F1）已解决**：`WorkflowScheduler` 已在 `main.rs` 完整接线，每稿本命令循环 + 全局并发 1 + 额度管理 + 设置/凭据端点均已实现。仅差真实外部模型烟测验收 |
| M2 交互正确性 | ~10% | **~65%** | 提案门控、暂停/恢复、意见批次、幂等回执、SSE 游标、中断恢复和额度续授已接通；四类审核均可委托为无工具子 Run，支持取消、重启恢复和重复回调保护。引用 UI、前端 SSE 重连验证与完整竞态矩阵仍待完成 |
| M3 完整创作流程 | ~30% | **~85%** | 多章计划、自动正文、稿本新建/复制/选择、完本和番外路径已接通；自由跨章导航、提示词覆盖与动态项目设置均已落地。仍缺真实模型下的多章到完本/番外验收 |
| M4 知识与管理 | ~5% | ~5% | 无变化。仅 LanceDB 连接存在；知识 CRUD、Embedding、检索、模板、Dashboard 未动 |
| M5 本地交付 | ~10% | ~10% | 无变化。长文虚拟化、回收站、导出、退出流程、发行包未做 |

**距离完成实施计划还差约 40%。**

## 3. 上次审查发现（F1–F13）的解决情况

| 编号 | 上次发现 | 现状 |
| --- | --- | --- |
| F1（高） | 模型执行链路零接线 | ✅ **已解决**。`app/scheduler.rs`（约 2,600 行，含大量测试）实现 `WorkflowScheduler`；`main.rs` 挂载 `enqueue_preparation` / `enqueue_chapter_outline` / `enqueue_chapter_body`，`ChapterBodyToolHost` 实现 `ToolHost`；设置与 `/settings/test-model` 端点存在 |
| F2（高） | 细则 §14 里程碑勾选过时 | ✅ **已解决**。spec §14 与方案 §8 已改写为"M1 已有可运行的本地闭环……真实外部模型验收仍待完成" |
| F3（高） | 前端偏离既定选型（无 TanStack Query/XState/Virtual） | ⚠️ **维持原状**。`package.json` 仍只有 React + Tailwind + Base UI，无 Query/State 库；App.tsx 仍为手写状态管理。属于设计决策，需二选一：要么改文档选型，要么补齐 |
| F4（中） | 折叠排序违反时间线 | ✅ **已解决**。`ContextBuilder::build` 现按 `(Preparation → chapter_order → within_chapter_order → run_id)` 排序，并校验 `requires_chapter_order()`，有对应测试 |
| F5 | UI 章节计划只能录一项 | ✅ **已解决**。`PreparationPanel` 支持多计划项增删（上限 500），store 侧 `materialize_chapter_plan` 逐项物化 |
| F6 | clippy 警告未消化 | ⚠️ 数量从 108 → **255**（代码量翻倍的自然结果）。仍需专项清理 |
| F7 | 25 张表仅建 13 张 | 维持。符合分期；未核对新表数量 |
| F8 | 数据目录双重 NekoWriter | 未复核 |
| F9 | nightly 工具链未固定日期 | 未复核（`rust-toolchain.toml` 仍存在） |
| F10 | 项目不是 Git 仓库 | ✅ **已解决**。已以 `feat: 初始化 NekoWriter MVP` 建立首个提交（140 个文件），`.DS_Store` 等系统文件未纳入 |
| F11–F13 | 正文整文档替换性能等 | 未复核 |

## 4. 各里程碑剩余工作明细

### M1（剩 ~10%）
- **真实模型烟测验收**（spec §14 明确要求：记录模型 ID、服务地址、已验证能力与用量；测试 SSE 服务不能替代）。这是当前 M1 唯一实质缺口。
- 启动打开浏览器 + `logs/` 落地（M0 尾巴）。

### M2（剩 ~35%）
- 引用 UI（持久化与归属校验已在存储层完成，缺前端与交互门控展示）。
- 原生 EventSource 重连策略（后端 SSE 游标已就绪，前端重连逻辑未验证）。
- 细则 §15 的完整编辑、反馈和审核竞态矩阵。
- clippy 255 条警告清理。

### M3（剩 ~40%）
- 跨章导航：App.tsx 永远只渲染 `active_chapter_id` 对应的单章工作区，没有章节列表/自由跳转；后端 `enter_next_chapter` 存在但前端路径未验证。
- 动态项目设置与提示词覆盖。
- 用真实模型验证从多章计划到完本、复制稿本和番外的完整路径。

### M4（剩 ~95%）
- Space/知识 CRUD、Embedding 适配器、向量检索与候选补取、零实现引擎接口、模板管理、Dashboard 统计。目前全部未动。

### M5（剩 ~90%）
- 长篇阅读优化（虚拟化）、主题/键盘体验、回收站、导出、退出流程、发行包。

## 5. 关键路径与建议顺序

1. **真实模型烟测**（M1 收口）——用真实凭据跑通一个最小章节并记录验收证据。完成后 M1 ≈ 100%，整体约 60%。
2. **跨章导航 + 项目提示词设置**（M3 主干）——UI 目前是"单章隧道"，多章流程仍缺自由浏览入口。
3. **M2 收尾**——引用 UI、前端 SSE 重连验证和完整竞态矩阵。
4. **初始化 Git 仓库并提交**——当前所有工作（含 84 个测试）无版本控制保护，风险随代码量增长而放大。
5. **M4 起步**——在 M1–M3 稳定后按方案推进知识/检索。

## 6. 2026-09-11 增量更新：提示词覆盖与跨章浏览

本次增量在 M3 上落地了两块能力，并修复了两个会阻断整个工作区构建的既有缺陷。

### 6.1 修复的构建阻断缺陷（本次之前无法编译）

| 编号 | 缺陷 | 影响 | 修复 |
| --- | --- | --- | --- |
| F14（高） | `nekowriter-core` 在库代码里使用 `serde_json::Number`，但 `serde_json` 只声明在 `[dev-dependencies]` | 任何把 core 当依赖构建的目标（`cargo build`、`cargo check -p nekowriter-store`、发行包）都无法编译 | 把 `serde_json` 提升为 `nekowriter-core` 的常规依赖 |
| F15（高） | `store/preparation.rs` 调用 `review_confidence_from_model` 四次，但该函数不存在 | `nekowriter-store` 无法编译 | 新增该辅助函数：把 `Option<f64>` 还原为 `Option<serde_json::Number>`，NaN/无穷值按损坏记录拒绝 |

### 6.2 新增实现

- **内建提示词覆盖**（`prompt_overrides` 表、`SCHEMA_VERSION` 20 → 21）：系统级与项目级各保存一对 system/user，空字符串是合法的实际空内容，重置即删除行；乐观并发修订、命令回执与 `settings.changed` 事件与覆盖同事务提交。
- **三级解析与请求快照**：调度器在每次请求构建时按「项目覆盖 → 系统覆盖 → 代码内建」解析（`app/src/prompts.rs` 是唯一实现），并把 `prompt.key` / `prompt.source` 写进 `context_snapshot`。
- **目录、变量说明与预览**：`GET /api/prompts/system`、`/api/projects/{id}/prompts`、`PUT|DELETE /api/prompts/system/{key}`、`PUT|DELETE /api/projects/{id}/prompts/{key}`、`POST /api/prompts/preview`；变量元数据新增类型与来源，预览用示例值渲染。
- **跨章浏览与导航**：`ChapterDirectory` 列出全部章节与状态，`ChapterReader` 只读地展示任意章节的指导/细纲/已提交正文，选中后滚动到视口并给出「回到写作章」；浏览不发出任何写作命令，浏览目标以稿本为键，切换稿本即失效。

### 6.3 本次实测

| 验证项 | 结果 |
| --- | --- |
| `cargo test --workspace` | ✅ 88 个测试全部通过（agent 22 / app 20 / core 15 / store 31，含新增的 4 个提示词覆盖测试） |
| `cargo fmt --all -- --check` | ✅ 通过（顺带修正了此次之前已存在的格式漂移） |
| `cargo clippy --workspace --all-targets` | ⚠️ 无新增诊断；新增的 3 个文件零警告，既有 pedantic 警告仍在 |
| `pnpm lint` | ⚠️ 与基线一致：9 条既有诊断，本次新增文件零诊断 |
| `tsc -b && vite build` | ✅ 成功 |
| HTTP 冒烟（真实进程 + curl） | ✅ 目录 10 条、覆盖保存 200/修订 1、过期修订 409、未知变量 422、项目覆盖生效、预览渲染示例值、重置 removed=true、回退到 `builtin`、`/chapters` 返回 `[]` |

### 6.4 更新后的剩余工作

- **M3 尾巴**：用真实模型验证多章计划 → 完本 → 番外的完整路径（见第 8 节）。
- **M2 尾巴**：引用 UI；前端 EventSource 重连验证；§15 竞态矩阵。
- **M4 / M5**：与上一版结论一致，尚未开始。
- **测试缺口**：跨章浏览尚无自动化用例（需要可驱动 `api` 的假模型或 Playwright），长篇虚拟化属于 M5。

## 7. 与上次审查报告的差异说明

本报告替换了上一版 `docs/03-code-review.md`（该文件在会话之间已被删除）。上一版的 13 项发现中：F1/F2/F4/F5/F10 已解决，F3/F6 仍然成立，其余未复核。代码量显著增长（如 `store/preparation.rs` 约 1,100 → 9,000+ 行，新增 `app/scheduler.rs`）；补齐审核委托后测试从 54 增至 84，两次增量（提示词覆盖与跨章浏览、项目级设置）后增至 92。构建阻断项记为 F14/F15，详见第 6 节。

## 8. 2026-09-11 增量更新：项目级设置

本次增量把 M3 的最后一块「动态项目设置」接通：模型引用、思考等级与辅助工具预算。

### 8.1 实现

- **领域类型**（`nekowriter-core`）：新增 `ThinkingLevel`（`Off`/`Low`/`Medium`/`High`）与 `ProjectSettings` / `NewProjectSettings`。字段级缺省是真实语义：早先写入的 `{}` 或缺失字段解析为同一默认值（预算 10、思考 `Off`、模型 `None`），而不是损坏记录。
- **持久化**（`store/project_settings.rs`）：内容存入 `projects.settings_json`，`projects.settings_revision` 独立于项目实体修订 —— 保存设置不会让正在进行的稿本切换命令意外过期。读取、保存、校验（模型 ID 长度、模型能力资料、预算 0–10000、空白模型 ID 等价于沿用默认）、命令回执重放、`IDEMPOTENCY_CONFLICT` 与 `settings.changed` 事件均沿用既有模式。
- **API**：`GET /api/projects/{project_id}/settings`、`PUT /api/projects/{project_id}/settings`。写入与系统设置共用 `settings_write_lock`，并使用 `ensure_trusted_write_request`；越界返回 422 `INVALID_PROJECT_SETTINGS`，过期修订返回 409 `REVISION_CONFLICT`。
- **调度器重建**：每次取得模型许可后重新读取项目设置，解析「项目覆盖 → 系统默认」的有效模型与其能力资料，并把系统设置修订、项目设置修订、`modelOverrideId`、`thinkingLevel`、`auxiliaryToolBudget`、`effectiveModelId`、有效模型来源、能力资料与 `effectiveThinkingLevel` 写进 `settings_snapshot`。思考等级经 `ModelRequest.thinking_level` 映射为 `reasoning_effort`；有效模型不满足当前用途所需能力时，在发请求前返回带模型定位的错误，不静默忽略。
- **能力资料**：`ModelCapabilityProfile` 新增 `thinking`（缺省 `false`，表示未经验证），项目模型引用与能力资料一起保存。系统模型与项目模型共用连接测试；声明支持思考时会真实探测 `reasoning_effort`，并由调度器和 HTTP 命令前置检查共用同一份有效模型解析与能力判定。
- **界面**：新增 `ProjectSettingsPanel`，编辑模型引用（含该模型能力资料）、思考等级与预算；草稿以「项目 id + 修订」为身份，切换项目即失效，保存回写服务端修订，修订冲突以提示呈现；思考等级会按当前有效模型的能力资料给出阻断提示。

### 8.2 已知边界

辅助工具预算目前只做到「持久化 + 暴露 + 写入请求快照」，**尚未实际扣减**：仓库里还没有任何辅助工具（知识检索属于 M4），因此没有可记账的主体。这不影响 M3 的退出标准，但预算的实际语义要等 M4 的检索工具接入后才完整。

### 8.3 本次实测

| 验证项 | 结果 |
| --- | --- |
| `cargo test --workspace` | ✅ 107 个测试全部通过（agent 23 / app 31 / core 15 / store 38，含模型能力归属、有效模型调度与思考参数探测测试） |
| `cargo fmt --all -- --check` | ✅ 通过 |
| `cargo clippy --workspace --all-targets` | ✅ 新增/修改文件的告警已收敛（`prompts.rs` 另修 3 处）；仅剩与既有代码同类的「函数过长」等结构性告警 |
| `tsc -b && vite build` | ✅ 成功 |
| HTTP 冒烟（真实进程 + curl） | ✅ 默认值 `{modelId:null,thinkingLevel:"off",auxiliaryToolBudget:"10",revision:"0"}`；保存返回修订 1；同命令重放 200 且不重复计修订；过期修订 409；预算 10001 返回 422；同命令异载荷返回 `IDEMPOTENCY_CONFLICT`；非可信 Origin 403；未知项目 404 |

未做真实浏览器渲染验证：当前环境无法让后台进程跨工具调用存活（`BACKGROUND` 不可用，`nohup … &` 的命令组在调用结束时被回收），因此无法注册并查看实时预览。面板的正确性由 `tsc`/`vite build` 通过、API 冒烟与源码通读共同保证；真实渲染仍建议在本机 `pnpm dev` 下人工确认一次。
