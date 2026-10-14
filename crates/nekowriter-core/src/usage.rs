//! Token usage of a model request (`docs/product-spec.md` §15).

use sea_orm::FromJsonQueryResult;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The token counts the model service reported. A count it did not report is `None`,
/// which means unknown, never zero (§4 invariant 13).
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS, FromJsonQueryResult,
)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TokenUsage {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    /// The part of the input served from the provider's cache.
    pub cached_input_tokens: Option<i64>,
    /// The part of the output spent on reasoning.
    pub reasoning_tokens: Option<i64>,
}

impl TokenUsage {
    /// Whether the counts are complete and consistent: input and output are known and not
    /// negative, and the cached and reasoning parts fit within them.
    #[must_use]
    pub fn is_consistent(&self) -> bool {
        let (Some(input), Some(output)) = (self.input_tokens, self.output_tokens) else {
            return false;
        };
        let part_fits =
            |part: Option<i64>, whole: i64| part.is_none_or(|part| (0..=whole).contains(&part));
        input >= 0
            && output >= 0
            && part_fits(self.cached_input_tokens, input)
            && part_fits(self.reasoning_tokens, output)
    }
}

/// Whether a request's token usage is known (§15).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsageStatus {
    /// The service reported consistent counts.
    Reported(TokenUsage),
    Unknown(UnknownUsage),
    /// The service reported counts that are incomplete or contradict each other.
    Invalid,
}

/// Why a request's token usage is unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnknownUsage {
    /// The request is still in flight.
    Pending,
    /// The answer came without usage.
    NotReported,
    /// No answer came: the request failed, was cancelled, or NekoWriter stopped during it.
    /// The service may still have charged for it.
    NoAnswer,
}

impl UsageStatus {
    /// Classifies a request by what the service reported (`reported` is `None` when it
    /// sent no usage object) and whether an answer came.
    #[must_use]
    pub fn of(reported: Option<TokenUsage>, answer: Answer) -> Self {
        match (reported, answer) {
            (Some(usage), _) if usage.is_consistent() => Self::Reported(usage),
            (Some(_), _) => Self::Invalid,
            (None, Answer::Pending) => Self::Unknown(UnknownUsage::Pending),
            (None, Answer::Received) => Self::Unknown(UnknownUsage::NotReported),
            (None, Answer::Missing) => Self::Unknown(UnknownUsage::NoAnswer),
        }
    }
}

/// Whether a request was answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Pending,
    Received,
    Missing,
}

/// The token usage of a group of requests. Counts are the sums over the requests whose
/// usage was reported; the other requests are counted by why their usage is unknown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UsageTotals {
    pub requests: u32,
    pub reported: u32,
    pub input_tokens: i64,
    pub output_tokens: i64,
    /// The part of the input served from the provider's cache, as far as reported.
    pub cached_input_tokens: i64,
    /// The part of the output spent on reasoning, as far as reported.
    pub reasoning_tokens: i64,
    pub pending: u32,
    pub not_reported: u32,
    pub no_answer: u32,
    pub invalid: u32,
}

impl UsageTotals {
    pub fn add(&mut self, status: UsageStatus) {
        self.requests += 1;
        match status {
            UsageStatus::Reported(usage) => {
                self.reported += 1;
                self.input_tokens += usage.input_tokens.unwrap_or_default();
                self.output_tokens += usage.output_tokens.unwrap_or_default();
                self.cached_input_tokens += usage.cached_input_tokens.unwrap_or_default();
                self.reasoning_tokens += usage.reasoning_tokens.unwrap_or_default();
            }
            UsageStatus::Unknown(UnknownUsage::Pending) => self.pending += 1,
            UsageStatus::Unknown(UnknownUsage::NotReported) => self.not_reported += 1,
            UsageStatus::Unknown(UnknownUsage::NoAnswer) => self.no_answer += 1,
            UsageStatus::Invalid => self.invalid += 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: Option<i64>, output: Option<i64>, cached: Option<i64>) -> TokenUsage {
        TokenUsage {
            input_tokens: input,
            output_tokens: output,
            cached_input_tokens: cached,
            reasoning_tokens: None,
        }
    }

    #[test]
    fn requests_are_classified_by_what_was_reported() {
        let full = usage(Some(900), Some(300), Some(800));
        assert_eq!(
            UsageStatus::of(Some(full), Answer::Received),
            UsageStatus::Reported(full)
        );
        for broken in [
            usage(Some(900), None, None),
            usage(Some(900), Some(300), Some(901)),
            usage(Some(-1), Some(300), None),
        ] {
            assert_eq!(
                UsageStatus::of(Some(broken), Answer::Received),
                UsageStatus::Invalid
            );
        }
        assert_eq!(
            UsageStatus::of(None, Answer::Received),
            UsageStatus::Unknown(UnknownUsage::NotReported)
        );
        assert_eq!(
            UsageStatus::of(None, Answer::Missing),
            UsageStatus::Unknown(UnknownUsage::NoAnswer)
        );
    }

    #[test]
    fn totals_sum_reported_counts_only() {
        let mut totals = UsageTotals::default();
        totals.add(UsageStatus::Reported(usage(Some(900), Some(300), None)));
        totals.add(UsageStatus::Unknown(UnknownUsage::Pending));
        totals.add(UsageStatus::Invalid);
        assert_eq!(
            totals,
            UsageTotals {
                requests: 3,
                reported: 1,
                input_tokens: 900,
                output_tokens: 300,
                pending: 1,
                invalid: 1,
                ..UsageTotals::default()
            }
        );
    }
}
