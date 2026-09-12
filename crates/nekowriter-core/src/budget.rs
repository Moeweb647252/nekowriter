//! 辅助工具预算账本（细则 §7.1）。
//!
//! 作用域按「准备阶段」和「每章细纲」划分，只记录已用数；有效上限在每次工具执行前
//! 从项目最新设置读取，因此修改预算不会清空已用数，也不创建新的作用域。

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// `ask_user` 不消耗辅助工具预算。
pub const TOOL_ASK_USER: &str = "ask_user";
/// 知识检索工具，每次真实执行消耗 1 次预算。
pub const TOOL_QUERY_KNOWLEDGE: &str = "query_knowledge";
/// 联网研究工具，每次真实执行消耗 1 次预算；MVP 没有生产引擎。
pub const TOOL_RESEARCH_WEB: &str = "research_web";

/// 准备阶段的预算作用域键。
pub const QUOTA_SCOPE_PREPARATION: &str = "preparation";
/// 单章细纲的预算作用域键前缀。
pub const QUOTA_SCOPE_CHAPTER_OUTLINE_PREFIX: &str = "chapter_outline:";

/// 准备阶段与每章细纲共用的预算作用域键。
#[must_use]
pub fn chapter_outline_quota_scope_key(chapter_id: Uuid) -> String {
    format!("{QUOTA_SCOPE_CHAPTER_OUTLINE_PREFIX}{chapter_id}")
}

/// 该工具是否消耗辅助工具预算。`ask_user` 与正文工具不消耗。
#[must_use]
pub fn is_chargeable_auxiliary_tool(tool_name: &str) -> bool {
    matches!(tool_name, TOOL_QUERY_KNOWLEDGE | TOOL_RESEARCH_WEB)
}

/// 一个预算作用域的余额。`limit` 始终来自当前项目设置，不是作用域创建时的快照。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaBalance {
    #[serde(with = "crate::decimal_i64")]
    pub used: i64,
    #[serde(with = "crate::decimal_i64")]
    pub limit: i64,
    #[serde(with = "crate::decimal_i64")]
    pub remaining: i64,
}

impl QuotaBalance {
    /// 由已用数与当前上限计算余额；下调到已用数以下时剩余为 0。
    #[must_use]
    pub fn new(used: i64, limit: i64) -> Self {
        Self {
            used,
            limit,
            remaining: (limit - used).max(0),
        }
    }
}

/// 一次收费工具执行的扣费结果。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuxiliaryToolCharge {
    /// 本次登记了新消耗。
    Charged(QuotaBalance),
    /// 同一个工具调用之前已经扣过费，不重复扣除。
    AlreadyCharged(QuotaBalance),
    /// 余额为 0，本次不扣除，工具必须返回 `QUOTA_EXHAUSTED`。
    Exhausted(QuotaBalance),
}
