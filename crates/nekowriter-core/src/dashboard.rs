//! Dashboard 的可回算统计口径。
//!
//! 这里的类型只描述查询结果，不保存任何累计值：Token 用量来自已封存模型请求的
//! `usage_json`，任务计数来自 Run 行，字数增删来自正文修订链，因此统计不会与
//! 实际记录发生漂移。时间范围统一由用户时区换算成 UTC 左闭右开区间，日期分桶
//! 只在查询层完成。

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use super::{UsageStatus, UsageUnknownReason};

/// 一天对应的毫秒数。
pub const DASHBOARD_DAY_MS: i64 = 86_400_000;
/// 默认统计范围：今天与近六天。
pub const DEFAULT_DASHBOARD_RANGE_DAYS: i64 = 7;
/// 单次统计允许的最大天数。
pub const MAX_DASHBOARD_RANGE_DAYS: i64 = 90;
/// 时区偏移上限（±14 小时）。
pub const MAX_TIMEZONE_OFFSET_MINUTES: i32 = 14 * 60;

/// 统计查询参数不合法时的稳定错误分类。
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum DashboardQueryError {
    #[error("统计天数必须在 1 到 {max} 天之间")]
    InvalidRangeDays { max: i64 },
    #[error("时区偏移必须在 ±{max} 分钟之间")]
    InvalidTimezoneOffset { max: i32 },
}

/// 一次统计的 UTC 左闭右开区间。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardRange {
    #[serde(with = "super::decimal_i64")]
    pub start_ms: i64,
    #[serde(with = "super::decimal_i64")]
    pub end_ms: i64,
    pub timezone_offset_minutes: i32,
    #[serde(with = "super::decimal_i64")]
    pub days: i64,
}

impl DashboardRange {
    /// 由「现在」、用户时区偏移与天数推出统计区间。
    ///
    /// 区间结束于当前时刻（右开），起点落在 `days - 1` 天前的本地零点。
    ///
    /// # Errors
    ///
    /// 天数不在 1..=[`MAX_DASHBOARD_RANGE_DAYS`]，或时区偏移超出 ±14 小时时
    /// 返回 [`DashboardQueryError`]。
    pub fn new(
        now_ms: i64,
        timezone_offset_minutes: i32,
        days: i64,
    ) -> Result<Self, DashboardQueryError> {
        if !(1..=MAX_DASHBOARD_RANGE_DAYS).contains(&days) {
            return Err(DashboardQueryError::InvalidRangeDays {
                max: MAX_DASHBOARD_RANGE_DAYS,
            });
        }
        if timezone_offset_minutes.unsigned_abs() > MAX_TIMEZONE_OFFSET_MINUTES.unsigned_abs() {
            return Err(DashboardQueryError::InvalidTimezoneOffset {
                max: MAX_TIMEZONE_OFFSET_MINUTES,
            });
        }
        let offset_ms = i64::from(timezone_offset_minutes) * 60_000;
        let today = (now_ms + offset_ms).div_euclid(DASHBOARD_DAY_MS);
        let start_day = today - (days - 1);
        let start_ms = start_day * DASHBOARD_DAY_MS - offset_ms;
        Ok(Self {
            start_ms,
            end_ms: start_ms + days * DASHBOARD_DAY_MS,
            timezone_offset_minutes,
            days,
        })
    }

    /// 某个时间戳落在哪个本地日（自 Unix 纪元起的整数天）。
    #[must_use]
    pub fn day_index(&self, timestamp_ms: i64) -> i64 {
        (timestamp_ms + i64::from(self.timezone_offset_minutes) * 60_000)
            .div_euclid(DASHBOARD_DAY_MS)
    }

    /// 该本地日在 UTC 下的起点。
    #[must_use]
    pub fn day_start_ms(&self, day_index: i64) -> i64 {
        day_index * DASHBOARD_DAY_MS - i64::from(self.timezone_offset_minutes) * 60_000
    }

    /// 统计区间覆盖的本地日，按时间升序；未落入任何一天的条目不参与分桶。
    #[must_use]
    pub fn day_indexes(&self) -> Vec<i64> {
        let last = self.day_index(self.end_ms - 1);
        let first = self.day_index(self.start_ms);
        (first..=last).collect()
    }
}

/// 一次 Dashboard 查询的完整结果。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dashboard {
    #[serde(with = "super::decimal_i64")]
    pub generated_at_ms: i64,
    pub range: DashboardRange,
    pub project_id: Option<Uuid>,
    pub tokens: DashboardTokens,
    pub jobs: DashboardJobs,
    pub words: DashboardWords,
}

/// 对话 Token 用量：主计数是已知的输入与输出，未知用量的请求数单独列出。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardTokens {
    pub input_tokens: i64,
    pub output_tokens: i64,
    /// `input_tokens` 的子集，不重复计入总量。
    pub cached_input_tokens: i64,
    /// 供应商报告为输出子集的思考用量，不重复计入总量。
    pub reasoning_tokens: i64,
    pub request_count: i64,
    /// 没有任何可用用量的请求数，等于 `unknown_usage` 各原因之和。
    pub unknown_usage_requests: i64,
    /// 未知用量按原因拆分，使这个数字能自己说明原因。
    pub unknown_usage: Vec<DashboardUnknownUsage>,
    pub by_role: Vec<DashboardTokenRole>,
    pub daily: Vec<DashboardDailyTokens>,
    /// 不属于任何稿本的系统操作（例如模型连接测试）。
    pub system_operations: DashboardSystemOperations,
    /// Embedding 的输入用量；单独列示，不计入上面的对话总量。
    pub embedding: DashboardEmbeddingUsage,
}

/// 一种「用量未知」原因及其请求数。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardUnknownUsage {
    pub reason: UsageUnknownReason,
    pub requests: i64,
}

/// 角色镜头里的 Embedding 角色名。
///
/// 它与写作、审核并列，但统计的是独立列示的向量输入用量：`DashboardEmbeddingUsage`
/// 保留明细，这里只是把同一批数字放进角色镜头。
pub const DASHBOARD_ROLE_EMBEDDING: &str = "embedding";

/// 用量数字的归属范围。
///
/// 概览可以被 `projectId` 过滤，但有些数字不属于任何项目；字段自身说明这一点，
/// 界面因此不必猜“这个数字随筛选变化吗”。
pub const DASHBOARD_SCOPE_GLOBAL: &str = "global";

/// Embedding 的用量汇总。
///
/// 索引与检索的向量请求不属于任何稿本，也不是对话请求，因此既不能混进对话
/// Token 总数，也不能消失：它们在这里独立列示，并保留供应商没有报告用量时的
/// 类型化原因。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardEmbeddingUsage {
    pub request_count: i64,
    /// 嵌入请求没有输出侧，因此这里只有输入用量。
    pub input_tokens: i64,
    /// 没有任何可用用量的请求数，等于 `unknown_usage` 各原因之和。
    pub unknown_usage_requests: i64,
    pub unknown_usage: Vec<DashboardUnknownUsage>,
    /// 按操作类型拆分，使这个数字能说明自己是索引、检索还是连接测试。
    pub by_kind: Vec<DashboardEmbeddingKind>,
    /// 按本地日分桶，与 `tokens.daily` 同一套时间口径，但独立成序列：向量用量
    /// 不进入对话日柱。
    pub daily: Vec<DashboardDailyEmbedding>,
    /// 这份数字的归属范围，取值见 [`DASHBOARD_SCOPE_GLOBAL`]：知识 Space 是全局的，
    /// 向量请求不属于任何项目，因此即使请求带了 `projectId`，这里仍是全局数字。
    pub scope: String,
}

/// 按本地日分桶的 Embedding 用量。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardDailyEmbedding {
    #[serde(with = "super::decimal_i64")]
    pub day_start_ms: i64,
    pub input_tokens: i64,
    pub request_count: i64,
}

/// Embedding 用量按操作类型的拆分。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardEmbeddingKind {
    pub kind: String,
    pub request_count: i64,
    pub input_tokens: i64,
}

/// 系统操作的用量汇总与最近若干条明细。
///
/// 连接测试这类请求没有所属稿本，不能混进对话 Token，也不能消失；它们单列在
/// 这里，并保留失败原因，用户因此能对上「刚才那次测试花了多少、成功没有」。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardSystemOperations {
    pub request_count: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_input_tokens: i64,
    pub reasoning_tokens: i64,
    pub unknown_usage: Vec<DashboardUnknownUsage>,
    /// 按时间倒序的最近若干条，供界面解释每一次系统请求。
    pub recent: Vec<DashboardSystemOperation>,
}

/// 一条系统操作的明细。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardSystemOperation {
    pub id: Uuid,
    /// 操作类型，例如 `connection_test`。
    pub kind: String,
    pub model_id: String,
    pub api_base_url: String,
    pub status: String,
    pub usage_status: UsageStatus,
    #[serde(with = "super::decimal_i64")]
    pub created_at_ms: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    /// 失败时的类型化原因；成功时为 `None`。
    pub error_kind: Option<String>,
}

/// 按角色（写作/审核/Embedding）拆分的 Token 用量。
///
/// `embedding` 角色与对话角色同形，但它统计的是独立列示的向量输入用量，因此
/// 各角色之和只在对话角色（写作、审核）上等于 `DashboardTokens` 的对话总量。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardTokenRole {
    pub role: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_input_tokens: i64,
    pub reasoning_tokens: i64,
    pub request_count: i64,
    pub unknown_usage_requests: i64,
}

/// 按本地日分桶的 Token 用量。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardDailyTokens {
    #[serde(with = "super::decimal_i64")]
    pub day_start_ms: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub request_count: i64,
}

/// 进行中任务与需要处理的任务。
///
/// 主计数只统计顶层 Run，避免一个作品因审核子 Run 重复计数；审核子 Run 在
/// `reviews` 中展开显示。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardJobs {
    pub queued: i64,
    pub running: i64,
    pub waiting_user: i64,
    pub waiting_review: i64,
    pub paused: i64,
    /// 已中断与失败，需要用户处理。
    pub needs_attention: i64,
    pub reviews: Vec<DashboardReviewJob>,
}

/// 一条展开显示的审核子 Run。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardReviewJob {
    pub run_id: Uuid,
    pub parent_run_id: Option<Uuid>,
    pub project_id: Uuid,
    pub version_id: Uuid,
    pub stage: String,
    pub status: String,
}

/// 正文净字数增量与当前可见总量。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardWords {
    /// 区间内正文修订的净增量，可为负数。
    pub net_increment: i64,
    /// 当前可见稿本正文的现有非空白字符总数（含所有正文修订的最终状态）。
    pub current_total: i64,
    pub daily: Vec<DashboardDailyWords>,
}

/// 按本地日分桶的正文净增。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardDailyWords {
    #[serde(with = "super::decimal_i64")]
    pub day_start_ms: i64,
    pub net_increment: i64,
    pub revisions: i64,
}

/// 一次模型请求在统计中的角色。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DashboardUsageRole {
    Writing,
    Review,
}

impl DashboardUsageRole {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Writing => "writing",
            Self::Review => "review",
        }
    }
}

/// 把 Run 类型映射为 Token 统计角色。
#[must_use]
pub fn dashboard_usage_role(run_kind: &str) -> DashboardUsageRole {
    if run_kind == "review" {
        DashboardUsageRole::Review
    } else {
        DashboardUsageRole::Writing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_is_left_closed_right_open_in_user_timezone() {
        // UTC+8 的 2026-09-12 10:00。
        let now_ms = 1_788_000_000_000;
        let range = DashboardRange::new(now_ms, 480, 7).unwrap();
        assert_eq!(range.days, 7);
        assert_eq!(range.end_ms, range.start_ms + 7 * DASHBOARD_DAY_MS);
        assert!(range.start_ms < now_ms && now_ms < range.end_ms);
        // 起点必须是本地零点。
        assert_eq!((range.start_ms + 480 * 60_000) % DASHBOARD_DAY_MS, 0);
        assert_eq!(range.day_indexes().len(), 7);
        assert_eq!(range.day_index(range.start_ms), range.day_indexes()[0]);
        assert_eq!(range.day_index(range.end_ms - 1), range.day_indexes()[6]);
        assert_eq!(
            range.day_start_ms(range.day_index(now_ms)) + DASHBOARD_DAY_MS,
            range.day_start_ms(range.day_index(now_ms) + 1)
        );
    }

    #[test]
    fn range_rejects_out_of_bounds_parameters() {
        assert_eq!(
            DashboardRange::new(0, 0, 0).unwrap_err(),
            DashboardQueryError::InvalidRangeDays {
                max: MAX_DASHBOARD_RANGE_DAYS
            }
        );
        assert_eq!(
            DashboardRange::new(0, 15 * 60, 7).unwrap_err(),
            DashboardQueryError::InvalidTimezoneOffset {
                max: MAX_TIMEZONE_OFFSET_MINUTES
            }
        );
    }

    #[test]
    fn usage_role_splits_review_from_writing() {
        assert_eq!(dashboard_usage_role("review").as_str(), "review");
        assert_eq!(dashboard_usage_role("chapter_body").as_str(), "writing");
        assert_eq!(dashboard_usage_role("preparation").as_str(), "writing");
    }
}
