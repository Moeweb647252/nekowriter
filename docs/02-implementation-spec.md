# NekoWriter MVP 实现细则

文档状态：实施基线与进度清单，D01–D16 补充约定已全部由用户确认；勾选项表示代码已落地，不等同于真实外部模型验收完成。  
编写日期：2026-09-07。  
配套文档：[`实现方案`](01-implementation-plan.md)。原始需求见 [`NekoWriter.txt`](../NekoWriter.txt)。

本文中的“必须”是实施基线内的约束；方案 §3.2 的 D01–D16 共 16 项补充约定已全部由用户确认，本文规定其实现行为与验收标准，后续实施直接应用。其中 D16 遵循 §9.4 的产物纯文本折叠规则。代码实现与本文冲突时，应先解决冲突，不能让实现暗中改变产品语义。

## 1. 术语、所有权与不变量

### 1.1 术语

| 术语 | 定义 |
| --- | --- |
| Project / 项目 | 设置与稿本的容器，项目名称不等于小说标题 |
| NovelVersion / 稿本 | 独立小说版本，拥有标题、准备产物、章节、工作流与正文完本标记 |
| Document / 文档 | 背景、资料、人物、粗纲、细纲、正文等可被审核和引用的内容对象 |
| Revision / 修订 | 文档的一次不可变提交，旧修订不得原地覆盖 |
| Proposal / 编辑提案 | 基于明确正文修订提出的 append/replace，尚未被接受 |
| Run / 执行任务 | 某阶段的 Agent 执行记录；一次 Run 包含多次模型请求 |
| ModelRequest / 模型请求 | 一次实际请求尝试及其配置、上下文、用量和结果记录 |
| ToolCall / 工具调用 | 模型提供完整参数后形成的调用，包含工具名、调用 ID、状态与结果 |
| Review / 审核 | 针对一个目标修订或编辑提案的决定，可由用户或受托 Agent 作出 |
| Feedback / 自由意见 | 自动接受模式等场景下排队交付的意见，不等于正式审核或问题答案 |
| QuotaScope / 预算作用域 | 一个准备阶段或一章细纲的辅助工具预算账本 |

### 1.2 必须成立的不变量

1. 只有后端提交的文档修订才是正式内容；SSE 预览和待审提案不算正式正文。
2. 一个稿本同一时刻最多有一个写作执行入口；同一项目不能同时写两个稿本。
3. 一个写作任务最多有一个未完成的工具调用；委托审核是独立子任务，不获得写作权。
4. 一个工具调用最多应用一次编辑；一个审核最多产生一次终态决定。
5. 接受编辑必须校验提案的 `base_revision_id` 仍为当前修订。
6. 审核通过只覆盖审核时的修订及其依赖；新修订不继承旧修订的通过状态。
7. 每条自由意见有持久 ID、顺序号和投递状态，不凭 UI 清空输入框认定送达。
8. 一个已经发送的模型请求及其消息历史不可被原地改写。
9. 状态、正文变更、工具结果和对应持久事件在同一业务事务中提交。
10. 模型、研究、Embedding 请求和用户等待不占用 SQLite 事务。
11. 模型决定不能绕过工具白名单、预算、引用所属稿本及阶段守卫。
12. 软删除对象及其祖先不可从普通列表、工作流或知识检索中使用。
13. 未报告的 Token 用量是未知，不是 0；复制稿本不制造创作字数增量。
14. 错误、中断、预算不足和未安装能力不得被解释为审核通过或阶段完成。
15. 当前阶段全部必需产物审核通过后，该阶段历史消息在后续 Agent 上下文中必须折叠为仅含产物全文的纯文本；未通过阶段不得提前折叠，原始本地记录不删除。

## 2. 模块与执行模型

### 2.1 模块职责

| 模块 | 职责 | 不承担 |
| --- | --- | --- |
| `core::workflow` | 嵌套状态、命令校验、阶段守卫、产生业务决策 | 数据库连接、网络调用 |
| `core::document` | 编辑校验、字数、引用位置、修订依赖 | React DOM、ORM Entity |
| `agent::runner` | 构建一次请求、解析模型结果、请求工具执行、识别结构化阶段提交 | 自行持久化正文或批准审核 |
| `agent::provider` | 一种模型协议适配器、流解析、能力声明与用量映射 | 推测所有供应商都支持同一参数 |
| `agent::prompts` | 内建提示词目录、变量声明、模板渲染、上下文编排 | 保存项目覆盖值 |
| `store` | SeaORM Entity、事务、查询、SQLite 初始化、LanceDB 索引 | 决定某章是否通过审核 |
| `app::services` | 串行命令处理、事务编排、权限范围、工具 Host、任务恢复 | 通用流程脚本解释器 |
| `app::capabilities` | Embedding、研究与积累注册表、预算封装 | MVP 中的研究/积累具体引擎 |
| `app::http` | DTO 校验、API、SSE、错误映射、静态页面 | 在 HTTP handler 中等待用户回答 |

Core 的决策函数以状态、命令、已读取的领域输入为参数，返回待提交的状态与变更；App 在同一短事务中检查修订并提交。不要把整套系统做成历史事件回放才能读取的 Event Sourcing：普通页面直接查询当前表，事件用于同步与诊断。

### 2.2 每稿本命令循环

使用一个 Tokio 任务和有界命令通道管理一个活跃稿本。数据库仍是事实来源，内存任务只是串行化执行。API 命令、模型完成回调、审核完成回调都进入同一命令序列。

命令循环不能在 `await` 模型网络结果或用户回答时阻塞自身。它提交 `request_in_flight` 后启动请求任务，继续处理暂停、意见、模式切换和设置命令；请求结束后通过回调命令处理结果。

全局模型请求并发度为 1，队列按入队顺序调度。等待用户和父任务等待审核不占许可；取消任务移除尚未发出的请求。知识索引使用独立单任务队列，不能长期占住模型生成许可。

请求真正取得许可后，才解析最新设置、提示词和可用工具并封存请求；不能在排队时冻结配置。封存请求与反馈绑定属于一个串行命令边界；其后的设置修改应用于下一请求。

### 2.3 工程边界

HTTP 超时、响应大小、单个工具参数大小和循环轮次使用集中定义的工程限制。建议初值：完整工具参数上限 256 KiB；单次模型响应上限 2 MiB；初始及每轮重新获准执行的请求额度各为 100 次；连续 3 次协议错误后停止自动推进并等待用户处理。这些限制不等于辅助工具 quota，界面必须说明具体触发原因。

Run 分开保存累计 `request_count` 和本轮允许达到的累计上界 `request_allowance_end`，初始为 0 和 100。只有 `request_count < request_allowance_end` 才能发送下一次模型请求；在同一事务中将新 attempt 标为 dispatched 并使 request_count 加 1，之后再执行网络 I/O。prepared 后取消的请求不计次，dispatched 后失败或结果未知的请求仍计次；同一 attempt 的重复回调不增加次数。最后一个额度内请求的响应、工具结果和审核仍完整处理，额度耗尽只阻止下一次模型请求。

额度耗尽或连续协议错误触发停止时，Run 进入 stopped，记录 `wait_reason = request_limit|protocol_error_limit`。用户显式继续 stopped/failed/interrupted 的原 Run 时，同事务设置 `request_allowance_end = request_count + 100`、清零连续协议错误数、增加 resume_count 并取得执行槽。累计请求数、Token、工具结果和辅助 quota 消耗保持不变。重复 command_id 返回原回执；Run 已恢复后再次提交新的继续命令返回状态冲突，不叠加额度。

普通用户暂停的恢复、提交意见、回答问题、接受编辑、审核修订、网络重连及进程启动均不续授额度。UI 在额度耗尽时显示“本轮请求额度已用完，继续可再执行最多 100 次”，由用户明确继续；修改设置本身不重启任务。

模型请求超时区分连接、首包和流空闲时间，不用一个过短的总超时终止正常长输出。初值在 M1 实测后锁定并记录；请求失败不自动换模型、协议或研究引擎。

## 3. 分层状态机与工作流

### 3.1 后端状态结构

```text
Workflow
├── initializing
├── preparing
│   ├── generating
│   ├── reviewing
│   └── revising
├── writing
│   ├── chapter_ready
│   ├── outline_generating
│   ├── outline_reviewing
│   ├── drafting
│   ├── chapter_reviewing
│   └── chapter_done
├── completed
└── extras
    └── 复用 writing 的章节子状态，chapter.kind = extra

Execution（独立于 Workflow）
├── idle
├── queued
├── request_in_flight
├── tool_boundary
├── waiting_answer
├── waiting_review
├── paused
├── interrupted
├── failed
└── stopped
```

`Execution` 附带 `run_id`、`request_id`、`pending_tool_call_id`、`wait_reason` 和恢复位置；控制数据为 `approval_mode` 与 `pause_requested`。等待审核的具体目标在 Review 中，不再以多个布尔值复制。

`stopped` 停止本次执行，不删除正文，也不表示小说完本；用户继续时复用原 Run ID、上下文、工具调用、意见和辅助工具预算，并按 §2.3 续授模型请求额度。只有需要下一次模型调用时才创建新 attempt；已有待答问题或待审提案先恢复原等待状态。`interrupted` 表示进程意外退出等造成的执行中断，见 §11。

顶层 Run 按准备、每章细纲、每章正文分别建立，审核拒绝后的修订仍属于原 Run；只有对应阶段审核通过且产物纯文本折叠检查点已提交，才标记 succeeded。审核委托是子 Run；阶段上下文折叠由应用服务完成，不创建模型子任务。Run 的汇总状态为 queued/running/waiting/paused/interrupted/stopped/failed/succeeded，由 Execution 映射得到，二者不可分别任意修改。

### 3.2 主要转换

| 当前工作流 | 输入 | 守卫与动作 | 下一工作流 |
| --- | --- | --- | --- |
| initializing | 初始化成功 | 项目、稿本、目标输入槽、四类文档槽、工作流、预算作用域同事务就绪；执行轴 idle，等待用户开始 | preparing.generating |
| preparing.generating/revising | 提交准备产物 | 初次四项齐全；修订轮提交需要修订的项；校验粗纲章节计划 | preparing.reviewing |
| preparing.reviewing | 本轮审核完成 | 四项当前修订及依赖全部有效通过，提交四项产物纯文本折叠 | writing.chapter_ready |
| preparing.reviewing | 本轮审核完成 | 存在拒绝或失效项，持久化意见和待修订集合 | preparing.revising |
| writing.chapter_ready | 开始本章 | 存在计划项；指导已保存；首章要求准备阶段折叠已提交，后续章要求前章通过且正文阶段折叠已提交 | writing.outline_generating |
| writing.outline_generating | 提交细纲 | 当前章、粗纲修订和指导一致 | writing.outline_reviewing |
| writing.outline_reviewing | 通过 | 目标修订仍然有效，提交细纲全文折叠 | writing.drafting |
| writing.outline_reviewing | 拒绝 | 保留意见，复用本章细纲预算 | writing.outline_generating |
| writing.drafting | 提交本章 | 无未完成工具、无未投递意见、无暂停请求；正文非空 | writing.chapter_reviewing |
| writing.chapter_reviewing | 通过 | 当前正文及依赖未变；锁定本章通过修订，提交正文全文折叠 | writing.chapter_done |
| writing.chapter_reviewing | 拒绝 | 把意见加入该轮修订上下文，保留正文历史 | writing.drafting |
| writing.chapter_done | 进入下一章 | 尚有未完成计划章；进入指导界面，不自动调用模型 | writing.chapter_ready |
| writing.chapter_done | 确认完本 | 所有正文计划章通过，无开放审核/编辑/执行请求 | completed |
| completed | 新建番外 | 创建 extra 章节、细纲预算和指导槽；保持正文完本时间 | extras.chapter_ready |
| extras.chapter_done | 结束本篇番外 | 该番外通过，无待处理内容 | completed |

阶段通过时按 §9.4 同步生成产物纯文本并保存折叠检查点，之后才允许下一阶段请求。该操作不调用模型、不生成章节记忆摘要；`chapter_done` 可以直接进入下一章指导或等待确认完本。

所有不合法转换返回 `INVALID_TRANSITION` 和当前快照，前端不能通过隐藏按钮代替后端校验。

### 3.3 准备四项审核与依赖

准备产物的稳定键为 `background`、`research`、`characters`、`outline`。Tab 顺序采用背景、资料、人物、粗纲，不决定 Agent 的生成顺序。

第一轮全部产物持久化后创建四个审核。用户可按任意顺序审核；本轮所有开放审核作出决定后统一进入修订或正文。修订轮只审核新修订和失效项，不要求重审未受影响项。

固定依赖关系：资料 → 背景、人物、粗纲；背景 → 人物、粗纲；人物 → 粗纲。每个依赖保存被引用的具体修订 ID。同批提交先确定全部新修订，再一次性记录依赖，避免批内先后顺序产生误失效。

任一来源改变，引用旧来源的下游审核标记为 `stale`。MVP 不判断“这次改动是否语义无关”；需要用新依赖重新提交下游产物。未改动的正文内容可以复用，但重新确认依赖必须产生可审核的新提交。

进入正文后准备产物在本稿本锁定。要调整已通过的设定或粗纲，创建新的空稿本并重新准备；MVP 不提供跨已写章节自动重算和追溯改写。

### 3.4 章节计划与稿本复制

粗纲除可读 Markdown 外，必须提交结构化 `chapter_plan`：稳定 `plan_item_id`、顺序、标题、目标摘要。计划至少一章，顺序唯一；正文开始后本稿本计划冻结。新增番外不修改正文计划。

稿本创建有两个明确操作：新空稿本；复制已通过内容。后者要求源稿本准备产物已全部通过，且无活跃 Run、无待审提案；复制小说标题、目标、准备产物和已通过章节，重建属于新稿本的文档、依赖和通过依据，记录 `source_version_id` 与源修订映射。复制界面明确列出不包含的未通过草稿。

复制不继承未投递意见、Run、工具调用、预算消耗或 Token 使用。复制得到的已通过内容沿用可追溯的通过依据，不触发一次虚构审核，也不计创作增量。存在未通过章节时停止在首个未完成计划项，不能越过空缺继续后面的章节。

新稿本沿用项目当前设置及覆盖提示词；请求快照保存执行时的实际值，稿本不会私藏一份无法解释的旧设置。

## 4. Agent 协议与工具

### 4.1 一轮 AgentLoop

第一个模型适配器采用 Chat Completions 兼容 HTTP 协议，使用配置的 API 根地址拼接 `chat/completions`；Embedding 适配器使用 `embeddings`。根地址明确以 `/v1` 等实际 API 前缀结尾，不通过失败请求猜测 URL。它可连接符合已验证合同的本地或远程服务；vLLM 官方也声明提供这些兼容端点，但本项目不包含部署 vLLM 的工作。[兼容端点说明](https://docs.vllm.ai/en/latest/serving/online_serving/openai_compatible_server/)

协议层统一为 `ModelRequest → TextDelta / ToolCallDelta / Usage / Completed / Failed`；按 tool index 累积 ID、名称和参数，直到响应完整才交给 ToolHost。工具名称、arguments JSON、assistant/tool 消息配对和结束原因均显式校验。阶段提交采用完整 assistant 文本中的 JSON；服务端校验始终必须存在，不以供应商是否启用约束解码为转移。

```text
读取可继续的工作流
→ 等待模型调度许可
→ 在命令边界读取最新设置并封存请求
→ 流式调用模型
→ 持久化完整模型消息与用量
→ 分支：工具调用 / 结构化阶段提交 / 协议错误
→ 工具调用经门控和事务执行，或挂起等待用户
→ 在下一请求封存时物化工具结果与已排队意见
→ 下一轮，直到阶段提交通过业务守卫
```

阶段角色只包含完成本阶段所需目标、工具与提交结构。审核和研究子 Agent 使用独立对话，不能把其内部消息插入写作 Agent 的工具调用序列。

模型请求明确关闭并行工具调用；适配器仍须检查实际输出。若一次响应给出多个工具调用，本次批次全部不执行，给每个调用 ID 返回 `PARALLEL_TOOL_CALLS_NOT_ALLOWED`，保持供应商消息配对，再要求模型串行调用。

### 4.2 工具白名单

| 执行角色 | 工具 | quota |
| --- | --- | --- |
| 准备 Agent | `ask_user`、`query_knowledge`、`research_web`（有实现时） | ask 为 0，另外两种为 1/次 |
| 细纲 Agent | 同准备 Agent | 同上 |
| 正文 Agent | `ask_user`、`append_content`、`replace_content` | 均为 0 |
| 审核 Agent | 无业务写入工具；只读取服务端提供的审核上下文 | 不消耗辅助 quota |

`research_web` 注册表为空时不加入工具声明。`query_knowledge` 在 Embedding 未配置或索引不可查询时不宣告可用。工具不可用原因进入本轮能力说明；旧请求返回一个现已不可用的调用时返回明确错误，不能改用另一种工具。

“免费”只表示不扣辅助工具次数，模型理解工具结果仍可能计 Token。用户直接在知识库页面搜索也不扣写作 quota，Embedding 实际用量独立记录。

### 4.3 结构化阶段提交

提交采用模型响应中的类型化 JSON，由适配器解析为 `PhaseSubmission`，不是额外授予的正文工具。生产适配器必须能够区分工具响应和终态结构化输出，M1 用真实模型验证。

```text
PreparationSubmission { artifacts: [...], chapter_plan: [...] }
OutlineSubmission     { chapter_id, content, goals: [...] }
ChapterSubmission     { chapter_id, summary }
ReviewSubmission      { passed, confidence, issues }
```

`ChapterSubmission` 的摘要不是正文，正文只来自已接受的编辑工具。阶段提交不能覆盖一个非空待审提案，也不能替代终稿审核。

格式无效时保留原始完整响应和校验错误，向模型返回明确协议纠正信息，在本轮剩余请求额度及连续协议错误限制内继续；不从散文中猜测 JSON 或审核结论。未完成流的片段不能作为有效提交。

### 4.4 询问用户

`ask_user` 参数包含问题列表；每题有稳定 ID、标题、`single|multiple|text` 类型、选项 ID/标签、是否必答。单选及多选始终支持自定义文本，不把“其他”作为唯一逃生选项。

回答结构分开保存 `selected_option_ids` 与 `custom_text`。单选最多选一个预设选项，自定义作为另一种答案；多选可以同时包含多个选项及自定义补充。必答题既无选择又无非空文本时不能提交。

一次调用只创建一个持久 Question 记录和一个待完成工具调用，界面可以展示其中多题。用户提交后一次性生成结果。刷新可还原问题，重复提交同一命令不会回答两次。

问题答案、正式审核意见与自由意见使用不同命令类型；暂停期间提交自由意见不会自动替用户作答。

### 4.5 正文编辑

`append_content({ content })` 与 `replace_content({ old_str, new_str })` 不接受项目 ID 或章节 ID，执行对象由 ToolHost 的当前任务上下文提供。

正文存储为 UTF-8 纯文本，入库输入统一换行至 LF；之后精确匹配，不再做 Unicode 归一化、trim、大小写转换或空白折叠。正文展示按段落处理，准备产物与细纲可使用安全 Markdown。

`append_content` 的 `content` 必须非空，按原样追加，不擅自补换行。`replace_content` 的 `old_str` 必须非空，在当前正文中按精确子串匹配，**包括重叠位置在内必须恰好匹配一次**。例如 `aaa` 中的 `aa` 有两处匹配，必须拒绝。`new_str` 可为空，表示删除该片段。

查找时遍历有效 UTF-8 字符边界，不能直接把非重叠 `match_indices` 的数量当作全部匹配数。emoji、组合字符、中文、换行均有专门用例。

编辑流程：完整参数到达 → 校验所属上下文和参数 → 记录调用与 `base_revision_id` → 计算候选结果和差异 → 进入暂停/审核门控 → 接受时重验基础修订 → 单事务提交新修订、计数、工具结果与事件。

`old_str == new_str` 返回 `NO_CHANGE`，不创建新修订、不记新增字数；它不是需要用户批准的正文变更。无匹配、多匹配、修订过期、已删除章节和错误阶段均不改变正文。

工具输出采用固定结构：

```json
{
  "tool_call_id": "tc_...",
  "status": "applied",
  "base_revision_id": "rev_12",
  "result_revision_id": "rev_13",
  "data": { "word_count": 2146 },
  "feedback": [
    { "id": "fb_7", "text": "这一段请减少内心独白。", "citations": [] }
  ]
}
```

正文提案拒绝时 `status = rejected`，返回正式审核意见；工具业务校验失败时 `status = error`，返回稳定错误码。自由意见以独立 `feedback` 数组附加，不混入错误字符串或伪造成执行结果。

## 5. 暂停、意见与审核模式的精确时序

### 5.1 线性化点与结果封存

每个工具调用区分：`received → held/awaiting_review/awaiting_answer → resolved → included_in_request`。`resolved` 说明结果已持久化但尚未用于下一模型请求；`included_in_request` 的结果在对应请求快照中已封存。

单个工具的基础执行结果不可变，反馈绑定关系另存。构建下一请求时，把基础结果与按序绑定的反馈物化为一次完整工具消息，并保存进请求快照；不修改此前已发送请求。

同一 Run 的全部 queued/bound 意见组成唯一的未封存反馈批次，不新增批次表。批次先等待或绑定一个工具结果，随后持续接收新意见；只有携带该工具结果的请求封存时才关闭批次。工具结果 resolved 不代表批次已封存。未绑定工具结果时，协议纠正等其他请求的封存也不会清空这个批次。

反馈状态为 `queued → bound → included`。`included` 表示已放入某次封存请求，不承诺模型理解或网络另一端已经接收。UI 先显示“等待投递/已加入请求”，请求进入 dispatched 后才显示“已随请求发送”；请求失败时补充“请求失败，尚无模型回应”。prepared 请求尚未发出即被停止时，保留该请求快照并标记 cancelled，继续时把同一工具消息与意见纳入新的请求快照，不创建第二条逻辑意见。

### 5.2 AutoApprove 行为矩阵

| 用户事件及当前情况 | 服务端行为 | 正文与反馈结果 |
| --- | --- | --- |
| 正常写作时点暂停，模型请求仍在途 | 设置 `pause_requested=true`，继续接收当前响应 | 下一个完整新工具调用在执行前挂起 |
| 新工具调用已被接受并开始处理后点暂停 | 按命令序列处理，不撤销已提交编辑 | 暂停作用于下一个新工具边界 |
| 已存在未发给模型的 resolved 结果时点暂停 | 保留结果；允许完成当前轮结果交付 | 仍在下一个新工具调用前挂起 |
| 暂停请求尚未挂起时提交意见 | 先入队并清除暂停请求；存在未封存的最近工具结果则绑定，否则等待下一新结果 | 不回写历史请求；无需再次点继续 |
| 工具已挂起时提交意见 | 先入队、取消暂停、恢复挂起调用的正常门控 | 调用完成后把全部排队意见附到结果 |
| 正常运行且没有未封存批次时提交首条意见 | 打开批次，等待提交后完成的下一次新工具结果 | 不绑定一个此前已经完成且没有待发送意见的旧结果 |
| 已有未封存批次时继续提交意见 | 新意见加入同一批次，沿用批次已选中的工具结果；结果已 resolved 也可追加 | 携带该结果的请求封存时一次排空整个批次，封存后的新意见进入下一批次 |
| 已暂停时点继续，未填意见 | 清除暂停并恢复调用 | 原有队列仍按顺序投递 |
| `ask_user` 已展示时提交自由意见 | 只入队；回答仍必需 | 回答提交产生工具结果后可附带意见 |

“暂停前最近一个结果”采用方案 D09 的不可变请求约定。只有 `resolved` 且未被请求快照封存的工具结果可追加；网络请求在途时，不能改变它已经携带的返回值。

反馈绑定按以下优先级处理：

1. 已有批次绑定了未封存的工具结果时，新意见直接加入该批次并绑定同一结果，不再逐条重新计算“下一次工具调用”。
2. 批次尚未绑定，且用户在暂停尚未挂起时提交意见、最近结果仍可追加时，将整个批次绑定到这个最近结果，同时清除暂停请求。该暂停特例可以改变批次原先等待新结果的目标。
3. 没有批次且不符合暂停特例时，首条意见记录 `after_tool_sequence`，其值为当时已经形成完整结果的最大工具序号；等待序号更大且之后完成的结果。已经到达但尚未完成的工具调用（例如等待用户回答）可以成为该目标。
4. 尚未绑定的批次收到更多意见时，后续意见继承首条的 after_tool_sequence 和当前 preferred_tool_call_id。目标结果完成后，整个批次统一设置 bound_tool_call_id；不得以单条意见的提交时刻把后续意见拆到下一批次。

这些批次锚点值保存在 feedback 行中，同一批次保持一致，由每稿本命令序列及短事务统一更新。封存携带目标工具结果的请求时，读取该 Run 已绑定该结果的整个未封存批次，按反馈 sequence 排序写入同一工具返回值，并一次性标记 included；尚未绑定或请求不携带目标结果时，保持批次不变。批次封存命令之后到达的意见才打开下一批次。

例如：A 已绑定工具 T 的结果，T 已 resolved，但请求尚未封存；此时追加 B，A、B 必须一起进入 T 的返回值。只有 A 已随请求封存后才到达的 B，才等待下一批次。停止或中断不拆分已存在的未封存批次。

### 5.3 没有下一个工具调用的情况

若模型直接提交章节而 `pause_requested=true`，在结构化提交边界进入已暂停状态，保存候选提交；继续后重新检查完成守卫。这样不会无限显示“等待暂停”。这是工具调用缺席时的终态控制约定，不允许提前批准本章。

若模型提交章节时仍有自由意见且不存在可追加的工具结果，不允许进入整章审核。保存意见，拒绝本次阶段提交，通过协议纠正消息要求继续该阶段并产生合法工具调用，意见仍按工具结果规则投递。若连续不产生可推进动作，触发工程执行限制并展示未投递意见，不静默丢弃或改写历史结果。

请求失败、用户停止或进程中断时，意见保持原投递状态。继续任务时 `bound` 意见仍绑定原工具结果，`included` 意见从封存上下文重建，不再生成一份重复工具消息。

### 5.4 ApproveRequired

合法的实际正文变更先形成 Proposal，正式正文保持原修订；阅读区显示候选差异和“待确认”标识。一个任务最多有一个提案，避免多次编辑依赖未批准的内容。

正式审核提交规则：意见在去除首尾 Unicode 空白后为空，且没有引用/附件性文本时表示接受；有文本或引用意见时表示拒绝并反馈。UI 主按钮动态显示“接受编辑”或“拒绝并反馈”，也允许用户清空草稿后选择接受。

自由意见入口不复用这个判定：空自由意见返回 `EMPTY_FEEDBACK`，绝不批准编辑。引用对象随意见保存；引用本身就是提供意见的一部分。

接受时重验基础正文修订，成功才写正式正文并返回工具成功。拒绝时提案终态为 rejected，正文不变，工具结果包含用户意见，Agent 再次提出编辑。

正式审查每次编辑不等于本章终稿审核；即使逐次编辑全部接受，Agent 提交本章后仍需整章审核。

### 5.5 随时切换模式

| 切换时机 | 确定行为 |
| --- | --- |
| AutoApprove → ApproveRequired，模型在途 | 下一完整编辑调用采用新模式；已提交编辑不撤销 |
| AutoApprove → ApproveRequired，工具因暂停挂起 | 不解除暂停；恢复后进入逐次审核 |
| ApproveRequired → AutoApprove，存在待审编辑 | 切换操作本身不批准既有提案；该提案仍须接受、拒绝或委托；后续新编辑自动接受 |
| 任何切换，正在等待 `ask_user` | 不替代问题回答 |
| 任何切换，存在委托审核 | 不取消已委托的当前审核，不改变其目标 |
| 切换与新调用同时到达 | 每稿本命令顺序决定先后，响应返回生效的控制修订号 |

保留已有待审提案不限制“随时切换”：模式设置立即变更，正在等待的明确审核承诺仍须完成。界面显示“已切换；当前编辑仍待确认”。

## 6. 审核体系与委托

### 6.1 统一审核对象

审核类型为 `preparation.background/research/characters/outline`、`chapter.outline`、`chapter.edit`、`chapter.final`。每条 Review 保存类型、目标 ID、目标修订、依赖修订、创建原因、状态及决定来源。

状态为 `open（待决） → delegated → approved/rejected`，并允许 `stale`。用户直接决定的记录从 `open` 进入终态。取消或失败的是本次委托 attempt：子 Run 标为 `cancelled` 或 `failed`，Review 回到 `open`，而不是另造一个已取消的 Review 终态。`stale` 不算通过，且不用于触发修订模型的空意见。

准备审核采用 §3.3 的批次规则；细纲与整章拒绝立即回到对应生成状态；正文提案拒绝只拒绝本次编辑。

### 6.2 委托协议

点击委托时保存当前草稿意见、引用及目标快照，创建子 Run。默认使用项目最新模型和该类型审核提示词，不新增一套用户必须配置的审核模型设置。审核子 Run 不获得写作工具，只能返回一次结构化审核结论。

结构要求：`passed` 必须是 bool；`confidence` 必须是有限数且在 `[0,1]` 内；`issues` 必须是字符串数组；三个字段都必须存在。字符串不得作为布尔值，缺字段不可自动填充。

`passed=true` 直接通过，置信度不参与阈值判断；非空 issues 作为观察项展示。`passed=false` 拒绝；如果 issues 为空，仍遵守原始协议，将“审核未通过，审核 Agent 未提供具体问题”作为系统说明交给写作 Agent，而不是伪造具体问题。

委托产出的有效决定自动应用于本次审核，无需再加一次“确认 Agent 结论”。失败则保留未完成审核，用户可重新委托或自行审核；不会自动选择替代审核方。

### 6.3 并发与过期结果

委托期间 UI 提供“取消委托并自行审核”，不直接并发提交两个决定。取消会使 `attempt_id` 失效并重新开放 Review；迟到结果不得应用。

结果落库前校验 Review 尚处于 `delegated`、attempt 有效、目标修订与依赖仍匹配。审核结果、提案应用/拒绝、工作流转换和事件必须一次提交；当该决定使阶段全部必需产物通过时，同事务保存产物纯文本折叠检查点。同一审核子 Run 与模型请求 ID 的成功回调重放返回已提交快照，不重复生成折叠块。

## 7. 预算、知识库与零引擎能力

### 7.1 预算账本

项目设置中的 `auxiliary_tool_budget` 是非负整数，默认建议 10。准备阶段与每章细纲各自有作用域，作用域只记录已用数，有效上限在每次工具执行前读取项目最新设置。

修改预算不会清空已用数。下调到已用数以下时剩余为 0，不追回已执行调用；上调后立即增加后续可用次数。阶段修订、任务暂停/继续、显式重试均不创建新预算作用域。

收费工具通过校验并取得执行资格时，以 `(scope_id, tool_call_id)` 唯一键登记一次消耗。执行前验证失败、没有引擎和没有预算不扣费；一旦进入真实检索/外部调用，无结果或服务失败仍计 1 次，重复处理同一个调用不再次扣除。

quota 为 0 时工具返回 `QUOTA_EXHAUSTED`，Agent 可以继续撰写或询问用户，不能靠改写工具名绕过。请求构建时说明余额，工具执行前仍重新检查。

联网研究未来接入时，父调用占用 1 次项目辅助 quota；引擎内部网络与模型调用另受引擎执行上限约束并记录 Token，不递归无限借用免费子任务。MVP 只有接口合同与契约测试，没有这类实际执行。

### 7.2 知识数据与索引

知识条目有标题、关键词数组、正文、`space_id`、内容修订号和软删除时间。Space 允许用户创建、重命名；非空 Space 删除被拒绝，UI 要求先移动或删除条目。避免隐式批量删除知识。

知识录入和修改先在 SQLite 提交内容并创建 `index_tasks`。条目索引状态为 `pending/running/ready/failed`，附带内容修订和 Embedding profile 指纹。未配置 Embedding 时保存知识照常成功，状态显示 `not_configured`，不偷偷发请求。

初版切分规则采用段落优先、目标 600 个非空白字符、硬上限 1000 字符、约 80 字符重叠；超长段按 Unicode 字符边界拆分。这是可调的质量参数，先作为集中常量，M4 使用中文资料验证，不先开放复杂调参界面。

Embedding 的模型、地址、输出维度和 profile 指纹必须固定于一次索引任务。Rust 查询明确先计算查询向量，再交给 LanceDB；不能假定文本查询自动产生向量。[LanceDB Embedding 文档](https://docs.lancedb.com/embedding)

LanceDB 表按 Embedding profile 隔离，字段至少包含：`chunk_id`、`entry_id`、`entry_revision`、`space_id`、`chunk_index`、`text`、`embedding_profile_id`、向量。小规模先用精确向量扫描，不预建需要训练的大型 ANN 索引；性能实测需要时再在同一表上添加索引。[LanceDB Rust 能力](https://docs.rs/lancedb/latest/lancedb/)

### 7.3 一致性与检索

索引任务以 `entry_id + revision + profile` 为唯一键，操作为按确定 chunk ID 幂等 upsert，再确认任务完成。旧任务完成时若条目已经更新或删除，不能把旧修订标为 ready。

查询先校验当前 Embedding 可用，计算一次查询向量，再向量搜索；候选回查 SQLite 的存活状态、所属 Space、当前修订、Embedding profile 与索引完成状态，剔除无效 chunk。索引写入中途产生的部分结果，在相应任务 ready 前不可见。`top_k` 指过滤、去重后的最终有效 chunk 数，不是第一次从 LanceDB 取出的候选数。

MVP 采用同一次查询内递增候选上限的补取流程：

1. 为本次搜索打开独立的 LanceDB Table 实例并固定读取版本，所有轮次使用同一查询向量、profile、Space 条件和距离度量。版本固定只作用于本次只读实例，索引写入继续使用自己的实例；活动查询期间不清理其使用的表版本。[LanceDB Table 版本读取](https://docs.rs/lancedb/latest/lancedb/table/struct.Table.html#method.checkout)
2. 首轮候选上限 N 为 top_k。回查 SQLite、按 chunk_id 去重后，按相关度选取最多 top_k 条有效结果。
3. 有效结果不足且本轮取得 N 条候选时，将 N 翻倍，从同一表版本重新取前 N 条，再对完整候选集合回查、去重；不能因为某一轮筛选结果为空就提前返回空。
4. 直到取得 top_k 条有效结果，或返回的候选数量小于 N、确认该查询范围内的候选已经耗尽时结束。耗尽时允许返回少于 top_k 条；以 chunk_id 作为等分结果的稳定次序。同一候选在多轮中出现只计一次。
5. 最终返回前在一个短 SQLite 读事务中重新校验已取候选集合并选出结果；若期间更新/删除使数量不足且候选未耗尽，结束该事务后扩大候选集合，再用新的短事务校验，不持有读事务执行补取。直到满足数量或该固定版本候选耗尽，返回的有效性以这次最终校验快照为准；固定版本之后新写入的向量由下次搜索覆盖。

候选补取属于一次 query_knowledge 执行，复用原 tool_call_id 和查询向量，只扣一次辅助 quota、只产生一次查询 Embedding 用量。搜索达到超时或资源限制时返回明确错误，不能把未完成搜索当作“没有相关知识”。上述补取复用向量查询，不切换检索引擎或检索方式。

`query_knowledge` 输入为 query、可选 Space 列表、`top_k`（默认 5，最大 10）；服务端使用类型化过滤参数，不能把模型输入拼成数据库过滤表达式。结果包含标题、原文块、条目/修订/块 ID、来源和相关分值；分值不称为事实置信度。

有未就绪条目时，搜索结果附带可读的索引覆盖信息。更换 Embedding 模型或维度后建立新的 profile 索引，在新 profile 就绪前明确显示不可进行完整语义检索；不能混合新旧向量，也不能自动切到关键词搜索冒充语义搜索。

知识列表的标题/关键词筛选是独立的浏览功能，使用 SQLite 查询，不作为向量检索失败时的隐式替代。

### 7.4 研究与积累接口

联网研究接口合同：输入研究问题、用户线索、语言及运行上下文；输出发现摘要、带 URL/标题/访问时间的来源、限制与实际用量。实现内部可以是具备网络能力的子 Agent，但不能直接写小说正文。

知识积累接口合同：输入用户线索和目标 Space；输出候选知识条目及来源，或类型化执行失败。未来引擎通过应用服务验证并插入条目、触发索引，不直接绕过 SQLite 主数据写 LanceDB。

两种接口各有编译期注册表和 capability descriptor，MVP 的生产注册项数量严格为 0。能力查询返回空数组和原因；调用未注册引擎返回 `ENGINE_NOT_AVAILABLE`，不创建运行任务、不扣 quota。测试专用实现仅用于契约测试，不打包到生产注册表。

## 8. 数据模型与持久化

### 8.1 类型与公共约定

ID 使用 UUID，时间以 UTC 毫秒保存、前端按本地时区展示；稳定排序使用数据库序列或显式顺序字段，不依赖 UUID 排序。跨 Rust/TypeScript 的数据库长整数在 JSON 中编码为十进制字符串，避免超过 JS 安全整数。

业务枚举序列化为稳定字符串；未知枚举报错，不兼容旧格式。正文、资料和模型消息在本地数据库保存，凭据只保存引用。状态 JSON 必须反序列化为 Rust 类型，不能把任意 JSON 当工作流执行指令。

### 8.2 关系表清单

以下按行为分组；字段为最低需要，不是要求每种内容再拆一张表。

| 表 | 关键字段与约束 |
| --- | --- |
| `projects` | id、name、selected_version_id、settings_json、settings_revision、revision、created_at、updated_at、deleted_at |
| `novel_versions` | id、project_id、label、novel_title、source_version_id、body_completed_at、revision、created_at、updated_at、deleted_at |
| `chapters` | id、version_id、kind、plan_item_id、order_index、title、status、approved_revision_id、deleted_at；有效顺序唯一 |
| `documents` | id、version_id、chapter_id?、role、current_revision_id；每稿本/章节固定角色唯一 |
| `document_revisions` | id、document_id、revision_no、content、structured_payload?、word_count、created_by、source_revision_id?；document + revision_no 唯一 |
| `revision_dependencies` | revision_id、dependency_revision_id、purpose；保存准备及章节依赖 |
| `workflow_instances` | version_id 唯一、state_json、state_revision、active_run_id、approval_mode、pause_requested |
| `runs` | id、project_id?、version_id?、kind、parent_run_id?、stage、status、resume_count、request_count、request_allowance_end、consecutive_protocol_errors、checkpoint_json、quota_scope_id?、started_at、ended_at |
| `model_requests` | id、run_id、attempt_no、status、settings_snapshot、prompt_snapshot、context_snapshot、provider_request_id?、usage_json、error_json |
| `agent_messages` | id、run_id、sequence、role、content_json、request_id?、tool_call_id?；run + sequence 唯一 |
| `tool_calls` | id、run_id、request_id、provider_call_id、sequence、name、args_json、base_revision_id?、proposal_json?、status、result_json?；request + provider_call_id 唯一 |
| `questions` | id、tool_call_id 唯一、schema_json、answer_json?、status |
| `feedback` | id、run_id、version_id、sequence、text、citations_json、after_tool_sequence、preferred_tool_call_id?、bound_tool_call_id?、included_request_id?、status |
| `reviews` | id、version_id、batch_id?、type、target_revision_id?、target_tool_call_id?、dependencies_json、status、attempt_id?、decision_source、source_review_id?、passed?、confidence?、issues_json、user_input_json |
| `quota_scopes` | id、version_id、stage_key、used_count；version + stage_key 唯一 |
| `quota_charges` | scope_id、tool_call_id、charged_at；二者联合唯一 |
| `command_receipts` | command_id 唯一、method、target、payload_hash、response_status、response_json |
| `events` | seq 整数主键、version_id?、type、entity_id、revision、payload_json、occurred_at |
| `text_changes` | id、revision_id 唯一、project_id、version_id、chapter_id、delta、reason、occurred_at |
| `system_settings` | singleton_id、settings_json、revision |
| `prompt_overrides` | scope_type、scope_id、prompt_key、role、content、revision；作用域 + key + role 唯一 |
| `prompt_templates` | id、title、content、variables_json、revision、deleted_at |
| `knowledge_spaces` | id、title、revision、deleted_at |
| `knowledge_entries` | id、space_id、title、keywords_json、content、revision、source_json、deleted_at |
| `index_tasks` | id、entry_id、entry_revision、embedding_profile_id、operation、status、error、checkpoint_json；目标组合唯一 |

小说目标使用稿本级 `novel_brief`，开始准备前必须有非空目标；用户指导作为当前章文档角色 `chapter_guidance` 保存修订。复制的审核通过依据标记 `decision_source = copied` 并引用 source_review_id，不能伪造一次新的人工或 Agent 审核。

阶段折叠数据保存在 `runs.checkpoint_json.context_fold`，字段和事务规则见 §9.4。`agent_messages` 保存原始消息，`model_requests.context_snapshot` 保存每次实际请求使用的折叠产物及当前阶段消息；不为折叠新建摘要文档或模型任务。复制稿本不复制原 Run，由复制事务为已通过产物建立新稿本的纯文本上下文来源清单，记录在 `workflow_instances.state_json` 的类型化上下文数据中。

feedback 的 after_tool_sequence、preferred_tool_call_id、bound_tool_call_id 表示所属未封存批次的共同锚点，后续追加意见继承批次值，不能按每条意见的到达时刻分别选目标。模型请求累计数与执行额度上界按 §2.3 在 Run 中持久化，不能用清空请求记录或重建 Run 的方式实现继续。

`model_requests.status` 为 prepared/dispatched/succeeded/failed/cancelled/unknown；prepared 快照不可原地改写，未发送的作废快照仍可留作诊断。更换设置、继续任务时建立新 attempt，关联相同已完成工具消息；`feedback.included_request_id` 记录首次纳入的请求，后续是否再次随上下文发送由各请求快照说明。

所有可修改资源都有 created_at/updated_at 与 revision，表中只突出特别依赖这些字段的对象。`selected_version_id` 是项目持久的展示选择，切换它不启动生成；服务端验证所属项目。它与 `active_run_id` 不同，用户可以查看另一稿本而保持原任务运行。

所有权完整性必须由外键及事务校验共同保证，例如审核的 revision 必须属于该 version，工具目标必须属于其 run。不能只靠 UUID 不容易猜到就忽略跨项目校验。

### 8.3 索引与事务

至少建立：有效项目更新时间索引、稿本所属项目索引、章节顺序索引、文档修订倒序索引、Run 状态索引、事件稿本/序号索引、未投递意见索引、开放审核索引、知识 Space/删除状态索引和待执行索引任务索引。

建立 SQLite partial unique index，分别约束项目及稿本的一个活跃顶层写作 Run，索引条件限定 preparation/outline/writing 类型且 parent_run_id 为空；必要的 `project_id` 冗余到 Run 并在创建时校验。活跃状态集合包括 queued、running、waiting、paused、interrupted；stopped/failed/succeeded 释放槽。恢复 stopped/failed 的原 Run 时须在事务中重新获得槽，开放提案仍阻止本稿本不相干的新阶段入口；委托子 Run 不与父任务争用这个业务槽。

稿本级 documents 的 chapter_id 为空，SQLite 普通联合 UNIQUE 对 NULL 的处理不能保证角色唯一；分别对 `chapter_id IS NULL` 与 `chapter_id IS NOT NULL` 建 partial unique index。章节 order_index 使用逻辑编号，排序唯一约束限定未删除记录并包含 chapter.kind。

编辑接受事务：读取并校验命令去重 → 校验当前阶段与修订 → 插入新 Revision → 更新 Document → 记录 TextChange → 完成 ToolCall/Review → 更新工作流修订 → 写 Event → 写命令回执 → commit。

数据库连接启用 `foreign_keys=ON`、WAL、`synchronous=FULL` 和有限 `busy_timeout`。用 SeaORM 事务完成原子写入；不要假定 ORM 普通 `save()` 自动保护多个表。[SeaORM 事务](https://www.sea-ql.org/SeaORM/docs/advanced-query/transaction/)

新数据库的表、列、普通唯一键和外键以 SeaORM Entity 注册表为唯一来源，使用 `SchemaBuilder::apply` 初始化。SQLite partial index 由 SeaQuery 的类型化 `Index` 构造器补充，以保留条件唯一性；初始化代码不维护手写 `CREATE TABLE` 或 `CREATE INDEX` 字符串。

首次启动只在新数据库创建当前 schema 和内建必要行；保存 `schema_version`。版本不匹配则报错并停止打开，不能自动删除、迁移、补字段或读取旧格式。此规则来自项目 `AGENTS.md`，不是小说内容修订的限制。

### 8.4 删除规则

| 对象 | 行为 |
| --- | --- |
| 项目、稿本、用户模板、知识条目、空 Space | 软删除，回收站可恢复 |
| 已有内容的正文计划章 | MVP 不提供独立删除，以保持冻结章节计划和依赖一致 |
| 番外章 | 无活跃 Run/审核时允许软删除；所属内容随祖先不可见 |
| 项目/系统提示词覆盖 | 明确“清除覆盖”，物理删除该覆盖行，使优先级解析重新生效 |
| OS 凭据 | 用户明确移除配置时删除对应凭据 |
| 临时流片段、到期事件、已失效向量块 | 按各自保留规则物理清理，不承担唯一业务记录 |

删除活跃项目/稿本返回 `RESOURCE_BUSY`，界面要求先停止任务，不暗中丢弃正在写作的数据。父对象软删除不批量改写所有孩子的 deleted_at；查询检查祖先，因此恢复父对象不会误恢复此前单独删除的孩子。

MVP 不提供永久清空正文回收站按钮。索引中软删除条目立即被回查过滤，后续清理任务负责物理移除向量。

## 9. 提示词、配置与上下文

### 9.1 内建目录

每个键提供 `system.md`、`user.md` 和代码中的变量定义：

```text
preparation.generate
chapter.outline
chapter.write
review.preparation.background
review.preparation.research
review.preparation.characters
review.preparation.outline
review.chapter.outline
review.chapter.edit
review.chapter.final
```

番外复用章节提示词并使用 `chapter_kind` 变量。研究/积累没有实现时不提供假定已可执行的内建引擎提示词；以后随具体引擎一起加入目录。系统和项目展示的内建条目集合始终来自同一个代码目录。

编译期 `include_str!` 嵌入 Markdown。系统修改只保存覆盖值，项目修改保存更高优先级覆盖值；重置系统表示删除系统覆盖，重置项目表示删除项目覆盖。覆盖为空字符串视为实际空内容，不当作“没有覆盖”。

### 9.2 模板语言与变量

MVP 只支持 `{{variable_name}}` 文本替换，不支持表达式、循环、文件包含或执行代码。变量按扫描位置一次替换，变量值中的 `{{...}}` 不递归展开。需要字面量占位符时使用 `\{{...}}`，渲染后去除转义符。

内建变量由代码声明 name、中文说明、类型、required、示例和来源。保存时拒绝未知变量、缺失必需占位符和不完整括号；预览时拒绝缺少必需的运行变量。可选变量缺省行为逐项声明，不通过统一空字符串掩盖必需信息缺失。

用户模板独立管理标题、正文和自定义变量说明，可插入章节指导或意见草稿；它不会自动获得 system 消息权限。系统提示词和项目覆盖均提供保存前预览，预览使用示例值，不包含真实密钥。

### 9.3 设置作用域

系统设置：模型服务地址与凭据引用、默认模型、模型能力资料、Embedding profile、内建提示词覆盖。项目设置：模型引用（模型 ID 与它自己的能力资料）、思考等级、辅助工具预算、项目提示词覆盖。没有额外审核模型开关，委托使用项目当前模型。

每次请求先解析 `project → system default` 的有效模型设置，再解析提示词的三级优先级，最后用**有效模型自己的能力资料**验证模型支持工具、流式输出、结构化提交和当前思考参数。能力资料与被引用的模型 ID 一起声明，因此校验对象与请求实际发往的模型始终是同一个；同一套判定既用于调度器发请求前，也用于 HTTP 命令受理前。MVP 的协议适配器只承诺验收过的能力组合；不能宣称任意兼容地址都可用。

请求保存：设置修订、提示词实际渲染内容及哈希、模型 ID、有效能力资料、有效思考参数、工具描述、上下文来源修订、最终消息数组、供应商请求 ID（若有）。凭据只在请求发送时读取，不进入快照或日志。

模型服务连接测试是用户显式触发的实际请求，显示成功/失败与用量；静态格式校验不能显示“连接成功”。测试请求显式携带被测模型 ID 与能力资料，并始终使用系统设置的服务地址与凭据：声明支持思考时会真的带上 `reasoning_effort`，服务返回 400 时给出“该模型可能不支持思考参数”的明确错误。因此“支持思考”是一个可被验证的声明，而不是用户的假设。Embedding 连接测试同时确认输出维度与有限浮点值。

### 9.4 上下文与缓存

#### 9.4.1 折叠时机

当前阶段全部必需产物的当前修订及依赖审核通过后，必须将该阶段历史消息在 Agent 上下文中整体折叠为仅含产物内容的纯文本。人工审核与委托审核遵循同一规则。

| 阶段 | 触发条件 | 折叠结果 |
| --- | --- | --- |
| 准备阶段 | 背景、资料、人物、粗纲的有效修订全部通过 | 按背景、资料、人物、粗纲顺序组成完整产物纯文本 |
| 每章细纲阶段 | 本章细纲有效修订通过 | 审核通过的细纲全文 |
| 每章正文阶段 | 整章终稿有效修订通过 | 审核通过的本章正文全文 |
| 番外细纲/正文阶段 | 对应细纲/终稿有效修订通过 | 对应产物全文 |

准备阶段只有部分产物通过、审核拒绝或失效、AutoApprove 自动接受一次编辑、ApproveRequired 批准一次编辑，均不代表该阶段结束，不折叠其历史。审核 Agent 的结果只触发目标产物的审核决定，审核结果 JSON 本身不是需要保留的创作产物。

#### 9.4.2 纯文本合同

折叠由确定性的 `render_artifact_plaintext` 完成，不调用模型。每个产物读取审核通过的不可变修订，完整保留内容、段落与原有格式；多产物之间只用两个 LF 分隔，顺序固定。正文直接使用通过修订的全文。

结构化业务产物也必须完整呈现为可审核的文本，例如粗纲的章节计划以章节顺序、标题、目标正文表示；这些业务内容在提交审核时就使用同一个渲染器展示，确保折叠文本与用户实际审核内容一致。内部 ID 和依赖关系留在类型化数据中。产物原有的 Markdown 字符仍是文本，不额外包一层 JSON、代码围栏或系统说明。

折叠块的模型可见 content 仅包含上述产物文本，不包含阶段运行说明、消息角色标签、历史问答、tool call/结果信封、失败修订、用户意见、审核理由、通过标志、置信度、Token 或时间记录。资料产物中已经审核通过的来源链接属于产物内容，原样保留。

模型 API 必需的 messages/role 字段是传输信封：折叠块作为普通 assistant 文本消息携带，不带 tool_calls，也不提升为 system 指令。源 Run、消息边界、修订 ID 与哈希仅作为本地上下文来源元数据，不拼进 content。

#### 9.4.3 事务、幂等与恢复

最终通过事务先校验所有必需产物和依赖有效、没有在途请求/未完成工具/待审提案/未回答问题/未投递意见，再锁定产物修订并生成文本。不满足条件时保留待处理状态，不能用折叠清掉未完成事项。

在 `runs.checkpoint_json.context_fold` 保存 `through_message_sequence`、按顺序排列的 `artifact_revision_ids`、`plain_text` 与 `content_hash`。当前 Run 的审核通过、fold、succeeded、工作流下一阶段及事件一次提交；折叠不计辅助 quota 或模型 Token。

ContextBuilder 以稿本工作流中的已完成阶段来源为准，将该 Run 的历史消息完整替换为一个纯文本块。一个阶段在一次上下文中只有一份产物表示，不同时注入 fold 和其原始消息/同一产物副本；审核子 Run 的对话也不进入写作上下文。新阶段只累积自己的新消息。

原始 `agent_messages`、工具/意见/审核记录及已封存请求快照继续保存在本地，折叠不物理删除记录，也不改写已发出的请求。不能通过全量加载数据库历史或复用包含旧历史的服务端会话重新把已折叠过程消息带回上下文。

相同审核命令/回调重复到达时复用已提交 fold。事务前中断则仍是原阶段待处理状态，事务后中断则直接从 fold 恢复；不会出现已进入下一阶段却仍携带旧阶段过程消息的中间状态。下一请求构建前必须验证来源修订与 fold 一致。

#### 9.4.4 请求组装与长度限制

上下文固定顺序：最新有效的角色及任务约束 → 稳定工具定义 → 已完成阶段的产物纯文本块（按阶段/章节顺序）→ 当前阶段输入、当前正文和未折叠的新消息。已通过的当前章细纲使用其 fold，避免再作为独立细纲副本重复注入。项目/系统设置仍按每次请求读取最新值，不作为旧阶段历史折叠掉。

当前阶段未通过时保留完整工具调用/结果配对、问题回答和意见，便于修订。通过后只保留最终产物；需要持续影响后文的设定和结论应体现在被审核的产物中，不能在折叠时另写一份过程摘要附加给模型。

只在工具可用性改变、设置/提示词/产物真实变化或阶段完成折叠时更新相应块，不注入每轮时间戳、随机 ID 或无意义重排。折叠后文本保持稳定；供应商缓存用量据实际报告统计，历史请求用量不因折叠被抹去。

长度控制使用模型资料中的上限并保留输出空间。适配器有可靠 tokenizer 时使用其计数，否则标注估算并保守预留。折叠只减少过程消息，不保证产物全文始终放得下；仍超出窗口时返回 `CONTEXT_LIMIT_EXCEEDED`，提示选择更大上下文模型或调整创作范围。不能自动将产物改为章节记忆摘要、截断原文或重新引入旧摘要流程。

## 10. HTTP、SSE 与前端合同

### 10.1 路由分组

所有 API 以 `/api` 开头；以下列出核心资源，CRUD 使用一致的 GET/POST/PATCH/DELETE 约定。

| 路由 | 目的 |
| --- | --- |
| `GET /api/health`、`GET /api/capabilities` | 本地状态与可用模型/研究/积累能力 |
| `/api/projects`、`/api/projects/{id}` | 项目 CRUD |
| `/api/projects/{id}/settings` | 项目设置读取/修改；模型引用包含模型 ID 与能力资料 |
| `/api/projects/{id}/versions` | 稿本列表与创建；创建体指定 empty/copy 及来源 |
| `/api/versions/{id}`、`/api/versions/{id}/chapters` | 稿本、目录与小说标题 |
| `GET /api/versions/{id}/snapshot` | 一致的工作台快照与事件游标 |
| `POST /api/versions/{id}/commands` | start、pause、resume、stop、set_mode、start_chapter、complete、add_extra |
| `GET /api/chapters/{id}/content` | 某章当前正文、待审提案、按需修订内容 |
| `PUT /api/chapters/{id}/guidance` | 保存章节指导；只允许 chapter_ready |
| `POST /api/runs/{id}/feedback` | 自由意见与引用 |
| `POST /api/questions/{id}/answer` | 正式问题答案 |
| `POST /api/reviews/{id}/decision` | 人工审核决定及意见 |
| `POST /api/reviews/{id}/delegate`、`.../cancel-delegation` | 委托及取消委托 |
| `GET /api/documents/{id}/revisions/{revision}` | 引用与历史修订读取 |
| `GET /api/events?after={seq}` | 全应用 SSE，前端按实体分发 |
| `/api/prompt-templates` | 用户模板 CRUD |
| `/api/prompts/system`、`/api/projects/{id}/prompts` | 内建提示词目录、来源与覆盖 |
| `POST /api/prompts/preview` | 类型化模板校验与预览 |
| `/api/knowledge/spaces`、`/api/knowledge/entries` | 知识 CRUD |
| `POST /api/knowledge/search`、`.../reindex`、`.../accumulate` | 语义检索、显式重建、积累入口 |
| `GET /api/dashboard` | 指定时区/时间范围/项目的统计 |
| `/api/settings`、`POST /api/settings/test-model`、`.../test-embedding` | 系统设置与连接测试；test-model 请求体携带被测模型 ID 与能力资料 |
| `GET /api/recycle-bin`、`POST /api/recycle-bin/{type}/{id}/restore` | 回收站与恢复 |
| `GET /api/versions/{id}/export?format=md\|txt` | 导出已提交内容 |
| `POST /api/system/shutdown` | 显式退出本地服务 |

### 10.2 命令、幂等与冲突

所有变更请求携带 `command_id`；涉及状态的命令携带 `expected_state_revision`，正文决定还需 `base_revision_id`。设置、模板、知识修改携带各自的 `expected_revision`。

```json
{
  "command_id": "7a6b3eb8-7808-4ad0-b04f-787a13e29309",
  "expected_state_revision": "42",
  "type": "set_mode",
  "payload": { "mode": "approve_required" }
}
```

先查询命令回执：ID、目标与请求 hash 一致时返回原结果，不再次执行；同 ID 不同内容返回 `IDEMPOTENCY_CONFLICT`。只有新命令才校验当前修订，避免已成功操作的重试被错误地当作过期操作。

同步短命令在事务提交后返回 200/201；创建后台执行返回 202 和 Run ID。网络超时后前端使用同一个 command_id 重试，不生成新 ID。持久保存命令回执，MVP 不做可能破坏业务去重的短 TTL 清理。

错误结构：`{ error: { code, message, details, request_id } }`。参数错误用 422，状态/修订冲突用 409，资源不存在或祖先已删除用 404，外部服务错误用相应执行失败状态并在请求接口返回 502/504，不把模型失败包装成成功正文。

至少定义：`INVALID_TRANSITION`、`REVISION_CONFLICT`、`RESOURCE_BUSY`、`EDIT_NO_MATCH`、`EDIT_AMBIGUOUS_MATCH`、`QUOTA_EXHAUSTED`、`ENGINE_NOT_AVAILABLE`、`EMBEDDING_NOT_CONFIGURED`、`INVALID_REVIEW_RESULT`、`MODEL_CAPABILITY_MISMATCH`、`CONTEXT_LIMIT_EXCEEDED`、`SCHEMA_VERSION_UNSUPPORTED`。

### 10.3 SSE 协议

持久事件示例：

```text
id: 1082
event: document.committed
data: {"seq":"1082","version_id":"v1","entity_id":"doc1","revision":"13","payload":{"chapter_id":"c3","revision_id":"rev_13","word_count":2146}}
```

持久事件类型至少包括：`workflow.changed`、`run.changed`、`tool.proposed`、`tool.resolved`、`question.created`、`review.changed`、`document.committed`、`feedback.changed`、`settings.changed`、`knowledge.index_changed`。事件通知变更，正文增量可携带 patch 与基础修订；冲突时重新读取该章。

临时事件使用 `preview.delta`，携带 request/tool/offset，不携带可恢复的 SSE id。半截 JSON 参数只在独立生成预览层展示；不把字符流直接写入正式正文。预览丢失后丢弃该请求的临时缓冲，不影响已提交修订。

快照响应在同一个 SQLite 读事务中取得业务数据与事件水位。客户端随后从该水位订阅事件。服务端以 events 表为准持续读取 `seq > cursor`；内存广播只用作唤醒，广播溢出时继续按游标读数据库，避免“查历史到订阅实时之间”丢事件。

每 15 秒发送 SSE 注释心跳。游标解析优先级固定为：存在非空 `Last-Event-ID` 请求头时只以该值为准；请求头缺失或为空时使用 URL 的 `after`，首次连接传入快照水位。两者同时存在且不同是正常重连情况，不判为冲突。只对选中的游标做非负十进制整数及范围校验，非法非空请求头不能改用 URL 游标继续。

例如首次连接 `/api/events?after=100`，收到事件 105 后，原生 EventSource 自动重连会沿用初始 URL 并携带 `Last-Event-ID: 105`，服务端必须从 105 之后继续推送。该请求头优先级与浏览器重连行为一致。[HTML SSE 标准](https://html.spec.whatwg.org/multipage/server-sent-events.html#the-last-event-id-header)

客户端仅对持久事件按 payload.seq 去重，检测实体修订冲突后刷新对应数据。`preview.delta` 不参与持久游标去重，按 request/tool/offset 处理；浏览器事件的 lastEventId 可能继承前一个持久事件的 ID，不能据此丢弃生成预览。

持久同步事件保留 7 天，业务修订、工具、审核和反馈不随之删除。选中的游标早于保留窗口或大于当前事件水位时，发送 `stream.reset_required` 并关闭流；客户端收到后关闭旧 EventSource、重新取快照，并用新 after 创建连接，防止旧请求头继续参与重连。此同步操作不重新启动 Agent。

### 10.4 前端状态所有权

TanStack Query 缓存后端资源；XState 管理 workbench 的层级 UI 状态、连接状态和命令状态；组件本地 state 保存短暂展示信息。一个服务端业务字段不能同时在三处成为可写事实源。

工作台 machine 的根包含 loading/ready/error；ready 内按服务端阶段选择准备、正文、完本视图，同时以并行区域表示 connected/reconnecting 和 idle/submitting。用户点击只发命令并显示提交状态，收到服务端确认后再切换业务阶段。[XState 并行状态](https://stately.ai/docs/parallel-states)

API DTO 由 Rust 类型生成 OpenAPI，使用 `utoipa` 和 `openapi-typescript` 生成前端类型；锁定工具版本，检查生成差异。不手工维护另一份字段枚举，不把 SeaORM Entity 直接当 API 返回类型。[utoipa](https://docs.rs/utoipa/latest/utoipa/)、[openapi-typescript](https://openapi-ts.dev/introduction)

章节目录与内容查询分离，虚拟列表以 chapter_id 作 key；点击未挂载章使用虚拟列表定位与测量，不能依赖 DOM 已存在。[TanStack Virtual](https://tanstack.com/virtual/latest/docs/introduction)

### 10.5 引用与草稿

引用数据：document_id、revision_id、`start_char`、`end_char`、quote、所属 chapter_id（若有）。位置单位统一为 Unicode scalar value 数，采用左闭右开区间；后端按原修订验证 quote 完全一致。

浏览器原生 selection offset 常为 UTF-16 code unit，提交前必须显式转换，Rust 不得把前端数字直接当 UTF-8 字节偏移。引用跨多个 DOM 文本节点时映射到同一底层原文；跨章节选择不提供引用操作并解释范围限制。

正文提案中的新文本尚无文档修订，引用改用 `tool_call_id + candidate_hash + candidate_range`，服务端验证候选内容；接受后可解析到结果修订，拒绝后仍引用提案快照。不能给待审新增文字伪造正式 revision ID。

Dock 草稿按目标键存到浏览器本地存储，提交成功才清除；提交失败保留原文本。已提交的意见以数据库为准。引用过期仍允许阅读原修订，但不得据旧位置自动修改现稿。

## 11. 中断、退出与本地交付

### 11.1 持久检查点

检查点至少覆盖：准备发请求、完整响应入库、完整工具入库、提案待审、等待问题答案、工具结果已提交、意见已绑定、请求已封存、审核已决定、阶段产物纯文本已折叠。流式 token 不要求逐个落盘。

启动时检查非终态 Run：稳定的等待问题、待审提案和已暂停状态直接恢复其界面；原来 queued/request_in_flight/tool 执行中等自动执行状态标记为 interrupted，展示“上次运行中断”，不自动发新的付费请求。

用户继续时按以下事实推进：

| 中断位置 | 继续行为 |
| --- | --- |
| 请求未实际发送 | 重新读取最新设置构建新请求 |
| 请求是否被服务端接受不确定 | 原 attempt 标记 unknown；明确继续会产生新 attempt，原费用可能未知 |
| 工具调用已持久化但编辑未提交 | 根据门控及当前修订重新处理同一调用 |
| 编辑已提交但模型未收到结果 | 使用已存工具结果和原调用 ID 继续，不重复编辑 |
| 意见已绑定但请求未封存 | 仍绑定原工具结果，构建下一请求时包含 |
| 审核决定已提交但 UI 未收到 | 快照/事件重建决定，不重复审核 |
| 阶段通过与折叠已提交，下一请求尚未发出 | 从产物纯文本 fold 构建下一请求，不重放已折叠的阶段历史 |
| 本轮请求额度耗尽或连续协议错误触发停止 | 用户显式继续时复用原 Run 并按 §2.3 续授额度，辅助 quota 与累计用量保留 |
| 向量索引写入中断 | 继续同一索引任务，以确定 chunk ID 幂等写入并重验源修订 |

本地副作用可用事务与调用 ID 去重；无法保证外部模型服务在网络不确定时恰好计费一次。UI 和统计必须保留这种不确定性。

### 11.2 停止与退出

停止是明确命令：不再发出后续请求、取消在途请求、保存已提交正文及未投递意见，把 Run 标为 stopped 并释放执行资源。待审提案保留；继续原 Run 后仍须作出决定，不能在停止或继续时自动应用。停止操作记录失效的 request/attempt ID，取消后迟到的模型回调不得重新应用工具或推进状态。

停止后的显式继续按 §2.3 续授模型请求额度；单纯从用户暂停恢复使用原有剩余额度。达到工程上限而停止时，自由意见仅保存入队，不能借由“提交意见并继续”的普通暂停操作自动续授额度。

退出服务先拒绝新生成、取消/收尾在途请求、提交必要状态、关闭数据库连接，再退出。已经发往外部服务的请求是否取消成功不作保证。UI 显示连接已断开和重新启动方法。

数据目录使用 OS 本地应用数据路径下的 `NekoWriter`：`nekowriter.sqlite3`、`lancedb/`、`logs/`、进程锁文件。`--data-dir` 指定目录需解析成绝对路径，禁止把未校验路径用于清理操作。

服务绑定回环地址，默认固定端口 47821，可通过 `--port` 指定。端口占用时报错并说明，不静默连接到未知服务。Windows 启动器打开默认浏览器，后台进程生命周期与浏览器标签页独立。

### 11.3 基础本地安全

校验 Host 和写请求 Origin，写接口只接受 JSON 与自定义请求头；浏览器不能跨站直接触发本地写入。开发 CORS 仅允许明确的 Vite 地址。所有 GET 接口保持只读。

渲染不启用 Markdown 原始 HTML，外链只允许 http/https 并安全打开，服务端文件导出名不包含路径穿越内容。知识和研究内容作为资料块输入模型，不当作能改变工具权限的系统指令。

API key 存 OS 凭据库，SQLite 保存引用，日志和请求快照脱敏。移除服务配置前提示仍引用它的项目，不能留下不可见的悬空配置。日志滚动保存有限数量文件，不搭建外部监控平台。

### 11.4 导出

Markdown/TXT 导出当前稿本已提交正文，按正文计划顺序和番外顺序排列，包含小说标题和章节标题；待审提案、流式预览、系统提示词、Token 记录及密钥不进入导出。未通过章节明确标为草稿，不伪装为定稿。

导出在一致的读取快照中完成，不因生成期间翻页查询而混入多个时刻的章节版本。导出是用户获取文本的功能，不承担数据库格式兼容或迁移。

## 12. Dashboard 口径

### 12.1 Token Usage

数据来自实际模型请求的 `usage_json`，按项目、时间、角色（写作/审核/Embedding）汇总。研究未来接入后也必须归属其父项目；系统连接测试单列为系统操作。阶段上下文折叠由本地程序执行，不产生模型用量，也不删除历史已发生的用量。

字段采用 input_tokens、output_tokens、cached_input_tokens、reasoning_tokens、usage_source、usage_status。cached input 是输入子集，reasoning 若是输出子集就不能再次加到总量；各供应商由适配器统一映射并保留原始用量字段。

总量以已知 input + output 为主，未知请求数另行展示。Embedding 的输入用量独立列示，不能混入“对话 Token 总数”而无说明。MVP 不计算费用或维护价格表。

### 12.2 Running Jobs

Dashboard 的进行中任务包括排队、模型运行、等待用户、等待审核和已暂停，分别标识；已中断与失败位于需处理列表。主计数按顶层 Run 统计，审核子 Run 展开显示，避免一个作品因父子任务重复计数。

执行槽只有一个也可以有多个等待用户的项目，界面不得把“无模型正在请求”显示成“没有进行中的写作”。

### 12.3 Word count increment

字数采用 Rust Unicode 字符遍历，排除 `char::is_whitespace()`，标点及可见符号计入；这属于中文写作产品口径，不是语言学词数。章节标题、细纲、设定和模型思考不计正文总字数。

每次正文修订记录 `delta = new_count - old_count`，正负均可；拒绝编辑、重复处理和复制稿本为 0。删除/恢复对象属于库存变动，不伪装为创作事件；图表展示写作净增，另显示当前可见稿本总字数。

跨稿本总量明确表示所有可见稿本的内容总量，复制会增加内容库存但不增加“今天创作”。Dashboard 默认只汇总各项目当前选中稿本，用户切到全部稿本时提示可能包含相同内容。

统计范围使用用户选定时区转换成 UTC 左闭右开区间，日期分桶在查询层统一完成，不能前后端分别猜测“今天”。

## 13. UI 验收规则

布局、视觉和 Dock 状态见方案 §6，以下补充实现判定：

- 首次进入空项目时提供小说目标输入和开始按钮；缺模型配置时给出明确设置入口。
- 每个 Dock 状态只有一个主要推进操作，审核、提问和自由意见的语义清楚；等待时不显示可重复启动按钮。
- 每个准备 Tab 显示当前修订、待审/通过/拒绝/失效状态，切换不会丢意见草稿。
- 自动接受模式显示“编辑自动接受，整章仍需审核”；逐次审核模式展示真实差异。
- “已保存”只在后端确认提交后显示；草稿自动保存到浏览器时使用“输入草稿已保存到本机浏览器”。
- 新内容跟随只在用户原本处于生成末尾且没有选择/差异操作时生效；滚动回看不被打断。
- 字体缩放、窗口变化、Dock 拖动触发章节高度重测，保持当前阅读锚点。
- 正式正文始终可选中复制；一条引用必须对应可验证的原文或提案快照。
- 所有对话框管理焦点，Escape 只关闭非破坏性面板；Ctrl/Cmd+Enter 提交时避开中文 IME composition。
- 待审差异使用新增/删除文本标签，不只依靠红绿；键盘可访问变量说明、目录、模式切换和委托按钮。
- 连续屏幕阅读播报只报状态变化与完整段落摘要，不逐 token 打断用户。
- 模型失败、索引失败和零引擎能力显示实际原因，不能以无限 skeleton 隐藏不可完成状态。

## 14. 实施任务清单

任务按阶段交付；每项的完成证据记录在实现 PR/变更说明中。M0 已在 Windows x64 本机完成联合编译、测试和 HTTP 冒烟验证。M1 已有可运行的本地闭环、自动准备/细纲请求，以及自动接受的正文 `append_content` / `replace_content` 循环；真实外部模型验收仍待完成，不能以测试 SSE 服务替代。

### M0 本地骨架

- [x] 建立四 crate workspace、工具链与锁文件；Windows 上验证 Axum/SeaORM/LanceDB 联合编译。
- [x] 实现数据目录、进程锁、当前 schema 初始化、数据库 pragma 校验。
- [x] 接通同源静态页面、健康检查、项目/稿本创建与持久查询。
- [x] 用临时数据执行 LanceDB 写入和查询烟测，记录实际原生依赖。

### M1 最小写作闭环

- [x] 实现当前阶段所需数据表、Core 状态转换与命令回执。
- [x] 接通 Chat Completions 兼容模型协议、能力校验、内建提示词和结构化输出；真实外部服务烟测待凭据可用时记录。
- [x] 完成四项准备、人工审核、单章指导、细纲审核、自动接受正文和整章审核的本地事务与界面路径。
- [x] 实现阶段全部产物通过后的纯文本折叠、审核/推进原子提交、按折叠来源构建下一请求（准备、细纲与正文自动 `append_content` / `replace_content` 已接通；`ask_user`、逐次审核和意见门控仍属于后续阶段）。
- [x] 接通 SSE 快照同步、正文显示、任务状态和基础用量记录。

### M2 交互正确性

- [x] 实现编辑提案、基础修订校验、逐次审核和差异 UI；候选正文只存于 `tool_calls.proposal_json`，接受/拒绝均以命令回执和同一 Run 的工具结果原子落库。
- [x] 实现暂停控制、未封存意见批次的统一追加/投递、直接终态提交完成守卫和模式切换边界；等待正式问答时的自由意见不会替代答案。
- [x] 实现 `ask_user`、重复命令回执、原生 EventSource 游标重连、进程中断恢复与每次 100 次请求额度续授；完整但无效的模型响应按连续三次协议错误上限纠正或停止。
- [x] 实现用户显式停止：同事务作废 prepared/dispatched attempt、记录失效请求 ID，并中止本地在途 HTTP 任务；迟到回调不能推进工作流，待答/待审/暂停边界在显式继续后恢复。
- [ ] 实现引用与附件性反馈。
- [ ] 通过 §15 中全部编辑、反馈、审核竞态用例。

### M3 完整创作流程

- [x] 完成结构化章节计划、多章切换、多阶段产物纯文本上下文、稿本复制、完本与番外。多章计划录入、自动正文、稿本新建/复制/切换、完本与番外、已完成阶段 fold，以及章节目录与任意章节的只读浏览/跳转均已接通；长篇虚拟化与滚动锚点仍属于 M5。
- [x] 完成各类型审核委托与过期结果处理：准备产物、细纲、逐次正文编辑和终稿均有独立提示词、目标/依赖重验、取消、失败/重启恢复与成功回调重放保护。
- [x] 完成系统/项目设置、提示词覆盖、变量说明和请求快照。系统写作模型设置与 Windows 凭据引用、内建提示词的系统级/项目级覆盖、变量元数据说明与保存前预览均已接通；项目级模型引用、思考等级与辅助工具预算具备持久化、独立乐观并发修订、API 与工作台界面；模型引用连同它自己的能力资料一起保存，因此项目覆盖不会沿用系统模型的能力声明。调度器在每次取得许可后重新解析「项目覆盖 → 系统默认」的有效模型与思考等级，把系统/项目设置修订、提示词来源、有效模型、有效能力资料与思考等级写入请求快照，并在有效模型未声明支持所需能力时于发请求前报错；命令受理前的同一判定与连接测试都使用同一份解析结果。辅助工具预算的账本扣减依赖辅助工具本身，随 M4 的知识检索一并实现。

### M4 知识与管理

- [ ] 实现 Space、知识 CRUD、Embedding 接入、索引任务与语义检索，补齐过期候选过滤后的持续补取及单次计费。
- [ ] 实现研究/积累接口、能力注册表及零生产实现的明确返回行为。
- [ ] 实现模板模块和可回算的 Dashboard 统计。

### M5 本地交付

- [ ] 完成长篇章节虚拟化、滚动锚点、主题、键盘及中文输入体验。
- [ ] 完成软删除/恢复、正文导出、退出流程及脱离源码的 Windows 发行包。
- [ ] 记录真实模型验收、性能数据、依赖版本和已知限制。

## 15. 测试与验收矩阵

### 15.1 测试层次

Core 单元测试关注业务不变量与字符串边界；SQLite 临时文件集成测试关注跨表事务、去重和检查点；Agent 测试用可控假服务模拟流、工具与错误；Playwright 验证真实界面的关键写作路径。

假模型只用于自动化测试。M1、M3 和最终交付必须完成真实模型烟测，记录模型 ID、服务地址类型、锁定版本、已验证能力和用量；没有可用凭据时应明确标记真实模型验收未完成，不能以假模型替代。

常规检查为 Rust fmt/clippy/test，以及前端 TypeScript 项目构建、lint 和相关 UI 测试。现有前端 `build` 使用 `tsc -b && vite build`，实施时采用项目构建检查；根引用式 tsconfig 的单独 `tsc --noEmit` 不应成为唯一类型验证证据。

### 15.2 高风险用例

| 编号 | 场景 | 必须观察到的结果 |
| --- | --- | --- |
| A01 | 初次准备只提交三项 | 不创建可进入正文的通过状态，返回缺失项 |
| A02 | 四项齐全但一项未通过 | 留在审核/修订流程，不能开始细纲 |
| A03 | 资料修订改变背景依赖 | 相关通过状态失效，UI 标明原因 |
| A04 | 细纲未通过就提交正文工具 | 拒绝错误阶段，正文不变 |
| A05 | 最后一章通过 | 显示确认完本，用户确认前不自称完本 |
| A06 | 完本后新增番外 | 保留正文完本标记，番外走完整章节流程 |
| E01 | append 正常提交与重复调用 | 只增加一次内容和一条字数变化 |
| E02 | replace 无匹配/两处匹配/重叠匹配 | 均拒绝，原文不变 |
| E03 | 中文、emoji、组合字符、CRLF 输入 | 无非法切片；引用与匹配遵循统一规则 |
| E04 | 新编辑尚未批准 | 正式正文不变，差异提案可见 |
| E05 | 空审核意见与非空审核意见 | 分别接受与拒绝提案；空自由意见不批准 |
| E06 | 提案基础修订被改变 | 接受返回冲突，不自动套用到新正文 |
| E07 | 模型流中断在工具 JSON 中间 | 不执行编辑，正式正文不变 |
| E08 | 模型一次给出多个工具调用 | 全批不执行，每个调用有配对错误结果 |
| F01 | 点暂停，下一工具尚未到达 | 先显示等待暂停，到边界才已暂停 |
| F02 | 已暂停后提交意见 | 恢复调用、清除暂停、工具结果带意见 |
| F03 | 尚未挂起且最近结果未封存时提交意见 | 绑定该结果并取消暂停 |
| F04 | 最新工具结果已随请求发出后提交意见 | 历史请求不变，意见等待新结果 |
| F05 | 正常运行、没有未封存批次，已有旧 resolved 结果时提交首条意见 | 新批次等待之后完成的工具结果，区别于暂停特例和已有批次追加 |
| F06 | 同一交付窗口连续提交三次意见 | 按顺序一次交付三条，无重复或丢失 |
| F07 | 请求封存后又提交新意见 | 新意见留到下次，已封存快照不变 |
| F08 | 有未投递意见时模型直接提交章节 | 不进入终稿审核，意见保留并继续协议处理 |
| F09 | 暂停请求期间模型直接提交章节 | 停在提交边界，可继续，无永久等待 |
| F10 | 逐次审核切到自动接受且有待审提案 | 既有提案仍待决定；后续新提案用新模式 |
| F11 | 自动接受切逐次审核与新调用竞争 | 由命令顺序决定，UI 与后端控制修订一致 |
| F12 | 等待回答时提交自由意见 | 问题未自动回答，意见不丢 |
| F13 | A 已绑定 resolved 工具结果，封存前追加 B | A、B 按序一起进入该结果，不把 B 推迟到下一次工具调用 |
| F14 | 工具已到达但等待回答，期间多次提交意见 | 回答完成产生结果时绑定整个批次；调用到达时间不阻止投递 |
| F15 | 批次尚未绑定，暂停未挂起时提交意见且有可追加最近结果 | 整个批次改绑最近结果并取消暂停，无意见拆批或丢失 |
| R01 | 委托返回缺字段、字符串 bool、NaN/越界置信度 | 不应用决定，明确协议错误 |
| R02 | `passed=true, confidence=0.2` | 通过且显示置信度，不隐式加阈值 |
| R03 | `passed=false, issues=[]` | 拒绝并显示缺少具体问题的系统说明 |
| R04 | 用户取消委托后迟到结果 | 不改变当前审核结果 |
| R05 | 审核返回时目标修订已变化 | 标为过期，不批准新修订 |
| Q01 | quota 用完、重试同一调用、修改预算 | 不超扣，重复不扣，下调不清零已用 |
| Q02 | 免费工具与委托审核 | quota 不变，实际模型 Token 仍记录 |
| Q03 | 同一 Run 已完成 100 次请求后用户继续 | 累计保持 100，上界变为 200，第 101 次请求可发送，辅助 quota 不重置 |
| Q04 | 重复继续命令、普通暂停恢复、提交意见、刷新或启动程序 | 同一命令只续授一次；普通交互与程序启动不续授额度 |
| Q05 | prepared 后取消、dispatched 后失败/未知、重复回调 | 分别不计次、计一次、不能重复计次；累计数及额度跨重启保持 |
| Q06 | 连续协议错误触发停止后用户显式继续 | 保留旧错误和请求记录，连续错误计数归零，新请求受新额度约束 |
| K01 | 没有研究/积累引擎 | 能力为空，调用不启动任务不扣费 |
| K02 | 没有 Embedding 配置 | 知识可保存，语义检索明确不可用 |
| K03 | 索引完成前条目被更新/删除 | 旧结果不可检索，旧任务不误标 ready |
| K04 | Embedding 更换维度或模型 | 索引隔离，不混用向量 |
| K05 | 索引事务中途失败再继续 | chunk 不重复，ready 前不暴露部分结果 |
| K06 | top_k=5，前五个候选均过期，后续存在有效候选 | 继续补取并返回最多五个有效结果，不误报空 |
| K07 | 多轮补取包含重复 chunk，最终有效候选不足 top_k | 按 chunk_id 去重，确认候选耗尽后返回实际数量，无死循环 |
| K08 | 一次搜索多轮补取，或返回前所选条目被删除 | 只产生一次查询 Embedding 和一次 quota 消耗；删除条目被剔除并继续补取 |
| K09 | 候选未耗尽时触发搜索超时或资源限制 | 返回明确搜索失败，不把未完成搜索包装成空结果 |
| S01 | 模型请求排队时修改设置 | 真正发送时采用最新配置 |
| S02 | 模型在途时修改项目提示词 | 当前快照不变，下一请求使用新内容 |
| S03 | 清除项目覆盖与系统覆盖 | 依次解析到系统覆盖与内建内容 |
| S04 | 模板含未知变量或嵌套占位文本 | 按规则报错或只替换一次，不执行表达式 |
| C01 | 准备阶段只有三项通过或有失效项 | 不折叠共享阶段历史，保留修订所需消息 |
| C02 | 准备阶段四项全部通过 | 下一请求中的该阶段内容只有按固定顺序排列的四项完整产物文本 |
| C03 | 细纲通过、整章终稿通过、番外通过 | 分别折叠对应阶段，保留前序产物及新阶段消息 |
| C04 | 单次正文编辑被自动接受或人工批准 | 不提前折叠未完成的正文阶段 |
| C05 | 折叠包含结构化粗纲及资料来源 | 文本与审核时呈现内容一致，章节计划及来源无缺失，无消息/工具/审核 JSON 信封 |
| C06 | 审核拒绝后修订再通过 | 修订期间保留过程；通过后仅保留最终通过修订全文 |
| C07 | 最终通过事务前后中断或重复审核回调 | 阶段状态与 fold 一致，下一请求无重复块、无旧历史回流 |
| C08 | 折叠后查看本地历史与旧请求快照 | 原始消息、审核、引用依据及历史用量仍在，已发请求未改写 |
| C09 | 阶段存在未完成工具或未投递意见 | 拒绝完成折叠，未完成事项保持可处理 |
| C10 | 多阶段折叠全文仍超过模型窗口 | 明确报告上下文超限，不调用摘要模型、不裁剪产物 |
| D01 | HTTP 成功后响应丢失，客户端重试 | 原命令只执行一次 |
| D02 | 快照读取与 SSE 建连之间发生更新 | 更新仍由水位后的持久事件收到 |
| D03 | SSE 重复事件或超过保留窗口 | 去重或重新取快照，不重复启动生成 |
| D04 | 编辑提交后立即杀进程 | 重开正文只修改一次，结果可继续交付 |
| D05 | 两标签页同时批准相同提案 | 最多一次接受，另一请求返回既有结果或冲突 |
| D06 | 复制稿本和软删除/恢复 | 无串写、无虚构创作增量，子对象删除状态正确 |
| D07 | 首连 after=100，收到 105 后原生 EventSource 自动重连 | 接受原 URL 与 Last-Event-ID=105，继续推送 seq>105 的事件，不重启 Run |
| D08 | 请求头游标非法，或选中游标超出保留范围 | 非法值报错；超出范围要求新快照，客户端关闭旧连接后重新订阅 |
| D09 | 持久事件后连续收到不带 id 的 preview.delta | 持久事件仍去重，预览按自己的 request/tool/offset 更新，不因继承 lastEventId 被丢弃 |
| U01 | 用户回看早期章节时新增正文 | 视口保持，有新内容提示 |
| U02 | 点击未挂载章节、调整 Dock/字体 | 正确滚动到目标章，锚点稳定 |
| U03 | 引用含 emoji 或待审新增文本 | 后端验证通过，来源可重定位 |
| U04 | 中文输入法按 Enter/Ctrl+Enter | 组合输入不误提交 |
| P01 | 在全新数据目录启动发行包 | 无源码、无额外数据库服务即可使用 |
| P02 | 第二进程打开同一数据目录 | 明确拒绝，数据库不被双写 |
| P03 | 打开不支持的 schema | 保留原文件并报错，不迁移或重建 |
| P04 | 百万字样本和断线重连 | 达到方案性能目标或明确记录未达指标 |

## 16. 原始需求覆盖索引

| 原始需求 | 本文位置 | 主要验收 |
| --- | --- | --- |
| Rust/Axum/SeaORM/内建 Agent/LanceDB、目录约束 | §2、§8、方案 §2–§4 | M0、P01 |
| Dashboard 三项指标 | §12 | E01、D06、用量回算 |
| 项目设置、小说标题、小说版本 | §3.4、§8、§9.3、§10 | D06、S01 |
| 分层状态机、章节列表与实时展示 | §3、§10.3–§10.4、§13 | D02、U01–U02 |
| 引用、问答、四 Tab、章节指导 | §3.3、§4.4、§10.5、§13 | A01–A04、F12、U03 |
| 初始化、准备硬性目标与阶段推进 | §3.2–§3.3、§4.3 | A01–A06 |
| AutoApprove 暂停与意见排空 | §5.1–§5.3 | F01–F09、F13–F15 |
| ApproveRequired、模式切换、正文工具 | §4.5、§5.4–§5.5 | E01–E08、F10–F11 |
| 所有审核可委托、不同提示词与返回结构 | §6、§9.1 | R01–R05 |
| 审核拒绝后修订并重新提交 | §3.2、§6.1 | A02–A04、E05 |
| 内建系统/项目提示词、宏嵌入、变量说明 | §9、§13 | S02–S04 |
| 每次请求最新设置与缓存优化 | §2.2、§9.3–§9.4 | S01–S02 |
| 所有产物通过后阶段历史折叠为仅产物内容的纯文本 | §3.2、§6.3、§8.2、§9.4、§11.1 | C01–C10 |
| 独立提示词模板模块 | §9.2、§10.1 | S04 |
| 知识条目、Space、知识积累基础设施 | §7.2–§7.4、§8 | K01–K09 |
| 联网研究子 Agent 能力、零具体引擎 | §4.2、§7.1、§7.4 | K01、Q01 |
| 大部分软删除但非全部 | §8.4 | D06、P03 |
| 本地运行补全：执行额度、继续与断线同步 | §2.3、§3.1、§10.3、§11 | Q03–Q06、D07–D09 |

本次文档交付不声称上述测试已运行。实现阶段必须补齐测试证据和真实模型验证，才能把相应任务标为完成。
