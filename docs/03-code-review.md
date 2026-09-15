# NekoWriter 实现进度审查报告（更新版）

- 初始审查日期：2026-09-11；最近收口更新：2026-09-13
- 审查范围：`crates/`（core / agent / store / app）、`webui/`、`docs/01-implementation-plan.md`、`docs/02-implementation-spec.md`
- 审查方式：代码审查 + 实测验证；本报告保留各轮发现时的证据，**每一节的数字都是该节时点的值**，最新状态以 §45–§47 与方案 §2.1 为准。

2026-09-13 收口状态（**该时点**，不是当前值）：`cargo test --workspace` 通过 232 个用例（agent 44、app 单元 61 + 集成 2、core 38、store 83、xtask 4），前端生产构建通过；M0–M4 和 macOS aarch64 的 M5 范围均已完成。M2 的可达竞态与浏览器复核、M3 的多章/复制稿本/番外真实路径以及内建提示词升级均已在 §33–§34 收口。§36 记录最新一轮真实端点实战暴露的缺陷，§36.1 是它对应的修复（被业务拒绝的阶段提交按「被拒回合」处理），但修复尚未在真实端点上复跑；全平台计划只剩非 macOS 发行产物。

**该时点之后的数字变化**（当前值）：`cargo test --workspace` **253 通过 / 0 失败**（本仓 251：agent 44、app 单元 67、app 集成 2、core 38、store 96、xtask 4；另一线程在建的 `nekowriter-prompt-lab` 2），`cargo clippy -p …` 逐包与 `--workspace --all-targets` 均 **0 error / 0 warning**，前端 `pnpm lint` **0 error / 0 warning**。来源与口径见 §46.5、§47.4；本节以下各轮的用例数一律是各轮自己的值，不再回填。

## 1. 初始审查实测验证结果（2026-09-11）

| 验证项 | 结果 |
| --- | --- |
| `cargo test --workspace` | ✅ 107 个测试全部通过（agent 23 / app 31 / core 15 / store 38） |
| `cargo clippy --workspace --all-targets` | ✅ 检查完成；既有代码仍有 pedantic/结构性警告，本次新增与修改文件未发现新增告警 |
| `pnpm lint` | ⚠️ 未通过：8 个既有 React effect / Fast Refresh 错误与 1 个 Hooks 依赖警告；本次没有新增 lint 诊断 |
| `tsc -b` + `vite build` | ✅ 前端类型检查与构建成功（bundle 368.50 KB / gzip 110.12 KB） |

## 2. 里程碑进度对照（2026-09-13 当前状态）

按方案 §8 六个里程碑加权估算（M0 10% / M1 25% / M2 25% / M3 15% / M4 15% / M5 10%）：

| 里程碑 | 上次 | 本次 | 关键变化 |
| --- | --- | --- | --- |
| M0 本地骨架 | ~95% | **~100%** | 启动时打开浏览器、`logs/` 日志落地、数据目录锁与健康检查均已验收 |
| M1 最小写作闭环 | ~60% | **~100%** | `WorkflowScheduler` 已完整接线，并用 InferX `deepseek-v4.1-flash` 在 macOS aarch64 上以 `reasoning_effort=low` 跑通准备 → 细纲 → 正文工具 → 整章审核 → 完本 |
| M2 交互正确性 | ~10% | **100%** | 提案门控、暂停/恢复、意见批次、幂等回执、SSE 游标、中断恢复、引用 UI 与预览层已接通；A/E/F/R 可达竞态与真实浏览器路径已复核 |
| M3 完整创作流程 | ~30% | **100%** | 真实模型已验收两章主线、番外、稿本复制、导出和项目/稿本切换 |
| M4 知识与管理 | ~5% | **~100%** | 知识 CRUD、Embedding、向量检索、预算账本、零引擎注册表、积累写入命令、模板与 Dashboard 均已完成 |
| M5 本地交付 | ~10% | **~98%** | 长文阅读、回收站、导出、退出流程和 macOS aarch64 发行包已验收，仅剩非 macOS 发行产物 |

**macOS aarch64 验收范围已完成**。按全部平台的原计划估算约为 99.8%，余量只是不在本轮范围内的非 macOS 发行产物。

## 3. 上次审查发现（F1–F13）的解决情况

| 编号 | 上次发现 | 现状 |
| --- | --- | --- |
| F1（高） | 模型执行链路零接线 | ✅ **已解决**。`app/scheduler.rs`（约 2,600 行，含大量测试）实现 `WorkflowScheduler`；`main.rs` 挂载 `enqueue_preparation` / `enqueue_chapter_outline` / `enqueue_chapter_body`，`ChapterBodyToolHost` 实现 `ToolHost`；设置与 `/settings/test-model` 端点存在 |
| F2（高） | 细则 §14 里程碑勾选过时 | ✅ **已解决**。spec §14 与方案 §8 已改写为"M1 已有可运行的本地闭环……真实外部模型验收仍待完成" |
| F3（高） | 前端偏离既定选型（无 TanStack Query/XState/Virtual） | ⚠️ **维持原状**。`package.json` 仍只有 React + Tailwind + Base UI，无 Query/State 库；App.tsx 仍为手写状态管理。属于设计决策，需二选一：要么改文档选型，要么补齐 |
| F4（中） | 折叠排序违反时间线 | ✅ **已解决**。`ContextBuilder::build` 现按 `(Preparation → chapter_order → within_chapter_order → run_id)` 排序，并校验 `requires_chapter_order()`，有对应测试 |
| F5 | UI 章节计划只能录一项 | ✅ **已解决**。`PreparationPanel` 支持多计划项增删（上限 500），store 侧 `materialize_chapter_plan` 逐项物化 |
| F6 | clippy 警告未消化 | ⚠️（**该时点**）仍有历史 pedantic/结构性建议；本轮已应用可自动修复的低风险建议，但严格 `-D warnings` 仍不通过。该卫生项不属于 M2/M3 的功能验收条件。**后续状态**：§46 已把 `clippy` 清零（414 条 / 230 个唯一站点里 75 个机械项逐条修掉、155 个写成有理由的政策），该发现关闭，见 §46.1–46.3。 |
| F7 | 25 张表仅建 13 张 | 维持。符合分期；未核对新表数量 |
| F8 | 数据目录双重 NekoWriter | 未复核 |
| F9 | nightly 工具链未固定日期 | 未复核（`rust-toolchain.toml` 仍存在） |
| F10 | 项目不是 Git 仓库 | ✅ **已解决**。已以 `feat: 初始化 NekoWriter MVP` 建立首个提交（140 个文件），`.DS_Store` 等系统文件未纳入 |
| F11–F13 | 正文整文档替换性能等 | 未复核 |

## 4. 各里程碑剩余工作明细

### M1（已完成）
- InferX 真实连接与一章从准备到完本的全流程已经在 macOS aarch64 上验收，能力、`reasoning_effort` 和供应商用量记录见实现细则 §15.5–§15.6。

### M2（已完成）
- §15.2 A/E/F/R 的可达编辑、反馈和审核竞态已由 core、store、agent 与 scheduler 回归覆盖；E06、R05 由状态机主动拒绝构造，防御性校验仍保留。
- 真实浏览器已验证 EventSource、稿本/项目切换、阅读与搜索、中文组合输入、导出、项目设置保存和设置隔离；保存成功提示不会被同步设置事件立即清掉。

### M3（已完成）
- 真实 InferX 路径已完成两章主线、一个番外、稿本复制、Markdown/TXT 导出和完本确认；成功请求保留 `reasoning_effort=low` 与非零推理用量。
- 真实浏览器已补齐项目切换、源稿本与副本切换、三章阅读和项目设置编辑验收。

### M4（已完成）
- 知识库、Embedding、预算账本、零生产引擎注册表、积累写入命令、模板与 Dashboard 均已实现；生产注册表保持为空，具体引擎接入仍是产品范围外的后续工作。

### M5（剩 ~2%）
- 仅剩非 macOS 发行产物；长篇阅读、主题/键盘体验、回收站、导出、退出流程和 macOS aarch64 发行包均已验收。

## 5. 当前剩余项

macOS aarch64 范围没有剩余实现或验收项。全平台计划仅在目标环境构建和验收非 macOS 发行产物；该项不在本轮用户验收范围内。

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

本报告替换了上一版 `docs/03-code-review.md`（该文件在会话之间已被删除）。上一版的 13 项发现中：F1/F2/F4/F5/F10 已解决，F3/F6 仍然成立，其余未复核。代码量显著增长（如 `store/preparation.rs` 约 1,100 → 9,000+ 行，新增 `app/scheduler.rs`）；补齐审核委托后测试从 54 增至 84，提示词覆盖与跨章浏览增量后增至 88，项目级设置增量后增至 92，当前模型能力修正后为 107。构建阻断项记为 F14/F15，详见第 6 节。

## 8. 2026-09-11 增量更新：项目级设置与有效模型能力

本次增量补齐了「动态项目设置」的有效模型能力归属：模型引用、思考等级与辅助工具预算现在与实际请求使用的模型一致，并支持思考参数探测。

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
| `cargo clippy --workspace --all-targets` | ✅ 检查完成；新增与修改文件未发现新增告警，既有代码仍有 pedantic/结构性告警 |
| `tsc -b && vite build` | ✅ 成功 |
| HTTP 冒烟（真实进程 + curl） | 本次未重新运行；上一轮已验证项目设置默认值、保存/重放、修订冲突、预算校验、来源校验与未知项目响应。本次项目模型能力归属、有效模型选择、dispatch 前能力拒绝和 `reasoning_effort` 载荷由 app/store/agent 自动化测试覆盖 |

未做真实浏览器渲染验证：当前环境无法让后台进程跨工具调用存活（`BACKGROUND` 不可用，`nohup … &` 的命令组在调用结束时被回收），因此无法注册并查看实时预览。面板的正确性由 `tsc`/`vite build` 通过、API 冒烟与源码通读共同保证；真实渲染仍建议在本机 `pnpm dev` 下人工确认一次。

## 9. 2026-09-11 增量更新：正文引用（附件性反馈）

本次增量补齐 M2 剩余的「引用与附件性反馈」：工作台可以把选中文字固定成引用并随意见提交，后端补上引用回看所需的只读修订接口。

### 9.1 实现

- **只读修订接口**（`nekowriter-core` / `nekowriter-store` / `nekowriter-app`）：新增领域视图 `DocumentRevisionView`（`document_id`、`revision_id`、`revision_no`、`role`、`chapter_id`、`word_count`、`content`、`is_current`、`created_at_ms`）与 `Store::get_document_revision`。引用永久锚定在提交时的修订上，因此该修订即使已被更新也必须可读；`is_current` 只用于界面提示，不改变引用指向。新增 `StoreError::DocumentRevisionNotFound` 并映射为 404 `RESOURCE_NOT_FOUND`。新路由 `GET /api/documents/{document_id}/revisions/{revision_id}`。
- **偏移换算**（`webui/src/lib/citations.ts`）：后端按 Unicode scalar value 计数、浏览器 Selection 按 UTF-16 code unit 计数。新增 `utf16OffsetToScalar` / `scalarOffsetToUtf16` / `scalarSlice` / `selectionInside` / `rangeFromSelection`，把选区收敛为 `{ startChar, endChar, quote }`；emoji 与组合字符不会再引用到错误位置。
- **可选正文组件**（`webui/src/components/CitableText.tsx`）：把原文渲染成单个文本节点，选区偏移可精确映射回底层文本；选中后浮出「引用选中文字」按钮，按下时阻止默认行为以保留选区。
- **引用展示与回看**（`webui/src/components/CitationList.tsx`）：区分「正文引用」与「提案快照」，当前正文修订与引用修订不一致时标记「来自旧修订」；点击可只读回看原修订，并在位置与原文一致时高亮引用区间。
- **接入工作台**（`webui/src/components/ChapterBodyPanel.tsx`）：逐次编辑对比面板的「当前正式正文」「候选正文」都可生成引用；自由意见区提供可引用的当前正文折叠区。引用按章节键隔离，切换章节即丢弃。逐次编辑的主操作改为按「有无文字或引用」动态显示「接受编辑」或「拒绝并反馈」，不再允许带反馈接受并把反馈静默丢弃；提交成功后清空草稿，失败保留。
- **修复引用线上格式**（`nekowriter-core`）：`FeedbackCitation` 的枚举级 `rename_all` 只重命名变体名，字段仍是 snake_case，而前端一直按 camelCase 发送，因此任何真实引用都会在 HTTP 层以 422 `missing field document_id` 失败——之前前端始终传空数组，所以未被发现。现为两个变体加上 `#[serde(rename_all = "camelCase")]`，并用 `feedback_citations_round_trip_the_camel_case_wire_contract` 锁定序列化与反序列化两向格式。

### 9.2 委托与人工决定统一携带引用

- `Store::delegate_review` 与 `DelegateReviewRequest` 新增 `citations`；委托事务内用 `validate_delegation_citations` 逐字复核后才写入 `user_input_json`，并与 `notes` 一起进入命令幂等哈希。
- `DelegatedReviewTask` 新增 `citations`，调度器把引文渲染进审核子 Run 的用户内容（`delegated_review_user_content`），只输出引文文本，不泄露文档、修订或工具调用 ID。
- 校验范围：准备审核可引用本稿本内任意文档；细纲/整章/逐次编辑审核只能引用本章文档；待审候选引用仅逐次编辑审核可用，且必须匹配候选工具调用与候选哈希。
- 前端：`delegateReview` 增加引用参数，逐次编辑 Dock 的「委托审核」会把已选引用一并提交并在成功后清空。
- **人工审核决定携带引用**：`decide_review` 与三个 `decide_*_review` 新增 `citations` 参数（并计入命令幂等哈希），事务内校验后由 `persist_human_review_decision` 与决定一起写入 `review.user_input_json`。决定只能引用已持久化修订；未落库的候选引用只属于逐次编辑审核，不能借决定绕过候选哈希校验。
- **统一校验**：抽出 `validate_document_citation`（修订存在 → `document_id` 匹配 → 文档未删除 → 属于本稿本 → `chapter_id` 一致 → 范围逐字匹配），正文意见、委托审核与人工决定原先两份近乎重复的实现改为调用同一函数；新增 `validate_review_decision_citations` 只负责决定路径的候选拒绝与章节范围。
- **三处面板**：`PreparationPanel`、`ChapterOutlinePanel` 与 `ChapterBodyPanel` 的终稿区块把产物/细纲/终稿原文换成 `CitableText`，可选区并附带 `CitationList`；引用随决定或委托提交，成功后清空，失败保留。
- **类型化调用点**（`webui/src/components/ChapterBodyPanel.tsx`）：删除可传错配对的字符串参数 `citeSelection(range, "proposal", "document")`，改为 `citeProposalFromDocument` / `citeProposalFromCandidate` / `citeFeedbackFromDocument` / `citeFinalReview` 四个无字符串参数的入口，来源与目标由函数名唯一确定。

### 9.3 已知边界

- 引用回看只展示全文与高亮，不提供从引用跳转到当前正文对应位置。
- 自由意见仍要求非空文本，暂不接受「仅引用」的意见。
- Dock 草稿的浏览器本地存储持久化（细则 §10.5 的「提交成功才清除」）仍未实现，本次只保证引用草稿按章节隔离。

### 9.4 本次实测

| 验证项 | 结果 |
| --- | --- |
| `cargo test --workspace` | ✅ 110 个测试全部通过（agent 23 / app 32 / core 16 / store 39）；新增 `review_decision_persists_validated_citations_and_rejects_bad_ones` 覆盖决定引用的正向落库、错误 quote 的事务回滚，以及 snake_case/camelCase 线格式 |
| `cargo fmt --all -- --check` | ✅ 通过 |
| `cargo check -p nekowriter-app` | ✅ 通过（本机需要以 `--config 'env.PROTOC.value="…"'` 覆盖 `.cargo/config.toml` 里写死的 Windows `protoc` 路径） |
| `tsc -b && vite build` | ✅ 通过（bundle 378.33 KB / gzip 112.66 KB） |
| `pnpm lint` | ⚠️ 与基线一致：8 个既有错误与 1 个警告，本次新增文件零诊断 |
| HTTP 冒烟（真实进程 + curl） | ✅ `/api/health` 200；未知文档修订 404 `RESOURCE_NOT_FOUND`；未知接口 404 `NOT_FOUND`；非法 UUID 路径 400。新决策路径用真实项目走到准备审核：camelCase 引用在 `POST /api/reviews/{id}/decision` 上正确反序列化并进入逐字校验（错误 quote 返回 `INVALID_CITATION` 与领域原因，合法引用返回 200 且 Review 变 `approved`） |

未做真实浏览器渲染验证：选区交互与 `CitableText` 的浮出按钮需在本机 `pnpm dev` 下人工确认。后台进程本轮已能在单次命令内启动并接受 curl 探测，但跨工具调用保活仍不可用，因此未注册实时预览。引用算术的正确性由 `scalarSlice` 与服务端逐字复核共同保证。

## 10. 2026-09-11 增量更新：M4 知识库、预算账本与零引擎能力

本次增量为细则 §7 的完整落地：知识数据与索引、Embedding 接入、语义检索与补取、辅助工具预算账本、研究与积累的零生产引擎，以及准备/细纲 Agent 的 `query_knowledge` 工具。

### 10.1 知识数据、索引与检索

- **领域与实体**（`nekowriter-core/src/knowledge.rs`、`nekowriter-store/src/entity/{knowledge_space,knowledge_entry,knowledge_index_task,embedding_setting}.rs`）：Space 与条目带独立修订与软删除；索引任务以 `entry_id + entry_revision + embedding_profile_id` 唯一；Embedding 配置单独一行并带 profile 指纹。
- **索引器**（`nekowriter-app/src/knowledge.rs`）：后台轮询领取任务，按 600 非空白字符目标、1000 字符硬上限、80 字符重叠切分（按 Unicode 边界），批量调用 OpenAI 兼容的 `POST {api_base}/embeddings`，把向量写入按 profile 隔离的 LanceDB 表。进程启动时把 `running` 任务重新排队，向量始终可由正文重建。
- **一致性**：任务完成时若条目已更新，任务标记为失败而不是把旧修订标成 ready；检索先做向量近邻，再回查 SQLite 的存活、当前修订、Space 与索引状态，剔除失效块。
- **补取**：`top_k` 指过滤去重后的最终有效块数。首轮上限为 `top_k`，有效结果不足且本轮取满时翻倍并复用同一查询向量与固定表版本重取；候选耗尽则允许少于 `top_k`，以 `chunk_id` 作为稳定次序。全部轮次共用一次 Embedding 调用。
- **重建**：`POST /api/knowledge/reindex` 重置**全部**条目的索引任务并返回覆盖度快照，回执按 `command_id` 保存，重放返回已记录快照而不重算。重建期间搜索会因为没有 ready 条目而返回空结果与覆盖度，这是有意的：不把未重建完成的索引当作完整检索。
- **工作台**（`webui/src/components/KnowledgePanel.tsx`）：Space 侧栏、条目列表与编辑区、索引状态、Embedding 配置与连接测试、语义检索与显式重建。

### 10.2 辅助工具预算账本

- **表与作用域**：新增 `quota_scopes`（`version_id + stage_key` 唯一）与 `quota_charges`（`scope_id + tool_call_id` 唯一），schema 版本 23。准备阶段 Run 与每章细纲 Run 在创建时建立/复用作用域并写入 `runs.quota_scope_id`；阶段修订、暂停继续与显式重试都不新建作用域。
- **账本语义**（`nekowriter-store/src/budget.rs`）：作用域只记录已用数，**有效上限每次执行前从项目最新设置读取**，因此下调到已用数以下只让剩余为 0，不追回已执行的调用；上调立即生效且不清空已用数。
- **扣费**：`charge_auxiliary_tool` 以 `(scope_id, provider_call_id)` 唯一键登记，同一调用重放返回 `AlreadyCharged`；余额为 0 时返回 `Exhausted` 且不写入任何记录。`ask_user` 与正文工具不收费，传给账本会直接报错。

### 10.3 零生产引擎与 `query_knowledge`

- **合同与注册表**（`nekowriter-agent/src/engine.rs`）：`ResearchEngine` / `AccumulationEngine` 两个 trait、请求/结果类型与 `EngineDescriptor`；生产注册表 `RESEARCH_ENGINES` / `ACCUMULATION_ENGINES` 严格为 0 项。`research_engine` / `accumulation_engine` 查不到时返回 `EngineError::NotAvailable`（协议码 `ENGINE_NOT_AVAILABLE`）。测试专用实现只存在于该文件的契约测试里。
- **能力查询**（`GET /api/capabilities`）：返回空数组**与原因**（`researchUnavailableReason` / `knowledgeAccumulationUnavailableReason`），界面据此展示「暂无可用引擎」，不出现模拟进度。
- **积累入口**（`POST /api/knowledge/accumulate`）：先用应用服务校验线索非空与目标 Space 存在（422 / 404 优先于引擎结论），再查注册表。没有注册项因此总是 409 `ENGINE_NOT_AVAILABLE`：不创建运行任务、不写条目、不扣预算。
- **`query_knowledge` 工具**（`nekowriter-app/src/scheduler.rs`）：准备与细纲 Agent 的工具白名单在 **Embedding 已配置、索引确有 ready 条目且本项目还有余额** 时才宣告该工具，工具描述里带上当前剩余次数（细则 §7.1「请求构建时说明余额」）。执行顺序固定为「重放检查 → 参数校验 → 可用性校验 → 扣一次预算 → 检索 → 落库并推进工作流」：参数无效、没有可用 Embedding、预算耗尽都不扣费；一旦真的发起检索，失败仍计 1 次。
- **落库与重放**（`nekowriter-store/src/preparation.rs`）：`FreeAgentToolCall` + `execute_free_agent_tool_call` 把 assistant 工具调用、tool 结果、工具调用记录、工作流推进与事件放在一个事务里；同一 `provider_call_id` 重放只返回已提交结果，名称或参数变化则报协议错误。`ask_user` 的参数错误路径已改为复用同一个提交函数（`prepare_free_agent_tool_call` + `commit_free_agent_tool_call`），不再有两份近乎相同的工具提交代码。

### 10.4 已知边界

- 积累引擎的**成功路径尚未存在**：注册表为 0 项，因此「引擎返回候选条目 → 应用服务校验并插入」只写在路由里但无法执行，等第一个真实引擎接入时一并验证。
- `query_knowledge` 只对准备与细纲阶段宣告；正文 Agent 的 `append_content`/`replace_content` 与审核 Agent 不参与知识检索。
- 没有「按 Space 的浏览筛选 + 语义检索」联合入口之外的检索调参界面，切分参数仍是集中常量。
- 嵌入调用的 Token 用量未落库，因此 Dashboard 只统计对话请求的 `usage_json`；§12.1 的「Embedding 输入用量单独列示」待补。
- 引用回看、自由意见空文本、Dock 草稿本地持久化三项边界与 §9.3 相同，仍未处理。

### 10.5 本次实测

| 验证项 | 结果 |
| --- | --- |
| `cargo test --workspace` | ✅ 131 个测试全部通过（agent 30 / app 36 / core 16 / store 49） |
| `cargo fmt --all -- --check` | ✅ 通过 |
| `cargo clippy --workspace --all-targets` | ✅ 本次新增/修改文件零告警（`budget.rs`、`engine.rs`、`app/knowledge.rs`、`scheduler.rs` 新函数、`store/knowledge.rs` 测试）；其余既有 pedantic 告警不在本次范围 |
| `tsc -b && vite build` | ✅ 通过（bundle 396.32 KB / gzip 116.56 KB） |
| `pnpm lint` | ⚠️ 与基线一致：8 个既有错误与 1 个警告，`KnowledgePanel.tsx` 与 `api.ts` 零诊断 |
| HTTP 冒烟：知识全链路（真实进程 + 本地 Embedding stub） | ✅ 保存 Embedding 配置与连接测试、建 Space、建条目后索引从 `pending` 变为 `ready`（LanceDB 表按 profile 建立）、语义检索返回带 `entryRevision` 与 score 的命中、Space 过滤生效、更新条目后检索只返回新修订文本、手动重建后覆盖度回到全部 ready 且搜索恢复、空查询 400/422 |
| HTTP 冒烟：零引擎与预算 | ✅ `/api/capabilities` 返回空引擎数组与原因；`POST /api/knowledge/accumulate` 空线索 422、未知 Space 404、合法请求 409 `ENGINE_NOT_AVAILABLE` 且 Space 条目仍为 0；`quota_scopes` / `quota_charges` 表与两个唯一索引已由真实数据库 schema 确认 |
| 预算账本自动化 | ✅ `quota_charges_once_per_tool_call_and_never_below_zero`、`free_tools_and_other_runs_never_charge_the_scope`、`free_agent_tool_call_requeues_and_replays_its_stored_result`，以及 app 层三个 `knowledge_tool_*` 用例（无 Embedding 不扣费、预算为 0 返回 `QUOTA_EXHAUSTED` 不扣费、真实检索失败仍计 1 次且重放不重复扣费） |

`query_knowledge` 的**成功检索路径**只被 store 层与 HTTP 检索路由覆盖，Agent 环内的成功回送需要一个真实模型（或 chat-completions 桩）才能跑通；本轮覆盖的是它的全部失败与拒绝分支，以及成功分支之外的落库、重放与工作流推进。

## 11. 2026-09-12 增量更新：用户提示词模板与可回算 Dashboard（M4 收口）

本次增量完成 M4 第三个复选框，并把受限模板语言收敛到唯一实现。

### 11.1 受限模板语言现在只有一份

- **唯一实现**（`nekowriter-core/src/template.rs`）：`TemplateVariable`（名字、中文说明、类型、是否必需、示例）、`TemplateError`、`validate_template`、`require_declared_placeholders`、`render_template`、`render_template_preview` 与 `validate_template_variables`。扫描与渲染共用一个 `walk_template`，因此两者对括号、`\{{...}}` 转义与变量名的判定不可能分叉。
- **内建目录保留在 agent**（`nekowriter-agent/src/prompt.rs`）：`PromptVariableDefinition`（含 `source`）与 `include_str!` Markdown 仍在原处，只新增 `builtin_template_variables(key)` 把静态声明换成模板变量；原来的私有 `render_template` / `scan_placeholders` / `validate_variable_name` 与 `PromptTemplateError` 已删除，改为复用 core。
- **效果**：未知变量、不完整括号与缺失必需占位符的判定从两处降为一处；用户模板因此能直接使用与内建覆盖完全相同的规则。

### 11.2 用户模板

- **领域与持久化**：`PromptTemplate` / `NewPromptTemplate`（core）与 `prompt_templates` 表（schema 24，软删除）。创建/修改携带 `command_id`（幂等回执）与 `expected_revision`（乐观并发），二者都参与 payload hash。
- **存储边界重校验**：`create_prompt_template` / `update_prompt_template` 都在事务之前调用 `NewPromptTemplate::validated()`，客户端无法靠本地校验绕过未知变量、缺失必需占位符或不完整括号的限制。
- **预览统一路由**：`POST /api/prompts/preview` 改为 `builtin` / `template` 两个带标签的变体，共用一条路由；模板变体未提供的占位符回落到声明的示例值，缺少值又无示例时报 `MissingRuntimeVariable`。
- **界面**：`TemplatePanel` 提供列表、编辑与预览；`TemplateInserter` 把服务端渲染结果插入章节指导（`ChapterReadyPanel`）与意见草稿（`ChapterBodyPanel`），渲染规则因此与保存时完全一致。

### 11.3 可回算 Dashboard

- **无累计值**：新增的 `GET /api/dashboard` 每次都从已有记录重算，没有统计表、没有计数器。真实数据库 `.tables` 中不存在任何 stat/counter 表。
- **Token**（`nekowriter-store/src/dashboard.rs`）：读区间内已封存请求的 `usage_json`，按 Run 类型分为写作/审核，按用户时区偏移分本地日；`cached_input_tokens` 与 `reasoning_tokens` 作为子集单列，未报告用量的请求进入 `unknownUsageRequests`。适配器新增 `completion_tokens_details.reasoning_tokens` 映射，因此思考用量是真实值而不是占位。
- **任务**：顶层 Run 按状态分区（排队/运行/等待/已暂停），等待再按工作流当前执行轴分成等待用户与等待审核；中断与失败进入需处理；审核子 Run 展开显示且不进入主计数。
- **字数**：由 `document_revisions` 的修订链按（文档，修订号）排序后取相邻差值，复制稿本（`created_by = "copied"`）计 0 且不扰动后续基准；`currentTotal` 取当前可见正文修订的存量。
- **时间区间**：`DashboardRange` 负责时区→UTC 左闭右开区间与日期分桶，前端不猜测「今天」。

### 11.4 本次实测

| 验证项 | 结果 |
| --- | --- |
| `cargo test --workspace` | ✅ 145 个测试通过（agent 29 / app 37 / core 23 / store 56） |
| `cargo fmt --all -- --check` | ✅ 通过 |
| `cargo clippy --workspace --all-targets` | ✅ 新增/修改文件只产生与既有代码同类别的 pedantic 提示，无新类别、无编译错误 |
| `tsc -b && vite build` | ✅ 通过 |
| `pnpm lint` | ⚠️ 与基线一致：8 个既有错误与 1 个警告；新增的 `TemplatePanel.tsx` 与 `DashboardPanel.tsx` 零诊断 |
| 模板自动化 | ✅ store：CRUD 往返、修订冲突拒绝、未知变量与缺失必需占位符被拒且不留记录；core：非递归替换、`\{{...}}` 字面量、示例回落、重复变量声明拒绝；app：模板预览的正/负路径 |
| Dashboard 自动化 | ✅ store：空库零值结构、真实记录回算 Token 与字数增量（含复制计 0）、重复查询完全一致、等待分区与审核子 Run 展开、项目筛选 |
| HTTP 冒烟：模板（真实进程） | ✅ 空列表 200；创建 201；未声明变量 422 `INVALID_PROMPT_TEMPLATE`；缺失必需占位符 422；同一 `commandId` 重放返回原结果；更新到 rev 2 后再用 rev 1 返回 409 `REVISION_CONFLICT`；删除后列表为空、读取 404 |
| HTTP 冒烟：模板预览 | ✅ `{"kind":"template",...}` 带 camelCase 值渲染为「保持冷静。」；省略值回落到示例「保持克制。」；未知变量 422；`{"kind":"builtin"}` 内建预览仍正常 |
| HTTP 冒烟：Dashboard | ✅ `timezoneOffsetMinutes=480&days=7` 返回完整零值结构；`days=365` 返回 422 `INVALID_DASHBOARD_QUERY`；创建项目后按 `projectId` 过滤返回该项目的空统计；两次查询除 `generatedAtMs` 外完全一致；`sqlite3 .tables` 确认没有任何统计表 |

### 11.5 本次未做

- 概览与模板以工作台侧边栏面板接入，没有新增方案 §6 的顶部主导航；Dashboard 的任务分项也还没有「点击回到对应章节」的跳转。
- 嵌入调用的 Token 用量未落库，Dashboard 的 Token 只统计对话请求。
- 模板仅服务手动插入章节指导与意见草稿，尚未作为阶段提示词的变量替换来源参与自动流程。

## 12. 2026-09-12 增量更新：概览可信性与用量口径（§12.1 收口）

四维审计实测出的三个缺陷都在本轮修完：概览用错了刷新信号、连接测试的用量彻底消失、§12.1 要求的 `usage_source` / `usage_status` 只存在于文档里。

### 12.1 用量口径只剩一个定义处

- **`nekowriter-core/src/usage.rs`**：`PersistedModelUsage` 现在由 `UsageUnknownReason`（`provider_silent` / `request_failed` / `request_cancelled` / `pending` / `invalid_report`）同时决定 `usage_status` 与中文说明，未知记录只能由 `PersistedModelUsage::unknown(reason)` 构造，因此「状态是未知、却说不出原因」在类型上不可表达。没有报告的计数是 `null` 而不是 `0`，测试断言了这一点。
- **适配器是唯一写入点**：`ModelCompletion::persisted_usage()`（`nekowriter-agent/src/provider.rs`）把 `ModelUsage` 与原始字段一并封存，scheduler 与连接测试共用它，删掉了 scheduler 里那份重复映射。
- **删除**：store 的 `usage_number`（按字符串字段名手解析 `usage_json`）及其测试一并移除，Dashboard 改为反序列化 `PersistedModelUsage`。provider 改字段名时不再会静默把统计算成 0。
- **没有记录的请求也有原因**：`nekowriter-store/src/dashboard.rs` 的 `persisted_usage` 按请求终态（`succeeded` → 供应商未报告，`failed`/`unknown` → 请求失败，`cancelled` → 被取消，`prepared`/`dispatched` → 尚未结束）补出原因；损坏的 `usage_json` 标为 `invalid_report` 而不是让整份统计失败。

### 12.2 连接测试作为系统操作落库

- **新表**：`system_model_operations`（schema 25，实体 `entity/system_model_operation.rs`，读写集中在 `store/system_operations.rs`）。连接测试这类请求不属于任何稿本，因此不能写进以 Run 为主键的 `model_requests`，也不应该消失。
- **写路径**：`Store::record_system_model_operation` 在同一事务内写入明细并广播 `system.model_operation` 事件。`test_model_connection` 在成功与失败两条路径上都调用它（`persist_connection_test`），成功时留下供应商报告的用量与原始字段，连接/超时失败标为 `unknown`、其余失败标为 `failed`，并保留 `{"kind":"provider_error","message":…}`；随后才把原始错误映射成 API 错误返回。
- **读路径**：`Store::dashboard_system_operations` 汇总区间内的计数、子集用量、未知原因与最近 5 条明细，接进 `DashboardTokens.system_operations`；系统操作不进入对话 Token 的 `by_role` / `daily`。

### 12.3 概览订阅真正的写作事件

- **修掉误接**：`App.tsx` 之前把 Dashboard 的 `refreshToken` 接在 `knowledgeRefreshToken` 上（只有知识索引变化才 +1）。现在任何持久事件（`run.changed`、`document.committed`、`review.changed`、`feedback.changed`、`system.model_operation` 等）都会递增独立的 `metricsRefreshToken`，因为任一事件都可能改变 Token、任务或字数。
- **零数据不再装样子**：`DailyBars` 在全零时改成一行说明，不再渲染一整块不可见的柱状区；审核子 Run 的阶段与状态改由 `reviewStageLabels` / `runStatusLabels` 映射成中文，不再把 `review.preparation.background：running` 直接显示给用户。
- **陈旧列表**：模板 CRUD 已经发 `prompt_template.changed`，但没有任何界面消费它；现在 `App.tsx` 递增 `templatesRefreshToken` 并透传到 `ChapterReadyPanel` / `ChapterBodyPanel` → `TemplateInserter`，插入器不再停留在挂载时的那份列表。
- **系统操作也能触发重算**：设置面板与连接测试同一次操作的两个入口（`ModelSettingsPanel`、`ProjectSettingsPanel`）都接受 `onModelOperation`，在请求结束后通知概览重新读取，避免「没有选中稿本 → 没有事件流 → 测试完看不到」的死角。

### 12.4 本次实测

| 验证项 | 结果 |
| --- | --- |
| `cargo test --workspace` | ✅ 150 个测试通过（agent 29 / app 37 / core 26 / store 58） |
| `cargo fmt --all` / `cargo clippy --workspace --all-targets` | ✅ 新增与修改文件无新类别的 clippy 提示；已清掉 `unnecessary_wraps` 与 `needless_pass_by_value` |
| `pnpm typecheck` / `pnpm build` / `prettier --check`（改动文件） | ✅ 通过；`pnpm lint` 与基线一致（8 错误 1 警告） |
| 用量口径自动化（store / core） | ✅ 无记录时按 `succeeded`/`failed`/`cancelled`/`dispatched` 分别得到四种原因；损坏的 `usage_json` 得到 `invalid_report` 而不报错；未知状态字符串仍报错；缺失计数序列化为 `null` 而非 `0`；原因与状态的 snake_case 名稳定 |
| 系统操作自动化（store） | ✅ 两次连接测试（一次报告用量、一次失败）汇总为请求 2 / 输入 12 / 输出 3 / 缓存 4，最近一条在前且 `errorKind` 可见；区间外记录被排除；`compute_dashboard` 的 `tokens.request_count` 仍为 0 而 `system_operations.request_count` 为 2 |
| HTTP 冒烟：连接测试落库（真实进程 + 本地 SSE 桩） | ✅ `PUT /api/settings` 后用本地桩调用 `POST /api/settings/test-model` 返回 `usage.inputTokens=21`；`GET /api/dashboard` 的 `systemOperations` 显示 `requestCount 1 / inputTokens 21 / outputTokens 7 / cachedInputTokens 5`、`usageStatus: reported`、`errorKind: null`，而对话 Token 全部为 0 |
| 真实页面探针：活动后自动变化 | ✅ 保持页面打开、全程不点击「刷新」：桩停掉后发起一次准备请求，页面自行从「请求 0 / 用量未知 0 / 需要处理 0」变为「请求 1 / 用量未知 1」并显示原因「请求失败，没有收到可用用量」、写作角色行与「需要处理 1」；同一页面的系统操作区仍显示输入 21 / 输出 7 / 请求 1 |
| 真实页面探针：列表不再陈旧 | ✅ 保持页面打开，通过 API 新建模板后，「选择模板」下拉自行出现「雨幕·氛围句」 |
| 边界 | ⚠️ `TemplateInserter` 只在章节阶段挂载，本轮探针没有跑到该阶段，因此它的实时刷新是按同一事件接线成立、未经真实界面验证；概览的事件订阅依赖选中稿本的游标，没有任何项目时系统操作要等下一次事件或手动刷新 |

### 12.5 本次未做

- 嵌入调用的 Token 用量仍未落库（§12.1 的「Embedding 输入用量独立列示」因此尚未满足）。
- 概览的读取形状仍是「把区间内的 runs / requests / revisions 读进内存再汇总」，没有 SQL 聚合与分页；本地规模够用，但不是长期正确的形状。
- 仍未拆分 `nekowriter-store/src/preparation.rs`（命令、审核、召回与工具的巨大清单）、`nekowriter-app/src/scheduler.rs` 与 `nekowriter-store/src/lib.rs` 这三块巨石；审计指出的「同一份知识有两个所有者」已通过 `PersistedModelUsage` 单一写入点解决，但巨石本身仍在。

## 13. 2026-09-12 增量更新：稿本正文导出（M5 第一项能力）

M5 的清单里「长篇虚拟化 / 主题 / 键盘体验」排在导出之前，而导出是唯一在细则里有完整接口与行为约定的那一项（§11.4 与路由表），因此本轮按用户确认选择先做导出；M5 的复选框保持未勾选，因为同一行还包含软删除恢复、退出流程与发行包。

### 13.1 内容只来自已提交修订链

- **核心渲染**（`nekowriter-core/src/export.rs`）：`ExportFormat`（`md` / `txt`）、`ExportChapter`、`VersionExport` 与 `render()`。渲染是纯函数，不碰数据库，也能单独测试版式与草稿标记。
- **存储读取**（`nekowriter-store/src/export.rs`）：`Store::build_version_export` 在**一个只读事务**里读完稿本、章节、正文文档与当前修订，读完即回滚。SQLite 以 WAL 运行，因此同一个事务看到的是一份一致快照，不存在把不同时刻的章节拼在一起的可能；也没有导出缓存或另存副本。
- **定稿判定**：只有 `chapter.approved_revision_id == document.current_revision_id` 的章节算定稿。该判定同时覆盖两种情况：从未通过的章节，以及通过整章审核之后又被编辑过的章节。
- **顺序**：正文按 `order_index` 在前，番外按自身 `order_index` 在后，同序用章节 id 稳定排序（`list_chapters` 的展示顺序是按 kind 字符串排的，导出没有沿用它）。

### 13.2 文件名与响应

- **消毒**：`sanitize_file_stem` 去掉路径分隔符、控制字符与 Windows 保留字符，折叠空白与连续点号，剪掉首尾点号并限长 80；结果为空时回退 `nekowriter-export`。因此结果不会出现 `..`、不以点开头、也不含任何路径分隔符（对照测试里对多个恶意标题做了断言）。
- **下载头**：`content-disposition: attachment; filename="nekowriter-export.md"; filename*=UTF-8''%E9%9B%A8%E5%B9%95.md`——ASCII 回退名加上 RFC 5987 的 UTF-8 名，中文标题因此能原样落盘。
- **前端**：`downloadVersionExport` 用 fetch 拿到正文与文件名，再以 `Blob` + `a[download]` 交给浏览器；错误走与其它接口相同的 `toApiError`（为此把错误解析从 `request()` 里抽成了共用函数）。

### 13.3 本次实测

| 验证项 | 结果 |
| --- | --- |
| `cargo test --workspace` | ✅ 157 个测试通过（agent 29 / app 37 / core 31 / store 60） |
| `cargo fmt --all` / `cargo clippy --workspace --all-targets` | ✅ 新增文件无新类别的 clippy 提示 |
| `pnpm typecheck` / `pnpm build` / `prettier --check`（改动文件） | ✅ 通过 |
| core 单元测试 | ✅ Markdown 与纯文本的标题层级、草稿后缀、空章节提示、空稿本的「本稿本还没有章节」、UTC 时间行可复现、`md`/`txt` 之外的取值被拒；文件名消毒对 `../../etc/passwd`、`..`、`.hidden`、`a/../b` 等标题断言不含分隔符、不以点开头、不含 `..` |
| store 集成测试 | ✅ 正文在前、番外在后且番外独立编号；当前修订 ≠ 通过修订的章节标为草稿；无正文文档的章节内容为空；软删除的章节不再导出；未知稿本返回 `VersionNotFound`；无章节的稿本导出标题与说明行 |
| 真实进程：造出有正文的稿本 | ✅ 用真实 API 走完准备四产物（含两项章节计划）、四项准备审核、第一章细纲与审核、两次 `body/append`、整章终稿审阅并通过；第一章 `approved` 且 `approvedRevisionId` 等于当前修订，第二章 `planned` 无正文 |
| 真实进程：从界面发起导出 | ✅ 工作台「稿本」面板点击「导出 Markdown」得到文件名 `雨幕.md`、体积 397 字节，内容是 `# 雨幕` + 两章；点击「导出纯文本」得到 `雨幕.txt` 且不含 `#`；两次都真实请求了 `GET /api/versions/{id}/export?format=…`（200） |
| 产物与库一致 | ✅ 导出正文与 `sqlite3` 里 `documents ⋈ document_revisions` 的当前修订逐字一致（脚本断言 `content in export` 为真，草稿标记出现 1 次，对应未在写的第二章） |
| 边界（真实进程 + HTTP） | ✅ `format=pdf` 与缺少 `format` 都返回 422 `INVALID_EXPORT_FORMAT`；随机不存在的稿本返回 404 `RESOURCE_NOT_FOUND`；新建的空稿本（0 章）从界面导出得到 `尚未开篇.md`，内容为标题 + `正文 0 / 0 章已通过整章审核` + `本稿本还没有章节。` |
| 边界说明 | ⚠️ 真实探针只遇到正文两章，**番外排列与番外编号只有 store 层用例覆盖**；导出不写服务端文件，因此没有导出历史 |

### 13.4 本次未做

- 导出没有章节级选择（不能只导某一章），也没有 EPUB/PDF；细则只要求 Markdown/TXT。
- 番外下的真实界面验证缺失（见上表边界说明）。
- 长篇虚拟化、主题/键盘体验、回收站恢复、退出流程与发行包仍属于 M5 后续。

## 14. 2026-09-12 增量更新：长篇阅读、虚拟化与阅读位置（M5 第二项能力）

### 14.1 顺序与命名只有一份

M5 的阅读区必须和目录、导出回答同一个问题（这部作品由哪些章组成、按什么顺序读、每章叫什么）。此前导出自己写了一套排序与番外编号，目录用 SQL 的 `kind` 排序（`extra` 在字母序上排在 `main` **之前**，即番外在目录里排到了正文前面）。现在 `nekowriter-core::manuscript` 定义唯一一份规则：`manuscript_sort_key`（正文在前、番外在后）、`ChapterNamer`（番外按自身出现次序编号）、`ManuscriptChapter`（章快照与草稿标记）。`store/manuscript.rs` 的 `load_manuscript_index` 是唯一的章节索引读取，`store/chapters.rs` 的目录与导出都从它派生，`export.rs` 缩减为“读取快照 → 渲染”。`ChapterSummary` 因此新增了服务端给出的 `label` 与 `bodyWordCount`。

### 14.2 阅读区、窗口与定位

`webui/src/components/ManuscriptReader.tsx` 在一个连续滚动区里按章虚拟化（TanStack Virtual）：只挂载视口附近章节（overscan 2），正文按 20 章的窗口向 `GET /api/versions/{id}/manuscript` 索取，因此打开成本与总章数无关。定位不使用未挂载 DOM 的 `scrollIntoView()`：`pinTo` 先 `scrollToIndex` 按虚拟下标滚动，渲染后逐步校正到目标块顶与视口顶对齐；末尾章节被滚动范围夹住时接受可达位置。阅读位置以「章 id + 块内偏移」写入浏览器本地存储（`nekowriter.reading-anchor.{versionId}`），滚动时按帧合并写入、离开阅读区时补写一次，重新打开时恢复。字号变化会 `measure()` 丢弃高度缓存并把当前章重新对齐到顶部。`ChapterDirectory` 改成以章节 id 作 key 的虚拟列表，`ChapterReader.tsx` 被阅读区取代并删除。

### 14.3 整稿查找

窗口化列表的代价是浏览器自带页内查找只能看到已挂载的章节。为此新增 `GET /api/versions/{id}/manuscript/search?q=`：服务端读遍全部已提交正文（与导出同一份快照）逐字匹配，仅对 ASCII 字母不区分大小写（这种折叠不改变字符数，偏移永远落在正确位置），命中不重叠、最多 50 条，返回 `start_char` / `end_char`（Unicode scalar、左闭右开，与引用同一单位）与摘要。阅读区把命中跳到的章节高亮成 `<mark>`，因此查找结果、高亮与引用校验共用同一套位置语义。

### 14.4 探针发现并修掉的两个真实缺陷

这两个都不是读代码看出来的，是真实进程里量出来的：

1. **加载占位被当成章节高度。** 章节未加载时渲染占位块，`measureElement` 把占位高度（约 192px）写成这一章的尺寸，于是未加载区域的几何逐帧塌缩，跳到靠前章节时落到了第 1 章（滚动位置 384px）而不是第 3 章（4178px）。修法是只测量真正呈现出来的章节（`ref={content ? measureElement : undefined}`）：未挂载章节保持来自字数的估计值，加载后再用真实高度替换。
2. **挂载时恢复位置与定位请求打架。** 打开阅读区时，“恢复上次位置”与“用户刚点的那一章”会在同一次挂载里先后发出两个滚动目标，目的地互相干扰。现在挂载只恢复一次，且用户显式目标优先；此外位置校正以**真实滚动位置是否对齐**作为收敛条件，而不是“估计尺寸连续两帧不变”，并只在仍在前进时延长期限。

### 14.5 本次实测

| 验收面 | 结果 |
| --- | --- |
| 真实长篇数据 | ✅ 直接用 SQL 向探针库写入一部 401 章的作品（正文 400 章 + 番外 1 章、正文约 80 万字符，两章计划中未写正文）与一部 51 章的短篇，经界面读取 |
| 渲染与章数脱钩 | ✅ 401 章作品挂载 6 块、阅读区 49 个 DOM 节点、打开耗时 43ms；51 章作品挂载 3 块、25 个节点、39ms——总章数涨约 8 倍，渲染开销基本不变 |
| 定位未加载章节 | ✅ 目录跳到第 390 章、从写作视图跳第 3 章、查找命中第 380 章、跳末章番外，四处都在正文未加载时发起；前三处目标块顶与视口顶相差 1px，末章停在滚动范围夹住的可达位置 |
| 阅读位置 | ✅ 以「第 368 章 + 偏移 80px」保存在本地存储，重新加载页面后再打开阅读区仍在同一位置（±1px）；写作/阅读视图切换同样保持；字号 +1 再 -1 后仍停在第 368 章 |
| 内容与库一致、可复制 | ✅ 已挂载章节的正文与 `/manuscript` 返回的当前修订逐字相同；程序化选中整段得到 1886 字符与原文等长；两个未写正文的计划章节显示「（本章尚无正文）」 |
| 查找 | ✅ 在 401 章中查到 3 处「孤灯」（都命中第 380 章同一段文字），点击结果后定位到该章（1px）并高亮 2 个字符 |
| 自动化 | ✅ `cargo test --workspace` 167 通过（agent 29 / app 37 / core 37 / store 64），`cargo fmt --check`、`tsc -b && vite build`、改动文件 `prettier --check` 通过 |
| 边界 | ⚠️ 未挂载章节的高度是按字数估计的（实测估计值比真实高度小约 12%），滚动条长度在整部作品被测量过之前是近似的；探针没有逐章核对全部 401 章的内容 |

### 14.6 本次未做

- 主题/键盘与中文输入体验、回收站恢复、退出流程与脱离源码的发行包仍属于 M5。
- 阅读区的「章节指导与细纲」展开只接了按需读取，探针没有逐章展开验证。
- 阅读位置刻意保存在浏览器本地而不是服务端：它是展示状态，不是作品数据。

## 15. 2026-09-12 增量更新：退出流程与脱离源码的发行目录（M5 第三项能力）

### 15.1 退出只有一个信号，且必须写进同一个信号

退出触发有三条：Ctrl+C、SIGTERM、`POST /api/system/shutdown`。它们都汇到 `crates/nekowriter-app/src/lifecycle.rs` 的 `Shutdown`（`tokio::sync::watch`）。两条细节值得单独说：

- `request()` 用 `send_replace` 而不是 `send`。`watch::Sender::send` 在没有接收者时既不更新值也不唤醒；退出信号必须先于等待者成立，否则「退出请求发出时还没有任何订阅者」就会静默丢失。
- 信号路径在返回前必须调用 `request()`。第一版没有：SIGTERM 让 `serve` 的优雅退出开始，但 watch 里的值仍是 `false`，于是已经打开的 SSE 连接看不到退出、不会结束，axum 一直在等持久连接断开。真实进程探针抓到了这个：不带浏览器连接时 SIGTERM 0.1 秒退出，浏览器页面开着（SSE 存活）时进程一直不退。现在 `wait()` 无论由哪条触发唤醒都会写回同一个信号并记录触发来源。

退出顺序（`main`）：服务器停止接受新连接 → `WorkflowScheduler::cancel_in_flight()`（abort 本地 HTTP 任务、最多等 5 秒、丢弃命令循环）→ `Store::interrupt_active_runs(ProcessShutdown)` → `Store::close()`（`PRAGMA wal_checkpoint(TRUNCATE)` + 关闭连接池）→ 退出码 0。

### 15.2 启动恢复与退出收尾只有一条路径

`recover_interrupted_writing_runs` / `recover_interrupted_review_delegations` 原来硬编码了「服务重启」的文案与 `startup_recovery` 事件原因。现在它们是 `interrupt_active_writing_runs` / `interrupt_active_review_delegations`，由 `InterruptionSource`（`StartupRecovery` / `ProcessShutdown`）决定事件 `reason` 与用户可见文案，`Store::interrupt_active_runs` 是唯一入口：`Store::open` 用它做启动恢复，退出流程用它做收尾。同一批事务里把活跃 Run 标 `interrupted`、prepared 请求标 `cancelled`、dispatched 请求标 `unknown`，并广播 `run.changed` / `workflow.changed`。

### 15.3 日志、浏览器与前端资源位置

- `lifecycle::init_logging` 在数据目录建 `logs/`，写 `nekowriter.<日期>.log`：按天滚动、`max_log_files(7)`、文件不带 ANSI、标准输出与文件同时写；`WorkerGuard` 由 `main` 持有到退出。日志在拿到数据目录后、打开数据库前初始化，因此启动与迁移诊断都进文件。
- 绑定端口成功后用 `webbrowser` 打开 `http://127.0.0.1:{port}/`，失败只记日志；`--no-browser` 关闭。
- `default_web_dir()` 从 `current_dir()/webui/dist` 改为**可执行文件旁的 `webui/dist`**：发行目录整体拷走后任何工作目录都能打开界面，源码树不再参与运行期路径。源码内开发用 `--web-dir webui/dist` 或 Vite。
- `GET /api/health` 增加 `dataDir`，界面不再需要用户猜数据在哪里。
- 新增 `crates/xtask`（`cargo xtask package [--debug] [--protoc <路径>]`）：构建前端 → 构建 `nekowriter-app` → 组装 `target/package/nekowriter/`（可执行文件 + `webui/dist` + `README.txt`）。`.cargo/config.toml` 的 `PROTOC` 是 `force = true` 的 Windows 路径，所以非 Windows 平台用 `--protoc` 转发 `--config env.PROTOC.value=…`。

### 15.4 界面：本地服务面板与「已退出」

`webui/src/components/ServicePanel.tsx` 显示界面地址与实际数据目录、二次确认后调用 `POST /api/system/shutdown`，并说明退出保留什么、如何重新启动。退出动作不假设响应一定到达：它以「后端还能不能应答健康检查」为准。事件同步断开时（`EventSource.onerror` 或首次读取失败）先探一次健康检查，只有确认服务不可达才切到「本地服务已退出」并停止重连——服务可能在终端里被 Ctrl+C 掉，那时界面必须说清楚，而不是一直显示正在重连。

### 15.5 本次实测

| 验收面 | 结果 |
| --- | --- |
| 脱离源码的发行目录 | ✅ `cargo xtask package --debug` 产出的目录整体拷到 `/tmp/nw-relocate`，以该目录为工作目录启动：首页 200（498B）、引用到的 JS 200（455KB），`webui/dist` 按可执行文件旁解析 |
| 数据与日志位置 | ✅ 指定数据目录里恰好是 `nekowriter.sqlite3`(+wal/shm)、`lancedb/`、`logs/`、进程锁；仓库外的目录只有发行目录、数据目录与 `app.log` |
| 真实内容 | ✅ 真实 API 建项目 → 保存目标 → 四项准备产物与细纲逐项通过 → 正文两次追加共 72 字 → 整章终稿通过，第一章 `approved`，正文与 `/manuscript` 一致 |
| 接口退出 | ✅ 在途细纲请求（`request_in_flight`，桩收到 1 条连接）+ 一条打开的 SSE 连接：`{"status":"shutting_down"}` 3ms 返回，进程 0.2 秒内退出码 0，SSE 被关闭，`-wal` 消失（已回写主库），日志有「收到界面退出请求，正在停止服务」「已取消在途模型请求 cancelled=1」「NekoWriter 已退出 writing_runs=1」 |
| 重启后数据仍在 | ✅ 项目、正文、第一章定稿原样；在途 Run 变 `interrupted`，请求 `status=unknown` 且 `error_json.kind=process_shutdown`，事件流有两条 `reason=process_shutdown` |
| 信号退出 | ✅ 带一条打开的 SSE 连接走 SIGTERM：0.1 秒内退出码 0、SSE 被关闭、日志有「收到退出信号，正在停止服务」；`pkill` 后无孤儿进程 |
| 启动打开浏览器 | ✅ 不带 `--no-browser` 启动的日志出现「已在默认浏览器打开工作台」，随后 SIGTERM 退出码 0 |
| 界面退出动作 | ✅ 在预览里看到「本地服务」面板（界面地址、`/private/tmp/nw-relocate/data`、二次确认文案），点击「退出服务 → 确认退出」后服务端日志出现退出请求并结束进程 |
| 自动化 | ✅ `cargo test --workspace` 172 通过（agent 29 / app 41 / core 37 / store 65），`cargo fmt --all -- --check`、`tsc -b && vite build`、改动文件 `prettier --check` 通过 |
| 边界 | ⚠️ 退出之后的界面状态（「本地服务已退出」渲染）没有在真实界面里截到：预览桥在服务进程结束后即断开；⚠️ 发行目录用 `--debug` 产物验证，首次 `--release` 全量构建（lancedb/arrow 冷启动）超出单次命令的时间预算、未完成，Windows x64 发行包未产出 |

### 15.6 本次未做

- 主题与键盘/中文输入体验、回收站恢复仍属于 M5；Windows x64 发行包仍未产出。
- 退出时只取消本地 HTTP 任务，已发往外部服务的调用是否执行由 `unknown` + 「用量未知」保留，不做补偿。
- 数据目录仍是单一全局目录，没有「切换作品库」入口。

## 16. 2026-09-12 增量更新：主题与键盘/中文输入体验（M5 第四项能力）

### 16.1 快捷键与组合输入只有一份判定

新增 `webui/src/lib/shortcuts.tsx`（`ShortcutProvider` + 四个注册 Hook）与 `webui/src/components/ShortcutHelpPanel.tsx`，`main.tsx` 在 `ThemeProvider` 内挂上 Provider。四条绑定：

- `Ctrl/Cmd+Enter`：光标在输入框里时提交该表单的 `type=submit` 主操作（`form.requestSubmit()`，取表单自己的校验与提交路径，不另写一份）；光标不在输入框里且注册表中**恰好只有一个**可用主操作时触发它。
- `Ctrl/Cmd+K`：聚焦注册为查找目标的元素（阅读区的整稿查找框）。
- `Esc`：关闭注册表里最上层处于活动状态的可安全取消层（查找结果、说明面板、退出确认），后注册的先关闭。
- `?`：开关快捷键说明；只在光标不在输入框里时生效。

破坏性操作（审核通过/退回、停止执行、退出服务）没有绑定：注册 Hook 每个面板只注册一个非破坏性主操作。

### 16.2 组合输入：三次修正

1. **仅看 `event.isComposing` 不够。** 部分浏览器在确认候选词的 `keydown` 里已经给出 `isComposing === false`。判定改为 `isComposing || keyCode === 229 || (组成中的目标就是本次事件目标)`。
2. **不能把状态做成一个全局布尔值。** 第一版用一个模块级 `composing` 布尔值，探针里只要出现一个**没有配对**的 `compositionstart`（输入法在页面获得焦点时可能发出），`?`、`Esc`、`Ctrl+Enter` 就全部静默失效——这是一种不会报错、只会“键盘突然不好使”的故障。现在只认「可编辑目标上的 `compositionstart`」，并记下是哪个元素在组合；窗口失焦同时清除。
3. **只 `preventDefault` 拦不住 React 表单。** 组合期间那次 Enter 属于确认候选词，需要在提交层拦下。但表单由 React 接管：`preventDefault` 只取消浏览器默认动作，`onSubmit` 仍会执行、请求照发（探针里表现为“提交事件被阻止了，但项目还是创建了”）。现在在 document 捕获阶段 `preventDefault` 与 `stopPropagation` 同时生效。

### 16.3 主题：删掉单键 `d`

`theme-provider.tsx` 的全局 `d` 监听被删除（它会在阅读和文本操作里误触，见 §13）。新增 `webui/src/components/WorkspaceControls.tsx` 作为头部控件：浅色/深色/跟随系统三段选择器（`aria-pressed` 反映当前值）与「快捷键」按钮，主题仍由 `ThemeProvider` 存在浏览器本地。

### 16.4 焦点

说明面板打开时焦点进入其关闭按钮、关闭后回到打开它的元素；退出确认出现时焦点落到「取消」而不是破坏性按钮；快捷键动作让触发它的按钮消失后，焦点回落到工作台主区域（而不是 `<body>`）；查找输入框由 ref 回调登记，不依赖重新渲染。`useEscapeDismiss` 的 id 用 `useId()`，不在渲染期间调用不纯函数。

### 16.5 本次实测

| 验收面 | 结果 |
| --- | --- |
| 组合输入不误提交 | ✅ 真实浏览器、真实事件序列，`新建项目` 表单上逐项计数（每次尝试前重新填表以避开 `required` 干扰）：组合中 Enter（含同步合成的隐式 `submit`）→ 新建项目 0；组合中 `Ctrl+Enter` → 0；组合结束后 `keyCode 229` 的 Enter → 0；组合结束后 `Ctrl+Enter` → 1；无组合状态下 `Ctrl+Enter` → 1；普通 Enter（无 Ctrl）→ 0 |
| 组合输入不误查找 | ✅ 阅读区查找框：组合中 `Ctrl+Enter` 不发起查找（结果面板不出现），组合结束后 `Ctrl+Enter` 提交并得到「找到 5 处（最多显示 50 处）」 |
| 未配对 compositionstart 不再杀死键盘 | ✅ 在 `body` 上派发一个没有 `compositionend` 的 `compositionstart` 后，`?` 仍能打开说明、`Esc` 仍能关闭 |
| 快捷键生效清单 | ✅ `Ctrl+Enter`（提交表单：新建项目、查找）、`Ctrl+K`（聚焦到 `#manuscript-search`）、`Esc`（关闭查找结果、关闭说明）、`?`（打开/关闭说明）、头部按钮（同样的开关） |
| 焦点 | ✅ 打开说明后焦点在「关闭」；`Esc` 后回到打开它的按钮；退出确认出现后焦点在「取消」 |
| 主题 | ✅ 点「浅色/深色」后 `documentElement` 的 class 与 `localStorage` 同步变化并刷新后保持；连续按 `d` 三次主题不变（`d` 已不绑定） |
| 既有写作/阅读流程 | ✅ 用 `Ctrl+Enter` 从真实表单创建项目；切到阅读视图后用 `Ctrl+K` + `Ctrl+Enter` 查到 5 处「孤灯」（内容由 SQL 预置），点结果定位并高亮，全程未手动触发任何按钮之外的额外操作 |
| 自动化 | ✅ `tsc -b`、`vite build`、改动文件 `prettier --check` 通过；`pnpm lint` 与基线一致（8 个错误、3 条警告，全部为既有项）；本轮无 Rust 改动，因此 `cargo test` 的 172 项沿用上一轮结果 |
| 边界 | ⚠️ 验证用真实事件序列而非操作系统里实际的输入法候选窗；⚠️ `Ctrl+Enter` 的「唯一阶段主操作」分支只登记了「开始准备」与「显式继续」 |

### 16.6 本次未做

- 回收站恢复与 Windows x64 发行包仍属于 M5。
- 「唯一阶段主操作」没有逐阶段登记；两个主操作同时可用时保守地都不触发。
- 没有独立的全局设置页：主题只在头部选择器里，字号仍在阅读区。
## 17. 2026-09-12 增量更新：回收站恢复（M5 第五项能力）

### 17.1 恢复只清 `deleted_at`，不写新修订

`nekowriter-store::recycle` 是回收站唯一的实现处：`delete_extra_chapter`、`delete_version`、`get_recycle_bin`、`restore_recycle_item`、`purge_recycle_item`。删除只把 `deleted_at` 写回去，章节行、文档槽与全部修订都不动，因此恢复时正文、`revision_id` 与 `order_index` 还是删除前那一份——不会为了「恢复」再插一条修订（那会让历史、引用与统计跟现实对不上）。

恢复一份番外时先把被它占用的顺序号让出来（从高到低把同稿本、同类别、序号不小于它的行依次上移），因此删除期间新增的番外不会撞上唯一键，也不会把恢复的章节排到末尾。恢复要求祖先仍然可见：稿本还在回收站里时返回 409 `RECYCLE_ANCESTOR_DELETED`，对象不存在或已被彻底删除时返回 404，不做半恢复。

删除番外时 `withdraw_extra_chapter` 在同一事务里把工作流退回到仍然可见的最后一章；恢复**不把写作光标搬回去**——恢复只恢复数据与位置，光标保持删除后的位置（删除之后用户可能已经在别处继续写）。这条语义在 store 用例里有断言，并写进 docs/02 §8.4。

### 17.2 彻底删除只对模板与知识对象开放

稿本与正文为保留审计链只留在回收站里，`purge` 返回 409 `RECYCLE_NOT_PURGEABLE`；可物理删除的用户模板、知识条目与空 Space 才有「彻底删除」路径，删除后不可恢复（再恢复是 404）。分类原则是：正文与它的事件、审核和修订引用共同构成审计链。

### 17.3 界面

新增 `webui/src/components/RecycleBinPanel.tsx`：按删除时间倒序列出类别、标题、删除时间、原属稿本与阻塞原因，提供「恢复」与（仅对可物理删除的对象）二次确认的「彻底删除」；`restorable` / `purgeable` 与原因都来自服务端，界面不重新推断。`App.tsx` 订阅 `recycle.changed` 刷新面板；`ExtraChapterPanel` 列出已有番外并提供带二次确认的删除入口，`VersionPanel` 对非当前稿本提供同样的入口。

### 17.4 探针发现并修掉的一个真实缺陷

番外列表原本以 `workflow.stateRevision` 作刷新键。删除会改变工作流修订（`withdraw_extra_chapter`），所以删除后列表会更新；但**恢复不改变工作流修订**，于是从回收站恢复一章后，番外列表仍是空的——这是探针在真实界面里发现的，不是读代码看出来的。改为以服务端重读出来的准备快照本身作刷新信号（`preparation` 对象身份），因为它随每次重读被替换，而不是它的某一个字段。

### 17.5 本次实测

| 验收面 | 结果 |
| --- | --- |
| 删除后回收站可见 | ✅ 真实进程里删除番外（18 字正文）：`GET /api/recycle-bin` 返回类别 `chapter`、标题「尾声 · 雨停之后」、删除时间 `1789204468274`、原属「雨停之后（初稿）」、`detail`「18 字正文 · 所属稿本…」、`restorable=true`、`purgeable=false` 与不可物理删除的原因 |
| 界面看到它并恢复 | ✅ 预览里「回收站」面板显示「1 项已删除 · 章节 · 尾声 · 雨停之后 · 2026/9/12 17:14 删除 · 原属稿本《雨停之后（初稿）》· 18 字正文」，点击「恢复」后面板变为「回收站是空的 / 已恢复章节「尾声 · 雨停之后」」，目录重新出现「番外 1 尾声 · 雨停之后」 |
| 恢复复用原来的内容与位置 | ✅ 恢复后 `revisionId` 与删除前逐字一致（`f9cec71f-…`）、正文一致、`orderIndex` 不变（`1`）、状态仍是 `approved`、`bodyWordCount` 一致（18） |
| 删除入口也走得通 | ✅ 从界面「番外」列表点「删除 → 确认删除」后，列表清空、`GET /chapters` 不再包含它、回收站重新出现同一条记录 |
| 彻底删除后不可恢复 | ✅ 用户模板：创建 → 删除进回收站（`purgeable=true`）→ `purge` 200 → 再 `restore` 返回 404 `RESOURCE_NOT_FOUND` |
| 正文不能物理删除 | ✅ `POST /api/recycle-bin/chapter/{id}/purge` 返回 409 `RECYCLE_NOT_PURGEABLE`，记录仍在回收站里 |
| 恢复不存在的项 | ✅ 重复 `restore` 同一番外返回 404 `RESOURCE_NOT_FOUND` |
| 自动化 | ✅ `cargo test --workspace` 173 通过（agent 29 / app 41 / core 37 / store 66），`cargo fmt --all -- --check`、`tsc -b && vite build`、改动文件 `prettier --check` 通过；`pnpm lint` 与基线一致（8 个错误、3 条警告，全部为既有项） |
| 边界 | ⚠️ 探针里的番外章与正文是用 SQL 预置的真实内容（与长篇阅读那轮同一做法），删除/恢复/彻底删除本身全部走真实 HTTP 接口与真实界面；⚠️ 项目本身的软删除未做 |

### 17.6 本次未做

- §8.4 里列的「项目」还没有对应的删除与恢复入口。
- 恢复不把写作光标搬回被恢复的章节，用户需要点一下目录才能回到它。
- Windows x64 发行包仍未产出。

## 18. 2026-09-12 增量更新：项目级回收站（M5 第五项能力的收口）

### 18.1 只标记项目本身，孩子不动

`Store::delete_project` 只写项目自己的 `deleted_at` 与 `revision`，不批量改写任何孩子的状态。这是 §8.4 定下的规则，代价与收益都很具体：代价是每条读取路径都必须自己检查祖先（这一版已经如此——`list_projects` / `get_project` / `list_versions` / `get_version_snapshot` 等全部带 `project.deleted_at IS NULL` 或经 `project_exists` 校验），收益是恢复项目时不需要「猜」哪些孩子该活过来：项目自己的标记是唯一开关，删除项目之前单独删掉的稿本仍然留在回收站里，不会被恢复项目顺手复活。

这条语义用 store 用例锁住：在待删项目里先单独删掉一个稿本，再删项目，再恢复项目，断言被单独删掉的稿本仍在回收站里、而项目当前稿本原样可见（`get_version_snapshot` 的 `version` 与删除前相等）。

### 18.2 忙检查与修订检查

`project_has_active_work` 是单稿本判定的项目级版本：只要项目下任一稿本仍有 `open`/`delegated` 审核，或仍有 `queued`/`running`/`waiting`/`paused`/`interrupted` 的顶层 Run，删除就被拒绝为 `RESOURCE_BUSY`。过期修订返回 `REVISION_CONFLICT`（复用 select_version 对项目修订的同一错误），命令回执仍然生效——同一条 `command_id` 重放返回原结果，不会重复删除。

### 18.3 回收站里的项目

`RecycleItemKind::Project` 与其余类别同形：`restorable` 恒为真（项目没有祖先），`purgeable` 为假并给出理由（项目下的稿本、正文、事件、审核与修订引用共同构成审计链），`detail` 给出「N 份稿本 · M 章」，`parent` 为空。界面不需要为它写特殊分支——`RecycleBinPanel` 只多了 `project: "项目"` 一个标签。

### 18.4 探针发现的第三个陈旧数据缺陷

`RecycleBinPanel` 的 `onChanged` 原本只调用 `bumpChapters()`。恢复/彻底删除一个**项目**并不改变章节目录，于是项目列表不会重读——从回收站恢复一个项目后，界面里仍然只有原来那个项目，要点一下刷新才出现。这是探针在真实界面里点「恢复」之后发现的（`/api/projects` 已经返回两个项目，而渲染出来的卡片只有一个）。现在 `App` 增加 `refreshProjects()`（重读列表但不切 `loading` 占位，已有内容不消失），面板的 `onChanged` 同时重读项目与目录。

### 18.5 本次实测

| 验收面 | 结果 |
| --- | --- |
| 删除项目 | ✅ 真实进程：两个项目 → 待删项目里再建一个稿本并单独删掉 → `DELETE /api/projects/{id}` 200，`GET /api/projects` 只剩一个，`GET /api/projects/{被删}` 404 |
| 回收站里的项目 | ✅ 类别 `project`、标题「待删项目」、`deletedAtMs` 有值、`parent` 为空、`detail`「2 份稿本 · 0 章」、`restorable=true`、`purgeable=false` 与不可物理删除的原因 |
| 祖先规则 | ✅ 被删稿本显示 `restorable=false` 与「所属项目仍在回收站里，请先恢复它」；直接恢复它返回 409 `RECYCLE_ANCESTOR_DELETED` |
| 不可物理删除 | ✅ `POST /api/recycle-bin/project/{id}/purge` 返回 409 `RECYCLE_NOT_PURGEABLE`，被拒后记录仍在回收站 |
| 修订与幂等 | ✅ 用修订 0 删除返回 409 `REVISION_CONFLICT` 且项目仍在；同一条 `command_id` 重放返回相同的回收站快照 |
| 恢复项目 | ✅ 200，项目列表恢复为两个；恢复后 `GET /api/versions/{当前稿本}/snapshot` 200 且 `novelTitle` 不变；**删除项目之前单独删掉的稿本仍在回收站里**，单独恢复它也 200，项目下两个稿本都在 |
| 恢复不存在的项目 | ✅ 404 `RESOURCE_NOT_FOUND`（不是假装成功） |
| 界面看到它并恢复 | ✅ 预览里「回收站」面板显示「1 项已删除 · 项目 · 待删项目 · 删除时间 · 2 份稿本 · 0 章 · 恢复 · 不能彻底删除：…」；点击「恢复」后面板变为「回收站是空的 / 已恢复项目「待删项目」」，**项目卡片同时重新出现（无需手动刷新）** |
| 界面删除入口 | ✅ 项目卡片下的「删除项目 → 确认删除项目」：`/api/projects` 只剩一个、渲染的卡片只剩一个、回收站重新出现同一条 |
| 自动化 | ✅ `cargo test --workspace` 174 通过（agent 29 / app 41 / core 37 / store 67），`cargo fmt --all -- --check`、`tsc -b && vite build`、改动文件 `prettier --check` 通过；`pnpm lint` 与基线一致（8 个错误、3 条警告，全部为既有项） |
| 边界 | ⚠️ 项目与稿本的删除都还没有「归档」或批量整理入口；⚠️ 恢复项目不会把工作流的写作光标搬回任何章节（与章级恢复同一语义） |

### 18.6 本次未做

- 删除没有多选或归档；项目与稿本都要一个一个删。
- 项目删除不检查「项目下所有稿本都已完成」——只要没有活跃任务就可以删，这与 §8.4 一致（删除是软删除，内容仍在）。
- Windows x64 发行包仍未产出。

## 19. 2026-09-12 增量更新：积累引擎的成功写入路径（M4 收口）

### 19.1 先读规范，再决定“做”到底意味着什么

docs/01 把“积累引擎的成功写入路径”列为 M4 剩余项，但 docs/02 §7.4 与 docs/01 §3.3 同时规定：MVP 不实现具体知识积累引擎，生产注册项严格为 0，测试专用实现不打包到生产注册表。因此“让这条路径跑通”不可能靠新增一个生产引擎来完成——那会把一个被明确延后的产品决定偷偷改掉。本轮做的是把 §7.4 已经规定的成功路径实现成一条真正的写入命令，并把它验证到“接入第一个真实引擎时就能用”的程度，同时把“生产进程为什么不会产生写入”写进 docs/02，而不是假装它已经跑过。

### 19.2 成功路径的可观察结果

引擎返回候选条目，应用服务把它们写进知识库并触发索引。写成代码就是：`POST /api/knowledge/accumulate` 携带 `commandId`、`spaceId`、`clues`；命中回执时直接返回上次写入的条目，否则调用引擎，再在 Store 边界逐条验证候选（标题、关键词、正文、来源与 `create_knowledge_entry` 同规则），然后在**一个事务**里插入全部条目、为每条建立索引任务、写入命令回执。响应就是写入的条目数组，它们随即出现在 `GET /api/knowledge/entries` 里，索引任务处于 `pending`（未配置 Embedding 时为 `not_configured`）。

### 19.3 三处必须原子或幂等的地方

- **一条命令一个事务**。原先的实现在路由里 `for` 循环逐条 `create_knowledge_entry`：第二条候选无效时，第一条已经落库，违反“状态与对应变更在同一业务事务中提交”的不变量 9。现在整批候选先验证、后写入，任何一条无效或引擎失败都不留下半批条目。
- **重放不再次调用引擎**。原先的请求没有 `command_id`（注释里的理由正是“MVP 不会产生写入，幂等回执无处可存”），一旦真有引擎，前端重试就会再写一遍。现在它是普通写入命令：应用服务在调用引擎**之前**先查回执，命中且内容一致直接返回原条目；同 ID 不同内容返回 409 `IDEMPOTENCY_CONFLICT`；失败的命令不留下回执，所以修好后用同一个 `command_id` 重试是重新执行而不是返回上次的失败。
- **能力查询与入口读同一份注册表**。引擎列表原本是处理函数直接读的进程全局常量，`/api/capabilities` 另读一次同一个常量；现在它属于 `AppState`（生产启动时传入 `ACCUMULATION_ENGINES`），两处都从应用状态读，因此“能力为空”与“入口返回不可用”不可能互相矛盾，测试也可以用同一状态注入契约引擎。

### 19.4 实测

| 验收面 | 结果 |
| --- | --- |
| 成功写入（真实端口 HTTP + 真实 SQLite） | ✅ 测试内注册表 + 真实回环端口：POST 返回 200 与 2 条候选；`GET /api/knowledge/entries` 有 2 条；索引覆盖度 `pending=2`；引擎调用次数 = 1 |
| 重放 | ✅ 同 `commandId` 同线索再 POST：返回体与首次逐字相同、条目仍为 2 条、**引擎调用次数仍为 1** |
| 幂等冲突 | ✅ 同 `commandId` 换一组线索：409 `IDEMPOTENCY_CONFLICT` |
| 引擎失败 | ✅ `EngineError::Failed` → 502 `ENGINE_FAILED`，零条目、零索引任务、无回执（同一命令可重试） |
| 生产 0 引擎（真实进程 `./target/debug/nekowriter`） | ✅ `GET /api/capabilities` 返回空数组与原因；`POST /api/knowledge/accumulate`（camelCase `commandId`/`spaceId`/`clues`）409 `ENGINE_NOT_AVAILABLE`；缺 `commandId` 422、空线索 422 `VALIDATION_ERROR`、不存在的 Space 404 |
| 库侧事实（直接读 SQLite） | ✅ `knowledge_entries=0`、`knowledge_index_tasks=0`、`quota_charges=0`、该命令无 `command_receipts` 行 |
| 自动化 | ✅ `cargo test --workspace` 179 通过（agent 29 / app 44 / core 37 / store 69），`cargo fmt --all -- --check`、`tsc -b && vite build`、改动文件 `prettier --check` 通过；`pnpm lint` 与基线一致（8 个错误、3 条警告，全部为既有项） |

### 19.5 本次未做与边界

- **真实进程无法验证成功写入**：生产注册项严格为 0 是规范决定，不是遗漏。因此成功路径的端到端证据来自测试内注册表 + 真实端口的 HTTP + 真实 SQLite，真实进程探针只能证明 0 引擎时的行为。已写进 docs/02 §16.1。
- 积累候选没有“先审后写”的中间状态：§7.4 只要求应用服务验证后插入并触发索引，因此要么整批写入、要么整批不写。
- 前端只把入口改成普通写入命令（重试沿用同一个 `commandId`）；界面文案不变，因为生产环境下这条路仍然只返回“暂无可用引擎”。

## 20. 2026-09-12 增量更新：Embedding 用量落库（M4 最后一项）

### 20.1 写在哪张表、走哪条路径

docs/02 §12.1 定了三件事：用量按项目/时间/角色（写作/审核/Embedding）汇总；Embedding 的输入用量独立列示；系统连接测试单列为系统操作。知识 Space 是全局的，索引与检索的向量请求不属于任何项目，因此它们和模型连接测试走同一张表、同一条写入路径：`system_model_operations`（schema 25）与 `Store::record_system_model_operation`。没有新表、没有新的写入旁路，只是多了三个操作类型：`embedding_index`、`embedding_query`、`embedding_connection_test`（后者包含保存设置前的那次验证调用），并用一处 `is_embedding_operation` 同时供写入与读取分类。

### 20.2 适配器只说供应商说了什么

`nekowriter-agent::embedding` 原先直接丢掉响应里的 `usage`，`embed()` 只回向量。现在它回 `EmbeddingBatch { vectors, usage }`：`usage` 是一份与对话请求同类型的 `PersistedModelUsage`（`usage_source = embeddings_http`，`prompt_tokens` 映射为 `input_tokens`，嵌入没有输出侧所以 `output_tokens` 是结构性的 0）。适配器不算任何东西，调用方也不估算。当时按固定结构解 `usage`，读不出计数就记 `unknown / invalid_report`、字段缺失记 `unknown / provider_silent`——这份结构解后来在 §21 改成了宽容解析，因为“形状不合预期”不应该让一次本来可用的 Embedding 服务不可用。

### 20.3 概览：单独列示，且不重复计数

`DashboardTokens` 新增 `embedding`（请求数、输入用量、未知计数与原因、按操作类型拆分），并从 `systemOperations` 里排除 Embedding 行——否则同一批 token 会既算系统操作又算 Embedding。对话总量完全不变：`tokens.inputTokens` / `requestCount` 仍然只来自 `model_requests`。工作台概览在“系统操作”上方新增「Embedding 用量」区块，写明它只统计输入、不混进对话 Token，并列出连接测试/知识索引/语义检索各自的输入与请求数。

### 20.4 实测（真实进程 + 真实 embeddings 桩服务）

桩服务是一个真的 HTTP 服务，真的返回向量与 `usage`，应用真的去调它。

| 验收面 | 结果 |
| --- | --- |
| 保存设置前的验证调用 | ✅ 1 次向量请求 → 1 行 `embedding_connection_test`、`usage_source=embeddings_http`、`input_tokens` 等于桩服务报告的 4 |
| 长条目索引 | ✅ 41,090 字的条目被切成 67 块、拆成**两批**请求（64 + 3 条输入），各自一行：44,673 与 1,896 token，没有合并成一个估算值 |
| 语义检索 | ✅ 1 次查询向量请求 → 1 行 `embedding_query`（5 token） |
| 不重复、不丢失 | ✅ 桩服务收到 4 次向量请求时库里有 4 行；行数与请求数逐次相等，每行输入用量与供应商报告逐字相等 |
| 供应商不报告用量 | ✅ 桩服务改成不返回 `usage` 后再索引一条：新增一行 `usage_source=none` / `usage_status=unknown` / `reason=provider_silent`，输入用量为 `null` 而不是 0 |
| 向量请求真的失败 | ✅ 关掉桩服务后再索引一条：索引任务变 `failed`，同时新增一行 `status=unknown` / `usage_status=unknown` / `reason=request_failed` / 输入为 `null`，`error_json` 带 `{"kind":"embedding_error","message":"…error sending request for url (…)…"}` |
| 概览口径 | ✅ `tokens.embedding.requestCount=6`、`inputTokens=46,578`（已报告行之和）、`unknownUsageRequests=2`（与库里的未知行数一致）、`byKind` 之和等于总请求数；`tokens.requestCount=0`、`tokens.inputTokens=0`；`systemOperations.requestCount=0`（Embedding 没有重复计入） |
| 项目过滤 | ✅ 带 `projectId` 的概览返回同一份 Embedding 数字，且它不进入该项目的对话总量 |
| 界面 | ✅ 真实浏览器：概览面板出现「Embedding 用量：输入 46,578 · 请求 5 · 用量未知 1」，未知原因写作“供应商未在响应中报告用量：1 次”，下面按连接测试/知识索引/语义检索分行列出输入与请求数（界面截取早于失败路径那一步，因此当时是 5 行） |
| 自动化 | ✅ `cargo test --workspace` 182 通过（agent 30 / app 44 / core 38 / store 70），`cargo fmt --all -- --check`、`tsc -b && vite build`、改动文件 `prettier --check` 通过；`pnpm lint` 与基线一致（8 个错误、3 条警告，全部为既有项） |

### 20.5 本次未做与边界

- Embedding 用量没有项目归属：它在概览里是一个独立字段，项目过滤下显示的是同一份全局数字（已写进 docs/02 §16.1）。
- 向量请求失败时也会写一行：用量是 `usage_status=unknown` / `reason=request_failed`（没有报告就是未知，不补 0），操作的 `status` 在连接与超时类失败上是 `unknown`、其余是 `failed`，`error_json` 带 `embedding_error` 与可读原因。连接失败已由探针验证；“其余失败标为 `failed`”只在代码路径上成立。
- 没有费用估算与价格表（§12.1 明确不做）。

## 21. 2026-09-12 增量更新：Embedding 用量形状宽容解析与保存前的准入检查

审计在真实进程里实测出两个具体缺陷，本次只修这两项（角色/日序列口径与两份记账函数的合并是审计点名的下一项，不在本轮）。

### 21.1 统计字段不再能弄丢向量

`EmbeddingResponse.usage` 由固定结构改为 `Option<serde_json::Value>`，`embed_usage` 从形状里宽容地读计数：对象里的数字与字符串形式的数字（`{"prompt_tokens":"7"}`）都能认出来，读完整个对象拿不到计数就记 `unknown / invalid_report`，字段缺失或为 `null` 记 `unknown / provider_silent`。适配器仍然只校验向量本身（条数、维度、有限值）；用量是统计字段，它的形状永远不能决定向量能不能拿到。改前 `{"prompt_tokens":"7"}` 与 `usage: 7` 都会 502，而这类服务在改动前是可用的；`raw` 现在保存供应商原文，不再是重新序列化的结构。

### 21.2 保存配置前的真实验证调用排在准入检查之后

应用层的 `save_embedding_settings` 原先在修订检查之前就发了真实验证请求。现在顺序是：命令重放检查 → 字段与修订准入检查 → 真实验证 → 写凭据与配置。重放检查与写入共用同一个新抽出的 `embedding_settings_payload`（校验 + 命令指纹），因此“保存前先检查”不会变成第二份哈希逻辑；命中已记录的结果时直接返回上次的配置，既不重复写凭据也不重复调用供应商。

### 21.3 实测（真实进程 + 真实 embeddings 桩服务）

桩服务是一个真的 HTTP 服务，每个形状都返回真的四维有限向量，应用真的去调它。62 条断言全部通过。

| 验收面 | 结果 |
| --- | --- |
| 字符串数字 `{"prompt_tokens":"7"}` | ✅ 保存成功（向量维度与有限值都通过），用量 `reported` / `input_tokens=7` |
| 裸数字 `usage: 7` | ✅ 向量拿到，用量 `unknown` / `invalid_report`，输入为 `null`（不猜它是哪个计数） |
| `usage: null` | ✅ 向量拿到，用量 `unknown` / `provider_silent`，输入为 `null` |
| `usage: []` | ✅ 向量拿到，用量 `unknown` / `invalid_report` |
| `usage: {}` | ✅ 向量拿到，用量 `unknown` / `invalid_report` |
| 只有 `total_tokens` | ✅ 用量 `reported` / `input_tokens=9`（嵌入没有输出侧，`total_tokens` 就是输入） |
| 缺 `usage` 字段 | ✅ 向量拿到，用量 `unknown` / `provider_silent` |
| 每次形状都恰好一次向量请求 | ✅ 七种形状共 7 次请求、7 行用量，来源标记在 `reported` 时是 `embeddings_http`、否则是 `none` |
| 过期修订提交 5 次 | ✅ 5 次都报 409 `REVISION_CONFLICT`，供应商**零**调用，用量**零**新增 |
| 无效地址提交 | ✅ 422 `VALIDATION_ERROR`，供应商零调用（字段校验同样在真实请求之前） |
| 合法提交 | ✅ 200，供应商 1 次调用，新增一行 `embedding_connection_test` / `embeddings_http` / `reported` / `input_tokens=7`，修订前进一位 |
| 同一命令重放（修订已过期） | ✅ 200 返回与上次逐字相同的配置，不再调用供应商，不新增用量行 |
| 同一命令换配置 | ✅ 409 `IDEMPOTENCY_CONFLICT`，零调用、零新增行 |
| 自动化 | ✅ `cargo test --workspace` 184 通过（agent 31 / app 44 / core 38 / store 71），`cargo fmt --all -- --check` 通过 |
| §20 用量回路回归 | ✅ 上一轮那次真实进程探针（标准形状的索引与检索、供应商不报告用量、关掉服务、概览汇总）原样重跑全部通过，没有回归 |

### 21.4 本次未做与边界

- 七种形状是通过“保存配置”这条路径打的：向量解析本身的边界（条数、维度、非有限值）仍由既有适配器单测覆盖。
- `usage: 7` 记 `invalid_report` 而不是把 7 当作 `total_tokens`：报告在但读不出是哪个计数，不猜。
- 没有合并两份记账函数、没有把 Embedding 并入 `byRole` 与按日序列：审计点名的下一项，已在 §22 完成。
- 用 SQL 把两行的 `created_at` 挪到更早的本地日来造多天数据（行本身由真实接口写出），因此“多天区间”的证据包含一次人为的时间位移，已在 §22.3 标注。

## 22. 2026-09-12 增量更新：§12.1 的三个镜头与一份记账实现

### 22.1 Embedding 进入角色与按日两个镜头

`tokens.byRole` 新增 `embedding` 角色（与写作、审核并列），`tokens.embedding` 新增 `daily`（与 `tokens.daily` 同一套本地日口径、平行的一日一格），两者都是同一批向量用量的另一种镜头：`embedding` 角色的输入与请求数逐字等于 `tokens.embedding`，对话总量与对话日序列仍只来自 `model_requests`，因此没有新的计数，也没有重复计数。角色名字与归属值放在 `nekowriter-core::dashboard`（`DASHBOARD_ROLE_EMBEDDING`、`DASHBOARD_SCOPE_GLOBAL`），写入与读取共用一处分类器（`is_embedding_operation`）照旧。

归属不是靠前端拼的：规范只说 Embedding 要按“项目、时间、角色”汇总，而定下知识 Space 是全局的。因此数字仍是全局的，但 `tokens.embedding.scope` 把这件事写进响应自身，界面直接把它写成“归属全局知识库，不受当前项目筛选影响”，不需要调用方自己推断。

### 22.2 一份记账实现

`app/src/main.rs` 的 `persist_connection_test` 与 `app/src/knowledge.rs` 的 `record_embedding_operation` 原先逐行同构（match → (status, usage, error) → 写系统操作）。现在策略只写一次：`nekowriter-app::operations` 拥有 `SystemOperationResult`（`Succeeded` / `Failed { reached_provider_unknown, kind, message }`）与 `record_system_operation`（状态判定、类型化未知原因、错误的 JSON 形状、写入与事件广播）。两个调用点只剩自己的类型信息：模型连接测试提供 `completion.persisted_usage()` 与 `provider_request_id`，Embedding 提供 `batch.usage`。哪种失败算“可能已到达供应商”也留在各自的适配器里，因为只有它俩知道自己的错误类型。

### 22.3 实测（真实进程 + 真实接口）

真实进程 + 真的 embeddings 桩服务；项目、知识 Space、条目、检索、Embedding 配置全部经真实接口产生，写作与审核各一条请求用 SQL 预置（请求行属于真实库结构，接口必须把它折进角色与日序列），另用 SQL 把两行 `created_at` 挪到更早的本地日以造出多天区间。33 条断言全部通过。

| 验收面 | 结果 |
| --- | --- |
| 角色镜头 | ✅ `byRole` 为 `['embedding','review','writing']`；`embedding` 角色（输入 9,278 · 请求 3 · 未知 0）与 `tokens.embedding` 逐字相等 |
| 按日序列 | ✅ 7 天各有自己的格子；昨天 1 次 / 9,269 token（索引）、前天 1 次 / 4（连接测试）、今天 1 次 / 5（检索），与库内行的本地日逐日对上，日序列之和等于区间总量 |
| 归属字段 | ✅ `tokens.embedding.scope == "global"`；真实项目过滤下 Embedding 数字与全局响应完全相等，而对话数字收窄到该项目（输入 16 / 请求 2），第二个项目看到对话 0 但同一份 Embedding |
| 不重复计数 | ✅ `tokens.inputTokens = 16`（只有对话请求）、对话日序列合计 16、`systemOperations.requestCount = 0`；向量用量不出现在任何对话数字里 |
| 一次向量请求一行 | ✅ 桩服务收到 3 次请求（连接测试、索引、检索），库里 3 行 Embedding 操作，区间请求数与库内行数相等 |
| 真实界面 | ✅ 浏览器里三个角色并列显示（Embedding（独立列示）/ 审核 / 写作），Token 区块与 Embedding 区块各有一组按日柱（对话 21 token vs 向量 5 / 9,269 / 4），Embedding 区块写明“归属全局知识库，不受当前项目筛选影响” |
| 自动化 | ✅ `cargo test --workspace` 185 通过（agent 31 / app 44 / core 38 / store 72），`cargo fmt --all -- --check`、`tsc -b`、`vite build`、改动文件 `prettier --check` 通过；`pnpm lint` 与基线一致（8 个错误、3 条警告，全部为既有项） |
| 既有探针回归 | ✅ §20 的用量回路探针与 §21 的形状/准入探针原样重跑全部通过 |

### 22.4 本次未做与边界

- 归属仍是全局的：知识 Space 没有项目归属，所以没有给 `system_model_operations` 加项目列；规范只要求“研究未来接入后必须归属其父项目”，那属于尚未存在的功能。要真的按项目归属时，索引（全局 Space）仍然无项目可归，只能部分归属——那是更差的口径。
- 多天区间用了一次时间位移（见上），接口侧的分桶与库内行逐日对账仍然成立。
- 角色镜头里 `sum(by_role.inputTokens)` 不再等于 `tokens.inputTokens`（Embedding 不在对话总量里）；这是 §12.1 “Embedding 输入独立列示”的直接后果，已写进 docs/02。
- `crates/nekowriter-store/src/system_operations.rs`（表、写入与聚合）与 `crates/nekowriter-app/src/operations.rs`（一次调用的落库策略）名字相近但不是重复：前者拥有存储与读取，后者拥有 HTTP 层的翻译与落库入口。
- `credential_ref` 仍不进命令指纹（保持原语义）；重放直接返回，因此不会因为重放多出一个凭据槽。

## 23. 2026-09-12 增量更新：全平台发行目录与 macOS 的 release 发行包

### 23.1 为什么是“全平台同一形状 + macOS 先验收”，而不是本机交叉编出 PE

M5 的最后一项原本写作“Windows x64 发行包”。先查了本机实际情况：`rustup target list --installed` 只有 `aarch64-apple-darwin`，没有 `mingw-w64`、没有 llvm / `lld-link`、没有 `zig` 或 `cargo-xwin`；`tools/` 里只有 `bootstrap-protoc.ps1`，没有 `tools/protoc/bin/protoc.exe`。因此本机没有任何能产 Windows PE 的链接器 / C 编译器。逐一核实过依赖后可以确定这不是“注定做不成”：`ring` 0.17.14 作为 crates.io 包从 `pregenerated/` 构建、自带 COFF 目标文件（`aesni-x86_64-nasm.o` 就是 Intel COFF），不需要 nasm/perl；需要目标 C 编译器的只有 `libsqlite3-sys` 与 `zstd-sys` 这类普通 C。也就是说只要装一个目标工具链（`brew install mingw-w64` 走 `x86_64-pc-windows-gnu`，或 `brew install llvm` + `cargo install cargo-xwin` 走 msvc）就能产出真 PE。这条环境改动交给用户决定后，方向定为“先把打包做成全平台，先在 macOS 上把 `--release` 验收掉”，Windows 只交付配方与 CI 步骤（§11.5），不假装有产物。

### 23.2 改成全平台后 `cargo xtask package` 做了什么

- **目标由参数决定**：缺省用 `rustc -vV` 读出的本机三元组，`--target <三元组>` 指定其它平台；构建与产物目录都按三元组分开（`target/<三元组>/<profile>` → `target/package/<三元组>/nekowriter/`），多个平台的包能共存。
- **前端不再可能用旧产物**：打包第一步就是 `pnpm --dir webui build`（原本已是如此，这次把它写进文档口径）。
- **头部检查**：读可执行文件头部判定容器格式与机器类型（PE32+/Mach-O/ELF），并要求它与目标三元组一致；不符直接失败。这是“别拿改名或拷错的二进制充数”的硬约束，也是本地上一次只能用人工检查的地方。
- **发行清单 `release-manifest.txt`**：目标三元组、构建配置、容器格式、生成时间，以及逐个文件的字节数与 SHA-256（含 `webui/dist` 的完整资源清单）。清单描述的是拷贝到发行目录之后的那一份（大小与摘要都从目标文件算），所以它就是交付物的账。
- **README 按平台写**：写明本次发行的三元组，并把凭据存储写成“macOS 钥匙串 / Windows 凭据管理器 / Linux Secret Service”，而不是硬写 Windows。

打包过程里发现并修掉一个真实缺陷：资源清单在递归目录时丢了目录前缀，子目录里的 `assets/index-*.js` 被写成 `webui/dist/index-*.js`——清单指向一个不存在的路径。这正是“拿清单对账”能拓出来的错，现在由 §23.3 的脚本逐行重算覆盖。

### 23.3 实测（真实进程 + 真实 release 产物）

| 验收面 | 结果 |
| --- | --- |
| 构建与产物 | ✅ `cargo xtask package --release --protoc /opt/homebrew/bin/protoc` 跑完（前端 + 全量 release 构建，含 lancedb/arrow 冷启动）；发行目录 `target/package/aarch64-apple-darwin/nekowriter/`，里面是 `nekowriter`（262,571,600 字节）+ `webui/dist`（11 个资源）+ `README.txt` + `release-manifest.txt` |
| 可执行文件格式 | ✅ 头部 `0xfeedfacf` + cputype `0x0100000c`（`file` 也报 `Mach-O 64-bit executable arm64`）；`__TEXT` 191MB、代码段 148MB、无 DWARF，说明体积是真实代码而不是调试信息 |
| 清单对账（独立脚本） | ✅ 28 条断言全部通过：清单里的容器格式与头部一致、每个文件的字节数与 SHA-256 重算一致、清单覆盖 `dist` 的每个文件、`index.html` 引用的 `assets/…` 都在包内 |
| 脱离源码树启动 | ✅ 把发行目录拷到仓库之外的 `/tmp/nw-relocate-release`，以那里为工作目录启动：界面首页 200（498 字节）、引用的 JS 资源 200（473,367 字节），`lsof` 确认进程打开的**所有**文件都不在仓库路径下 |
| 数据与日志位置 | ✅ 数据目录里恰好出现 `nekowriter.sqlite3`（+ `-wal` / `-shm`）、`lancedb/`、`logs/nekowriter.<日期>.log` 与进程锁文件；仓库之外的目录只有 `nekowriter/`、`data/` 与应用日志 |
| 真实写作回路 | ✅ 在发行包上用真实 API 建项目、存目标、提交四项准备产物与细纲并逐项审核、两次追加正文共 72 字、整章终稿通过，第一章为 `approved` |
| 退出（接口） | ✅ 在途请求 + 开着 SSE 时 `POST /api/system/shutdown`：3ms 返回 `shutting_down`，0.211 秒退出码 0、SSE 被关闭、`-wal` 消失、日志记下退出原因与 `writing_runs=1` |
| 重启 | ✅ 数据与正文原样，在途 Run 变 `interrupted`，事件流有 `run.changed` / `workflow.changed` 两条 `reason = process_shutdown` |
| 退出（信号） | ✅ 带 SSE 连接发 SIGTERM：0.105 秒退出码 0、SSE 被关闭；不带 `--no-browser` 时日志出现「已在默认浏览器打开工作台」 |
| 自动化 | ✅ `cargo test --workspace` 187 通过（agent 31 / app 44 / core 38 / store 72 / xtask 2），`cargo fmt --all -- --check` 与 `cargo clippy -p xtask` 无新增提示 |

### 23.4 本次未做与边界

- **没有 PE 产物**：本机没有 Windows 目标的工具链，所以只有一个平台的真产物；`docs/02 §11.5` 给出三平台配方（含 Windows runner 的 CI 步骤），但没有跑过。
- 只验证了本机三元组（`aarch64-apple-darwin`）。x86_64 macOS / Linux 同样只需 `rustup target add`，形状不变，但未实跑。
- 未做符号裁剪：262.6MB，实测 `strip -x` 为 205MB、完整 `strip` 为 202MB，但没有写进打包流程。
- `--target` 与头部检查都不需要在目标机器上运行：没验证过的是“这个包在 Windows 上真的能双击运行”。那是上一项（需 CI 或 Windows 主机）才能给出的证据。

## 24. 2026-09-12 增量更新：概览字数统计的列投影与发行打包的两道硬检查

上一轮审计在真实进程里量出两个具体缺陷：概览的字数统计把修订正文一并读进内存，以及打包器只校验容器家族、不校验架构。本轮只修这两处，不加功能。

### 24.1 概览的字数统计不再读正文

`nekowriter-store::dashboard::dashboard_words` 原来对 `document_revisions` 用 `Entity::find().all()`，把每一条修订的全部列（含 `content` 与 `structured_payload`）读进内存，再在 Rust 里过滤出可见文档。统计真正用到的只有六个列。

- 查询改成 `into_partial_model::<RevisionStat>()`（`RevisionStat` 是只含这六列的 partial model），正文与结构化载荷不再进入结果集。
- 可见性交给 SQL：`document_revisions.document_id IN (可见正文文档)`，主键来自同一个可见性映射（`current_total` 也用它），所以「哪些文档算可见」仍然只有一处定义；已删除文档与非正文文档的修订不再读进来再丢掉。
- 语义没有变：同一个文档仍按 `revision_no` 排序、仍用相邻修订的 `word_count` 之差算净增量、`copied` 仍计 0。

### 24.2 打包器校验架构，并让注定失败的构建早停

- `describe_binary` 现在返回结构化的 `BinaryFormat { container, arch, description }`（容器与架构分别判定），目标策略移到 `BinaryFormat::ensure_matches(&target)`：容器与架构都必须与三元组一致。审计点名的情形（`aarch64-unknown-linux-gnu` 配 x86-64 ELF）现在会被拒绝；不认识的架构在参数校验阶段就报错。发行清单里的格式行改用三元组的写法（`Mach-O 64 位（aarch64）`）。
- 新增 `preflight_target`：`--target` 不是本机三元组时，先查编译器 sysroot 下有没有该目标的 `lib/rustlib/<三元组>/lib`，缺了就在**任何构建之前**以 `rustup target add <三元组>` 结束。
- 构建顺序改为「先编译后端、再构建前端」：编译失败不再白写一次 `webui/dist`。

### 24.3 实测（真实进程 + 真实 release 产物）

| 验收面 | 结果 |
| --- | --- |
| 数字完全没变（逐字节） | ✅ 同一批数据在新旧两个 release 二进制上取 7 天、项目过滤与 90 天三份概览，除 `generatedAtMs` 外**逐字节相同**；正文体积也不改变任何一个数字 |
| 内存不再随正文体积增长 | ✅ 改动前：正文 10 字符时 RSS 40MB、换成 4,000 字符后 192MB（+152MB，不回落）；改动后：两种情况都是 39MB |
| 耗时 | ✅ 同一对照：改动前 105ms（小正文时 25ms），改动后 46ms（小正文时 29ms）；剩下的差别来自 SQLite 把正文与统计列放在同一页 |
| 与库内行对账 | ✅ 当前字数 600,000 = 可见正文文档当前修订 `word_count` 之和；按日修订数之和 2,100 = 区间内可见正文修订数（库内另有 10 条隐藏文档修订，未被计入）；按日净增量之和 = 区间净增量；区间请求数 3 与输入用量 720 与 `model_requests.usage_json` 一致；连接测试单列 1 条、Embedding 单列 1 条（未知原因 `provider_silent`）且进入角色镜头 |
| 空集边界 | ✅ 空库概览可读、字数为 0（`IN ()` 不会让查询失败） |
| 打包早停（真实命令） | ✅ 未 `rustup target add` 时 `cargo xtask package --debug --target x86_64-pc-windows-gnu` 立即以「目标 … 的标准库未安装：请先运行 `rustup target add …`」结束；`webui/dist/index.html` 的 mtime 未变（没有白构建前端），`target/package/` 下没有留下该目标的空壳目录 |
| 打包正例（真实命令） | ✅ 同一条命令去掉 `--target` 后用 `--release` 跑完：`Mach-O 64 位（aarch64）`、262,506,224 字节、13 个文件的清单 |
| 架构校验（单元测试） | ✅ 4 个 xtask 用例：错容器（Mach-O 当 Windows）、错架构（Mach-O arm64 当 x86_64-apple-darwin）、PE/USB 与 ELF 的正反例、`aarch64-unknown-linux-gnu` 配 x86-64 ELF 被拒、未知架构报错、缺标准库的目标预检失败 |
| 回归 | ✅ §22 的毫秒边界与规模探针在新二进制上重跑全部通过（区间边界、日序列之和、角色镜头与独立列示一致、`days=1`/`days=90` 长度） |
| 自动化 | ✅ `cargo test --workspace` 189 通过（agent 31 / app 44 / core 38 / store 72 / xtask 4），`cargo fmt --all -- --check` 通过；release 打包过程本身跑了前端 `tsc -b && vite build` |

### 24.4 本次未做与边界

- 没有走「加一份不含正文的字数索引」这条更彻底的路：行扫描本身还在，所以表很大时仍有页读开销（SQLite 把正文与统计列存在同一页里）。省下的是内存与大部分耗时，不是全部。
- 概览的 Token 与系统操作仍然是区间内读行到内存再汇总；在 20 万行铺在 80 天的极端数据上是百毫秒到秒级，真实库量级下是个位数毫秒，因此有意不动。
- 架构校验只识别 `x86_64` 与 `aarch64`；32 位与其它架构会被明确拒绝，而不是被默默放过。
- 审计点名但本轮按指示没有动的项：两份同名 `persisted_usage`（语义确实不同，合并会掏空「未知带原因」的契约）、`DashboardTokens` 的字段拆分、巨型模块（`store/src/preparation.rs` 13,274 行等）的拆分——它们被判定为当前负收益。

## 25. 2026-09-12 增量更新：真实模型凭据的探查与 aarch64 发行包的重新验收

计划里剩下的两件都自称受环境限制（真实模型验收、非 macOS 平台的发行产物）。本轮先把凭据查清，再按用户决定只做 aarch64-darwin 的发行包并把它重新验收一遍。

### 25.1 凭据探查：本机没有真实模型凭据

只看存在性、不打印任何密钥值：

- 环境变量里没有任何模型服务的 key（名字里带 `API` / `KEY` / `TOKEN` / 供应商名的只有编辑器自身的 `FREEBUFF_CDP_BRIDGE_TOKEN` 与 `FREEBUFF_MCP_CONSENT_TOKEN`）。
- OS 凭据库里 `NekoWriter` 服务下**只有一条**记录（account `writing-model-76f738d6-…`），它是本轮探针自己写入、指向回环桩地址的凭据；没有任何指向真实供应商的凭据。
- 工作台的应用数据目录在本机不存在（`~/Library/Application Support/NekoWriter` 与 `…/nekowriter` 均缺失），所以也没有一份真正的、由用户建过的模型设置。

结论：M1 与 M3 的真实模型烟测**明确未完成**，且本轮没有用桩服务冒充（桩只用于退出流程、竞态与用量记账，相关记录里都写明了它是桩）。这一条已写进细则 §15。

### 25.2 发行包：按用户决定只做 aarch64-darwin

先前尝试交叉编译到 `x86_64-apple-darwin` 需要该目标的标准库与它自己的链接器，本机都没有；按用户决定不再用链接器替身绕路，改为把本机三元组的发行包按**当前代码**重建并重新验收，同时把打包器的两道硬检查在真实产物上真的走一遍。

- `cargo xtask package --release --protoc /opt/homebrew/bin/protoc` 完整跑完：发行目录 `target/package/aarch64-apple-darwin/nekowriter/`，可执行文件 262,506,224 字节（Mach-O 64 位 arm64、头部 `0x0100000C`），清单 13 个文件。
- 反例用的是**真实的** x86_64 Mach-O 可执行文件（`clang -arch x86_64` 编译的 8,440 字节小程序），把它放到打包器要读的位置：打包以「编译产物是 Mach-O 64 位（x86_64），与目标平台 aarch64-apple-darwin 不符；请确认 --target 与工具链」结束，退出码 1。
- 反例的附带证据同样重要：这项校验发生在清理旧发行目录**之前**，所以上一次已验证的发行目录逐字节未动（清单生成时间 1789221098243 与二进制 SHA-256 `a770576a…` 不变），失败后 `target/package/` 下也没有留下空壳；未安装标准库的目标（`x86_64-apple-darwin`）则在**任何构建之前**以 `rustup target add <三元组>` 失败，`webui/dist/index.html` 的 mtime 未变。

### 25.3 实测（真实进程 + 真实 release 产物）

| 验收面 | 结果 |
| --- | --- |
| 清单独立对账 | ✅ 另一套脚本（不复用打包器逻辑）重算 13 行的字节数与 SHA-256 全部相符、磁盘上没有未列出的文件、`file` 与头部一致、`index.html` 引用的 JS/CSS 都在包内，28 条断言全过 |
| 错架构被拒（真实产物） | ✅ 真的 x86_64 Mach-O 被拒：退出码 1、错误文案点明「实际是什么 / 目标是什么 / 该查 --target 与工具链」 |
| 失败不留假包 | ✅ 反例跑完后旧发行目录的清单生成时间、二进制 SHA-256 与 mtime 全部不变，`target/package/` 下只有一个已验收的目录 |
| 早停不白写前端 | ✅ 缺标准库的目标在构建前失败，`webui/dist/index.html` 的 mtime 未变 |
| 脱离仓库的完整回路 | ✅ 把重建后的发行目录整体拷到 `/tmp/nw-relocate-release`，从中启动：`lsof` 显示运行中的进程没有任何打开的文件落在仓库路径下；首页 200（498 字节）、引用的 473,367 字节 JS 200；数据目录里恰好出现 `nekowriter.sqlite3`(+`-wal`/`-shm`)、`lancedb/`、`logs/` 与进程锁 |
| 真实 API 写出正文 | ✅ 建项目 → 存目标 → 四项准备产物与细纲逐项审核 → 两次追加正文共 72 字 → 整章终稿通过（第一章 `approved`，第二章 `planned`） |
| 退出与重启 | ✅ 在途模型请求 + 一条打开的 SSE 下，接口退出 2ms 返回、进程 0.21 秒退出码 0、SSE 被服务端关闭、`-wal` 消失；重启后项目、正文与第一章定稿原样在，在途 Run 为 `interrupted`，事件带 `reason=process_shutdown`；SIGTERM 路径 0.105 秒退出码 0 |
| 回归 | ✅ 发行包与错误架构反例跑完后，重建的发行目录仍是同一份（逐字节），既有探针没有回归 |

### 25.4 本次未做与边界

- **其它平台仍无产物**：本机只有 `aarch64-apple-darwin` 一个已安装目标，没有 Windows 目标的链接器 / C 编译器，也没有 x86_64 的 macOS 目标标准库；Windows 与 Linux 的配方、`--target` 参数与 CI 步骤见细则 §11.5，形状与本次相同但未经真实验收，这是环境边界而非代码缺口。
- 反例是把真实的 x86_64 Mach-O 放到打包器要读的位置，而不是真的交叉编译出一个 x86_64 版本——本机没有那条工具链；因此它证明的是「打包环节会拒」（包括旧包不被破坏、不留空壳），不是「交叉构建能成」。
- 真实模型验收仍待凭据可用时记录：本轮只确认了「本机没有凭据」这个事实，没有用桩服务替代。
- 发行包仍未做符号裁剪（262.5MB，`strip -x` 可降到 205MB），与上一轮相同。

## 26. 2026-09-12 增量更新：发行包敌意试玩（阶段提交合同、回收站事件与跨平台文案）

### 26.1 本轮做法与边界

- 用**真实发行包** `target/package/aarch64-apple-darwin/nekowriter` 启动（不是开发树二进制），数据目录为全新的 `/tmp/nw-play2/data`，在真实浏览器（预览桥）里以第一次使用者的身份走界面流程。
- 模型调用没有真实凭据，统一指向本地打桩服务 `/tmp/nw-model-stub2.py`。桩按系统提示词判断当前阶段，返回该阶段合法的结构化提交或 `append_content` 工具调用；章节级提交需要的 `chapter_id` 由试玩脚本经 `/tmp/nw-play2/stub-state.json` 交给桩。
- **桩不是真实模型**：本轮的生成结果只用来驱动界面流程与暴露缺陷，不构成 M1/M3 的真实验收；凡涉及桩的记录都如实标注。

### 26.2 实际走过的步骤与结果

| 步骤 | 结果 |
| --- | --- |
| 建项目「试玩」+ 填入小说目标 | ✅ 201，工作流 `initializing` → 可编辑目标 |
| 启动准备 run | ✅ 桩提交四项产物，`preparing_generating` → `preparing_reviewing` |
| 界面逐项「通过」四项准备产物 | ✅ 每项 `approved`，全部通过后进入 `writing_chapter_ready` |
| 「开始细纲」 | ✅ 桩提交 `{chapter_id, content, goals}` → `writing_outline_reviewing` |
| 「通过并进入正文」 | ✅ 桩先调用 `append_content` 落库「雨落在站台上。」，再提交 `{chapter_id, summary}` → `writing_chapter_reviewing` |
| 「通过并定稿」 | ✅ 第 1 章 `approved`，正文 7 字 |
| 概览是否无需手动刷新 | ✅ 未点任何刷新，事件到达后「等待审核」由 1 变 0、「正文净字数」由 0 变 7 |
| 阅读视图 + 全文查找 | ✅ 阅读区按章渲染，查找「站台」得到「找到 1 处（最多显示 50 处）」 |
| 导出纯文本 | ✅ 界面点击后发出 `GET /api/versions/{id}/export?format=txt` 并 200 |
| 提示词模板 新建 → 删除 → 回收站 → 恢复 → 再删除 → 彻底删除 | ⚠️ 见 26.3 缺陷二：删除后回收站停在「回收站是空的」，手动「刷新」才出现；恢复与彻底删除本身正确（恢复后模板回到列表，彻底删除后提示「已彻底删除提示词模板「试玩模板」，无法再恢复」且列表清空） |
| 快捷键 `?` / `Esc` / `Ctrl+K` | ✅ `?` 打开快捷键面板、`Esc` 关闭、`Ctrl+K` 把焦点落到阅读区查找框 |
| 「退出服务」二次确认 | ✅ 弹出确认说明（停止接收新请求、取消在途调用、把 Run 收束为「已中断」），点「取消」后服务照常运行 |
| 停止服务 | ⏸️ 本轮没有真的退出，界面上只验证到确认对话框 |

### 26.3 修掉的三个缺陷

**缺陷一（最严重）：阶段角色从未把「提交结构」发给模型，章节级 `chapter_id` 更是模型无法推断的值。**

真实进程抓到的准备阶段请求体（`/tmp/nw-play2/requests/001-preparation.json`）只有 `messages`（127 字符 system + 111 字符 user）、`response_format: {"type":"json_object"}` 与工具声明——既没有字段名，也没有章节 ID。而 `parse_preparation_submission` / `parse_outline_submission` / `parse_chapter_submission` / `parse_review_submission` 要求精确字段（`artifacts`/`chapter_plan`，`chapter_id`/`content`/`goals`，`chapter_id`/`summary`，`passed`/`confidence`/`issues`），`chapter_id` 还必须与当前章节逐字相等。这违反细则 §4.1「阶段角色只包含…提交结构」，也意味着真实模型下细纲与正文提交**不可能**通过（与 M1/M3 真实验收一直未完成互为因果）。

修法：`nekowriter_agent::runner::submission_contract(PhaseSubmissionKind) -> String` 与解析器**同源**生成合同文本（章节级带具体 `chapter_id`），`scheduler::stage_system_instructions` 把它接到准备、细纲、正文、审核四个阶段的系统消息上。守卫：`runner` 新增单元测试逐字段检查合同文本；`outline_lane_*` 与 `body_lane_*` 两条 lane 测试直接断言发给假服务的 HTTP 请求体里带着 `chapter_id` 与本章 ID。

**缺陷二：删除用户提示词模板与知识 Space/条目不发 `recycle.changed`，已打开的回收站页面停在陈旧状态。**

细则 §10.3 明确「每次删除、恢复与彻底删除都在同一事务里写一条 `recycle.changed` 事件，工作台据此刷新回收站与目录，不靠手动刷新」。实测（界面 + `/api/recycle-bin` 对照）：删除模板后 API 已返回该模板，界面仍显示「回收站是空的」，点一次「刷新」才出现；恢复路径因为本来就发事件，反而即时刷新。修法：`delete_prompt_template`、`delete_knowledge_space`、`delete_knowledge_entry` 三处 SoftDelete 事务各补一条 `recycle.changed`（`knowledge.rs` 两处在上一轮已补，本轮补 `prompt_templates.rs`）。

**缺陷三：凭据库文案写死 Windows。**

`ModelSettingsPanel` 的密钥说明写「存入 Windows 凭据管理器」，`ApiError::Credential` 的错误消息写「请检查 Windows Credential Manager 后重试」。在 macOS 发行包上这是与本机无关的提示。改为平台中立措辞（错误消息同时点出 Windows 凭据管理器与 macOS 钥匙串）。

### 26.4 本轮未能踩到的

- **委托审核（delegated review）**：界面里的「委托审核」没有真正跑完一次子 Run（桩虽然能返回审核结论，本轮把预算放在主线与三处缺陷上）；只确认按钮存在且可用。
- **暂停/继续、意见投递、中断恢复**：本轮界面流程没有重跑这些路径；§24/§25 已用真实进程验证过退出与在途取消，但那是 API 探针而非本次界面流程。
- **模型用量口径**：概览显示的 4 次请求、输入 120 / 输出 48 全部来自打桩服务自报的数字，不代表真实供应商行为。
- **真实模型验收**：本机仍无凭据，M1/M3 的真实模型验收仍未完成——本轮反而证明了在补上提交合同之前它也不可能通过。

## 27. 2026-09-12 增量更新：把「提交结构」用最敌意的方式证死（本地运行时探查 + 只懂合同的朴素桩 + 反例）

### 27.1 先找本机可用的本地模型运行时：一个都没有

| 检查面 | 结果 |
| --- | --- |
| `ollama` / `llama-cli` / `llama-server` / `lms` / `mlx_lm.generate` / `text-generation-launcher` / `vllm` / `koboldcpp` | 全部不在 PATH |
| `/Applications/Ollama.app`、`/Applications/LM Studio.app`、`~/.ollama`、`~/.cache/lm-studio`、`~/.lmstudio` | 都不存在 |
| 权重文件（`~` 下 6 层内的 `*.gguf` / `*.safetensors` / `*.mlx*`） | 一个都没有；`~/.cache/huggingface` 不存在 |
| Python 推理栈 | 无 `transformers`、无 `torch`、无 `mlx` |
| 容器 | 无 docker |
| 常见本地服务端口（11434 / 1234 / 8080 / 5000 / 8000 / 11435 / 1337） | 无人监听 |

结论：**本机没有任何可用的本地模型运行时与权重**，用真实权重的 M1/M3 验收不可行，这条环境边界写进 docs/02 §16。

### 27.2 替代方案：一个只懂合同的朴素桩

`/tmp/nw-naive-contract-stub.py` 刻意不预设任何解析器细节，只有三条规则：

1. 从**系统消息**里用括号配对取出第一段完整 JSON，作为提交骨架（`json.loads` 验证）；
2. 骨架里含中文说明或 `…` 的字符串按「待填内容」替换成生成文本；纯 ASCII 的值（枚举、UUID）原样保留；
3. 工具调用只使用「合同正文提到的、且本次请求声明过的」工具，参数按**该工具自己声明的 JSON Schema** 生成。

它不认识 preparation / outline / body / review，也不认识 `artifacts`、`chapter_id`、`kind` 等任何字段名——输出里的每一个键名都来自合同正文。

### 27.3 正向：真实进程跑完整回路

| 阶段 | 合同 system 字符数 | 桩的推导（`/tmp/nw-contract-pos/derivation.log`） | 结果 |
| --- | --- | --- | --- |
| 准备 | 419 | 四个 `kind` 原样保留，四段 `content` 与 `chapter_plan[0]` 的 `title`/`goal` 由 `…` 填充 | `preparing_reviewing`，四项产物落库 |
| 细纲 | 288 | `content`/`goals` 填充；`chapter_id` 直接取自合同里的 UUID | `writing_outline_reviewing` |
| 正文（第 1 轮） | 336 | 合同提到 `append_content` → 发工具调用，参数按该工具的 Schema 造 | `append_content` 落库，进入第 2 轮 |
| 正文（第 2 轮，已带 tool 结果） | 336 | `summary` 由「一句话本章小结」填充 | `writing_chapter_reviewing` |
| 终稿审核通过 | — | — | 第 1 章 `approved`，正文 13 字 |

这就是「合同是否足以让一个只知道合同的模型完成提交」的直接证据；用的仍是真实进程与真实阶段流转。

### 27.4 反例：只改一个字段名，同一回路必须失败

把 `submission_contract` 里**章节级骨架**的 `chapter_id` 改写成 `chapterId`（其余一字不动），用同一份源码重建（`cargo build -p nekowriter-app`，debug），跑同一个驱动：

- 准备阶段：合同没被改，照常 `preparing_reviewing` → 四项通过；
- 细纲阶段：桩按合同推导出 `{"chapterId": "012e9695-…", "content": …, "goals": […]}` 并连续提交三次，解析器因未知字段全部拒绝；
- 结果：`连续模型协议错误达到上限，Run 已停止等待用户显式继续`，工作流停在 `writing_outline_generating` / `execution = stopped` / `waitReason = protocol_error_limit`，驱动等待细纲提交超时。

把字段名改回后重建，同一回路再次走到第 1 章 `approved`（`/tmp/nw-contract-pos2`）。因此：合同是模型唯一的提交依据，且这次修复确实落在需要它的位置。

### 27.5 本轮没能证明的

- **真实模型验收**：本机既没有本地运行时/权重，也没有远程供应商凭据，M1/M3 的真实模型验收仍未完成——§27.1 是环境边界，不是代码缺口。
- 两条实验都用**桩**：朴素桩证明的是「合同充分」「修复位置正确」，不能当作真实模型能照着提交的证明；真实模型可能还需要提示词层面的额外引导（例如更强的协议纠正话术）。
- 反向实验跑在 **debug 构建**上（真进程、真阶段流转）；生产形态的 release 发行包在这一轮之前已经单独验收过，两者源码相同。
- 委托审核子 Run、暂停/继续、意见投递仍未在界面里重跑；本轮没有新增功能，也没有重构。

### 27.6 过程清理

试玩与实验留下的后台进程已全部收掉：`pgrep` 无 `nekowriter` / `nw-model-stub` / `nw-naive-contract-stub` 进程，47841 / 47851 / 47861 / 47871 / 47881 与 47995–47999 端口无监听；实验数据留在 `/tmp/nw-play2`、`/tmp/nw-play3`、`/tmp/nw-contract-pos`、`/tmp/nw-contract-neg`、`/tmp/nw-contract-pos2` 供复查。

## 28. 2026-09-12 增量更新：四条支路的真实进程重跑与「无推进提交」的工程上限

§26 的试玩只覆盖了主回路，委托审核、暂停/继续、意见投递与在途中断恢复当时都没有在真实进程里跑到。本轮把四条支路逐条跑通，模型调用一律是**本地打桩**（`/tmp/nw-branch-stub.py`：提交内容只从系统消息里的合同骨架推导，工具调用只挑「合同提到 + 本次声明」的工具；桩不是真实模型，本文所有结论都不构成 M1/M3 验收），进程是发行包 `target/package/aarch64-apple-darwin/nekowriter/nekowriter`（驱动 A/B/C/D 与界面复核）与当前源码的 `target/debug/nekowriter`（缺陷复现与修复验证）。

### 28.1 四条支路的结果

| 支路 | 做法 | 结果 |
| --- | --- | --- |
| A 委托审核 | `POST /api/reviews/{id}/delegate` 委托背景产物，子 Run 完成后读回 | ✅ 202 + 子 Run `review.preparation.background` 成功；父 Review 变为 `approved` 且 `decision_source = agent`（不是 human）；界面在途显示「审核 Agent 处理中 / 正在依据当前修订审查」+「取消并自行审核」，完成后显示「已通过」 |
| A 审核角色的合同 | 抓取审核请求的完整请求体 | ✅ 审核（含委托）系统消息 214 字符，尾部就是 `{"passed":true,"confidence":0.0,"issues":[]}` 与字段约束；委托的意见作为 user 内容附上。§26 修的根因覆盖审核角色，不会重复出现同类失败 |
| B 暂停/继续 | 正文在途时 `POST /api/runs/{id}/pause`，到边界后 `resume` | ✅ 请求在途时 `pauseRequested = true`，界面显示「正在等待当前或下一次完整工具调用到达暂停边界」；命中边界后 `execution = paused`，界面显示「已在一条完整工具调用边界暂停；正式正文尚未因该调用改变」；这一次命中的是**结构化提交**边界，界面显示挂起的本章摘要（§5.3「不会无限显示等待暂停」）；`resume` 后继续跑到 `writing_chapter_reviewing` |
| B 暂停期间提交意见 | 暂停点上用界面提交一条自由意见 | ✅ 提交即清除暂停请求（`pauseRequested` 立刻为 false），工作流转 `queued → request_in_flight`；队列项显示「等待下一条工具结果」 |
| C 意见投递 | 在途投递意见，检查它是否真的进入下一次模型请求 | ✅ `feedback` 行 `queued → included`；下一次请求的 tool 消息里带 `feedback: [{id, text, citations}]`（§4.1 的投递形状）；驱动读 `body.pendingFeedback` 为 `null` 是探针自己的路径写错（该数组在快照顶层），不是接口缺陷 |
| D 在途中断恢复 | 在途请求下 SIGTERM → 重启 → `continue` | ✅ 0.055 秒退出码 0；重启后 Run 变 `interrupted`、Run id 不变、`checkpoint_json` 保留章节；在途请求 `status = unknown` 且 `error_json.kind = process_shutdown`（与 §11.2 一致）；`continue` 后新请求 attempt 2/3 成功并跑到 `writing_chapter_reviewing` |

### 28.2 找到并修掉的缺陷：无推进的提交会无限重排请求

在 C 支路上把模型换成「每次都直接提交本章」的响应后，工作流在 `writing_chapter_generating` 与 `queued` 之间反复循环，连续发出了 13 次以上请求而 `runs.consecutive_protocol_errors` 始终是 0，也没有任何停止迹象（`request_allowance_end = 100`，即最多可以这样烧掉 100 次真实模型调用）。

原因是「有未投递意见时直接提交」这条完成守卫的拒绝路径与协议错误的计数是分开的：它在 `preparation.rs` 的守卫分支里显式 `consecutive_protocol_errors = Set(0)`，而 §5.3 明确要求「若连续不产生可推进动作，触发工程执行限制并展示未投递意见」。修复是让这类纠正与无效模型响应共用同一个计数：每拒绝一次不推进的提交就加一，第三次改用 `stop_writing_after_protocol_error` 把工作流收束为 `stopped`（`wait_reason = protocol_error_limit`、`ended_at` 落库），事件里带上 `consecutive_protocol_errors` 与 `wait_reason`；调度器日志也按三种结果分开（工具循环 / 达到上限停止 / 进入整章审核），不再把「被上限停止」写成「已进入整章审核」。

**真实进程证据**（数据目录全新，桩固定只提交、不产生工具结果）：先让第一轮工具调用把 14 字正文写出来，再投递一条意见，然后改桩为「永远只提交」。修复前（发行包）：13 次以上请求、计数 0、不停止。修复后（当前源码 debug 进程）：

- 状态轨迹只有三步：`request_in_flight → queued → stopped(waitReason=protocol_error_limit)`；
- `runs` 行是 `('stopped', request_count = 4, consecutive_protocol_errors = 3)`，`model_requests` 恰好 4 条（1 次工具调用 + 3 次不推进提交），`agent_messages` 里恰好 3 条 `protocol_correction`；
- 未投递意见原样保留（`feedback.status = queued`、绑定为空、没有进入任何请求）；
- 界面显示「连续协议错误已停止 / 模型已连续三次返回不符合当前阶段协议的响应……显式继续可重新开始一轮请求额度」+「显式继续」按钮，同一面板的「交给正文 Agent 的意见」队列里仍是那条未投递意见；
- 点「显式继续」后 `resume_count = 1`、`request_allowance_end = 104`、计数清零，随后再次以最多 3 次不推进提交停止——上限是每次续授后的额度，不再无限重排。

仓储测试同步加断言：`direct_chapter_submission_with_pending_feedback_requeues_for_a_tool_result` 现在覆盖第三次提交收束为 `stopped`、`wait_reason = protocol_error_limit`、事件里 `consecutive_protocol_errors = 3` 与 `reason = pending_feedback_requires_tool_result`、未投递意见仍为 `queued`。`cargo test --workspace` 190 通过（agent 32 / app 44 / core 38 / store 72 / xtask 4），`cargo fmt --all -- --check` 干净。

### 28.3 本轮没有踩到与仍是缺口的

- 意见进入 `included` 之后的界面显示仍是缺口：队列项消失，工作台没有 §5.1 要求的「已随请求发送 / 请求失败，尚无模型回应」独立显示（投递本身已被请求体证明）。已写进 docs/01 的边界清单与 docs/02 §5.1。
- 委托审核只要到「结论被应用」这一层，没有在界面里点击「取消并自行审核」（接口路径在 §15 的用例里已覆盖）。
- 暂停只踩到「结构化提交边界」这一次；工具边界挂起由 store 用例覆盖，本轮真实进程没有重现到它。
- 模型仍是桩：真实模型下的准备→细纲→正文→定稿验收与 §15 的完整竞态矩阵仍未完成，本机没有凭据也没有本地运行时（§25、§27）。
- 支路探针的桩脚本在本轮改用 `always_submit` 开关；`nohup` 的后台子进程在 macOS 上会随调用方进程组一起被收走，因此改用 `start_new_session=True` 的启动器（`/tmp/nw-detach.py`），这是探针基础设施，不影响产品代码。

## 29. 2026-09-12 增量更新：修复后的状态收口（全仓测试、重建发行包、真实产物上的完整回路与两条路径）

上一轮（§28）修掉的「无推进提交会无限重排请求」落在上一次发行包之后，本轮把它连同整个交付状态一起收到可交付：测试、重建产物、在产物上重跑完整回路与两条相关路径、清理残留进程，最后同步文档。本轮的模型仍然只有桩（`/tmp/nw-branch-stub.py`），因此凡涉及生成结果的结论都只适用于桩，不是真实模型验收。

### 29.1 全仓测试与格式

`cargo test --workspace`（`--config 'env.PROTOC.value="/opt/homebrew/bin/protoc"'`）**190 通过、0 失败**：agent 32、app 44、core 38、store 72、xtask 4。上一轮新增的上限断言就在其中：调度器用例 `direct_chapter_submission_with_pending_feedback_requeues_for_a_tool_result` 覆盖第三次不推进提交收束为 `stopped`、`wait_reason = protocol_error_limit`、计数为 3、未投递意见仍为 `queued`；store 用例 `protocol_errors_requeue_with_paired_results_then_stop_at_the_limit` 覆盖同一上限的配对结果重排队。`cargo fmt --all -- --check` 干净。本轮**没有改任何代码**，因此这 190 个用例就是发行包里那一份代码。

### 29.2 发行包按当前源码重建并独立核对

从当前源码执行 `cargo xtask package --release --protoc /opt/homebrew/bin/protoc`，产出 `target/package/aarch64-apple-darwin/nekowriter/`。独立脚本（不复用打包器逻辑）核对结果：

| 项目 | 结果 |
| --- | --- |
| 清单 | 13 个文件、共 263,354,796 字节；逐行重算字节数与 SHA-256 全部一致；磁盘上没有未列出的文件 |
| 可执行文件 | 262,589,360 字节；`file(1)` 为 `Mach-O 64-bit executable arm64`；头部 cputype `0x0100000C` |
| 构建配置与三元组 | 清单记录 `release` 与 `aarch64-apple-darwin`，容器/架构行是 `Mach-O 64 位（aarch64）` |
| 前端资源 | `index.html` 引用的 `assets/index-BvTut9ud.js` 与 `assets/index-z3WZ9jpB.css` 都在包内 |
| 结论 | `VERIFY: 0 problem(s)` |

与重建前的清单相比形状不变（同样 13 个文件、同一个三元组与容器/架构写法），二进制 SHA-256 与上一份不同（修复改了源码，理应如此）。探针进程启动的是这份产物的副本：`/tmp/nw-final/pkg/nekowriter` 的 SHA-256 与 `target/package/.../nekowriter` 相同（`7b39114427ae…`），所以下文的完整回路、两条路径与界面复核都发生在**重建产物**上。

### 29.3 重建产物上的完整回路

驱动 `/tmp/nw-final-e2e.py` 用真实 HTTP 接口走完整条链路，产物记录在 `/tmp/nw-final/e2e.json`：

| 步骤 | 结果 |
| --- | --- |
| 建项目 → 存目标 → 启动准备 run | ✅ 桩提交四项产物，`preparing_generating → preparing_reviewing` |
| 逐项「通过」四项准备产物 | ✅ 四项 `approved` → `writing_chapter_ready` |
| 「开始细纲」→ 通过 → 进入正文 | ✅ 细纲审核通过并推进 |
| 正文工具调用 + 整章终稿 | ✅ 第 1 章 `approved`（`final_review` 决定落库），`chapter_body` 修订内容为「桩按合同推导的第12段内容。」、`word_count = 14` |
| 导出 | ✅ `GET /api/versions/{id}/export?format=md|txt` 都是 200，106 / 101 字符；正文与库内 `chapter_body` 当前修订逐字一致，文件名取自小说标题「雨幕」，头部写明「正文 1 / 1 章已通过整章审核」；`format=markdown` 仍按规范返回 422（只接受 `md` / `txt`） |
| 概览对账 | ✅ 项目口径 `输入 200 / 输出 100 / 请求 4 / 用量未知 0`，`byRole` 只有 `writing`（4 次），按日只有 9/13（300 token）；与库内 `model_requests` 联接 `runs` 后逐行求和（4 行 → 输入 200、输出 100、未知 0）完全一致；正文净字数「区间净增 14 / 当前总量 14」与本章 `chapter_body` 当前修订的 `word_count = 14` 一致 |

### 29.4 两条相关路径

| 路径 | 证据 |
| --- | --- |
| 反复不推进的章级提交必须到上限即停 | 数据目录全新、桩固定「永远直接提交」：状态轨迹只有 `request_in_flight → queued → stopped(protocol_error_limit)`；`runs` 行为 `('stopped', request_count = 4, consecutive_protocol_errors = 3)`；`model_requests` 恰好 4 条（全部 `succeeded`）；3 条 `protocol_correction`；未投递意见保持 `('queued', bindings = 0)`。修复前在同一脚本下是「13 次以上请求、计数 0、不停止」 |
| 意见在能投递时必须真的进入下一条请求并推进 | 在途投递一条自由意见后：`feedback` 行从 `queued` 变 `('included', sequence = 1, bindings = 1)`，`INCLUDED REQUEST 43def9a0-…` 命中的正是捕获到的请求快照（该请求的 tool 消息里带 `feedback: [{id, text, citations}]`），工作流由 `writing_drafting` 推进到 `writing_chapter_reviewing`、正文追加了新段落，`pendingFeedback` 清空 |

### 29.5 重建产物上的界面复核（真实浏览器）

| 步骤 | 结果 |
| --- | --- |
| 阅读视图 | ✅ 「共 1 章」、虚拟化列表挂载第 1 章、正文「桩按合同推导的第12段内容。」完整可读 |
| 整稿查找 | ✅ 输入「第12段」→「找到 1 处（最多显示 50 处）」，结果落在当前章 |
| 回收站（模板往返） | ✅ 点「删除」后**未点刷新**，回收站自己变成「1 项已删除 / 提示词模板 / 终验模板 / 2026/9/13 00:25 删除」并给出「恢复 / 彻底删除」；点「恢复」后回收站回到「回收站是空的」、模板下拉自动重新出现该模板（接口 `/api/prompt-templates` 也返回它）；再删除 → 点「彻底删除」→ 二次确认「确认彻底删除」→ 回收站空、状态「已彻底删除提示词模板「终验模板」，无法再恢复」、接口列表为空 |
| 回收站边界 | ✅ 恢复已彻底删除的 id、恢复不存在的 id、彻底删除不存在的 id 都是 `404 RESOURCE_NOT_FOUND` |
| 概览 | ✅ 与 §29.3 的对账是同一批数据；页面显示与库内记录一致，未点刷新 |
| 本地服务面板与退出 | ✅ 面板显示界面地址、实际数据目录与「确认退出」；点「确认退出」后日志依次写「收到界面退出请求，正在停止服务」「正在取消在途请求并收尾状态」「NekoWriter 已退出 writing_runs=0 review_delegations=0」，进程退出码 0、`-wal` / `-shm` 消失 |

### 29.6 收尾与残留

界面退出后核对：`pgrep` 无 `nekowriter` 与任何桩进程，`lsof` 在 478xx / 479xx 上没有任何监听端口，数据目录只剩 `nekowriter.sqlite3`（无 WAL 旁文件）、`lancedb/`、`logs/` 与 0 字节的进程锁。实验数据与产物保留在 `/tmp/nw-final`（`pkg/`、`data/`、`requests*/`、`e2e.json`、`limit-probe.txt`、`feedback-probe.txt`）供复查。

### 29.7 仍未验证的边界

- **真实模型验收仍未完成**：本机没有可用凭据，也没有本地模型运行时（§25、§27）。本轮所有生成、审核与用量数字都来自桩，不能当作真实模型的通过。
- **非 macOS 平台仍无产物**：`rustup target list --installed` 只有 `aarch64-apple-darwin`，本机没有 Windows 目标的链接器与标准库，也没有 x86_64 的 macOS / Linux 目标；配方与 CI 方案在细则 §11.5，未经真实验收。
- **release 产物未做符号裁剪**（262.6MB）。未签名、未强化运行时，拷到别人的 Mac 上仍会被 Gatekeeper 拦下。
- 当时意见进入 `included` 后界面不显示「已随请求发送」的独立状态（§28.3 的既有缺口，本轮再次观察到队列项直接消失）；该项已在 §31 的后续收口中修复。
- 委托审核的「取消并自行审核」、暂停/继续的完整竞态矩阵与前端重连仍只有 §15 的用例层覆盖，本轮没有在界面里重跑。

## 30. 2026-09-13 增量更新：§15.2 竞态矩阵的真实进程逐行重跑

目标是把细则 §15.2 的高风险用例在真实进程上逐行跑掉。证据标准沿用前几轮：真实 `nekowriter` 进程 + 真实 HTTP/浏览器，模型是本地桩（`/tmp/nw-branch-stub.py`，**不是真实模型**，凡结论来自桩都在此说明）；逐行判定与证据写在 `/tmp/nw-mx/matrix-results.json`。

### 30.1 逐行结果

| 组 | 行 | 结果 |
| --- | --- | --- |
| A | A01–A03 | 通过 |
| A | A04–A06、S01–S04 | 未验到（被 30.3 第 2 条缺口阻断） |
| E | E01–E05、E07 | 通过 |
| E | E06 | 真实接口不可构造（待审提案期间用户编辑被 WaitingReview 守卫拒绝） |
| E | E08 | 通过（本轮修复，见 30.2） |
| R | R01–R04 | 通过 |
| R | R05 | 真实接口不可构造 |
| F | F01–F04、F06、F08–F10 | 通过（F09 修正场景后通过） |
| F | F12 | 未验到（桩无法产出合法的 `ask_user` 参数） |
| F | F05、F07、F11、F13–F15 | 未验到（脚本未覆盖） |
| Q、K、C、D、U、P | 全部 | 未跑（D02/D03/D07/D08 另在真实浏览器里部分验证，见 30.4） |

逐行证据与判定文本（含快照字段与 SQLite 行）见 §15.3 的同名表；这里只记取证据的方式：每一行都用真实 HTTP 触发状态迁移，再用 `GET /api/chapters/{id}/body` / `GET /api/versions/{id}/preparation` 快照和直接读 SQLite（`runs`、`model_requests`、`tool_calls`、`agent_messages`、`feedback`、`reviews`、`revision_dependencies`、`questions`）交叉核对。

### 30.2 本轮修掉的缺陷

**E08 之前是坏的**：`nekowriter-agent::provider` 的流式适配器把「一次响应里出现多个工具调用」当作传输错误（`ParallelToolCalls`），于是整个请求以 `provider_error` 收场——真实进程实测该 Run 直接 `failed`、`tool_calls` 表零行，模型永远等不到「请改为串行」的纠正。这与细则 §4.3「本次批次全部不执行，给每个调用 ID 返回 `PARALLEL_TOOL_CALLS_NOT_ALLOWED`，保持供应商消息配对，再要求模型串行调用」正面冲突。修法是把单一职责放回正确的地方：适配器只负责把响应里的每个调用完整还原（仍校验 ID/名称/参数/type），「一轮只允许一个」由阶段守卫唯一判定；调度器遇到 `ParallelToolCalls` 时写一条带全部调用的 assistant 消息、为每个调用 ID 写配对的 `PARALLEL_TOOL_CALLS_NOT_ALLOWED` 工具结果、再追加串行纠正，`ModelProtocolCorrection` 新增 `tool_result_code` 承载原因码。修复后的真实进程证据：助手消息保留两个调用，`agent_messages` 里两条 tool 结果分别对 `branch-call-117-a` / `-b`，`code = PARALLEL_TOOL_CALLS_NOT_ALLOWED`，随后一条 `protocol_correction`，Run 继续。

**协议纠正消息此前不给具体原因**：解析器已经算出「准备提交必须恰好包含背景、资料、人物和粗纲四项产物」，但写给模型的纠正只有通用措辞，模型只能猜。现在纠正文本追加 `本次未通过的原因：…`。

**core 单测回归**：上一轮为修「在途编辑把工作流留成没有继续入口的空闲态」改了 `record_chapter_body_edit`（回合进行中保持 `Queued`），但 `chapter_guidance_and_outline_start_require_the_active_chapter` 仍断言编辑后是 `Idle`，全仓测试因此从 190 变成 189 通过 1 失败。现在同一用例同时断言两种语义（回合进行中保持 `Queued`、无回合时回到 `Idle`），`cargo test --workspace` 190 全通过，`cargo fmt --all -- --check` 干净。

### 30.3 发现但未修的缺口

1. **重复的非法工具调用没有工程上限**。桩在细纲阶段一直返回参数不符 schema 的 `ask_user`，真实进程连续发出 76 次请求、`consecutive_protocol_errors` 始终为 0，最后 Run `failed`；`tool_calls` 里 76 条 `ask_user` 全部是 `TOOL_ARGUMENTS_INVALID`。「不推进」的纠正计数只覆盖阶段提交，工具结果错误走的是另一条路，模型重复同一个非法调用会把请求额度耗光。
2. **准备修订轮的提交合同与 store 规则互相矛盾**。系统消息里的合同要求 artifacts 恰好四项齐全，store 在修订轮只接受「需要修订的项」，真实进程实测模型按合同提交后 `submission_commit` 失败（「资料收集 当前不需要修订，不能覆盖已通过或待审修订」），工作流停在 `execution = failed` 并阻断后续矩阵行。这与上一轮修掉「合同从未发给模型」属于同一类问题：模型唯一的提交依据必须与校验规则一致。
3. **暂停在不可应用的提交边界时 `resume` 返回 422**。`POST /api/runs/{id}/resume` 给出 `VALIDATION_ERROR`（内层「章节正文不能为空」），Run 永久停在 `paused`，只能被显式停止；F09 要求的是「停在提交边界且可继续，无永久等待」。
4. **`preview.delta` 未实现**。细则 §675/§683 与 D09 要求的临时生成预览事件（携带 request/tool/offset、不带 SSE id、不参与持久游标去重）在 `crates/` 与 `webui/src/` 中都不存在，因此 D09 无法验证。

### 30.4 前端重连（真实浏览器）

在真实进程的预览页里用 `fetch` 读 SSE 原始流并主动制造真实写入：

- D02：快照水位 1960 之后开流，真实写入带来 1961 `settings.changed`、1962 `document.committed`，序号严格递增、无重复。
- D07：以 `Last-Event-ID: 1962`（不带 URL 游标）重连，只收到 1963、1964，不重放 1962，也没有重启 Run。
- D08：`?after=abc` → 400 `INVALID_EVENT_CURSOR`；`?after=<水位+100000>` → 200 `stream.reset_required`（`reason=cursor_ahead` + watermark）。
- D03：同一连接内序号严格递增且去重；重连不重放已确认的游标。

### 30.5 清理与复现

矩阵驱动脚本为 `/tmp/nw-mx-a.py`（A 组）、`nw-mx-b.py`（E/R 组）、`nw-mx-c.py`（F 组），共用 `nw-matrix-common.py` / `nw-matrix.sh`；单行复现脚本为 `nw-mx-e03.py`、`nw-mx-f09.py`、`nw-mx-a03b.py`。实验数据留在 `/tmp/nw-mx/`（含 `matrix-results.json`、`app.log`、`stub.log`、`requests/`）供复查。

## 31. 2026-09-13 后续收口

§30.3 的四项缺口现已修复；原节保留的是发现时的取证结果，以下记录当前实现和覆盖范围。

1. **工具错误有统一的三次上限。** `ask_user`、知识工具和正文工具的错误结果都会递增 Run 的 `consecutive_protocol_errors`；第三次把 Run 与工作流停止为 `protocol_error_limit`，成功工具结果会清零。store 用例覆盖连续非法提问，scheduler 用例覆盖连续非法正文工具。
2. **准备修订合同与 store 接受规则一致。** runner 直接从 `required_artifacts` 生成提交合同，只列本次要重修的产物。研究项和粗纲项的回归用例同时确认必填项与禁止混入的已通过产物。
3. **不可应用的暂停提交可以恢复。** 恢复时若没有可提交的正文或依赖，服务端保留原模型文本、写入纠正消息、清除 held submission，并从 `paused` 重新排队；用例确认下一次请求仍可调度。
4. **D09 临时预览已接通。** 调度器把文本和工具参数流发送到独立内存广播，SSE 用 `preview.delta` 推送且故意没有 `id`，工作台按请求、工具槽位和 Unicode scalar 偏移展示暂存内容。持久请求事件、断线和切换稿本都会丢弃暂存内容。路由集成用例读取真实 SSE 响应并断言没有 `id:` 行。
5. **意见投递状态已经可见。** 正文快照读取当前 Run 最近一个尚未成功响应的请求：其 `included` 意见以 `deliveredFeedback` 单列返回，并给出 `sent`、`failed`、`unknown` 或 `cancelled`。Dock 继续显示 `queued` / `bound` 队列，同时把这组意见展示为「已随请求发送，等待模型回应」或对应的失败/未知/取消原因。现有反馈投递用例同时断言发送中和失败两种状态。

本轮 `cargo test --workspace` 通过 197 个用例（agent 34、app 47、core 38、store 74、xtask 4），前端 `pnpm --dir webui build` 通过。真实外部模型的一章最小创作闭环已在 macOS aarch64 上完成；非 macOS 发行产物、完整 §15 竞态矩阵、完整浏览器路径以及 M3 的多章/复制稿本/番外路径仍是验收边界。以上为 §31 当时的阶段性记录，当前结论见 §33–§34。

## 32. 2026-09-13 真实模型 reasoning_effort 全流程验收

在 macOS `aarch64-apple-darwin` 上使用 InferX `deepseek-v4.1-flash` 完成真实进程全流程，过程与细则 §15.6 对齐：`reasoning_effort=low` 的直连请求与应用 `/api/settings/test-model` 探针均成功，探针返回 `thinkingProbed=true`，随后系统能力保存为 `thinking=true`、项目思考等级保存为 `low`。

真实项目完成准备四产物审核、章节细纲审核、正文四次 `append_content`、终态提交、整章审核和完本确认。正文当前修订 4、4,683 字符，版本工作流为 `completed`；Markdown/TXT 导出均为 200，阅读窗口返回 1 个定稿章节，整稿搜索“林舟”返回 17 处。写作请求 11 次中，已报告用量合计输入 48,183、输出 10,111、推理 1,401；4 次请求因容量 429、首轮混合文本/工具协议错误或流空闲超时没有可用用量，按账本保留为未知。

真实流暴露两处适配问题并已修复：正文阶段带工具时去掉 `response_format=json_object`，以及在工具调用响应中忽略供应商附带的 reasoning 文本而保留工具参数。后者新增 Provider 回归测试；当前 Agent 测试 34 个全部通过。混合流修复后同一 Run 可继续完成正文，未增加兼容回退路径。

## 33. 2026-09-13 M2/M3 macOS aarch64 验收完成

§31 与 §32 保留的是当时的阶段性结论；本节是当前状态。范围严格限于 macOS `aarch64-apple-darwin`，不把 Windows、Linux 或其它 macOS 架构纳入通过结论。

M2 的 A/E/F/R 可达用例已完成回归覆盖。新增断言补齐了 F13（工具请求封存前的追加意见）、F12/F14（`ask_user` 等待回答时的意见批次）与 F15（暂停工具边界的完整批次）；E06、R05 所要求的并发写入被公开状态机拒绝构造，服务端冲突校验仍保留。真实浏览器复核通过 EventSource、稿本与项目切换、三章阅读、整稿搜索与高亮、中文组合输入、章节详情、浏览器导出、主题、快捷键、项目设置保存与项目间设置隔离。设置面板保存成功提示曾被随后的 `settings.changed` 重读立即清除，现只在切换项目时清理旧项目的提示。

M3 使用真实 InferX `deepseek-v4.1-flash` 端点和 `reasoning_effort=low` 完成两章主线、一个番外、完本确认与稿本复制；源稿本和副本均核对到三篇正文，Markdown/TXT 导出成功。该次写作共 15 个请求，记录输入 57,784、输出 13,428、推理 7,800 个 token，成功请求保留有效思考等级与非零推理用量。

最终验证为 `cargo test --workspace` 198 通过（agent 35、app 47、core 38、store 74、xtask 4）、`cargo fmt --all -- --check` 通过，前端 `pnpm --dir webui build` 通过。M0–M4 与 macOS aarch64 的 M5 范围已完成；全平台计划只剩非 macOS 发行产物的目标环境构建和验收，按用户范围不阻塞本次结论。

## 34. 2026-09-13 内建提示词升级收口

在完成现状调查并得到用户同意后，重写 `nekowriter-agent/prompts/` 下 10 个用途的 system/user 共 20 个内建提示词，并在 `crates/nekowriter-agent/src/prompt.rs` 增加阶段质量断言。实现保留现有变量、工具白名单、结构化提交 JSON、提示词覆盖优先级和请求快照协议，没有增加兼容层或新的状态轨道。

新版提示词把准备、细纲、正文和各类审核按实际职责分开：准备阶段固化可随折叠产物传递的创作约定，细纲明确起始状态、知情边界、场景目标/阻力/选择/后果与收束，正文强调人物声音、具体场景、因果连续性和增量工具编辑，审核按可见依据区分阻断问题与建议，并要求可定位、可修复的意见。质量原则参考 [no-ai-slop 编辑规则](https://github.com/petergyang/no-ai-slop/blob/main/skills/no-ai-slop/SKILL.md) 与本地 [NovelOps writer 提示词](/Users/misaka/Downloads/novel-ops-v1.0.0/novel-system/static/prompts/writer.md)，没有复制固定字数、AI 比例或跨题材禁词表。

验证结果：`cargo fmt --all -- --check` 通过，工作区 198 个 Rust 测试全部通过；macOS `aarch64-apple-darwin` 使用 InferX `deepseek-v4.1-flash`、`reasoning_effort=low` 完成准备四产物、两章主线、一个番外、审核、完本、稿本复制和 Markdown/TXT 导出。三章均为已通过正文，16 次写作请求记录输入 69,857、输出 15,209、推理 9,705 个 token；请求快照保留有效思考等级，生成产物包含可传递创作约定和章节因果信息。

## 35. 2026-09-13 阶段提交改为提交工具调用

§15.6 与 §32、§33 里真实端点跑通的准备阶段，在换成 `deepseek-v4.1-flash` 后暴露出一个此前被掩盖的失败模式：模型连续三轮把 10 章章节清单写进 `outline` 产物的正文，从不输出顶层 `chapter_plan` 数组；三轮提交都是合法 JSON，但缺了合同要求的字段，于是同一个 `protocol_error` 被连续记录了三次，Run 以 `waitReason = protocol_error_limit` 停下。

根因不在模型，而在协议本身。阶段提交原本是一段散文 JSON：模型唯一能得知「缺的是哪一项」的渠道是一条纯文本的用户纠正消息，而与提交同一轮的辅助文本无法携带字段级回执；`response_format=json_object` 又让 `OpenAI` 兼容服务把「本轮必须给 JSON」理解为「本轮不要调用工具」，工具与终态提交因此在带工具的阶段互相排斥（§15.6 记录过正文阶段为此去掉该字段）。

本次把这层协议整体换成工具调用，并删掉随之失效的旧路径：

- 每个阶段声明恰好一个提交工具（`submit_preparation` / `submit_chapter_outline` / `submit_chapter` / `submit_review`）。工具 schema 与 `parse_phase_submission` 同源：准备阶段的产物清单按本轮真正需要提交的项生成，本轮含粗纲时 `chapter_plan` 是 `required`；章节级 `chapter_id` 被声明成单值 `enum`，模型没有推断余地。
- 提交参数校验失败时，被拒的那次调用拿到一条配对的错误工具结果，里面是解析器给出的具体原因（缺哪一项、哪个取值非法），模型可以在同一次调用里补齐；成功提交写配对的 `applied` 结果。账本因此始终成对，`serialize_request_messages` 的 `MissingToolResult` 校验在重放准备、细纲、正文历史时不会失败。
- 没有提交工具调用的纯文本终态不再被当作提交：它按协议错误处理，纠正消息点名该调用哪个工具，Run 仍在剩余额度与连续协议错误上限内继续。
- 所有阶段去掉 `response_format`，并删除 `ModelCapabilityProfile.structured_output` 与前端「结构化提交」开关；工具能力因此成为写作与审核共同的硬要求，`pre_dispatch_capability_problem` 不再需要按用途分支。
- 细纲提交去掉 `goals`：它一直被解析校验却从未落库，属于合同里承诺了却不存在的字段。

验证：`cargo test --workspace` 202 通过（agent 39、app 47、core 38、store 74、xtask 4）、`cargo fmt --all -- --check` 通过、`cargo clippy --workspace --all-targets` 无新增告警（余下的是既有基线）、`pnpm --dir webui typecheck` 与 `build` 通过。调度器的假服务改为真的发出 `tool_calls` 增量与 `tool_finish_reason`，准备、细纲、正文三条 lane 的端到端断言据此更新；账本消息数相应地从「1 条 assistant 文本」变为「提交调用 + 配对结果」。

### 35.1 真实 InferX 端点复跑（同日随后执行）

复跑口径：用当前源码构建的 `target/debug/nekowriter --data-dir /tmp/nw-toolcall-replay`（监听 127.0.0.1:8799），系统写作模型改指本机记录代理 127.0.0.1:8798，代理逐字节转发到 `https://model.inferx.net/endpoints/v1` 并把请求体与响应流落盘；代理唯一附加行为是在上游返回 429 时按固定间隔重试（端点当时经常整体无容量）。模型 `deepseek-v4.1-flash`，能力 streaming / tools / streamUsage / thinking，`maxContextTokens = 65536`，`reservedOutputTokens = 8192`，项目 `thinkingLevel = off`。新建 v2 稿本，用同一句小说目标（「写一个三章短篇：雨夜，值夜班的车站播音员收到一封预告列车事故的匿名信。」）发起准备阶段。

**请求侧：改动已经在真实请求里生效。** 准备阶段第一轮请求体 6,988 字节，顶层字段恰好是 `messages, model, parallel_tool_calls, stream, stream_options, tool_choice, tools`——不再有 `response_format`。`tools` 两项：`ask_user`（required `questions`）与 `submit_preparation`（required `artifacts, chapter_plan`）。system 末尾的阶段提交契约原文为：

> 阶段提交：本轮结果只能通过 `submit_preparation` 工具提交，不要在普通文本里输出 JSON，也不要在工具调用之外附加说明。
> 工具参数的 `artifacts` 必须恰好包含 background、research、characters、outline 各一次（本轮只需提交这些产物），每项 content 不能为空。
> `chapter_plan` 是本轮章节清单的唯一承载字段，至少一项；只把章节清单写进粗纲的 content 不算提交，它必须放在 chapter_plan 里，且标题、顺序与目标同粗纲一致。

请求里没有 `max_tokens`，`reservedOutputTokens` 只影响上下文预算，不构成这把输出上限。

**模型侧：不再走散文 JSON。** 真实响应流里出现的就是提交工具调用，例如 `data: {…,"delta":{"tool_calls":[{"index":0,"id":"call_05aab32cec87464db8b5850d","type":"function","function":{"name":"submit_preparation"}}]}}`，随后 `arguments` 以 `{"artifacts":[{"kind":"background","content":"【创作约定】…` 开头。原来那种「10 章清单塞进 outline 正文、顶层没有 `chapter_plan`」的形态在协议上已经不存在：散文 JSON 不再是一条可被接受的提交路径。

**但这条真实请求没有跑完，原因在端点侧。** 应用侧的两次真实请求分别记到 `模型服务在首包阶段超时`（上游 429 把 45 秒首包窗口耗尽）与 `模型服务在流空闲阶段超时`；代理捕获的响应流停在 `research` 产物正文中间，`finish_reason` 与 usage 都没到。为把端点行为与应用的超时分开，用同一份请求体绕过应用直连端点复放：总共 850,076 字节 / 270 秒，**中途出现 134.2 秒完全没有字节的空档**（应用的流空闲上限是 90 秒），结束时给出 `finish_reason:"tool_calls"`、usage `prompt 1802 / completion 9127 / reasoning 4526 / cached 1792` 与 `[DONE]`，但 `arguments` 字符串在 1,889 字符处被截断（正文停在「…其中包括一」后直接跟 finish 事件），因此这轮工具调用的参数不是合法 JSON。两个事实同时成立：端点静默超过应用上限，请求会在拿到参数前就被中止（本次实际先到的就是这个错误）；而参数即便送达也已经是截断的——当时 `provider.rs` 把无法解析的工具参数记成 `ProviderError::InvalidStreamToolArguments`，所以它会以 `provider_error` 结束这一轮，而不是走「错误工具结果 + 纠正消息」那条协议路径，本轮 Run 因此需要用户显式继续。这一处分类缺陷已在 §35.2 修复；端点侧的静默与 429 未动，仍属下一步与用户确认的范围。

**提交工具这条通道本身可用，字段级要求也已取得真实证据。** 受控探测（同端点、流式、`tools` 里一个 `submit_text`，required `payload`）在 400 / 2000 / 6000 字符三档上分别返回 418 / 2292 / 3271 字节的 `arguments`，全部完整合法、`finish_reason = tool_calls`，说明多 KB 的参数可以正常送达；以与 `submit_preparation` 相同的 schema（required 含 `chapter_plan`）发一次最小真实请求，模型返回完整工具调用，`chapter_plan` 是顶层数组、三项标题与目标齐全（如「第一章：末班车的第三封信」+ 该章需要发生什么）。因此「模型会把章节清单放进 `chapter_plan`」在真实端点上成立。

**仍未取得的部分**：这条「四项产物 + 章节计划」的巨大提交没有在真实端点上完整落地，`docs/01` §8 的退出标准仍以完整跑通为准，本节不声称准备阶段端到端真实验收通过。复跑途中反复出现的端点侧约束是：`service failure: endpoint deepseek-v4.1-flash all replicas at capacity` 的 429（按 5 秒间隔重试时，一次请求在 45 秒首包窗口内重试 10 次仍未拿到 200；按 3 秒间隔重试时到第 6 次、约 26 秒后才拿到 200）、上面那条 >134 秒静默、以及这轮 `arguments` 的截断。是否为此放宽模型超时、下调单轮产物体积或改变提交粒度，属于与用户确认后的下一步。

### 35.2 不可解码的提交工具参数改按「被拒的提交」处理

§35.1 暴露的不只是端点行为，还有一处分类缺陷：上一轮提交工具的 `arguments` 不可解码（被截断、不是合法 JSON、不是 object）时，`provider.rs` 拿它当传输失败，整轮以 `provider_error` 收场。模型既拿不到自己那次调用被拒的配对回执，也没有任何可重试的路径，Run 只能等用户显式继续——这与「阶段结果必须通过提交工具提交，失败由调用方变成配对回执 + 纠正消息」这条协议正面冲突。本轮只修这一处，端点侧的首包/流空闲超时与 429 重试策略按用户边界未动。

**改动的分界线在「谁的输入坏了」。** 参数是模型给的，就不是传输失败：适配器负责把响应里的每个调用完整还原，解码责任交给阶段守卫。因此 `complete_stream_tool_call` 不再对不可解码的参数报错，而是把模型给出的原文（JSON string）连同调用 ID、名称一起保留；`ModelToolCall::decode_arguments` 给出具体原因（「参数不是合法 JSON：…」/「参数必须是 JSON object」），来源是模型的那次调用就走被拒提交（`InvalidPhaseSubmission` + 参数原文），业务工具调用走 `UndecodableToolArguments`（同样配对错误结果 + 纠正）。两条路径都计入既有的连续协议错误上限，也就是模型可以在限额内补齐后重跑。

同一个缺陷类的另一条出口是单参数 256 KiB 上限（`MAX_TOOL_ARGUMENT_BYTES`、`ToolArgumentsTooLarge`、`ToolCallArgumentsTooLarge`）：它同样是「模型把参数写长了」变成整轮传输失败。单次响应总上限（2 MiB）已经界定了参数体积，独立的参数上限只多造一条不可纠正的死路，因此随本轮一并删除；`validate_complete_tool_call` 现在接受 object 与原文两种形态——不可解码的那次调用必须能连同配对结果一起重放，否则重放的配对校验会把可纠正的协议错误升级成 `MissingToolResult`。

**账本级回归测试**（`crates/nekowriter-app/src/scheduler.rs`，与「被拒的提交」共用同一条真实链路：假 SSE 服务 → 适配器 → 阶段守卫 → Store 事务）。

| 测试 | 断言 | 修复前 | 修复后 |
| --- | --- | --- | --- |
| `submission_without_a_chapter_plan_records_a_paired_rejection` | 缺 `chapter_plan` 的提交：请求终态 `protocol_error`（不是 `provider_error`）、配对的错误工具结果含 `PHASE_SUBMISSION_INVALID` 与 `chapter_plan`、随后一条 `protocol_correction`、`consecutive_protocol_errors = 1`、历史重放为「调用 + 配对结果 + 纠正」三连 | 通过（守护 §35 的提交工具重构） | 通过 |
| `truncated_submission_arguments_are_rejected_instead_of_failing_the_run` | §35.1 那轮被截断的参数：原文逐字保留在账本、配对错误工具结果含 `PHASE_SUBMISSION_INVALID` 与「不是合法 JSON」、纠正消息、`consecutive_protocol_errors = 1`、历史里调用与结果配对 | **失败**：`这一轮提交必须留下配对的工具结果: Elapsed(())`——整轮根本不落库 | 通过 |

第二条的「修复前」不是推断：把 `complete_stream_tool_call` 的不可解码分支临时改回返回 `ProviderError`（备份原文后改动、跑完立即 `cp` 还原，还原后全量测试重跑通过），该测试即以上面那条 panic 失败；第一条同一时刻仍通过，说明它守护的是上一轮重构而不是本轮修复，两条各司其职。

**验证**（一次批量）：`cargo test --workspace` 207 通过（agent 42、app 49、core 38、store 74、xtask 4）、`cargo fmt --all -- --check` 干净、`cargo clippy --workspace --all-targets` 无新增告警。真实端点复跑留给下一轮：本轮只证明「参数不可解码时协议仍然闭合」，不证明端点能把这份 8.5 KB 参数完整送达。

**排查：其余仍会致死的坏提交路径**（同一次通读四条 lane 的 `classify_model_completion` 分支）。

1. **委托审核 lane 没有纠正回路**（`scheduler.rs::handle_delegated_review_completion`，`classify_model_completion(PhaseSubmissionKind::Review, …)` 的 `Err` 分支）。准备、细纲、正文三条写作 lane 的 `Err` 都交给 `record_model_protocol_correction`，审核 lane 却直接 `persist_delegated_review_failure(…, "protocol_error", …)`，由 `fail_delegated_review_attempt` 把审核重新开放。这不是本轮修复覆盖的四条提交路径中的同一处缺陷，而是一种不同的设计：审核结论不可纠正，失败只重开审核，模型也拿不到配对回执。它按缺陷上报、本轮不改——要改成写作 lane 那样，需要先决定「审核失败重开」与「让模型改写结论」哪个才是要的行为。
2. **提交已通过但 `commit_*_model_submission` 写库失败时，assistant 工具调用会落成未配对记录**（`persist_model_failure(…, "submission_commit", …)` → `resolve_model_request_inner` 只追加 assistant 消息）。这类 Run 以 `failed` 结束，但用户显式继续会在同一个 Run 上重放历史，`serialize_request_messages` 会以 `MissingToolResult` 拒绝——同样是「一个坏掉的消息序列变成 provider error」。它与本轮修的分类缺陷不同源（模型这次没说错，是本地事务失败），因此只上报。
3. 纯文本终态已不存在可用路径：`classify_model_completion` 对空 `tool_calls` 返回 `MissingSubmissionToolCall`，三条写作 lane 都会把它变成点名提交工具的纠正消息（已由 §35 的 `preparation` lane 用例与 runner 单测覆盖）。

### 35.3 真实端点完整复跑：准备阶段端到端跑通（同日 07:23–07:39）

这一轮的目的是取回 §35.1 缺的那一段：**在同一条真实链路上确认模型把章节清单放在顶层 `chapter_plan`，而不是塞进粗纲正文**。

**装置。** 应用是当前源码的 debug 构建（`cargo build -p nekowriter-app`），`--data-dir /tmp/nw-rerun/data`、监听 127.0.0.1:8801，由 `launchctl submit -l com.nekowriter.rerun` 拉起（不经终端会话，避免被信号回收）。驱动脚本 `/tmp/nw-rerun/{drive,answer,review}.py` **只调应用自己的 HTTP API**（`GET/PUT /api/settings` → `POST /api/settings/test-model` → `POST /api/projects` → `PUT /api/versions/{id}/brief` → `POST /api/versions/{id}/preparation/start` → `POST /api/questions/{id}/answer` → `POST /api/reviews/{id}/delegate`），模型请求全部由应用自己构造，没有手工拼过。抓包是 `/tmp/nw-rerun/proxy.py`——**透明中继**：不重试、不改写、不补字节，上游给什么就给应用什么，同时把每次请求体写入 `requests.jsonl`、响应流写入 `responses/`，并把上游状态码、首包时间、总耗时与**最长零字节空档**逐条记入 `timeline.jsonl`。

**模型替换（必须写明的环境事实）。** `deepseek-v4.1-flash` 在本轮整个窗口内不可用：直连探测 8/8 都是 429 `service failure: endpoint deepseek-v4.1-flash all replicas at capacity`（07:17 一次、07:42 复核一次）。因此写作模型换成同一端点、同一凭据下的同族 `deepseek-v4-flash-0731`（同批探测 200）。**所以本节结论属于 `deepseek-v4-flash-0731`，不是§35.1 那个模型。**

**请求侧（准备阶段第一轮真实提交，抓包第 4 条，10,821 字节）。** 顶层字段恰好是 `messages, model, parallel_tool_calls, stream, stream_options, tool_choice, tools`，**没有 `response_format`**；`tool_choice: auto`、`parallel_tool_calls: false`；`messages` 6 条（system、user，然后是两轮 `ask_user` 的调用/结果对）。`tools` 恰好两项，`required` 数组原文为：

```
ask_user            required = ["questions"]
submit_preparation  required = ["artifacts", "chapter_plan"]
```

`submit_preparation` 的 schema 把 `chapter_plan` 定为**顶层**属性：`type: array`、`minItems: 1`、每项 `required: ["title", "goal"]`、`additionalProperties: false`；`required` 是 `["artifacts", "chapter_plan"]`。system 末尾的阶段提交契约原文：

> 阶段提交：本轮结果只能通过 `submit_preparation` 工具提交，不要在普通文本里输出 JSON，也不要在工具调用之外附加说明。
> 工具参数的 `artifacts` 必须恰好包含 background、research、characters、outline 各一次（本轮只需提交这些产物），每项 content 不能为空。
> `chapter_plan` 是本轮章节清单的唯一承载字段，至少一项；只把章节清单写进粗纲的 content 不算提交，它必须放在 chapter_plan 里，且标题、顺序与目标同粗纲一致。
> 缺少必填字段或字段非法时整轮提交会被拒绝、不会保存任何产物：工具结果里会给出具体原因，请在同一次调用里补齐后重新提交。

**模型侧（同一次响应，抓包第 4 条）。** `finish_reason = "tool_calls"`，工具名 `submit_preparation`，调用 id `chatcmpl-tool-a4d0f3c5414c4cb0`；`arguments` **14,342 字节**，是合法 JSON object，顶层键恰好 `["artifacts", "chapter_plan"]`。`chapter_plan` 是顶层数组，**3 项**，与粗纲一致：

| 序 | 标题 | goal（截断） |
| --- | --- | --- |
| 1 | 雨夜·来客 | 建立雨夜小站的值班环境与周辰的孤独处境；周辰在广播室发现预言列车的匿名信… |
| 2 | 隐患·回溯 | 通过行车日志与监控碎片展开三年前广播失误致林婉事故的回忆，周辰核查预告条件… |
| 3 | 零点·广播 | 揭示匿名信作者为林婉之姐林清、预告本为试探而非真实事故；周辰冲上站台公开道歉… |

`artifacts` 四项齐全（background 1055 字符 / research 643 / characters 1578 / outline 1381）。§35.1 里「清单只进 outline 正文、顶层没有 `chapter_plan`」的形态没有出现。

**账本侧（应用自己的库）。** 准备 Run `e6017282-00ba-45ea-b3ec-36a28e440e1f` 的 `agent_messages` 里，序列 7 是 `assistant_tool_calls`（`submit_preparation`，同一调用 id、同一份参数），序列 8 是 `tool_call_id = chatcmpl-tool-a4d0f3c5414c4cb0` 的 tool 消息，内容 `{"code":"PHASE_SUBMISSION_ACCEPTED","status":"applied"}`——**提交调用与它的结果在真实账本里配对闭合**。四个产物落库为 rev 1；`outline` 的 `chapterPlan.items` 是 3 项、`orderIndex` 1/2/3；`GET /api/versions/{id}/chapters` 给出 3 章，标题与顺序逐字等于上面的 `chapter_plan`（`雨夜·来客` / `隐患·回溯` / `零点·广播`）。**章节来源可以钉死**：`commit_preparation_model_submission`（`model_requests.rs:606`）只接受 `submission.chapter_plan`，没有任何一处从粗纲正文里解析章节。四项审核全部由审核 Agent 判过（`decision_source = agent`，confidence 0.92 / 0.85 / 0.82 / 0.85）。

**终态（应用 API）。** 工作流 `preparing_generating/queued` →（两轮 `ask_user`）→ `preparing_reviewing/waiting_review` → **`writing_chapter_ready/idle`**（stateRevision 13）；准备 Run `status = succeeded`、`request_count = 3`、`consecutive_protocol_errors = 0`。9 条抓包请求全部 200，全程没有 429、没有 provider 错误（超时/连接失败一类都没有）；唯一的失败是下面那条审核协议错误。

**一句诚实话：准备阶段这次端到端跑通了**——四项产物提交、落库、全部通过审核，工作流自己推进到了正文阶段。

**同一轮里真实发生的审核 lane 缺陷（§35.2 上报第 1 条的第一个真实样本）。** 粗纲审核的第一次委派（review Run `04fd5ab6-22c6-4f12-907d-7f5660d6de00`）以 `failed` 结束，错误原文是 `{"kind":"protocol_error","message":"模型对提交工具 submit_review 的调用无效：提交工具的参数不是当前阶段要求的 JSON：invalid type: string \"[…\", expected a sequence"}`——模型把 `issues` 写成了**装着数组的 JSON 字符串**（`"issues":"[\"【建议】…\"]"`），内容对、形状错，正是本轮修的那一类失败在审核阶段的翻版。它的账本 `agent_messages` 只有 system / user / assistant 三条，**没有任何配对工具结果**：审核 lane 确实没有纠正回路。这一轮能继续，是因为驱动脚本在下一轮轮询里发现该审核又变回 `open` 并**第二次委派**（Run `914bbbe6-c5ae-47b8-9769-a37bee05a621`，同一次提交通过、confidence 0.85）；换成界面，用户得自己点重试。这条与 §35.2 的判断完全一致，也说明修它是下一轮值得做的事，而不是本轮顺手改的东西。

**本节证明了什么、没证明什么。**

- **在要求的路径上（应用自己的界面/API → 调度器 → 适配器 → 真实端点）新取得的证据**：顶层 `chapter_plan` 在真实提交里存在且被应用消费；提交调用与配对结果在真实账本里闭合；准备阶段端到端完成；14,342 字节的工具参数完整送达并解码（这同时替代了 §35.1 里 400/2000/6000 字符的受控探测——量级大得多，而且是应用自己发的请求）。
- **仍然只有账本级测试、尚未取得真实端点证据**：§35.2 修的那条路径——「参数被截断/不可解码时按被拒提交处理」——本轮**没有复现**。§35.1 的 >134 秒零字节静默与 1,889 字符截断都没再出现：本轮准备阶段请求的最长空档是 32.4 秒（上限 90 秒），参数完整合法。所以那条修复目前只由 `truncated_submission_arguments_are_rejected_instead_of_failing_the_run` 与 runner/provider 单测证明，真实端点上的证据仍是空白。
- **未参与本轮的对象**：`deepseek-v4.1-flash` 因端点容量全程 429，本轮没有任何一次它发出的响应，因此「原模型现在会把清单放进 `chapter_plan`」这一句在本节只对 `deepseek-v4-flash-0731` 成立；`docs/01` §8 的退出标准（完整跑通准备阶段）按本节结论对替换后的模型成立，对原模型未验证。

### 35.4 审核 lane 的提交纠正回路（样本取自 §35.3）

**真实样本。** §35.3 那轮里，粗纲审核的第一次委派（Run `04fd5ab6-22c6-4f12-907d-7f5660d6de00`）以 `failed` 收场，错误原文是 `{"kind":"protocol_error","message":"模型对提交工具 submit_review 的调用无效：提交工具的参数不是当前阶段要求的 JSON：invalid type: string \"[…\", expected a sequence"}`——模型把 `issues` 写成了装着数组的 JSON 字符串（`"issues":"[\"【建议】…\"]"`），内容对、形状错。它的 `agent_messages` 只有 system / user / assistant 三条，**没有任何配对工具结果、也没有纠正消息**；Review 直接回到 `open`，界面里用户必须再点一次「委托审核」（§35.3 是脚本第二次委派 `914bbbe6-c5ae-47b8-9769-a37bee05a621` 才通过）。写作三条 lane 从 §35.2 起对同一种输入走「配对错误工具结果 + 纠正 + 计数」，审核 lane 是唯一例外。

**改动。**

- `nekowriter-agent`：`deserialize_review_issues` 单独接管 `RawReviewSubmission.issues`，认出「装着数组的 JSON 字符串」并给出点名原因 `审核 issues 必须是字符串数组，不能是装着数组的 JSON 字符串；请把数组直接放在 issues 字段上`；字符串、非字符串项各有一句（`收到数组` / `收到字符串` / `收到布尔值` 等）。这句话原样进入配对的错误工具结果与纠正消息。
- `nekowriter-app`：`handle_review_completion` 里 `classify_model_completion` 的 `Err` 分支不再直接收束本次委托，改走调度器唯一的 `record_protocol_correction`，以 `ProtocolCorrectionTarget::DelegatedReview { review_id }` 指明这次纠正记在审核子 Run 头上，与写作 lane 共用 `protocol_correction` 的原因码与文案（`PHASE_SUBMISSION_INVALID` / `TOOL_ARGUMENTS_INVALID` / `PARALLEL_TOOL_CALLS_NOT_ALLOWED` / `PROTOCOL_ERROR`）；`requeue` 为真时 lane 重发一次审核请求（同一个 `LaneCommand::Start` 入口，与写作 lane 一致）。
- `nekowriter-store`：`record_model_protocol_correction` 接受 `ProtocolCorrectionTarget`，在目标 Run（写作 Run 或**审核子 Run 自己的**）账本里追加「被拒调用 + 配对错误结果 + 纠正」，`consecutive_protocol_errors` 沿用同一个 `MAX_CONSECUTIVE_PROTOCOL_ERRORS`；审核 target 达上限时按原行为收束（子 Run `failed`、`reopen_delegated_review`）。重排请求的历史只由该子 Run 的账本渲染，所以被拒调用与它的结果始终成对，`serialize_request_messages` 不会看到未配对的 assistant 工具调用（审核子 Run 本就不重放父 Run 历史）。§35.5 把这条路径收拢到 `store::protocol_correction` 一个模块。
- `nekowriter-store`（可见性，本轮补上）：被拒的那一轮在同事务里写两条事件——`run.changed`（`kind = review`、`status`、`request_id`、`request_count`、`consecutive_protocol_errors`、达到上限时的 `wait_reason = protocol_error_limit` ＋ `protocol_correction = true`）与 `review.changed`（`status = delegated`、`attempt_id`、`review_id`、`request_id`、`consecutive_protocol_errors` ＋ `protocol_correction = true`），字段与写作 lane 的纠正事件一致。`consecutive_protocol_errors` 只通过事件暴露给客户端（快照与 Dashboard 的 DTO 都不含该字段），没有这两条事件，这次纠正对外就是完全不可见的——审核停在 `delegated`、随后突然重新开放，正是原始报告结尾那句抱怨。同时子 Run 的 `request_allowance_end` 从 1 改为 `MAX_CONSECUTIVE_PROTOCOL_ERRORS`（3）：被拒后必须能重提，额度要容得下重提轮数，否则「可纠正的提交」会在重排时撞上额度耗尽，反而变成不可纠正的失败（docs/02 §2.3 已写明这条例外）。

**测试证据（本节的用例都实测「改前失败、改后通过」，包括本轮补上的事件断言）。**

- `scheduler::tests::rejected_review_submissions_are_paired_and_retried_up_to_the_limit`：同一份 JSON 字符串 `issues` 连发三次——三条请求都 `succeeded` 且 `error.kind = protocol_error`，每次都能按 `tool_call_id` 找到配对的错误工具结果（含 `PHASE_SUBMISSION_INVALID`、`issues`、「不能是装着数组的 JSON 字符串」），末尾一条 `protocol_correction` 点名 `submit_review`；`requests.len() == 3` 之后 Review 重新 `open`。第二次请求原文里带着 `review-call-1`、`PHASE_SUBMISSION_INVALID` 与「未通过本阶段校验」，且 user/tool 消息计数为 2/1——重提确实带着自己的错误历史，初始输入没有重复堆叠。**改前**以 `被拒的审核提交必须重提: Elapsed(())` 失败（旧分支直接收束为 `failed`，不再排队）。
- `scheduler::tests::corrected_review_resubmission_commits_the_decision`：第一次被拒、第二次合法提交真的落成 `approved`（`passed = Some(true)`），整次委托不作废。**改前**以 `合法重提必须应用审核决定: Elapsed(())` 失败。
- `nekowriter-agent::runner::tests::review_issues_as_a_json_string_names_the_field_and_the_expected_shape`：解析器对同一形状给出点名原因，真正的数组仍然通过。**改前**失败（回落到泛泛的类型错误）。
- 同一个用例还断言事件：三轮纠正各写一条 `run.changed`（`protocol_correction = true`，`consecutive_protocol_errors` 依次 1/2/3，最后一轮 `wait_reason = protocol_error_limit`，前两轮为 null）与一条 `review.changed`（`status = delegated`、`attempt_id` 为子 Run、“`consecutive_protocol_errors` = 1”），随后仍是原来的「委派失败 + Review 重新开放」两条事件。**改前**（只回退事件写入）以 `left: 0, right: 3` 失败。
- `scheduler::tests::truncated_review_arguments_are_corrected_then_resubmitted`：把 §35.1 那种被端点截断的参数（`{"passed": true, "confidence": 0.82, "issues": ["【建议】第三章`）交给审核 lane——账本里逐字保留模型原文、回执含 `PHASE_SUBMISSION_INVALID` 与「不是合法 JSON」、请求终态是 `protocol_error` 而不是 `provider_error`，第二轮合法重提真的落成 `approved`（子 Run 只发两轮请求）。**改前**：事件断言失败（回退事件写入后同一位置失败）。

**排查：其它仍会致死的 `submit_review` 形状。** 审核请求只声明 `submit_review`（`delegated_review_lane_declares_only_the_submission_tool` 断言请求里恰好只有它），所以 `classify_model_completion` 对审核阶段只可能返回 `Submission` 或 `Err`：任何非 `submit_review` 的调用名都命中 `DisallowedToolCall` / `UndeclaredToolCall`，同样走纠正。`handle_review_completion` 里 `Ok(outcome)` 的 `unexpected_outcome` 分支因此在当前声明下不可达（保留为防御）。仍然致死的两条路径都不是「提交形状」，按要求保持原行为：`Err(provider_error)`（传输失败）与 `submission_commit`（提交合法但写库事务失败，例如状态冲突或目标已变）。

**未证明的部分。** 这条回路目前只有调度器/账本级测试背书：本轮按约定没有再用真实端点复跑，因此「真实端点上下一次委派能靠纠正自愈」仍未在端点上验证；§35.1 的 >134 秒静默与本节的 1,889 字符截断仍然是端点行为，不是应用能修的。§35.3 的环境事实（写作模型替换为 `deepseek-v4-flash-0731`、`deepseek-v4.1-flash` 全程 429）也不因此改变。截断参数（写作 lane 与审核 lane）这一形状同样仍未在真实端点复现过，两边的证据都来自假服务。另外这两条事件只是把纠正写进了事件流：工作台目前不渲染 `consecutive_protocol_errors`（没有任何 DTO 携带它，唯一消费者是事件订阅方），所以界面上仍看不到「正在第几次纠正」这样的进度，那是另一次界面工作。

### 35.5 协议纠正收拢到单一模块（架构轮 + 真实二进制复跑）

**背景。** §35.2 与 §35.4 给两条 lane 补上了同一种行为，但写成了两份事务：写作 Run 的 `Store::record_model_protocol_correction`（当时在 `model_requests.rs`）与审核子 Run 的 `Store::record_delegated_review_protocol_correction`（在 `preparation.rs`，连同 `apply_delegated_review_protocol_correction` / `settle_delegated_review_protocol_correction` / `review_correction_run_event` / `review_correction_review_event`），加上调度器里两个几乎逐行相同的 wrapper。同一条策略（配对消息、计数、事件、达上限收束）有两个所有者，任何一处改动都得改两遍。

**结构（本轮产出，后续按此扩展）。** 新增 `crates/nekowriter-store/src/protocol_correction.rs`，它是「被调度器判为不合格的模型回合」的唯一入口（§35.6 修正了本节原来的说法：`preparation.rs` 还有两处不由调度器判定的拒绝，见那里）：

- `record_model_protocol_correction(target, correction)` 是唯一的事务入口；请求 id 从 `correction.assistant_message.request_id` 推出，不再由调用方另传（少一个能写错的地方）。
- `ProtocolCorrectionTarget`（`WritingRun` / `DelegatedReview { review_id }`）是两条 lane 唯一的差别开关，分派到 `settle_writing_run`（工作流继续/停止 + `run.changed`/`workflow.changed`）或 `settle_delegated_review`（`run.changed`/`review.changed` + 子 Run 失败/重开 Review）。`RejectedRound` 把两边都要用的那一轮事实只组装一次。
- `ModelProtocolCorrection`、`ProtocolCorrectionOutcome` 随该模块公开；`ModelProtocolCorrectionResult`、`DelegatedReviewProtocolCorrectionResult` 删除——两条 lane 返回同一个结果类型。
- 留在原处的是「因为一个回合怎么被拒」与「写给模型的消息长什么样」：阶段守卫在 `agent::runner`，纠正文案与原因码在调度器的 `protocol_correction`，消息构造 `protocol_correction_messages` / `validate_model_protocol_correction` / `protocol_error_tool_result_messages` 留在 `model_requests` 并改为 `pub(crate)`；`preparation` 只对 sibling 暴露收束所需的 `delegated_review_attempt`（字段对 crate 内可见）、`fail_delegated_review_child_run`、`reopen_delegated_review`。
- 调度器侧两个 wrapper 合成一个 `record_protocol_correction(target, …)`，日志用 `lane = ?target` 区分。

删掉的代码：`record_delegated_review_protocol_correction`、`apply_delegated_review_protocol_correction`、`settle_delegated_review_protocol_correction`、`review_correction_run_event`、`review_correction_review_event`、`DelegatedReviewProtocolCorrectionResult`、`ModelProtocolCorrectionResult` 与调度器的第二个 wrapper。这是形状改动，行为不变：既有用例（含 §35.2 / §35.4 的两条 ledger 级用例）一字未改地通过，正是这一步要的证词。

**真实二进制复跑（本轮为此做的「真实运行」）。** 上一次真实端点复跑（§35.3）脚本：应用 `target/debug/nekowriter`（当前源码，`cargo build -p nekowriter-app`）监听 127.0.0.1:8802、全新数据目录 `/tmp/nw-refactor/data`，用 `launchctl submit` 拉起；模型侧换成脚本化假端点 `/tmp/nw-refactor/fake_model.py`（`launchctl submit -l com.nekowriter.fakemodel`，监听 127.0.0.1:8797/v1，实现 OpenAI 兼容流式子集）。驱动 `/tmp/nw-refactor/drive2.py` 只用应用自己的 HTTP API（settings → test-model → projects → brief → preparation/start → 逐个委派四项产物审核）。假端点按剧本制造两种被拒形状：第 1 次 `submit_preparation` 不带顶层 `chapter_plan`，第 1 次 `submit_review` 把 `issues` 写成装着数组的 JSON 字符串；之后都发合法提交。

结果（全部取自这次运行自己的 SQLite 账本）：

- 准备 Run `dc96883b-c4bc-4496-bf15-8945f5680d33`：`request_count = 2`、终态 `succeeded`、`consecutive_protocol_errors = 0`（成功提交后清零）。消息序列：3 = `submit_preparation` 调用（`artifacts` 齐全、无 `chapter_plan`）；4 = `tool_call_id = fake-prep-1` 的配对工具结果，内容 `PHASE_SUBMISSION_INVALID` + `提交粗纲时必须同时提交至少一项 chapter_plan`；5 = `protocol_correction` 用户消息点名 `submit_preparation`；6 = 模型重提；7 = `PHASE_SUBMISSION_ACCEPTED` 的配对结果。
- 第一份审核（子 Run `802e346c-8392-4551-82c9-62fa588a34c3`）：第 3 条调用 `issues` 是字符串 `"[\"【建议】内容可以更扎实\"]"`，第 4 条回执点名「审核 issues 必须是字符串数组，不能是装着数组的 JSON 字符串」，第 5 条纠正、第 6 条合法重提、第 7 条接受。子 Run `request_count = 2`、`consecutive_protocol_errors = 1`、终态 `succeeded`——这次委派在**没有人工重试**的情况下自愈（§35.4 里真实端点靠脚本第二次委派才过）。
- 事件流里两种 lane 的纠正信号同形：`workflow.changed`（seq 9，`protocol_correction = true`、`consecutive_protocol_errors = "1"`）与审核侧的 `run.changed`（seq 21，`kind = review`、`wait_reason = null`）+ `review.changed`（seq 22，`status = delegated`）。
- 提交真的落了库：3 章标题与 `chapter_plan` 逐字相同（雨夜·来客 / 隐患·回溯 / 零点·广播），7 条 `model_requests` 全部 `succeeded`，两条被拒回合的 `error_json` 是本地协议原因（`kind = protocol_error`）；工作流从 `preparing_generating/queued` 走到 `writing_chapter_ready/idle`。

**未证明的部分。** 这一轮用的是脚本化假端点，它证明的是应用自己这条链（二进制 / HTTP / SSE 解析 / SQLite 事务 / 事件流）在重构后仍然闭合，**不**证明端点行为：§35.1 的 >134 秒静默与 1,889 字符截断仍然只在 §35.3 的抓包里出现过，§35.2 / §35.4 那两条修复依然没有真实端点上的证据。工作台不渲染 `consecutive_protocol_errors` 这一点也不变（§35.4 末段）。假端点与驱动脚本在 `/tmp/nw-refactor/`，是临时装置，不入库。

### 35.6 被拒回合的计数必须与它旁边的事件一致（交接前审计发现）

**缺陷。** `protocol_correction.rs` 的 `settle_writing_run` 总是落库 `next_error_count`，而 `settle_delegated_review` 只在未达上限的分支里落库：达上限那一轮只把子 Run 标成 `failed`（`fail_delegated_review_child_run` 当时只写状态、结束时间与 `updated_at`）。后果是一次耗尽限额的审核委托（三轮被拒）在 `runs.consecutive_protocol_errors` 里停在 **2**，而它刚写的 `run.changed` / `review.changed` 两条事件都说 **3**——落库状态与它自己发出的事件矛盾，而避免这类矛盾正是这个模块存在的理由。发现方式：交接前审计用脚本化假端点把审核 lane 逼到上限（三个被拒轮），对比 `runs` 与 `events` 两张表时看到 `2 vs 3`，连续五次委派都是这个数字。

**修复。** 子 Run 的行每轮只写一次，两条分支都带上这一轮的计数：

- `settle_delegated_review` 先在一个 `ActiveModel` 上写 `consecutive_protocol_errors` 与 `updated_at`，达上限时再用 `fail_delegated_review_child_run` 补上失败状态并同一次 `update` 落库；重开 Review 仍在同一事务里跟在后面。计数与事件来自同一个 `round`，两者不可能再分叉。
- `fail_delegated_review_child_run` 从「自己开事务写四列」改成纯字段函数（`run::ActiveModel -> run::ActiveModel`）：它只定义「失败长什么样」，本轮自己的字段由调用方在同一次写入里补齐。取消/失败尝试那条路径继续用它，行为不变。

**同类扫描（本模块内）。** 逐字段检查了被拒回合里会写库、且旁边就写着事件的字段：

- `consecutive_protocol_errors`：上述缺陷，已修。
- `status` / `ended_at`：两条 target 都在每个分支里写，事件里的 `status` 取自同一个值（写作 lane 为 `queued` / `stopped`）。
- `request_count`：由派发时写入，结算函数不碰它，事件读的也是派发后的值。
- 请求行的 `status` / `provider_request_id` / `usage_json` / `error_json` / `completed_at`：`apply_rejected_round` 无条件写全部六列，没有分支差异。
- 工作流快照：写作 lane 在同一事务里 `persist_workflow`，审核 lane 不动工作流（它的事件用 `state_revision` 作版本）。

**发现但不改的一处（不同类）。** 审核 lane 的 `run.changed` 纠正事件里的 `status` 取的是**本轮被拒之前**的值（`running`），而达上限那一轮行最终是 `failed`——失败由紧随其后的 `reopen_delegated_review` 两条事件（`run.changed status=failed`、`review.changed status=open`）宣布。这是「事件是本轮快照、状态变更另有归属」，不是「只写一半」：客户端按序应用事件后落在 `failed`，同一事务内自洽。要把它改成后置状态就得改事件字段的值与顺序，属于本次明令禁止的对外可见变更，因此按上报处理、未改。

**计数策略收拢。** `preparation.rs` 里三处内联的 `checked_add(1)` 加溢出文案（正文工具结果路径、`commit_chapter_body_model_submission`、未投递意见分支）改为调用 `store::model_requests::next_protocol_error_count`：同一个判定、同一句错误文案现在只有一份实现。错误码、文案、数字、事件字段与顺序一律未动——`PHASE_SUBMISSION_REJECTED` 与 `PHASE_SUBMISSION_INVALID` 仍是两套词汇，本轮只统一策略。`docs/02` §2.1 已改成真实地图：`store::protocol_correction` 是被调度器判为不合格的回合的唯一入口，`preparation.rs` 另有两处不由调度器判定、不共享词汇但共享计数规则的拒绝。

**测试。** `store::tests::rejected_review_round_persists_the_counter_it_publishes`（新增）把审核 lane 逼到上限：每一轮都断言 `runs.consecutive_protocol_errors` 等于该轮事件公布的 `consecutive_protocol_errors`，达上限那轮再断言行是 `failed`、计数为 3、Review 重新开放。**改前**以 `left: 2, right: 3`（第 3 轮）失败。写作 lane 一侧的同一不变量由 `store::tests::protocol_errors_requeue_with_paired_results_then_stop_at_the_limit` 逐轮断言——它在改前就通过，是守护而不是回归证据（写作 lane 本来就总是落库计数）。

**同一装置复跑（修复后）。** 交接前审计发现这个缺陷用的那个假端点（`/tmp/nw-refactor/fake_invalid.py`：审核每轮都发「装着数组的 JSON 字符串」）再跑一次完整准备阶段：两份被逼到上限的审核子 Run 都是 `request_count = 3`、`consecutive_protocol_errors = 3`、`failed`，与它们最后一轮事件里公布的 `3` 一致——同一探针在修复前读到的是 `2`。

**验证。** `cargo test --workspace` 212 通过（agent 43、app 52、core 38、store 75、xtask 4）；`cargo fmt --all -- --check` 干净；`cargo clippy --workspace --all-targets` 在本次改动上无新增告警。全部命令带 `--config 'env.PROTOC.value="/opt/homebrew/bin/protoc"'`（工作区 `.cargo/config.toml` 把 `PROTOC` 钉在一个本 checkout 不存在的 Windows 路径上）。

**未证明的部分。** 本次没有重跑真实端点：缺陷与修复由 store 级测试和交接前审计的假端点复跑证实，端点行为（§35.1 的 >134 秒静默、1,889 字符截断）依旧只有 §35.3 的抓包。`preparation.rs` 那两处拒绝路径仍未接进 `protocol_correction.rs` 的事务（本轮只统一了计数规则与文档地图）；它们的计数与事件是否逐轮一致，目前没有任何测试按 §35.6 的不变量去钉。工作台仍不渲染 `consecutive_protocol_errors`（§35.4 末段）：这个数字现在保证与落库一致，但界面依旧没有显示它的地方。

## 36. 2026-09-14 真实外部端点实战复跑：被业务拒绝的章节提交不写配对工具结果，正文 Run 从此发不出请求

**装置。** 一次「新建作品 → 准备 → 审核 → 逐章细纲/正文」的完整业务走查，**只走应用自己的界面与 HTTP 接口**（由 ego-browser 驱动 webui，没有调内部函数、没有直接写库），模型是外部真实端点：

- 应用：当前源码的 debug 构建 `target/debug/nekowriter`，用 `launchctl submit` 拉起（命令组会被回收，否则跨调用不存活），`--data-dir /tmp/nw-story-run/data --port 8811 --web-dir webui/dist --no-browser`；`GET /api/health` 返回 `schemaVersion=25`。
- 系统设置：先指 `https://model.inferx.net/endpoints/v1` + `deepseek-v4.1-flash`，实战中途改指 `https://api.scnet.cn/api/llm/v1`（密钥重新配置，「测试真实连接」返回「已收到模型响应；输入 28，输出 22」）。
- 业务目标：三章短篇（`#/works` → 新建作品 → 工作台 → 设定作品目标 → 开始准备 → 四项产物人工审核 → 第一章指导 → 开始细纲 → 细纲审核 → 正文）。

**跑到正文阶段为止是通的。** 准备阶段在真实端点上一次生成成功：背景、资料、人物、粗纲四项与三章 `chapter_plan` 同批提交并落库，四项人工审核（均以空意见「通过」）后阶段折叠与目录冻结正常；第一章细纲一次生成、审核通过、`copiedContextFolds` 折叠正常，工作流自动进入 `writing.drafting`。也就是说，本次实战卡住的不是准备/审核/细纲链路，而是正文 lane 的一处状态收口。

**缺陷。** 第一章正文 Run（`018a644d-ef7b-40df-bd1e-756cc66b4daa`，chapter `cebd70b3-fdd4-4f47-9209-155d3e68243f`）永久卡死，根因是**被业务拒绝的阶段提交只落了 assistant 消息，没有落下配对的工具结果**：

1. 第 17 次模型请求成功返回，但模型把整章写成**普通文本**（`agent_messages.sequence=3`，2677 字符，内容确实是一章正文）而没有调用 `append_content`；服务端按协议错误纠正，写入 `sequence=4` 的纠正消息。
2. 第 18 次请求里模型调用 `submit_chapter(chapter_id=…, summary=…)`（provider call id `call_2fd37d08dace4a29b43d05a5`，即 `agent_messages.sequence=5`）。此时章节正文为空（该 Run 从没有一次成功的正文编辑），`commit_chapter_body_model_submission` 返回 `StoreError::InvalidChapterBodyEdit("章节正文不能为空")`（`crates/nekowriter-store/src/preparation.rs:6405`），scheduler 走 `persist_model_failure(…, "submission_commit", …, unknown=false)`（`crates/nekowriter-app/src/scheduler.rs:1875` 一带）。
3. `persist_model_failure` 只调 `resolve_model_request_with_assistant_message`，于是这一轮**只多了一条 assistant 消息**（`kind=assistant_tool_calls`），**没有任何 `role=tool` 的结果消息**：`select count(*) from agent_messages where run_id=… and role='tool' and content_json like '%call_2fd37d08dace4a29b43d05a5%'` 为 **0**；`tool_calls` 表里这个 Run 同样是 **0 行**。
4. 之后每一次「显式继续」（界面点击与直接 `POST /api/runs/{id}/continue` 都试过；后者返回 200 并把执行轴置 `queued`，`resume_count` 从 16 一路涨到 20）都在派发前失败，请求一次也发不出去：

```text
WARN 无法启动模型请求 version_id=3ee73c0d-e367-4695-8d8c-0049688b4603 run_id=018a644d-ef7b-40df-bd1e-756cc66b4daa
  kind="chapter_body"
  error=模型上下文构建失败：当前消息不符合模型协议：工具调用 call_2fd37d08dace4a29b43d05a5 缺少紧随其后的工具结果
```

事件侧留下的痕迹是同一件事的两个快照：`run.changed`（`error.kind=pre_dispatch`，`revision=82`）与紧随其后的 `workflow.changed execution=failed`（`events.seq=178/179`）。Run 行停在 `status=failed, request_count=18, resume_count=20, consecutive_protocol_errors=0`，`model_requests` 不再新增任何 attempt——`resume_count` 增长而 `request_count` 不动，就是「受理了继续、但请求根本没发出去」。

**为什么这是缺陷。** 模型发一条被业务拒绝的提交在规范内（提交守卫本来就该拒绝），规范同时要求这份账本始终可重放：docs/02 §4.3 写明「账本因此始终保存成对的 assistant 工具调用与工具结果，重放时 `serialize_request_messages` 的配对校验不会失败」。构造配对结果的机制在同一仓库里已经存在——`crates/nekowriter-store/src/model_requests.rs:1379` 的 `protocol_error_tool_result_messages` 专门为「未被实际执行的 assistant tool call」生成错误工具结果，协议纠正 lane 正是用它（`model_requests.rs:1318`）——但 `submission_commit` 这条 lane 没有走它。后果不是「这一轮被拒」，而是**这个 Run 连同它已经花掉的 18 次请求一起报废**：界面只提供「显式继续」，点多少次都只会重复同一条上下文错误，用户没有任何界面出口（`body/replace` + `body/review` 的人工正文路径仍在，但那是绕过 Agent 的另一条路，不是 run 的恢复）。

**修复方向（本次未实施，按用户要求只记录）。** 对模型而言，「被拒的阶段提交」与「协议纠正」是同一件事：这一轮不推进，模型要在剩余额度内重提。把 `submission_commit` 的 assistant 消息也补上配对工具结果（复用 `protocol_error_tool_result_messages` 的形状），并按 §35.6 的不变量让计数与事件同一次落库，即可把「这一轮被拒」与「这个 Run 报废」分开。落地前必须先钉住语义：补配对结果之后这次提交仍然是**拒绝**——不得被当作成功提交写入正文、也不得进入整章审核。

**另一处观察（不同类，未定论）。** 第 15、16 次请求（`effectiveModelId = DeepSeek-V4-Flash-0731`）都以 `provider_error: 模型响应超过 2097152 字节上限` 结束，两次稳定复现；换成 `DeepSeek-V4-Pro` 后同一条请求不再撞上限（转向了上面的提交语义问题）。实测该端点约 220 字节 SSE/token（单次 174 token 的流为 38,844 字节），2 MiB 上限大约允许 9.5k token；而应用不发送 `max_tokens`，重推理模型在这条正文请求上很容易把流推到上限。这属于「工程上限与模型选择」，本次只记录现象，未动任何上限。

**现场保留与复现。** 应用仍在运行（`launchctl list com.nekowriter.storyrun`，`127.0.0.1:8811`，数据目录 `/tmp/nw-story-run/data`），稿本 `3ee73c0d-e367-4695-8d8c-0049688b4603`／第一章 `cebd70b3-fdd4-4f47-9209-155d3e68243f`／Run `018a644d-ef7b-40df-bd1e-756cc66b4daa` 都未被清理，界面停在 `writing.drafting` + `failed`。复现只要让模型在正文阶段先「只回普通文本、不调用正文工具」，再让它调用 `submit_chapter` 而正文仍为空；此后每次 continue 都会命中上面那条上下文配对错误。本次未修：用户明确要求先记录缺陷、暂停实战。

### 36.1 修复：被业务拒绝的阶段提交按「被拒回合」处理（同日随后执行）

**已确认的语义。** 被业务守卫拒绝的阶段提交是一条「被拒回合」：立即退回模型重提，不推进 `consecutive_protocol_errors`，也绝不写入正文或进入整章审核。非进展回合的工程上限由请求额度承担，不由协议错误计数承担。覆盖 `submit_preparation`、`submit_chapter_outline`、`submit_chapter`、`submit_review` 四条阶段提交路径。

**改动。**

| 位置 | 变化 |
| --- | --- |
| `nekowriter-store/src/protocol_correction.rs` | 模块改为「被拒回合的唯一归属地」：`ModelProtocolCorrection`/`ProtocolCorrectionOutcome` 更名为 `RejectedRoundRecord`/`RejectedRoundOutcome`，新增私有 `RejectedRoundCounting`，单一事务入口 `record_rejected_round` 配上两个公开入口——`record_model_protocol_correction`（计数加一、可触发上限收束）与 `record_phase_submission_rejection`（计数不变、永不触发上限）。事件构造集中到 `writing_run_event`/`writing_workflow_event`/`review_correction_run_event`/`review_correction_review_event`：协议错误公布计数与 `wait_reason`，被拒提交公布 `reason = submission_rejected` 且两者都不出现 |
| `nekowriter-store/src/model_requests.rs` | `protocol_correction_messages`/`validate_model_protocol_correction`/`protocol_correction_matches` 更名为 `rejected_round_messages`/`validate_rejected_round`/`rejected_round_matches`；终态写入改为 `terminal_request_messages`，为响应里尚未被执行过的工具调用补写配对错误结果（已存在的按 `tool_call_id` 跳过，因此成功路径先追加真实结果的顺序不受影响），理由码 `TOOL_NOT_EXECUTED` 并带上本轮结局 |
| `nekowriter-app/src/scheduler.rs` | 四条阶段提交的 `Err` 分支从 `persist_model_failure`/`persist_delegated_review_failure` 改走新的 `record_phase_submission_rejection`；新增 `phase_submission_rejection` 生成点名提交工具与真实原因的纠正提示（理由码 `PHASE_SUBMISSION_REJECTED`／`INVALID_REVIEW_RESULT`） |

**被拒的提交不得被当成成功。** 新入口只在「提交已经被业务守卫拒绝」处调用，写入的永远是错误工具结果与纠正提示；正文、产物、审核决定一律不变。契约由测试钉住：被拒提交之后 `body.content` 仍为 `None`、`body.review` 仍为 `None`。

**契约与测试。** 新增五个用例（store 四个、app 一个），并把审核 lane 两个用例的公共起点抽成 `delegated_background_review`：

| 用例 | 断言 |
| --- | --- |
| `business_rejected_submission_pairs_the_result_the_next_request_needs`（store） | 账本只有三条且形状正确（assistant 调用 → 同 `tool_call_id` 的 `tool_result` → `protocol_correction`），结果为 `PHASE_SUBMISSION_REJECTED` 并点名 `submit_preparation`；计数仍为 0、请求以 `succeeded` 收束、事件带 `reason = submission_rejected` 且不含计数与 `wait_reason` |
| `business_rejected_submissions_never_stop_at_the_protocol_limit`（store） | 连续 4 轮（超过上限 3）被拒后工作流仍是 `queued`、`wait_reason` 为 `null`、计数仍为 0，账本每轮各三条完整配对 |
| `business_rejected_review_submission_keeps_the_delegation_running`（store） | 子 Run 的生命周期状态与 `ended_at` 不变（不为 `failed`）、计数为 0、Review 仍为 `delegated`、配对结果理由码为 `INVALID_REVIEW_RESULT` |
| `terminal_failure_pairs_tool_calls_that_never_ran`（store） | 请求以 `failed` 收束时，响应里未被执行的工具调用也拿到配对结果（`TOOL_NOT_EXECUTED` ＋ 本轮失败原因），计数仍为 0 |
| `business_rejected_chapter_submission_leaves_a_buildable_history`（app） | 第一次拒绝即重排；账本还原出 assistant 调用 + 配对结果，且 `build_context_with_history` 能成功组装下一次请求——即 §36 里每次 continue 都失败的那一步 |

**验证。** `cargo test --workspace` 221 通过、0 失败（agent 43 / app 56 / core 38 / store 80 / xtask 4）；`cargo fmt --all -- --check` 干净；`cargo clippy --workspace --all-targets` 在改动文件上无新增诊断（既有 pedantic 告警仍在）。

**本轮未做的两件事。**

1. **没有用真实端点复跑。** §36 的现场（应用、数据目录、稿本与 Run）按用户要求原样保留，修复只由自动化用例验证；那条失效的正文 Run 仍停在 `failed`，需要新的真实端点轮次才能把完整业务流程走通。
2. **两处仍按旧语义计的计数没有动。** `submit_chapter` 携带未投递意见时的直接提交（`pending_feedback_requires_tool_result`）仍在 `commit_chapter_body_model_submission` 内部计入 `consecutive_protocol_errors` 并在第三次把 Run 收束为 `stopped`（docs/02 §5.3 明确要求，`direct_chapter_submission_with_pending_feedback_requeues_for_a_tool_result` 覆盖）。它是提交内容被拒、却按协议错误计数的唯一一处，与 §36.1 的新语义不一致；按「只改方案已覆盖的四条路径」的范围没有改动，语义归属需要单独拍定。此外 §36 记录的「单次响应 2 MiB 上限」现象不属于本次范围。

## 37. 2026-09-14 移除 OS 凭据库：模型密钥改存本地 SQLite 的设置行

### 37.1 现场：凭据库读取把整条派发路径挂住

第 3 章正文推进时 Run 回到 `queued`，之后再没有发出任何请求。定位结果不是调度器丢了任务，而是派发路径里的 `read_api_key` 卡在读系统凭据库上：`credentials::read_api_key` 走 `spawn_blocking` 且**没有任何超时**，macOS 首次访问会弹出钥匙串授权，弹窗无人确认时该 future 永远不返回。

同一个原因让 `POST /api/runs/{id}/continue` 直接挂住不返回（实测 `http=000`、20 秒无响应），界面因此把「显式继续」按成永久 disabled（面板的 `pending` 状态等不到 promise 落地）。这条路径上没有任何应用内可自愈的出口：密钥读取不是业务逻辑，却决定了业务能否推进。

### 37.2 处置：删除 keyring 与整个凭据库边界

按用户要求「移除所有 keyring 功能」，应用不再接触任何 OS 凭据库。密钥正文与所属设置同库、同事务保存，只在构建请求前从设置行读出。

| 位置 | 改动 |
| --- | --- |
| `Cargo.toml`、`crates/nekowriter-app/Cargo.toml` | 删除 `keyring` 依赖；app 侧同时删除只用于凭据指纹的 `sha2` |
| `crates/nekowriter-app/src/credentials.rs` | 整文件删除（`OsCredentialStore`、三个异步包装、`CredentialStoreError`） |
| `store/entity/system_model_setting.rs`、`entity/embedding_setting.rs` | 列 `credential_ref` → `api_key` |
| `store/settings.rs` | `SystemModelSettings.credential_ref` → `credential_configured: bool`（对外视图不含密钥）；新增 `Store::system_model_api_key`；`NewSystemModelSettings` 的 `credential_ref` + `credential_fingerprint` 合并为 `api_key: Option<String>`（`None` = 保留原密钥），写入事务内并入既有密钥 |
| `store/knowledge.rs` | `NewEmbeddingSettings` 同上；`embedding_credential_ref()` → `embedding_api_key()`；`embedding_settings_payload` 返回归一化结构体，命令指纹与落库用同一份结果 |
| `store/lib.rs` | `SCHEMA_VERSION` 25 → 26；新增共享的 `normalize_api_key`，两条设置路径共用一套密钥规则（trim、非空、≤ 4096 字符、无控制字符） |
| `app/src/main.rs` | 删除 `ApiError::Credential`、其 `From`、`Display` 与 `into_response` 分支及 `credential_fingerprint`；保存路径由「生成 ref → 写凭据库 → 失败回滚」三步改为一次事务写入；连接测试与命令前置检查改读 `system_model_api_key` |
| `app/src/knowledge.rs`、`app/src/scheduler.rs` | 删除密钥的读包装与 `Credential` 错误变体，改调 store 的读出口；`SchedulerError` / `IndexError` 各去掉一个变体 |

**对外合同不变。** 界面继续提交 `apiKey`、读取 `credentialConfigured`，`webui/` 未改动。

**旧库不迁移。** 列名变化后，旧数据目录按仓库既有约定被拒绝打开（`UnsupportedSchema`），需要新的数据目录；这与 docs/02 §8 的不迁移约定一致。

**不变量已写入 docs/02 §1.2 第 17 条。** 密钥只有一个读出口；对外视图、SSE 事件、命令回执、持久事件、日志与模型请求快照都不携带密钥正文；保存时省略密钥表示保留原值，密钥本身进入命令指纹（只落摘要），因此同一 `command_id` 换密钥重放仍然是冲突而不是幂等命中。

### 37.3 测试与验证

新增三个 store 用例：

- `system_model_api_key_is_kept_when_omitted_and_guarded_when_replayed`：省略密钥的保存推进修订但不动密钥；空密钥被拒；同一命令换密钥重放得到 `IdempotencyConflict`。
- `normalizes_and_rejects_invalid_api_keys`：`None`、需要 trim、空、含换行、超长五类输入的归一化结果。
- `embedding_api_key_is_kept_when_omitted`：Embedding 路径的同一条语义。

既有用例 `system_model_settings_are_revisioned_idempotent_and_emit_sync_events` 同步扩容：断言对外视图序列化后 `credentialConfigured` 为 `true` 且全文不含密钥，密钥只能由 `system_model_api_key` 取得。

`cargo test --workspace` 225 通过、0 失败（agent 43 / app 57 / core 38 / store 83 / xtask 4）；`cargo fmt --all -- --check` 干净；`cargo clippy --workspace --all-targets` 在改动文件上无新增告警类别。

### 37.4 附带发现：`.cargo/config.toml` 的 PROTOC 让非 Windows 的全新构建无法开始

验证上述改动时 `target/` 为空，构建在 `lance-encoding` 的构建脚本处失败：

```
Error: Custom { kind: NotFound, error: "Could not find `protoc`..." }
```

原因不在代码，而在仓库的 `.cargo/config.toml`：它以 `force = true` 把 `PROTOC` 固定为 `tools/protoc/bin/protoc.exe`，那是 `tools/bootstrap-protoc.ps1` 在 Windows 上才会生成的路径；仓库里没有这个文件，而 `force` 让环境变量也无法覆盖。因此任何非 Windows 主机上的干净构建都会停在这里（此前能通过，是因为构建脚本产物已被缓存）。

本轮没有改动这个文件：它属于发行引导，是独立问题。验证时用命令行覆盖绕过：`cargo --config 'env.PROTOC.value="/opt/homebrew/bin/protoc"' …`。正确的修法是把这条 `[env]` 收进 `[target.'cfg(windows)'.env]`，让非 Windows 主机使用系统 `protoc`。

### 37.5 本轮未做的两件事

1. **没有用真实端点复跑。** §36 的现场（应用、数据目录、失效的正文 Run）按用户要求原样保留，本次改动只由自动化用例验证。同日随后执行的真实端点全流程复跑见 §38。
2. **只更新了 docs/02 与 docs/03。** docs/01 与 `NekoWriter.txt` 里仍写着密钥入 OS 凭据库的旧口径，按用户指示未改。（本节之后又更新了界面文案，见 §38.4。）

## 38. 2026-09-14 移除 keyring 后的真实端点全流程复跑：三章短篇完本

### 38.1 环境与配置

数据目录 `/tmp/nw-story-run/data3`（schema 26），端口 8811，二进制由当前源码构建；写作模型 `https://api.scnet.cn/api/llm/v1` 上的 `DeepSeek-V4-Pro`，能力为流式输出、工具调用、流式用量，思考关闭，上下文上限 32768（中途改为 128000）、预留输出 4096。

密钥保存后落在 `system_model_settings.api_key`（46 字符），整个流程期间机器上没有出现过钥匙串授权弹窗（`SecurityAgent` 进程不存在）。

### 38.2 走通的链路

新建作品与稿本 → 保存作品目标 → 准备四项产物与三章计划逐项审核通过 → 第 1 章指导/细纲审核/正文/整章审核/定稿 → 第 2 章（含一次失败与恢复）→ 第 3 章 → 确认完本 → 长篇阅读视图 → 导出 Markdown。

三章正文 2,055 / 3,588 / 2,625 字符（合计 8,268），导出文件 23,594 字节，首行标明「正文 3 / 3 章已通过整章审核」，章标题与正文完整。7 个 Run 全部 `succeeded`（准备 1、细纲 2+2+2、正文 2+5+4 次请求），17 次模型请求，已报告用量为输入 274,662、输出 54,603、推理 22,015，未知用量 0。

### 38.3 三个被这次复跑正面验证的修复

**1. keyring 移除（§37）。** 上一轮同一台机器上，`POST /api/runs/{id}/continue` 会挂住 20 秒（`http=000`）、界面把「显式继续」按成永久 disabled、钥匙串弹窗等人点。这一次点「显式继续」在 3 毫秒内返回，Run 立即回到 `request_in_flight` 并真的发出了请求（`resume_count=1`，`request_count` 从 3 涨到 5）。密钥的写入与读取只经过 SQLite。

**2. 省略密钥不覆盖既有密钥（本次新增的不变量）。** 把上下文上限从 32768 改成 128000 时，设置表单没有重填 API key；保存后 `revision` 从 1 进到 2，`api_key` 仍是原来那把 46 字符的密钥。

**3. 被业务守卫拒绝的阶段提交按「被拒回合」处理（§36.1）。** 第 3 章正文 Run 在 11 秒内同时出现了两种纠正：

```
15:06:30 模型协议错误已保存，已要求同一 Run 纠正 consecutive_protocol_errors=1
15:06:41 阶段提交被业务守卫拒绝，已要求同一 Run 按纠正重提 consecutive_protocol_errors=1
15:07:12 章节正文工具调用已处理 status="applied"
15:07:22 章节正文模型提交已进入整章审核
```

业务拒绝**没有**让计数前进（仍停在协议错误留下的 1，该 Run 行最终 `consecutive_protocol_errors=0`），Run 也没报废：11 秒后工具调用落库、31 秒后进入整章审核。这正是 §36 里「一次被拒就把 Run 永久卡死」的路径。此外第 2、3 章的细纲 Run 各出现一次真正的协议错误（14:50:48、15:04:22），都由同一 Run 纠正后继续并成功提交。

### 38.4 这次复跑暴露的问题

**1. 上下文超限是纯配置，界面没有把它说清。** 第 2 章正文在第 3 次请求时失败：`模型上下文构建失败：上下文预计使用 29039 Token，加上 4096 Token 输出预留后超过上限 32768`。产品行为符合 docs/02（不发出注定被拒的请求），但界面只给出这句话，没有指出「这是窗口大小可以调大」；用户需要自己从错误文本里推断。调大上限后，同一个 Run 用「显式继续」接着写完了该章。

**2. 界面文案在这次改动前是错的（已修复并重建 `webui/dist`）。** 模型设置面板写着「密钥只会交给本地服务，并存入本机系统凭据库；数据库和请求快照不保存密钥」，知识库面板写着「只写入本机凭据库」，项目设置面板写着「服务地址与凭据仍来自系统设置」。三处分别改为「密钥只保存在本机数据库的模型设置里，只在构建请求前读出；请求快照、事件与日志都不写密钥」、「只写入本机数据库」与「服务地址与密钥仍来自系统设置」，`pnpm build` 后由同一个运行中的进程提供。

### 38.5 未覆盖

逐次确认模式下的正文编辑（本次用自动接受）、委托审核、知识库检索、退出与发行打包均未在本轮走通；准备四产物、章节指导、细纲、正文、整章审核、完本与导出都在真实端点下完成。

## 39. 2026-09-14 旧 schema 数据目录被拒绝打开的现场与收口

### 39.1 现场（未改动任何既有数据）

§37 把 `SCHEMA_VERSION` 从 25 提到 26，却没有验过「已有数据目录」这条路径。取证方式：把上一轮实战的数据目录 `/tmp/nw-story-run/data2`（schema 25、6 章、含已定稿正文）整份拷到 `/tmp/nw-schema-probe/data2-copy`，用当前源码构建的二进制指向拷贝启动。

用户真实看到的东西：

- **进程直接退出，退出码 1，服务没有起来**：`/api/health` 与首页都是 `000`（连接不通）。浏览器打不开，没有页面、没有报错文案。
- **stderr 上只有一行英文 Rust 调试输出**：`Error: Store(UnsupportedSchema { found: "25", expected: 26 })`。它给出两个版本号，但没说这是哪个目录、意味着什么、接下来能做什么。
- **数据目录里连日志都没有**：`logs/` 里的文件名仍是上一轮 14:32 的正常退出记录，没有任何新行（`StartupError` 由 `main` 的返回值打印，从未经过 `tracing`）。
- **既有作品完全无法访问**：库里有 6 章，但新版本既不打开也不导出。
- **原目录没有被写坏**：拷贝与原始文件的 SHA-256 完全一致（`a1e37b05907f4444…`，收口后连同原目录一起复核，仍为同一值），数据库字节级不变——拒绝发生在任何写入之前。

在图形界面启动（没有终端）时，这等同于「静默消失」。

### 39.2 为什么保留版本拒绝，而不是让旧目录照常打开

列改名是本次唯一的 schema 变化，「不动数据就能打开」看似可行。实测的反对证据在旧库的设置行里：

```
api_base_url   = https://api.scnet.cn/api/llm/v1
model_id       = DeepSeek-V4-Pro
credential_ref = writing-model-95e31d22-0eec-43bd-b980-e613a261e48e
```

那一列存的是**凭据库里的不透明引用**，不是密钥。沿用旧列名让旧目录打开，这个字符串就会被当成 API key 发往供应商（401），而界面仍显示「凭据已配置」——于是从「打不开」变成「能打开但静默坏掉」。保留版本拒绝、把原因与前进路径说清楚，比静默进入一个坏状态好。真正的迁移被 docs/02 §8.3 与项目约定禁止，本轮也没有新造导入导出。

### 39.3 收口（最小改动）

| 位置 | 改动 |
| --- | --- |
| `crates/nekowriter-app/src/main.rs`（进程锁与 `Store::open` 两处启动步骤） | 失败走统一出口 `startup_failure`：先 `error!` 写进该数据目录的 `logs/`，再向 stderr 打印 `startup_failure_message`，然后原样透传错误（进程仍以非零状态退出） |
| 新增纯函数 `startup_failure_message` | 点名数据目录；`UnsupportedSchema` 说明「由更早的 NekoWriter 创建、on-disk 版本、本版本支持的版本、不会改动也读不了它、要继续已有作品请用旧版本、要用本版本请指定另一个数据目录」；`UnknownSchema` 说明它不是 NekoWriter 的数据；其余错误回落到 Display 文本；末尾给出 `logs/` 位置 |
| 新增单测 `startup_failure_messages_name_the_directory_cause_and_way_forward` | 钉住三个要素：目录、原因、`--data-dir` 前进路径 |

### 39.4 验证（同一份拷贝、同一台机器）

修复后重新指向拷贝启动，用户看到的是：

```
ERROR 启动失败 data_dir=/private/tmp/nw-schema-probe/data2-copy error=无法启动 NekoWriter：数据库 schema 版本为 25，当前程序仅支持 26
NekoWriter 无法使用数据目录：/private/tmp/nw-schema-probe/data2-copy
原因：这个目录由更早的 NekoWriter 创建，数据库 schema 版本为 25，本版本只支持 26。
本版本不会改动它，但读不了它；要继续其中已有的作品，请用创建它的旧版本打开。
要用本版本开始创作，请指定另一个数据目录启动，例如：nekowriter --data-dir <新目录>
本次启动失败的原因同时记录在 /private/tmp/nw-schema-probe/data2-copy/logs。
Error: Store(UnsupportedSchema { found: "25", expected: 26 })
```

- 同一原因同时进了 `data2-copy/logs/nekowriter.2026-09-14.log`；从终端启动时这段说明打印在 stderr，不再只剩一行英文调试输出（图形界面启动看不到终端，可依赖的仍然只有 `logs/`，见 §39.5）。
- 退出码仍为 1，数据库的 SHA-256 与探测前一致（`a1e37b05907f4444…`）：拒绝依旧不写入既有数据。
- 另两条路径同场核验：指向一个装有非 NekoWriter SQLite 文件的目录 → 给出「这个目录里的数据库不是 NekoWriter 的数据（缺少 schema 标识）」与同样的前进路径；指向全新空目录 → 照常启动，`/api/health` 返回 `schemaVersion=26`，无回归。

### 39.5 留下的事

1. **图形界面启动仍无窗口级提示。** 现在保证的是 stderr 与 `logs/` 有说明；macOS 双击发行包启动时，用户依旧可能只看到「闪一下就没了」。真正做到可诊断需要一条启动失败的原生提示路径，那是独立的新功能，本轮按「不扩大范围」没有做。
2. **旧目录被本版本读取的迁移仍然不存在。** 旧作品只能由创建它的旧版本打开，这是 docs/02 §8.3 写明的约定，不是本轮遗留的缺口。
3. **拒绝打开时会在这个目录的 `logs/` 里多一行。** 日志在拿到数据目录之后、打开数据库之前初始化；数据库本体与既有日志内容不受影响，这是把「启动诊断进入 logs/」这条承诺兑现的代价。

## 40. 2026-09-14 Embedding 设置与知识库路径的真界面核对（含一处报错可诊断性修复）

### 40.1 为什么核对这条路径

§37 移除 keyring 时改了两条设置路径（实体列改名、新增「`apiKey` 省略即保留原密钥」语义与唯一的密钥读取出口），但真实界面上只跑过**模型设置**；**Embedding 保存、连接测试与知识库检索**此前只有 store 层单测。同一类漏检本轮已经出现过两次：`/api/questions/{id}/answer` 期望 `answers.answers` 而界面发 `answers: [...]`，全程 422 却单测全绿；§39 的旧 schema 拒绝打开也是「断言过、没跑过」。这一轮把这条表面按真实入口走了一遍，并对请求体与响应体**两侧**抓报文核对，而不是只看返回码。

### 40.2 环境与端点可用性

| 对象 | 取值 |
| --- | --- |
| 实例一 | 数据目录 `/tmp/nw-embed-probe/data`（schema 26，端口 8814）：配置 Embedding，走完整知识路径 |
| 实例二 | 数据目录 `/tmp/nw-embed-probe/data-nocfg`（端口 8815）：从不配置 Embedding，验降级 |
| 二进制 | 由当前源码构建（本机仍需覆盖 `.cargo/config.toml` 的 `PROTOC`，见 §37.4）；界面为应用托管的 `webui/dist` |

**真实 Embedding 端点在本环境不存在**：scnet 的 `GET /api/llm/v1/models` 返回 18 个对话模型、没有 Embedding 模型，`POST /api/llm/v1/embeddings`（模型 `bge-m3`）返回 422 `Model Not Exist`；InferX 的 `/embeddings` 返回 404。因此供应商一侧用本地 OpenAI 形状的桩（`/tmp/nw-embed-probe/embed-stub.py`，`http://127.0.0.1:8899/v1`）承接：它记录收到的每个请求体与 `Authorization` 的指纹（只记长度与 SHA-256 前 12 位），返回 8 维向量与 usage。请求规则与响应解析因此是被验证过的；**真实服务商的向量质量没有被验证，这一点不在下文结论里冒充。**

### 40.3 两侧形状核对（请求体与响应体都抓）

界面里注入 `fetch` 记录器（不改产品代码），把真实的请求体字符串与响应体原文都留下。逐次结果：

| 界面动作 | 请求体（原文） | 响应 |
| --- | --- | --- |
| 打开面板 | `GET /api/settings/embedding` | `200 {"embedding":null}` |
| 保存（带密钥） | `PUT` `{"commandId":"754082d0-…","expectedRevision":"0","apiBaseUrl":"http://127.0.0.1:8899/v1","modelId":"stub-embed","dimension":"8","apiKey":"probe-embed-key-001"}` | `200 {"embedding":{"profile":{…"dimension":"8"},"credentialConfigured":true,"revision":"1","updatedAtMs":"1789399705480"}}` |
| 再保存（密钥留空） | `PUT` 同上，但 `"apiKey":null`、`"expectedRevision":"1"` | `200`，`revision` 进到 `"2"`，`credentialConfigured` 仍为 `true` |
| 连接测试（密钥留空） | `POST /api/settings/test-embedding` `{"apiBaseUrl":…,"modelId":"stub-embed","dimension":"8","apiKey":null}` | `200 {"dimension":"8"}`，面板提示「连接成功，服务返回 8 维向量」 |

供应商一侧（桩日志）证明密钥确实从设置行读出并发出：三条路径（`保存前验证`、`索引`、`查询`）的请求都是 `POST /v1/embeddings`，体为 `{"model":"stub-embed","input":[…],"encoding_format":"float"}`，`Authorization: Bearer <19 字符>`，其 SHA-256 前 12 位 `4a5a4da63eaf` 与 `sha256("probe-embed-key-001")` 的前 12 位相同。**第二次保存与连接测试的请求体里 `apiKey` 是 `null`，供应商收到的仍是同一把已存密钥**——这就是「省略即保留」语义与唯一读取出口 `embedding_api_key()` 的现场证据。

密钥正文的落点也逐项核过：不在任何 HTTP 响应里（读取视图只带 `credentialConfigured`），不在应用日志里，不在 `command_receipts` 的响应列里，只在本机 SQLite 的 `embedding_settings.api_key` 列。回执里落的是 `payload_hash`（`2baf8e43…`、`9eafe2e4…`）——**正文不在那里，但那个摘要覆盖了请求里的密钥字段**，属 §37 遗留的开放项（见本轮 §41 末尾的遗留清单）。

### 40.4 完整知识路径

以「雾港设定 / 雾港的灯塔制度」为例，全部经真实界面触发：

1. 建 Space：`POST /api/knowledge/spaces` `{"commandId":…,"title":"雾港设定"}` → `201`。
2. 建条目：`POST /api/knowledge/entries`（`keywords` 是数组、`source` 可空）→ `201`，`indexStatus:"pending"`；索引器在 1 秒内完成：`knowledge_index_tasks.status = ready`，`embedding_profile_id = c823ca0c…`，条目正文作为一次 `input` 发给供应商（桩日志可见原文），向量写入 `data/lancedb/knowledge_c823ca0c….lance`。
3. 检索：`POST /api/knowledge/search` `{"query":"守灯人的任期与铜牌","spaceIds":["19f9f2c3-…"],"topK":5}` → `200`，命中 `chunkId`/`entryId`/`entryRevision`/`spaceId`/`chunkIndex`/`title`/`text`/`score`，并在命中之外返回 `coverage`（`ready 1 · pending 0 · failed 0 · notConfigured 0`）。查询词作为一次 `input` 发给供应商。
4. 重建索引：`POST /api/knowledge/reindex` `{"commandId":…}` → `200 {"readyEntries":"0","pendingEntries":"1",…}`，面板提示「已重建索引：已索引 0 · 等待 1 · 失败 0」；随后任务回到 `ready`，`embedding_index` 操作记录第二次。

用量与操作记录（`system_model_operations`，全部 `usageSource = embeddings_http`、`usageStatus = reported`）：`embedding_connection_test` ×7、`embedding_index` ×2、`embedding_query` ×1；`system_model_operations` 的写入路径与模型请求共用一条（docs/02 §11）。

### 40.5 降级：从未配置 Embedding（实例二）

- 建 Space、建条目照常成功（`201`），条目的 `indexStatus` 是 `not_configured`，面板提示「已保存；尚未配置 Embedding，暂不索引」——知识不会被静默丢弃，也不会被标成已索引。
- 检索返回 `409 {"error":{"code":"EMBEDDING_NOT_CONFIGURED","message":"尚未配置 Embedding，语义检索不可用；知识仍可正常保存",…}}`，同一条文案就是界面提示。
- `GET /api/capabilities`、知识积累区显示「暂无可用引擎」（生产注册项为 0，docs/02 §7.4），因此**知识积累没有也不能在本轮走通**。

### 40.6 发现的唯一实质缺陷：Embedding 失败不可诊断（已修）

形状本身没有发现不匹配；发现的是**报错不可诊断**。修复前，两条失败路径都返回同一句不含原因的文案，`details` 还是空的：

```
PUT  /api/settings/embedding      dimension "4"（供应商返回 8 维）
→ 502 {"code":"EMBEDDING_SERVICE_FAILED","message":"Embedding 服务调用失败；请检查地址、模型、维度与密钥","details":{}}

POST /api/settings/test-embedding  apiBaseUrl http://127.0.0.1:9999/v1（无人监听）
→ 502 同上文案、details:{}
```

而适配器早就知道确切原因（`Embedding 维度不符：期望 4，实际 8`、`Embedding 服务请求失败：error sending request for url (…)`），它只出现在日志与索引任务的 `index_error` 里。界面的错误提示只渲染 `message`（`KnowledgePanel.errorText` → `error.message`），用户拿到的是「请检查地址、模型、维度与密钥」——**四个候选里猜一个**，而这类失败恰恰是本轮要求修掉的那一类。

修复（`crates/nekowriter-app/src/main.rs` 的 `ApiError::Embedding` 分支，单一分支按变体判定状态码）：

| 项 | 修复前 | 修复后 |
| --- | --- | --- |
| `message` | 通用「请检查地址、模型、维度与密钥」 | 适配器的具体原因 |
| `details` | `{}` | `{"reason": <同上>}`，机器可读 |
| 状态码 | 配置类 `422 INVALID_EMBEDDING_SETTINGS`／其余 `502 EMBEDDING_SERVICE_FAILED` | 不变 |

同场重放（同一份桩、同一台机器、同一个界面）：

```
502 {"code":"EMBEDDING_SERVICE_FAILED","message":"Embedding 维度不符：期望 4，实际 8","details":{"reason":"Embedding 维度不符：期望 4，实际 8"},…}
502 {"code":"EMBEDDING_SERVICE_FAILED","message":"Embedding 服务请求失败：error sending request for url (http://127.0.0.1:9999/v1/embeddings)",…}
```

面板提示与 `message` 一致。两次失败都没有写坏状态：`embedding_settings` 仍是 `model_id = stub-embed / dimension = 8 / revision = 2`，密钥原样；失败也照旧落进 `system_model_operations`（维度不符记 `failed`，连不上记 `unknown`——无法断定请求是否到达供应商，与 docs/02 §11 的口径一致）。

新增三个测试钉住这条路径的两侧真实形状（`crates/nekowriter-app/src/main.rs` tests）：

| 测试 | 钉住的东西 |
| --- | --- |
| `embedding_requests_accept_the_payload_the_ui_sends` | 界面真实 PUT 与 POST 报文可解析；`apiKey: null` 保持 `None`；`dimension` 只接受十进制字符串（数字 8 被拒绝，避免与 JS 安全整数边界分叉） |
| `embedding_settings_response_matches_the_interface_shape` | `{"embedding":null}` 与配置后的完整键名（`profile.apiBaseUrl`/`modelId`/`dimension`、`credentialConfigured`、`revision`、`updatedAtMs`，长整数一律字符串） |
| `embedding_failures_name_the_concrete_reason` | 维度不符 → `502` + 具体文案 + `details.reason`；空模型 ID → `422` + 具体文案 |

`nekowriter-app` 用例从 58 增至 61。

### 40.7 验证与未覆盖

- `cargo test --workspace` **229 通过 / 0 失败**（agent 43 / app 61 / core 38 / store 83 / xtask 4）；`cargo fmt --all -- --check` 干净；`cargo build -p nekowriter-app` 只有既有的链接器告警（`__eh_frame section too large`，与本改动无关）。
- 实测留下的文件：`/tmp/nw-embed-probe/{data,data-nocfg}`、`stub.log`（供应商侧请求）、`app*.log`。实例一在本轮结束时仍在 `127.0.0.1:8814` 上运行；实例二（`8815`）在本节写完后即被关停，数据目录 `/tmp/nw-embed-probe/data-nocfg` 保留，需要时用同一条命令重启即可。
- **未覆盖**：真实 Embedding 服务商（本环境没有，见 §40.2）；知识积累（生产注册项为 0）；写作流程里的 `query_knowledge` 工具调用（需要稿本内的一次准备/细纲运行，本轮没有为它起新稿本）；逐次确认模式与委托审核（同 §38.5）。
- **并列缺口（本轮按范围没动）**：模型连接测试走 `ApiError::Provider`，仍是通用文案「模型服务未通过实际连接测试；请检查地址、模型、密钥和能力配置」+ `details: {}`。它和本次修复后的 Embedding 表面不再一致；要不要同样点名原因，需要拍定。

## 41. 2026-09-14 启动诊断收口：每一步失败都可诊断，且说明与行为一致

§39 引入的启动失败说明有两处缺陷（审计指出，本轮先复现后修）：出口只接住了五步里的两步；以及说明里那句「本版本不会改动它」与行为不符。本节记录修复、重放现场与口径修正，并修正 §40 里两处不准的陈述。

### 41.1 缺陷 A：五步里只接住了两步（复现）

`startup_failure` 只包住了 `ProcessLock::acquire` 与 `Store::open`；在它之前还有 `default_data_dir`、`resolve_data_dir`、`init_logging`，之后还有 `default_web_dir`，四处仍是裸 `?`。三个真实场景（现场文件：`/tmp/nw-startup-probe/before/{readonly,notdir,logs}.txt`）：

```
# 只读文件系统：--data-dir /nw-probe-should-fail（只读挂载的旧备份正是这个场景）
exit=1
Error: Io(Os { code: 30, kind: ReadOnlyFilesystem, message: "Read-only file system" })

# 数据目录的父路径是一个普通文件（ENOTDIR）
exit=1
Error: Io(Os { code: 20, kind: NotADirectory, message: "Not a directory" })

# 日志位置被一个普通文件占住（init_logging 失败）
exit=1
Error: Io(Os { code: 17, kind: AlreadyExists, message: "File exists" })
```

三例都有目录缺失、无中文原因、无下一步，也没留下任何日志行；`main.rs` 里那句注释「启动期失败的统一出口」因此名不副实。

### 41.2 缺陷 B：说明声称「不会改动它」，实际会写（复现）

拒绝打开旧目录时会往该目录写入两样东西：`logs/` 里一行 ERROR，以及 `nekowriter.lock`（一个此前没有锁文件的目录在拒绝后会多出 0 字节的锁文件：§39.1 的 `alien` 探针目录里就是如此）。而在修复前，同一场景的说明写着「本版本不会改动它，但读不了它」——「不改动作品数据」为真，「不会改动它」为假。

### 41.3 收口（最小改动，均落在 `crates/nekowriter-app/src/main.rs`）

| 位置 | 改动 |
| --- | --- |
| 新增 `StartupStep`（`DataDirectory` / `Logging` / `ProcessLock` / `Database` / `WebDirectory`） | 每一步一个中文标签与一句只属于该步的下一步（可写、不能用 `--data-dir`、只能有一个进程、用旧版本打开、用 `--web-dir`） |
| `startup_failure(data_dir, logs_dir, step, error)` | 唯一的出口：先 `error!`（日志已就绪时同时进 `logs/`），再向 stderr 打印说明，然后原样透传错误；`logs_dir` 只在日志确实可写后给出 |
| `startup_failure_reason(step, error)` | 原因与下一步的唯一出处：旧目录两种情形保留原有详细中文说明；`NoLocalDataDirectory` 单独一句；其余统为「步骤标签（原始错误）」+ 该步的下一步 |
| `startup_failure_message(data_dir, logs_dir, step, error)` | 头部「NekoWriter 无法启动。」+「数据目录：…」+「原因：…」+ 下一步；日志可写时追加一句声明实际写入内容：**「除了该目录原有的文件，本次启动只写入了启动日志与进程锁文件，作品数据（SQLite 与向量）未被修改」**，不再出现「不会改动它」 |
| `prepare(cli) -> Startup` | 启动期五步（`default_data_dir` / `resolve_data_dir` / `init_logging` / `ProcessLock::acquire` / `Store::open` / `default_web_dir`）从 `main` 移到这个函数，每一步都是 `match … { Err(error) => return Err(startup_failure(…)) }`，不再有裸 `?`；返回值持有日志守卫与进程锁，`main` 只负责把它们保持到进程结束 |

不新增功能、不改其它行为：正常路径只是多了一层 `match`，`web_dir` 不存在仍只是一条 `warn!`。把启动步骤收进 `prepare` 是必需的：直接写在 `main` 里会让它超过 100 行并触发新的 clippy 告警（重构后 `crates/nekowriter-app` 的告警数与改动前一致，均为 18 条）。

### 41.4 修复后现场（同一批场景重放）

```
# 只读文件系统（与 41.1 同一条命令）
exit=1
NekoWriter 无法启动。
数据目录：/nw-probe-should-fail
原因：无法准备数据目录（文件系统错误：Read-only file system (os error 30)）
请确认这个路径存在且可写，然后重试；也可以改用另一个数据目录启动，例如：nekowriter --data-dir <新目录>
Error: Io(Os { code: 30, kind: ReadOnlyFilesystem, message: "Read-only file system" })

# logs 位置被文件占住
exit=1
NekoWriter 无法启动。
数据目录：/private/tmp/nw-startup-probe/before/logsblocked/data
原因：无法在该目录下创建日志（文件系统错误：File exists (os error 17)）
请确认这个目录可写（应用需要它下面的 logs/ 子目录），然后重试
Error: Io(Os { code: 17, kind: AlreadyExists, message: "File exists" })
```

在数据目录与日志两步失败时，说明里**不出现**「只写入了启动日志」这类话（那一步根本写不出日志）；修正后的单测与集成测试都钉住了这一点。

只接住了两步的那个旧出口本来就带「进程锁」分支，但它此前只有笼统错误行；现在它也有同一套说明。同一份数据目录上开第二个进程（现场：`/tmp/nw-startup-probe/after/`）：

```
exit=1
NekoWriter 无法启动。
数据目录：/private/tmp/nw-startup-probe/ok
原因：无法锁定数据目录（数据目录已被另一个 NekoWriter 进程占用：…/nekowriter.lock）
同一时间只允许一个进程打开同一个数据目录；请关闭正在运行的那个 NekoWriter，或改用另一个数据目录启动
除了该目录原有的文件，本次启动只写入了启动日志与进程锁文件，作品数据（SQLite 与向量）未被修改；日志：…/logs。
```

同一场回归还确认了重构没有碰坏正常启动：新目录在 `127.0.0.1:8816` 上起服务、`/api/health` 返回 `schemaVersion=26`，`logs/` 里确实落下了「已启动」那行（日志守卫没有被提前 drop），而上面那个第二进程被拒——进程锁仍然有效。

拒绝打开 schema 25 的旧目录（把 `/tmp/nw-story-run/data2` 再拷一份到 `/tmp/nw-startup-probe/schema25/copy`，原始目录只读不动）：

```
exit=1
NekoWriter 无法启动。
数据目录：/private/tmp/nw-startup-probe/schema25/copy
原因：这个目录由更早的 NekoWriter 创建，数据库 schema 版本为 25，本版本只支持 26。
本版本不会读取或迁移其中的作品数据；要继续其中已有的作品，请用创建它的旧版本打开。
要用本版本开始创作，请指定另一个数据目录启动，例如：nekowriter --data-dir <新目录>
除了该目录原有的文件，本次启动只写入了启动日志与进程锁文件，作品数据（SQLite 与向量）未被修改；日志：/private/tmp/nw-startup-probe/schema25/copy/logs。
Error: Store(UnsupportedSchema { found: "25", expected: 26 })
```

该说法逐项核过（拒绝前/后对同一份拷贝取文件清单与哈希）：

| 对象 | 拒绝前 | 拒绝后 |
| --- | --- | --- |
| `nekowriter.sqlite3` | `a1e37b05907f4444…` | **同一值**（与原始 `data2` 也一致） |
| `nekowriter.lock` | 0 字节 | 0 字节（哈希不变；本就没有时会被创建） |
| `logs/nekowriter.2026-09-14.log` | `d28ad9f9c2c19574…` | `0f44549e543e44f3…`（只多一条 ERROR） |
| 空的 `nekowriter.sqlite3-wal` / `-shm` | 存在 | 被 SQLite 在干净关闭时清掉 |

也就是说：作品数据（主库内容）不变，写入的只有日志与进程锁；文案现在说的是这两件真事，而不是「不会改动它」。

### 41.5 测试：把接线也钉住

| 测试 | 位置 | 钉住的东西 |
| --- | --- | --- |
| `startup_failure_messages_name_the_directory_step_cause_and_way_forward`（改写自 §39 那条） | `crates/nekowriter-app/src/main.rs` 单测 | 旧 schema/未知 schema 两种详细说明；IO 失败时原因用中文点名步骤（`原因：无法准备数据目录（…）`）；进程锁失败给出「只能有一个进程」的下一步；未指定数据目录时给出 `--data-dir`；`--web-dir` 路径；**日志不可写时不得出现「只写入了启动日志」；不得出现「不会改动它」** |
| `unusable_data_directory_is_reported_with_cause_and_next_step`、`unwritable_log_location_is_reported_too` | 新增 `crates/nekowriter-app/tests/startup_failure.rs` | 跑真正的二进制（`CARGO_BIN_EXE_nekowriter`），断言上面两个场景的 stderr 与退出码——纯函数测试证明不了「每一步都接在了出口上」，集成测试能 |

两个集成用例用文件系统造确定性失败，不需要 root、不绑端口：数据目录的父路径放一个普通文件（ENOTDIR），以及把 `logs` 占位成普通文件（`init_logging` 失败）。

### 41.6 同轮修正的口径差

| 位置 | 修正 |
| --- | --- |
| docs/03 §40.3 | 「不在 `command_receipts`（那条只存 `payload_hash`）」改为「响应列里没有密钥；回执里落的 `payload_hash` 覆盖了请求里的密钥字段，属开放项」——原文读起来像那里没有密钥材料，与事实不符 |
| docs/03 §40.7 | 实例二（`8815`）在本节写完后即被关停，原句写成「在 8815 上运行」已过期 |
| docs/02 §8.3 | 「声明该目录未被修改」改为「声明其中的作品数据未被读取或修改」，并补上启动每一步都走同一出口、日志未就绪时不得声称已记录的两条要求 |
| docs/01 | 密钥一行从「Windows 凭据存储 + keyring 生态」改为「本地 SQLite 设置行」（全仓最后一句 keyring 表述） |

### 41.7 验证与未覆盖

- `cargo test --workspace` **231 通过 / 0 失败**（agent 43 / app 单元 61 + 集成 2 / core 38 / store 83 / xtask 4）；`cargo fmt --all -- --check` 干净；`cargo clippy -p nekowriter-app --all-targets` 的告警数与改动前一致（bin 18 条、test 27 条），改动区域无新增告警——中途曾多出 3 条 `format_push_string` 与 1 条 `too_many_lines`，已由 `lines.join("\n")` 与 `prepare` 拆分消除。
- 现场文件：`/tmp/nw-startup-probe/{before,after}/`（各场景的原始 stderr）。原始数据目录 `/tmp/nw-story-run/data2` 始终只读，未被本次实验改动（其拷贝的主库哈希与其自身一致）。
- **未覆盖**：崩溃遗留**非空** WAL 的目录（本轮只造出 0 字节 WAL；观察到的是拒绝打开会清掉遗留的空 `-wal`/`-shm`，主库字节不变。若 WAL 非空，SQLite 在打开/关闭时可能把它 checkpoint 回主库——内容不变、字节可能重写，这一点没有实测）；图形界面双击启动仍无窗口级提示（§39.5.1，本轮按范围未做）。
- **仍未动的开放项**：`command_receipts.payload_hash` 覆盖密钥（§40.7 已有，本轮按指示未碰）；没有任何路径把密钥清回未配置。

## 42. 2026-09-14 移除 keyring 后的主流程验收跑：一章闭环（准备 → 细纲 → 正文 → 整章审核 → 定稿 → 导出）

§37 到 §41 四次带代码改动的回合都只验过边界（启动失败出口、HTTP 表面、幂等与重启持久化、Embedding 表面）。本节补上一直没有发生的那件事：用**当前源码构建的二进制**在**全新数据目录**上、通过**真实界面**、对着**真实端点**从空目录跑完一章的完整闭环。结论：全流程走通，没有需要修的失败。

### 42.1 环境与入口

| 项 | 值 |
| --- | --- |
| 二进制 | `cargo build -p nekowriter-app`（沿用 `cargo --config 'env.PROTOC.value="/opt/homebrew/bin/protoc"'`，原因见 §37.4） |
| 数据目录 | `/tmp/nw-e2e-chapter/data`（全新，schema 26），`--port 8820 --web-dir ./webui/dist --no-browser` |
| 界面 | `http://127.0.0.1:8820`（桌面端预览标签页，真实渲染 + 真实指针/键盘事件） |
| 服务 | `https://api.scnet.cn/api/llm/v1`，模型 `DeepSeek-V4-Pro`，上下文 131072 / 预留输出 4096，能力 streaming + tools + streamUsage，thinking=false |
| 作品 | 项目「守灯人」/ 稿本「初稿」/ 小说《灯塔三夜》，现实向三章短篇，目标逐字写在界面的「小说目标」框里 |

### 42.2 逐步现场

**① 模型设置保存两次，第二次不填密钥。** 第一次（带密钥）落 `revision=1`；连接测试 `connection_test` `succeeded`，用时 7.2 秒，用量 输入 28 / 输出 22 / 推理 18。第二次只把上下文上限 128000 改成 131072、密钥框留空：

```
保存前: revision=1 key_sha=d4c2af9bd54f ctx=128000
不重填密钥保存后: revision=2 key_sha=d4c2af9bd54f ctx=131072
```

（`key_sha` 是 `sqlite3 … 'select api_key from system_model_settings;' | shasum -a 256` 的前 12 位。）此后**全程 6 次成功的模型请求都由这条 revision=2 的设置派发**。

**② 建作品、保存目标、开始准备。** 界面操作对应 `POST /api/projects` 201（返回 project/version 一对 id）、`PUT …/brief`、`POST …/preparation/start`。准备 Run `c8eb8eb1` 发生 3 次请求（121.4s / 207.9s / 94.6s）。

**③ 模型反问 → 界面提交回答 → Run 真的续跑。** 准备阶段模型用 ask_user 提了 4 个问题，界面出现「继续前需要你的回答」，选完提交后抓到请求与响应两侧的真实形状：

```
POST /api/questions/b8d0ed73-…/answer
{"commandId":"f1d48356-…","expectedStateRevision":"3",
 "answers":[{"questionId":"location","selectedOptionIds":["east_china_sea"],"customText":null},
            {"questionId":"letter_origin","selectedOptionIds":["supplies"],"customText":null},
            {"questionId":"protagonist_age","selectedOptionIds":["mid40s"],"customText":null},
            {"questionId":"ending_tone","selectedOptionIds":["brief_contact"],"customText":null}]}
→ 200 {"questionId":"b8d0ed73-…","runId":"c8eb8eb1-…","workflow":{"stage":"preparing_generating","execution":"queued","stateRevision":"4",…}}
```

随后 Run 回到 `running` 并发出 attempt 2——这正是 §36 修掉的那个「提交回答后稿本卡在 `waiting_answer`、一次请求都发不出去」的路径，方向相反地验了一遍。

**④ 四项准备产物逐项审核。** 界面出现 4 张待审卡片（背景设定 / 资料收集 / 人物设定 / 小说粗纲），逐一点「通过」：

```
POST /api/reviews/0995dbf4-…/decision
{"commandId":"3672f7d5-…","expectedStateRevision":"16","passed":true,"issues":"","citations":[]}
→ 200 {"versionId":"fc21be6c-…","workflow":{"stage":"writing_drafting",…}}
```

4 条 review 全部 `approved` / `decision_source=human`，Run `c8eb8eb1` `succeeded`（`requests=3`），章节计划按粗纲落库：`第一章「补给日」`、`第二章「对岸」`、`第三章「应答」`，状态 `planned`。

**⑤ 本章指导 + 细纲。** 指导逐字填进「本章指导」框并保存，随后：

```
POST /api/chapters/c38cc529-…/outline/start  {"commandId":"…","expectedStateRevision":"24"} → 200，stage=writing_outline_generating
```

Run `91a59f0e` 一次请求（141.0s）产出细纲，内容忠实于指导（第三章标题与「通过信号灯与对岸简短交流」这条约定都落进了「本章定位 / 写作约定 / 收束与承接」）。

**⑥ 细纲审核 → 正文。** 点「通过并进入正文」：`POST /api/reviews/…/decision` `passed:true` → 200，stage `writing_drafting`，Run `448c5257` 两次请求（115.6s / 27.5s）。日志两行把「工具落库 + 进入整章审核」写清：

```
16:46:48 INFO 章节正文工具调用已处理 request_id=fc16121f-… chapter_id=0b0e4de8-… tool_call_id=call_727dfdb2ed334ca0a41768bd status="applied"
16:47:15 INFO 章节正文模型提交已进入整章审核 request_id=da32c7b6-… chapter_id=0b0e4de8-…
```

正文由 `append_content` 工具落库，正文修订幅 4202 字符。

**⑦ 整章审核 → 定稿。** 点「通过并定稿」：`POST /api/reviews/e784393e-…/decision` `passed:true` → 200，stage `writing_chapter_done`、`execution=idle`。库中 `chapters` 第 1 章 `status=approved` 且 `approved_revision_id` 非空，第 2、3 章仍 `planned`。

**⑧ 导出 Markdown 并落盘。** 界面「稿本管理 → 导出 Markdown」，面板提示 **「已导出 灯塔三夜.md」**；同端点取回文件核对：

```
GET /api/versions/fc21be6c-…/export?format=md → 200
content-type: text/markdown; charset=utf-8
content-disposition: attachment; filename="nekowriter-export.md"; filename*=UTF-8''%E7%81%AF%E5%A1%94%E4%B8%89%E5%A4%9C.md
content-length: 12443
```

落盘 `/tmp/nw-e2e-chapter/export/灯塔三夜.md`，12443 字节；开头 `# 灯塔三夜` + `由 NekoWriter 导出 · 2026-09-14 16:50:15 UTC · 正文 1 / 3 章已通过整章审核`，第 1 章是完整定稿正文，第 2、3 章标「（草稿）」「（本章尚无正文）」。

### 42.3 三条顺带确认

**不重填密钥保存一次后，派发仍能拿到密钥。** 见 42.2 ①：保存把 `revision` 从 1 推到 2 而密钥摘要不变，之后 6 次请求全部成功。更强的证据是全库扫描——密钥正文在整个数据目录里**只出现 1 次**，就在 `system_model_settings` 那一行：

```
$ sqlite3 … ".dump" | rg -c "$K"          → 1
$ sqlite3 … ".dump" | rg "$K" | rg -o "INSERT INTO [a-z_]*"  → INSERT INTO system_model_settings
$ rg -c "$K" logs/*.log run.log            → 0（日志 0 次）
$ rg -a -c "SecKeychain|find-generic-password|keyring" target/debug/nekowriter → 0 次
```

即：密钥不在任何一次请求快照、事件、工具调用、命令回执或日志里；二进制里连钥匙串符号都没有。

**`/api/runs/{id}/continue` 立即返回且真的发出请求。** 为造出「需要显式继续」的状态，先把第 2 章细纲 Run 在运行中点「停止执行」（`attempt 1` 于 27.7s 被 `cancelled`），再直接调这个接口：

```
POST /api/runs/7757159b-…/continue  {"commandId":"…","expectedStateRevision":"27"}
→ http=200 time=0.003374s
立即:   stopped rq=1 resume=0  →  queued  rq=1 resume=1（stateRevision 27→28）
15 秒后: running rq=2 resume=1；model_requests attempt 1 cancelled / attempt 2 dispatched
```

这条续跑没有停在「发出了请求」：attempt 2 于 267.0s 后 `succeeded`（输入 11278 Token），日志 `16:55:54 INFO 章节细纲模型产物已提交，等待审核 chapter_id=c38cc529-…`，Run 转入 `waiting`。即显式继续不只是把请求发出去，被中断阶段真的被接着做完了。

对比 §37 记录的同一动作：keyring 时代它 `http=000`、20 秒超时、界面把「显式继续」按成永久 disabled。附带一条形状观察：同一接口在缺 `commandId` 时 422 用时 0.0035s、缺 `expectedStateRevision` 时 422 用时 0.0016s——拒绝也从不阻塞。

**全过程没有钥匙串/授权弹窗。** 整轮里 `ps -eo pid,comm | rg -i SecurityAgent` 只出现过 `_securityagent /usr/sbin/distnoted agent`（系统常驻守护，与钥匙串弹窗无关），没有 SecurityAgent 提示进程。

### 42.4 用量与耗时（本轮实测）

| Run | 请求数 | 墙钟 |
| --- | --- | --- |
| `c8eb8eb1` preparation | 3 | 967.1s |
| `91a59f0e` chapter_outline（第 1 章） | 1 | 622.4s |
| `448c5257` chapter_body（第 1 章） | 2 | 263.2s |
| `7757159b` chapter_outline（第 2 章，仅用于验 continue） | 2（1 cancelled + 1 dispatched） | — |

6 次成功请求合计 **输入 40910 / 输出 30253 / 推理 12002 Token**；单次最慢 207.9s，最快 27.5s。`command_receipts` 18 条、`events` 81 条、`system_model_operations` 只有 1 条（连接测试）——写作请求走的是 `model_requests`，与 §40.1 的口径一致。

### 42.5 过程中出现的一次协议错误（§36.1 再次现场生效）

准备 Run 中途有一次协议错误：

```
16:25:18 INFO 模型协议错误已保存，已要求同一 Run 纠正 request_id=Some(a2eecd93-…) lane=WritingRun consecutive_protocol_errors=1
```

该 Run 最终 `succeeded`、`consecutive_protocol_errors` 归 0、请求数停在 3——与 §38 观察到的行为一致：协议错误只在同一 Run 内自适应纠正，不推进业务计数、不报废 Run。

### 42.6 未覆盖（本轮范围之外）

- **本轮只跑了第 1 章闭环。**（第 2 章的细纲→正文→定稿已于 §43 走完；第 3 章仍未跑。）第 2 章当时只走到细纲审核：那一轮用来验显式继续，续跑后确实产出细纲并停在 `waiting`（见 42.3），我没有再点「通过并进入正文」，也没有跑它的正文与定稿。章节计划已锁定的三章标题与 42.2 ④ 一致。
- ~~**逐次确认正文编辑**与**委托审核**~~：两条路径已于 §43 补走（委托审核端到端走通；逐次确认候选接受两次；人工编辑正文后再提交整章审核）。其中一处界面事实值得记住：正文编辑器只在「自动接受」模式下渲染（§43.6）。
- **知识库检索**：准备阶段没有声明 `query_knowledge`（本实例没有知识条目、生产引擎为 0），因此写作流程里的检索工具调用仍未走过；Embedding 表面见 §40。
- **完本确认与长篇阅读视图**未走（只到单章定稿）。
- **导出只验了 Markdown**：`format=txt` 未点，同一 handler 的不同分支。
- 一条未进一步定位的工具观察：第一次用 DOM 合成 `click()` 提交细纲审核时没有产生请求，改用真实坐标点击后 200；没有深追（可能是我点击时那份待审列表正在按 SSE 重渲染）。

## 43. 2026-09-15 分支实战：委托审核与正文逐次确认（同一真实实例续跑）

§42 的一章闭环之后，本轮补走两条此前只在单测里绿过的分支，并在现场找到两个真缺陷（一个已修并在真实端点做了同一模型同一上下文的 A/B，一个记录了修法但语义待拍定）。

### 43.1 装置与环境

- 实例：`target/debug/nekowriter --port 8820 --data-dir /tmp/nw-e2e-chapter/data --web-dir ./webui/dist --no-browser`，schema 26，数据目录沿用 §42（第 1 章已定稿、第 2 章停在细纲审核、第 3 章 planned）。端点 `https://api.scnet.cn/api/llm/v1`、模型 `DeepSeek-V4-Pro`、上下文 131072、思考关闭。
- 入口：真实界面（预览里的真实指针/键盘事件）。本轮既有真实坐标点击也有 DOM 合成 `click()`，两者都产生了真实请求（每次操作后都用 `POST/GET` 的真实报文核对），§42.6 里那条「合成 click 不产生请求」的观察在本轮没有复现。
- 二进制在过程中重建两次（第一次带 43.3 的修复，第二次带 43.4 b 的修复），每次停旧进程、起新进程、`GET /api/health` 确认后继续。`/tmp/nw-story-run/data2` 与 `data3` 全程未动。

### 43.2 委托审核：分支端到端走通

待审对象是第二章细纲 `review 9e7c9905`（此前一次委托的子 Run `b88eca16` 因进程退出被记为 `failed` + `reason=process_shutdown`，review 回到 `open`——这条恢复本身也顺带验证了）。

| 时刻 | 动作与证据 |
| --- | --- |
| 界面点「委托审核」 | `POST /api/reviews/9e7c9905-…/delegate` → **202** |
| 子 Run 建出 | `f2c4908d`（`kind=review`，`parent_run_id=7757159b`，`stage=review.chapter.outline`）；events 91 `review.changed status=delegated` → 92 `run.changed queued` → 93 `running`（同一事务） |
| 子 Run 进行期间 | 父 Run `7757159b` 停在 `waiting`（stage `writing.outline`，request_count 2、cpe 0），审核条目 `delegated` |
| 子 Run 完成 | 1 次请求 102.4s，`succeeded`；events 94 `review.changed approved source=agent passed=true confidence=0.92`（issues 三条【建议】）→ 96 父 Run `succeeded` → 97 `workflow.changed stage=writing_drafting` → 98 子 Run `succeeded` |
| 日志 | `审核 Agent 结论已应用 review_id=9e7c9905… run_id=f2c4908d… request_id=dd7bb17f…` |

结论：委托审核在真实端点上完全正确——子 Run 生命周期、父 Run 停在 `waiting_review`、结论回填（`decision_source=agent`）与随后的阶段推进都对；委托之后工作流自动进入正文（不需要人再点「通过并进入正文」）。

### 43.3 缺口一：`chapter_id` 抄写错误无法从纠正里恢复（已修，真实端点 A/B）

**现场（修复前）。** 第二章正文 Run `735fc0e8` 在正文已经写入之后，模型连续三轮提交同一个抄错的 UUID：`c38cc529-7a2b-4408-9edf-…`，而真实值是 `c38cc529-7a2b-4409-8edf-…`（中间两组换位）。`agent_messages` seq 5/8/11 是三次 assistant 提交，seq 6-13 是三条配对错误结果与三条纠正；Run 以 `stopped`、`consecutive_protocol_errors=3` 收束（events 108-117，日志三行「模型协议错误已保存，已要求同一 Run 纠正…1/2」+「连续模型协议错误达到上限，已按 lane 收束…3」）。

**根因不在模型，在协议本身**（与 §35 同类）：当时纠正文本实测为

```text
上一轮对 `submit_chapter` 的调用未通过本阶段校验：阶段提交的 chapter_id 与当前章节不一致
请补齐后重新调用 `submit_chapter`；已提交且合法的内容可以原样复用，不必重写。
```

它既没有给出本轮唯一合法取值，也把「抄错」说成「缺字段」。模型的自救方式是**升级为提问**：点「显式继续」后它直接调 `ask_user`（seq 14，question `a2ae1ae1`，`wait_reason=ask_user`），问题正文就是「提交第二章时 chapter_id 持续被拒，提示与当前章节不一致。请确认应使用的 chapter_id，或指出问题所在。」——即从这条纠正里它无法修复。

**修复。** `PhaseSubmissionError::ChapterIdMismatch` 由无字段变成 `{ expected, actual }`，消息点名两个取值；纠正模板「请补齐后重新调用」改为「请按上面的原因修正后重新调用」（`crates/nekowriter-app/src/scheduler.rs`）。docs/02 §4.3 补一句要求：章节级提交的取值类校验失败必须给出合法取值本身。

**A/B 证据（同一 Run、同一模型、同一上下文，只差纠正文本）。** 人工回答把正确 `chapter_id` 写进答案后，模型**仍然**抄错（seq 16）；这一次纠正带上了两个取值（seq 17/18 原文）：

```text
上一轮对 `submit_chapter` 的调用未通过本阶段校验：阶段提交的 chapter_id 与当前章节不一致：本轮必须原样提交 c38cc529-7a2b-4409-8edf-62328409ff6d，收到的却是 c38cc529-7a2b-4408-9edf-62328409ff6d
请按上面的原因修正后重新调用 `submit_chapter`；已提交且合法的内容可以原样复用，不必重写。
```

下一轮（seq 19）它一次就改对，seq 20 收到 `PHASE_SUBMISSION_ACCEPTED`；日志 `00:25:41` 协议错误 1 → `00:26:01` 「章节正文模型提交已进入整章审核」。修复前同一上下文连续三轮都没能改对。

**测试。** `nekowriter-agent` 新增 `chapter_id_mismatch_names_the_only_legal_value`，直接用上面对 UUID 断言原因里同时出现必须原样提交的值与收到的值（细纲与正文两条提交路径都断言）；既有 `outline_and_chapter_submissions_are_bound_to_the_active_chapter` 改为匹配带字段的变体。agent 43 → 44。

### 43.4 缺口二：整章审核退回意见到不了正文 Agent（已修，见 §44）与它暴露的旧语义停 Run（已修）

**a. 退回意见只落在审核行里（语义待拍定）。** 在整章审核点「退回修订」并写下具体修改要求后：`POST /api/reviews/{id}/decision` 200、workflow 回到 `writing_drafting`、正文 Run 立刻被重新派发。但那份意见**只写进 `reviews.issues_json`**：

- `feedback` 表在该时刻 0 行（对照：用面板的意见框提交同类文字会立刻入库并显示为队列 1 条，§43.5 那一条就是这样来的）；
- 该意见文本在 `model_requests`（`prompt_snapshot_json`、`context_snapshot_json`）、`agent_messages`、`prompt_overrides` 里各 0 次命中。

后果是**退回后原稿空转回到审核**：events 140（`review.rejected`）→ 153（`review.changed open source=agent_submission`）→ 158（`workflow.changed stage=writing_drafting`），第二次审核的 `target_revision_id` 与第一次完全相同，正文一个字符都没变；中间还额外吃了两条「没有调用提交工具」的协议错误（见 b）。界面标签（「退回时说明情节、叙事、人物或文本层面的修改要求」）与正文 system 提示词（「结合后续历史消息中的工具结果、审核意见和用户反馈」）都承诺这份意见会到达模型，所以这是缺陷而不是设计。修法清楚：在 `decide_chapter_final_review` 的拒绝分支按 `submit_run_feedback` 的形状入队一条 `feedback`，让意见走既有的投递通道。**已于 §44 收口**：先把「谁才是权威」核到底——界面标签只承诺「说明要求」，而正文提示词（`chapter.write.user.md` 要求「结合后续历史消息中的工具结果、审核意见和用户反馈」）与 docs/02 §6.2 都明确承诺送达，因此按缺陷处理，而不是按「退回=人工接管」的语义处理。

**b. 未投递意见守卫的拒绝被当成协议错误，并在第三次把 Run 停下（已修）。** 现场（后一次退回 + 意见框提交之后）：模型先两个纯文本回合（日志 `00:33:06` 计数 1、`00:33:15` 计数 2），第三个回合它调 `submit_chapter`，被业务守卫拒绝——配对结果与纠正都写得很清楚：

```text
本章仍有未投递的自由意见，不能直接提交终稿审核。请继续正文阶段，并使用 append_content 或 replace_content 形成下一条完整工具结果，让意见随该结果投递。
```

但这一次拒绝被计入 `consecutive_protocol_errors`，计数到 3 → Run 当场 `stopped`（events 171：`reason=pending_feedback_requires_tool_result`、`consecutive_protocol_errors=3`、`wait_reason=protocol_error_limit`；172 workflow `stopped`；日志 `00:33:25`「章节提交连续不推进，已达到工程执行限制并停止」）。也就是说：**模型刚拿到的正确指令再没有执行机会，未投递意见永远留在 `queued`**（feedback seq 1 保持 `queued`），章节停在 `writing_drafting`。这正是 §36.1 明确留给后续处理的「提交内容被拒却按协议错误计数」那一处，现场证明它的后果不只是计数不准，而是整条 Run 报废。

修复：`commit_chapter_body_model_submission` 的未投递意见分支改为 §36.1 的「被拒回合」语义——立即重排、不推进计数、永不触发上限（非进展回合的上限由请求额度承担），`run.changed` 仍公布 `reason=pending_feedback_requires_tool_result` 但不再带计数与 `wait_reason`。docs/02 §2.3/§4.3/§5 的三处旧口径同步改写（原来明写「计入 `MAX_CONSECUTIVE_PROTOCOL_ERRORS`、第三次收束为 `stopped`」）；`crates/nekowriter-app/src/scheduler.rs` 的 `direct_chapter_submission_with_pending_feedback_requeues_for_a_tool_result` 改成断言「连续 3 次直接提交都只重排、`wait_reason` 为 `None`、事件不带计数」。

**已在真实端点复验**（见 §44.4）：修复后同一 Run 里守卫真实触发一次（请求 `9c1c06ab`），重排事件 seq 260 不带 `consecutive_protocol_errors`、不带 `wait_reason`，模型随后改用 `replace_content` 投递意见，Run 最终 `succeeded`、`cpe = 0`。触发一次而非修复前现场那种连续三次，但「不带计数、原地重排」的行为特征已在真实事件上出现。要完整复现「连续三次都被拒」仍需要桩模型，本轮没做。

### 43.5 正文逐次确认 + 人工编辑 + 定稿：走通

**逐次确认（`approve_required`）** 在真实界面设置：`POST /api/versions/{id}/approval-mode {mode:"approve_required"}` 200。随后：

| 步骤 | 证据 |
| --- | --- |
| 意见框提交意见 | `POST /api/runs/735fc0e8/feedback` 200；`feedback` seq 1 `queued`，界面显示「队列 1 条」；events 161 `feedback.changed`。这就是 43.4 b 里停在 `queued` 的那一条：events 173 `08:41:04` 是修复重建进程后的「显式继续」，它在 183/187 才被 `bound`→`included` |
| 模型改正文 | 事件 190 `tool.proposed name=replace_content status=awaiting_review`，候选 `baseRevisionId=99ddc296`（修订 1）、`candidateHash=f58df259…` |
| 界面「接受编辑」 | `POST /api/chapters/{id}/body/proposals/32856ec9-…/decision` 200 → 修订 2（`0f5f2e96`，4885 字）；工具结果带 `base_revision_id` / `result_revision_id` / `word_count`；`chapter_edit` review `1f6287ab approved/human` |
| 意见真的送达模型 | 同一 `feedback` 行转 `bound`→`included`（`included_request_id=1194969d`）；下一轮模型按意见把无线电段压到三句（seq 36），第二个候选 `ffc62001` → 接受 → 修订 3（`ef3be284`，4530 字），`chapter_edit 6ebcc43d approved/human` |
| 计数 | 全过程中该 Run 的 `consecutive_protocol_errors` 始终 0；期间一次 `ask_user` 提问与一次纯文本回合都被同一 Run 纠正 |

**人工编辑正文 + 再确认。** 一处界面事实先要说明：**正文编辑器只在「自动接受」模式下渲染**（`ChapterBodyPanel` 的条件 `approvalMode === "auto_approve"`），候选待决状态与整章审核状态都不渲染它。所以「人工改一段再走整章审核」的实际路径是：切回自动接受（`POST /api/versions/{id}/approval-mode {mode:"auto_approve"}` 200）→ 在「本章正文」里把「他关了台灯。宿舍陷入黑暗。」改为「他伸手关了台灯，宿舍陷入黑暗。」→ `POST /api/chapters/{id}/body/replace` 200 → **修订 4**（`0a2405b4`，4532 字，比修订 3 的 4530 字多 2 字；正文 4691 字符）→「提交整章审核」`POST /api/chapters/{id}/body/review` 200（review `34c8092a` open，`targetRevisionId=0a2405b4`）→「通过并定稿」`POST /api/reviews/34c8092a/decision passed:true` 200 → `writing_chapter_done`、`activeRunId=None`。章节 2 落库为 `approved`、`approvedRevisionId=0a2405b4`（人工那一版）、`bodyWordCount=4532`。人工链路不经过阶段提交，所以「提交被当成协议错误」在这条路上不存在；真正暴露它的是 43.4 b。

Run 收口：`735fc0e8 succeeded`、`request_count=17`、`resume_count=2`、`cpe=0`。

### 43.6 本轮确认的界面事实

- 正文编辑器只在自动接受模式可见；逐次确认里人工只能对候选作决定（接受／拒绝／委托审核），不能自己直接改正文。
- 整章审核状态下界面只提供审核决定与意见，不提供编辑器。
- 「退回修订」是唯一会把工作流退回 `writing_drafting` 的动作，而一进 drafting 就会派发模型请求（实测退回后 1.4 秒内 `request_in_flight`）；想人工接管必须先「停止执行」（本轮实测：退回 → 停止，未浪费整轮请求）。
- 审核阶段的外部入口在标题栏与面板两处都会出现同名按钮（「显式继续」出现两次），两次点击都命中同一 `POST /api/runs/{id}/continue`。

### 43.7 验证与未覆盖

`cargo test --workspace` **232 通过 / 0 失败**（agent 44、app 单元 61 + 集成 2、core 38、store 83、xtask 4），`cargo fmt --all -- --check` 干净。实例仍在 `127.0.0.1:8820`（数据目录 `/tmp/nw-e2e-chapter/data`），第 2 章已定稿、第 3 章仍 `planned`。

未覆盖：~~43.4 b 的真实端点复验~~（已于 §44.4 复验：守卫拒绝的 `run.changed` 不带计数与 `wait_reason`，原地重排）；~~43.4 a 的修法~~（已于 §44 收口并真实复跑）；~~第 3 章闭环~~（第 3 章的指导→细纲→正文→退回改写→定稿已于 §44 走完）；完本确认（第 3 章已定稿但未点确认完本）；长篇阅读视图与导出（本轮未导出）；`format=txt` 导出分支。

## 44. 验收：整章审核退回意见投递（审计发现 43.4 a 收口）

### 44.1 谁才是权威：先定语义，再决定动不动代码

审计给的前提是「界面标签与正文提示词都承诺这份意见会到达模型」。动代码前先把三处口径逐字读出来，确认这到底是被承诺的能力还是「退回 = 人工接管」的设计选择：

- **界面标签**（`webui/src/components/ChapterBodyPanel.tsx` 的整章审核意见框）：「退回时说明情节、叙事、人物或文本层面的修改要求」——只承诺「说明要求」，本身不指明由谁消费。
- **正文 system 提示词**（`chapter.write.user.md`）：「结合后续历史消息中的工具结果、审核意见和用户反馈」——明确承诺审核意见进入下一条正文请求。
- **docs/02 §6.2**：`passed=false` 的拒绝必须把 `issues` 交给写作 Agent。

三处里提示词与规格都明确承诺送达，界面标签也不与之冲突，所以权威在「送达」一侧：只把意见写进 `reviews.issues_json` 是缺陷，不是设计。修法因此限定为「让它走既有的投递通道」——不新增通道、不改界面文案、不改无关行为。

### 44.2 修复

`Store::decide_chapter_final_review`（`crates/nekowriter-store/src/lib.rs`）的拒绝分支此前只更新 Review 与工作流。现在它在同一事务里调用从 `Store::submit_run_feedback` 抽出的共享助手 `queue_chapter_body_feedback`（`crates/nekowriter-store/src/preparation.rs`）：意见按自由意见的批次锚点规则排进该章正文 Run 的未封存批次，沿用同一套 `queued → bound → included` 生命周期，并写一条 `feedback.changed`。把内联插入抽成助手而不是复制一段，是为了让「退回意见」与「意见框意见」共用同一份锚点规则（已有未封存批次就继承其锚点，否则挂到最后一条已有结果的工具调用之后）。

docs/02 §6.2 补一段规格，钉住「人工整章终稿拒绝的 `issues` 必须交给写作 Agent，走与意见框相同的通道；只写 `reviews.issues_json` 属缺陷」。

### 44.3 现场证据（真实界面，8820 / `/tmp/nw-e2e-chapter`）

对象是第 3 章 `daeec0cd-d6a2-45ba-ab52-265ae536efb3`「第三章「应答」」。界面在整章审核面板点「退回修订」，意见框写入（原文即后来落库的 `issues_json`）：

> 这一章的收束没有兑现承诺：林砚的「照三次」只写到伸手去按开关就结束了，没有写出对岸是否回应；请把结尾改成他真的做完三次长闪、并让他亲眼看到一个明确回应，再给出他离开或留下的不可撤销后果。另外第二段到第三段的时间跳跃没有交代，请补一句过渡。

命令：

```bash
DB=/tmp/nw-e2e-chapter/data/nekowriter.sqlite3
sqlite3 -header -column "$DB" "select seq, datetime(occurred_at/1000,'unixepoch','localtime') t, type, substr(entity_id,1,8) ent, substr(payload_json,1,95) payload from events where seq between 247 and 275 order by seq;"
```

事件与表行（`09:28:00` 同一秒：退回决定与意见入队同事务）：

| 事件 | 时刻 | 内容 |
| --- | --- | --- |
| seq 250 `review.changed` `ce6b6139` | 09:28:00 | `passed:false`、`status:rejected`（人工决定，`target_revision_id = 5cf6cc16`） |
| seq 251 `run.changed` `a6649032` | 09:28:00 | 正文 Run 回到 `queued` |
| **seq 252 `feedback.changed` `742e31bc`** | **09:28:00** | **`status:queued`——退回的同一秒意见已入队，不再是 0 行** |
| seq 266 `feedback.changed` `742e31bc` | 09:29:51 | `bound_tool_call_id = 2b4274d5`（`replace_content`） |
| seq 270 `feedback.changed` `742e31bc` | 09:29:51 | `included_request_id = f9b07ba1` |
| seq 273 `document.committed` `8b60478c` | 09:30:15 | 新修订，`word_count = 4484` |

两侧报文核对（不是只看返回码）：`feedback.text` 与 `reviews.issues_json` 逐字一致；请求 `f9b07ba1` 的 `prompt_snapshot_json` 与 `context_snapshot_json` 都含意见正文：

```bash
sqlite3 "$DB" "select case when prompt_snapshot_json like '%照三次%' then 'prompt ✓' else 'prompt ✗' end from model_requests where id like 'f9b07ba1%';"
# prompt ✓   （同一查询对 context_snapshot_json 亦为 context ✓）
```

第 3 章修订链（`document_revisions`，同一文档）：

```
b83b28a4  110   user   09:19:30   人工起始稿
fa747238  5726  agent  09:22:22   第一次正文提交
5cf6cc16  4447  agent  09:24:20   ← 被 ce6b6139 退回的那一版
afd904be  4445  agent  09:29:51   ← 意见绑定的工具结果（2b4274d5）
8b60478c  4484  agent  09:30:15   ← 携意见的请求 f9b07ba1 的产物，被 49612aa9 通过
```

修复前 / 修复后对照，关键判据是「下一次整章审核的目标修订是否变了」：

| | 修复前（§43.4 a 现场） | 修复后（本轮） |
| --- | --- | --- |
| 退回产生的 `feedback` 行 | 0 行 | `742e31bc` `queued → bound → included` |
| 意见进入模型请求 | `model_requests`/`agent_messages`/`prompt_overrides` 各 0 次命中 | `f9b07ba1` 的 prompt 与 context 均命中 |
| 下一次整章审核 `target_revision_id` | `c6c01de4`/`4ae3627a` 都是 `99ddc296`（与第一次相同） | `49612aa9` = `8b60478c`，与退回时的 `5cf6cc16` 不同 |
| 正文是否变化 | 一个字符都没变 | 新修订落库（`4484` 字，`afd904be 4445` 之后） |

Run 收口：`a6649032` `succeeded`、`request_count = 7`、`resume_count = 0`、`consecutive_protocol_errors = 0`。

### 44.4 顺带收口：43.4 b 也在真实端点复验到了

同一个 Run 里 §43.4 b 修的「未投递意见守卫」真实触发了一次：请求 `9c1c06ab`（`09:28:44`）是模型在意见尚未投递时的直接提交，写回 `reason = pending_feedback_requires_tool_result` 并**原地重排**（seq 260 `run.changed` `status:queued`），模型随后改用 `replace_content` 投递了意见。与同一 Run 里真正的协议错误对照（seq 256 `run.changed`，那条是纯文本回合的纠正）：

```text
seq 256  {"consecutive_protocol_errors":"1","kind":"chapter_body","request_count":"3","request_id":"9c183ce7-…","status":"queued","wait_reason":null}
seq 260  {"chapter_id":"daeec0cd-…","kind":"chapter_body","reason":"pending_feedback_requires_tool_result","request_count":"4","request_id":"9c1c06ab-…","status":"queued"}
```

守卫拒绝的载荷里**既没有 `consecutive_protocol_errors` 也没有 `wait_reason`**，真正的协议错误两者都带——这正是修复后的行为特征（对照 §43.4 b 记录的修复前 event 171，它带着 `consecutive_protocol_errors:3` 与 `wait_reason:protocol_error_limit`，并把 Run 收束为 `stopped`）。本次 Run 最终 `succeeded`、`cpe = 0`。因此 §43.7 里「43.4 b 只有 store/app 级测试」这一条也随之划掉：真实端点已复验（触发一次而非连续三次，但行为特征——不带计数、不带 `wait_reason`、原地重排——已在真实事件上出现）。

### 44.5 测试

- `crates/nekowriter-app/src/main.rs`：`review_decision_accepts_the_payload_the_ui_sends` 钉住界面实际发出的 `POST /api/reviews/{id}/decision` 请求形状（`passed:false` + `issues` 字符串；通过时 `issues:""` + `citations:[]`），防止再出现「单测全绿、界面 422」那类形状漂移。
- `crates/nekowriter-store/src/lib.rs`：`chapter_final_rejection_hands_its_issues_to_the_body_agent` 钉住整条链：拒绝后 `pending_feedback` 出现一条、`text` 与 `issues` 一致、`queued → bound → included` 三步齐全、`included_request_id` 落到真实请求上。

### 44.6 验证与未覆盖

`cargo test --workspace` **234 通过 / 0 失败**（agent 44、app 单元 62 + 集成 2、core 38、store 84、xtask 4），`cargo fmt --all -- --check` 干净。审计发现 43.4 a 按「已收口」处理，43.4 b 的真实端点复验一并完成。

仍未覆盖：本轮证明的是「意见进入了请求」，不是「模型一定按意见改稿」——本轮它确实改了（新修订被通过），但这取决于模型；`issues` 为空的拒绝（界面允许留空）只由 §6.2 的既有协议处理，没有现场；完本确认、长篇阅读视图、`format=txt` 导出仍未走。

## 45. 2026-09-15 修复轮收口：静态/`/api` 边界、幂等收尾统一、前端测试基建与真界面验收

本轮把第 05 号全仓复核报告的修复项做到「有现场」为止，并为三处此前只有单测或静态检查的行为补上真实验证。

### 45.1 静态目录与 `/api` 的边界（修复前 → 修复后）

现场是当前二进制加全新数据目录（`target/debug/nekowriter --port 8841 --data-dir /tmp/nw-route2/data`）：

| 请求 | 修复前 | 修复后 |
| --- | --- | --- |
| `GET /api/definitely-not-a-route` | 404 `application/json`，`{"error":{"code":"NOT_FOUND","message":"接口不存在",...}}` | 同左——`/api` 有自己的嵌套回落，没有被静态回落吞掉 |
| `GET /settings`（裸路径） | 404，空响应体 | 200 `text/html`，入口 HTML |
| `GET /assets/removed-chunk.js` | **200 `text/html`，入口 HTML** ← 缺陷 | 404 |

「修复前」不是回忆：先把回落临时换回 `ServeFile::new(index.html)` 再跑本轮新加的用例，得到
`assertion left: 200 right: 404: 缺失的静态资源被回落成了 HTML`；再退一步（完全没有回落）得到
`/settings 未回落到入口 HTML: left: 404 right: 200`。两次都跑在真实回环端口上。

**需要更正我自己先前的一处口径**：前端用的是 hash 路由（`webui/src/App.tsx` 的 `createHashRouter`），
刷新 `http://127.0.0.1:8851/#/settings` 时服务端只看到 `/`，所以「刷新深链接」并**不**需要这个回落。
它的实际价值只是裸路径（在地址栏敲 `/settings`）不再得到空 404；而它最初引入的真正代价是
**把所有未命中的路径都变成 200 + HTML**，包括缺失的静态资源——浏览器会把 HTML 当脚本执行（MIME 不匹配），
把「chunk 丢了」这种可诊断的 404 变成一条难定位的界面故障。

收口在 `crates/nekowriter-app/src/main.rs`：`fallback_service` 改成「末段不带 `.` 的路径才回落到入口 HTML，
带扩展名的路径如实 404」（`serve_client_route` / `is_static_asset_path`）；`/api` 保留自己的嵌套回落。

钉住真实形状的用例是 `static_dir_serves_client_routes_and_reports_missing_assets_honestly`（真实回环端口 + 真实路由 + 真实 HTTP）：
`/`、`/settings`、`/works/abc/versions/def` → 200 且 `text/html`、含入口标记；磁盘上存在的 `/assets/app.js` → 200 且是磁盘内容；
缺失的 `/assets/removed-chunk.js` → 404；`/api/definitely-not-a-route` → 404 且 `content-type: application/json`、`error.code = NOT_FOUND`。
同一个文件里，原本只覆盖 `Origin` 的路由级写信任用例补上了 `Host: evil.example`：403 的 `UNTRUSTED_ORIGIN` 与 400 的 `UNTRUSTED_HOST`（400 是该错误码既有的状态码映射）。

### 45.2 幂等收尾统一（复核 F-05 收口）

复核时的计数是「`finish_write` 只有 17/49 调用点」。统一第一轮把剩下 32 个站点迁到同一个收尾函数，
本轮（收口后的精简轮）再把这套东西换成一个真正持有状态的**幂等信封**，调用点因此不再复述它：

```rust
struct Command<'a> { id: Uuid, method: &'a str, target: &'a str, payload_hash: &'a str }
enum Notify { Listeners, Quiet }   // 取代 `notify: bool`

Command::replay_if_recorded(store)          // 事务外预查：已有回执就还原，指纹不符是冲突
Command::commit(store, notify, body)        // 预查 → 事务体 → 收尾，一条命令只写一行
Command::finish(store, notify, result)      // 只给「按事务结果决定要不要通知」的站点用
```

现况计数（`crates/nekowriter-store/src/*.rs`）：

| 项 | 数 | 分布 |
| --- | --- | --- |
| 命令站点 | **49** | `preparation` 24、`knowledge` 9、`recycle` 5、`lib` 3、`prompt_templates` 3、`prompts` 2、`version_copy` 1、`settings` 1、`project_settings` 1 |
| 其中走 `commit`（三段合一） | **46** | 通知判定在提交前就已知 |
| 其中走 `finish` + `Notify::on_change` | **3** | `set_approval_mode`、`request_chapter_body_pause`、提示词覆盖项重置：要不要通知由事务结果决定 |
| 只做预查的 helper | **3** | `replay_system_model_settings_if_known`、`replay_embedding_settings_if_known`、`find_accumulation_result` |
| 站点自己先预查一次（白名单） | **5** | `create_knowledge_entry`、`insert_accumulated_entries`、`update_knowledge_entry`、`delete_knowledge_entry`、`reindex_all_knowledge`：早退省掉 `embedding_profile_with_id()` 的读库（`reindex` 还省掉整张 `knowledge_entry` 表扫描） |
| `finish_write` 自由函数 | **0** | 已删除（定义与全部调用点） |

**为什么三个站点不硬塞进 `commit`：** 它们的 `notify` 来自事务体的返回值（`changed` / `removed`），
提交之前根本无从判断。强行合并得让 `commit` 认识「结果里第二个元素是 bool」这种约定，
那是把一处清晰的判断换成一个更隐晦的约定，因此这三个特意保留两步写法，只把通知判定换成了
`Notify::on_change(matches!(&result, …))`——语义与原来逐字相同，名字替 `notify` 说出了含义。

**为什么站点还留着自己的事务体：** 回执必须与业务写入在同一个事务里提交，`insert_receipt` 正是
那个事务的一部分，`commit` 的 `body` 就是它，边界在调用点仍然看得见；把它也藏进 helper 只会让
事务边界消失在两次调用之间。并发的重复命令由 `command_receipts.command_id` 主键冲突挡住，错误在
`finish` 里换成既有回执。

> 订正：此前这里（以及 `docs/02` §2.4、`docs/06` 的 F-05 行）写成「算指纹 → **事务内复查** → 写回执」，
> 把原因说成「复查与业务写入必须在同一个事务里」。代码里没有、也不需要事务内的回执复查：事务体里只有业务写入
> 与 `insert_receipt`，事务外的预查由 `Command::commit` 独占，并发正确性由主键冲突兜底。已按实现订正。

仍然直接调用 `notify_event_listeners()` 的地方只剩不属于命令写入的内部尝试记录：
`preparation` 8 处（Agent 工具调用落库、模型请求终态）、`model_requests` 6 处、
`knowledge` 1 处（索引任务状态）、`protocol_correction` 1 处，加上信封自己那 1 处。
它们都没有 `command_id`、也不写回执，不在幂等规则之内。

**行数与形状（真实的 `wc -l` 对照，§45.2 时点前后各量一次；右列现在等于 `HEAD`）：**

```
knowledge.rs        2808 → 2704  (-104)
lib.rs              6928 → 7025  (+97)     ← 信封本体 + 两个新增用例 + 钉住用例在这里，站点侧是净减
preparation.rs     13522 → 13218 (-304)
project_settings.rs  568 →  553  (-15)
prompt_templates.rs  481 →  438  (-43)
prompts.rs           850 →  830  (-20)
recycle.rs          1330 → 1260  (-70)
settings.rs          499 →  482  (-17)
version_copy.rs     1077 → 1062  (-15)
合计               28063 → 27572 (-491)
```

净减 **491 行**：信封重构 484 行 + 本轮预查去重 7 行（去重删掉 41 处纯重复预查共 123 行，
保留站点的注释与钉住用例占回 116 行）。重构前那 49 个 `notify` 参数是 **46 个字面 `true` + 3 个计算值**
（`notify`、`reset.removed`），现在全部消失：46 处写 `Notify::Listeners`，3 处写 `Notify::on_change(...)`——
调用点不再能传错一个布尔值。

**该表的口径与时点（事后标注）。** 这张表是 §45.2 时点的测量，不是当前值，也不能与 `HEAD` 互相代入：

- **右列现在等于 `HEAD`**（`873bed4`，就是收下本批的那个提交）逐文件一致，只有 `lib.rs` 差 2 行（提交时又多了 2 行：7025 → 7027，合计 27572 → 27574）。
- **左列已不可重测**：它是当时的中转工作树（复核整改已应用、信封尚未落地），而 `HEAD~1`（`a8374ff`）是**整批整改之前**的提交，同 9 个文件合计 **27766** 行，与左列的 28063 不是同一个状态。本节保留左列原值，不反推。
- **当前树**（§46 + §47 之后）同 9 个文件：`knowledge 2704`、`lib 7443`、`preparation 13205`、`project_settings 553`、`prompt_templates 438`、`prompts 830`、`recycle 1257`、`settings 481`、`version_copy 1061`，合计 **27972**；相对 `HEAD` 净增 **+398**（`lib.rs` +416，主要是 §47 的两个用例与两个测试辅助；其余四个文件是 §46 的 lint 机械项，−18）。所以**「净减 491 行」只是信封收口那一轮相对它自己左列的净额**，不是本批相对 `HEAD~1` 的净额。

**"等价"这一声称里唯一不成立的一处已单独钉住。** 收尾的失败分支在复核时是
`Err(StoreError::Database(error)) => 按回执重放`（其余错误原样返回），统一后放宽为「任何错误都先查回执」。
它今天只能由**并发或构造**真实抵达（两次预查之间另一条同 `command_id` 的命令已落库，而输的那次先撞上业务守卫），
因此不靠现场而靠用例钉住它的契约与边界：`command_finish_replays_an_existing_receipt_for_a_non_database_error`
（用一条真实命令留下真实回执，再以 `SystemSettingsRevisionConflict` 这条非数据库错误调用收尾）。
把分支改回 `matches!(error, StoreError::Database(_))` 后它立即失败：

```
thread 'tests::command_finish_replays_an_existing_receipt_for_a_non_database_error' panicked at
crates/nekowriter-store/src/lib.rs:2048:10:
已有回执时，非数据库错误也必须还原成回执: SystemSettingsRevisionConflict { expected: 0, actual: 1 }
test result: FAILED. 0 passed; 1 failed; 0 ignored; 91 filtered out
```

安全性由 `replay_after_receipt_conflict` 自身的边界保证：无回执时原样返回传入的错误，
有回执但方法/目标/指纹不一致仍是 `IdempotencyConflict`（两者都在同一用例里断言）。
`docs/06` §3.1 把这条记为本轮**唯一有意的非等价改动**及其理由。

**通知判定的新形状不是「编过就算」：两次反向实验证明它真被用例钉住。**

新用例 `a_no_op_command_writes_its_receipt_but_does_not_wake_subscribers`：把同一个模式再写一遍
（no-op 命令），断言它照样落回执、但不唤醒订阅者。

| 实验 | 改动 | 结果 |
| --- | --- | --- |
| A | 把 `set_approval_mode` 的 `Notify::on_change(…)` 改回无条件 `Notify::Listeners` | `a_no_op_command_writes_its_receipt_but_does_not_wake_subscribers` 失败于 `lib.rs:2084`：`assertion failed: matches!(notifications.try_recv(), Err(… TryRecvError::Empty))`（`0 passed; 1 failed`） |
| B | 把 `save_novel_brief` 站点的 `Notify::Listeners` 改反成 `Notify::Quiet` | `persisted_events_are_ordered_and_wake_subscribers` 失败于 `lib.rs:4481`：`assertion failed: notifications.try_recv().is_ok()`（`0 passed; 1 failed`） |

两次都随即还原（`rg -c "Notify::on_change" preparation.rs` 回到 2、`rg -c "Notify::Quiet" preparation.rs` 为 0）。
也就是说：这一轮既证明了 46 处常量通知不会被悄悄改成不通知，也证明了那 3 处「按结果决定」不会被悄悄改成无条件通知。

**事务外预查收敛到唯一所有者（收口轮）。** 信封落地后，46 个 `commit` 站点里曾有 41 处又自己写了
一遍同样的调用（`if let Some(replayed) = command.replay_if_recorded(self).await? { return Ok(replayed); }`），
而 `commit` 开头就会查回执；两边之间什么都没有的那 41 处因此是把同一次点查做了两遍。现在：

* 删掉 41 处纯重复预查（`preparation` 22、`recycle` 5、`knowledge` 4、`lib` 3、`prompt_templates` 3、
  `prompts`/`settings`/`project_settings`/`version_copy` 各 1；共 123 行）。
* 保留 5 处并把理由写在代码里（表格里的白名单）：它们后面紧跟着 `embedding_profile_with_id()` 的读库，
  `reindex_all_knowledge` 还跟着整张 `knowledge_entry` 表扫描，早退在重放时省掉的是真实计算。
* 3 个只看事务结果决定通知、收尾走 `finish` 的站点（`finish` 不做预查）与 3 个只做预查的 helper
  （路由层的幂等出口，在任何真实工作之前结束请求）本来就不在这批里。

**这条不变式靠源码形状钉住，因为多出来的那次查询在运行期不可观察**（结果被原样丢掉，重放语义不变）。
新用例 `transaction_external_recheck_has_a_single_owner` 扫描 `store` 里九个生产文件，断言「既预查又 `commit`」
的函数集合恰好等于那五个白名单。两个方向的故意反向实验都让它真的失败：

| 实验 | 改动 | 结果 |
| --- | --- | --- |
| C | 给 `project_settings::save_project_settings` 把删掉的预查加回去 | `left` 多出 `save_project_settings`，失败于 `lib.rs:2256`（`0 passed; 1 failed`） |
| D | 把白名单里的 `reindex_all_knowledge` 预查删掉 | `left` 少了 `reindex_all_knowledge`，失败于 `lib.rs:2256`（格式化前为 `2253`，同一断言） |

运行期次数改用 SQL trace 现场对账（`RUST_LOG=info,sqlx=trace`，全新数据目录）：

| 构建 | 成功的 `PUT /api/settings`（新命令） | 同一 `commandId` 重放 |
| --- | --- | --- |
| `HEAD`（`a8374ff` 源码构建的二进制，8876） | 写前查 **2** 次 `command_receipts`（`.112862`、`.113219`：路由 helper + 站点预查） | 1 次 |
| 去重前（信封重构后、本次去重前的二进制，8873） | 写前查 **3** 次（`.325329`、`.325735`、`.326037`：路由 helper + 站点 + `commit`） | 1 次 |
| 现在（8878，收口后的当前构建） | 写前查 **2** 次（`.931124`、`.931704`：路由 helper + `commit`） | 1 次 |

即：写命令的回执查询次数与 HEAD 完全一致，多出来的那一次已经还回去；写入序列本身（`.932375` 起：
事务内读设置 → `INSERT system_model_settings` → 事件 → `INSERT command_receipts` → 框架回读）与两条构建逐条同形。

```
PUT /api/settings  commandId=…  expectedRevision=0  apiKey=sk-final-first   → 200
PUT /api/settings  commandId=…（同一个）  apiKey=sk-final-second（只换密钥）  → 200，响应逐字节相同

command_receipts      : PUT /api/settings × 1，payload_hash=2493591dd8…（与上一轮同请求同值）
events                : settings.changed × 1
system_model_settings : revision=1，密钥仍是第一次那一个
rg sk-final 于 服务日志 / payload_hash → 0 命中
```

（HEAD 那次对照有一个细节：它还没有「密钥不进指纹」的修正，所以同一 `commandId` 只换密钥会得到
409 `IDEMPOTENCY_CONFLICT`；为了只比查询次数，HEAD 侧用的是逐字相同的请求体两次。）

**真实现场（这条路径在每一条命令底下，所以重跑一遍幂等）。** 用当前源码构建的二进制、全新数据目录
（`target/debug/nekowriter --port 8866 --data-dir /tmp/nw-envelope-live/data --no-browser`）：

```
PUT /api/settings  commandId=2fe9963f-…  expectedRevision=0  apiKey=sk-env-first   → 200  revision=1
PUT /api/settings  commandId=2fe9963f-…（同一个）  apiKey=sk-env-second  → 200  响应与上一条逐字节相同

system_model_settings : revision=1  model_id=envelope-live  key_set=1  key_len=12（仍是 first）
command_receipts      : PUT /api/settings × 1
events                : settings.changed × 1
rg sk-env 于 payload_hash / 服务日志 → 0 命中
```

与重构前那次现场（§45.5）逐项相同：一条回执、一条事件、`revision` 不增、第二次的密钥正文既不入库也不入摘要。

### 45.3 前端测试基建与 CI（复核 F-13、F-11）

引入 vitest 5.0.1（`webui` 的 devDependency，`pnpm test` 即 `vitest run`）。配置只跑纯逻辑、
`environment: "node"`、不加载 React/Tailwind 插件：`webui/vitest.config.ts`。

| 用例 | 钉住的东西 |
| --- | --- |
| `api.test.ts` × 7 | 错误信封 → `ApiError`（`code` / `message` / `details` / `requestId`）；响应体不是信封（代理换成 HTML、502）时仍给出「本地服务返回 502」+ `HTTP_ERROR` 而不是 `undefined`；读请求不重试、写请求用逐字相同的请求体重试一次；命令请求重试用同一个 `commandId`；保存模型设置时省略密钥就是省略（请求体里没有 `apiKey`） |
| `shortcuts.test.ts` × 3 | 中文输入法组合判定：`isComposing`、`keyCode === 229`（确认候选词那一次按键）、普通 Enter |

实际运行：`pnpm test` → **2 个文件 / 10 个用例通过**；`pnpm lint` → **0 error / 2 warning**（两个都是
`react-hooks/incompatible-library`：`ChapterDirectory.tsx` 与 `ManuscriptReader.tsx` 使用虚拟化库，
属于该规则的已知边界，已记入 §16.1 一类的已知项）；`npx tsc --noEmit` 与 `pnpm build` 干净。

CI 是 `.github/workflows/ci.yml` 两个 job：`rust`（装 `protobuf-compiler`、按 `rust-toolchain.toml` 装 nightly+clippy+rustfmt、
`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets`、`cargo test --workspace`）与
`webui`（`pnpm install --frozen-lockfile`、`pnpm lint`、`pnpm test`、`pnpm build`）。
本地等价跑法逐条执行且全绿（见 45.6）。YAML 用 `ruby -ryaml` 解析确认结构（`on` 在 YAML 1.1 里是布尔键，
解析出的 `true` 就是它）：「`job rust: [checkout, 安装 protoc, 安装 toolchain, rust-cache, 格式, Clippy, 测试]`、
`job webui: [checkout, pnpm/action-setup, setup-node, 安装依赖, lint, 测试, 构建]`」。没有在 GitHub 上实跑过——
这是本机没有远程执行器，不是没配。

### 45.4 真界面验收（全新数据目录 `/tmp/nw-ui`，`webui/dist` 为当前构建）

四个能力都在真实浏览器里点过、并用数据库/日志对账：

1. **快捷键说明**（复核 F-01 丢失能力之一）：工作台/任何页头部的「快捷键」按钮点击后 `aria-expanded="true"`，
   页面出现 `dialog "快捷键"`，含 5 条绑定（`⌘ + Enter` ×2、`⌘ + K`、`Esc`、`?`）与输入法说明，`关闭` 按钮可关。
2. **本地服务面板**（F-01 丢失能力之二，挂在系统设置页）：显示「界面地址 `http://127.0.0.1:8851`」与
   「数据与日志 `/private/tmp/nw-ui/data`」；点「退出服务」后按钮变为「确认退出 / 取消」，并给出
   「退出会先停止接受新请求，取消在途的模型调用，再把正在执行的 Run 收束为「已中断」。待审提案、已提交正文与未投递意见都会保留。」
   这一步同时证明了这个按钮不是死按钮（它依赖恢复回来的健康检查状态）。
3. **清除密钥**（复核 F-16 新增能力）：在设置页填 `模型 ID = dummy-model`、`API 密钥 = sk-should-be-cleared` 并保存 →
   界面「凭据已配置」、DB `revision=1 / api_key 长度 20`；点「清除密钥」第一次只是把按钮变成「确认清除密钥」
   （DB 仍是 `key_is_null=0`，两步确认有效），再点一次 → 界面回到「尚未配置」且「清除密钥」按钮消失，
   DB `revision=2 / api_key IS NULL`、`model_id` 与 `api_base_url` 原样保留、事件 `settings.changed @rev2`；
   服务日志与 `logs/` 里 `rg sk-should-be-cleared` **0 命中**。
4. **「本地服务已退出」横幅与面板状态**（`docs/02` §16.1 里原先写着「只有静态检查」的那一条）：预览桥绑在服务进程上，
   服务一停预览就断，所以这一条以前截不到。本轮把页面接到一个只做字节转发的本地 TCP 端口（8860 → 8851），
   预览绑在转发器上，然后真的 `kill -TERM` 后端：页面仍在，读到
   `role="status"`「本地服务已退出」+「下方内容是本机最后读取到的数据，不会继续更新，也暂时无法提交新操作。重新运行 NekoWriter（发行目录里的 nekowriter，或源码目录里的 cargo run -p nekowriter-app）后刷新此页面即可继续。」，
   面板同步变成「服务已退出，界面不再接收新事件」且退出按钮消失（没有留下死按钮）。后端日志侧是
   `收到退出信号，正在停止服务 → 正在取消在途请求并收尾状态 → NekoWriter 已退出`。

### 45.5 密钥不进回执摘要的现场（复核 F-15）

在同一个数据目录上：第一次 `PUT /api/settings`（新 `commandId`、`apiKey=sk-a-first`）→ 200、`revision=3`；
紧接着用**同一个 `commandId`、只把密钥换成 `sk-b-second`** → 也返回 200（重放既有回执），
DB 里 `revision` 仍是 3、`api_key` 仍是 `sk-a-first`，`command_receipts`（`target=/api/settings`）与 `events` 都**没有**增加。
这同时钉住两件事：密钥字段不进 `payload_hash`（否则同 ID 异体会是 409 冲突），以及重放不会重复写入、不会重复发事件。
「没有清除密钥入口」这一项已由 45.4 的第 3 条关闭。

本轮收口时又在一个**全新数据目录**上用当前二进制重跑了一遍同一条现场
（`target/debug/nekowriter --port 8864 --data-dir /tmp/nw-replay/data --no-browser`，前端产物为当前构建）：

```
PUT /api/settings  commandId=546ee42d-…  expectedRevision=0  apiKey=sk-live-first   → 200
  {"writingModel":{… "revision":"1", "credentialConfigured":true, …}}
PUT /api/settings  commandId=546ee42d-…（同一个）  apiKey=sk-live-second（只换密钥）  → 200
  与上一条响应逐字节相同（diff 无输出）

system_model_settings : revision=1  model_id=live-model  api_base_url=https://api.example/v1  key_set=1  key_len=13（仍是 first）
command_receipts      : PUT /api/settings × 1，payload_hash 长度 64
events                : settings.changed × 1
rg sk-live 于 payload_hash / 服务日志 / data/logs  → 0 命中
```

即：同一 `commandId` 两次 → **一条回执、一条事件、`revision` 不增**，第二次的密钥正文既没进库也没进摘要。

### 45.6 门禁与仍未覆盖

```
cargo fmt --all -- --check                     干净
cargo clippy --workspace --all-targets         0 error；warning 行 239（比复核基线 244 少 5 行，未新增）
cargo test --workspace                         249 通过 / 0 失败（agent 44、app 单元 67、app 集成 2、core 38、store 94、xtask 4）
webui: pnpm lint                               0 error / 2 warning
webui: pnpm test                               10 通过 / 0 失败
webui: npx tsc --noEmit && pnpm build          干净
```

**注（事后标注）**：这张表、§45.2 的行数表与 §45.3 的前端 lint 数字都是 §45 那一轮（2026-09-15）的现场数字，已被 §46 取代——
`clippy` 的 239 行 warning 在 §46 清零（逐包与 `--workspace --all-targets` 现均为 0 error / 0 warning）；
`cargo test --workspace` 的 249（store 94）现在是 §46.5 / §47.4 记的 **253 通过 / 0 失败（本仓 251，store 96）**；
前端 `pnpm lint` 的 2 条 warning 也在 §46.4 按文件收窄为 0。本仓内的历史记录一律保留不改，只在被取代处标注。

仍未覆盖：CI 未在 GitHub 上实跑（只是本地等价命令全绿 + YAML 解析通过）；hash 路由下裸路径回落的行为只由路由级用例与
curl 现场覆盖，没有用真实浏览器敲一次地址栏；删除密钥后 Embedding 侧的同一动作只在 store/HTTP 用例里（本机没有可用的
Embedding 端点的实测见 §40）；`.agents/skills/**` 移出版本控制只改了索引与 `.gitignore`（文件仍在磁盘上），需要维护者在提交时确认。

## 46. 2026-09-15 clippy 与前端 lint 清零：机械项逐条修掉，结构性项写成明确政策

本轮只动 lint，不改行为：`cargo clippy` 从 414 条（230 个唯一站点）降到 **0**，`pnpm lint` 从 2 条降到 **0**。

### 46.1 先量后动：基线构成

```
cargo clippy -p nekowriter-core -p nekowriter-agent -p nekowriter-store -p nekowriter-app --all-targets --message-format=json
→ 414 条（同一站点在 lib 与 test 两个目标下各计一次），按唯一站点去重 230：

  75 too_many_lines         60 missing_errors_doc        38 needless_update
  13 needless_pass_by_value  9 cast_possible_wrap         6 unused_async_trait_impl
   6 too_many_arguments      5 missing_panics_doc         4 match_same_arms
   3 struct_field_names      2 format_push_string        1 assert_is_empty
   1 similar_names           1 trivially_copy_pass_by_ref
   1 items_after_statements  1 cast_possible_truncation
   1 large_enum_variant      1 map_unwrap_or             1 suspicious_open_options
   1 match_wildcard_for_single_variants
```

`--workspace` 当时无法解析：另一个线程正在同一 checkout 里新建 `nekowriter-prompt-lab`，它的 `Cargo.toml` 已进 members
而 `src/main.rs` 还不存在。所以本轮所有命令都用 `-p` 逐个指定包（六个包全部跑到，`-p xtask` 单独一次）。

先试过一次 `cargo clippy --fix`：它只应用了 1 处，且把 `assert!(x.is_empty())` 改写成 `assert_eq!(x, [] as [T; 0])` 这种更难读的
形式，已还原，改为逐类手改。

**口径更正**：本轮开始前我口头给过一个「148 条」的数字，那是 `--all-targets` 下同一站点在 lib 与 test 两个目标里各计一次的
重复计数，不是待修项。真实的待修项就是上表按唯一站点去重后的 **75 个**，本轮全部改掉（46.2 的表按 lint 分组，合计 75），
没有任何一项被留成政策；政策收的是 155 个结构性 / 文档性项（46.3）。

### 46.2 修掉的 75 个（外加本轮改动自身带出的 6 个）

| lint | 站点 | 改法 |
| --- | --- | --- |
| `needless_update` | 38 | 删掉 `ActiveModel { …, ..Default::default() }` 里冗余的那一行：所有列都已显式 `Set`，`Default` 影响不到任何字段 |
| `needless_pass_by_value` | 13 (+1) | 签名改收引用并同步全部调用点；`router(web_dir)` 与测试辅助 `spawn_app_with_web_dir` 连调用侧那次 `clone()` 一并去掉 |
| `cast_possible_wrap` / `cast_possible_truncation` | 10 | 新增 `core::count_as_i64`（`TryInto` + 饱和到 `i64::MAX`）作为「内存计数 → `i64` 列」的唯一出口，替换 9 处 `as i64`；测试里的常量转换改 `usize::try_from(…)` |
| `match_same_arms` | 4 | 合并同体分支（HTTP 错误映射、阶段序号） |
| `format_push_string` | 2 | 改 `write!`（percent 编码、反馈拼接） |
| `assert_is_empty` | 1 (+1) | 按 pedantic 要求改成等值断言（其中 1 处由本轮改动自身带出） |
| `similar_names` | 1 | 测试里 `replaced`/`replayed` 只差一个字母，前者改 `recorded` |
| `trivially_copy_pass_by_ref` | 1 | `approval_mode_name(&ApprovalMode)` 改按值 |
| `items_after_statements` | 1 | 测试内的 `functions` 辅助函数提到模块级 |
| `large_enum_variant` | 1 | `LaneCommand::ModelCompleted.result` 装箱 |
| `map_unwrap_or` | 1 | `.map(…).unwrap_or(…)` 改 `.map_or(…)` |
| `suspicious_open_options` | 1 | 进程锁文件显式 `.truncate(false)` |
| `match_wildcard_for_single_variants` | 1 | 测试里点名唯一变体 |
| `needless_borrow` | 4 | 本轮改动带出的多余取地址，回头删掉 |

**这一行曾与上表对不上账，在此选定一侧并留下依据。** 本小节初稿把 `assert_is_empty` 记成 `2` 且未加标注，于是“表内合计 80”
与标题的“75 + 5”只能靠“基线就是 2”自圆；但 §46.1 把该 lint 的基线记为 **1**，而 §46.1 只有按 1 算才自洽（20 条合计恰好 **230**，
减去政策六类 **155** 后机械项恰好 **75**）。两侧差且只差这一项。**基线现场已不存在，无法重测**，因此按“可验证的那一侧”
定为：`assert_is_empty` = 基线 **1**、本轮自身带出 **1**，标题的自身带出总数相应为 **6**（`needless_pass_by_value` 1 + `assert_is_empty` 1
+ `needless_borrow` 4）。按此口径可以对着表重算：站点列合计 **79** ＋ 两处 `(+1)` 标注 **2** ＝ **81** ＝ 基线 **75** ＋ 自身带出 **6**。
若日后能从提交历史里重算基线，以重算结果为准。

行为不变的依据：该时点本仓全部 249 条用例在改后仍全绿（当前树的 251 + 另一线程的 2 = **253**，口径见 46.5），
且上面每一类都是等价改写——`as i64` 换饱和转换只在「字符数超过 `i64::MAX`」这种不可能出现的值域上有差别，
其余是删除冗余、收引用与合并同体分支。

### 46.3 剩下 155 个写成政策：4 类按批准，2 类按证据就地收窄

这 155 个只能靠拆文件或补模板化文档消掉，因此逐条写成带理由的 `allow`，理由就写在配置里。
**这里要把集合口径说清**：政策只覆盖方案里提出的 **4 类**（`too_many_lines`、`too_many_arguments`、`missing_errors_doc`、
`missing_panics_doc`），写在 `Cargo.toml` 的 `[workspace.lints.clippy]` 里；另外 **2 类**（`unused_async_trait_impl`、
`struct_field_names`）是我在实施中按证据就地收窄的，不在批准的四类之内——**这是一次 4 → 6 的口径扩张**（本小节即更正，
此前把它笼统写成「剩下 155 个就是这六类」，读者看不出这层差别）。

- `too_many_lines`（75）+ `too_many_arguments`（6）：全部落在 `preparation.rs`/`lib.rs` 的阶段命令体与它们的 8 参签名上，
  与「拆分 store 巨型文件」（`docs/06` F-09，本轮仍搁置）是同一件事；为过阈值把参数打包成只此一处使用的结构体只会多一层跳转。
- `missing_errors_doc`（60）+ `missing_panics_doc`（5）：为 65 个函数补模板化段落会稀释函数上方已有的注释；
  失败路径由 `StoreError`/`SchedulerError` 的变体承担，调用方与实现同仓。
- `unused_async_trait_impl`（6）+ `struct_field_names`（3）：两条都指向 sea-orm 的框架约定——`#[sea_orm::model]` 生成的 impl
  不含 `.await`，`Model` 这个约定名必然与 `model_id` 这类列名相撞，而实体字段名即数据库列名，改名等于改 schema。
  因为触发点只在这套实体定义上，这两条**没有**放进 `Cargo.toml`，而是收在
  `crates/nekowriter-store/src/entity/mod.rs` 的模块级 `#![allow(…)]` 里；收窄的承重性与「范围外仍会提示」两侧证据见 46.6.2。
  粒度选模块级而不是逐文件/逐结构体：这两条的触发点分散在 6 个实体文件里，而这套约定（实体字段名即列名）是实体注册表整体的事，
  写 6 遍只会多出 6 处需要同步维护的抑制。

#### 46.3.1 仓库里现存哪些抑制（单一完整清单）

本批之后的**全部**抑制如下；「本批新增」指 §46 这一轮加上去的，其余在本批之前就已存在（用 `git show HEAD:<file>` 逐文件核对过）。

| 范围 | 抑制 | 站点 |
| --- | --- | --- |
| workspace `[workspace.lints.clippy]` | `too_many_lines`、`too_many_arguments`、`missing_errors_doc`、`missing_panics_doc` | `Cargo.toml`（**本批新增 4 条**） |
| workspace `[workspace.lints.clippy]` | `module_name_repetitions` | `Cargo.toml`（既有） |
| 模块级 `#![allow(…)]` | `unused_async_trait_impl`、`struct_field_names` | `crates/nekowriter-store/src/entity/mod.rs:14`（**本批新增**，也是全仓唯一的 `#![allow`） |
| 项级 `#[allow(…)]` | `ref_option` ×3 | `agent/src/provider.rs:1361`、`core/src/lib.rs:64`、`store/src/model_requests.rs:1916`（均既有） |
| 项级 | `struct_excessive_bools` ×1 | `core/src/lib.rs:1399`（既有） |
| 项级 | `too_many_arguments` ×5 | `store/src/knowledge.rs:1329`、`store/src/preparation.rs:7983`、`store/src/version_copy.rs:999`、`app/src/scheduler.rs:2041`、`app/src/scheduler.rs:2150`（均既有） |
| 前端 eslint | `react-hooks/incompatible-library` 对 2 个文件 `off` | `webui/eslint.config.js`（**本批新增**，按 `files` 收窄） |

本仓自有 crate 的项级抑制共 **9 处**（`ref_option` 3 + `struct_excessive_bools` 1 + `too_many_arguments` 5），与 `HEAD` 逐个一致：
`git diff` 里唯一的 `+#[allow` 是上面那条 `entity/mod.rs`，没有任何 `-#[allow`。另一线程未跟踪的 `prompt-lab/src/main.rs:144` 还有一处
`struct_excessive_bools`，不属于本仓这批改动。

### 46.4 前端 2 条

`ChapterDirectory` 与 `ManuscriptReader` 的 `react-hooks/incompatible-library` 来自虚拟化库（两个文件都 `useVirtualizer`）。
该规则提示的是「React Compiler 会跳过这个组件的记忆化」，而本仓**没有接入 React Compiler**（`babel-plugin-react-compiler`
只是 eslint 插件的可选依赖，不在 `package.json` 里），规则没有作用对象。覆盖**按 `files` 收在这两个文件上**
（`webui/eslint.config.js`），不是对全部 `ts/tsx` 关闭，也不是整条规则停用；将来真的接入编译器时删掉这个覆盖即可恢复提示。
范围外仍会提示的证据见 46.6.2。

### 46.5 现场与门禁

全新数据目录 + 当前二进制（`--port 8881 --data-dir /tmp/nw-lint-live/data`），这条路径正好经过本轮改过的两处：
密钥归一化与回执重放。

```
PUT /api/settings  同一 commandId 两次（只换密钥）→ 200 / 200，两次响应逐字节相同
  第二次的密钥没进库：系统设置仍是 sk-lint-first（key_len=13）
PUT /api/settings  apiKey=""                 → 422 VALIDATION_ERROR「API key 不能为空」
PUT /api/settings  apiKey="sk-lint\u0007bad" → 422 VALIDATION_ERROR「API key 不能包含控制字符」

command_receipts      : PUT /api/settings × 1（payload_hash 长度 64）
events                : settings.changed × 1
system_model_settings : revision=1
rg sk-lint 于 payload_hash / 服务日志 / data/logs → 0 命中
```

```
cargo fmt --all -- --check                                            干净
cargo clippy -p nekowriter-core -p nekowriter-agent -p nekowriter-store -p nekowriter-app --all-targets   0 error / 0 warning
cargo clippy -p xtask --all-targets                                   0 error / 0 warning
cargo clippy --workspace --all-targets                                0 error / 0 warning（测量过程见 46.6.3）
cargo test --workspace                                                253 通过 / 0 失败（本仓 251：agent 44、app 单元 67、app 集成 2、core 38、store 96、xtask 4；另一线程的 nekowriter-prompt-lab 2）
webui: pnpm lint                                                      0 error / 0 warning
webui: pnpm test                                                      10 通过 / 0 失败
webui: pnpm exec tsc --noEmit && pnpm build                           干净
```

用例数口径：**253 里含另一线程的 `nekowriter-prompt-lab` 2 条**，本仓自有 crate 是 251。本表的数字是**写入时按当前树重测**的
（§47 新增两条用例后为 253；§46 收口当时为 251，其中 store 94）。同一份表格只保留一份数字，历史时点不再在本节重复。

规模：§46 时点 20 个代码文件 +243 / −237（不含 `Cargo.lock`；文档改动另计）。**当前**同一批 20 个代码文件（`git diff --stat`，不含 `docs/`、`Cargo.lock` 与另一线程的 `prompt-lab`）是 **+670 / −237**：插入数比 §46 时点多出的 427 行全部来自 §47 写进 `crates/nekowriter-store/src/lib.rs` 的用例与测试辅助。

仍在事实层面的两点：`Cargo.toml` 里新增的策略段与另一线程的 `members` 改动同处一个文件，提交时需要分开归属；
`crates/nekowriter-prompt-lab/`（另一线程在建，未跟踪）不属本批，本轮的 workspace 级数字里它单独标出。

### 46.6 收口：抑制范围、被切坏的注释与 workspace 级证据

只读审计对 46.1–46.5 提了三处缺口（注释被搬动切坏、抑制范围大于证据、`--workspace` 级清零未演示），
本轮只动抑制口径、注释与文档，不改任何运行时行为。

#### 46.6.1 被切坏的注释已修回，并全仓扫描过同类

`items_after_statements` 的修法把用例内的 `functions` 辅助函数提到 `lib.rs` 的模块级；第一次搬动把
「事务外回执预查只有一个所有者」这段不变式说明挂到了 `functions` 上，而它所属的
`transaction_external_recheck_has_a_single_owner` 丢了第一句。现在两段各归其位：不变式说明在用例上方，
`functions` 只留描述它自己的注释。

扫描方式：找「`///` 文档块后紧跟普通 `//` 注释、再往下才是项」的形状，在 `crates/**/*.rs`（整棵源码树）上返回 **0** 处；
本批 diff 里被删除的 `///` 行也是 **0** 行（`git diff -U0 -- 'crates/**' | rg -c '^-\s*///'`），所以没有第二段注释被同样切错。

#### 46.6.2 两条抑制收到证据所在处，且两侧都验过

| 实验 | 做法 | 结果 |
| --- | --- | --- |
| 承重（收窄不是形式） | 在隔离副本里临时撤掉 `crates/nekowriter-store/src/entity/mod.rs` 的模块级 `#![allow(…)]` | `clippy -p nekowriter-store --all-targets` 立刻报 **6** 条 `unused_async_trait_impl` + **3** 条 `struct_field_names`，逐个落在 6 个实体文件的 `#[sea_orm::model]` 与 `Model::model_id` 上 |
| 信号未丢 | 保持收窄，在 `entity` 模块**之外**放一个探针模块：一份 `#[sea_orm::model]` 实体 + 一个字段名与结构体名同前缀的 3 字段结构体 | 两条 lint 都在探针处触发（`unused_async_trait_impl` → 探针的 `#[sea_orm::model]`；`struct_field_names` → `all fields have the same prefix: \`widget\`` 与 3 个字段） |
| 前端同样收窄 | 在覆盖范围之外放一个只用 `useVirtualizer()` 的组件 | `react-hooks/incompatible-library` 照常报出（`TanStack Virtual's \`useVirtualizer()\` API returns functions that cannot be memoized safely`）；被覆盖的两个文件仍为 0 |

探针只在隔离副本与临时文件里存在，跑完即删；仓库里无残留（`rg -n "lint_probe|lint-probe" --glob '!docs/**' .` 为 0 命中）。

#### 46.6.3 workspace 级清零的演示，与真实 checkout 的现状

本轮开始时真实 checkout 跑不了 `--workspace`：另一线程的 `nekowriter-prompt-lab` 有 9 个编译错误
（`E0282`×2、`E0277`×3、`E0308`×3 等），所以先在 `/tmp/nw-ws-clippy` 隔离副本（给它补一个最小占位）里跑：

```
/tmp/nw-ws-clippy$ cargo clippy --workspace --all-targets                                   3 警告，均为占位 manifest 的 unused dependency（clap / nekowriter-agent / sea-orm）
/tmp/nw-ws-clippy$ cargo clippy --workspace --all-targets --exclude nekowriter-prompt-lab    0 警告
```

那个线程随后补齐了 `main.rs`，于是真实 checkout 也能跑 workspace 级命令，同一轮内出现了三个时点的状态（都如实记录，
因为 prompt-lab 是另一线程的未跟踪 crate、双方并行改动）：

```
22:35  cargo clippy --workspace --all-targets   6 警告，全部在 crates/nekowriter-prompt-lab/src/main.rs（145 / 250 / 362 / 649 / 928 / 934）
        （`unused_async`、`unnecessary_result_map_or_else`、`unnecessary_wraps`、`struct_excessive_bools`、`similar_names`、`semicolon_if_nothing_returned`）
        → 本仓的 core / agent / store / app / xtask 五个包 0 条；这 6 条属另一线程，本批未动它
22:47  cargo clippy --workspace --all-targets   0 警告（那 6 条已由另一线程自己修掉）
22:47  cargo clippy -p nekowriter-prompt-lab --all-targets  0 警告
```

旁证：prompt-lab 当时报出的 `unused_async`（不是 `unused_async_trait_impl`）说明收窄没有关掉任何别处的提示；
`docs/05` 复核里点名的 `--workspace` 级 CI 命令现在与本地结果一致。

#### 46.6.4 收口轮的门禁

见 46.5 的表格（本轮重跑）：`fmt` 干净、五个包 `clippy` 各 0 error / 0 warning、`cargo test --workspace` 253 通过 / 0 失败
（本仓 251，含另一线程 `nekowriter-prompt-lab` 的 2 条）、前端 `lint / test / tsc / build` 全绿。本轮唯一一次未复现的抖动：连跑多轮逐包测试时 `nekowriter-app` 有一次
（仅一次）出现 1 例失败，紧接着连跑 5 次都是 67 + 2 全绿，未抓到用例名也未复现；app 的测试端口用的是临时端口
（`TcpListener::bind("127.0.0.1:0")`），所以最可能的解释是与另一个线程共用同一个 `target/` 目录时的构建竞争，
不是本批代码的回归——但不复现就不结案，如实记在这里（后续追查见 §47.2）。

## 47. 2026-09-15 两个工具错误出口的行为证据，与 app 抖动的隔离重跑

本轮只加证明：新增两条经库层公开入口的用例，再加两次反向实验与一次隔离 target 的重复跑；
运行时行为一行未改（对生产代码的唯一改动是 §47.1 里那个文档注释的 backtick）。

### 47.1 新增用例：错误码与错误形状被断言到字段与消息

| 用例（`crates/nekowriter-store/src/lib.rs` 测试模块） | 覆盖 |
| --- | --- |
| `tests::chapter_body_edit_failures_report_their_codes_shapes_and_keep_the_body` | 经 `Store::execute_chapter_body_agent_tool_call` 走 `EDIT_NO_MATCH` / `EDIT_AMBIGUOUS_MATCH` / `EDIT_NO_CHANGE` / 参数层 `VALIDATION_ERROR` 四条分支；每个结果都整对象相等（键集也钮住），并断言失败不推进正文（`revision_id`/`revision_no`/`content` 不变）、失败计协议错误、一成功后计数归零、全书结束时 Run 仍 `Queued` |
| `tests::ask_user_argument_failures_keep_the_model_visible_shape` | 经 `Store::execute_agent_question_tool_call` 走两个触发：违反领域合同（零个问题 → 消息 `问题列表不能为空`）与连问题模式都解析不了（`questions` 不是数组）→ 四字段连键集一起断言，`tool_call_id` 等于 provider 调用 id |

两条用例都使用新加的两个测试辅助（`drafting_chapter_body_run` 把「准备 → 四项审核 → 章节细纲 → 细纲审核 → 正文写作」写成一处，
`dispatched_body_tool_call` 把「派发请求 + 执行工具调用」写成一处），因此用例本身只断言差异。

### 47.2 两次反向实验：签名等价、码真被钮住

实验 A（借用改动不改行为）：把 `agent_body_edit_error_result`、`agent_tool_error_result`、`question_tool_error_result`
连同四个调用点**逐字改回 `HEAD` 的 `String` 形状**（先用脚本确认这三段与 `HEAD` 逐字一致），再跑两条新用例：

```
test tests::ask_user_argument_failures_keep_the_model_visible_shape ... ok
test tests::chapter_body_edit_failures_report_their_codes_shapes_and_keep_the_body ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 94 filtered out; finished in 0.20s
```

即：`String` → `&str` 的改动两侧产出同一个可观察合同，而这两条用例就是那个合同本身。

实验 B（码不是摆设）：临时把 `ChapterBodyEditError::NoMatch => "EDIT_NO_MATCH"` 改成 `"EDIT_UNKNOWN"`：

```
test tests::chapter_body_edit_failures_report_their_codes_shapes_and_keep_the_body ... FAILED
assertion `left == right` failed
  left: Object {"base_revision_id": …,"code": String("EDIT_UNKNOWN"), "feedback": Array [], "message": String("待替换正文在当前章节中没有匹配"), "status": String("error"), "tool_call_id": String("body-tool-call-2")}
 right: Object {"base_revision_id": …,"code": String("EDIT_NO_MATCH"),  "feedback": Array [], "message": String("待替换正文在当前章节中没有匹配"), "status": String("error"), "tool_call_id": String("body-tool-call-2")}
```

两次实验都已还原，`crates/nekowriter-store/src/preparation.rs` 的 SHA-256 回到实验前的 `423c6e21…b21989`。

### 47.3 app 抖动：在隔离 target 下重跑，未复现

按上一轮的怀疑（与另一线程共用 `target/`）把 target 目录换掉：`CARGO_TARGET_DIR=/tmp/nw-app-iso`，冷构建 4 分 34 秒，然后：

```
直接重复执行同一测试二进制 8 次： 每次 67 通过 / 0 失败（集成用例同样 2 / 0）
CARGO_TARGET_DIR=/tmp/nw-app-iso cargo test -p nekowriter-app  4 次：每次 67 / 0 + 2 / 0
```

即隔离 target 后 12 次全部全绿（加上本轮在共享 target 下的几次，共 17 次未复现）。按约定如实记为环境竞争、
**这一点未结案**：既没有抓到用例名，也没有把「共用 target 导致失败」直接做出来（故意制造并发写入会干扰另一线程的构建，没有做）。

### 47.4 本轮门禁（实测）

```
cargo fmt --all -- --check                     干净
clippy core / agent / store / app / xtask      各 0 error / 0 warning
cargo test -p nekowriter-store                 96 通过 / 0 失败（本轮 +2）
cargo test --workspace                         253 通过 / 0 失败（本仓 251：agent 44、app 单元 67、app 集成 2、core 38、store 96、xtask 4；prompt-lab 2）
```

规模的唯一变化：`crates/nekowriter-store/src/lib.rs` 仅新增测试代码（两条用例 + 两个辅助 + 注释），生产代码只改了一个文档注释的 backtick。
