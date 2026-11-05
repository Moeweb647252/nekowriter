# Implementation Plan: the context view shows the book's subdivisions

> Milestone **M24** in `docs/PLAN.md`. Builds on M23 (D-171). The decision is recorded as **D-172**.

## 0. What the author asked for

The context treemap groups the kickoff as one opaque block per prompt key (`block.settings`,
`block.memory`). The author's book sidebar, however, subdivides the book into **设定资料** (大纲,
世界观, 主角, 角色), **正典** (实体, 时间线, 记忆, 伏笔, 写作目标, 变更记录, 设定建议) and **正文**
(最近章节, 本章草稿). They asked: "这些细分也要体现在上下文组成里" — the context composition must
show the same subdivisions. Asked whether tool results should also be split by their source system,
they answered: **"工具结果不细分"**.

## 1. Decisions

| # | Decision |
|---|---|
| S1 | Only the **kickoff** is subdivided. The `results` group keeps one part per tool, as today. |
| S2 | `block.settings` splits into four prompt keys: `block.settings.outline`, `block.settings.characters`, `block.settings.world`, `block.settings.protagonist`. Each renders exactly its section of today's template (its heading included). |
| S3 | `block.memory` splits into four prompt keys: `block.memory.recent` (近期章节摘要 + 上一章结尾 + 本章章纲), `block.memory.entities` (主角状态 + 状态变化 + 出场记录 + 久未出场 + 主要实体), `block.memory.facts` (核心事实与世界规则), `block.memory.foreshadowing` (未回收的伏笔). |
| S4 | The sub-blocks replace their parents in every `kickoff_blocks` list, in the same order, with the same cache breakpoint on the first settings sub-block. The rendered text, concatenated in order, is **identical** to today's parent rendering — proven by a test that renders the old parent and the new sequence and compares them byte for byte. |
| S5 | The context treemap labels the new parts with the sidebar's names: `block.settings.outline` → 大纲 / Outline, and so on. The existing `agent:context.block.*` keys gain the new entries in both locales. |
| S6 | Nothing else changes: no migration, no API change, no change to `StoredBlock` (the existing `source` carries the new key). |

## 2. Tasks

| # | Task | Acceptance |
|---|---|---|
| 24.1 | **Docs**: this plan, D-172, the spec rows that name the kickoff blocks (`spec/03` §10, `spec/09` §2) | `check.sh` green (docs only) |
| 24.2 | **Prompt pack and assembly**: the eight sub-block keys in `prompts/blocks.toml`, `kickoff_blocks` rewritten to use them, `validate_prompts.py`, samples | The concatenation test of S4; `kickoff_baseline` updated deliberately with a per-kind diff; no text lost |
| 24.3 | **Context labels**: `agent:context.block.*` for the eight keys in both locales; the treemap renders them | Vitest: the kickoff group shows 大纲 / 世界观 / 主角 / 角色 and 近期 / 实体 / 事实 / 伏笔; tool results still one part per tool |
| 24.4 | **Smoke**: open the context view on a write session and check the subdivisions against the sidebar | screenshots; D-172 consequences |

## 3. Acceptance

- The rendered kickoff text for every session kind is byte-identical to the pre-split rendering
  (a test renders both and compares).
- The treemap's kickoff parts are the new subdivisions, labelled like the sidebar.
- Tool results remain one part per tool.
- `check.sh` green; both locales; `kickoff_baseline` diff explained per kind.
