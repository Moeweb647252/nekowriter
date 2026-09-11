//! 内建提示词的覆盖解析、目录与保存前预览。
//!
//! 有效提示词的优先级固定为「项目覆盖 → 系统覆盖 → 代码内建」。解析只在这里实现
//! 一次：HTTP 目录、预览与调度器构建请求都必须经过它，避免出现第二套优先级规则。
//!
//! 覆盖内容按受限模板语言校验（未知变量、缺少必需占位符、不完整括号都会被拒
//! 绝），预览使用内建变量声明的示例值渲染，因此不需要真实稿本或密钥。

use std::collections::BTreeMap;

use nekowriter_agent::{
    PromptKey, PromptTemplateError, PromptVariableDefinition, RenderedBuiltinPrompt,
    builtin_prompt, builtin_prompt_examples, builtin_prompt_variables, render_prompt_content,
    validate_builtin_prompt_pair,
};
use nekowriter_core::{ProjectId, VersionId};
use nekowriter_store::{PromptOverride, PromptOverrideScope, Store, StoreError};
use serde::{Deserialize, Serialize};

use crate::ApiError;

/// 一个提示词键当前生效内容的来源。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptSource {
    Builtin,
    System,
    Project,
}

impl PromptSource {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::System => "system",
            Self::Project => "project",
        }
    }
}

/// 一个提示词键按优先级解析出的实际内容。
#[derive(Clone, Debug)]
pub struct EffectivePrompt {
    pub system: String,
    pub user: String,
    pub source: PromptSource,
}

impl EffectivePrompt {
    /// 用运行时变量渲染这一对提示词，并生成可封存进请求快照的内容哈希。
    ///
    /// # Errors
    ///
    /// 覆盖内容不符合受限模板语言，或缺少必需运行时变量时返回错误。
    pub fn render(
        &self,
        key: PromptKey,
        variables: &BTreeMap<String, String>,
    ) -> Result<RenderedBuiltinPrompt, PromptTemplateError> {
        render_prompt_content(key, &self.system, &self.user, variables)
    }
}

/// 解析某项目（或系统作用域）下提示词键的有效内容。
///
/// # Errors
///
/// 数据库中保存的覆盖记录损坏或读取失败时返回错误。
pub async fn resolve_prompt(
    store: &Store,
    project_id: Option<ProjectId>,
    key: PromptKey,
) -> Result<EffectivePrompt, StoreError> {
    if let Some(project_id) = project_id
        && let Some(overridden) = store
            .prompt_override(PromptOverrideScope::Project, Some(project_id), key.as_str())
            .await?
    {
        return Ok(effective_from_override(overridden, PromptSource::Project));
    }
    if let Some(overridden) = store
        .prompt_override(PromptOverrideScope::System, None, key.as_str())
        .await?
    {
        return Ok(effective_from_override(overridden, PromptSource::System));
    }
    let builtin = builtin_prompt(key);
    Ok(EffectivePrompt {
        system: builtin.system.to_owned(),
        user: builtin.user.to_owned(),
        source: PromptSource::Builtin,
    })
}

/// 解析某稿本正在使用的提示词内容：项目作用域由稿本所属项目决定。
///
/// # Errors
///
/// 稿本不存在，或覆盖记录损坏、读取失败时返回错误。
pub async fn resolve_prompt_for_version(
    store: &Store,
    version_id: VersionId,
    key: PromptKey,
) -> Result<EffectivePrompt, StoreError> {
    let project_id = store.project_id_for_version(version_id).await?;
    resolve_prompt(store, Some(project_id), key).await
}

fn effective_from_override(overridden: PromptOverride, source: PromptSource) -> EffectivePrompt {
    EffectivePrompt {
        system: overridden.system,
        user: overridden.user,
        source,
    }
}

/// 目录中一条内建提示词的覆盖与当前来源。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptOverrideContent {
    pub system: String,
    pub user: String,
    pub revision: String,
    pub updated_at_ms: String,
}

/// 一条内建提示词的目录条目。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptCatalogEntry {
    pub key: &'static str,
    pub variables: &'static [PromptVariableDefinition],
    pub builtin_system: String,
    pub builtin_user: String,
    pub system_override: Option<PromptOverrideContent>,
    pub project_override: Option<PromptOverrideContent>,
    pub effective_source: PromptSource,
    pub effective_system: String,
    pub effective_user: String,
}

/// 列出全部内建提示词及其覆盖状态与当前有效内容。
///
/// `project_id` 为 `None` 时只解析系统作用域，用于系统设置页。
///
/// # Errors
///
/// 提示词覆盖记录损坏或读取失败时返回错误。
pub async fn prompt_catalog(
    store: &Store,
    project_id: Option<ProjectId>,
) -> Result<Vec<PromptCatalogEntry>, StoreError> {
    let system_overrides = overrides_by_key(store, PromptOverrideScope::System, None).await?;
    let project_overrides = match project_id {
        Some(project_id) => {
            overrides_by_key(store, PromptOverrideScope::Project, Some(project_id)).await?
        }
        None => BTreeMap::new(),
    };
    Ok(PromptKey::ALL
        .into_iter()
        .map(|key| {
            let builtin = builtin_prompt(key);
            let system_override = system_overrides.get(key.as_str());
            let project_override = project_overrides.get(key.as_str());
            let (source, system, user) = match project_override {
                Some(overridden) => (
                    PromptSource::Project,
                    overridden.system.as_str(),
                    overridden.user.as_str(),
                ),
                None => match system_override {
                    Some(overridden) => (
                        PromptSource::System,
                        overridden.system.as_str(),
                        overridden.user.as_str(),
                    ),
                    None => (PromptSource::Builtin, builtin.system, builtin.user),
                },
            };
            PromptCatalogEntry {
                key: key.as_str(),
                variables: builtin_prompt_variables(key),
                builtin_system: builtin.system.to_owned(),
                builtin_user: builtin.user.to_owned(),
                system_override: system_override.map(override_content),
                project_override: project_override.map(override_content),
                effective_source: source,
                effective_system: system.to_owned(),
                effective_user: user.to_owned(),
            }
        })
        .collect())
}

async fn overrides_by_key(
    store: &Store,
    scope: PromptOverrideScope,
    project_id: Option<ProjectId>,
) -> Result<BTreeMap<String, PromptOverride>, StoreError> {
    Ok(store
        .list_prompt_overrides(scope, project_id)
        .await?
        .into_iter()
        .map(|overridden| (overridden.prompt_key.clone(), overridden))
        .collect())
}

fn override_content(overridden: &PromptOverride) -> PromptOverrideContent {
    PromptOverrideContent {
        system: overridden.system.clone(),
        user: overridden.user.clone(),
        revision: overridden.revision.to_string(),
        updated_at_ms: overridden.updated_at_ms.to_string(),
    }
}

/// 保存前预览的请求体。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewPromptRequest {
    #[serde(default)]
    pub project_id: Option<ProjectId>,
    pub prompt_key: String,
    #[serde(default)]
    pub system: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
}

/// 用示例值渲染后的提示词内容。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewPromptResponse {
    pub prompt_key: &'static str,
    pub source: PromptSource,
    pub system: String,
    pub user: String,
}

/// 校验并渲染一份提示词草稿。
///
/// 未提供的部分沿用当前有效内容，因此界面既能预览「将会保存的草稿」，也能预览
/// 「模型实际会收到什么」。示例值只用于预览，不写入任何持久状态。
///
/// # Errors
///
/// 提示词键未知、草稿不符合受限模板语言、缺少必需占位符或读取覆盖失败时返回错误。
pub async fn preview_prompt(
    store: &Store,
    request: PreviewPromptRequest,
) -> Result<PreviewPromptResponse, ApiError> {
    let key = parse_prompt_key(&request.prompt_key)?;
    let effective = resolve_prompt(store, request.project_id, key).await?;
    let system = request.system.unwrap_or(effective.system);
    let user = request.user.unwrap_or(effective.user);
    // 保存前的校验与渲染共用同一份示例值，因此预览失败即保存会被拒绝。
    validate_builtin_prompt_pair(key, &system, &user)?;
    let rendered = render_prompt_content(key, &system, &user, &builtin_prompt_examples(key))?;
    Ok(PreviewPromptResponse {
        prompt_key: key.as_str(),
        source: effective.source,
        system: rendered.system,
        user: rendered.user,
    })
}

/// 把 HTTP 路径或请求体中的稳定键解析为提示词键。
///
/// # Errors
///
/// 键不属于内建目录时返回 [`ApiError::UnknownPromptKey`]。
pub fn parse_prompt_key(value: &str) -> Result<PromptKey, ApiError> {
    PromptKey::from_stable_key(value.trim())
        .ok_or_else(|| ApiError::UnknownPromptKey(value.trim().to_owned()))
}
