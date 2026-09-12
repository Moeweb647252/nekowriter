//! 联网研究与知识积累引擎的接口合同与编译期注册表（细则 §7.4）。
//!
//! MVP 的生产注册表严格为空：能力查询返回空数组与原因，调用未注册引擎返回
//! `ENGINE_NOT_AVAILABLE`，不创建运行任务，也不扣辅助工具预算。合同本身是完整的，
//! 测试只为本文件中的契约测试实现引擎，不进入生产注册表。

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// 一个引擎的编译期描述符。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
}

/// 引擎调用失败。`NotAvailable` 对应协议里的 `ENGINE_NOT_AVAILABLE`。
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum EngineError {
    #[error("没有可用的引擎：{reason}")]
    NotAvailable { reason: String },
    #[error("引擎执行失败：{message}")]
    Failed { message: String },
}

impl EngineError {
    /// 协议错误码（细则 §13）。
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotAvailable { .. } => "ENGINE_NOT_AVAILABLE",
            Self::Failed { .. } => "ENGINE_FAILED",
        }
    }
}

/// 一次联网研究请求：研究问题、用户线索与语言。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchRequest {
    pub question: String,
    pub clues: String,
    pub language: String,
}

/// 一条研究来源；`accessed_at_ms` 是引擎实际访问时间。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchSource {
    pub url: String,
    pub title: String,
    #[serde(with = "nekowriter_core::decimal_i64")]
    pub accessed_at_ms: i64,
}

/// 研究结果：发现摘要、来源与限制说明。它不包含小说正文。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchOutcome {
    pub summary: String,
    pub sources: Vec<ResearchSource>,
    pub limitations: String,
}

/// 联网研究接口合同。
pub trait ResearchEngine: Send + Sync {
    fn descriptor(&self) -> EngineDescriptor;
    fn research<'a>(
        &'a self,
        request: &'a ResearchRequest,
    ) -> BoxFuture<'a, Result<ResearchOutcome, EngineError>>;
}

/// 一次知识积累请求：用户线索与目标 Space。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccumulationRequest {
    pub clues: String,
    pub space_id: Uuid,
}

/// 引擎提出的候选知识条目；由应用服务校验后写入 SQLite 并触发索引。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccumulatedEntry {
    pub title: String,
    pub keywords: Vec<String>,
    pub content: String,
    pub source: Option<String>,
}

/// 积累结果：候选条目列表。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccumulationOutcome {
    pub entries: Vec<AccumulatedEntry>,
}

/// 知识积累接口合同。
pub trait AccumulationEngine: Send + Sync {
    fn descriptor(&self) -> EngineDescriptor;
    fn accumulate<'a>(
        &'a self,
        request: &'a AccumulationRequest,
    ) -> BoxFuture<'a, Result<AccumulationOutcome, EngineError>>;
}

/// 生产联网研究注册表；MVP 为 0 项。
pub static RESEARCH_ENGINES: &[&dyn ResearchEngine] = &[];
/// 生产知识积累注册表；MVP 为 0 项。
pub static ACCUMULATION_ENGINES: &[&dyn AccumulationEngine] = &[];

/// 能力查询为空时给出的原因。
pub const RESEARCH_UNAVAILABLE_REASON: &str =
    "本版本未内置联网研究引擎；接口合同已就绪，接入具体引擎后这里才会出现可选项。";
/// 能力查询为空时给出的原因。
pub const ACCUMULATION_UNAVAILABLE_REASON: &str =
    "本版本未内置知识积累引擎；接口合同已就绪，接入具体引擎后这里才会出现可选项。";

/// 按 ID 查找已注册的联网研究引擎。
///
/// # Errors
///
/// 没有注册任何引擎，或不存在该 ID 时返回 [`EngineError::NotAvailable`]。
pub fn research_engine(id: &str) -> Result<&'static dyn ResearchEngine, EngineError> {
    RESEARCH_ENGINES
        .iter()
        .copied()
        .find(|engine| engine.descriptor().id == id)
        .ok_or_else(|| EngineError::NotAvailable {
            reason: RESEARCH_UNAVAILABLE_REASON.to_owned(),
        })
}

/// 按 ID 查找已注册的知识积累引擎。
///
/// # Errors
///
/// 没有注册任何引擎，或不存在该 ID 时返回 [`EngineError::NotAvailable`]。
pub fn accumulation_engine(id: &str) -> Result<&'static dyn AccumulationEngine, EngineError> {
    ACCUMULATION_ENGINES
        .iter()
        .copied()
        .find(|engine| engine.descriptor().id == id)
        .ok_or_else(|| EngineError::NotAvailable {
            reason: ACCUMULATION_UNAVAILABLE_REASON.to_owned(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::future;

    struct StubResearchEngine;
    struct StubAccumulationEngine;

    impl ResearchEngine for StubResearchEngine {
        fn descriptor(&self) -> EngineDescriptor {
            EngineDescriptor {
                id: "stub-research",
                display_name: "契约测试研究引擎",
            }
        }

        fn research<'a>(
            &'a self,
            request: &'a ResearchRequest,
        ) -> BoxFuture<'a, Result<ResearchOutcome, EngineError>> {
            Box::pin(future::ready(Err(EngineError::Failed {
                message: request.question.clone(),
            })))
        }
    }

    impl AccumulationEngine for StubAccumulationEngine {
        fn descriptor(&self) -> EngineDescriptor {
            EngineDescriptor {
                id: "stub-accumulation",
                display_name: "契约测试积累引擎",
            }
        }

        fn accumulate<'a>(
            &'a self,
            request: &'a AccumulationRequest,
        ) -> BoxFuture<'a, Result<AccumulationOutcome, EngineError>> {
            Box::pin(future::ready(Ok(AccumulationOutcome {
                entries: vec![AccumulatedEntry {
                    title: request.clues.clone(),
                    keywords: Vec::new(),
                    content: String::new(),
                    source: None,
                }],
            })))
        }
    }

    #[tokio::test]
    async fn production_registry_is_empty_and_reports_not_available() {
        assert!(RESEARCH_ENGINES.is_empty());
        assert!(ACCUMULATION_ENGINES.is_empty());

        let research = research_engine("anything").err().expect("注册表为空");
        assert_eq!(research.code(), "ENGINE_NOT_AVAILABLE");
        let accumulation = accumulation_engine("anything").err().expect("注册表为空");
        assert_eq!(accumulation.code(), "ENGINE_NOT_AVAILABLE");
    }

    #[tokio::test]
    async fn engine_contracts_dispatch_through_a_test_only_registry() {
        let registry: &[&dyn ResearchEngine] = &[&StubResearchEngine];
        let engine = registry
            .iter()
            .copied()
            .find(|engine| engine.descriptor().id == "stub-research")
            .expect("测试注册表应当含有该引擎");
        let outcome = engine
            .research(&ResearchRequest {
                question: "雾港的历史".to_owned(),
                clues: String::new(),
                language: "zh".to_owned(),
            })
            .await
            .unwrap_err();
        assert_eq!(outcome.code(), "ENGINE_FAILED");

        let registry: &[&dyn AccumulationEngine] = &[&StubAccumulationEngine];
        let engine = registry.first().copied().expect("测试注册表应当含有该引擎");
        let outcome = engine
            .accumulate(&AccumulationRequest {
                clues: "灯塔".to_owned(),
                space_id: Uuid::nil(),
            })
            .await
            .unwrap();
        assert_eq!(outcome.entries.len(), 1);
    }
}
