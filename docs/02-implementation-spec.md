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

阶段角色的 system 消息必须写明本阶段的提交结构（字段名、取值约束与「只输出 JSON」的要求）；章节级提交还必须带上本次请求对应的 `chapter_id` 具体值，因为模型无法自行推断它。合同文本与解析器同源（`nekowriter_agent::runner::submission_contract`）：改了字段就必须同时改解析器与合同，否则模型只会拿到一份过期的要求。

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

反馈状态为 `queued → bound → included`。`included` 表示已放入某次封存请求，不承诺模型理解或网络另一端已经接收。UI 先显示“等待投递/已加入请求”，请求进入 dispatched 后才显示“已随请求发送”；请求失败时补充“请求失败，尚无模型回应”。prepared 请求尚未发出即被停止时，保留该请求快照并标记 cancelled，继续时把同一工具消息与意见纳入新的请求快照，不创建第二条逻辑意见。正文快照同时提供 `delivered_feedback`：只返回当前 Run 最新且未成功响应的请求所携带的 `included` 意见，并附带 `sent|failed|unknown|cancelled` 投递状态；Dock 因而把它们和 `queued` / `bound` 队列分开显示。最近请求成功后不在当前 Dock 保留历史意见，持久请求快照、工具消息和 feedback 行仍保留用于追溯。

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

若模型提交章节时仍有自由意见且不存在可追加的工具结果，不允许进入整章审核。保存意见，拒绝本次阶段提交，通过协议纠正消息要求继续该阶段并产生合法工具调用，意见仍按工具结果规则投递。若连续不产生可推进动作，触发工程执行限制并展示未投递意见，不静默丢弃或改写历史结果。这类纠正与无效模型响应共用同一个 `consecutive_protocol_errors` 计数：每拒绝一次不推进的提交加一，达到 3 次即把 Run 收束为 `stopped`（`wait_reason = protocol_error_limit`、`ended_at` 落库），事件带上 `reason = pending_feedback_requires_tool_result` 与最新计数；只有真正推进到稳定业务边界的响应才把该计数清零，暂停恢复时的那一次纠正不单独重置它。这条上限在真实进程与发行产物上都验证过：固定「永远直接提交」的桩最多发出 4 次请求即收束为 `stopped`，未投递意见保持 `queued`，界面显示停止原因与「显式继续」（见 docs/03 §28、§29）。

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

积累命令的成功路径是一条普通写入命令：请求携带 `commandId`、目标 Space 与线索；先查该 `command_id` 的回执（命中且内容一致时返回原条目，**不再次调用引擎**；同 ID 不同内容返回 409 `IDEMPOTENCY_CONFLICT`），再校验线索非空与目标 Space 存在，再取本次进程的积累引擎。引擎返回候选条目后，应用服务在 Store 边界逐条验证（标题、关键词、正文与来源沿用条目创建的同一套规则），并在同一事务里插入全部条目、为每条建立索引任务、写入回执：任何一条候选无效或引擎执行失败都不留下半批条目，也不留下可重放的回执。引擎不可用返回 409 `ENGINE_NOT_AVAILABLE`（不创建运行任务、不扣 quota），引擎执行失败返回 502 `ENGINE_FAILED`。引擎注册表属于应用状态，`GET /api/capabilities` 与积累入口读同一份，因此「能力查询为空」与「入口返回不可用」不会互相矛盾。

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
| `system_model_operations` | id、kind、model_id、api_base_url、status、provider_request_id?、usage_json、error_json?、created_at、completed_at?；与稿本无关的系统请求（目前只有连接测试） |

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

回收站是只读快照加两个动作：`GET /api/recycle-bin` 按删除时间倒序列出全部软删除对象（稿本、番外章、用户模板、知识条目、空 Space），每条都带上 `restorable` / `purgeable` 与各自被拒绝的原因，因此界面可以直接说明「现在为什么不能恢复/不能彻底删除」，而不是等用户点下去才失败。每次删除、恢复与彻底删除都在同一事务里写一条 `recycle.changed` 事件，工作台据此刷新回收站与目录，不靠手动刷新。

删一个项目（`DELETE /api/projects/{id}`）只标记项目本身，不批量改写它下面任何孩子的 `deleted_at`；所有读取路径逐层检查祖先，因此项目在回收站里时它的稿本、章节与正文都不可见。恢复项目也只清项目自己的 `deleted_at`，于是它的稿本与章节原样可见，而删除项目之前单独删掉的稿本仍然留在回收站里（不会因为恢复项目而被误恢复）。项目下任一稿本仍有开放审核或顶层写作 Run 时，删除项目返回 `RESOURCE_BUSY`。

恢复只把 `deleted_at` 清回 `NULL`：章节行、文档槽、修订链与在作品里的顺序都不动，所以恢复后的正文、`revision_id` 与 `order_index` 与删除前完全一致，不会为了「恢复」再写一条新修订。恢复不把工作流的写作光标搬回去：删除番外时 `withdraw_extra_chapter` 已经把工作流退回到仍然可见的最后一章，删除之后用户可能已经在别处继续写，所以恢复只恢复数据与位置，光标保持在删除后的位置。恢复需要它的祖先仍然可见：父对象（稿本）还在回收站里时返回 409 `RECYCLE_ANCESTOR_DELETED`，对象不存在或已被彻底删除时返回 404，不做半恢复。

彻底删除只对用户模板、知识条目与空 Space 开放（`POST /api/recycle-bin/{kind}/{id}/purge`），删除后不可恢复；项目、稿本与番外章不提供物理删除，请求会被拒绝为 `RECYCLE_NOT_PURGEABLE`，理由是正文与它的事件、审核和修订引用共同构成审计链，本版本只把它们留在回收站里。删除项目用 `DELETE /api/projects/{id}`（携带 `expected_revision`，过期返回 `REVISION_CONFLICT`），恢复用 `POST /api/recycle-bin/project/{id}/restore`。番外章用 `DELETE /api/chapters/{id}` 软删除（正文计划章不支持单独删除，返回 `INVALID_CHAPTER_STATE`，有开放审核或活跃写作任务时返回 `RESOURCE_BUSY`），稿本用 `DELETE /api/versions/{id}` 软删除（当前选中的稿本要先切走，否则返回 `INVALID_VERSION_STATE`）。

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

上述语言、变量声明与渲染在 `nekowriter-core::template` 中实现一次，`agent::prompts` 的内建目录与用户模板共用它；因此“拒绝未知变量、缺失必需占位符与不完整括号”只有一份判定。内建变量声明另带 `source`（运行值来自哪里），用户模板变量声明不需要它，因为插入时由用户提供值。

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
| `GET /api/health`、`GET /api/capabilities` | 本地状态（含实际数据目录）与可用模型/研究/积累能力 |
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
| `POST /api/reviews/{id}/decision` | 人工审核决定、意见与引用 |
| `POST /api/reviews/{id}/delegate`、`.../cancel-delegation` | 委托及取消委托 |
| `GET /api/documents/{id}/revisions/{revision}` | 引用与历史修订读取 |
| `GET /api/events?after={seq}` | 全应用 SSE，前端按实体分发 |
| `/api/prompt-templates`、`/api/prompt-templates/{id}` | 用户模板 CRUD（创建/读取/替换/软删除，携带 `command_id` 与 `expected_revision`） |
| `/api/prompts/system`、`/api/projects/{id}/prompts` | 内建提示词目录、来源与覆盖 |
| `POST /api/prompts/preview` | 类型化模板校验与预览；`builtin` 变体预览内建提示词草稿，`template` 变体预览用户模板 |
| `/api/knowledge/spaces`、`/api/knowledge/entries` | 知识 CRUD |
| `POST /api/knowledge/search`、`.../reindex`、`.../accumulate` | 语义检索、显式重建、积累入口 |
| `GET /api/dashboard` | 指定时区/时间范围/项目的统计 |
| `/api/settings`、`POST /api/settings/test-model`、`.../test-embedding` | 系统设置与连接测试；test-model 请求体携带被测模型 ID 与能力资料 |
| `GET /api/recycle-bin`、`POST /api/recycle-bin/{kind}/{id}/restore`、`POST /api/recycle-bin/{kind}/{id}/purge` | 回收站、恢复与彻底删除（只对模板/知识对象） |
| `DELETE /api/chapters/{id}`、`DELETE /api/versions/{id}`、`DELETE /api/projects/{id}` | 番外章、稿本与项目的软删除（正文计划章与当前选中稿本会被拒绝，项目下有活跃任务会被拒绝） |
| `GET /api/versions/{id}/export?format=md\|txt` | 导出已提交内容 |
| `GET /api/versions/{id}/manuscript?offset=&limit=` | 长篇阅读窗口：按阅读顺序取这一窗口的已提交正文（默认 20 章、上限 50） |
| `GET /api/versions/{id}/manuscript/search?q=` | 整稿查找；命中位置与引用同一单位（Unicode scalar，左闭右开） |
| `POST /api/system/shutdown` | 显式退出本地服务：先把响应写回客户端，再停止接受连接、取消在途请求并收束在执行中的 Run；已请求退出时返回 `already_shutting_down` |

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

临时事件使用 `preview.delta`，不携带可恢复的 SSE `id`。当前线格式为：

```text
event: preview.delta
data: {"versionId":"v1","requestId":"r1","tool":null,"offset":"0","content":"正在生成"}
```

`tool = null` 表示普通文本；工具参数流使用 `{"index":0,"id":"call_1","name":"append_content"}`，并为每个工具槽位独立计数。`offset` 是 Unicode scalar 偏移，序列化为十进制字符串。半截 JSON 参数只在独立生成预览层展示；不把字符流直接写入正式正文。预览丢失后丢弃该请求的临时缓冲，不影响已提交修订。

快照响应在同一个 SQLite 读事务中取得业务数据与事件水位。客户端随后从该水位订阅事件。服务端以 events 表为准持续读取 `seq > cursor`；内存广播只用作唤醒，广播溢出时继续按游标读数据库，避免“查历史到订阅实时之间”丢事件。

每 15 秒发送 SSE 注释心跳。游标解析优先级固定为：存在非空 `Last-Event-ID` 请求头时只以该值为准；请求头缺失或为空时使用 URL 的 `after`，首次连接传入快照水位。两者同时存在且不同是正常重连情况，不判为冲突。只对选中的游标做非负十进制整数及范围校验，非法非空请求头不能改用 URL 游标继续。

例如首次连接 `/api/events?after=100`，收到事件 105 后，原生 EventSource 自动重连会沿用初始 URL 并携带 `Last-Event-ID: 105`，服务端必须从 105 之后继续推送。该请求头优先级与浏览器重连行为一致。[HTML SSE 标准](https://html.spec.whatwg.org/multipage/server-sent-events.html#the-last-event-id-header)

客户端仅对持久事件按 payload.seq 去重，检测实体修订冲突后刷新对应数据。`preview.delta` 不参与持久游标去重，按 request/tool/offset 处理；浏览器事件的 lastEventId 可能继承前一个持久事件的 ID，不能据此丢弃生成预览。服务端用独立的 256 槽内存广播发送预览，落后订阅者直接丢弃片段；前端只接受连续或重复一致的 scalar 偏移，持久请求事件、断线和切换稿本都会清除对应临时缓冲。

持久同步事件保留 7 天，业务修订、工具、审核和反馈不随之删除。选中的游标早于保留窗口或大于当前事件水位时，发送 `stream.reset_required` 并关闭流；客户端收到后关闭旧 EventSource、重新取快照，并用新 after 创建连接，防止旧请求头继续参与重连。此同步操作不重新启动 Agent。

### 10.4 前端状态所有权

TanStack Query 缓存后端资源；XState 管理 workbench 的层级 UI 状态、连接状态和命令状态；组件本地 state 保存短暂展示信息。一个服务端业务字段不能同时在三处成为可写事实源。

工作台 machine 的根包含 loading/ready/error；ready 内按服务端阶段选择准备、正文、完本视图，同时以并行区域表示 connected/reconnecting 和 idle/submitting。用户点击只发命令并显示提交状态，收到服务端确认后再切换业务阶段。[XState 并行状态](https://stately.ai/docs/parallel-states)

API DTO 由 Rust 类型生成 OpenAPI，使用 `utoipa` 和 `openapi-typescript` 生成前端类型；锁定工具版本，检查生成差异。不手工维护另一份字段枚举，不把 SeaORM Entity 直接当 API 返回类型。[utoipa](https://docs.rs/utoipa/latest/utoipa/)、[openapi-typescript](https://openapi-ts.dev/introduction)

章节目录与内容查询分离，虚拟列表以 chapter_id 作 key；点击未挂载章使用虚拟列表定位与测量，不能依赖 DOM 已存在。[TanStack Virtual](https://tanstack.com/virtual/latest/docs/introduction)

### 10.5 引用与草稿

引用数据：document_id、revision_id、`start_char`、`end_char`、quote、所属 chapter_id（若有）。位置单位统一为 Unicode scalar value 数，采用左闭右开区间；后端按原修订验证 quote 完全一致。

引用可以附在逐次编辑决定、自由意见、委托审核与人工审核决定（`POST /api/reviews/{id}/decision`）四条路径上；四种写入共用同一套校验：修订存在、`document_id` 与 `revision_id` 匹配、文档未删除、属于当前稿本、`chapter_id` 与文档归属一致、引用范围逐字匹配。准备阶段审核可引用本稿本任意文档；章节细纲与整章终稿审核只能引用本次审核所在章节的文档。尚未落库的候选引用只属于逐次编辑审核，不能借人工决定绕过候选哈希校验。

浏览器原生 selection offset 常为 UTF-16 code unit，提交前必须显式转换，Rust 不得把前端数字直接当 UTF-8 字节偏移。引用跨多个 DOM 文本节点时映射到同一底层原文；跨章节选择不提供引用操作并解释范围限制。

正文提案中的新文本尚无文档修订，引用改用 `tool_call_id + candidate_hash + candidate_range`，服务端验证候选内容；接受后可解析到结果修订，拒绝后仍引用提案快照。不能给待审新增文字伪造正式 revision ID。

Dock 草稿按目标键存到浏览器本地存储，提交成功才清除；提交失败保留原文本。已提交的意见以数据库为准。引用过期仍允许阅读原修订，但不得据旧位置自动修改现稿。

### 10.6 长篇阅读、目录虚拟化与阅读位置

阅读区是整部作品的**一个连续滚动区**，按章虚拟化（TanStack Virtual），只有视口附近的章节被挂载；正文按窗口向服务端索取（`GET /api/versions/{id}/manuscript`，默认 20 章、上限 50）。因此打开一部几百章的作品只需要读它真正要显示的那几章，挂载的章节数只与视口有关。目录与内容分离查询：`/chapters` 只给目录（`label`、`bodyWordCount`、状态，不含正文），`/manuscript` 给窗口正文。

章节顺序、显示名与草稿判定只由 `nekowriter-core::manuscript` 定义：正文按计划顺序在前，番外按自身出现次序编号在后；目录、阅读窗口与导出因此不可能给出三种不同的答案。

- **高度估计与测量**：未挂载章节的高度按目录带回的字数与当前字号估计，因此远距离定位与滚动条长度从第一次渲染起就接近真实；章节挂载后用真实高度替换估计值。加载占位**不参与测量**——把占位高度当成章节真实高度会让未加载区域的几何逐帧塌缩，定位随之错位。
- **定位**：点击目录或查找命中时先按虚拟下标滚动（`scrollToIndex`），再在正文加载、高度被测得后继续校正，直到目标块的块顶与视口顶对齐。不使用 `scrollIntoView()`，因此未挂载的章节也能定位；末尾章节被滚动范围夹住、无法对齐到顶部时接受可达位置。
- **阅读位置**：以「章 id + 块内像素偏移」保存在浏览器的 `nekowriter.reading-anchor.{versionId}`，滚动时按帧合并写入、离开阅读区时再写一次；重新打开阅读区（切换写作/阅读视图或重新加载页面）时先恢复位置。字号与窗口变化触发重新测量，并把当前章重新对齐到顶部——位置以章为单位稳定，不会因为高度变化跳到别的章。
- **查找**：服务端读遍全部已提交正文（与导出同一份快照）逐字匹配，仅对 ASCII 字母不区分大小写（这种折叠不改变字符数，偏移永远落在原文正确位置），命中不重叠、最多 50 条。偏移单位是 Unicode scalar value、左闭右开，与引用完全相同，因此命中可以直接高亮，也可以被引用校验复核。浏览器自带的页内查找只能看到已挂载的章节，这是窗口化的必然代价；搜索整部作品请用阅读区的查找。
- **内容完整性**：虚拟化只影响渲染，不影响内容——每一章都能滚动到、可以选中复制，正文逐字来自当前已提交修订，与导出、与 `/manuscript` 一致。

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

实现口径（`nekowriter-app::lifecycle`）：

- **唯一触发点**：Ctrl+C、SIGTERM 与 `POST /api/system/shutdown` 三条路径都汇到同一个 `Shutdown` 信号（`tokio::sync::watch`，用 `send_replace` 而不是 `send`：后者在没有接收者时不更新值也不唤醒）。信号触发后必须把「已请求退出」写回同一个信号，否则已经打开的 SSE 连接看不到退出，优雅退出会一直等它断开。
- **退出顺序**：服务器停止接受新连接（因此不再产生新的生成请求）→ 取消在途模型请求（abort 本地 HTTP 任务，最多等 5 秒；丢弃各稿本命令循环）→ 在同一批事务里把活跃 Run 收束为 `interrupted`、把 prepared / dispatched 请求标为 `cancelled` / `unknown` 并广播 `reason=process_shutdown` → `PRAGMA wal_checkpoint(TRUNCATE)` 把 WAL 回写主库文件 → 关闭 SQLite 连接池 → 返回退出码 0。已经发往外部服务的调用是否取消成功不作保证，这些不确定性由 `unknown` 与 `（用量未知）` 保留下来。
- **收尾与启动恢复共用同一条路径**：`Store::interrupt_active_runs(InterruptionSource)` 同时服务启动恢复与退出收尾，两者只有事件原因（`startup_recovery` / `process_shutdown`）与用户可见文案不同，因此不可能出现「退出时的收束与重开时的收束规则不一致」。
- **接口退出是幂等的**：`POST /api/system/shutdown` 先校验写请求 Host/Origin，再把 `{"status":"shutting_down"}` 写回客户端，然后才触发收尾（已请求退出时返回 `already_shutting_down`）；它接受空请求体，不带 `command_id`。
- **SSE 随退出结束**：持久连接在退出信号触发后立即结束，浏览器随后发现服务已不可达，界面据此显示「本地服务已退出」与重新启动方法，而不是一直显示正在重连。
- **日志**：启动时在数据目录创建 `logs/`，写入 `nekowriter.<YYYY-MM-DD>.log`，按天滚动、只保留最近 7 个文件；文件格式不带 ANSI 颜色，标准输出与文件同时写，退出前由写线程守卫刷盘。凭据与 API key 从不进入日志。
- **启动打开浏览器**：绑定端口成功后用系统默认浏览器打开 `http://127.0.0.1:{port}/`；打开失败只记日志，不影响服务。`--no-browser` 关闭该行为（无界面环境与探针使用）。
- **前端资源位置**：默认解析到可执行文件旁的 `webui/dist`，因此发行目录整体拷走后从任何工作目录启动都能打开界面；`--web-dir` 可覆盖（源码内开发用 `cargo run -p nekowriter-app -- --web-dir webui/dist`，或直接跑 Vite 开发服务器）。
- **`GET /api/health`** 额外返回实际使用的 `dataDir`，工作台「本地服务」面板直接展示它，用户不必去猜数据与日志在哪里。
- **发行目录（全平台同一形状）**：`cargo xtask package [--target <三元组>] [--debug] [--protoc <路径>]` 先构建前端（`pnpm --dir webui build`，保证包里的 `webui/dist` 是本次构建的结果而不是仓库里的旧产物），再用 `--target`（缺省为本机三元组，由 `rustc -vV` 读出）显式构建 `nekowriter-app`，最后组装 `target/package/<三元组>/nekowriter/`：`nekowriter`（Windows 上是 `nekowriter.exe`）、`webui/dist`、`README.txt` 与 `release-manifest.txt`。因为资源按「可执行文件旁的 `webui/dist`」解析，所有平台的发行目录形状完全一致；显式传 `--target` 也让多个平台的产物能共存。
- **发行清单 `release-manifest.txt`**：目标三元组、构建配置（`release` / `debug`）、可执行文件的容器格式、生成时间与逐个文件的字节数 / SHA-256（含 `webui/dist` 的完整资源清单）。打包时会读可执行文件头部判定容器格式（PE / Mach-O / ELF 及其机器类型），并在与三元组不符时**直接失败**：拷错或改名一个别的平台的二进制不会被当成打包成功。清单与包内文件可用 `sha256sum -c` 之类的工具逐行核对。
- **非 Windows 主机打 Windows 包**：`.cargo/config.toml` 把 `PROTOC` 钉在 Windows 工具目录上（`tools/protoc/bin/protoc.exe`，由 `tools/bootstrap-protoc.ps1` 下载校验）且 `force = true`；从其它平台构建时用 `--protoc <本机 protoc>` 覆盖（只是宿主构建工具，与目标平台无关）。Windows 目标需要在 PATH 里有该目标的链接器与 C 编译器：`x86_64-pc-windows-msvc` 用 `lld-link`（见 `.cargo/config.toml`），`x86_64-pc-windows-gnu` 用 `mingw-w64` 的 `x86_64-w64-mingw32-gcc`。

### 11.3 基础本地安全

校验 Host 和写请求 Origin，写接口只接受 JSON 与自定义请求头；浏览器不能跨站直接触发本地写入。开发 CORS 仅允许明确的 Vite 地址。所有 GET 接口保持只读。

渲染不启用 Markdown 原始 HTML，外链只允许 http/https 并安全打开，服务端文件导出名不包含路径穿越内容。知识和研究内容作为资料块输入模型，不当作能改变工具权限的系统指令。

API key 存 OS 凭据库，SQLite 保存引用，日志和请求快照脱敏。移除服务配置前提示仍引用它的项目，不能留下不可见的悬空配置。日志滚动保存有限数量文件（当前为按天滚动、保留最近 7 个），不搭建外部监控平台。

### 11.4 导出

Markdown/TXT 导出当前稿本已提交正文，按正文计划顺序和番外顺序排列，包含小说标题和章节标题；待审提案、流式预览、系统提示词、Token 记录及密钥不进入导出。未通过章节明确标为草稿，不伪装为定稿。

导出在一致的读取快照中完成，不因生成期间翻页查询而混入多个时刻的章节版本。导出是用户获取文本的功能，不承担数据库格式兼容或迁移。

原文没有钉死、由实现定的口径（以下即当前行为）：

- **内容来源**：每章取它 `documents`（role = `chapter_body`）当前已提交修订的 `content`，没有另存副本、没有导出缓存。章节没有正文文档或没有当前修订时，正文为空字符串。
- **草稿判定**：只有当本章 `approved_revision_id` 等于当前修订时才算定稿。已通过但之后又被编辑过的章节（当前修订 ≠ 通过的那一份）与尚未通过的章节一样标为 `（草稿）`，并在标签后紧跟，不依赖颜色或图标。
- **顺序与标题**：正文按 `order_index` 在前，番外按自身 `order_index` 在后（同一顺序用章节 id 保证稳定）。正文标签为 `第{order_index}章 {title}`，番外为 `番外 {n} {title}`（`n` 是番外内的 1 起序号），与工作台目录的写法一致。
- **版式**：Markdown 用 `# 小说标题` 与 `## 章节标签`；TXT 把同样两行写成裸文本，不带任何标记符号。两者都先给一行口径说明：`由 NekoWriter 导出 · YYYY-MM-DD HH:MM:SS UTC · 正文 X / Y 章已通过整章审核`。时间固定用 UTC，便于逐字核对导出结果。没有正文的章节给一行 `（本章尚无正文）`。
- **文件名**：`{消毒后的小说标题}.{md|txt}`。标题会去掉路径分隔符、控制字符与 Windows 保留字符，折叠空白与连续点号并剪掉首尾点号；结果为空时回退 `nekowriter-export`，最长 80 个字符。响应头的 `content-disposition` 同时给出 ASCII 回退名与 RFC 5987 的 `filename*=UTF-8''…`，因此中文标题能原样落盘又不依赖非 ASCII 头字段。
- **边界行为**：稿本不存在或已删除返回 404 `RESOURCE_NOT_FOUND`；`format` 缺失或不是 `md`/`txt` 返回 422 `INVALID_EXPORT_FORMAT`；稿本一个章节都没有时仍导出标题与口径行，并写明「本稿本还没有章节。」，而不是报错。
- **不进入导出的内容**：待审提案、流式预览、系统提示词、Token 用量、凭据、审核意见、Run 与工具调用记录。导出只读正文修订链。

### 11.5 发行构建与跨平台打包

发行目录只有一种形状（可执行文件 + `webui/dist` + `README.txt` + `release-manifest.txt`），平台差异只在可执行文件本身。每个平台在自己的主机（或装有该目标链接器的主机）上执行同一条命令：

```text
# 本机（macOS / Linux）
cargo xtask package --release --protoc "$(command -v protoc)"

# Windows（PowerShell，先由 tools/bootstrap-protoc.ps1 准备 tools/protoc）
cargo xtask package --release

# 从别的平台交叉打包某个目标
cargo xtask package --release --target x86_64-pc-windows-msvc --protoc /usr/bin/protoc
```

打包器在组装之前有两道硬检查，它们都是「别拿别的平台的二进制充数」的一部分：

- 目标平台的标准库必须已安装。`--target` 不是本机三元组时，先看编译器 sysroot 下有没有该目标的 `lib/rustlib/<三元组>/lib`，缺了就**在任何构建之前**以 `rustup target add <三元组>` 结束。后端编译先于前端构建，所以注定失败的打包不会白写一次 `webui/dist`。
- 可执行文件的头部必须与三元组一致，而且**容器格式与架构都要对**：读 PE / Mach-O / ELF 的容器与机器类型，`aarch64-unknown-linux-gnu` 配一个 x86-64 的 ELF 会被拒绝，因为容器同为 ELF、架构却不是目标要的那一个。目前只识别 `x86_64` 与 `aarch64`，其它架构在参数解析后即报错，不做猜测。

本机的验收事实（2026-09-12）：发行包按本机三元组 `aarch64-apple-darwin` 从当前代码重建并逐项核对——清单里 13 个文件的字节数与 SHA-256 全部由独立脚本重算一致、可执行文件头是 Mach-O `0x0100000C`（262,506,224 字节）、`index.html` 引用的 JS/CSS 都在包内。两道硬检查都在**真实产物**上走过：把真实的 x86_64 Mach-O 可执行文件放到打包器要读的位置后，打包以「编译产物是 Mach-O 64 位（x86_64），与目标平台 aarch64-apple-darwin 不符；请确认 --target 与工具链」失败（退出码 1），并且因为这项校验发生在清理旧发行目录之前，上一次已验证的发行目录逐字节未动（清单生成时间与二进制 SHA-256 不变）、失败后 `target/package/` 下没有留下该目标的空壳；未安装标准库的目标（例如 `x86_64-apple-darwin`）在**任何构建之前**就以 `rustup target add <三元组>` 结束，`webui/dist/index.html` 的 mtime 未变。**其它平台仍没有产物**：本机 `rustup target list --installed` 只有 `aarch64-apple-darwin`，没有 Windows 目标的链接器 / C 编译器，也没有 x86_64 的 macOS 目标标准库；因此本轮只产出并验收本机三元组的发行包，其余平台按上面的命令或 CI 产出，形状相同但未经真实验收，这属于环境边界而非代码缺口。发行包在「无推进提交计入协议错误上限」的修复之后又按当前源码重建过一次，独立脚本重算的结论不变：清单 13 个文件、可执行文件 262,589,360 字节、头部 Mach-O `0x0100000C`；二进制 SHA-256 与上一份不同（源码已变），发行目录形状与校验规则未变。完整回路（建项目 → 准备四项 → 细纲 → 正文 → 终稿 → 阅读与查找 → 回收站 → 概览 → 本地服务面板与退出）与两条相关路径都在这一份产物上重跑，证据见 docs/03 §29。

CI 方案（Windows runner 直接产出 x64 产物，不需要交叉工具链）：

```yaml
# .github/workflows/package.yml
name: package
on: workflow_dispatch
jobs:
  windows:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with: { node-version: 22 }
      - uses: pnpm/action-setup@v4
        with: { version: 10 }
      - run: rustup toolchain install nightly --profile minimal
      - run: pnpm --dir webui install --frozen-lockfile
      - run: pwsh tools/bootstrap-protoc.ps1
      - run: cargo xtask package --release
      - uses: actions/upload-artifact@v4
        with:
          name: nekowriter-x86_64-pc-windows-msvc
          path: target/package/x86_64-pc-windows-msvc/nekowriter
```

各平台的实际状态：

- **macOS（aarch64-apple-darwin）**：已用 `--release` 真实产出并验证（见 §15 与 docs/03），包括从仓库之外的路径启动、`webui/dist` 解析、数据与日志落盘、退出与重启；发行清单里的容器与架构写作 `Mach-O 64 位（aarch64）`，即三元组的写法（旧记录里的 `arm64` 是同一种架构的另一种拼法）。
- **Windows x64**：配方与 CI 步骤都在这里，但**未在本机产出**：本机（macOS arm64）没有 Windows 目标的链接器 / C 编译器（`lld-link` 或 `mingw-w64`），也没有 `x86_64-pc-windows-msvc` 标准库，因此没有 PE 产物可验证；`cargo xtask package --target x86_64-pc-windows-msvc` 会在 `rustup target add` + 交叉链接器就位后才成立。这属于环境边界，不是代码缺口：`tools/bootstrap-protoc.ps1`、`.cargo/config.toml` 的 msvc 链接器与 `--target` 参数都已经就位。在本机执行 `cargo xtask package --target x86_64-pc-windows-gnu`（未 `rustup target add`）会立即以 `目标 x86_64-pc-windows-gnu 的标准库未安装：请先运行 rustup target add x86_64-pc-windows-gnu，或改用本机三元组` 结束，且不改写 `webui/dist`、不留下空壳发行目录。
- **其它平台**：同一命令即可，但同样未经真实验收（Linux 目标与 x86_64 的 macOS 目标都只需要 `rustup target add <三元组>`，不改变发行目录形状）。

不在发行包里的东西：源码、`node_modules`、Cargo 缓存、数据库的初始副本（数据目录在首次启动时创建）；`logs/` 与作品库都在数据目录里，根据命令行参数或系统默认位置决定，不写进发行目录。

## 12. Dashboard 口径

### 12.1 Token Usage

数据来自实际模型请求的 `usage_json`，按项目、时间、角色（写作/审核/Embedding）汇总。研究未来接入后也必须归属其父项目；系统连接测试单列为系统操作。阶段上下文折叠由本地程序执行，不产生模型用量，也不删除历史已发生的用量。

字段采用 input_tokens、output_tokens、cached_input_tokens、reasoning_tokens、usage_source、usage_status。cached input 是输入子集，reasoning 若是输出子集就不能再次加到总量；各供应商由适配器统一映射并保留原始用量字段。

总量以已知 input + output 为主，未知请求数另行展示。Embedding 的输入用量独立列示，不能混入“对话 Token 总数”而无说明。MVP 不计算费用或维护价格表。

实现口径（`nekowriter-core::usage`、`nekowriter-store::{dashboard,system_operations}`）：

- 每条已封存请求的 `usage_json` 都是一份 `PersistedModelUsage`。`usage_source` 说明数字来自哪里（`chat_completions_stream`、`embeddings_http` 与 `none`），`usage_status` 说明是否真的拿到了报告（`reported` / `unknown` / `invalid`），供应商原始字段保存在 `raw` 内供事后核对；适配器（`ModelCompletion::persisted_usage`）是这两者的唯一写入点。
- 没有报告的请求写缺失字段而不是 0，并带上类型化原因 `provider_silent` / `request_failed` / `request_cancelled` / `pending` / `invalid_report`。请求在区间内没有任何封存记录时由它的终态推出原因，因此「用量未知」永远能说明自己，而不是一个无法追问的数字；分类计数由 `unknownUsage` 给出，顺序稳定。
- 连接测试这类不属于任何稿本的请求写入 `system_model_operations`（schema 25），带上与对话请求相同的用量口径与失败原因，在概览里单列为 `systemOperations`（含最近 5 条明细）：既不混进对话 Token，也不会凭空消失。
- Embedding 的每一次向量 HTTP 请求同样写入 `system_model_operations`，操作类型为 `embedding_index`（索引任务，长条目拆成多批时每批一行）、`embedding_query`（语义检索的查询向量）或 `embedding_connection_test`（显式连接测试与保存设置前的验证调用）；`usage_source` 为 `embeddings_http`，供应商报告的 `prompt_tokens` 映射为 `input_tokens`，没有报告时记为 `unknown / provider_silent` 而不是 0。概览按这些操作类型聚合出 `tokens.embedding`（请求数、输入用量、未知计数与原因、按操作类型拆分），并从 `systemOperations` 中排除它们：同一批 token 不会既算系统操作又算 Embedding。嵌入请求没有输出侧，因此 Embedding 只统计输入，并且永远不进入对话 Token 总数。
- 供应商的用量形状不参与请求成败：`usage` 按宽松解析，供应商可以把它写成对象、字符串形式的数字、裸数字、`null`、空数组或干脆不写；能读出计数就是 `reported`（`prompt_tokens` 优先、缺失时用 `total_tokens`），报告在但读不出计数就是 `unknown / invalid_report`，字段缺失或为 `null` 就是 `unknown / provider_silent`。用量只是一个统计字段，因此它永远不能把向量一起弄丢，也不能因为解析不出而拒绝一次本来可用的 Embedding 服务。
- 保存 Embedding 配置前的真实验证调用排在准入检查之后：同一 `command_id` 已有可重放结果时直接返回上次结果（不写凭据、不再调用供应商），修订已过期或字段无效时直接报错，两种情况下都不花掉一次真实请求；只有确实会写入的保存才调用供应商，并把这次调用作为 `embedding_connection_test` 落库。
- 三个镜头都要覆盖 Embedding：角色拆分（`byRole`）里 Embedding 与写作、审核并列，按日序列里它有自己的 `tokens.embedding.daily`（与 `tokens.daily` 同一套本地日口径和平行的一日一格）。它们是同一批向量用量的不同镜头，不是新的计数：`embedding` 角色的输入与请求数逐字等于 `tokens.embedding`，而对话总量与对话日序列仍然只来自 `model_requests`。
- Embedding 用量不属于任何项目（知识 Space 是全局的），因此项目过滤下的概览仍返回同一份 `tokens.embedding`；它在独立字段里，不会混进该项目的对话用量，并且自带 `tokens.embedding.scope`（当前只有 `global`）把这份归属写进响应本身，而不是留给调用方猜“这个数字随筛选变化吗”。
- 系统模型调用（模型连接测试与向量请求）的落库只有一条应用层写入路径：状态判定、用量口径与失败原因形状只写一次，对话与 Embedding 两种适配器只负责把自己类型的结果翻译成统一输入（`nekowriter-app::operations`）。

### 12.2 Running Jobs

Dashboard 的进行中任务包括排队、模型运行、等待用户、等待审核和已暂停，分别标识；已中断与失败位于需处理列表。主计数按顶层 Run 统计，审核子 Run 展开显示，避免一个作品因父子任务重复计数。

执行槽只有一个也可以有多个等待用户的项目，界面不得把“无模型正在请求”显示成“没有进行中的写作”。

### 12.3 Word count increment

字数采用 Rust Unicode 字符遍历，排除 `char::is_whitespace()`，标点及可见符号计入；这属于中文写作产品口径，不是语言学词数。章节标题、细纲、设定和模型思考不计正文总字数。

每次正文修订记录 `delta = new_count - old_count`，正负均可；拒绝编辑、重复处理和复制稿本为 0。删除/恢复对象属于库存变动，不伪装为创作事件；图表展示写作净增，另显示当前可见稿本总字数。

跨稿本总量明确表示所有可见稿本的内容总量，复制会增加内容库存但不增加“今天创作”。Dashboard 默认只汇总各项目当前选中稿本，用户切到全部稿本时提示可能包含相同内容。

统计范围使用用户选定时区转换成 UTC 左闭右开区间，日期分桶在查询层统一完成，不能前后端分别猜测“今天”。

实现上这些指标全部是回算：`GET /api/dashboard` 每次读取已封存请求、`runs` 行、工作流当前执行轴与 `document_revisions` 修订链，没有任何持久化的累计计数器，因此统计不会与记录漂移，同一批数据重复查询结果一致。字数统计只取它真正用的六个列（`id` / `document_id` / `revision_no` / `word_count` / `created_by` / `created_at`），并且只取可见正文文档的修订——修订正文与结构化载荷不参与统计，把它们读回来会把整部作品的文本搬进进程内存（实测 6,000 条修订即可让常驻内存多出上百 MB），而已删除文档与非正文文档的修订读进来又会被丢掉。不同供应商的用量字段由适配器统一映射到 `input_tokens` / `output_tokens` / `cached_input_tokens` / `reasoning_tokens`；嵌入调用虽然已经落库，但按 §12.1 的前一条只进 `tokens.embedding`，不计入这份对话用量。

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
- 已实现的快捷键集合限定为四条，且可由界面发现（头部按钮与 `?` 打开说明）：`Ctrl/Cmd+Enter` 提交当前表单或全场唯一可用的阶段主操作、`Ctrl/Cmd+K` 聚焦整稿查找框、`Esc` 关闭最上层可安全取消的层、`?` 开关快捷键说明。没有单键绑定，也没有给破坏性操作绑定快捷键；输入框内的复制/粘贴/撤销/全选保持浏览器原生行为。
- 连续屏幕阅读播报只报状态变化与完整段落摘要，不逐 token 打断用户。
- 模型失败、索引失败和零引擎能力显示实际原因，不能以无限 skeleton 隐藏不可完成状态。
- 「本地服务」面板显示界面地址与实际数据目录，退出服务需二次确认并说明退出后保留什么；服务退出后界面区分「已退出」与「正在重连」，并给出重新启动方法与刷新提示。

## 14. 实施任务清单

任务按阶段交付；每项的完成证据记录在实现 PR/变更说明中。M0 已在本机完成联合编译、测试和 HTTP 冒烟验证。M1 已有可运行的本地闭环、自动准备/细纲请求，以及自动接受的正文 `append_content` / `replace_content` 循环；macOS aarch64 上的 InferX 真实模型全流程证据见 §15.6。

### M0 本地骨架

- [x] 建立四 crate workspace、工具链与锁文件；Windows 上验证 Axum/SeaORM/LanceDB 联合编译。
- [x] 实现数据目录、进程锁、当前 schema 初始化、数据库 pragma 校验。
- [x] 接通同源静态页面、健康检查、项目/稿本创建与持久查询。
- [x] 用临时数据执行 LanceDB 写入和查询烟测，记录实际原生依赖。

### M1 最小写作闭环

- [x] 实现当前阶段所需数据表、Core 状态转换与命令回执。
- [x] 接通 Chat Completions 兼容模型协议、能力校验、内建提示词和结构化输出；macOS aarch64 上已用 InferX `deepseek-v4.1-flash` 完成真实连接烟测，并以 `reasoning_effort=low` 跑通一章从准备到完本的全流程（§15.5–§15.6）。
- [x] 完成四项准备、人工审核、单章指导、细纲审核、自动接受正文和整章审核的本地事务与界面路径。
- [x] 实现阶段全部产物通过后的纯文本折叠、审核/推进原子提交、按折叠来源构建下一请求（准备、细纲与正文自动 `append_content` / `replace_content` 已接通；`ask_user`、逐次审核和意见门控已在 M2 补齐）。
- [x] 接通 SSE 快照同步、正文显示、任务状态和基础用量记录。

### M2 交互正确性

- [x] 实现编辑提案、基础修订校验、逐次审核和差异 UI；候选正文只存于 `tool_calls.proposal_json`，接受/拒绝均以命令回执和同一 Run 的工具结果原子落库。
- [x] 实现暂停控制、未封存意见批次的统一追加/投递、直接终态提交完成守卫和模式切换边界；等待正式问答时的自由意见不会替代答案。模型在仍有未投递意见时反复直接提交不再是无限重排：每一次无推进的纠正都计入连续协议错误，第三次即按工程执行限制把 Run 收束为 `stopped`，未投递意见原样保留并可由「显式继续」重新开始一轮请求额度（真实进程实测：修复前同样的桩会一直重排到请求额度上限，修复后最多 4 次请求即停止，见 docs/03 §28）。
- [x] 实现 `ask_user`、重复命令回执、原生 EventSource 游标重连、进程中断恢复与每次 100 次请求额度续授；完整但无效的模型响应按连续三次协议错误上限纠正或停止。
- [x] 实现用户显式停止：同事务作废 prepared/dispatched attempt、记录失效请求 ID，并中止本地在途 HTTP 任务；迟到回调不能推进工作流，待答/待审/暂停边界在显式继续后恢复。
- [x] 实现引用与附件性反馈：工作台可把选中的正文/候选文字固定成引用，并随逐次编辑决定或自由意见一起提交；正式引用锚定不可变修订，候选引用锚定工具调用、候选哈希与候选区间，写入前仍由服务端逐字复核；浏览器 UTF-16 选区偏移在提交前换算为 Unicode scalar 偏移。逐次编辑的主操作按「有无文字或引用」动态显示接受或拒绝。引用随正文变化显示「来自旧修订」，并通过 `GET /api/documents/{id}/revisions/{revision}` 只读回看原修订。委托审核会在委托事务内校验并固化当时选中的引用，审核子 Run 只读到引文文本，不接触文档、修订或工具调用 ID；准备、细纲与整章终稿的人工审核决定同样可以携带引用，并与委托共用同一套逐字校验，引用与决定在同一事务内写入同一条 Review。
- [ ] 通过 §15 中全部编辑、反馈、审核竞态用例。本轮在真实进程 + 桩模型上重跑了 A/E/R/F 组：17 行通过（A01–A03、E01–E05、E07、R01–R04、F01–F04、F06、F08–F10），两行记录为真实接口不可构造（E06、R05），一行因桩无法产出合法工具参数而未验到（F12）；其余各组仍未跑，逐行原因见 §15.3。该轮之后发现的四个实现缺口已在 §15.4 修复并加入回归，但完整矩阵尚未重跑。

### M3 完整创作流程

- [x] 完成结构化章节计划、多章切换、多阶段产物纯文本上下文、稿本复制、完本与番外。多章计划录入、自动正文、稿本新建/复制/切换、完本与番外、已完成阶段 fold，以及章节目录与任意章节的只读浏览/跳转均已接通；长篇虚拟化与滚动锚点仍属于 M5。
- [x] 完成各类型审核委托与过期结果处理：准备产物、细纲、逐次正文编辑和终稿均有独立提示词、目标/依赖重验、取消、失败/重启恢复与成功回调重放保护。
- [x] 完成系统/项目设置、提示词覆盖、变量说明和请求快照。系统写作模型设置与 Windows 凭据引用、内建提示词的系统级/项目级覆盖、变量元数据说明与保存前预览均已接通；项目级模型引用、思考等级与辅助工具预算具备持久化、独立乐观并发修订、API 与工作台界面；模型引用连同它自己的能力资料一起保存，因此项目覆盖不会沿用系统模型的能力声明。调度器在每次取得许可后重新解析「项目覆盖 → 系统默认」的有效模型与思考等级，把系统/项目设置修订、提示词来源、有效模型、有效能力资料与思考等级写入请求快照，并在有效模型未声明支持所需能力时于发请求前报错；命令受理前的同一判定与连接测试都使用同一份解析结果。辅助工具预算的账本扣减依赖辅助工具本身，随 M4 的知识检索一并实现。

### M4 知识与管理

- [x] 实现 Space、知识 CRUD、Embedding 接入、索引任务与语义检索，补齐过期候选过滤后的持续补取及单次计费。Space 与条目 CRUD、软删除、非空 Space 拒绝删除、Embedding 配置与连接测试、按 profile 隔离的 LanceDB 索引任务、按 Unicode 边界的目标 600 / 硬上限 1000 / 重叠 80 切分均已接通；检索先向量近邻再回查 SQLite 的存活与当前修订，剔除失效块后在同一查询内翻倍候选继续补取，全程复用一次查询向量。辅助工具预算账本同时落地：`quota_scopes`（version + stage_key 唯一）与 `quota_charges`（scope + tool_call_id 唯一）随 schema 23 建立，准备阶段与每章细纲的 Run 在创建时绑定作用域；有效上限每次执行前从项目最新设置读取，下调不清零已用数，同一工具调用重放不重复扣除。
- [x] 实现研究/积累接口、能力注册表及零生产实现的明确返回行为。`nekowriter-agent/src/engine.rs` 定义联网研究与知识积累的接口合同（请求、结果、来源与限制说明、类型化失败）及编译期注册表 `RESEARCH_ENGINES` / `ACCUMULATION_ENGINES`，生产注册项严格为 0；`GET /api/capabilities` 返回空数组与原因；`POST /api/knowledge/accumulate` 先校验线索与目标 Space，再返回 409 `ENGINE_NOT_AVAILABLE`，不创建运行任务、不写条目、不扣预算。积累的成功写入路径按 §7.4 实现为一条普通写入命令（重放检查 → 引擎 → 逐条验证 → 同一事务插入条目、建立索引任务与写入回执），引擎注册表由应用状态持有、能力查询与积累入口读同一份；真实端口上的 HTTP 用例覆盖了正例、引擎失败（502，零写入、无回执）、重放（不再次调用引擎）与幂等冲突，生产进程的探针覆盖了 0 引擎时的行为。准备与细纲 Agent 在 Embedding 已配置、索引有可检索条目且仍有余额时才宣告 `query_knowledge`，工具描述带上剩余次数；执行顺序为重放检查 → 参数校验 → 可用性校验 → 扣一次预算 → 检索 → 落库推进，参数无效与预算耗尽都不扣费，进入真实检索后失败仍计 1 次。
- [x] 实现模板模块和可回算的 Dashboard 统计。受限模板语言（一次性 `{{variable_name}}` 替换、`\{{...}}` 字面量转义、未知变量/不完整括号/缺失必需占位符拒绝）在 `nekowriter-core::template` 中只实现一次，`agent::prompts` 的内建覆盖与用户模板共用它；用户模板以 `prompt_templates` 表（schema 24）保存标题、正文与自定义变量声明，支持 `command_id` 幂等与 `expected_revision` 乐观并发、软删除，保存与修改都在 Store 边界重新校验；`POST /api/prompts/preview` 用同一条路由的 `builtin` / `template` 两个变体服务内建草稿与用户模板，未提供的变量回落到声明的示例值；工作台提供模板管理与章节指导/意见草稿的插入入口。`GET /api/dashboard` 不保存任何累计值：Token 由已封存请求的 `usage_json` 按写作/审核角色与本地日汇总（`cached_input_tokens`、`reasoning_tokens` 作为子集，未报告用量的请求单列 unknown），任务由 `runs` 与工作流当前执行轴分区（审核子 Run 展开、不进入主计数），字数由 `document_revisions` 修订链回算（复制稿本计 0），时区偏移换算成 UTC 左闭右开区间，日期分桶在查询层完成。每一条用量都带 `usage_source` / `usage_status` 与类型化未知原因，连接测试作为系统操作单独落库并在概览单列；Embedding 的向量请求（索引、检索、连接测试）同样落库并带上来源，概览按操作类型把输入用量单独列示、并从系统操作中排除，因此同一批 token 不会重复计数，也不会混进对话总量；三个镜头都覆盖 Embedding：`byRole` 里有独立的 `embedding` 角色，`tokens.embedding.daily` 与对话日序列平行，`tokens.embedding.scope` 明写它是全局数字；工作台概览订阅 `run.changed` / `document.committed` / `review.changed` / `feedback.changed` / `system.model_operation` 等持久事件自动重算，不需要用户手动刷新。

### M5 本地交付

- [x] 完成长篇章节虚拟化、滚动锚点、主题、键盘及中文输入体验。长篇阅读已接通：阅读区在连续滚动区里按章虚拟化（只有视口附近的章节被挂载，正文按窗口加载），目录也是以章节 id 作 key 的虚拟列表；定位先按虚拟下标滚动、再在正文加载与高度测得后校正到标题对齐顶部，不依赖未挂载 DOM 的 `scrollIntoView()`；阅读位置以「章 + 块内偏移」记在浏览器本地存储，切换视图与重新加载页面后回到原处，字号变化重新测量并把当前章对齐到顶部；整稿查找由服务端读遍全部已提交正文，返回与引用同一单位的偏移并支持高亮；渲染开销与总章数脱钩已由真实进程实测（401 章与 51 章作品的挂载块数与打开耗时基本相同）。
  主题与键盘/中文输入体验已接通：单键 `d` 切换主题的全局监听被删除，主题改为工作台头部的三段选择器（浅色/深色/跟随系统，仍由 `ThemeProvider` 存在浏览器本地）。快捷键在 `webui/src/lib/shortcuts.tsx` 一处实现：`Ctrl/Cmd+Enter` 提交当前表单命中（光标在输入框里时提交该表单的 `type=submit` 主操作，不在输入框里时只触发全场唯一可用的阶段主操作）、`Ctrl/Cmd+K` 聚焦整稿查找框、`Esc` 关闭最上层可安全取消的层（查找结果、快捷键说明、退出确认）、`?` 开关快捷键说明；说明由头部「快捷键」按钮与 `?` 打开（见 §13）。组合输入只认「可编辑目标上的 compositionstart」，并同时用 `isComposing` 与 `keyCode === 229` 判定，因此一个没有配对的 compositionstart（输入法在页面获得焦点时可能发出）不会把整个键盘永久关掉；窗口失焦也会清除该状态。组合期间那次 Enter 属于「确认候选词」，在 document 捕获阶段拦下随后的 `submit`，且必须 `preventDefault` 与 `stopPropagation` 同时生效——表单由 React 接管，只 `preventDefault` 仍会执行 `onSubmit` 并发出请求。破坏性操作（审核通过/退回、停止执行、退出服务）不绑定任何快捷键。焦点在各处被显式管理：说明面板打开时焦点进入其关闭按钮、关闭后回到打开它的元素；退出确认出现时焦点落到「取消」而不是破坏性按钮；快捷键动作让触发它的按钮消失后，焦点回落到工作台主区域而不是 `<body>`。
- [ ] 完成软删除/恢复、正文导出、退出流程及脱离源码的 Windows 发行包。正文导出已接通：`GET /api/versions/{id}/export?format=md|txt` 每次在同一个读取事务里从已提交修订链现读（无副本、无缓存），按正文计划顺序与番外顺序排列，只把「当前修订就是通过的那一份」算作定稿，其余标为 `（草稿）`，文件名经消毒并带 RFC 5987 下载头，工作台「稿本」面板提供 Markdown 与纯文本两个导出入口；真实进程探针从界面点击导出，产物与 `document_revisions` 逐字一致，并行覆盖了 422 非法格式、404 不存在与无章节的空稿本三个边界。退出流程与脱离源码的发行目录已接通：`nekowriter-app::lifecycle` 统一 Ctrl+C / SIGTERM / `POST /api/system/shutdown` 三条触发（详见 §11.2），启动时把日志落到数据目录的 `logs/` 并打开默认浏览器，`cargo xtask package` 组装「可执行文件 + `webui/dist` + `README.txt`」的发行目录；真实进程探针把发行目录拷到仓库之外的路径启动，界面与资源可用、数据与日志只落在指定数据目录，走真实 API 写出一章 72 字正文后从界面退出（0.2 秒内退出码 0、WAL 已回写、在途模型请求被取消、SSE 关闭）、重启后正文与项目仍在且在执行中的 Run 为 `interrupted`（请求 `error_json.kind = process_shutdown`、事件 `reason = process_shutdown`），带一条打开的 SSE 连接走 SIGTERM 同样在 0.1 秒内退出码 0。回收站恢复已接通：`nekowriter-store::recycle` 把列出、恢复与彻底删除实现为唯一一处，恢复只清回 `deleted_at`（章节行、文档槽与修订链不动），因此恢复后的正文、`revision_id` 与 `order_index` 与删除前一致；恢复不从工作流搬回写作光标，父对象仍在回收站里返回 409、对象已被彻底删除返回 404。彻底删除只对模板/知识对象开放，稿本与正文返回 `RECYCLE_NOT_PURGEABLE`。工作台新增「回收站」面板（类别、标题、删除时间、原属稿本与阻塞原因）并订阅 `recycle.changed` 自动刷新；「番外」面板与稿本列表提供带二次确认的删除入口。真实进程探针用真实正文跑完整回路：删除番外后在回收站看到它（18 字正文、删除时间、原属稿本），从界面点击恢复后正文与修订 id 与删除前逐字一致、排回原位置（`order_index` 不变），并从界面的番外列表再次删除。项目级回收站也已接通：`DELETE /api/projects/{id}` 只标记项目本身，所有读取路径逐层检查祖先，所以项目在回收站里时它的稿本、章节与正文都不可见；恢复项目只清项目自己的 `deleted_at`，它的稿本原样回来，而删除项目之前单独删掉的稿本仍留在回收站里（不会因为恢复项目而被误恢复）。项目下仍有开放审核或活跃写作任务时返回 `RESOURCE_BUSY`，过期修订返回 `REVISION_CONFLICT`；项目与正文一样构成审计链，不做物理删除。真实进程探针跑完整回路：建两个项目 → 在待删项目里再建一个稿本并单独删掉它 → 删项目（列表只剩一个、被删项目 404、回收站显示「2 份稿本 · 0 章」）→ 其下稿本恢复被拒为 409 `RECYCLE_ANCESTOR_DELETED`、项目物理删除被拒为 `RECYCLE_NOT_PURGEABLE` → 恢复项目（两个项目都在、单独删掉的稿本仍在回收站）并从真实界面再次删除与恢复。Windows x64 发行包本身仍待做。
- [x] 完成一章真实模型的完整创作路径验收。macOS aarch64 的 InferX `reasoning_effort=low` 连接测试与准备 → 细纲 → 正文工具 → 整章审核 → 完本记录在 §15.5–§15.6；该证据未用桩模型替代。
- [ ] 完成多章、稿本复制和番外的真实模型路径验收；这些超出本轮一章 macOS aarch64 验收范围。

2026-09-13 当前状态说明：M5 的软删除/恢复、正文导出、退出流程和脱离源码的 macOS `aarch64-darwin` 发行包均已完成并有实测记录；上述未完成项只指 Windows x64 发行产物本身。M2 的完整竞态矩阵和真实浏览器路径仍是验收工作，不是未实现的业务路径。

## 15. 测试与验收矩阵

### 15.1 测试层次

Core 单元测试关注业务不变量与字符串边界；SQLite 临时文件集成测试关注跨表事务、去重和检查点；Agent 测试用可控假服务模拟流、工具与错误；Playwright 验证真实界面的关键写作路径。

假模型只用于自动化测试。M1、M3 和最终交付必须完成真实模型烟测，记录模型 ID、服务地址类型、锁定版本、已验证能力和用量；没有可用凭据时应明确标记真实模型验收未完成，不能以假模型替代。

截至 2026-09-12 的记录显示，本机当时没有可用的真实模型凭据：环境变量里没有模型服务 key，OS 凭据库里只有回环桩凭据。本轮（2026-09-13）在用户提供 InferX 凭据后完成了 macOS aarch64 的真实连接测试，历史桩记录不作真实模型证据。

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

### 15.3 矩阵重跑记录（2026-09-13，当时状态）

环境：当前源码的 `target/debug/nekowriter` 真实进程（端口 47895、数据目录 `/tmp/nw-mx/`）+ 本地桩模型（`/tmp/nw-branch-stub.py`，端口 47993，**不是真实模型**）；判定与证据落在 `/tmp/nw-mx/matrix-results.json`，证据取自 API 快照与 SQLite 行。

| 行 | 结果 | 关键证据 |
| --- | --- | --- |
| A01 | 通过 | 只提交三项：`preparing_generating` 起三次协议纠正后 `stopped`/`protocol_error_limit`，无审核批次、无章节计划 |
| A02 | 通过 | 四项齐全但资料未通过：`/chapters/next` 409 `INVALID_TRANSITION`，章节数 0 |
| A03 | 通过 | 拒绝资料 → `preparing_revising`；提交新资料 → 背景/人物/粗纲三条审核变为 `stale`，资料为 `open` |
| E01 | 通过 | 重复 `commandId` 的同载荷 append 只产生 1 条修订；换载荷 409 `IDEMPOTENCY_CONFLICT` |
| E02 | 通过 | 无匹配 / 两处匹配 / 重叠匹配均 422（`EDIT_NO_MATCH`、`EDIT_AMBIGUOUS_MATCH`），原文不变 |
| E03 | 通过 | CRLF 归一为 LF；整个 ZWJ 字素簇可替换；半簇 422 `EDIT_NO_MATCH` 且正文逐字不变 |
| E04 | 通过 | `approve_required` 下模型编辑只形成待审提案，正式正文 0 修订 |
| E05 | 通过 | 空意见接受提案、非空意见拒绝且正文不变、空白自由意见 422 `EMPTY_FEEDBACK` |
| E06 | 真实接口不可构造 | 待审提案期间用户正文编辑被 409 拒绝，自动接受不会让该 Run 重新请求，`ProposalBaseRevisionConflict` 只有 store 内部路径可达 |
| E07 | 通过 | 工具 JSON 中途断流：0 修订、请求 `failed`（provider_error） |
| E08 | 通过（本轮修复） | 两个工具调用 → 助手消息保留两个调用、每个 ID 一条配对 `PARALLEL_TOOL_CALLS_NOT_ALLOWED` 工具结果 + 串行纠正；修复前同一场景是 run `failed`、零配对结果 |
| R01–R04 | 通过 | 缺字段/字符串 bool/NaN/越界置信度只记协议错误不应用决定；`confidence=0.2` 通过并保留；`passed=false, issues=[]` 保留系统说明；取消委托后迟到结果不改人工决定 |
| R05 | 真实接口不可构造 | 背景审核已被更早的行占用，同一稿本无法再造一个可变修订的委托 |
| F01 | 通过 | 暂停先登记 `pauseRequested`，到边界才 `paused`，结构化提交进入 `heldSubmission` |
| F02 | 通过 | 暂停后提交意见清除暂停，意见 `bound → included` 且确实出现在后续请求快照里 |
| F03 | 通过 | 未挂起且最近结果未封存时提交意见：绑定该结果并取消暂停请求 |
| F04 | 通过 | 最新结果已随请求发出后提交意见：历史快照逐字节不变，意见等待新结果后才投递 |
| F06 | 通过 | 同一窗口三条意见一次投递，`included_request_id` 相同且三条齐全 |
| F08 | 通过 | 有未投递意见时模型的直接提交不被当作已交付：该请求快照不含意见，意见随下一请求投递 |
| F09 | 通过 | 暂停落在提交边界且提交可应用时，`resume` 200 并进入 `writing_chapter_reviewing`，正文不变 |
| F10 | 通过 | 有待审提案时切到自动接受：既有提案仍待决定，后续编辑用新模式直接落库 |
| F12 | 未验到 | 桩按合同生成的 `ask_user` 参数不符合问题模式（`TOOL_ARGUMENTS_INVALID`），Run 无法推进到「等待回答」 |
| A04–A06、S01–S04 | 未验到 | A03 之后的准备流程被下面的第 2 条缺口阻断（Run `submission_commit` 失败），组脚本在此中止 |
| F05、F07、F11、F13–F15 | 未验到 | 本轮驱动脚本未覆盖这些行 |
| Q01–Q06、K01–K09、C01–C10、D01–D09、U01–U04、P01–P04 | 未验到 | 本轮未跑（仅有 §15.3 末尾的浏览器 SSE 片段覆盖 D02/D03/D07/D08 的一部分） |

前端重连在真实浏览器（同一真实进程的预览页）里单独验证：

- D02：快照水位 1960 之后开流，真实写入产生的事件 1961 `settings.changed`、1962 `document.committed` 全部到达，序号严格递增且无重复。
- D07：以 `Last-Event-ID: 1962` 重连（不带 URL 游标）只收到 1963、1964，不重放 1962，也没有重启 Run。
- D08：`?after=abc` 返回 400 `INVALID_EVENT_CURSOR`；`?after=<水位+100000>` 返回 200 `stream.reset_required`（`reason=cursor_ahead`，带 watermark）。
- D03：同一连接内序号严格递增且不重复，重连不从已确认的游标重放。
- D09：无法验证——细则 §675/§683 要求的临时事件 `preview.delta`（携带 request/tool/offset、不带 SSE id）在实现里不存在，`crates/` 与 `webui/src/` 都没有任何预览事件类型。

本轮新修（有真实进程证据）：

1. **并行工具调用不再是传输失败**。`nekowriter-agent::provider` 删除了 `ParallelToolCalls` 这个错误，改为把一次响应里的每个调用完整还原；「一轮只允许一个」只剩阶段守卫 `AgentRunner::classify_model_completion` 一处判定。`nekowriter-app::scheduler` 对 `ParallelToolCalls` 走协议纠正，并为整批每个调用 ID 写入配对的 `PARALLEL_TOOL_CALLS_NOT_ALLOWED` 工具结果和串行纠正；`ModelProtocolCorrection` 增加 `tool_result_code` 承载这个机器可读原因码。
2. **协议纠正消息带上具体原因**。此前纠正只说「不符合当前阶段的结构化协议」，模型与用户都看不到缺的是哪一项；现在追加解析器给出的原因（例如「准备提交必须恰好包含背景、资料、人物和粗纲四项产物」）。
3. **core 单测对齐「在途编辑不把执行轴改成空闲」**。该语义是上一轮为修「在途编辑把工作流留成没有继续入口的空闲态」引入的，单测未同步，导致 190 用例回归 1 例；现在同时断言「回合进行中编辑保持 Queued」与「无回合时编辑回到 Idle」。

本轮发现但**尚未修**（都有真实进程证据，留给下一轮决定）：

1. **反复提交参数非法的同一工具调用没有工程上限**：桩在细纲阶段始终返回参数不符 schema 的 `ask_user`，Run 连续发出 76 次请求、`consecutive_protocol_errors` 仍为 0，最后以 `failed` 收场。工具结果错误与协议纠正没有共用同一个上限，模型只要重复同一个非法调用就能把请求额度耗光。
2. **准备修订轮的提交合同与 store 规则互相矛盾**：系统消息里的提交合同写「artifacts 必须恰好包含 background、research、characters、outline 四项」，而 store 在修订轮只接受「需要修订的项」（实测拒绝信息为「资料收集 当前不需要修订，不能覆盖已通过或待审修订」）。任何照合同提交的模型都会在修订轮失败并把工作流留在 `execution = failed`，这正是 A04–A06/S01–S04 未验到的原因。
3. **暂停在无法应用的提交边界时 `resume` 返回 422**：`POST /api/runs/{id}/resume` 返回 `VALIDATION_ERROR`（「正文编辑参数无效或无法应用到当前内容」，内层为「章节正文不能为空」），Run 永久停在 `paused`，只能被显式停止，与 F09「可继续、无永久等待」的要求相冲突。本轮把 F09 改成「先落一段真实正文再提交」后通过，说明缺口只在提交本身不可应用时的处理。
4. **`preview.delta` 未实现**（见上），因此 D09 无法验证。

### 15.4 2026-09-13 后续收口

§15.3 记录的四个缺口已经实现并加入自动化回归；它保留发现时的真实进程证据，以下是当前状态。

1. `ask_user`、知识工具和正文工具返回错误时，都会推进同一个 `consecutive_protocol_errors`。第三次连续错误把 Run 和工作流收束为 `stopped` / `protocol_error_limit`；成功工具结果清零。store 与 scheduler 用例分别覆盖非法提问和非法 `append_content`。
2. 准备修订的提交合同从当前 `required_artifacts` 生成，只要求本轮需要重修的项。runner 用例覆盖只修资料和只修粗纲，且拒绝混入不需重修的产物。
3. 暂停期间的直接正文提交若没有可应用的正文，恢复时会保留模型原文、追加纠正消息、清除 held submission，并从 `paused` 回到可继续发送的 `queued`；不会再把 Run 永久留在暂停态。scheduler 用例覆盖该回路。
4. D09 已实现：模型字符流经独立广播送入同一 SSE 连接，`preview.delta` 没有持久事件 id；前端按请求、工具槽位和 scalar 偏移拼接，并只展示未验证的临时内容。路由集成用例消费真实 SSE 字节流，断言事件名、载荷与缺失的 `id:` 行。

截至本轮，`cargo test --workspace` 共 197 个用例通过（agent 34、app 47、core 38、store 74、xtask 4）。这覆盖上述回归，但不替代 §15 中尚未运行的完整竞态矩阵和真实浏览器路径。

### 15.5 2026-09-13 macOS aarch64 环境验收

本轮按验收范围只检查 macOS `aarch64-apple-darwin`，未把 Windows 或其它目标计入结论。主机工具链为 Rust nightly `1.100.0`，已安装目标只有 `aarch64-apple-darwin`。

真实模型端点使用 `https://model.inferx.net/endpoints/v1`，模型 ID 为 `deepseek-v4.1-flash`。先用用户提供的 Chat Completions 请求直接得到有效回复；随后启动当前源码的 NekoWriter 进程，在本地 `/tmp/nw-real-accept` 数据目录保存该服务设置，并调用 `POST /api/settings/test-model`。连接测试返回 `contentReceived=true`、`toolCallCount=0`，供应商用量为输入 54、输出 92、推理 87（总计 146），随后以 `POST /api/system/shutdown` 正常退出。API key 只写入 macOS 钥匙串引用，不写入仓库、数据库或本节文档。

因此，M1/M3 的“真实模型端点可用”这一环境前置条件已经在本机满足；本节记录连接测试，完整创作回路见 §15.6。随后执行 `cargo xtask package --release --protoc /opt/homebrew/bin/protoc` 成功生成当前 aarch64 包：13 个文件、Mach-O arm64 可执行文件 262,596,528 字节；把包放在源码树外启动后，首页与 `/api/health` 均成功（schema 25），再通过 `POST /api/system/shutdown` 正常退出。Windows x64 不属于本轮验收范围。

### 15.6 2026-09-13 macOS aarch64 InferX reasoning_effort 全流程 E2E

本轮继续只验收 macOS `aarch64-apple-darwin`，使用真实 InferX Chat Completions 端点和 `deepseek-v4.1-flash`。先用短请求显式发送 `reasoning_effort: "low"`，服务返回 HTTP 200、正文 `READY`，并报告输入 35、输出 15、推理 11、总计 50。当前源码进程随后通过 `POST /api/settings/test-model` 发出同样的能力探针，返回 `thinkingProbed=true`、`contentReceived=true`、`toolCallCount=0`，用量为输入 54、输出 54、推理 49、总计 108；系统设置保存为 `thinking=true`，项目设置保存为 `thinkingLevel=low`。凭据只由 macOS 钥匙串引用提供，本节不记录 API key。

真实进程使用 `/tmp/nw-real-e2e-clean` 数据目录完成一章全流程：项目与稿本创建、目标保存、准备阶段四项产物生成并逐项审核通过、章节细纲生成并审核通过、正文工具编辑、整章审核通过和完本确认。准备阶段因端点容量限制先收到两次 429，显式继续后成功；正文首轮因 InferX 同时发出 reasoning 文本与工具调用而被记录为失败，重建当前二进制后继续同一 Run，四次 `append_content` 工具调用依次把正文推进到 4,683 个字符、4 个正文修订，随后结构化终态提交使工作流进入整章审核并通过，版本最终为 `completed`。一次后续正文请求还遇到流空闲超时，保持 `unknown` 账本并显式继续，未自动重发不确定请求。

该 Run 的写作请求共 11 次：已报告用量的请求合计输入 48,183、输出 10,111、推理 1,401、总计 58,294；4 次请求没有可用用量（两次 429、一次混合流协议错误、一次流空闲超时），均保留在 Dashboard 的未知用量栏。成功请求的 `context_snapshot` 全部记录 `effectiveThinkingLevel=low`，并报告非零 reasoning tokens。Dashboard 最终为排队/运行/待审核/需处理均为 0，正文净增量 4,464，当前总字数 4,464。

本次真实流暴露并收口两处兼容问题：正文阶段带工具时不再附加 `response_format=json_object`，避免 InferX 直接返回空终态；Provider 在同一响应包含工具调用时丢弃供应商附带的辅助文本，保留并执行工具参数。后者有 Agent 回归测试覆盖。Markdown 与 TXT 导出均返回 200，正文窗口返回 1 个 `draft=false` 章节，整稿搜索“林舟”返回 17 处；所有证据来自真实端点和真实 NekoWriter 进程，未用桩模型替代。

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

截至 2026-09-13，自动化 Rust 工作区测试共 197 个用例通过（agent 34、app 47、core 38、store 74、xtask 4），格式检查通过；§15.2 的 A/E/R/F 组已在真实进程 + 桩模型上逐行重跑，其中 17 行通过、两行记为真实接口不可构造、一行因桩限制未验到，详情与逐行证据见 §15.3。§15.4 已补齐其后暴露的四个实现缺口，§15.6 又在真实 InferX 端点上完成一章从准备到完本的 `reasoning_effort=low` 全流程。完整 §15 竞态矩阵（Q/K/C/D/U/P 各组）和真实浏览器路径仍未全部完成，因此相应里程碑不能仅凭自动化测试标为完成。

字数统计的列投影由真实进程探针覆盖：同一批 6,010 条修订（含 200 条隐藏文档的修订）在新旧两个 release 二进制上各跑四个测量回合，7 天、项目过滤与 90 天三份概览 JSON 除 `generatedAtMs` 外逐字节相同，且正文体积不影响任何一个数字；改动前小正文 25ms / RSS 40MB，同一批行换成 4,000 字符后 105ms / RSS 192MB，改动后两种情况都是 RSS 39MB（耗时 29ms → 46ms，剩下的差别来自 SQLite 把正文与统计列存在同一页里）。发行打包的两道硬检查同样有证据：未装标准库的目标在任何构建之前失败且不改写 `webui/dist`、不留下空壳发行目录（真实命令实测），容器与架构的错配由单元测试正面盯住（含 `aarch64-unknown-linux-gnu` 配 x86-64 ELF 这一例）。

已由自动化测试与真实进程冒烟共同覆盖的本轮能力：用户模板的创建/读取/替换/软删除与修订冲突、未声明变量和缺失必需占位符被拒、同一 `command_id` 重放返回原结果、模板预览的取值与示例值回落；Dashboard 在空库、真实活动后与项目筛选下的回算，以及同一批记录重复查询结果一致。用量口径与系统操作由 store 层正/负向用例（无记录时按终态给出原因、损坏记录标为无法解析、区间外排除、系统操作单列不混入对话 Token）与真实进程探针共同覆盖：连接测试落库后概览的 `systemOperations` 显示输入 21 / 输出 7 / 请求 1，随后一次真实失败的准备请求让打开的页面在无人点击刷新的情况下从「请求 0 / 用量未知 0 / 需要处理 0」变为「请求 1 / 用量未知 1（请求失败，没有收到可用用量）/ 需要处理 1」。导出由真实进程探针覆盖：用真实 API 走完准备与第一章（细纲 → 正文两次追加 → 整章终稿通过）后从工作台点击导出的两个按钮，产物与 `document_revisions` 的当前修订逐字一致，文件名分别由服务端给出 `雨幕.md` / `雨幕.txt`，未通过的第二章节标为 `（草稿）`，另行验证了 422 非法格式、404 不存在稿本与无章节空稿本三个边界。

长篇阅读由真实进程探针覆盖：在一份 401 章（正文 400 章 + 番外 1 章、正文约 80 万字符）与一份 51 章的样本上，从界面打开阅读区分别挂载 6 块 / 3 块且阅读区 DOM 节点为 49 / 25，打开耗时 43ms / 39ms——总章数涨 8 倍，渲染开销基本不变；四处定位（目录跳到第 390 章、从写作视图跳到第 3 章、查找命中第 380 章并高亮「孤灯」、跳最后一章）都在正文未加载时发起，事后测得目标块顶与视口顶相差 1px（末章因滚动范围夹住而停在可达位置）；阅读位置以「章 + 块内偏移」（第 368 章、偏移 80px）保存在浏览器本地存储，重新加载页面后再打开阅读区仍在同一位置（±1px）；切换写作/阅读视图保持同一位置；字号 +1 再 -1 后仍停在第 368 章；已挂载章节的正文与 `/manuscript` 返回的当前修订逐字相同，选中复制得到同样长度的文本，两个未写正文的计划章节显示「（本章尚无正文）」。

退出与本地交付由真实进程探针覆盖：把 `cargo xtask package --release` 产出的发行目录（`target/package/aarch64-apple-darwin/nekowriter/`，可执行文件为 Mach-O 64 位 arm64、262,571,600 字节）整体拷到仓库之外的 `/tmp/nw-relocate-release`，从那里以工作目录启动——另外用 `lsof` 确认运行中的进程**没有**任何打开的文件落在仓库路径下，即它不依赖源码树；同一次启动的发行清单显示目标三元组 `aarch64-apple-darwin` / 构建配置 `release` / 容器格式 `Mach-O 64 位（arm64）`。界面首页 200（498 字节）且引用到的 JS 资源 200（455KB），证明前端资源按可执行文件旁的 `webui/dist` 解析；数据目录里恰好出现 `nekowriter.sqlite3`（+ `-wal` / `-shm`）、`lancedb/`、`logs/`、进程锁文件，`logs/nekowriter.<日期>.log` 记录了启动地址与实际数据目录，仓库之外的那个目录只有发行目录、数据目录与应用日志。同一次运行里用真实 API 建出项目、保存目标、提交四项准备产物与细纲并逐项审核通过、两次追加正文（72 字）并整章终稿通过，第一章状态为 `approved`；随后把模型换成一个只接受连接、永不回应的桩，为第二章发起细纲请求（工作流进入 `request_in_flight`、桩收到 1 条连接），保持一条 SSE 连接并从界面点击「退出服务 → 确认退出」：`{"status":"shutting_down"}` 在 3ms 内返回，进程 0.2 秒内以退出码 0 结束，SSE 连接被服务端关闭，`nekowriter.sqlite3-wal` 消失（已 truncate 回主库），日志依次出现「收到界面退出请求，正在停止服务」「已取消在途模型请求 cancelled=1」「NekoWriter 已退出 writing_runs=1」。重新启动后项目、正文与第一章定稿原样还在，而那个在途 Run 已变为 `interrupted`：`runs` 里 `chapter_outline = interrupted`、该请求 `status = unknown` 且 `error_json` 为 `{"kind":"process_shutdown","message":"退出服务时请求是否被模型服务接受未知"}`、事件流里有 `run.changed` 与 `workflow.changed` 两条 `reason = process_shutdown`。同一条路径也验证了 SIGTERM：带一条打开的 SSE 连接时 0.1 秒内退出码 0、SSE 被关闭、日志出现「收到退出信号，正在停止服务」；不带 `--no-browser` 启动时日志出现「已在默认浏览器打开工作台」并且退出码为 0。

### 16.1 尚未覆盖的边界

- 退出后的界面状态（「本地服务已退出」横幅与面板状态）只有静态检查：预览桥在服务进程结束后即断开，因此没有在真实界面里截到退出之后的那一次渲染。已验证的是退出前的面板（地址、实际数据目录、二次确认文案）与退出动作本身。
- 退出收尾已在 macOS 的 **release** 产物上验证（见 §15）；Windows x64 发行包仍未经真实验收：本机（macOS arm64）没有 Windows 目标的链接器 / C 编译器，也没有 `x86_64-pc-windows-msvc` 标准库，因此没有 PE 产物可验。配方、`--target` 参数与 CI 步骤见 §11.5，这属于环境边界而非代码缺口。
- 发行目录已在 macOS 上真实产出并验证，但只验证了本机三元组（`aarch64-apple-darwin`）；x86_64 的 macOS 目标、Linux 与 Windows 目标都只需要 `rustup target add <三元组>` 加上该目标的链接器，形状与本次相同，但没有实跑过。
- release 产物未做符号裁剪：262.6MB（`__TEXT` 191MB、其中代码段 148MB，不含 DWARF）；本机实测 `strip -x` 可降到 205MB、完整 `strip` 降到 202MB，但没有把裁剪写进打包流程（各平台工具不同，且不影响「脱离源码可用」的验收）。
- 退出时取消的是本地 HTTP 任务；已经发往外部服务的请求是否被供应商执行不作保证，这一类不确定性以 `unknown` 状态与「用量未知」保留（见 §11.2）。
- 长篇阅读的滚动几何：未挂载章节的高度是按字数估计的（实测估计值与真实高度相差约 12%），因此远距离定位的前几步基于估计值，靠正文加载与测量逐步收敛；滚动条长度在整部作品都被测量过之前是近似的。
- 真实进程探针只在 401 章样本上做了定位与复制验证，没有逐章滚动完整部作品来核对每一章的内容。
- 导出的番外顺序只有 store 层用例覆盖（正文与番外混排、番外按自身序号编号）；本轮真实进程探针只走到正文两章，没有造出番外，因此「已有番外时的实际导出」未经真实界面验证。
- 字数统计的内存开销已与正文体积脱钩（只取统计列），但仍然要把可见文档的**全部**修订行扫一遍：净增量需要每个文档在区间之前的相邻修订，而 SQLite 把正文与其它列放在同一页里，所以表很大时仍有页读开销。实测 6,010 条修订 / 75MB 库：正文 10 字符时 7 天查询 29ms，同一批行换成 4,000 字符后 46ms（改动前是 25ms → 105ms，常驻内存从 40MB 飙到 192MB；现在两种情况都停在 39MB）。要走整条路（去掉行扫描本身）需要在库里额外维护一份不含正文的字数索引，属于尚未做的优化。
- 概览的 Token 与系统操作仍然是「区间内读行到内存再汇总」，不是 SQL 聚合。在 20 万行铺在 80 天的极端数据上，7 天查询百毫秒级、90 天查询秒级；真实库通常只有几千行，落在个位数毫秒，因此没有加保留策略或预聚合。
- 发行包的架构校验只识别 `x86_64` 与 `aarch64`（PE `0x8664` / `0xAA64`、Mach-O `0x01000007` / `0x0100000C`、ELF `0x3E` / `0xB7`）；32 位目标与其它架构会在参数解析阶段被明确拒绝，而不是被默默当成对的。
- 导出在每次请求时现读并直接返回，不在服务端落任何文件；因此没有导出历史、断点续传或后台打包，这是有意的（详见 §11.4）。
- 章节「通过整章审核后又编辑」在导出里会标为草稿，而不是设法找回通过的那一份文本；这是 §11.4 定下的判定，不是遗漏。

- Embedding 用量已经落库并单独列示（§12.1），角色与按日两个镜头也已补齐；它的已知边界是归属：知识 Space 是全局的，索引与检索的向量请求不属于任何项目，因此项目过滤下的概览仍显示同一份 Embedding 数字，它只是待在独立字段里、不进该项目的对话总量，并在 `tokens.embedding.scope` 里明写自己是全局数字。
- 概览的事件订阅挂在选中稿本上（游标来自该稿本的快照），因此没有任何项目时页面不会自行收到系统操作事件；此时连接测试的落库结果要等下一次事件或手动刷新才出现在页面上。
- 插入器的自动更新只被「模板列表随 `prompt_template.changed` 实时变化」这一路径验证过：`TemplateInserter` 只在章节阶段挂载，本轮探针没有跑到该阶段，所以「新建模板后立刻插入」仍是按代码接线成立、未经真实界面验证。
- Dashboard 的时区偏移由调用方给出，服务端不校验它与调用方本地时间是否一致；这是现有接口的明确边界，而非服务端猜测。
- 恢复不把工作流的写作光标搬回被恢复的那一章（有意，见 §8.4）；因此删除一章又从回收站恢复后，界面高亮的是删除时退回到的那一章，用户需要点一下目录才能回到恢复的章节。
- 删除没有「归档」或批量整理入口：项目、稿本与番外都要一个一个删，不能多选；项目删除不要求先切走，稿本删除则要求先切到别的稿本（§8.4 的原子性优先于便利）。
- 概览与提示词模板目前以工作台侧边栏面板呈现（可读可写、可在真实进程中验证），方案 §6 的顶部主导航与「项目卡片 → 概览跳转」仍属于 M5 的界面收尾。
- 阅读区里的「章节指导与细纲」展开是唯一的按需详情：展开只读取该章指导与细纲，不会加载别的章；本轮探针没有逐章展开验证它，只用到了正文路径。
- 键盘与中文输入验证用的是真实浏览器里的真实事件序列（`compositionstart` / 带 `isComposing` 与 `keyCode 229` 的 `keydown` / `compositionend` / 合成的隐式 `submit`），而不是操作系统里实际的输入法候选窗：因此「真实拼写过程中不误触发」在本机由事件语义等价地证明，但 macOS 以外平台的输入法行为、以及候选窗自身的按键，未经真实验收。
- `Ctrl/Cmd+Enter` 的「唯一阶段主操作」分支只在准备阶段注册了 `开始准备`、在恢复面板注册了 `显式继续`；这两个动作同时可用时会因为「不唯一」而都不触发（有意保守），其余阶段尚未逐一登记。
- 主题选择器放在工作台头部而不是独立的设置页；本次只做了浅色/深色/跟随系统，§13 提到的「字体缩放」在阅读区实现，没有作为全局设置。
- 阅读位置保存在浏览器本地存储，因此换浏览器或清空存储后回到第一章；这是有意的（位置是展示状态，不是作品数据）。
- Dashboard 的进行中任务区只暴露分项计数与展开的审核子 Run，尚未提供「点击任务回到对应章节」的跳转。
- 积累引擎的成功写入路径已按 §7.4 实现并有端到端验证，但生产注册项严格为 0（§7.4、方案 §3.3），因此运行中的应用不会产生任何积累写入：真实进程探针验证的是 `ENGINE_NOT_AVAILABLE` 与「不写条目、不建任务、不扣预算」，成功写入只在测试内注册表于真实端口上的真实 HTTP 里验证过；接入第一个真实引擎之前，界面上仍只显示「暂无可用引擎」。
- 积累候选没有「先审后写」的中间状态：§7.4 定的是「未来引擎通过应用服务验证并插入条目、触发索引」，没有第二个审核阶段，因此引擎返回的候选要么整批写入、要么整批不写，不会以「待审候选」形式停在知识库里。
- 阶段提交合同已经作为系统消息发出（§4.3）。本轮在真实 InferX 端点完成了连接测试；真实模型驱动的准备、细纲、正文、终稿完整端到端验收仍未完成。本地打桩服务走通四阶段只证明合同与状态机路径，不替代真实模型验收。
- 本机没有可用的本地模型运行时：`ollama` / `llama-cli` / `llama-server` / `lms` / `mlx_lm.generate` / `text-generation-launcher` / `vllm` / `koboldcpp` 均不在 PATH，无 Ollama 与 LM Studio 的应用或数据目录，没有任何 `.gguf` / `.safetensors` 权重，无 `transformers` / `torch` / docker，常见本地端口（11434 / 1234 / 8000 等）无人监听。因此「用真实权重做一次 M1/M3 验收」在本机不可行，属于环境边界；能用真实权重完成验收的环境应重跑 §15 的完整回路。
- 作为替代，用一个「只懂合同」的朴素桩证明了合同文本本身足以推导出合法提交：它不预设阶段名与字段名，只把系统消息里取出的 JSON 骨架当模板（含中文说明或 `…` 的字符串替换为生成文本，纯 ASCII 的枚举与 UUID 原样保留），工具调用只挑「合同正文提到且本次请求声明」的工具并按该工具的 JSON Schema 造参数。真实进程下准备四项 → 细纲 → 正文工具调用 → 终稿全部走通，第 1 章 `approved`。反例：只把章节级骨架的 `chapter_id` 改成 `chapterId` 并重建，同一回路在细纲阶段连续三次协议错误、Run 以 `waitReason = protocol_error_limit` 停在 `writing_outline_generating`（准备阶段合同未改，照常通过）；改回后再次 `approved`。这证明的是合同充分且修复位置正确，仍然不是真实模型验收。
