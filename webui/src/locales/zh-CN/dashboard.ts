import type en from "../en/dashboard"
import type { Catalog } from "@/i18n/catalog"

export default {
  title: "仪表盘",
  allWorks: "全部作品",
  work: "作品",
  range: "时间范围",
  ranges: {
    today: "今天",
    week: "最近 7 天",
    month: "最近 30 天",
  },
  jobs: {
    title: "任务",
    running: "进行中或等待中",
    noneRunning: "没有正在进行的任务。",
    attention: "需要处理",
    noneAttention: "没有需要处理的任务。",
    where: "{{work}}，版本 {{version}}",
    kind: {
      preparation: "前期准备",
      chapter_outline: "章节大纲",
      chapter_body: "章节正文",
      review: "审阅",
    },
    reviewAgent: "审阅 Agent：{{status}}",
  },
  words: {
    title: "字数",
    description: "章节正文新增的字数减去删除的字数。每个汉字计为一个字。",
    net: "净增长",
    total: "当前所有章节字数",
    chart: "净增长趋势",
  },
  usage: {
    title: "Token 用量",
    description: "模型服务为每个请求报告的 token 数。未报告用量的请求单独计数，不按零计算。",
    role: "用途",
    roles: {
      writing: "写作",
      review: "审阅 Agent",
      connectionTests: "连接测试",
      embeddings: "向量嵌入",
    },
    requests: "请求数",
    input: "输入",
    cachedInput: "缓存输入",
    output: "输出",
    reasoning: "推理",
    known: "已知合计",
    unknown: "未知",
    unknownNote:
      "用量未知的请求：{{notReported}} 个有回答但未报告用量，{{noAnswer}} 个没有回答，{{pending}} 个仍在进行，{{invalid}} 个数据自相矛盾。",
    chart: "已知 token 趋势",
  },
} satisfies Catalog<typeof en>
