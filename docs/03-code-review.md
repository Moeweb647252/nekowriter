# NekoWriter 实现进度审查报告（更新版）

- 初始审查日期：2026-09-11；最近收口更新：2026-09-13
- 审查范围：`crates/`（core / agent / store / app）、`webui/`、`docs/01-implementation-plan.md`、`docs/02-implementation-spec.md`
- 审查方式：代码审查 + 实测验证；本报告保留各轮发现时的证据，当前状态以 §33 和方案 §2.1 为准。

当前本地收口状态：`cargo test --workspace` 通过 198 个用例（agent 35、app 47、core 38、store 74、xtask 4），前端生产构建通过；M0–M4 和 macOS aarch64 的 M5 范围均已完成。M2 的可达竞态与浏览器复核、M3 的多章/复制稿本/番外真实路径以及内建提示词升级均已在 §33–§34 收口；全平台计划只剩非 macOS 发行产物。

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
| F6 | clippy 警告未消化 | ⚠️ 仍有历史 pedantic/结构性建议；本轮已应用可自动修复的低风险建议，但严格 `-D warnings` 仍不通过。该卫生项不属于 M2/M3 的功能验收条件。 |
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
