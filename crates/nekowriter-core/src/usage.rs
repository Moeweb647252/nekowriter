//! 模型用量的持久化口径。
//!
//! 一条用量记录既给出统一的计数字段，也说明数字从哪来（`usage_source`）以及是否
//! 真的拿到了报告（`usage_status`），并在没有报告时保留类型化的原因。没有报告时
//! 各计数是缺失而不是 0：0 表示「供应商报告了零用量」，缺失表示「我们不知道」，
//! 两者不能混为一谈。

use serde::{Deserialize, Serialize};

/// 一次用量报告来自哪里。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageSource {
    /// 供应商在 Chat Completions 流中报告的用量。
    ChatCompletionsStream,
    /// 供应商在 Embeddings 响应里报告的用量。
    EmbeddingsHttp,
    /// 没有任何供应商报告。
    None,
}

/// 一次请求是否真的拿到了用量报告。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageStatus {
    /// 供应商报告了用量，计数字段完整。
    Reported,
    /// 请求正常结束，但供应商没有在响应里报告用量。
    Unknown,
    /// 用量报告存在，但无法映射成统一字段。
    Invalid,
}

/// 一次请求没有可用用量的原因。
///
/// 这是「用量未知」的唯一来源：持久化记录带上它，Dashboard 按它分类展示，因此
/// 用户看到的是一个能解释自己的数字，而不是一个无法追问的 0。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageUnknownReason {
    /// 请求正常结束，供应商没有报告用量。
    ProviderSilent,
    /// 请求以失败或未知状态结束。
    RequestFailed,
    /// 请求被用户取消，从未收到结果。
    RequestCancelled,
    /// 请求已发出但尚未结束。
    Pending,
    /// 用量报告存在但无法解析。
    InvalidReport,
}

impl UsageUnknownReason {
    /// 面向用户的中文说明。
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::ProviderSilent => "供应商在完整响应中没有报告用量",
            Self::RequestFailed => "请求失败，没有收到可用用量",
            Self::RequestCancelled => "请求被取消，从未收到用量",
            Self::Pending => "请求尚未结束",
            Self::InvalidReport => "用量报告无法解析",
        }
    }

    /// 这条原因对应的口径状态：只有「报告无法解析」是收到了报告。
    #[must_use]
    const fn status(self) -> UsageStatus {
        match self {
            Self::InvalidReport => UsageStatus::Invalid,
            Self::ProviderSilent | Self::RequestFailed | Self::RequestCancelled | Self::Pending => {
                UsageStatus::Unknown
            }
        }
    }
}

/// 一条已封存的用量记录。
///
/// `raw` 保留供应商的原始用量字段，用于事后核对；它只在供应商确实报告过用量时
/// 存在。未知与无效记录带上 `reason`，而不是留一个空洞的 0。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersistedModelUsage {
    pub usage_source: UsageSource,
    pub usage_status: UsageStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<UsageUnknownReason>,
    #[serde(default, with = "super::option_decimal_i64")]
    pub input_tokens: Option<i64>,
    #[serde(default, with = "super::option_decimal_i64")]
    pub output_tokens: Option<i64>,
    #[serde(default, with = "super::option_decimal_i64")]
    pub total_tokens: Option<i64>,
    #[serde(default, with = "super::option_decimal_i64")]
    pub cached_input_tokens: Option<i64>,
    #[serde(default, with = "super::option_decimal_i64")]
    pub reasoning_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<serde_json::Value>,
}

impl PersistedModelUsage {
    /// 记录一次供应商在流中报告的用量。
    #[must_use]
    pub fn from_provider_stream(
        input_tokens: i64,
        output_tokens: i64,
        total_tokens: i64,
        cached_input_tokens: Option<i64>,
        reasoning_tokens: Option<i64>,
        raw: Option<serde_json::Value>,
    ) -> Self {
        Self {
            usage_source: UsageSource::ChatCompletionsStream,
            usage_status: UsageStatus::Reported,
            reason: None,
            input_tokens: Some(input_tokens),
            output_tokens: Some(output_tokens),
            total_tokens: Some(total_tokens),
            cached_input_tokens,
            reasoning_tokens,
            raw,
        }
    }

    /// 记录一次供应商在 Embeddings 响应里报告的用量。
    ///
    /// 嵌入请求没有输出侧，供应商报告的 `total_tokens` 就是输入用量，因此
    /// `output_tokens` 是结构性的 0（不是估算）：这条记录只会贡献输入用量。
    #[must_use]
    pub fn from_embeddings_response(
        prompt_tokens: i64,
        total_tokens: i64,
        raw: Option<serde_json::Value>,
    ) -> Self {
        Self {
            usage_source: UsageSource::EmbeddingsHttp,
            usage_status: UsageStatus::Reported,
            reason: None,
            input_tokens: Some(prompt_tokens),
            output_tokens: Some(0),
            total_tokens: Some(total_tokens),
            cached_input_tokens: None,
            reasoning_tokens: None,
            raw,
        }
    }

    /// 记录一次没有可用用量的请求，并说明原因。
    #[must_use]
    pub fn unknown(reason: UsageUnknownReason) -> Self {
        Self {
            usage_source: UsageSource::None,
            usage_status: reason.status(),
            reason: Some(reason),
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            cached_input_tokens: None,
            reasoning_tokens: None,
            raw: None,
        }
    }

    /// 这条记录是否给出了可用计数。
    #[must_use]
    pub const fn is_reported(&self) -> bool {
        matches!(self.usage_status, UsageStatus::Reported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeddings_usage_records_input_only() {
        let record = PersistedModelUsage::from_embeddings_response(
            128,
            128,
            Some(serde_json::json!({ "prompt_tokens": 128, "total_tokens": 128 })),
        );
        assert_eq!(record.usage_source, UsageSource::EmbeddingsHttp);
        assert_eq!(record.usage_status, UsageStatus::Reported);
        assert_eq!(record.input_tokens, Some(128));
        // 嵌入没有输出侧：0 是结构性的，不是猜测出来的用量。
        assert_eq!(record.output_tokens, Some(0));
        assert_eq!(record.cached_input_tokens, None);
        assert!(
            serde_json::to_string(&record)
                .unwrap()
                .contains("\"usageSource\":\"embeddings_http\"")
        );
    }

    #[test]
    fn reported_usage_round_trips_with_decimal_strings() {
        let record = PersistedModelUsage::from_provider_stream(
            10,
            4,
            14,
            Some(2),
            Some(1),
            Some(serde_json::json!({ "prompt_tokens": 10 })),
        );
        let encoded = serde_json::to_string(&record).unwrap();
        assert!(encoded.contains("\"inputTokens\":\"10\""));
        assert!(encoded.contains("\"usageStatus\":\"reported\""));
        assert_eq!(
            serde_json::from_str::<PersistedModelUsage>(&encoded).unwrap(),
            record
        );
    }

    #[test]
    fn records_without_a_report_carry_a_typed_reason_and_no_counts() {
        let silent = PersistedModelUsage::unknown(UsageUnknownReason::ProviderSilent);
        assert_eq!(silent.usage_status, UsageStatus::Unknown);
        assert_eq!(silent.usage_source, UsageSource::None);
        assert_eq!(silent.reason, Some(UsageUnknownReason::ProviderSilent));
        assert!(silent.input_tokens.is_none());
        assert!(!silent.is_reported());
        let invalid = PersistedModelUsage::unknown(UsageUnknownReason::InvalidReport);
        assert_eq!(invalid.usage_status, UsageStatus::Invalid);
        // 计数缺失必须与 0 区分：序列化后是 null，不是任何数字。
        let encoded = serde_json::to_string(&invalid).unwrap();
        assert!(encoded.contains("\"inputTokens\":null"));
        assert!(!encoded.contains("\"inputTokens\":\"0\""));
        assert!(
            serde_json::to_string(&silent)
                .unwrap()
                .contains("\"reason\"")
        );
    }

    #[test]
    fn unknown_reasons_have_stable_names_and_messages() {
        assert_eq!(
            serde_json::to_string(&UsageUnknownReason::ProviderSilent).unwrap(),
            "\"provider_silent\""
        );
        assert_eq!(
            serde_json::to_string(&UsageUnknownReason::RequestCancelled).unwrap(),
            "\"request_cancelled\""
        );
        assert_eq!(UsageUnknownReason::Pending.message(), "请求尚未结束");
        assert_eq!(
            UsageUnknownReason::InvalidReport.status(),
            UsageStatus::Invalid
        );
    }
}
