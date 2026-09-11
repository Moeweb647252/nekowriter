//! 有效写作模型的解析与「某用途需要哪些能力」的判定。
//!
//! 能力资料属于模型，而不是某一级设置：系统写作模型与项目模型引用各自携带一份。
//! 解析只在这里实现一次，调度器构建请求、HTTP 命令放行判断与快照都必须经过它，
//! 避免出现「用系统模型的能力校验项目引用的模型」这类第二套规则。
//!
//! 项目引用只改变「在已配置服务上使用哪个模型」，不改变服务地址与凭据，因此
//! [`EffectiveModel`] 不携带 `api_base_url` / `credential_ref`。

use nekowriter_core::{ModelCapabilityProfile, ProjectSettings, ThinkingLevel};
use nekowriter_store::SystemModelSettings;

/// 有效模型来自哪一级设置。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelSource {
    System,
    Project,
}

impl ModelSource {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Project => "project",
        }
    }
}

/// 一次请求实际使用的模型：ID 与能力资料必须同源。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EffectiveModel<'a> {
    pub model_id: &'a str,
    pub capabilities: &'a ModelCapabilityProfile,
    pub source: ModelSource,
}

impl EffectiveModel<'_> {
    /// 供错误消息使用的定位信息：模型 ID 与它来自哪一级设置。
    #[must_use]
    pub fn describe(&self) -> String {
        match self.source {
            ModelSource::System => format!("系统写作模型 {}", self.model_id),
            ModelSource::Project => format!("项目引用的模型 {}（项目模型引用）", self.model_id),
        }
    }
}

/// 能力资料适用于哪一类模型请求。
///
/// 写作阶段需要工具（`ask_user`）与结构化提交；委托审核不调用工具，但同样必须能
/// 稳定返回结构化结论。这个差异只在这里表达一次。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelUse {
    Writing,
    Review,
}

/// 阻止请求发出的能力缺口。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityProblem {
    StructuredOutput,
    Tools,
    Thinking,
}

impl CapabilityProblem {
    /// 面向用户的说明；调用方负责在其前面补上有效模型的定位信息。
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            Self::StructuredOutput => "未声明支持结构化提交，而当前阶段每次请求都要求 JSON 对象",
            Self::Tools => "未声明支持工具调用，而写作阶段需要 ask_user",
            Self::Thinking => "未声明支持思考参数，而项目设置了思考等级",
        }
    }
}

/// 解析有效模型：项目引用优先，否则沿用系统写作模型。
#[must_use]
pub fn resolve_effective_model<'a>(
    system: &'a SystemModelSettings,
    project: &'a ProjectSettings,
) -> EffectiveModel<'a> {
    match project.model.as_ref() {
        Some(reference) => EffectiveModel {
            model_id: &reference.model_id,
            capabilities: &reference.capabilities,
            source: ModelSource::Project,
        },
        None => EffectiveModel {
            model_id: &system.model_id,
            capabilities: &system.capabilities,
            source: ModelSource::System,
        },
    }
}

/// 判断某个用途 + 思考等级在该能力资料下是否会必然失败。
///
/// 返回 `None` 表示可以发出请求；否则调用方必须在 dispatch 之前失败。
#[must_use]
pub fn pre_dispatch_capability_problem(
    capabilities: ModelCapabilityProfile,
    use_case: ModelUse,
    thinking_level: ThinkingLevel,
) -> Option<CapabilityProblem> {
    if !capabilities.structured_output {
        return Some(CapabilityProblem::StructuredOutput);
    }
    if use_case == ModelUse::Writing && !capabilities.tools {
        return Some(CapabilityProblem::Tools);
    }
    if thinking_level != ThinkingLevel::Off && !capabilities.thinking {
        return Some(CapabilityProblem::Thinking);
    }
    None
}

/// 把能力缺口组合成面向用户的完整说明。
#[must_use]
pub fn capability_problem_message(problem: CapabilityProblem, model: &str) -> String {
    format!("{model}{}", problem.describe())
}

/// 连接测试使用的探针等级。
///
/// 只有声明支持思考时才真的带上 `reasoning_effort`：这是「已验证」与「用户声称」
/// 的唯一区别，所以它必须是纯函数并可被单测锁住。
#[must_use]
pub const fn thinking_probe_level(thinking: bool) -> ThinkingLevel {
    if thinking {
        ThinkingLevel::Low
    } else {
        ThinkingLevel::Off
    }
}

#[cfg(test)]
mod tests {
    use nekowriter_core::ProjectModelReference;

    use super::*;

    fn capabilities(
        tools: bool,
        structured_output: bool,
        thinking: bool,
    ) -> ModelCapabilityProfile {
        ModelCapabilityProfile {
            streaming: true,
            tools,
            structured_output,
            stream_usage: true,
            thinking,
        }
    }

    fn system_settings(capabilities: ModelCapabilityProfile) -> SystemModelSettings {
        SystemModelSettings {
            api_base_url: "https://model.example/v1".to_owned(),
            credential_ref: None,
            model_id: "system-model".to_owned(),
            capabilities,
            max_context_tokens: 32_768,
            reserved_output_tokens: 4_096,
            revision: 1,
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    #[test]
    fn effective_model_prefers_project_reference() {
        let system = system_settings(capabilities(true, true, false));
        let project = ProjectSettings {
            model: Some(ProjectModelReference {
                model_id: "project-model".to_owned(),
                capabilities: capabilities(true, true, true),
            }),
            ..ProjectSettings::default()
        };
        let effective = resolve_effective_model(&system, &project);
        assert_eq!(effective.model_id, "project-model");
        assert_eq!(effective.source, ModelSource::Project);
        assert!(effective.capabilities.thinking);
    }

    #[test]
    fn effective_model_falls_back_to_system_without_reference() {
        let system = system_settings(capabilities(true, true, true));
        let project = ProjectSettings {
            model: None,
            ..ProjectSettings::default()
        };
        let effective = resolve_effective_model(&system, &project);
        assert_eq!(effective.model_id, "system-model");
        assert_eq!(effective.source, ModelSource::System);
        assert!(effective.capabilities.thinking);
    }

    #[test]
    fn thinking_probe_only_carries_reasoning_effort_when_declared() {
        assert_eq!(thinking_probe_level(true), ThinkingLevel::Low);
        assert_eq!(thinking_probe_level(false), ThinkingLevel::Off);
    }

    #[test]
    fn capability_matrix_matches_each_use() {
        let complete = capabilities(true, true, true);
        assert_eq!(
            pre_dispatch_capability_problem(complete, ModelUse::Writing, ThinkingLevel::High),
            None
        );
        assert_eq!(
            pre_dispatch_capability_problem(complete, ModelUse::Review, ThinkingLevel::Off),
            None
        );
        // 审核阶段不要求工具。
        let review_only = capabilities(false, true, false);
        assert_eq!(
            pre_dispatch_capability_problem(review_only, ModelUse::Review, ThinkingLevel::Off),
            None
        );
        assert_eq!(
            pre_dispatch_capability_problem(review_only, ModelUse::Writing, ThinkingLevel::Off),
            Some(CapabilityProblem::Tools)
        );
        assert_eq!(
            pre_dispatch_capability_problem(complete, ModelUse::Writing, ThinkingLevel::Low),
            None
        );
        let no_thinking = capabilities(true, true, false);
        assert_eq!(
            pre_dispatch_capability_problem(no_thinking, ModelUse::Writing, ThinkingLevel::Low),
            Some(CapabilityProblem::Thinking)
        );
        assert_eq!(
            pre_dispatch_capability_problem(no_thinking, ModelUse::Writing, ThinkingLevel::Off),
            None
        );
        let no_structured = capabilities(true, false, true);
        assert_eq!(
            pre_dispatch_capability_problem(no_structured, ModelUse::Review, ThinkingLevel::Off),
            Some(CapabilityProblem::StructuredOutput)
        );
    }
}
