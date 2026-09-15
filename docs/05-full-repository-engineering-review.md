# NekoWriter 全仓工程审查（第二轮：含冗余与简化专项）

审查对象：本仓库当前工作树（`main`，HEAD = `a8374ff`，schema 26，228 个被跟踪文件）。
审查方式：**只读**。本轮没有修改任何源码、配置或既有文档，只重写本报告（第一轮草稿由本文件直接替换，不留第二份重叠报告）。
命令一律带 `--config 'env.PROTOC.value="/opt/homebrew/bin/protoc"'` 覆盖，原因见 §5 F-03。

**事后状态（不在本报告的审查范围内，仅作指针）：本报告是只读快照，下面每一处观测数字都是审查时点（`HEAD = a8374ff`）的值，不是当前值。**
与本报告同时存在的另一类口径——每个 F 项的「已修 / 收窄 / 延期」——不在本文件里，而在
[修复报告](06-remediation-report.md)（F-01…F-18 的处置矩阵）与 `docs/03` §45–§47（逐条现场证据）。
与门禁有关的几条后来都变了，本文件不再回填：F-02 的前端 lint（审查时点 30 errors / 4 warnings）与
F-09 的 clippy 警告（审查时点 244 条 warning 行）在 2026-09-15 那轮里都是 **0**（`docs/03` §46）；
本报告的 `cargo test --workspace` 234 个用例也早已不是当前数（现为 253，见 §45.6 / §47.4）。
本报告给出的**具体做法建议**也不等于最终方案：例如 F-02 第 ③ 条建议「剩余项逐条显式 `eslint-disable`」并未采用，
实际是**按 `files` 把那条规则收窄到两个文件**（`docs/03` §46.4），仓库里没有新增散落的 `eslint-disable` 注释；
关于「仓库里现存哪些抑制」只有一份完整答案，在 `docs/03` §46.3.1。

本轮在第一轮（架构 / 正确性 / 一致性）之上，把**冗余与不必要复杂度**升为一等目标，并按「为什么它还存在 / 删掉它会怎样 / 哪条才是正典路径」三问逐项判定。
凡未亲自跑过、未在代码里追到 enforce 点、或无法构造反例的，都写清置信度与未覆盖面，不写成确定结论。

---

## 1. Executive Summary

**总体质量：核心业务不变量真被保护；验证门禁、结构承诺与「一次重构留下的新旧共存」是真债。**

这个仓库的「里子」明显好于「面子」。后端最难的事都做对了，且可核对：

- 有效模型解析只有一处实现（`app::model_settings::resolve_effective_model`），HTTP 准入与调度器 dispatch 共用它，
  没有出现「用系统模型的能力校验项目引用的模型」这第二套规则（`main.rs:2541`、`scheduler.rs:676`）。
- 提示词解析只有一条链（项目 → 系统 → 内建），全仓没有第二条 `builtin_prompt` 旁路。
- 协议纠正只有一个落账实现（`store::protocol_correction` + `ProtocolCorrectionTarget`），写作 lane 与审核子 Run 共用。
- 「一次系统模型调用的落库策略」只写一次（`app/operations.rs`），模型与 Embedding 各写一个**薄**适配器——
  这是**正确复用**的正面样本，不是重复实现。
- 幂等、乐观并发、事件原子性、外键与部分唯一索引、WAL + `synchronous=FULL`、崩溃后启动回收、启动失败统一诊断出口，
  都在代码里追到了具体 enforce 点。
- 密钥只落两列、只有一个读出口，且不在请求快照 / 事件 / 日志里。
- 0 个 `TODO`/`FIXME`/`HACK`，0 个 `unsafe`（workspace `forbid`），`panic!` 只在测试，只有一条非 ORM 原始 SQL。

**本轮新发现的最大问题是一处真实能力丢失：UI 重构（`cedca66`）把两个界面能力留在了没有挂载的文件里。**

- `webui/src/components/ServicePanel.tsx`（168 行）是**全仓唯一**调用 `shutdownService()` 的地方，而它从未被任何文件渲染：
  `rg "ServicePanel"` 除该文件自身只剩 `docs/03` 的叙述。也就是说**界面里已经没有任何退出服务的入口**。
- `webui/src/components/ShortcutHelpPanel.tsx`（94 行）是**全仓唯一**渲染 `SHORTCUT_ENTRIES` 的地方，同样从未被渲染；
  但工作台头部的「快捷键」按钮（`WorkspaceControls.tsx`，`aria-haspopup="dialog"`、`onClick={() => setOpen(!open)}`）
  仍在切换它消费的那个状态，`?` 键仍在 `ShortcutProvider` 里翻转同一个状态——**按钮和快捷键都变成了空操作**。
- 证据不止于「无人 import」：用当前源码构建出的产物 `webui/dist`（09:55，比所有 `webui/src` 文件都新）里，
  `SHORTCUT_ENTRIES` 的全部文案（「提交当前表单」「整稿查找」…）与 ServicePanel 的「确认退出」**一个字符串都不存在**；
  唯一命中「快捷键」的是头部那颗按钮本身。用户拿到的就是「点了没反应」。
- 而 `docs/02` §11/§13/§14、`docs/03` §15.4 与 §16.1、`docs/04` §6.1 都把这两项写成已实现、且经过真实浏览器验收。

**其余风险集中在「谁保证它继续对」与「一次重构留下的新旧共存」两层：**

1. 前端 `pnpm lint` 审查时点 **30 errors / 4 warnings**（已在 `docs/03` §46 清零，见本文件开头的「事后状态」），而 `docs/03` 最后一次记录是 **8 errors / 3 warnings**，
   且 UI 重构回合的验收清单里**根本没有 lint**——唯一能抓「render 期读写 ref」「effect 里 setState」这类 React 正确性问题的门禁，
   已经静默失效了一整轮重写（`cedca66` 未改 eslint 插件版本，这 22 个新错误来自新代码）。
2. 仓库**没有 CI**（无 `.github/`）；`.cargo/config.toml` 又把 `PROTOC` 以 `force = true` 钉在 Windows 路径，
   非 Windows 的干净构建连第一步都过不去（`docs/03` §37.4 已记录修法，未落地）。
3. 事件类型名在**两种语言里各写一份**：Rust 侧 19 个持久事件类型、161 个写库调用点，全是字符串字面量，没有清单常量；
   TS 侧 `events.tsx` 手写 15 个。当前有 **4 个 Rust 会发出、前端从不订阅**的类型
   （`chapter.changed`、`version.completed`、`model_request.cancelled`、`tool.included`）——
   今天被「同一事务里总有一个被订阅的事件」掩盖（我逐个站点核过），属于**潜伏漂移**而不是当前故障。
4. 抽出来的收尾器只有三分之一调用点在用：`finish_write` 出现在 `recycle.rs` / `prompt_templates.rs` / `knowledge.rs`（17 处），
   而 `lib.rs` / `preparation.rs` / `model_requests.rs` 里约 49 条命令仍在手写同样的 6 步
   （`replay_or_conflict` 54、`payload_hash` 53、`find_receipt` 55、`insert_receipt` 49、
   `replay_after_receipt_conflict` 33、`finish_write` 17）。这是「重构做了一半」的典型。
5. 事件写入也有一个被绕过的抽象：`insert_event` / `insert_app_event` → `insert_event_row` 是正典路径（6 个模块在用），
   但 `settings.rs:370`、`project_settings.rs:228`、`prompts.rs:560` 三处仍手搓 `event::ActiveModel`（形状完全等价）。
6. `docs/02` §2.1 的模块表里 **9 个路径有 6 个在代码里不存在**；它们的事实位置是 `core/lib.rs`（3,739 行）与
   `app/main.rs`（3,939 行）。文档把「应该在哪」写成了「已经在哪」，于是最大的两个文件看起来像被批准过的结构。

**是否存在系统性问题：** 是，但集中在**验证与结构**，不在业务正确性。业务正确性目前由**大量高质量的人工真实进程验收**兜底，
这在单人开发阶段可行，但它刚刚证明了自己会漏东西——两个界面能力的丢失就是因为验收清单里没有一行「挂载与可达性」的检查，
也没有 lint/测试能发现。

**是否适合继续扩展：** 后端可以继续按现有分层扩展（store / agent / app 的边界是真的，新增 lane 有明确接线点）。
前端在补上 lint 与最小测试基建之前，每加一个大组件都在增加不可验证面积。

**是否建议先偿还技术债：** 建议先做一轮**收口**：门禁（CI + lint）→ 补回丢失能力或删掉死文件 → 用构造取代手工纪律
（写信任提取器、幂等收口）→ 结构承诺对齐。这些都不改产品行为（唯一的行为变化是把丢失的入口修回来），却决定后面每一次改动的成本。

**Final Verdict：NEEDS HARDENING**（理由见 §23）。

---

## 2. Repository Architecture

```
NekoWriter（本地优先的 AI 小说写作应用，单进程、单用户、loopback）
├── crates/nekowriter-core    领域与不变量（状态机、编辑校验、字数、导出、回收站规则、模板语言、用量口径）  5,964 行
├── crates/nekowriter-agent   模型适配与协议（provider 流解析、runner 阶段守卫、prompt 模板、embedding、引擎注册表） 5,363 行
├── crates/nekowriter-store   SeaORM 实体 + SQLite 事务 + schema 初始化 + LanceDB 索引                34,820 行
├── crates/nekowriter-app     HTTP（axum）、调度器、生命周期、知识、提示词、模板、回收站、概览           13,133 行
├── crates/xtask              发行打包（容器/架构硬检查 + 发行清单）                                    596 行
├── webui                     React 19 + Vite + React Query + IndexedDB 草稿                          17,239 行 TS/TSX
├── docs/                     01 方案 / 02 细则 / 03 验收与代码审查 / 04 UI 重构计划 / 05 本报告
└── tools/                    Windows protoc 引导脚本（非 Windows 不存在该目录）
```

| Area | Responsibility | Entry Points | Important Dependencies | Persistence | Tests |
| --- | --- | --- | --- | --- | --- |
| core | 不变量与纯逻辑 | `core::*`（budget/dashboard/export/knowledge/manuscript/recycle/template/usage + `lib.rs`） | 无（无 DB / 无网络） | 无 | 38 |
| agent | 一次请求的构建/解析/工具识别 + 供应商适配 | `agent::runner`、`provider`、`prompt`、`embedding`、`engine` | `reqwest`（rustls）、SSE 解析 | `agent_messages` 由 store 写 | 44 |
| store | 事务、查询、schema、索引 | `Store`（20 个子模块，27 个实体） | `sea-orm` 2 + `sqlx-sqlite`、`lancedb` | SQLite（WAL/FULL）+ LanceDB | 84 |
| app | HTTP 合同、命令循环、调度、恢复 | `app::main::router`（68 条路由）、`WorkflowScheduler` | `axum`、`tokio`、`tower-http` | 通过 store | 62 + 2 集成 |
| xtask | 发行打包与校验 | `cargo xtask package` | `sha2` | 无 | 4 |
| webui | 界面、草稿、事件同步 | `main.tsx`、`EventSyncProvider`、`App.tsx`（7 个页面） | React 19、React Query、idb-keyval | IndexedDB（草稿） | **0** |

规模：76 个 `.rs` 文件 / 59,876 行 Rust；`webui/src` 17,239 行 TS/TSX；最大 12 个文件见 §5 F-09。

---

## 3. Review Coverage

| Domain | Reviewed | Findings | Notes |
| --- | --- | --- | --- |
| Architecture | yes | 4 | F-08、F-09；模块边界与文件体积；抽象所有权 |
| Backend API | yes | 2 | F-10（写信任逐处手写）、§13（错误信封/状态码映射一致但难改） |
| Domain Model | yes | 1 | 17 条不变量逐条找 enforce 点，见 §10 |
| Persistence | yes | 2 | F-12（无界增长）；部分唯一索引/外键/软删规则已核 |
| Migrations | yes | 0 | 按 `AGENTS.md` 有意不做，只拒绝打开；见 §11 |
| Scheduler / Runtime | yes | 0 | 单一许可、请求额度、被拒回合语义已核；见 §12 |
| Async / Concurrency | yes | 0 | 有界通道、短锁、无 await-under-lock；见 §12 |
| Error Handling | yes | 0 | 未发现吞错/错误重分类；见 §12 |
| Frontend State | yes | 3 | F-01（两个能力不可达）、F-14（render 期 ref）、F-13（零测试） |
| Security | yes | 2 | F-10（文档承诺的自定义头不存在）、F-15（回执摘要覆盖密钥） |
| Performance | yes | 1 | F-12；未发现 N+1 或二次方热点 |
| **Redundancy / Duplication** | yes | 6 | **F-04/F-05/F-06/F-07（新）+ 事件名双写 + 死代码 F-17** |
| Dead / Legacy Code | yes | 2 | F-01（两个死组件）、F-17；keyring 残留已确认清零 |
| Tests | yes | 3 | §16（不变量–测试矩阵）；F-13；测试本身无重复/无过度 mock |
| Observability | yes | 0 | 请求 id / command id / 快照 / 事件齐备 |
| Configuration | yes | 1 | F-03（PROTOC force） |
| Deployment | yes | 2 | F-11（无 CI）、F-03；打包器本身有硬检查 |
| Documentation | yes | 3 | F-08、F-18（stale 句）、§18 逐份清单 |
| Compatibility | yes | 0 | 明确不做旧格式兼容，与 `AGENTS.md` 一致 |
| Data Lifecycle | yes | 2 | F-12、F-16 |

**无法充分确认的区域（明确写出，不假装完成）：**

- **Windows / Linux 发行产物与运行行为**：本机无对应目标链接器与标准库，只有配方与打包器硬检查被反例验证过。
- **真实第三方 Embedding 供应商**：环境中不存在可用端点，知识库路径只有桩服务与降级路径证据。
- **知识积累生产引擎**：生产注册表严格为 0，成功写入只在测试注册表上验证过。
- **长时间/大数据量下的性能**：没有 20 万行级别的真实库可跑；`docs/02` §16.1 的量级是估算值。
- **前端运行期行为**：没有前端测试基建。本轮 F-01 的结论建立在「静态引用为 0」+「构建产物字符串缺失」两条硬证据上，
  但我**没有**用真实浏览器点过那颗「快捷键」按钮（当前也没有在跑的 8820 实例可供点）。这是可复核的缺一步，不是猜测。
- **崩溃在极小窗口内的行为**：`StartupRecovery` 与事务边界已核，但没有故障注入（事务中途 kill -9）的实证。

---

## 4. Architecture Assessment

**分层是真的。** `core` 不依赖 DB/网络；`agent` 不知道稿本结构；`store` 不做业务决策；`app` 拥有 HTTP 与调度；
依赖方向单向，`store` 没有反向依赖 `app`。这一点在本仓的规模下不容易做到。

**单一事实来源（Single Source of Truth）逐项可答（本轮逐条复核）：**

| 数据 | 唯一所有者 | 证据 |
| --- | --- | --- |
| 有效模型（ID + 能力） | `model_settings::resolve_effective_model` | 仅 1 处实现，2 类调用点 |
| 有效提示词 | `prompts::resolve_prompt` | project → system → builtin，无旁路 |
| 工作流状态 | `workflow_instances.state_json` + core 类型 | 状态 JSON 必须反序列化为 Rust 类型 |
| 密钥 | `system_model_settings.api_key` / `embedding_settings.api_key` | 各 1 个读出口 |
| 事件 | SQLite `events` 表（正典写入口 `insert_event`/`insert_app_event`） | 写入口有 3 处绕过（F-06） |
| 前端事件流 | 单个 `EventSyncProvider` | 全仓只有 1 处 `new EventSource` |
| 草稿 | 浏览器 IndexedDB，键 `(dataDir, versionId, phase, targetId)` | 与服务端快照分离 |
| 请求计数 | `runs.request_count` / `request_allowance_end` | 不用清空请求记录实现继续 |
| 当前稿本 | `projects.selected_version_id`（指针，非副本） | 只此一处；旧稿本仍可读 |

**所有权污染（本轮新发现的两处，都不是「两份事实」而是「两份实现」）：**

- **事件行有两个写法**：正典 `insert_event_row`（版本域/无版本域两个包装）+ 3 处手搓 `event::ActiveModel`（形状等价，F-06）。
  领域规则（例如「prompt 覆盖变更也发 `settings.changed`」）因此在两个文件里各写一次。
- **两条设置命令各有各的校验助手**：模型侧在 `normalize_system_model_settings` 里内联 `Url::parse` 与路径判定，
  Embedding 侧用 `validate_api_base_url`/`validate_required`（F-07）。同一条「API 根地址必须合法」的规则有两份判定。

**不必要的抽象与冗余包装（逐类判定的结论，避免机械去重）：**

| 候选 | 判定 |
| --- | --- |
| `app/operations.rs` + `persist_connection_test` + `record_embedding_operation` | **不是冗余**。共享策略只写一次，两个适配器各自翻译自己的错误类型——这是有语义的边界 |
| `normalize_api_key`（两处调用） | **正确复用**（同一函数，两个入口） |
| `validate_required`（5 处调用） | **正确复用** |
| `insert_event` / `insert_app_event` 两层包装 | **保留**。区分「属于稿本」与「不属于稿本」，是真实语义 |
| `Store` 的 `pub async fn` 343 个 | 多数是命令边界，不算包装；`provider.rs::api_base_url()` 是**无人调用的 getter**（F-17） |
| `finish_write` | 收尾器本身正确，问题是**只有 17/49 调用点用它**（F-05） |

**Architecture Verdict：Acceptable（偏 Good）。**
理由：所有权清晰、依赖方向正确、关键决策只有一份实现；但**模块分解与文档不符**（F-08）、
**万行文件承载多层逻辑**（F-09）、**两个抽象只迁移了一半调用点**（F-05、F-06），
再加上一次 UI 重构留下的两个死文件（F-01）。修掉这五项，这份结构会升到 Good 甚至 Excellent——
它们是「同一套设计已经写在文档里，但没有落在文件系统里」与「重构没做最后一步」。

---

## 5. Critical Findings

### P1

---

**F-01 UI 重构后两个已被文档声明、且被记入「已验收」的界面能力不再可达；它们只活在两个从未挂载的死文件里**（Confidence: Confirmed）

- **Location**：`webui/src/components/ServicePanel.tsx`（168 行）、`webui/src/components/ShortcutHelpPanel.tsx`（94 行）、
  `webui/src/components/WorkspaceControls.tsx:22/58-64`、`webui/src/lib/shortcuts.tsx:86-121/275-284`、`webui/src/lib/api.ts:607`；
  文档侧 `docs/02` §11（交付回路，第 849 行）、§13（UI 验收规则，第 938 行）、§14（实施任务清单，第 996 行）、
  `docs/03` §15.4（本地服务面板）与 §16.1（快捷键判定）、`docs/04` §6.1。
- **Observation**：
  1. `rg -n "ServicePanel|ShortcutHelpPanel"` 在**全仓**只命中这两个文件自身的定义与 `docs/` 的叙述，没有任何 import 或 JSX 使用点；
     `webui/src/pages/*`（7 个页面）与 `App.tsx`（9 条路由项 = 7 个页面 + index 与通配回退）里都没有第三个「服务/退出」入口。
  2. `shutdownService()` 的**唯一**调用点是 `ServicePanel.tsx:55`，`POST /api/system/shutdown` 在后端存在（`main.rs:1371`）并有实现。
  3. `SHORTCUT_ENTRIES` 的**唯一**渲染点是 `ShortcutHelpPanel.tsx:72`；
     `WorkspaceControls.tsx` 的「快捷键」按钮仍在 `aria-haspopup="dialog"` 下切换 `useShortcutHelp().setOpen`，
     `shortcuts.tsx` 的 `?` 键处理仍在 `setShortcutsOpen((current) => !current)`——**状态被翻转，但没有消费者**。
  4. 构建产物核对：`webui/dist`（2026-09-15 09:55）比所有 `webui/src` 文件都新（`find webui/src -newer webui/dist/index.html` 为空），
     其中**不存在** `SHORTCUT_ENTRIES` 的任一文案（「提交当前表单」「整稿查找」…）与 ServicePanel 的「确认退出」；
     唯一含「快捷键」的是头部按钮。Vite 只打包被 import 的模块，因此这是「没进产物」，不是「产物过期」。
  5. `git log -- webui/src/components/ServicePanel.tsx` 只有 `2df723d`（重构之前）；`cedca66` 的 diff 只新增
     `AppFrame.tsx` 与 `pages/WorkspacePage.tsx`，没有删除任何引用——即引用是**在某次重写中被丢掉**的，文件本身留着。
- **Expected invariant**：`docs/02` §13（第 941 行）写「「本地服务」面板显示界面地址与实际数据目录，退出服务需二次确认
  并说明退出后保留什么；服务退出后界面区分「已退出」与「正在重连」」，同节还规定「已实现的快捷键集合限定为四条，
  且可由界面发现（头部按钮与 `?` 打开说明）」；§14（第 996 行）重复确认这四条绑定与打开方式；`docs/03` §15.4 记录了 ServicePanel 的完整行为、
  §16.1 记录了四条快捷键的判定；`docs/02` §11 的交付验收段把「本地服务面板与退出」列为闭环回路的一站。
  文档层与产品层都要求这两项可达。
- **Violation**：两项能力在代码里**只有定义、没有挂载**；界面提供的是一颗点了不改变任何可见状态的按钮，
  以及一个没有 UI 的 `?` 快捷键；同时**没有任何界面入口能请求服务退出**（只能杀进程或直接 curl）。
- **Failure scenario**：写作者按 `docs/02` §13 的说明点「快捷键」（或按 `?`）想看清四条绑定——界面无反应，他会以为快捷键失效，
  进而怀疑整条命令路径；想按文档优雅退出服务时，界面里找不到任何入口。
- **Impact**：① 两处用户可见的功能缺失（其中退出属于「文档承诺的管理动作」）；② 262 行死代码留在源码里，
  每次改主题/快捷键都要读它、且它已不再被 lint 之外的任何东西验证；③ 更严重的信号意义——
  一次被记入「已验收」的重构丢掉了两个入口而无人发现，说明验收清单缺「可达性」这一项。
- **Root cause**：UI 重构以「新增 AppFrame + WorkspacePage」的方式替换了旧的应用外壳，两个旧组件落在新外壳之外；
  验收只测了新路径（路由、草稿、SSE、窄屏），没有对旧能力做「删除或接线」的收口，也没有死代码检查（无 lint 无用导出规则、无前端测试）。
- **Recommendation**：二选一，不要继续两边挂着。
  **要么接线**：在 `AppFrame`/工作台头部恢复挂载 `ShortcutHelpPanel`（它的 `useEscapeDismiss` 注册正好补齐 `Esc` 的「关闭非破坏性层」），
  并在全局导航里恢复服务面板入口（它已含二次确认与「以后端是否还能应答为准」的退出语义）；
  **要么删除**：删两个文件 + `shutdownService`，并把 `docs/02` §11/§13/§14、`docs/03` §15.4 与 §16.1、`docs/04` §6.1 的口径改为「已移除」。
  以产品承诺为准，推荐**接线**（退出入口与快捷键说明都是 `docs/01` 的既定范围）。无论选哪个，都要在验收清单里补一行
  「新增/重写界面后，逐项确认既有能力仍可达（引用搜索 + 构建产物字符串核对）」。

---

**F-02 前端 lint 门禁静默失效（8 → 30 个错误），UI 重写回合完全没有测它**（Confidence: Confirmed）

- **Location**：`webui/`（`eslint.config.js`、`src/hooks/use-workspace-draft.ts`、`PreparationPanel.tsx` 等）；`docs/03` 的 lint 记录行。
- **Observation**：`npx eslint . -f json` 审查时点实测 = **30 errors / 4 warnings**：
  `react-hooks/refs` 16、`react-hooks/set-state-in-effect` 8、`react-refresh/only-export-components` 6；
  warnings 为 `react-hooks/incompatible-library` 与 `exhaustive-deps`。按文件：`use-workspace-draft.ts` 10、
  `PreparationPanel.tsx` 4、`ChapterBodyPanel.tsx` 3、`ChapterOutlinePanel.tsx` 2、`ChapterReadyPanel.tsx` 2。
  `docs/03` 最后一次 lint 记录（§22）写的是「与基线一致（8 个错误、3 条警告）」，之后**没有任何一条 lint 记录**；
  `docs/04` §6.1 的验收清单列的是 `pnpm build`、`cargo fmt`、`git diff --check`、`cargo test` 与真实浏览器项，**没有 lint**。
- **Expected invariant**：`docs/02` §15.1 要求「常规检查为 Rust fmt/clippy/test，以及前端 TypeScript 项目构建、lint 和相关 UI 测试」；
  即 lint 是约定门禁，基线变化必须被记录。
- **Violation**：门禁失败 30 处；基线数字停在 8；变化发生在一个重写前端的提交里而无人测量。
  已核 `cedca66` **没有**改动 eslint 插件版本，因此新增错误来自新代码，不是规则升级。
- **Failure scenario**：有人改工作台草稿逻辑。`pnpm lint` 本来就红，没人看；`tsc`/`vite build` 只保证类型与打包；
  于是 render 期读写 ref 的行为变化（把 `pendingDraftsRef.current.set()` 从 render 移到 effect，或反过来）
  在没有任何自动化信号的情况下上线，表现为「切章后上一章没输完的文字写到新章」或「草稿丢一次」。
- **Impact**：一整类 React 正确性问题失去自动化检出；「与基线一致」这种不带数字的措辞还会继续掩盖新增长。
- **Root cause**：把「lint 失败」当成可接受基线（而不是修掉或显式降级），并在重写前端的那一轮跳过了这道门禁；
  更深层是没有 CI，门禁完全依赖执行者的记忆。
- **Recommendation**：① 修 `use-workspace-draft.ts` 的 render 期 ref 读写（见 F-14）；
  ② 把 6 个 `react-refresh/only-export-components` 拆出常量文件；③ 剩余项逐条显式 `eslint-disable` 并写明理由，
  让 `pnpm lint` 回到 0 退出码；④ 验收清单固定加上 lint 一行，禁止再用「与基线一致」这种不带数字的措辞。

### P2

---

**F-03 `.cargo/config.toml` 用 `force = true` 把 `PROTOC` 钉死为 Windows 路径，非 Windows 干净构建无法开始**（Confidence: Confirmed）

- **Location**：`.cargo/config.toml`（`PROTOC = { value = "tools/protoc/bin/protoc.exe", relative = true, force = true }`）、`tools/README.md`。
- **Observation**：该路径只由 `tools/bootstrap-protoc.ps1` 在 Windows 上生成，本 checkout 里不存在（`tools/protoc` 不存在）；
  `force = true` 让环境变量也无法覆盖。`docs/03` §37.4 已用一次真实失败记录过它（`lance-encoding` 构建脚本
  `Could not find protoc`）并给出正确修法，**那份记录之后没有落地**——本轮审查仍须给每条 cargo 命令手工加 `--config`。
- **Expected invariant**：干净 checkout 上执行文档里写的命令（`cargo test --workspace`）应当可以开始；平台相关配置应被限定到该平台。
- **Violation**：主开发平台（macOS aarch64）上首次构建必经失败；绕过方式只写在 `docs/03` 的历史记录里，
  README/工具说明里没有，新接手的人会先撞一次墙。
- **Impact**：挡住新环境与未来的 CI（Linux/macOS runner 会在 build script 处直接失败）；属「已知、已记录、未修」的确定性缺陷。
- **Root cause**：`[env]` 是全局段，被用来承载平台专属事实。
- **Recommendation**：移进 `[target.'cfg(windows)'.env]`；非 Windows 交给系统 `protoc`（并在 `tools/README.md` 写明
  `brew install protobuf` / `apt install protobuf-compiler`）。一行改动 + 一句文档。

---

**F-04 持久事件类型名在 Rust 与 TS 各写一份，当前已有 4 个类型前端从不订阅**（Confidence: Confirmed）

- **Location**：Rust 侧 161 个 `insert_event` / `insert_app_event` 调用点（`store/preparation.rs`、`model_requests.rs`、
  `protocol_correction.rs`、`knowledge.rs`、`prompt_templates.rs`、`recycle.rs`、`system_operations.rs`）+ 3 处手搓写入（F-06）；
  TS 侧 `webui/src/lib/events.tsx:32-48` 的 `persistentEventTypes`。
- **Observation**：逐调用点抽出类型实参后，Rust 侧发出 **19** 个持久类型（`run.changed` 41、`workflow.changed` 41、
  `review.changed` 17、`feedback.changed` 13、`document.committed` 10、`recycle.changed` 9、`knowledge.index_changed` 6、
  `prompt_template.changed` 5、`project.changed` 4、`tool.resolved` 4、`tool.proposed` 2、`model_request.cancelled` 2、
  `chapter.changed` 2、`settings.changed`（3 处手搓）、`tool.included` 1、`question.created` 1、`question.answered` 1、
  `version.completed` 1、`system.model_operation` 1）；TS 侧订阅 **15** 个。
  **差集（Rust 发、前端不订阅）**：`chapter.changed`、`version.completed`、`model_request.cancelled`、`tool.included`。
  我把这 4 个站点逐个读了同事务上下文：`chapter.changed` 之后紧跟 `workflow.changed`（`preparation.rs:7182` 与 `12426`）、
  `version.completed` 之后紧跟 `workflow.changed`（`preparation.rs:7612` + 下一处）、`tool.included` 与 `feedback.changed` 同事务
  （`model_requests.rs:352/375`）；因此**今天不构成故障**——但「刷新靠哪个事件」这件事现在是隐式的。
  反向差集为空（TS 订阅的 15 个今天都在发）。
- **Expected invariant**：`docs/02` §10.4/§13 的口径是「持久事件驱动查询缓存失效」；
  事件类型名是**跨语言的合同**，两侧必须描述同一集合。
- **Violation**：合同被各自独立声明（19 个字符串字面量无清单常量 vs 15 个手写数组）；
  新增/改名一个类型时，忘记同步另一侧**不会有任何编译或测试信号**，前端只是静默不刷新。
- **Failure scenario**：以后有人把 `chapter.changed` 从同事务里挪走（或新增一个不被订阅的类型作为**唯一**通知），
  章节状态变化就不会触发 `["chapters", versionId]` 失效，界面显示旧状态直到用户手动刷新——
  这正是已经咬过两次的那类「测试全绿但界面不更新」事故。
- **Impact**：跨语言合同的隐式同步；当前不痛，但它是「本地正确、全局错误」的温床，且定位成本极高（前端不刷新不会报错）。
- **Root cause**：事件类型是裸字符串字面量，没有单一清单；前端订阅集合是手写数组；两侧无对账机制。
- **Recommendation**：在 Rust 侧建一份 `EVENT_TYPES` 清单常量（或 `enum` + `as_str()`），所有写库点引用它；
  再加一步「由 Rust 生成/校验 TS 常量」的最小脚本或一条测试（例如 `include_str!` 读 TS 文件断言集合相等），
  让「两侧集合一致」变成一条可执行断言。同时**明确**那 4 个类型的定位：要么让前端订阅，
  要么在 `docs/02` 写明「它们只做审计、不驱动刷新」。

---

**F-05 幂等编排只迁移了一半：抽出的 `finish_write` 只有 17/49 个命令在用**（Confidence: Confirmed）

- **Location**：`store/recycle.rs`、`store/prompt_templates.rs`、`store/knowledge.rs`（用 `finish_write`）
  对 `store/lib.rs`、`store/preparation.rs`、`store/model_requests.rs`（手写 6 步）。原语定义在 `store/lib.rs:1273`。
- **Observation**：逐函数计数——`payload_hash` 53、`replay_or_conflict` 54、`find_receipt` 55、`insert_receipt` 49、
  `replay_after_receipt_conflict` 33、**`finish_write` 17**。
  「算 payload 结构体 → 算哈希 → 事务外预查 → 事务内复查 → 写回执 → 返回」这 6 步在约 49 条命令里是逐条手写的；
  用 `finish_write` 的恰好是**三个较新的模块**（recycle 5、prompt_templates 3、knowledge 9），
  老的三个大模块一个都没迁。`finish_write` 与手写款**语义等价**（我逐条比过 `create_project` 一类的手写序列），
  所以这不是「有意保留两条路径」。
- **Expected invariant**：`docs/02` §10.2「先查询命令回执：ID、目标与请求 hash 一致时返回原结果」是**一条**规则。
- **Violation**：一条规则有约 50 份实现；差异（某处忘了事务内复查、某处漏了 `expected_revision` 校验、某处 target 字符串写错）
  不会编译失败，也不会被现有测试发现（store 测试通常顺序执行）。
- **Failure scenario**：新增一条写命令时，作者复制最近的一条作为模板；若模板恰好是少数派写法，
  新命令就少了事务内复查，表现为「并发重试时写两次」；或 target/method 与回放时不一致，导致同 id 重放返回冲突。
- **Impact**：幂等性是产品明确承诺的合同，其正确性现在依赖约 50 处手工一致；任何一处漂移都难以现场定位。
- **Root cause**：原语被提取了，但「编排」没被提取；迁移停在「新模块用新写法」。
- **Recommendation**：把 `finish_write` 升级成接收事务闭包的泛型收口（method/target/payload + `|tx| …`），
  然后**按模块**迁移（建议顺序：`model_requests.rs` → `preparation.rs` → `lib.rs`），每迁一条跑一次 store 测试。
  迁移过程本身会暴露少写复查/校验的路径——这是有实际产出的重构，不只是整洁。不要为此造抽象层，一个函数足矣。

---

**F-06 事件写入的正典抽象被 3 个模块绕过，且其中一个与同域另一模块写法相反**（Confidence: Confirmed）

- **Location**：正典 `store/preparation.rs:13603`（`insert_event`）/`13625`（`insert_app_event`）/`13645`（`insert_event_row`）；
  绕过点 `store/settings.rs:370`（`insert_system_settings_event`）、`store/project_settings.rs:228`、
  `store/prompts.rs:560`（`insert_prompt_override_event`）。
- **Observation**：`insert_app_event` 被 6 个模块使用（knowledge、lib、prompt_templates、recycle、system_operations、version_copy）。
  上述 3 处则自己构造 `event::ActiveModel { version_id: Set(None), … }.insert(transaction)`——**与 `insert_event_row` 形状逐字段等价**。
  尤其刺眼的是**同域两个模块做法相反**：`prompt_templates.rs` 用 `insert_app_event` 发 `prompt_template.changed`，
  而 `prompts.rs`（提示词覆盖）手搓同一类事件并发 `settings.changed`。也就是说「提示词相关变更怎么记事件」有两份实现。
- **Expected invariant**：`docs/02` §10.3 要求事件与业务写入同事务、且事件集合有限；
  这类「往事件表追加一行」的操作应只有一个正典实现（否则将来加字段、加广播语义、加保留期规则要改多处）。
- **Violation**：同一张表有两个写法；3 处绕过点不带 `version_id` 语义包装、也不复用未来的任何集中改动。
- **Failure scenario**：将来要在事件行上统一加一个字段（例如 `scope`）或改成「写库 + 广播」一体化，
  改了 `insert_event_row` 就会漏掉这 3 处；且这 3 处发的事件今天恰好**不在**前端订阅集合里（§F-04 的 `settings.changed` 在，
  但它是靠 `persistentEventTypes` 手写进来的），一旦有人只按 `insert_*` 常量生成 TS 清单，这 3 处就会从合同里消失。
- **Impact**：中等。不是当前故障，但它是「半套抽象」的教科书案例：抽象存在、多数调用点遵守、少数绕过且语义等价。
- **Root cause**：这 3 个模块（settings/project_settings/prompts）在 `insert_app_event` 出现之前写好，之后没有回头迁移。
- **Recommendation**：把这 3 处改成调用 `insert_app_event`（各删 12 行、行为不变），并让 `insert_event_row` 保持 `pub(crate)` 私有。
  这一项与 F-04 应一起做：迁移后 `rg "event::ActiveModel"` 在生产代码里应当为空，事件类型清单才有唯一来源。

---

**F-07 两条设置命令是平行实现：store 侧两套归一化/校验/写事务，app 侧两段相同编排**（Confidence: Confirmed）

- **Location**：store 侧 `settings.rs:163`（`save_system_model_settings`）+ `settings.rs:289`（`normalize_system_model_settings`）
  对 `knowledge.rs:186`（`embedding_settings_payload`）+ `save_embedding_settings`；
  app 侧 `main.rs:1667`（`save_system_settings`）对 `knowledge.rs:189`（`save_embedding_settings`）。
- **Observation**：
  1. **校验规则两份**：模型侧在 `normalize_system_model_settings` 里内联 `Url::parse` + scheme/host/query/fragment/路径判定
     （「必须是 API 根路径，不能是 chat/completions 完整端点」），Embedding 侧用 `validate_api_base_url`
     （只判断 `http(s)://` 前缀）。**同一条「API 根地址合法」的语义，两份判定，而且严格程度不同**。
  2. **写事务结构两份**：两处都是「查 existing → 比对 `expected_revision` → 自增 revision → upsert → 写事件 → 写回执」，
     只有实体与字段不同；连「`None` 表示保留既有密钥」的 `or_else` 合并都各写一遍。
  3. **app 编排两份**：两段 handler 逐行同构——`ensure_trusted_write_request` → 取 `settings_write_lock` → 读当前 →
     拆请求体 → `replay_*_if_known` 命中即返回 → 比对 revision 报冲突 → 调用 store 保存 → 包响应。
   （Embedding 侧额外多一次真实探针调用，那是它的业务差异，不是重复。）
- **Expected invariant**：`docs/02` §8/§9 对两类设置给的是**同一组语义**（幂等、乐观并发、省略即保留、不返回密钥正文）。
- **Violation**：同一组语义有两份实现；地址校验的强度还不同（Embedding 侧更松）。
- **Failure scenario**：将来给「设置保存」加一条规则（例如禁止私有网段地址、或要求 URL 不能带凭据），
  只会改到其中一份；另一个设置面板静默地不受该规则约束——这正是「一条业务规则两种实现」的典型后果。
- **Impact**：中。可维护性与一致性风险；今天两处行为**都自洽**，没有发现已经漂移的用户可见差异（除校验宽严）。
- **Root cause**：Embedding 设置（较晚加入）按模型设置的形状抄了一遍，而不是抽出共同的「设置命令」骨架。
- **Recommendation**：**不要**追求把两类设置合并成一张表（它们实体不同、生命周期不同）。
  只做两件收敛：① 把地址校验统一到一处（抽出 `validate_api_base_url` 的严格版，两个设置都用它），
  并在 `docs/02` §9.3 记明这条规则从属于两者；② 在 app 层抽一个小的设置保存样板
  （`save_settings_with_receipt(replay_fn, revision_fn, save_fn)` 形状）——如果它能自然地覆盖两处就做，不能就只做 ①。
  本项目在 `app/operations.rs` 已经证明「共享策略写一次 + 薄适配器」是可行且干净的，这里沿用它。

---

**F-08 `docs/02` §2.1 的模块职责表与实际模块不符（9 个里 6 个不存在）**（Confidence: Confirmed）

- **Location**：`docs/02-implementation-spec.md:53-61`；`crates/nekowriter-core/src/lib.rs`；`crates/nekowriter-app/src/main.rs`；`crates/nekowriter-agent/src/lib.rs`。
- **Observation**：逐条核过 `pub mod` / `mod` 声明：
  - `core::workflow`、`core::document` → **不存在**；工作流状态机与文档编辑校验都在 `core/src/lib.rs`（3,739 行）。
  - `agent::prompts` → 实际模块名是 `agent::prompt`。
  - `app::services`、`app::capabilities`、`app::http` → **不存在**；HTTP、命令编排、能力注册在 `app/src/main.rs`（3,939 行）。
  - 只有 `agent::runner`、`agent::provider`、`store` 与文档同名。实际存在的模块是
    core `budget/dashboard/export/knowledge/manuscript/recycle/template/usage`、app `dashboard/knowledge/lifecycle/manuscript/model_settings/operations/prompts/recycle/scheduler/templates`、
    agent `embedding/engine/prompt/provider/runner`。
- **Expected invariant**：§2.1 是「模块职责」表，读者据此判断「某行为应该改在哪个文件」。
- **Violation**：六个路径指向不存在的模块，等于把「设计意图」写成「当前事实」；最直接后果是 §5 F-09 里那三个最大文件
  在文档上看起来并不存在（它们被描述成了若干个模块）。
- **Impact**：维护者按文档找位置会扑空；「一个行为改在一处」的判断失去依据；也让文件规模问题不被看见。
- **Root cause**：文档先行，随后在压力下把新职责塞进已有的两个大文件，没有回头改表。
- **Recommendation**：二选一，**不要**继续让文档漂移——要么把表改成事实（列出真实路径，并注明它们当前承载多个关注点），
  要么做一次机械拆分（`core::workflow`、`core::document`、`app::http` 是天然切缝，因为函数边界已经存在）。
  建议先改文档（成本极低），把拆分列进 §20 的长期项。

---

**F-09 万行文件与 678 行的错误映射函数**（Confidence: Confirmed）

- **Location**：`store/preparation.rs` 13,666 行；`app/scheduler.rs` 6,803；`store/lib.rs` 6,603；`app/main.rs` 3,939；
  `core/lib.rs` 3,739；`store/knowledge.rs` 2,730。单函数：`store::preparation::execute_chapter_body_agent_tool_call` 678 行、
  `store::version_copy::copy_version` 666 行、`app::scheduler::begin_model_request` 548 行、`ApiError::into_response` 466 行。
- **Observation**：`clippy --workspace --all-targets` 报 **0 errors / 244 条 warning 行**（去重后约 234 条），
  其中 `too_many_lines` 命中 **86 次**（即 80 余个函数超过 100 行）、`missing_errors_doc` 60、`needless_update` 38、
  `needless_pass_by_value` 11、`cast_possible_wrap` 9、`unused_async_trait_impl` 6、`too_many_arguments` 6。
- **Expected invariant**：`AGENTS.md` 要求「保持组件模块化、关注点清晰分离」；`docs/02` §2.1 也是按关注点分的。
- **Violation**：HTTP 错误映射（一个 match 覆盖全部错误域）、阶段提交边界、调度分派都在单函数里承载多层逻辑。
- **Failure scenario**：给某个错误码改文案或状态码时，要在 678 行的 match 里定位；改错分支不会编译失败（同类型）。
- **Impact**：review 成本、改动风险与合并冲突面都随规模上升；这是「局部正确、全局难改」的典型。
- **Root cause**：函数边界存在但文件边界没跟上（与 F-08 同根）。
- **Recommendation**：先做**只移动、不改行为**的拆分：`ApiError` 按错误域拆成 `http/errors/{run,review,knowledge,chapter,settings}.rs`；
  `scheduler.rs` 按 lane（准备 / 细纲 / 正文 / 审核）拆。用 `git mv` + 剪切粘贴 + `cargo test` 验证即可，风险极低。

---

**F-10 写请求信任校验逐个 handler 手写，且文档承诺的「自定义请求头」不存在**（Confidence: Confirmed）

- **Location**：`app/main.rs:3034` `ensure_trusted_write_request`；调用点分布 `main.rs` 35 / `knowledge.rs` 12 / `recycle.rs` 6 / `templates.rs` 4（含定义与 import）；`docs/02-implementation-spec.md:807`。
- **Observation**：该函数只校验两件事：Host 是 loopback；**若** `Origin` 头存在则必须在白名单里。
  全仓没有任何自定义请求头校验（`HeaderName|header::` 的命中只有 CONTENT_TYPE / CACHE_CONTROL / CONTENT_DISPOSITION 等响应头），
  前端 `api.ts` 也只发 `Content-Type: application/json`。而 §11.3 写的是「写接口只接受 JSON 与自定义请求头」。
- **Expected invariant**：文档承诺的写保护应当存在；且此类横切规则应由构造保证，而不是由每个 handler 记得调用。
- **Violation**：① 「自定义请求头」这一层不存在（实际防线是 JSON content-type + Origin 白名单，对浏览器 CSRF 仍有效，
  但比文档描述的弱；`/api/system/shutdown` 这类无 body 的 POST 连 content-type 都不带）；② 57 处手工调用意味着
  **下一个写端点完全可能漏掉**，而这种遗漏不会有任何测试或类型错误提示。
- **Impact**：安全上是一条「文档承诺 ≥ 实现」的缺口；工程上是一条纯靠纪律维持的不变量。
- **Root cause**：横切校验被实现成一个普通函数，而不是中间件/提取器（`FromRequestParts<AppState>`）。
- **Recommendation**：做成自定义提取器或 `Router` 层中间件，统一挂到写方法上；同时把 §11.3 的措辞改成事实
  （「JSON + Host/Origin 校验」），或真的加上一个自定义头（前端统一注入）。二选一，别再让两者不一致。

---

**F-11 没有 CI**（Confidence: Confirmed）

- **Location**：仓库根（无 `.github/`，无任何 workflow 文件）；`docs/02` §11.5 的 yaml 片段。
- **Observation**：唯一一次「CI 步骤」出现在文档里，且只覆盖 Windows 打包；`cargo fmt/clippy/test`、`pnpm lint/tsc/build`
  全靠人工执行。仓库里有 `rust-toolchain.toml` 钉 nightly + clippy + rustfmt，说明本意是要用这些门禁的。
- **Impact**：F-02 的直接成因（lint 失效无人发现），也是 F-01 没被发现的同因（无人核对构建产物/可达性）；
  F-03 会在 CI 第一次运行时就暴露。
- **Root cause**：单人 + 本地优先项目的自然结果，但门禁因此完全依赖记忆。
- **Recommendation**：最小 CI（GitHub Actions，两个 job）：Rust job（`fmt --check`、`clippy`、`test --workspace`，
  Linux runner 需 `protobuf-compiler`——正好把 F-03 逼出来）、前端 job（`pnpm install --frozen-lockfile`、`lint`、`build`、
  以及一条「`webui/dist` 与源码一致」的构建检查）。不需要发布步骤。

---

**F-12 请求快照与命令回执无界增长，无保留策略**（Confidence: Confirmed）

- **Location**：`model_requests.prompt_snapshot_json` / `context_snapshot_json`；`command_receipts.response_json`；对照组 `event_log.rs`。
- **Observation**：在只跑到第 3 章、35 条模型请求的实测库里：`model_requests` 快照合计 **2,644,282 字节（约 75 KiB/请求）**，
  `command_receipts` **347,623 字节（约 8.5 KiB/命令，且是响应体的完整副本）**，`agent_messages` 93 KiB，
  `document_revisions` 61 KiB，数据库文件 8.2 MB。对照组：`events` 有 7 天保留期并在启动时清理
  （`EVENT_RETENTION_MILLIS`），而 `model_requests` / `command_receipts` / `agent_messages` **没有任何清理或压缩**。
- **Expected invariant**：`docs/02` §11.5 要求「日志滚动保存有限数量文件」，§9.4 要求「已封存请求快照继续保存在本地」——
  即快照要留（诊断价值真实），但本地应用对长期增长应当有口径。
- **Violation**：一部长篇（60 章 × 平均 6 次请求）大致对应 360 条请求 ≈ 27 MB 快照，加上回执与消息表；几十年写作史没有任何回收路径。
- **Impact**：不是正确性问题，是「本地应用能不能一直被用」的问题（启动、备份、导出诊断都会变慢）。
- **Root cause**：快照被当作审计事实保存（合理），但缺一个「谁负责长期回收」的决策。
- **Recommendation**：先**不做**自动清理（会破坏审计链）；改为在数据目录的可诊断位置暴露统计（各表行数与字节数），
  并定一条口径（例如「只对已定稿且已导出超过 N 天的稿本压缩 context 快照」）。把决定写进 `docs/02`，而不是留给未来。

### P3

---

**F-13 前端零自动化测试，且文档承诺的 Playwright 层不存在**（Confidence: Confirmed）

- **Location**：`webui/package.json`（scripts 只有 dev/build/lint/format/typecheck/preview；devDependencies 无任何测试框架）；
  `webui/src` 无 `*.test.*` / `*.spec.*`；全仓 `playwright` 只出现在 docs 两处。
- **Observation**：`docs/02` §15.1 写「Playwright 验证真实界面的关键写作路径」，但仓库里没有 Playwright 依赖、配置或测试；
  `docs/03` 自己把「可驱动 api 的假模型或 Playwright」列为尚未有的测试缺口。
  本轮 F-01 正是这类缺口的直接后果：**两个能力丢失是靠人工读代码发现的，不是靠测试或验收清单**。
- **Impact**：17,239 行前端（含草稿隔离、SSE 游标恢复、IME 处理、虚拟化阅读）的全部正确性依赖人工探针，
  而人工探针清单里没有「可达性」项。
- **Root cause**：验证资源全部投给真实浏览器人工验收（质量高但不留回归护栏），没有留下最小自动化基建。
- **Recommendation**：引入 vitest + @testing-library/react，先只测三个纯逻辑模块：`lib/drafts.ts`（键构造与清除语义）、
  `hooks/use-workspace-draft.ts`（切章归属、dirty→未提交不清除）、`lib/events.tsx` 的 `persistentEventTypes` 与 Rust 清单一致（F-04）。
  再加一条「构建产物包含关键入口文案」的断言（F-01 的回归护栏）。

---

**F-14 `use-workspace-draft.ts` 在 render 体内读写 ref（React 纯渲染约定被违反）**（Confidence: Medium）

- **Location**：`webui/src/hooks/use-workspace-draft.ts`（render 体内 `pendingDraftsRef.current.set(identity, {...})`、
  `clearedValueRef.current = null`；随后在 effect 里 `setReadyIdentity`）；`main.tsx` 已启用 `<StrictMode>`。
- **Observation**：这是 `react-hooks/refs` 那 16 个错误最集中的来源（该文件 10 个）。文件注释表明作者是**有意识地**
  用「render 期同步写入」来避免「切章后把上一章的文字写到下一章」的 effect 间隙——目标是对的，手段是反模式。
- **Expected invariant**：React 约定 render 必须是纯的；`<StrictMode>` 下 render 会被重复执行，React 19 并发渲染可能丢弃一次 render。
- **Violation**：被丢弃/重复的 render 会写入 `pendingDraftsRef`，并可能清掉 `clearedValueRef` 的标记，
  而该标记决定「已确认提交的草稿不要再落盘」。
- **Failure scenario**：一次被丢弃的 render 把 `clearedValueRef` 清空后，紧跟的 effect 可能用「新一章的 identity + 旧文字」
  组一份草稿写进 IndexedDB；或反过来，清标记被跳过导致刚提交成功的草稿又被写回。
- **Confidence 说明**：我**没有**构造出可复现反例（本仓没有前端测试基建）。因此这是「规则违反 + 可信失效模式」，不是已证实 bug。
- **Recommendation**：改成 `useSyncExternalStore` + 组件外小 store（或 `useReducer` 持有 `{identity, text, cleared}`），
  让 render 只读、写入只发生在事件与 effect 里；顺手加 F-13 建议的那两个草稿测试。

---

**F-15 `command_receipts.payload_hash` 覆盖密钥字段，且是无盐 sha256**（Confidence: Confirmed）（与上一轮审计同一项，仍开放）

- **Location**：`store/settings.rs:66`（`SaveSystemModelSettingsPayload` 含 `api_key`）、`store/knowledge.rs:198`（Embedding 同构）、
  `store/lib.rs:1051`（`payload_hash = sha256(json)`）。
- **Observation**：`payload_hash = sha256(json)`，json 里含 `api_key` 正文；`settings.rs` 的头注释写「绝不进入对外视图、回执、
  事件或请求哈希之外」——语义自相矛盾（回执里落的正是覆盖密钥字段的摘要）。
- **Failure scenario**：数据目录被拷走（备份、共享、取证）后，攻击者不必破解密钥，只需对候选密钥做一次 sha256 即可离线确认
  「这把就是那一把」；而且摘要比密钥本身活得更久（密钥换了、摘要还在）。
- **Impact**：低危但真实；属于「密钥的派生材料长期留存」这一仓里唯一一处。
- **Recommendation**：把密钥字段排除在摘要之外（回执仍能阻止同 id 异体重放，因为 `expected_revision` 与其余字段也在摘要里），
  或改为每装一盐的 HMAC 并把盐随库保存。`settings.rs` 那句注释同时要改。

---

**F-16 没有任何路径把 API key 清回「未配置」**（Confidence: Confirmed）

- **Location**：`store/settings.rs`、`store/knowledge.rs`（`api_key: Option<String>` = 保留/替换，无第三种含义）。
- **Observation**：`None` 只有一个含义（保留既有），全仓没有把列置回 NULL 的写路径；`docs/02` §8.4 的删除规则表把它写成
  「模型密钥 | 不单独删除：随所属设置行一起更新，省略即保留」。
- **Expected invariant**：「未配置」是一个**被其它逻辑依赖的合法状态**：`EMBEDDING_NOT_CONFIGURED` 降级、
  界面「未配置」提示、`credential_configured=false` 都依赖它。
- **Violation**：填错一把密钥后只能覆盖，不能回到「未配置」。
- **Failure scenario**：用户想暂停使用 Embedding（或怀疑密钥泄漏想先清掉），必须伪造一把无效密钥，
  于是知识库从「干净的未配置降级」变成「每次检索都网络失败」。
- **Recommendation**：加一个显式清除动作（HTTP 合同里用 `apiKey: null` 与 `apiKey` 缺省区分，或新增 `clearApiKey: true`），
  写事务把列置 NULL 并写事件；界面给「清除密钥」+ 二次确认。§8.4 的表同时更新。

---

**F-17 死代码：两个从未挂载的组件（见 F-01）与三个无人引用的导出 / getter**（Confidence: Confirmed）

- **Location**：`webui/src/components/ServicePanel.tsx`（168 行）、`ShortcutHelpPanel.tsx`（94 行）；
  `webui/src/lib/shortcuts.tsx:81`（`commandKeyLabel`）、`:371`（`useWorkspaceFocusElement`）；
  `crates/nekowriter-agent/src/provider.rs:115`（`api_base_url()`）。
- **Observation**：对 `webui/src` 每个导出做「全文件出现次数 ≤ 1」扫描：4 个候选，其中两个就是 F-01 的两个组件；
  另两个是 `shortcuts.tsx` 里的导出（各只出现一次）。Rust 侧用同样方法扫 `pub fn`，扣掉路由注册的误报（`get(handler)` 不含 `handler(`），
  真正只有定义、没有任何调用点的是 `ChatCompletionsConfig::api_base_url()`（一个未被使用的 getter）。
- **Expected invariant**：`AGENTS.md` 要求「移除过时路径而不是留兼容层」；死代码会让「哪些文件是产品代码」变模糊。
- **Impact**：小（约 270 行）；但 F-01 的两个文件不只是噪音——它们承载的是**已承诺但不可达的能力**，删/接的选择见 F-01。
- **Recommendation**：F-01 决定两个组件的去留；`commandKeyLabel` / `useWorkspaceFocusElement` 与那个 getter 直接删除
  （前者若真要显示 `⌘`/`Ctrl`，应在 `SHORTCUT_ENTRIES` 内部使用而不是导出一个没人用的函数）。
  顺带加一条 ESLint 规则（或在 CI 里跑一次 `knip`/`ts-prune` 类检查），让「新死导出」不再静默累积。

---

**F-18 `docs/02` §16.1 有一句已被同文档 §15.9 与 `docs/03` §38 证伪的陈述；`.agents/skills/**` 被纳入版本控制**（Confidence: Confirmed）

- **Location**：`docs/02-implementation-spec.md:1278`；`.agents/skills/shadcn/**`、`.agents/skills/migrate-radix-to-base/**`（25 个文件）、
  `skills-lock.json`、根目录 `NekoWriter.txt`。
- **Observation**：§16.1 那句「真实模型驱动的准备、细纲、正文、终稿完整端到端验收仍未完成」停留在 2026-09-13 的状态，
  而 §15.9 与 `docs/03` §38/§42/§43/§44 都记录了真实端点的完整闭环。另外仓库共 228 个被跟踪文件，其中 25 个是
  与产品无关的第三方 agent 技能文档（含两张 PNG）。
- **Impact**：读者（含下一次审计）会据此误判「真实端到端验收未完成」，可能把已经做过的事再做一次；技能文档让
  「哪些文件是产品代码」变模糊，也让审计与打包排查多一份无关面。
- **Recommendation**：把该条改为事实（引用 §15.9 与 `docs/03` 的节号）或删除；`.agents/` 与 `skills-lock.json` 移出版本控制；
  `NekoWriter.txt` 移入 `docs/` 并改名（例如 `00-original-requirements.txt`）。
  这类「快照句不随进度更新」是这批文档唯一反复出现的小毛病：§16.1 其它条目也有多条已被后续轮次关闭。

### P4（仅记录，不要求修）

- 前端中文/英文注释混用（`use-workspace-draft.ts` 全英文，其它文件全中文）。
- `webui/src/lib/utils.ts` 只有 1 行。
- 根目录存在 `app.log`（已被 `.gitignore` 忽略，但物理存在）。

---

## 6. Implementation Drift Report

| Contract / Design | Expected | Actual | Drift Type | Impact |
| --- | --- | --- | --- | --- |
| `docs/02` §11/§13/§14、`docs/03` §15.4 与 §16.1、`docs/04` §6.1 | 本地服务面板（退出入口）与快捷键说明面板已实现并被浏览器验收 | 两个组件从未挂载；构建产物中两段文案都不存在；「快捷键」按钮与 `?` 变成空操作；界面无退出入口 | **Implementation ahead of docs 的反面：docs 声明能力存在，实现不可达** | F-01：两处用户可见能力缺失 + 262 行死代码 |
| `docs/02` §2.1 模块职责表 | `core::workflow` / `core::document` / `agent::prompts` / `app::services` / `app::capabilities` / `app::http` 是模块 | 6 个路径不存在；实为 `core/lib.rs` 3739 行与 `app/main.rs` 3939 行 | Docs ahead of implementation | 维护者按文档找位置会扑空；掩盖 F-09 |
| `docs/02` §11.3 | 「写接口只接受 JSON 与自定义请求头」 | 只校验 loopback Host 与（存在时的）Origin；无自定义头 | Docs ahead of implementation | 安全承诺大于实现；写保护靠 57 处手工调用（F-10） |
| 事件合同的隐含前提（§10.4 只失效相关查询键） | 前端订阅集合 = 后端发出的持久类型集合 | Rust 19 个 / TS 15 个；4 个类型前端不订阅（今天被同事务的其它事件掩盖） | Duplicated behavior（跨语言各写一份） | F-04：新增/改名会静默不刷新 |
| 幂等编排（§10.2） | 一条规则、一个收尾器 | `finish_write` 17/49；其余手写等价 6 步 | Partial implementation（半迁移） | F-05：一致性靠约 50 处手工 |
| 事件写入（§10.3） | 单一写入口 | 3 处手搓 `event::ActiveModel`；同域模块 `prompts.rs` 与 `prompt_templates.rs` 写法相反 | Partial implementation | F-06 |
| 设置保存语义（§8/§9） | 两类设置同一组语义与同一条地址校验 | 两套校验（Embedding 侧更松）、两段同构编排 | Duplicated behavior | F-07 |
| `docs/02` §15.1 | 「Playwright 验证真实界面的关键写作路径」 | 无 Playwright 依赖/配置/用例 | Stale contract | 读者以为有 UI 回归网（F-13） |
| `docs/02` §16.1 | 「真实模型驱动的完整端到端验收仍未完成」 | §15.9 与 `docs/03` §38/§42–§44 已记录完成 | Stale contract | 内部互相否定，可能重复劳动（F-18） |
| `docs/03` lint 基线 | 「与基线一致（8 个错误、3 条警告）」 | 实测 30 错误 / 4 警告；UI 重写回合无 lint 记录 | Drifted baseline | 门禁静默失效（F-02） |
| `.cargo/config.toml` / `tools/README` | 干净 checkout 可直接构建 | 非 Windows 必须先手工覆盖 `PROTOC` | Docs ahead of implementation（修法已写在 `docs/03` §37.4，未落地） | F-03 |
| `docs/02` §8.4 删除规则表 | 「模型密钥…省略即保留」 | 事实如此，但没有任何 reset 路径 | Broken lifecycle（表与实现一致，缺能力） | F-16 |
| `docs/02` §9.3 | 密钥不进入请求快照 | 已核：`settings_snapshot` 只含 baseUrl/modelId/capabilities/上下文额度 + 项目引用 id，无密钥 | 无漂移（正向确认） | — |
| `docs/02` §1.2 不变量 15/16/17 | 折叠、消息配对、密钥边界 | 全部追到 enforce 点（见 §10） | 无漂移（正向确认） | — |
| `docs/01` §3.2 D01–D16 | 16 项补充约定 | 抽查项（项目级覆盖、幂等、修订冲突、无自动摘要）与代码一致 | 无漂移 | — |
| `AGENTS.md`「移除过时路径而不是留兼容层」 | 不留兼容层 | `rg -i "keyring\|keychain\|SecKeychain"` 在代码与清单里为空；`Cargo.lock` 少了整棵 keyring 依赖树 | 无漂移（正向确认） | — |

**Drift 的类型分布值得注意：** 绝大多数是「文档比实现多/旧」；本轮新增的一类是**「文档声明的能力在实现里不可达」**
（F-01），而它不是文字问题，是行为问题。这说明这个项目的过程纪律是「先写规则再实现」，
代价是规则句子与结构不随重构回收——因此修复方向是**校准文档 + 在重构里收口旧路径**，而不是改代码逻辑。

---

## 7. Incomplete / Fake-Complete Features

| Feature | Implemented | Missing | Risk | 备注 |
| --- | --- | --- | --- | --- |
| 本地服务面板与退出 | 后端端点、前端组件、退出语义（以后端健康检查为准）都在 | **界面入口未挂载**（F-01） | **高** | docs 记「已验收」 |
| 快捷键说明 | 四条绑定 + Provider + 说明内容都在 | **说明面板未挂载**；按钮与 `?` 为空操作（F-01） | **高** | docs 记「已实现且可由界面发现」 |
| 模型设置 | 保存/读取/不重填保留/连接测试/能力校验 | **清除密钥回「未配置」**（F-16） | 中 | 界面、HTTP、store 三层都缺 |
| Embedding 设置 | 同上（含维度与有限值校验、失败点名具体原因） | 清除密钥；与模型侧校验宽严不一致（F-07）；无可用真实供应商 | 中 | 部分为环境边界 |
| 知识积累 | 注册表、预算、候选写入、事件 | **生产引擎严格为 0** | 低（有意） | 规格 §7.4 明确不承担；界面显示「暂无可用引擎」 |
| 研究/积累具体引擎 | 接口与降级 | 无实现 | 低（有意） | §2.1 已列为「不承担」 |
| Windows / Linux 发行产物 | 打包器 + 硬检查 + 配方 | 真实产物 | 中 | 环境边界，非代码缺口 |
| 前端 UI 路径验证 | 人工真实浏览器验收（质量高） | **自动化回归网**；且验收清单缺「可达性」一项（F-01、F-13） | 高 | 17k 行前端 0 测试 |
| 长篇阅读虚拟化 | 已实现（估计高度 + 测量收敛） | 逐章滚动完整校验 | 低 | §16.1 已写 |
| 持续集成 | — | **整个 CI**（F-11） | 高 | 无 `.github` |
| 幂等合同 | 原语 + 收尾器 + 逐域测试 | **收尾器只覆盖 17/49**（F-05） | 中 | 行为今天正确 |

**没有发现其它「表面完成」的功能**：所有声称完成的 UI 控件（除 F-01 的两项）都能在代码里找到对应的真实后端路径
（「导出 Markdown/TXT」两个入口都打到 `GET /api/versions/{id}/export`；「退回修订」的引用型提交规则在 store 与 API 合同测试里；
知识库/模板/回收站/概览面板都真的挂载在对应页面）。

---

## 8. Redundancy & Simplification Report

判定的三个问题对每一行都问过：**为什么两份都存在？哪一份是正典？删掉会怎样？**

| Area | Redundancy Type | Implementations | Canonical Path | Can Remove? | Risk |
| --- | --- | --- | --- | --- | --- |
| 幂等编排 | Partial implementation（半迁移） | `finish_write`（17 处）vs 手写 6 步（约 49 条命令） | `finish_write` 泛型化 | **可以**，逐模块迁移 | 中（迁移中会暴露漏写的复查/校验） |
| 事件写入 | Partial implementation | `insert_event_row`（经两个包装）vs 3 处手搓 `event::ActiveModel` | `insert_app_event` / `insert_event` | **可以**，3 处各删 ~12 行 | 低（形状逐字段等价，已核） |
| 事件类型名 | Duplicated declaration（跨语言） | Rust 19 个字符串字面量（161 调用点）vs TS 手写 15 项 | Rust `EVENT_TYPES` 清单 | 不能删，**必须生成或校验** | 中（当前 4 个差集被同事务掩盖） |
| 设置保存 | Duplicated behavior | 模型侧 vs Embedding 侧（校验 + 写事务 + app 编排） | 统一地址校验；编排视可行性收敛 | 部分 | 中（校验宽严已不同） |
| 前端死组件 | Dead-but-referenced / Dead | `ServicePanel.tsx`、`ShortcutHelpPanel.tsx` | 无（能力丢失） | **接线或删除**（F-01） | 高（涉及承诺能力） |
| 前端死导出 | Dead | `commandKeyLabel`、`useWorkspaceFocusElement`、`api_base_url()` | 删除 | **可以** | 低 |
| `ApiError::into_response` | Oversized coordinator | 1 个 466 行 match | 按错误域拆文件 | 不删，**拆** | 低（纯移动） |
| `store/preparation.rs` 等 | Oversized file | 13,666 行承载准备/细纲/正文/审核 4 个域 | 按域拆文件 | 不删，**拆** | 低（函数边界已存在） |
| `app/operations.rs` + 两个适配器 | — | 1 策略 + 2 薄适配器 | **已是正典** | **不要合并** | — |
| `normalize_api_key` / `validate_required` | — | 各 1 份、多调用点 | **已是正典** | **不要动** | — |
| 测试 | — | 0 个重名、0 个明显重复、0 个过度 mock | — | — | — |

**逐项工程判断（回答技能要求的三问）：**

1. **幂等编排（F-05）**：两份的成因是「先抽出原语、后抽出收尾器，新模块用新的、老的没回头」。
   正典应是 `finish_write` 的泛型版；**两份语义等价**（逐条比过），所以删掉手写款不改变支持的行为；
   唯一的兼容约束是「事务闭包要对 SeaORM 的 `DatabaseTransaction` 友好」，这一点 `finish_write` 已经解决了。
   收敛会**降低**耦合（命令只声明它的载荷与改动），不会引入新抽象层。
2. **事件写入（F-06）**：成因同上。正典是 `insert_app_event`。两份完全等价（我逐字段比过 `insert_event_row`），
   删掉手搓款不改变行为；**必须先做这一项再做 F-04 的清单生成**，否则清单会漏掉这 3 处。
3. **事件类型名（F-04）**：这不是「两份实现」，是**同一份合同被两侧独立重写**——技能里点名「什么时候合同测试才真的必要」的那一类。
   不能合并（两种语言），只能让一侧成为事实来源：Rust 清单 + 生成/校验 TS。收益明确（消掉一整类静默失效），成本约一个函数 + 一条断言。
4. **设置保存（F-07）**：成因是「新设置照抄旧设置的形状」。**不能**把两类设置合成一张表（实体与生命周期不同），
   但**地址校验**这条业务规则必须只有一份——而且现在两份已经**宽严不同**，这是已经发生漂移的证据。
   app 层编排的收敛是可选优化；若它需要引入比现在更多的参数传递，就只做校验收敛（技能里明确警告过「为了减少行数而抽象」）。
5. **死代码（F-01/F-17）**：成因是重构只做了「新增」没做「删除或接线」。`ServicePanel`/`ShortcutHelpPanel` 的删除会**改变支持的行为**
   （而且改变的方向是「丢弃已承诺能力」），所以它们不是普通死代码，必须先做产品决定。三个小导出/ getter 直接删，无影响。
6. **巨型函数/文件（F-09）**：不是冗余，是**关注点没落在文件边界上**；正典结构已经写在 `docs/02` §2.1（只是写了不存在的模块名）。
   纯移动即可，收益是 review 与改动成本。
7. **`app/operations.rs` 与两个适配器**：明确**不是**冗余——共享策略（状态映射、用量未知原因、失败分类）只写一次，
   两个适配器只做类型翻译。这是本仓应当被推广的范式（`protocol_correction` 的 target 机制同理），**不要**为了「少两个函数」合并。
8. **`insert_event` / `insert_app_event` 两层包装**：保留。它把「事件是否属于稿本」变成签名上的区分，不是空转发。

---

## 9. Engineering Smells / Ad-Hoc Engineering

只列真正重要的，并按「是否值得修」给出判断。

| # | Smell | 位置 | 是否值得修 | 说明 |
| --- | --- | --- | --- | --- |
| S1 | **重构后旧路径未收口**（新增外壳、旧入口悬空） | UI 重构 `cedca66` → F-01 | **值得（P1）** | 不是死代码问题，是能力丢失 |
| S2 | **半套抽象：同一操作两个写法** | 事件写入 3 处手搓（F-06）；幂等 49 处手写（F-05） | **值得** | 正典已存在，迁移成本低、收益是「一致性不再靠复制」 |
| S3 | **跨语言合同靠人工同步** | 事件类型名（F-04） | **值得** | 与 §36 那类事故同类；有具体差集（4 个） |
| S4 | **同一业务规则两份、且已漂移（校验宽严不同）** | 地址校验（F-07） | **值得** | 这是本仓唯一一处「已经漂移」的重复规则 |
| S5 | **横切校验靠每个 handler 记得调用** | `ensure_trusted_write_request` 57 处（F-10） | **值得** | 应做成提取器/中间件；并用它取代 57 处手工 |
| S6 | **巨型协调函数** | `ApiError::into_response` 466 行、`execute_chapter_body_agent_tool_call` 678 行 | **值得** | 纯移动即可改进（F-09） |
| S7 | **文档模块图与文件系统不符** | `docs/02` §2.1（F-08） | **值得** | 改文档即可，成本极低 |
| S8 | **render 期写 ref** | `use-workspace-draft.ts`（F-14） | **值得** | React 约定违反，不是风格问题 |
| S9 | **public API 有隐藏前置条件** | `Store` 的写方法要求先构造 payload 与 `expected_revision` | **不值得改** | 前置条件由类型与 store 内校验共同表达，形状可接受 |
| S10 | **靠调用顺序维持正确性** | 前端「先取水位再建流」、后端「先取许可再读设置」 | **不值得改** | 两处都有明确注释与位置邻近，且前者由 `EventSyncProvider` 单点保证 |
| S11 | **靠注释维持 invariant** | `settings.rs` 头注释（自相矛盾）、多处「调用方负责…」 | 部分值得 | `settings.rs` 那句必须改（F-15）；其余有 store 层校验兜底 |
| S12 | **`let _ =` 吞错** | `scheduler.rs`/`main.rs`/`lifecycle.rs` 的广播发送 | **不值得改** | 全是 fire-and-forget 的 `broadcast::send`，不是吞业务错误 |

**没有发现的草台班子模式**：无 debug/admin 后门端点；无「顺便写库」的旁路（全仓只有一条非 ORM 的原始 SQL，
且是关闭时的 `PRAGMA wal_checkpoint(TRUNCATE)`）；无 feature flag；无被绕过的核心抽象
（提示词、模型解析、回收站、协议纠正都只有一条路径）。**唯一的抽象绕过是事件写入那 3 处（F-06）。**

---

## 10. Missing Invariants

逐条核过 `docs/02` §1.2 的 17 条不变量，找 enforce 点：

| # | 不变量 | Enforce 点 | 结论 |
| --- | --- | --- | --- |
| 1 | 只有后端提交的修订是正式内容 | `document_revisions` 不可变 + `documents.current_revision_id` | 已保证 |
| 2 | 一个稿本最多一个写作入口；一个项目不能同时写两个稿本 | SQLite partial unique index（`uq_runs_active_top_level_project` / `..._version`） | **DB 级保证** |
| 3 | 一个写作任务最多一个未完成工具调用 | runner 阶段守卫 + `tool_calls` 状态 | 已保证（有测试） |
| 4 | 一个工具调用最多应用一次编辑 | `tool_calls.status` + 事务内检查 | 已保证 |
| 5 | 接受编辑必须校验 `base_revision_id` 仍是当前修订 | `StoreError::ProposalBaseRevisionConflict`，4 处 | 已保证 |
| 6 | 审核通过只覆盖该修订及其依赖 | `approved_revision_id == current_revision_id` 判定 | 已保证 |
| 7 | 每条自由意见有持久 ID/序号/投递状态 | `feedback` 表 + `queued→bound→included` + `after_tool_sequence` 锚点 | 已保证（现场证据） |
| 8 | 已发送的请求与消息历史不可原地改写 | `model_requests.status=prepared` 后快照不重写 | 已保证（未见反例） |
| 9 | 状态/正文/工具结果/事件同事务提交 | 各写命令的单事务 + 事务内 `insert_receipt` | 已保证 |
| 10 | 模型/研究/Embedding 请求与用户等待不占 SQLite 事务 | 网络 I/O 在事务外；`settings_snapshot_lock` 不覆盖 I/O | 已保证 |
| 11 | 模型不能绕过工具白名单/预算/引用/阶段守卫 | runner 阶段守卫 + 每阶段单一提交工具 | 已保证 |
| 12 | 软删除对象及其祖先不可用于普通列表/工作流/检索 | 逐层祖先检查 + `RECYCLE_ANCESTOR_DELETED` | 已保证 |
| 13 | 未报告用量是未知而非 0；复制稿本不造字数 | `usage_json` 的未知表达 + `text_changes` 只记真实改动 | 已保证（有测试） |
| 14 | 错误/中断/预算不足/未安装能力不得被解读为通过 | 未安装能力返回 `ENGINE_NOT_AVAILABLE`；失败收束不回填通过 | 已保证 |
| 15 | 阶段完成后历史消息折叠为产物纯文本 | `runs.checkpoint_json.context_fold` + `render_artifact_plaintext` | 已保证（测试 + 现场） |
| 16 | 每条 assistant 工具调用必须有配对结果 | `store::protocol_correction` 与两个 `settle_*`；`serialize_request_messages` 做配对校验 | 已保证，但**靠写入层纪律**（无类型级强制） |
| 17 | 密钥只落两列、只有一个读出口、不进响应/事件/日志/快照 | `system_model_api_key` / `embedding_api_key`；快照已核不含密钥 | 已保证（除 F-15 的回执摘要） |

**真正的隐式不变量（靠约定、代码没有强制的）：**

1. **「每个写 handler 都必须调用 `ensure_trusted_write_request`」**——57 处手工，无中间件、无测试断言（F-10）。
2. **「每条写命令都必须走完整的幂等编排」**——`finish_write` 只覆盖 17/49，其余靠复制模板（F-05）。
3. **「所有事件类型名必须在 Rust 与 TS 两侧保持同步」**——**本条已被证伪为「当前不同步」**：4 个类型前端不订阅（F-04）。
4. **「界面能力在重构后仍然可达」**——**本条已被证伪**：两个已承诺能力不可达（F-01）。
   它从未被写成不变量，也从未被任何检查覆盖，这正是本轮最重要的结构性发现。
5. **「草稿必须按 `(dataDir, versionId, phase, targetId)` 隔离，且提交成功后才清除」**——`docs/02` §13.1 是明文合同，
   实现方式（render 期 ref）违反 React 约定且无测试（F-13、F-14）。
6. **「`model_requests` 快照不得含密钥」**——已核成立，但没有测试锁住（`settings_snapshot` 是内联 `json!`）。
   一旦被破坏，密钥就会进入本地快照与诊断导出。

---

## 11. Data & Lifecycle Review

**Persistence：**`sea-orm` 2 + `sqlx-sqlite`；启动即 `PRAGMA foreign_keys=ON / journal_mode=WAL / synchronous=FULL / busy_timeout=5000`
（`schema.rs:67`），与 `docs/02` §8.3 逐字一致。部分唯一索引由 SeaQuery `Index` 构造器建立（`schema.rs` 30 余处），
初始化不维护手写 `CREATE TABLE/INDEX` 字符串（已核：`schema.rs` 里 `CREATE TABLE` 命中 0 次）。27 个实体表。

**Migration：**有意不做，且这是 `AGENTS.md` 的明文要求。`verify_schema_version` 在缺表时返回 `UnknownSchema`、
版本不符返回 `UnsupportedSchema`，两者都走统一启动失败出口（含数据目录、版本对比、下一步与「作品数据未被读取或修改」的声明）。
**这是一次正确的取舍**：它把「迁移正确性」这类最难长期维护的代码从系统里彻底删掉了，代价是旧数据目录不能被新版本打开。
唯一未覆盖的边界（`docs/03` §41.7 已记）：崩溃遗留**非空** WAL 的目录。

**Lifecycle 缺口：**

| 对象 | create | read | update | delete | restore | purge |
| --- | --- | --- | --- | --- | --- | --- |
| 项目 / 稿本 | ✅ | ✅ | ✅ | ✅ 软删 | ✅ | ✅ 明确拒绝 `RECYCLE_NOT_PURGEABLE` |
| 番外章 | ✅ | ✅ | ✅ | ✅ 软删 | ✅ | 明确拒绝（审计链） |
| 正文计划章 | ✅ | ✅ | ✅ | 有意不支持 | — | — |
| 用户模板 / 知识条目 / 空 Space | ✅ | ✅ | ✅ | ✅ 软删 | ✅ | ✅ 物理删除 |
| 提示词覆盖 | ✅ | ✅ | ✅ | ✅ 清除（物理删行） | — | — |
| **模型 / Embedding 密钥** | ✅ | ✅ | ✅（替换） | **❌ 无路径（F-16）** | — | — |
| events | ✅ | ✅ | — | ✅ 7 天自动清理 | — | — |
| **model_requests / command_receipts / agent_messages** | ✅ | ✅ | — | **❌ 无回收策略（F-12）** | — | — |
| knowledge 向量块 | ✅ | ✅ | ✅ | 软删后由清理任务物理移除 | — | — |

**孤儿数据：**未发现。软删除不批量改写孩子、读取逐层查祖先，恢复只清自己的 `deleted_at`——这套规则在 `docs/02` §8.4
与 `recycle.rs`（1,325 行，单一模块）里一致，且被真实进程探针走过（删除稿本 → 恢复项目不会误恢复它）。

**重复持久化（本轮专查）：**未发现同一事实存两处。`projects.selected_version_id` 是指针而非副本（旧稿本仍可读，有测试）；
`workflow_instances.state_json` 是工作流唯一事实来源，没有与之并行的布尔列；`chapters.status` 与 `approved_revision_id`
是「状态」与「指向」两种事实，不是同一事实的两份表达；dashboard 的用量/字数全部**回算**而不落累计值（`docs/02` §7 有明文）。

---

## 12. Concurrency & Runtime Review

**锁与并发（逐条核过，未发现死锁或 await-under-lock）：**

- 全局模型请求并发度 = 1（`Semaphore(1)`），连接测试与写作 Agent **共用**同一个许可（`main.rs:1790` 附近的注释与代码），
  因此「用户测连接时绕开全局单请求约束」被堵住。
- `settings_snapshot_lock: Mutex<()>` 覆盖「读设置 → 封存请求」，**不覆盖网络 I/O**（`scheduler.rs:665` 注释与位置）。
- 通道全部有界：`LANE_CHANNEL_CAPACITY = 32`、`PREVIEW_DELTA_CHANNEL_CAPACITY = 256`、`broadcast` 用于唤醒。
- 事件流是「SQLite 表是事实来源 + 广播只做唤醒」，接收者滞后不会丢事件。

**崩溃/重启安全（正向确认）：**`Store::open` 会在启动时 `interrupt_active_runs(StartupRecovery)`（`lib.rs:276`），
把已 dispatch 的 Run 标为 `interrupted` 并写原因；「显式继续」在同事务内重授额度。
测试 `startup_recovery_marks_dispatched_runs_interrupted_and_explicit_continue_regrants_allowance` 覆盖。
退出路径（Ctrl+C / SIGTERM / `POST /api/system/shutdown`）在停止接受新连接后取消在途请求、收尾、`wal_checkpoint(TRUNCATE)` 再关库。

**幂等：**同 id 同体 → 返回原回执；同 id 异体 → `IdempotencyConflict` → HTTP 409。
事务内复查覆盖「预查之后、提交之前」的竞争；`command_receipts.command_id` 唯一。
**注意**：该语义的**实现**有 49 份（F-05），**行为**今天一致——这是「结果正确、过程脆弱」的组合。
**未做**的是「崩溃在收到供应商响应之后、写库之前」的自动去重——该情形由 `unknown` 状态与「用量未知」保留（§11.2 已定）。

**未发现的模式**：无 blocking I/O in async、无无限重试、无未取消的 spawn 泄漏（`indexer.abort()`、任务 handle 均被 await 或 abort）。

---

## 13. API & Frontend Contract Review

- **错误信封**统一为 `{ error: { code, message, details, request_id } }`（真实进程探针核对过 404 与 422 的 `request_id`）。
- **状态码映射**集中在 `ApiError::into_response`（466 行）。`RESOURCE_BUSY` → 409、`REVISION_CONFLICT` → 409、
  参数错误 → 422、外部服务 → 502/504。**问题不是不一致，而是它太难改**（F-09）。
- **路由规模**：`router` 里 68 条 `.route()`；真实 axum 路由上的自动化测试只有 5 个（`spawn_app` 在源码中出现 6 次，含定义），
  落在 4 个路径（`/api/knowledge/accumulate`、`/api/capabilities`、`/api/events/cursor`、`/api/events`）。
  **校准（与初稿不同）**：所有写 handler 都收 `Json<TypedRequest>`，而三个「界面载荷形状」测试用的是界面逐字发出的 JSON
  （并额外断言 `answers.answers` 那层错嵌套必须被拒绝），因此**形状类事故已被更精准的测试盖住**；
  真正的空白只是**只存在于 HTTP 层且无断言**的行为：导出 `content-disposition`（含 RFC 5987 中文名，目前**零断言**）、
  幂等重放 vs 409 的状态码映射、`REVISION_CONFLICT` → 409、后台启动 → 202、错误信封 `request_id`、
  `/api/*` 404 与 SPA 回退的区分。建议补**约 6–10 个**用例，不要铺开。
- **前端契约**由 `webui/src/lib/api.ts` 单向声明；无 OpenAPI、无生成式类型，两侧 DTO 靠人工同步（同样的风险类别见 F-04）。
- **已发现并已修的历史不一致**：`answers.answers`（422）；Embedding 设置响应形状（`credential_configured`）。
- **前端状态所有权**：服务端事实在 React Query 缓存（由单个 `EventSyncProvider` 按事件失效），
  未提交编辑在 IndexedDB + 组件内 state，预览增量只在内存。与 `docs/02` §10.4 一致。
  **两处实质性风险**：F-01（失效事件集合与后端不一致）与 F-14（草稿归属的 render 期读写）。

---

## 14. Security Review

| 面 | 状态 | 备注 |
| --- | --- | --- |
| SQL 注入 | 好 | 全仓只有一条原始 SQL（关闭时 PRAGMA）；其余走 SeaORM 查询构造器 |
| 命令注入 | 好 | 无 shell 调用；`xtask` 用 `std::process::Command` 传参数组 |
| 路径穿越 | 好 | 导出文件名经消毒（去分隔符/控制字符/Windows 保留字，折叠点号，限长 80）并有 RFC 5987 头 |
| SSRF | 可接受 | 只允许用户显式配置的模型/Embedding 地址；模型地址必须是带主机的 HTTP(S) 且不能带 query/fragment。
**注意 F-07**：Embedding 侧只检查 `http(s)://` 前缀，比模型侧松 |
| XSS | 好 | 渲染不启用原始 HTML；外链限 http/https |
| CSRF | 基本足够 | 写请求要求 loopback Host + Origin 白名单 + JSON content-type。**但文档声称的「自定义请求头」不存在**（F-10） |
| 鉴权/授权 | 不适用（单用户本机） | 无账号体系；跨项目访问由 store 的归属校验兜底 |
| 密钥处理 | 好，一处例外 | 只落两列、单读出口、不进快照/事件/日志（已核）；**例外是回执摘要覆盖密钥字段**（F-15） |
| 日志泄漏 | 好 | 密钥不出现在日志；错误正文不带密钥 |
| 反序列化 | 好 | 状态 JSON 必须反序列化为 Rust 类型；未知枚举报错 |
| 资源耗尽 | 有界 | 模型响应 2 MiB 上限、有界通道、请求额度、连续协议错误上限。
**缺**显式的 HTTP 请求体上限配置（依赖 axum 默认）与显式服务端超时；`docs/02` §11.3 的自定义头若真要加，应同时收紧这两项 |

**结论**：没有 P0/P1 级安全问题。安全面上真正该修的是 F-10（文档大于实现 + 靠纪律）与 F-15（派生材料长期留存）；
F-07 的校验宽严差是次要但真实的收紧项。

---

## 15. Performance & Resource Review

- **未发现 N+1 或二次方热点**：dashboard 的字数/用量是批量列投影查询（`docs/03` §24 记录了把列投影从「读正文」改为
  「只读统计列」的优化，常驻内存从 192 MB 降到 39 MB）。
- **已知并被文档化的性能边界**：净增量字数需要扫可见文档的全部修订行（SQLite 同页存正文），
  实测 6,010 行 / 75 MB 库下 7 天查询 29–46 ms；概览的 Token/系统操作是「区间读行到内存再汇总」，
  20 万行/80 天时 7 天查询百毫秒级、90 天秒级。这两条都在 `docs/02` §16.1 里，有具体数字，属**已知并接受**的取舍。
- **真正的资源问题是无界增长**（F-12）。
- **前端**：入口 JS 245.84 KB（gzip 79.24 KB），工作台按需 190.67 KB（gzip 55.25 KB），构建 704 ms。可接受。
- **一个新观察**：`webui/dist` 是本轮 F-01 的关键证据来源，但仓库里没有「产物与源码一致」的检查；
  建议 CI 里加一步「重新构建并比对关键入口文案」，顺带覆盖这类「源码里有、产物里没有」的问题。

---

## 16. Test Coverage Gaps

现状：**234 个 Rust 测试**（core 38、agent 44、store 84、app 62 + 2 集成、xtask 4）、**0 个前端测试**、
**1 个集成测试文件**（`crates/nekowriter-app/tests/startup_failure.rs`，跑真二进制）。所有其它测试在 `#[cfg(test)]` 模块内，
但 store 的测试普遍使用真实临时 SQLite（`TempDir` + `Store::open`），因此「单元测试」在实践中是集成级的。

| Critical Invariant | Current Coverage | Missing Test |
| --- | --- | --- |
| 有效模型 = 项目引用优先，能力同源 | ✅ `resolve_effective_model` 单测 + scheduler 用例 | 可选：中间件化后顺带断言「项目引用不支持工具的模型 → 前置 422 且文案点名该模型」 |
| 命令幂等：同 id 同体返回原结果、异体 409 | ✅ store 层各域分支 | HTTP 层的状态码映射（1–2 个端点作模式） |
| 写端点信任校验（Host/Origin） | 部分：`is_loopback_host` 有纯函数单测；**接线无测试** | 正解是提取器/中间件（F-10）而不是逐端点测试 |
| **事件类型集合两侧一致** | ❌ 无 | 断言 Rust 清单 ≡ `persistentEventTypes`（F-04 的回归护栏） |
| **界面能力在重构后仍可达** | ❌ 无（本轮靠人工读代码发现 F-01） | 至少一条「构建产物包含关键入口文案」的断言 + 一份「能力–入口」清单 |
| 密钥不进快照/事件/日志 | ❌ 无 | store 级：保存带密钥的设置后扫描三类快照与日志断言不含密钥 |
| API key 不重填即保留 | ✅ 两个 store 用例 | 路由级：`PUT /api/settings` 省略 `apiKey` → `credential_configured` 仍为 true |
| 阶段完成后折叠为产物纯文本 | ✅ store 用例 + 现场证据 | 无（已足够） |
| 审核通过只覆盖该修订 | ✅ 导出/定稿判定用例 | 路由级：通过后编辑 → 导出标「（草稿）」 |
| 退回意见必须到达正文 Agent | ✅ store 行为用例 + 界面载荷形状用例 | 无（两半都已覆盖） |
| 草稿按 identity 隔离、提交后不复活 | ❌ 无 | 前端：切章/切稿本/提交成功后刷新，断言 IndexedDB 归属正确 |
| 单条 SSE 流与水位恢复 | ❌ 无 | 前端：连接重置后清空预览、失效缓存、按新水位重连 |
| 活跃写作 Run 唯一 | ✅ DB partial unique index | 无（DB 级已足够） |
| 启动失败统一出口 | ✅ 2 个集成测试跑真二进制 + 纯函数单测 | 只读文件系统与 schema 不匹配已有实测证据但未进集成测试 |
| 导出文件名消毒 + `content-disposition` | ❌ 无（只有生产代码 `main.rs:1937`） | 纯函数单测 + 1 个路由用例 |
| `/api/*` 未命中 vs SPA 回退 | ❌ 无 | 1 个用例 |

**测试假象评估：**没有发现 mock 过度、无断言或重复的用例（跨 crate 重名测试 0 个）。
存在**相反的问题**：一些测试用 SQL 直接预置行（`docs/03` §22.3 明确写过），这是为构造时间区间等边界的合理手段，
但读者需要知道它们验证的是「读路径」，不验证对应的「写路径」。

---

## 17. Dead / Duplicate / Legacy Paths

- **死代码（本轮新发现）**：`ServicePanel.tsx`、`ShortcutHelpPanel.tsx`（见 F-01/F-17），
  以及 `shortcuts.tsx` 的两个无人引用导出与 `ChatCompletionsConfig::api_base_url()`。
- **无旧实现残留**：`rg -i "keyring|keychain|SecKeychain"` 在代码与清单里为空；
  `credentials.rs` 已随 keyring 移除删除；`Cargo.lock` 少了整棵 keyring 依赖树（30 个包）。
  `rg -i "legacy|deprecated|fallback"` 只命中一个测试名（`project_settings_ignore_a_legacy_model_id_field`，
  它断言「未知字段被忽略」这一**当前**行为，不是兼容层）。
- **重复路径**：F-05（幂等编排 49 份手写 + 一个只用了 17 次的收尾器）、F-06（事件写入 3 处绕过）、
  F-07（两条设置命令）、F-04（事件类型名跨语言双写）。
- **半套抽象**：`finish_write`（F-05）、`insert_event`/`insert_app_event`（F-06）。
  **反例（健康）**：`protocol_correction` 的 target 机制是**完整**抽象——写作 lane 与审核子 Run 共用配对消息构造、
  计数策略、事件字段与达上限判定，`docs/02` §2.1 甚至写明「新增 lane 只需再加一个 target，不得再复制一份事务」。
- **旁路执行路径**：未发现 debug/test/admin 端点；未发现 CLI 与 HTTP 走不同校验的路径（`xtask` 只做打包，不碰业务表）。

---

## 18. Documentation Drift

除 §6 的表格外，逐份文档的状态：

| 文档 | 状态 | 需要修的点 |
| --- | --- | --- |
| `docs/01-implementation-plan.md` | 大体准确；§2.1 进度快照与 §6.6 界面状态清晰 | 无新增漂移 |
| `docs/02-implementation-spec.md` | 质量最高的文档；承载全部不变量与边界 | 需修：§2.1 模块表（F-08）、§11.3 自定义头（F-10）、§11/§13/§14 的服务面板与快捷键说明（F-01）、§15.1 Playwright（F-13）、§16.1 stale 句（F-18）、§8.4 密钥行需补「清除」（F-16）、§9.3 地址校验归属（F-07） |
| `docs/03-code-review.md` | 278 KB 的验收日志，证据密度高（数据库行、哈希、报文） | 需修：lint 基线数字（F-02）、§15.4 与 §16.1 的服务面板/快捷键记录（F-01）；§37.4 的 PROTOC 修法未落地；历史节保留原样即可（它是日志，不是规格） |
| `docs/04-ui-ux-refactor-plan.md` | 已完成计划 + §6.1 验收记录 | 需补：验收清单加 `pnpm lint`、加「既有能力可达性」一项（F-01/F-02） |
| `docs/05`（本报告） | 第二轮，替换第一轮草稿 | — |
| `NekoWriter.txt` | 原始需求，含 D01–D16 确认口径 | 与 `docs/01` §3.2 一致；建议移入 `docs/`（F-18） |

**一个系统性观察**：`docs/02` §16.1 有三十余条「尚未覆盖的边界」，其中**多条已被后续轮次关闭**，
但条目本身没有勾掉。这份列表目前读起来像「全部未覆盖」，实际是「历史边界清单」。
建议把已关闭的条目划掉或标注关闭日期，否则它作为风险清单的价值会逐年衰减。

---

## 19. Positive Observations

具体、可验证、且是这份代码真正的价值所在：

1. **有效模型解析只有一处实现，且被 HTTP 与调度器共用**：`app/model_settings.rs` 的头注释明确写了
   「避免用系统模型的能力校验项目引用的模型」，两个调用点都真的在用它。这类「同一判定两处使用」的设计在真实项目里很少见。
2. **提示词解析链唯一**：`resolve_prompt` 实现 project → system → builtin；全仓没有第二处 `builtin_prompt` 旁路（除测试）。
3. **协议纠正的 target 机制**：`ProtocolCorrectionTarget` + 两个 `settle_*` 收束函数，写作 lane 与审核子 Run 共用
   配对消息构造、计数策略、事件字段与达上限判定——这是**有约束力的设计**，不是口号。
4. **系统操作落库策略只写一次**：`app/operations.rs`（83 行）定义状态/用量/错误的统一翻译，
   模型与 Embedding 各写一个薄适配器。这是本仓**最值得推广**的范式，也是判定 F-07 时「不要合并、只要收敛」的依据。
5. **DB 承载业务不变量**：活跃顶层 Run 的部分唯一索引（按项目与稿本各一条，条件含 kind 与 status 集合）、
   `documents` 按 `chapter_id IS NULL / IS NOT NULL` 分别建部分唯一索引（绕开 SQLite 对 NULL 的处理）、`feedback(run_id, sequence)` 唯一。
6. **崩溃恢复真的接了**：`Store::open` → `interrupt_active_runs(StartupRecovery)`，并有专门用例断言「显式继续重授额度」。
7. **启动失败诊断的完整实现**：`StartupStep` + `startup_failure_message` 纯函数 + 六种失败场景实测 +
   跑真二进制的集成测试；拒绝打开旧 schema 时主库 SHA-256 前后一致——「不改动数据」是被证明过的，不是被声称的。
8. **密钥边界**：只落两列、单读出口、不进快照/事件/日志（已核），并在 `docs/02` §1.2 里升格为编号不变量。
9. **文档纪律**：`docs/02` §16.1 三十余条未覆盖边界、`docs/03` 每轮带数据库行与哈希的验收节、
   连「打包器架构不匹配时旧发行目录逐字节未动」这种反例都被记录。
10. **代码卫生**：0 个 TODO/FIXME/HACK；`unsafe_code = "forbid"`；`panic!` 只在测试；只有一条非 ORM 原始 SQL；
    跨 crate 重名测试 0 个。
11. **打包器的硬检查**：读可执行文件容器格式**与架构**，并在**任何构建之前**检查目标标准库是否安装。

---

## 20. Technical Debt Map

### Immediate（下一次改动之前就值得做，收益/成本比最高）

1. **F-01**：决定 `ServicePanel` / `ShortcutHelpPanel` 的去留并落地（推荐接线：恢复退出入口与快捷键说明面板）。
   这是唯一一条**用户可见的能力缺失**，且它已经在文档里被写成「已验收」。
2. **F-03**：`.cargo/config.toml` 的 `[env]` → `[target.'cfg(windows)'.env]` + `tools/README.md` 写清非 Windows 的 protoc 获取方式。
3. **F-11 + F-02**：加最小 CI（Rust job + 前端 job），让 `pnpm lint` 回到 0（含 F-14 的真正修复），
   并在验收清单里固定「lint」与「既有能力可达性」两行。
4. **文档校准（F-18 / F-15 注释 / F-16 表格行 / F-08）**：成本极低，且能立刻消除「文档与实现互相否定」的状态。

### Near-term（未来几个迭代）

5. **F-06 → F-04**：先把 3 处手搓事件写入迁到 `insert_app_event`，再建 Rust `EVENT_TYPES` 清单并校验 TS 集合。
   顺序不能颠倒——否则清单会漏掉那 3 处。
6. **F-05**：把 `finish_write` 泛型化并**按模块**迁移约 49 条命令（`model_requests.rs` → `preparation.rs` → `lib.rs`）。
7. **F-10**：把 `ensure_trusted_write_request` 做成提取器/中间件，用构造取代 57 处手工调用；
   同时把 `docs/02` §11.3 的措辞改成事实。
8. **F-07**：统一 API 根地址校验（一份严格实现、两处使用），消除已验证的「校验宽严不同」漂移。
9. **F-12 + F-16**：为三张只增不减的表定保留口径；补「清除密钥」生命周期（store + HTTP + 界面 + 文档）。
10. **HTTP 层 6–10 个用例**：导出头与文件名消毒、幂等重放 vs 409、`REVISION_CONFLICT` → 409、202、`request_id`、`/api/*` 404。

### Long-term（架构演进）

11. **F-08 + F-09**：让模块图与文件系统一致。先改文档，再做**只移动不改行为**的拆分：
    `core::workflow` / `core::document`、`app::http`（含按域拆分的 `ApiError`）、`scheduler` 按 lane 拆分。
12. **F-13**：前端最小测试基建（vitest + testing-library），先覆盖草稿、事件集合、能力可达性三块。
13. **F-17 的机制化**：在 CI 里加一次「无用导出 / 死文件」检查，让这一类问题不再只靠人工审计发现。
14. **仓库清理**：`.agents/skills/**` 与 `skills-lock.json` 移出版本控制；`NekoWriter.txt` 归入 `docs/`。

---

## 21. Simplification Opportunities

只列有明确工程价值的简化，并写明「当前复杂度 → 正典设计 → 可删什么 → 收益 → 风险」。

1. **幂等编排（F-05）**
   - 当前复杂度：约 49 条命令各自手写 6 步（`payload_hash` 53 / `find_receipt` 55 / `replay_or_conflict` 54 /
     `insert_receipt` 49 / `replay_after_receipt_conflict` 33）。
   - 正典设计：`finish_write` 泛型版（`(method, target, payload, |tx| …)`）覆盖全部命令。
   - 可删：约 49 处重复的编排样板（保守估计 800–1,200 行）。
   - 收益：一条规则一份实现；迁移过程会暴露漏写的事务内复查 / 修订校验。
   - 风险：迁移期间可能改变个别命令的**错误顺序**（先报冲突还是先报修订），需要逐模块跑测试。

2. **事件写入与事件类型清单（F-06 → F-04）**
   - 当前复杂度：2 套写入口 + 19 个裸字符串类型散在 161 个调用点 + TS 手写 15 项。
   - 正典设计：写入口只有 `insert_event` / `insert_app_event`；类型来自 `EVENT_TYPES` 清单；TS 侧由清单生成或校验。
   - 可删：3 处手搓写入（≈36 行）+ 前端手写数组（改为生成后不留手写副本）。
   - 收益：消掉整类「两侧不同步、前端静默不刷新」的失效模式。
   - 风险：把 `settings.changed` 这类经手搓入口发出的类型纳入清单时，要确认前端订阅集合确有它（有）。

3. **设置命令的公共校验（F-07）**
   - 当前复杂度：两条地址校验规则（宽严不同）+ 两段同构编排。
   - 正典设计：`validate_api_base_url`（严格版）一份、两处使用；app 层编排是否收敛视可行性而定。
   - 可删：一份重复校验（≈20 行）；**不**追求合并两张表。
   - 收益：一条安全/合同规则一份实现，且顺带收紧 Embedding 侧。
   - 风险：收紧校验可能让当前「能保存」的宽松地址变成拒绝——需要一次真实界面复验（F-07 的验收条件）。

4. **巨型函数与文件（F-09）**
   - 当前复杂度：单文件 13,666 行 / 6,803 行 / 6,603 行；单函数 678 行 / 666 行 / 548 行 / 466 行。
   - 正典设计：按域拆文件（`ApiError` 按错误域、`scheduler` 按 lane、`preparation` 按阶段）。
   - 可删：不删行为，只移动（配合 `docs/02` §2.1 改成事实）。
   - 收益：review 成本、改动风险、合并冲突面同时下降。
   - 风险：低（纯移动 + `cargo test`），但必须一次一域、每步验证。

5. **文档死重（F-08 / F-18）**
   - 当前复杂度：`docs/02` §2.1 六个不存在的模块、§16.1 已关闭的旧条目、`docs/03` 的过期 lint 基线。
   - 正典设计：模块表写事实；§16.1 标注关闭日期；把「本轮的 bug/修复」与「仍有效的边界」拆成两份清单。
   - 收益：审计与接手成本立刻下降。
   - 风险：无。

---

## 22. Recommended Fix Order

按「根因 + 依赖 + 能顺带删掉多少重复路径」排序：

1. **先让门禁能说话**：F-03（`PROTOC`，1 行）→ F-11（最小 CI）→ F-02（lint 归零 + F-14 的真正修复 + 两个草稿/事件集合测试）。
   理由：`PROTOC` 挡住干净构建，CI 不存在，于是 lint 的失效没人发现——这三件事互为因果，必须一起做完，
   否则后面每一步重构都没有信号。做完这一步，「以后还能不能发现回归」才有答案。
2. **决定并收口 F-01**：接线（推荐）或删除，并同步 `docs/02` §11/§13/§14、`docs/03` §15.4 与 §16.1、`docs/04` §6.1；
   在验收清单里补「既有能力可达性」一行。理由：这是**唯一一条用户可见的能力缺失**，且它同时消掉 262 行死代码。
3. **把靠纪律维持的不变量变成构造保证**：F-10（写信任做成提取器，一次性取代 57 处手工）
   → F-06（3 处手搓事件写入归并）→ F-04（事件类型清单 + TS 校验）→ F-05（幂等编排收口并按模块迁移）。
   顺序有依赖：F-06 必须在 F-04 之前（否则清单漏类型）；F-05 放最后（改动面最大，需要 CI 与测试先到位）。
4. **补最小的一组 HTTP 层用例**（导出头与文件名消毒、状态码映射、202、`request_id`、`/api/*` 404）。
   只盖「只存在于 HTTP 层」的行为，不铺开形状测试。
5. **收敛重复的校验规则（F-07）**，顺带收紧 Embedding 地址校验并做一次真实界面复验。
6. **长期项**：F-08 + F-09（结构对齐与拆分）→ F-12 / F-16（生命周期）→ F-13（前端测试基建）→ F-17 / F-18（仓库清理）。

**为什么这个顺序**：1 是**测量能力**，2 是**产品承诺**，3 是**构造保证**（收益最大，也最容易被误以为「测试」取代），
4 是补上测量里的真空白，5 是把已经漂移的重复规则收回一处，6 是长期演进。
把 F-05 这类大面迁移排在 CI 之前，等于在没有保护的情况下挪动约 50 处一致性代码——这正是这份仓里最不该出现的组合。

---

## 23. Final Verdict

**NEEDS HARDENING**

理由（不是「代码差」，恰恰相反）：

- **运行时正确性、数据完整性、安全边界目前是被保护的**，而且有高质量的真实进程证据：17 条不变量逐条追到 enforce 点，
  其中多条由数据库约束直接保证；没有发现 P0/P1 级缺陷（无数据破坏、无安全阻塞项、无不可恢复的失败模式）。
  幂等、乐观并发、事件原子性、崩溃恢复、密钥边界、启动失败诊断都是**真的**。
- **但「谁保证它继续对」这一层是破的，而且本轮被证明已经漏过东西**：UI 重构丢掉两个已承诺的界面能力
  （F-01，靠人工读代码才发现）；前端最关键的验证门禁（lint）静默失效一整轮重写（F-02）；
  17,239 行前端零自动化测试（F-13）；68 条路由只有 4 条被真实路由测试覆盖；
  没有 CI（F-11）；非 Windows 干净构建连第一步都过不去（F-03）。
  这些问题不会在今天出错，它们会在**下一次改动**时出错，而且出错的方式恰好是最难现场定位的那种
  （点了没反应、草稿丢一次、契约 422 而测试全绿、界面不刷新）。
- **「改动落在哪里」也在劣化，且本轮量化了**：文档承诺的模块分解有 6/9 不存在（F-08）；
  三个文件超过 6,000 行、四个函数超过 460 行（F-09）；约 49 条命令手写幂等编排而收尾器只用了 17 次（F-05）；
  事件写入有 3 处绕过正典抽象（F-06）；事件类型名跨语言双写且已有 4 个差集（F-04）；
  两条设置命令各有一份校验、且宽严已经不同（F-07）。

因此这不是 `HEALTHY WITH DEBT`（债存在但门禁在——这里门禁不在），
也不是 `SIGNIFICANT REWORK REQUIRED`（核心设计是对的、分层是真的、不存在需要推倒的部分）。
它是一个**设计正确、实现扎实、但验证与重构收口没有跟上规模**的系统：
补齐 §22 的第 1–3 步之后，它就可以稳定地进入 `HEALTHY WITH DEBT` 甚至 `HEALTHY`。

---

## 附：本轮实际执行的验证（只读）

```bash
# Rust
cargo --config 'env.PROTOC.value="/opt/homebrew/bin/protoc"' test --workspace
#   → 234 通过 / 0 失败（agent 44、app 单元 62 + 集成 2、core 38、store 84、xtask 4）
cargo fmt --all -- --check                        # → 干净
cargo --config '…' clippy --workspace --all-targets
#   → exit 0；244 条 warning 行（去重约 234 条）/ 0 errors
#     too_many_lines 86、missing_errors_doc 60、needless_update 38、needless_pass_by_value 11…

# 前端
npx eslint . -f json   # → 30 errors / 4 warnings（refs 16、set-state-in-effect 8、react-refresh 6）
                       #   按文件：use-workspace-draft 10、PreparationPanel 4、ChapterBodyPanel 3…

# 结构、冗余与死代码（本轮新增的取证）
rg -n "ServicePanel|ShortcutHelpPanel" .                      # → 只命中两个文件自身与 docs
rg -n "shutdownService|SHORTCUT_ENTRIES" webui/src            # → 唯一使用点都在那两个未挂载文件里
rg -l "保持浏览器原生行为|确认退出|整稿查找" webui/dist        # → 全部为空（产物 09:55，比所有 src 新）
find webui/src -newer webui/dist/index.html | head             # → 空
#   Python 抽取 insert_event/insert_app_event 的类型实参 → 18 个（+ 3 处手搓 = 19）
#   TS persistentEventTypes → 15 个；差集 4 个，逐站点确认同事务内有被订阅事件
for f in payload_hash replay_or_conflict find_receipt insert_receipt replay_after_receipt_conflict finish_write; do … done
#   → 53 / 54 / 55 / 49 / 33 / 17（收尾器只覆盖 17）
rg -n "event::ActiveModel" crates                             # → 生产代码 3 处绕过 + 测试 2 处

# 文档漂移
rg -n "core::workflow|app::services|app::http" docs/02         # → §2.1 表仍写着不存在的模块
rg -n "自定义请求头" docs/02                                   # → §11.3 仍承诺，代码里不存在
rg -n "真实模型驱动的准备、细纲、正文、终稿完整端到端验收仍未完成" docs/02   # → 第 1278 行仍是旧口径
rg -i "keyring|keychain|SecKeychain" crates webui/src Cargo.toml Cargo.lock  # → 空
```

**未执行的验证（并已在上文标注原因）**：真实浏览器点击「快捷键」/退出入口（无可用的运行实例）、
Windows/Linux 发行产物、真实第三方 Embedding 供应商、知识积累生产引擎、大数据量性能、
前端并发渲染下的可复现反例、事务中途 kill -9 的故障注入。
