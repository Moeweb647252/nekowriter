# 提示词实验室

`nekowriter-prompt-lab` 用隔离的 NekoWriter 数据目录运行真实模型请求，冻结输入、保存阶段快照，并导出模型请求与 Agent 消息账本。它服务于仓库内的十组内建提示词：候选改动先作为项目级覆盖运行，只有人工 Agent 根据实际文本判定改进后，才写回 `crates/nekowriter-agent/prompts/`。

它不替代编辑判断，也不会自动改写任何仓库提示词。

## 1. 构建与配置

先构建两个二进制：

```sh
cargo build -p nekowriter-app -p nekowriter-prompt-lab
```

模型配置是一个不含密钥的 JSON 文件，例如 `prompt-lab-model.json`：

```json
{
  "apiBaseUrl": "https://api.example.com/v1",
  "apiKeyEnv": "NEKOWRITER_PROMPT_LAB_API_KEY",
  "modelId": "writing-model",
  "capabilities": {
    "streaming": true,
    "tools": true,
    "streamUsage": true,
    "thinking": false
  },
  "maxContextTokens": 32768,
  "reservedOutputTokens": 4096
}
```

在运行前设置 `apiKeyEnv` 指定的环境变量。密钥不进入案例、模型配置、清单或证据报告。CLI 在服务运行期间按现有 NekoWriter 设置 API 提供密钥，并在正常停止服务前清空它；进程被强制终止时，实验数据库可能仍有该临时密钥，因此实验目录不得共享，随后应重新正常运行一次使 CLI 清理设置。

案例文件 `prompt-lab-case.json` 的最小形状如下。`chapterGuidance` 供操作者在进入章节时选用，CLI 不会替代外部 Agent 作创作决定。

```json
{
  "name": "悬疑开篇",
  "novelTitle": "雨夜来信",
  "novelBrief": "写一部都市悬疑小说……",
  "chapterGuidance": ["第一章从陌生来信切入，结尾给出可验证的反常细节。"]
}
```

## 2. 创建基线与候选实验

每次比较必须使用同一个案例文件；CLI 会记录它的 SHA-256。基线和候选各自复制完整提示词目录，之后的编辑互不影响。

```sh
target/debug/nekowriter-prompt-lab init \
  --case prompt-lab-case.json \
  --model prompt-lab-model.json \
  --prompts crates/nekowriter-agent/prompts \
  experiments/baseline

cp -R experiments/baseline experiments/candidate
# 只修改 experiments/candidate/prompts/ 中本轮要验证的 .system.md / .user.md 文件
target/debug/nekowriter-prompt-lab validate --prompts experiments/candidate/prompts
```

不要在候选中混入多项无关改动。`validate` 会拒绝未知模板变量、缺失必需变量和不完整的提示词对；阶段提交工具的协议由运行时追加，提示词不得试图改变它。

## 3. 运行与边界

`run` 启动一个仅监听回环地址的真实 NekoWriter 子进程，在实验目录的 `data/` 下创建作品、写入项目级提示词覆盖、保存简介，并开始准备阶段。它默认停在下一个人工边界。

```sh
target/debug/nekowriter-prompt-lab run experiments/baseline \
  --app target/debug/nekowriter
```

也可以用 `--until preparation|outline|chapter-final|complete` 要求观察到某一阶段；这不会绕过人工审核或问题回答。命令结束时会输出 `evidence/latest.json` 路径和当前阶段。

`evidence/latest.json` 包含：

- 当前准备产物、章节细纲、正文、审核、待答问题与状态修订；
- 已观察过的每个 Run 的真实模型请求快照和完整 Agent 消息账本；
- 不含 API key 的模型设置快照、上下文快照、用量与错误。

完整查看或只查看一个 Run：

```sh
target/debug/nekowriter-prompt-lab inspect experiments/baseline
target/debug/nekowriter-prompt-lab inspect experiments/baseline --run <run-id>
```

## 4. 外部 Agent 决定审核与下一步

外部 Agent 阅读 `inspect` 的实际产出后，自行决定是否通过、退回或回答问题。把决定保存为 JSON，再以 `act` 提交；每次提交后 CLI 会继续运行到下一个边界并写出一份新报告。

准备、细纲或终稿审核决定：

```json
{
  "kind": "review",
  "reviewId": "<report 中的 review id>",
  "expectedStateRevision": "<report 中 workflow.stateRevision>",
  "passed": false,
  "issues": "因果链缺少寄信人的行动动机。",
  "citations": []
}
```

提交它：

```sh
target/debug/nekowriter-prompt-lab act experiments/baseline \
  --app target/debug/nekowriter \
  --action review.json
```

其他支持的动作：

```json
{ "kind": "answer", "questionId": "…", "expectedStateRevision": "…", "answers": [] }
{ "kind": "start_next_chapter", "expectedStateRevision": "…", "guidance": "本章目标与约束" }
{ "kind": "start_outline", "chapterId": "…", "expectedStateRevision": "…" }
{ "kind": "continue_run", "runId": "…", "expectedStateRevision": "…" }
{ "kind": "confirm_completion", "expectedStateRevision": "…" }
```

`start_next_chapter` 会依次选择下一章、保存给定指导并启动细纲生成；这是明确的操作请求，不是自动审核或自动创作选择。

## 5. 比较与接受候选

在两个实验都到达可比较的边界后：

```sh
target/debug/nekowriter-prompt-lab compare experiments/baseline experiments/candidate
```

该命令只对齐冻结案例、提示词哈希、阶段、请求数量和请求状态。写作质量必须由外部 Agent 阅读两侧报告中的实际文本来判定。至少同时检查一个主案例与一个回归案例：候选应在主案例上有明确改善，且回归案例没有明显退化。

如果满足这两个条件，才把候选目录中的同一提示词文件写回 `crates/nekowriter-agent/prompts/`，再次执行 `validate`，并说明具体改善证据、回归结果和没有接受的风险。每轮最多接受一次候选，随后交回用户决定是否开始下一轮。
