import type en from "../en/prompts"
import type { Catalog } from "@/i18n/catalog"

export default {
  title: "提示词",
  description:
    "各个 Agent 遵循的指令。作品自己的提示词优先于系统提示词，系统提示词优先于内置文本。内置提示词用中文写成；Agent 用作品的写作语言撰写小说。",
  tabs: {
    agents: "Agent 提示词",
    templates: "我的模板",
  },
  scope: "适用范围",
  allWorks: "全部作品",
  list: "提示词",
  groups: {
    writing: "写作",
    review: "审阅",
  },
  keys: {
    "preparation.generate": "前期准备",
    "chapter.outline": "章节大纲",
    "chapter.write": "章节正文",
    "review.preparation.background": "审阅：背景",
    "review.preparation.research": "审阅：资料",
    "review.preparation.characters": "审阅：人物",
    "review.preparation.outline": "审阅：大纲",
    "review.chapter.outline": "审阅：章节大纲",
    "review.chapter.edit": "审阅：单次修改",
    "review.chapter.final": "审阅：整章",
  },
  roles: {
    system: "系统提示词",
    user: "用户提示词",
  },
  source: {
    builtin: "内置文本",
    system: "系统提示词",
    project: "本作品的提示词",
  },
  inEffect: "当前生效：{{source}}。",
  text: "提示词内容",
  textHint:
    "Agent 会在这段文字之后附上本阶段的材料和工具说明。双花括号中的占位符会在每次请求时填入。",
  save: "保存",
  saved: "已保存",
  reset: "恢复为{{source}}",
  conflict: "开始编辑后，这个提示词已被修改。保存前请先检查。",
  preview: "用示例预览",
  previewTitle: "预览",
  variables: "占位符",
  required: "必填",
  optional: "可选",
  example: "示例：{{example}}",
  variable: {
    writing_language: {
      help: "小说的写作语言。",
      example: "简体中文",
    },
    novel_brief: {
      help: "用户撰写的小说简介。",
      example: "寒冷海岸边一座灯塔里的鬼故事。",
    },
    required_artifacts: {
      help: "本轮需要撰写的前期文档，按顺序排列。",
      example: "背景、资料、人物、大纲",
    },
    chapter_kind: {
      help: "章节属于章节计划还是番外。",
      example: "正文",
    },
    chapter_number: {
      help: "章节在同类章节中的序号。",
      example: "3",
    },
    chapter_title: {
      help: "章节标题。",
      example: "风暴",
    },
    chapter_goal: {
      help: "章节计划中该章的目标；番外为空。",
      example: "风暴中灯塔的灯熄灭了。",
    },
    chapter_guidance: {
      help: "撰写前用户对本章的要求；可以为空。",
      example: "直到结尾都让守塔人保持沉默。",
    },
  },
} satisfies Catalog<typeof en>
