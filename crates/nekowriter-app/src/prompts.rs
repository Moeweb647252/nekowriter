//! 内建提示词的覆盖解析、目录与保存前预览。
//!
//! 有效提示词的优先级固定为「项目覆盖 → 系统覆盖 → 代码内建」。解析只在这里实现
//! 一次：HTTP 目录、预览与调度器构建请求都必须经过它，避免出现第二套优先级规则。
//!
//! 覆盖内容按受限模板语言校验（未知变量、缺少必需占位符、不完整括号都会被拒
//! 绝），预览使用内建变量声明的示例值渲染，因此不需要真实稿本或密钥。

use std::collections::BTreeMap;

use nekowriter_agent::{
    PromptKey, PromptVariableDefinition, RenderedBuiltinPrompt, builtin_prompt,
    builtin_prompt_examples, builtin_prompt_variables, render_prompt_content,
    validate_builtin_prompt_pair,
};
use nekowriter_core::{
    ProjectId, TemplateError, TemplateVariable, VersionId, render_template_preview,
    require_declared_placeholders, validate_template, validate_template_variables,
};
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
    ) -> Result<RenderedBuiltinPrompt, TemplateError> {
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
///
/// 两个变体共用同一条路由，因为它们都是「类型化模板校验与预览」：内建提示词用
/// 声明的示例值渲染，用户模板用调用方提供的值、缺省回落到示例值。
#[derive(Debug, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum PreviewPromptRequest {
    Builtin {
        #[serde(default)]
        project_id: Option<ProjectId>,
        prompt_key: String,
        #[serde(default)]
        system: Option<String>,
        #[serde(default)]
        user: Option<String>,
    },
    Template {
        body: String,
        #[serde(default)]
        variables: Vec<TemplateVariable>,
        #[serde(default)]
        values: BTreeMap<String, String>,
    },
}

/// 用示例值渲染后的提示词内容。
#[derive(Debug, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum PreviewPromptResponse {
    Builtin {
        prompt_key: &'static str,
        source: PromptSource,
        system: String,
        user: String,
    },
    Template {
        rendered: String,
        used_variables: Vec<String>,
    },
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
    match request {
        PreviewPromptRequest::Builtin {
            project_id,
            prompt_key,
            system,
            user,
        } => {
            let key = parse_prompt_key(&prompt_key)?;
            let effective = resolve_prompt(store, project_id, key).await?;
            let system = system.unwrap_or(effective.system);
            let user = user.unwrap_or(effective.user);
            // 保存前的校验与渲染共用同一份示例值，因此预览失败即保存会被拒绝。
            validate_builtin_prompt_pair(key, &system, &user)?;
            let rendered =
                render_prompt_content(key, &system, &user, &builtin_prompt_examples(key))?;
            Ok(PreviewPromptResponse::Builtin {
                prompt_key: key.as_str(),
                source: effective.source,
                system: rendered.system,
                user: rendered.user,
            })
        }
        PreviewPromptRequest::Template {
            body,
            variables,
            values,
        } => {
            validate_template_variables(&variables)?;
            let used = validate_template(&body, &variables)?;
            require_declared_placeholders(&used, &variables)?;
            let rendered = render_template_preview(&body, &variables, &values)?;
            Ok(PreviewPromptResponse::Template {
                rendered,
                used_variables: used.into_iter().collect(),
            })
        }
    }
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

#[cfg(test)]
mod tests {
    use nekowriter_core::NewProject;
    use nekowriter_store::NewPromptOverride;
    use tempfile::TempDir;
    use uuid::Uuid;

    use super::*;

    /// 三级优先级的唯一权威实现，必须由自动化测试锁住。
    async fn prompt_store() -> (TempDir, Store, ProjectId, VersionId) {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let bundle = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("提示词优先级", "雨夜的来客", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        (data_dir, store, bundle.project.id, bundle.version.id)
    }

    async fn save_override(
        store: &Store,
        key: PromptKey,
        scope: PromptOverrideScope,
        project_id: Option<ProjectId>,
        system: &str,
        user: &str,
    ) {
        store
            .save_prompt_override(
                Uuid::new_v4(),
                0,
                NewPromptOverride {
                    prompt_key: key.as_str().to_owned(),
                    scope,
                    project_id,
                    system: system.to_owned(),
                    user: user.to_owned(),
                },
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn resolve_prompt_prefers_project_then_system_then_builtin() {
        let (_data_dir, store, project_id, _version_id) = prompt_store().await;
        let key = PromptKey::PreparationGenerate;
        let builtin = builtin_prompt(key);

        // 没有覆盖时使用代码内建内容。
        let resolved = resolve_prompt(&store, Some(project_id), key).await.unwrap();
        assert_eq!(resolved.source, PromptSource::Builtin);
        assert_eq!(resolved.system, builtin.system);
        assert_eq!(resolved.user, builtin.user);

        // 系统覆盖生效。
        save_override(
            &store,
            key,
            PromptOverrideScope::System,
            None,
            "系统 system",
            "系统 user",
        )
        .await;
        let resolved = resolve_prompt(&store, Some(project_id), key).await.unwrap();
        assert_eq!(resolved.source, PromptSource::System);
        assert_eq!(resolved.system, "系统 system");

        // 项目覆盖优先于系统覆盖。
        save_override(
            &store,
            key,
            PromptOverrideScope::Project,
            Some(project_id),
            "项目 system",
            "项目 user",
        )
        .await;
        let resolved = resolve_prompt(&store, Some(project_id), key).await.unwrap();
        assert_eq!(resolved.source, PromptSource::Project);
        assert_eq!(resolved.system, "项目 system");
        assert_eq!(resolved.user, "项目 user");

        // 系统作用域看不到项目覆盖。
        let system_scope = resolve_prompt(&store, None, key).await.unwrap();
        assert_eq!(system_scope.source, PromptSource::System);
        assert_eq!(system_scope.system, "系统 system");
    }

    #[tokio::test]
    async fn resolve_prompt_falls_back_after_reset() {
        let (_data_dir, store, project_id, version_id) = prompt_store().await;
        let key = PromptKey::PreparationGenerate;
        save_override(
            &store,
            key,
            PromptOverrideScope::System,
            None,
            "系统 system",
            "系统 user",
        )
        .await;
        save_override(
            &store,
            key,
            PromptOverrideScope::Project,
            Some(project_id),
            "项目 system",
            "项目 user",
        )
        .await;

        let resolved = resolve_prompt_for_version(&store, version_id, key)
            .await
            .unwrap();
        assert_eq!(resolved.source, PromptSource::Project);

        // 重置项目覆盖后回落到系统覆盖；再重置则回到内建。
        store
            .reset_prompt_override(
                Uuid::new_v4(),
                1,
                PromptOverrideScope::Project,
                Some(project_id),
                key.as_str(),
            )
            .await
            .unwrap();
        let resolved = resolve_prompt_for_version(&store, version_id, key)
            .await
            .unwrap();
        assert_eq!(resolved.source, PromptSource::System);
        assert_eq!(resolved.system, "系统 system");

        store
            .reset_prompt_override(
                Uuid::new_v4(),
                1,
                PromptOverrideScope::System,
                None,
                key.as_str(),
            )
            .await
            .unwrap();
        let resolved = resolve_prompt_for_version(&store, version_id, key)
            .await
            .unwrap();
        assert_eq!(resolved.source, PromptSource::Builtin);
        assert_eq!(resolved.system, builtin_prompt(key).system);
    }

    #[tokio::test]
    async fn prompt_catalog_reports_the_effective_source_per_key() {
        let (_data_dir, store, project_id, _version_id) = prompt_store().await;
        save_override(
            &store,
            PromptKey::PreparationGenerate,
            PromptOverrideScope::Project,
            Some(project_id),
            "项目 system",
            "项目 user",
        )
        .await;

        let catalog = prompt_catalog(&store, Some(project_id)).await.unwrap();
        let entry = catalog
            .iter()
            .find(|entry| entry.key == PromptKey::PreparationGenerate.as_str())
            .expect("目录应包含准备阶段提示词");
        assert_eq!(entry.effective_source, PromptSource::Project);
        assert!(entry.project_override.is_some());
        assert!(entry.system_override.is_none());
        // 其余键没有覆盖，必须报告内建来源。
        assert!(
            catalog
                .iter()
                .filter(|entry| entry.key != PromptKey::PreparationGenerate.as_str())
                .all(|entry| entry.effective_source == PromptSource::Builtin)
        );
    }

    /// 模板预览与内建预览共用同一条路由，但必须走同一份受限模板校验。
    #[tokio::test]
    async fn preview_route_renders_user_templates_and_rejects_bad_ones() {
        let (_data_dir, store, _project_id, _version_id) = prompt_store().await;
        let variables = vec![TemplateVariable {
            name: "tone".to_owned(),
            description: "语气".to_owned(),
            value_type: "text".to_owned(),
            required: true,
            example: "克制".to_owned(),
        }];
        let rendered = preview_prompt(
            &store,
            PreviewPromptRequest::Template {
                body: "保持{{tone}}。".to_owned(),
                variables: variables.clone(),
                values: BTreeMap::from([("tone".to_owned(), "冷静".to_owned())]),
            },
        )
        .await
        .unwrap();
        match rendered {
            PreviewPromptResponse::Template {
                rendered,
                used_variables,
            } => {
                assert_eq!(rendered, "保持冷静。");
                assert_eq!(used_variables, vec!["tone".to_owned()]);
            }
            PreviewPromptResponse::Builtin { .. } => panic!("模板预览不能返回内建变体"),
        }

        // 未声明的变量、缺失必需占位符与不完整括号都必须被拒绝。
        let error = preview_prompt(
            &store,
            PreviewPromptRequest::Template {
                body: "{{unknown}}".to_owned(),
                variables: variables.clone(),
                values: BTreeMap::new(),
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(error, ApiError::PromptTemplate(_)));

        let error = preview_prompt(
            &store,
            PreviewPromptRequest::Template {
                body: "缺少占位符".to_owned(),
                variables,
                values: BTreeMap::new(),
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(error, ApiError::PromptTemplate(_)));
    }

    #[test]
    fn parse_prompt_key_trims_and_rejects_unknown_keys() {
        assert_eq!(
            parse_prompt_key(" preparation.generate ").unwrap(),
            PromptKey::PreparationGenerate
        );
        let error = parse_prompt_key("not.a.prompt").unwrap_err();
        assert!(matches!(error, ApiError::UnknownPromptKey(_)));
    }
}
